use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, middleware};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env;
use std::sync::Arc;
use std::time::Duration;

mod gemini;
mod key_rotator;
mod search;

use key_rotator::KeyRotator;

const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct ChatRequest {
    message: String,
}

#[derive(Debug, Serialize, Clone)]
struct ClassificationResult {
    id: i32,
    kode: String,
    deskripsi: String,
    path: String,
    similarity: f64,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    results: Vec<ClassificationResult>,
    perihal: String,
    explanation: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_secs: Option<u64>,
}

struct AppState {
    db: PgPool,
    key_rotator: Arc<KeyRotator>,
    last_request: std::sync::Mutex<std::time::Instant>,
    rate_limit_interval: Duration,
}

async fn chat(
    state: web::Data<AppState>,
    body: web::Json<ChatRequest>,
) -> HttpResponse {
    let message = body.message.trim();
    if message.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Pesan tidak boleh kosong".into(),
            retry_after_secs: None,
        });
    }

    // Rate limiter — hanya di level request, bukan per-key
    {
        let mut last = state.last_request.lock().unwrap();
        let now = std::time::Instant::now();
        if let Some(next_allowed) = last.checked_add(state.rate_limit_interval) {
            if now < next_allowed {
                let wait = (next_allowed - now).as_secs() + 1;
                return HttpResponse::TooManyRequests().json(ErrorResponse {
                    error: format!(
                        "Mohon tunggu {} detik. API Key gratis dibatasi {} detik per request.",
                        wait, state.rate_limit_interval.as_secs()
                    ),
                    retry_after_secs: Some(wait),
                });
            }
        }
        *last = now;
    }

    let embedding = match state.key_rotator.try_all(|key| {
        let msg = message.to_string();
        async move { gemini::embed_text(&key, &msg).await }
    }).await {
        Ok((emb, _used_key)) => emb,
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Embedding error (all keys exhausted): {err_str}");
            if err_str.contains("429") || err_str.contains("RESOURCE_EXHAUSTED") {
                return HttpResponse::TooManyRequests().json(ErrorResponse {
                    error: "Semua API Key Gemini sedang sibuk (rate limit). Coba lagi dalam beberapa menit.".into(),
                    retry_after_secs: Some(180),
                });
            }
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Gagal generate embedding: {err_str}"),
                retry_after_secs: None,
            });
        }
    };

    let results = match search::similarity_search(&state.db, &embedding, 10).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Search error: {e}");
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Gagal pencarian: {e}"),
                retry_after_secs: None,
            });
        }
    };

    let (reranked, perihal, explanation) = match state.key_rotator.try_all(|key| {
        let msg = message.to_string();
        let res = results.clone();
        async move { gemini::rerank_and_explain(&key, &msg, &res).await }
    }).await {
        Ok((result, _used_key)) => result,
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Rerank error (all keys exhausted): {err_str}");
            (
                results.clone(),
                String::new(),
                format!("\u{26a0}\u{fe0f} Gemini tidak dapat melakukan reranking: {}. Hasil diurutkan berdasarkan similarity semantic.", err_str),
            )
        }
    };

    HttpResponse::Ok().json(ChatResponse { results: reranked, perihal, explanation })
}

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"status": "ok"}))
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/klasifikasi_arsip".into());

    // Baca multi key dari GEMINI_API_KEYS (comma-separated)
    let api_keys_raw = env::var("GEMINI_API_KEYS")
        .or_else(|_| env::var("GEMINI_API_KEY")) // fallback ke key tunggal lama
        .unwrap_or_else(|_| "".into());

    let keys: Vec<String> = api_keys_raw
        .split(',')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();

    if keys.is_empty() {
        eprintln!("⚠️  WARNING: Tidak ada GEMINI_API_KEY ditemukan!");
        eprintln!("    Set GEMINI_API_KEYS='key1,key2,key3' atau GEMINI_API_KEY='key'");
    } else {
        println!("🔑 Loaded {} Gemini API key(s)", keys.len());
    }

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse()
        .unwrap_or(3000);

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    let key_rotator = Arc::new(KeyRotator::new(keys));
    let rate_limit_interval = Duration::from_secs(MIN_REQUEST_INTERVAL.as_secs());

    println!("Server running on http://{}:{}", host, port);

    let state = web::Data::new(AppState {
        db,
        key_rotator,
        last_request: std::sync::Mutex::new(std::time::Instant::now() - MIN_REQUEST_INTERVAL),
        rate_limit_interval,
    });

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .wrap(cors)
            .wrap(middleware::Logger::default())
            .app_data(state.clone())
            .route("/api/health", web::get().to(health))
            .route("/api/chat", web::post().to(chat))
    })
    .bind((host.as_str(), port))?
    .run()
    .await?;

    Ok(())
}
