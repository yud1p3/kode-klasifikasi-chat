use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, middleware};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env;
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod gemini;
mod search;

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
    gemini_api_key: String,
    last_request: Mutex<Instant>,
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

    {
        let mut last = state.last_request.lock().unwrap();
        let now = Instant::now();
        if let Some(next_allowed) = last.checked_add(MIN_REQUEST_INTERVAL) {
            if now < next_allowed {
                let wait = (next_allowed - now).as_secs() + 1;
                return HttpResponse::TooManyRequests().json(ErrorResponse {
                    error: format!(
                        "Mohon tunggu {} detik. API Key gratis dibatasi {} detik per request.",
                        wait, MIN_REQUEST_INTERVAL.as_secs()
                    ),
                    retry_after_secs: Some(wait),
                });
            }
        }
        *last = now;
    }

    let embedding = match gemini::embed_text(&state.gemini_api_key, message).await {
        Ok(emb) => emb,
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Embedding error: {err_str}");
            if err_str.contains("429") || err_str.contains("RESOURCE_EXHAUSTED") {
                return HttpResponse::TooManyRequests().json(ErrorResponse {
                    error: "API Key Gemini sedang sibuk (rate limit). Coba lagi dalam 30 detik.".into(),
                    retry_after_secs: Some(30),
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

    let (reranked, explanation) = match gemini::rerank_and_explain(
        &state.gemini_api_key,
        message,
        &results,
    ).await {
        Ok((reranked, explanation)) => (reranked, explanation),
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Rerank error: {err_str}");
            (
                results.clone(),
                format!("\u{26a0}\u{fe0f} Gemini tidak dapat melakukan reranking: {}. Hasil diurutkan berdasarkan similarity semantic.", err_str),
            )
        }
    };

    HttpResponse::Ok().json(ChatResponse { results: reranked, explanation })
}

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"status": "ok"}))
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/klasifikasi_arsip".into());
    let gemini_api_key = env::var("GEMINI_API_KEY")
        .unwrap_or_else(|_| "free-rotation".into());
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse()
        .unwrap_or(3000);

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    println!("Server running on http://{}:{}", host, port);

    let state = web::Data::new(AppState {
        db,
        gemini_api_key,
        last_request: Mutex::new(Instant::now() - MIN_REQUEST_INTERVAL),
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
