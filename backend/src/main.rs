use actix_cors::Cors;
use actix_multipart::Multipart;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, middleware};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;

mod auth;
mod feedback;
mod gemini;
mod key_rotator;
mod quota;
mod search;

use key_rotator::KeyRotator;

const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct ChatRequest {
    message: String,
    /// API Key Gemini milik pengguna (opsional, legacy tunggal). Diprioritaskan di atas key server.
    #[serde(default)]
    api_key: Option<String>,
    /// Daftar API Key Gemini milik pengguna (multi-key). Dicoba berurutan (rotasi)
    /// sebelum fallback ke key server. `api_key` tunggal tetap didukung sebagai kompatibilitas.
    #[serde(default)]
    api_keys: Option<Vec<String>>,
}

/// Gabungan key pengguna (api_key legacy + api_keys), urut, deduplikasi, tanpa kosong.
fn merge_user_keys(api_key: &Option<String>, api_keys: &Option<Vec<String>>) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Some(k) = api_key {
        let t = k.trim();
        if !t.is_empty() {
            v.push(t.to_string());
        }
    }
    if let Some(ks) = api_keys {
        for k in ks {
            let t = k.trim();
            if !t.is_empty() && !v.iter().any(|x| x == t) {
                v.push(t.to_string());
            }
        }
    }
    v
}

impl ChatRequest {
    fn user_keys(&self) -> Vec<String> {
        merge_user_keys(&self.api_key, &self.api_keys)
    }
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
    quota: quota::Quota,
    auth: auth::AuthConfig,
    /// Email admin (ADMIN_EMAILS, comma-separated) — satu-satunya yang boleh menghapus feedback.
    admin_emails: Vec<String>,
    /// Password secret (DELETE_SECRET) yang wajib dimasukkan admin untuk menghapus feedback.
    delete_secret: String,
    /// Pengaman anti brute-force percobaan password hapus feedback.
    delete_guard: DeleteGuard,
}

impl AppState {
    fn is_admin(&self, email: &str) -> bool {
        self.admin_emails.iter().any(|e| e.eq_ignore_ascii_case(email.trim()))
    }
}

/// Bandingkan dua string dengan waktu konstan (hindari timing attack pada password).
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Pengaman anti brute-force untuk password hapus feedback.
/// Pelacakan per email admin: setelah N percobaan gagal berurutan, email
/// terkunci selama durasi tertentu. State in-memory (reset saat server restart).
struct DeleteGuard {
    max_attempts: u32,
    lockout_secs: u64,
    /// email → (jumlah gagal berurutan saat ini, waktu terkunci sampai)
    state: std::sync::Mutex<HashMap<String, (u32, Option<std::time::Instant>)>>,
}

impl DeleteGuard {
    fn new(max_attempts: u32, lockout_secs: u64) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            // Clamp minimal 1 agar DELETE_LOCKOUT_SECS=0 tidak membatalkan proteksi
            lockout_secs: lockout_secs.max(1),
            state: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Ok bila boleh mencoba password. Err(detik tersisa) bila sedang terkunci.
    fn check(&self, email: &str) -> Result<(), u64> {
        let st = self.state.lock().unwrap();
        if let Some((_, Some(until))) = st.get(email) {
            let now = std::time::Instant::now();
            if now < *until {
                return Err(until.duration_since(now).as_secs() + 1);
            }
        }
        Ok(())
    }

    /// Catat percobaan gagal. Mengembalikan Some(detik lockout) bila percobaan
    /// ini memicu penguncian (N gagal berurutan tercapai).
    fn record_fail(&self, email: &str) -> Option<u64> {
        let mut st = self.state.lock().unwrap();
        let now = std::time::Instant::now();
        let (count, until) = st.entry(email.to_string()).or_insert((0, None));
        // Lockout lama sudah lewat → hitung ulang dari nol
        if let Some(u) = until {
            if now >= *u {
                *count = 0;
                *until = None;
            }
        }
        *count += 1;
        if *count >= self.max_attempts {
            *until = Some(now + std::time::Duration::from_secs(self.lockout_secs));
            return Some(self.lockout_secs);
        }
        None
    }

    /// Berhasil → bersihkan riwayat percobaan email ini.
    fn record_success(&self, email: &str) {
        self.state.lock().unwrap().remove(email);
    }
}

/// Susun teks untuk embedding yang KONSISTEN dengan query chat:
/// SELALU "FUNGSI > perihal_inti" via Gemini select_fungsi, baik naskah
/// pendek maupun panjang. perihal_inti dibersihkan dari nama orang,
/// tempat/wilayah, dan keterangan waktu — sesuai struktur path dataset yang
/// tidak pernah memuat keterangan semacam itu. Dikembalikan juga
/// perihal_lengkap (detail apa adanya) untuk ditampilkan di UI & feedback.
/// Fallback ke teks asli bila gagal / rate limit.
/// Catatan: setiap pemanggilan select_fungsi menghabiskan 1 kuota chat.
async fn build_embed_query(state: &AppState, api_keys: &[String], message: &str) -> (String, String) {
    // Baca Fungsi/Urusan induk langsung dari DB (distinct level-1 path)
    let daftar_fungsi: String = match sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT trim(deskripsi) FROM klasifikasi_embedding WHERE LENGTH(kode) = 3 ORDER BY 1"
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows.join(", "),
        Err(e) => {
            eprintln!("gagal baca fungsi DB: {e}");
            String::new()
        }
    };
    match state
        .key_rotator
        .try_all_prefer(api_keys, |key| {
            let msg = message.to_string();
            let df = daftar_fungsi.clone();
            async move { gemini::select_fungsi(&key, &msg, &df).await }
        })
        .await
    {
        Ok(((fungsi, perihal_inti, perihal_lengkap), _)) if !fungsi.is_empty() && !perihal_inti.is_empty() => {
            eprintln!("Fungsi terpilih: {fungsi} | Perihal inti: {perihal_inti}");
            state.quota.record_chat(1);
            (format!("{} > {}", fungsi, perihal_inti), perihal_lengkap)
        }
        Ok(((_fungsi, _perihal_inti, perihal_lengkap), _)) => {
            // Gemini tetap dipanggil (kuota terpakai) meski field inti kosong
            state.quota.record_chat(1);
            eprintln!("select_fungsi: field inti kosong, pakai teks asli");
            (message.to_string(), perihal_lengkap)
        }
        Err(e) => {
            eprintln!("select_fungsi gagal, pakai teks asli: {e}");
            (message.to_string(), String::new())
        }
    }
}

/// Ganti prefix "Perihal: ..." pada kalimat penjelasan agar SELALU konsisten
/// dengan perihal tampilan (perihal_lengkap), apa pun yang dikembalikan model.
/// Bila pola tidak ditemukan, kembalikan penjelasan apa adanya.
fn fix_explanation_perihal(explanation: &str, perihal: &str) -> String {
    const PREFIX: &str = "Perihal: ";
    if let Some(rest) = explanation.strip_prefix(PREFIX) {
        if let Some(end) = rest.find(". Kode klasifikasi") {
            // Buang tanda baca/whitespace di ujung perihal agar tidak dobel titik
            let p = perihal.trim().trim_end_matches('.').trim_end();
            return format!("{PREFIX}{}{}", p, &rest[end..]);
        }
    }
    explanation.to_string()
}

async fn chat(
    state: web::Data<AppState>,
    body: web::Json<ChatRequest>,
) -> HttpResponse {
    // Chat terbuka untuk semua (tanpa login). Rate limit & kuota tetap berlaku;
    // API key Gemini pengguna dikirim via body (opsional).
    let message = body.message.trim();
    if message.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Pesan tidak boleh kosong".into(),
            retry_after_secs: None,
        });
    }
    // Daftar API Key pengguna (multi-key, rotasi otomatis sebelum fallback key server)
    let keys = body.user_keys();

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

    // Proaktif: cek kuota free (RPM/RPD) SEBELUM memanggil Gemini,
    // agar tidak sampai kena error 429 dari Google.
    // Estimasi call: selalu 2 chat (select_fungsi + rerank) + 1 embed,
    // karena semua naskah (pendek & panjang) melewati select_fungsi.
    let chat_calls_estimate: u32 = 2;
    if let Err((wait, why)) = state.quota.check(chat_calls_estimate, 1) {
        eprintln!("⏳ Kuota free block: {why}, tunggu {wait} detik");
        return HttpResponse::TooManyRequests().json(ErrorResponse {
            error: format!(
                "Kuota free terpakai habis ({why}). Mohon tunggu {wait} detik sebelum mencoba lagi."
            ),
            retry_after_secs: Some(wait),
        });
    }

    // Susun teks query embedding — dipakai juga saat menyimpan feedback
    // (build_embed_query) agar few-shot dicocokkan dalam ruang embedding yang sama.
    let (embed_query, perihal_lengkap) = build_embed_query(&state, &keys, message).await;

    let embedding = match state.key_rotator.try_all_prefer(&keys, |key| {
        let msg = embed_query.clone();
        async move { gemini::embed_text(&key, &msg).await }
    }).await {
        Ok((emb, _used_key)) => {
            state.quota.record_embed(1);
            emb
        }
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Embedding error (all keys exhausted): {err_str}");
            // Prioritas 1: cooldown global aktif → 429 dengan retry_after akurat
            if let Some(secs) = state.key_rotator.cooldown_remaining_secs() {
                return HttpResponse::TooManyRequests().json(ErrorResponse {
                    error: format!("Semua API Key Gemini dalam cooldown rate limit. Tunggu {secs} detik."),
                    retry_after_secs: Some(secs),
                });
            }
            // Prioritas 2: semua key kena 429 → 429 dengan periode reset Google (~60 detik)
            if err_str.contains("429") || err_str.contains("RESOURCE_EXHAUSTED") {
                return HttpResponse::TooManyRequests().json(ErrorResponse {
                    error: "Semua API Key Gemini sedang sibuk (rate limit). Coba lagi dalam 1 menit.".into(),
                    retry_after_secs: Some(60),
                });
            }
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Gagal generate embedding: {err_str}"),
                retry_after_secs: None,
            });
        }
    };

    // Few-shot: koreksi arsiparis tervalidasi yang naskahnya paling mirip query ini
    let fewshot_text = match feedback::fetch_fewshot(&state.db, &embedding).await {
        Ok(examples) => feedback::format_fewshot(&examples),
        Err(e) => {
            eprintln!("fewshot fetch error: {e}");
            String::new()
        }
    };

    // Pencarian semantic: pgvector (PostgreSQL) — satu-satunya backend search
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

    let (reranked, explanation, perihal_rerank) = match state.key_rotator.try_all_prefer(&keys, |key| {
        let msg = message.to_string();
        let fs = fewshot_text.clone();
        let res = results.clone();
        // Perihal tampilan (perihal_lengkap) diteruskan agar penjelasan
        // "Perihal: X" konsisten dengan yang ditampilkan di UI.
        let ph = perihal_lengkap.clone();
        async move { gemini::rerank_and_explain(&key, &msg, &fs, &ph, &res).await }
    }).await {
        Ok((result, _used_key)) => {
            state.quota.record_chat(1);
            result
        }
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Rerank error (all keys exhausted): {err_str}");
            // Jika cooldown aktif, beri tahu user berapa lama harus menunggu
            let cooldown_note = state
                .key_rotator
                .cooldown_remaining_secs()
                .map(|s| format!(" Tunggu {s} detik sebelum mencoba lagi."))
                .unwrap_or_default();
            (
                results.clone(),
                format!("\u{26a0}\u{fe0f} Gemini tidak dapat melakukan reranking: {}. Hasil diurutkan berdasarkan similarity semantic.{}", err_str, cooldown_note),
                String::new(),
            )
        }
    };

    // Perihal tampilan: pakai perihal_lengkap dari select_fungsi (panggilan
    // Gemini awal, detail apa adanya); fallback ke perihal hasil rerank bila
    // select_fungsi gagal/tidak mengembalikan perihal_lengkap.
    let perihal = if !perihal_lengkap.is_empty() { perihal_lengkap } else { perihal_rerank };

    // Hard guarantee: prefix "Perihal: X" di kalimat penjelasan selalu memakai
    // perihal tampilan yang sama (tidak bergantung kepatuhan model).
    let explanation = if !perihal.is_empty() {
        fix_explanation_perihal(&explanation, &perihal)
    } else {
        explanation
    };

    HttpResponse::Ok().json(ChatResponse { results: reranked, perihal, explanation })
}


/// Ekstrak teks PDF via poppler (pdftotext).
/// Dipakai sebagai fallback untuk PDF SRIKANDI yang ToUnicode-nya rusak
/// (pdf.js menghasilkan karakter garbled, poppler membaca benar).
async fn extract_pdf(mut payload: Multipart) -> HttpResponse {
    use actix_web::web::BytesMut;
    use futures::StreamExt;

    // Terbuka untuk semua (bagian dari alur chat tanpa login).
    // Simpan file multipart ke temp
    let tmp_path = std::env::temp_dir().join(format!("kkl_upload_{}.pdf", std::process::id()));
    let mut buf = BytesMut::new();
    while let Some(Ok(mut field)) = payload.next().await {
        while let Some(Ok(chunk)) = field.next().await {
            buf.extend_from_slice(&chunk);
        }
    }
    if buf.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({"error": "File kosong"}));
    }
    if let Err(e) = std::fs::write(&tmp_path, &buf) {
        return HttpResponse::InternalServerError().json(serde_json::json!({"error": format!("Gagal simpan file: {e}")}));
    }

    // Jalankan pdftotext
    let out = std::process::Command::new("pdftotext")
        .args([tmp_path.to_str().unwrap(), "-"])
        .output();

    let _ = std::fs::remove_file(&tmp_path);

    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if text.is_empty() {
                HttpResponse::BadRequest().json(serde_json::json!({"error": "Tidak ada teks yang bisa diekstrak dari PDF"}))
            } else {
                HttpResponse::Ok().json(serde_json::json!({"text": text}))
            }
        }
        Ok(o) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("pdftotext gagal: {}", String::from_utf8_lossy(&o.stderr).chars().take(300).collect::<String>())
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("pdftotext tidak tersedia: {e}. Install poppler-utils atau gunakan fallback pdf.js.")
        })),
    }
}

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"status": "ok"}))
}

/// Info kuota free Gemini (RPM/RPD terpakai vs limit) untuk ditampilkan di UI.
async fn quota_info(state: web::Data<AppState>) -> HttpResponse {
    HttpResponse::Ok().json(state.quota.stats())
}

// ---------- Autentikasi Google ----------

#[derive(Debug, Deserialize)]
struct GoogleAuthRequest {
    code: String,
    code_verifier: String,
    /// Redirect URI yang dipakai frontend saat login (sesuai origin-nya).
    /// Divalidasi terhadap daftar GOOGLE_REDIRECT_URI sebelum dipakai tukar code.
    #[serde(default)]
    redirect_uri: Option<String>,
}

/// Konfigurasi auth untuk frontend (enabled, client_id, redirect_uri).
async fn auth_config(state: web::Data<AppState>) -> HttpResponse {
    HttpResponse::Ok().json(state.auth.info())
}

/// Tukar authorization code → token JWT sesi.
async fn auth_google(
    state: web::Data<AppState>,
    body: web::Json<GoogleAuthRequest>,
) -> HttpResponse {
    if !state.auth.enabled {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Auth Google tidak dikonfigurasi (GOOGLE_CLIENT_ID/GOOGLE_CLIENT_SECRET kosong).".into(),
            retry_after_secs: None,
        });
    }
    // Pakai redirect_uri yang dikirim frontend BILA termasuk daftar yang diizinkan;
    // selain itu (atau kosong) fallback ke URI pertama.
    let redirect_uri = body
        .redirect_uri
        .clone()
        .filter(|u| state.auth.is_allowed_redirect(u))
        .unwrap_or_else(|| state.auth.redirect_uris.first().cloned().unwrap_or_default());
    match auth::exchange_code(&state.auth, &body.code, &body.code_verifier, &redirect_uri).await {
        Ok(user) => match auth::issue_token(&state.auth, &user) {
            Ok(token) => HttpResponse::Ok().json(serde_json::json!({ "token": token, "user": user })),
            Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Gagal membuat token sesi: {e}"),
                retry_after_secs: None,
            }),
        },
        Err(e) => HttpResponse::Unauthorized().json(ErrorResponse {
            error: format!("Login gagal: {}", e.to_string().chars().take(200).collect::<String>()),
            retry_after_secs: None,
        }),
    }
}

/// Info user yang sedang login (dari JWT) + status admin.
async fn me(state: web::Data<AppState>, req: HttpRequest) -> HttpResponse {
    match auth::require_user(&req, &state.auth) {
        Ok(u) => {
            let is_admin = state.is_admin(&u.email);
            HttpResponse::Ok().json(serde_json::json!({
                "sub": u.sub,
                "email": u.email,
                "name": u.name,
                "is_admin": is_admin
            }))
        }
        Err(resp) => resp,
    }
}

// ---------- Feedback ----------

#[derive(Debug, Deserialize)]
struct FeedbackRequest {
    message: String,
    kode_ai: String,
    #[serde(default)]
    feedback_type: String, // positive | correction
    kode_koreksi: Option<String>,
    alasan: Option<String>,
    /// Perihal naskah (hasil rerank AI saat chat) — dipakai di prompt validasi & statistik.
    #[serde(default)]
    perihal: Option<String>,
    /// API Key Gemini milik pengguna (opsional, legacy tunggal). Diprioritaskan di atas key server.
    #[serde(default)]
    api_key: Option<String>,
    /// Daftar API Key Gemini milik pengguna (multi-key, rotasi otomatis).
    #[serde(default)]
    api_keys: Option<Vec<String>>,
    #[serde(default)]
    candidates: Vec<feedback::Candidate>,
    /// ID sesi perangkat/browser (di-generate frontend, disimpan di localStorage).
    /// Dipakai untuk mengaitkan feedback — termasuk yang anonim — ke sesi chat,
    /// tanpa perlu login.
    #[serde(default)]
    chat_id: Option<String>,
}

impl FeedbackRequest {
    fn user_keys(&self) -> Vec<String> {
        merge_user_keys(&self.api_key, &self.api_keys)
    }
}

/// Terima feedback (👍 atau koreksi), validasi koreksi via Gemini, simpan ke DB.
async fn submit_feedback(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<FeedbackRequest>,
) -> HttpResponse {
    let msg = body.message.trim();
    if msg.is_empty() || body.kode_ai.trim().is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "message dan kode_ai wajib diisi".into(),
            retry_after_secs: None,
        });
    }
    let ftype = body.feedback_type.trim().to_lowercase();
    if ftype != "positive" && ftype != "correction" {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "feedback_type harus 'positive' atau 'correction'".into(),
            retry_after_secs: None,
        });
    }
    // Daftar API Key pengguna (multi-key, rotasi otomatis sebelum fallback key server)
    let keys = body.user_keys();

    // ID sesi pengguna (opsional) — mengaitkan feedback anonim ke sesi chat.
    // Dibersihkan & dibatasi panjangnya agar aman masuk DB.
    let chat_id: Option<String> = body
        .chat_id
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(64).collect());

    // Privasi: simpan naskah terpotong (cukup untuk substansi & few-shot)
    let naskah_store: String = msg.chars().take(1000).collect();

    // Perihal naskah (hasil rerank AI saat chat) — untuk prompt validasi & statistik.
    // Dibatas panjangnya agar tidak membengkakkan prompt Gemini.
    let perihal: String = body
        .perihal
        .clone()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(300)
        .collect();

    // Feedback positif: TANPA WAJIB LOGIN — dicatat anonim bila user tidak
    // login; identitas tetap tercatat bila user kebetulan sedang login.
    if ftype == "positive" {
        let user = auth::optional_user(&req, &state.auth);
        let (sub, email, name) = match &user {
            Some(u) => (Some(u.sub.clone()), Some(u.email.clone()), Some(u.name.clone())),
            None => (None, None, None),
        };
        let res = sqlx::query(
            "INSERT INTO klasifikasi_feedback (naskah, kode_ai, feedback_type, status, kode_terbaik, perihal, user_sub, user_email, user_name, chat_id)
             VALUES ($1, $2, 'positive', 'validated', $3, $4, $5, $6, $7, $8)",
        )
        .bind(&naskah_store)
        .bind(&body.kode_ai)
        .bind(&body.kode_ai)
        .bind(&perihal)
        .bind(sub)
        .bind(email)
        .bind(name)
        .bind(chat_id)
        .execute(&state.db)
        .await;
        return match res {
            Ok(_) => HttpResponse::Ok().json(serde_json::json!({
                "valid": true,
                "kode_terbaik": body.kode_ai,
                "penjelasan": "Terima kasih atas konfirmasi! Feedback positif ini dicatat."
            })),
            Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Gagal simpan feedback: {e}"),
                retry_after_secs: None,
            }),
        };
    }

    // ---- Koreksi: WAJIB LOGIN (akuntabilitas koreksi yang dipakai few-shot) ----
    let user = match auth::require_user(&req, &state.auth) {
        Ok(u) => u,
        Err(r) => return r,
    };

    let kode_koreksi = match &body.kode_koreksi {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "kode_koreksi wajib diisi untuk koreksi".into(),
                retry_after_secs: None,
            });
        }
    };

    // Jaga-jaga: validasi koreksi memanggil Gemini (1 chat + 1 embed),
    // jadi pakai rate limiter & cek kuota yang sama seperti chat handler
    {
        let mut last = state.last_request.lock().unwrap();
        let now = std::time::Instant::now();
        if let Some(next_allowed) = last.checked_add(state.rate_limit_interval) {
            if now < next_allowed {
                let wait = (next_allowed - now).as_secs() + 1;
                return HttpResponse::TooManyRequests().json(ErrorResponse {
                    error: format!("Mohon tunggu {} detik sebelum mengirim koreksi.", wait),
                    retry_after_secs: Some(wait),
                });
            }
        }
        *last = now;
    }
    // Estimasi: koreksi = 2 chat (select_fungsi saat menyusun teks embedding
    // + validasi) + 1 embed, karena semua naskah melewati select_fungsi.
    let chat_calls_estimate: u32 = 2;
    if let Err((wait, why)) = state.quota.check(chat_calls_estimate, 1) {
        eprintln!("⏳ Kuota block (feedback): {why}");
        return HttpResponse::TooManyRequests().json(ErrorResponse {
            error: format!("Kuota free terpakai habis ({why}). Mohon tunggu {wait} detik."),
            retry_after_secs: Some(wait),
        });
    }

    // Lapis 1: kode koreksi harus ADA di dataset (blokir koreksi asal-asalan tanpa biaya Gemini)
    let kor_info = match feedback::lookup_kode(&state.db, &kode_koreksi).await {
        Ok(Some(info)) => info,
        Ok(None) => {
            let _ = sqlx::query(
                "INSERT INTO klasifikasi_feedback (naskah, kode_ai, feedback_type, kode_koreksi, alasan, perihal, user_sub, user_email, user_name, status, validasi_penjelasan, chat_id)
                 VALUES ($1, $2, 'correction', $3, $4, $5, $6, $7, $8, 'rejected', 'Kode tidak ditemukan di dataset', $9)",
            )
            .bind(&naskah_store)
            .bind(&body.kode_ai)
            .bind(&kode_koreksi)
            .bind(body.alasan.clone().unwrap_or_default())
            .bind(&perihal)
            .bind(&user.sub)
            .bind(&user.email)
            .bind(&user.name)
            .bind(chat_id)
            .execute(&state.db)
            .await;
            return HttpResponse::Ok().json(serde_json::json!({
                "valid": false,
                "kode_terbaik": serde_json::Value::Null,
                "penjelasan": format!("Kode {} tidak ditemukan di dataset klasifikasi.", kode_koreksi)
            }));
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Gagal lookup kode: {e}"),
                retry_after_secs: None,
            });
        }
    };
    let (kor_deskripsi, kor_path) = kor_info;

    // Ambil path kode AI dari DB (agar prompt validasi punya konteks hirarki utuh)
    let ai_info = feedback::lookup_kode(&state.db, &body.kode_ai).await.ok().flatten();
    let (ai_deskripsi, ai_path) = match &ai_info {
        Some((d, p)) => (d.clone(), p.clone()),
        None => ("(tidak diketahui)".to_string(), "(tidak diketahui)".to_string()),
    };

    // Lapis 2: validasi Gemini (path lengkap + top-3 kandidat)
    let validasi = state
        .key_rotator
        .try_all_prefer(&keys, |key| {
            let msg = msg.to_string();
            let ai_k = body.kode_ai.clone();
            let ai_d = ai_deskripsi.clone();
            let ai_p = ai_path.clone();
            let ko_k = kode_koreksi.clone();
            let ko_d = kor_deskripsi.clone();
            let ko_p = kor_path.clone();
            let alasan = body.alasan.clone().unwrap_or_default();
            let perihal_c = perihal.clone();
            let cands = body.candidates.clone();
            async move {
                feedback::validate_correction(
                    &key,
                    &msg,
                    &perihal_c,
                    (ai_k.as_str(), ai_d.as_str(), ai_p.as_str()),
                    (ko_k.as_str(), ko_d.as_str(), ko_p.as_str()),
                    &alasan,
                    &cands,
                )
                .await
            }
        })
        .await;

    let (valid, kode_terbaik, penjelasan, status, raw_note) = match validasi {
        Ok((res, _)) => {
            state.quota.record_chat(1);
            let mut kt = if res.kode_terbaik.trim().is_empty() {
                if res.valid { kode_koreksi.clone() } else { body.kode_ai.clone() }
            } else {
                res.kode_terbaik.clone()
            };
            // Anti-halusinasi: pastikan kode_terbaik benar-benar ada di dataset
            // sebelum masuk few-shot (fallback ke kode_koreksi yang sudah divalidasi lokal)
            if res.valid && kt != kode_koreksi {
                match feedback::lookup_kode(&state.db, &kt).await {
                    Ok(Some(_)) => {}
                    _ => kt = kode_koreksi.clone(),
                }
            }
            (
                res.valid,
                kt,
                res.penjelasan,
                if res.valid { "validated" } else { "rejected" },
                res.raw,
            )
        }
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Validasi koreksi gagal: {err_str}");
            // Simpan sebagai pending — tidak hilang, bisa direview/dipakai nanti
            (
                false,
                kode_koreksi.clone(),
                format!("Validasi gagal ({}). Disimpan sebagai pending.", err_str.chars().take(120).collect::<String>()),
                "pending",
                err_str,
            )
        }
    };

    // Embedding naskah (best-effort) — dipakai few-shot untuk query serupa.
    // Teks embedding DISELARASKAN dengan query chat (build_embed_query):
    // selalu "FUNGSI > perihal_inti", agar few-shot dicocokkan dalam ruang
    // embedding yang sama dengan query.
    let mut emb_store: Option<String> = None;
    if status == "validated" {
        let (embed_text, _perihal_lengkap) = build_embed_query(&state, &keys, msg).await;
        if let Ok((emb, _)) = state
            .key_rotator
            .try_all_prefer(&keys, |key| {
                let t = embed_text.clone();
                async move { gemini::embed_text(&key, &t).await }
            })
            .await
        {
            state.quota.record_embed(1);
            // Format pgvector harus dibungkus kurung siku: [0.1,0.2,...]
            emb_store = Some(format!(
                "[{}]",
                emb.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
            ));
        }
    }

    let insert_res = sqlx::query(
        "INSERT INTO klasifikasi_feedback (naskah, kode_ai, feedback_type, kode_koreksi, alasan, perihal, user_sub, user_email, user_name, status, kode_terbaik, validasi_penjelasan, validasi_raw, embedding, chat_id)
         VALUES ($1, $2, 'correction', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                 CASE WHEN $13::text IS NULL THEN NULL ELSE $13::vector END, $14)",
    )
    .bind(&naskah_store)
    .bind(&body.kode_ai)
    .bind(&kode_koreksi)
    .bind(body.alasan.clone().unwrap_or_default())
    .bind(&perihal)
    .bind(&user.sub)
    .bind(&user.email)
    .bind(&user.name)
    .bind(status)
    .bind(&kode_terbaik)
    .bind(&penjelasan)
    .bind(&raw_note)
    .bind(emb_store.as_deref())
    .bind(chat_id)
    .execute(&state.db)
    .await;

    match insert_res {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({
            "valid": valid,
            "kode_terbaik": kode_terbaik,
            "penjelasan": penjelasan,
        })),
        Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Gagal simpan feedback: {e}"),
            retry_after_secs: None,
        }),
    }
}

/// Statistik feedback (total, validasi, kode teratas, user teratas).
/// Mendukung filter opsional: ?perihal=<kata kunci>&status=validated|rejected|pending
async fn feedback_stats(
    state: web::Data<AppState>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    // Terbuka untuk semua (tidak wajib login)
    let perihal = query.get("perihal").map(|s| s.as_str());
    let status = query.get("status").map(|s| s.as_str());
    match feedback::fetch_stats(&state.db, perihal, status).await {
        Ok(v) => HttpResponse::Ok().json(v),
        Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Gagal baca statistik: {e}"),
            retry_after_secs: None,
        }),
    }
}

/// Hapus feedback — hanya admin (ADMIN_EMAILS) dengan password secret (DELETE_SECRET).
async fn delete_feedback(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<HashMap<String, String>>,
) -> HttpResponse {
    let user = match auth::require_user(&req, &state.auth) {
        Ok(u) => u,
        Err(r) => return r,
    };

    // Lapis 1: hanya admin yang boleh menghapus
    if !state.is_admin(&user.email) {
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "Hanya admin yang dapat menghapus feedback.".into(),
            retry_after_secs: None,
        });
    }
    // Lapis 2: fitur hapus harus dikonfigurasi (DELETE_SECRET)
    if state.delete_secret.is_empty() {
        return HttpResponse::ServiceUnavailable().json(ErrorResponse {
            error: "Fitur hapus feedback nonaktif: DELETE_SECRET belum dikonfigurasi di server.".into(),
            retry_after_secs: None,
        });
    }
    // Lapis 3: anti brute-force — blokir bila email ini sedang terkunci
    if let Err(wait) = state.delete_guard.check(&user.email) {
        return HttpResponse::TooManyRequests().json(ErrorResponse {
            error: format!(
                "Terlalu banyak percobaan password yang gagal. Tunggu {wait} detik sebelum mencoba lagi."
            ),
            retry_after_secs: Some(wait),
        });
    }
    // Lapis 4: verifikasi password secret (constant-time)
    let password = body.get("password").cloned().unwrap_or_default();
    if !constant_time_eq(&state.delete_secret, &password) {
        // Catat kegagalan; bila batas tercapai → kunci email ini
        if let Some(wait) = state.delete_guard.record_fail(&user.email) {
            let dur = if wait >= 60 {
                format!("{:.0} menit", wait as f64 / 60.0)
            } else {
                format!("{wait} detik")
            };
            eprintln!("🔒 Admin {} terkunci selama {dur} ({} percobaan gagal)", user.email, state.delete_guard.max_attempts);
            return HttpResponse::TooManyRequests().json(ErrorResponse {
                error: format!(
                    "Terlalu banyak percobaan password ({}) — terkunci selama {dur}.",
                    state.delete_guard.max_attempts
                ),
                retry_after_secs: Some(wait),
            });
        }
        return HttpResponse::Forbidden().json(ErrorResponse {
            error: "Password secret salah.".into(),
            retry_after_secs: None,
        });
    }
    state.delete_guard.record_success(&user.email);

    let id = path.into_inner();
    match sqlx::query("DELETE FROM klasifikasi_feedback WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok().json(serde_json::json!({"deleted": true})),
        Ok(_) => HttpResponse::NotFound().json(ErrorResponse {
            error: "Feedback tidak ditemukan.".into(),
            retry_after_secs: None,
        }),
        Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Gagal menghapus feedback: {e}"),
            retry_after_secs: None,
        }),
    }
}

/// Pencarian kode untuk dropdown koreksi.
async fn codes_search(
    state: web::Data<AppState>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    // Terbuka untuk semua (dipakai form koreksi yang tidak wajib login untuk mencari kode)
    let q = query.get("q").cloned().unwrap_or_default();
    if q.trim().len() < 2 {
        return HttpResponse::Ok().json(serde_json::json!([]));
    }
    match feedback::search_codes(&state.db, &q).await {
        Ok(rows) => HttpResponse::Ok().json(
            rows.iter()
                .map(|(k, d, p)| serde_json::json!({ "kode": k, "deskripsi": d, "path": p }))
                .collect::<Vec<_>>(),
        ),
        Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Gagal cari kode: {e}"),
            retry_after_secs: None,
        }),
    }
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

    // Pencarian semantic via pgvector (PostgreSQL) — satu-satunya search backend.
    // Data: tabel klasifikasi_embedding (embedding 768 dimensi, cosine similarity).
    println!("🔎 Search backend: pgvector (PostgreSQL)");

    // Konfigurasi kuota free (RPM/RPD) — mencegah 429 secara proaktif.
    // Default konservatif free tier: gemini-2.5-flash 10 RPM / 250 RPD,
    // embedding 100 RPM / 10.000 RPD. Sesuaikan via .env bila limit Anda beda.
    let quota_enabled = !matches!(env::var("QUOTA_ENABLED").as_deref(), Ok("0") | Ok("false") | Ok("no"));
    let quota_chat_rpm = env::var("QUOTA_CHAT_RPM").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let quota_chat_rpd = env::var("QUOTA_CHAT_RPD").ok().and_then(|v| v.parse().ok()).unwrap_or(250);
    let quota_embed_rpm = env::var("QUOTA_EMBED_RPM").ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let quota_embed_rpd = env::var("QUOTA_EMBED_RPD").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000);
    if quota_enabled {
        println!("📊 Kuota free aktif: chat {} RPM / {} RPD · embed {} RPM / {} RPD",
            quota_chat_rpm, quota_chat_rpd, quota_embed_rpm, quota_embed_rpd);
    } else {
        println!("📊 Kuota free NONAKTIF (QUOTA_ENABLED=0)");
    }

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    let key_rotator = Arc::new(KeyRotator::new(keys));
    let rate_limit_interval = Duration::from_secs(MIN_REQUEST_INTERVAL.as_secs());

    // Konfigurasi autentikasi Google (nonaktif bila GOOGLE_CLIENT_ID kosong)
    let auth_cfg = auth::AuthConfig::from_env();
    if auth_cfg.enabled {
        println!("🔐 Auth Google AKTIF (redirect: {})", auth_cfg.redirect_uris.join(", "));
        if auth_cfg.uses_default_secret() {
            eprintln!("⚠️  WARNING: JWT_SECRET masih default! Set JWT_SECRET di .env untuk produksi.");
        }
    } else {
        println!("🔐 Auth Google NONAKTIF (isi GOOGLE_CLIENT_ID/GOOGLE_CLIENT_SECRET untuk mengaktifkan login)");
    }

    println!("Server running on http://{}:{}", host, port);

    // Konfigurasi admin: email admin (boleh banyak, comma-separated) + password
    // secret wajib untuk fitur hapus feedback.
    let admin_emails: Vec<String> = env::var("ADMIN_EMAILS")
        .unwrap_or_default()
        .split(',')
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty() && e.contains('@'))
        .collect();
    let delete_secret = env::var("DELETE_SECRET").unwrap_or_default();
    if admin_emails.is_empty() {
        eprintln!("⚠️  WARNING: ADMIN_EMAILS kosong — fitur hapus feedback nonaktif untuk semua user.");
    } else {
        println!("👑 Admin feedback: {}", admin_emails.join(", "));
    }
    if delete_secret.is_empty() {
        eprintln!("⚠️  WARNING: DELETE_SECRET kosong — hapus feedback ditolak sampai DELETE_SECRET diisi.");
    } else {
        println!("🔒 DELETE_SECRET terkonfigurasi ({} karakter)", delete_secret.len());
    }
    // Pengaman anti brute-force hapus feedback (default: 5 gagal → lockout 15 menit)
    let delete_max_attempts = env::var("DELETE_MAX_ATTEMPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let delete_lockout_secs = env::var("DELETE_LOCKOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15 * 60);
    let delete_guard = DeleteGuard::new(delete_max_attempts, delete_lockout_secs);
    if !delete_secret.is_empty() {
        println!("🛡️  Anti brute-force hapus: {delete_max_attempts} percobaan gagal → lockout {delete_lockout_secs} detik");
    }

    let quota = quota::Quota::new(quota_enabled, quota_chat_rpm, quota_chat_rpd, quota_embed_rpm, quota_embed_rpd);

    let state = web::Data::new(AppState {
        db,
        key_rotator,
        last_request: std::sync::Mutex::new(std::time::Instant::now() - MIN_REQUEST_INTERVAL),
        rate_limit_interval,
        quota,
        auth: auth_cfg,
        admin_emails,
        delete_secret,
        delete_guard,
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
            .route("/api/extract-pdf", web::post().to(extract_pdf))
            .route("/api/quota", web::get().to(quota_info))
            .route("/api/auth/config", web::get().to(auth_config))
            .route("/api/auth/google", web::post().to(auth_google))
            .route("/api/me", web::get().to(me))
            .route("/api/feedback", web::post().to(submit_feedback))
            .route("/api/feedback/stats", web::get().to(feedback_stats))
            .route("/api/feedback/{id}", web::delete().to(delete_feedback))
            .route("/api/codes", web::get().to(codes_search))
    })
    .bind((host.as_str(), port))?
    .run()
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_guard_mengunci_setelah_n_gagal() {
        let g = DeleteGuard::new(3, 900);
        assert!(g.check("a@x").is_ok());
        assert!(g.record_fail("a@x").is_none());
        assert!(g.record_fail("a@x").is_none());
        assert_eq!(g.record_fail("a@x"), Some(900)); // percobaan ke-3 → kunci
        assert!(g.check("a@x").is_err()); // email ini terkunci
        assert!(g.check("b@x").is_ok()); // email lain tidak terpengaruh
    }

    #[test]
    fn delete_guard_sukses_mereset_counter() {
        let g = DeleteGuard::new(3, 900);
        g.record_fail("a@x");
        g.record_fail("a@x");
        g.record_success("a@x");
        // Counter di-reset → butuh 3 gagal lagi untuk kunci
        assert!(g.record_fail("a@x").is_none());
        assert!(g.record_fail("a@x").is_none());
        assert_eq!(g.record_fail("a@x"), Some(900));
    }

    #[test]
    fn delete_guard_lockout_kedaluwarsa() {
        let g = DeleteGuard::new(3, 1); // lockout 1 detik, butuh 3 gagal
        g.record_fail("a@x");
        g.record_fail("a@x");
        assert_eq!(g.record_fail("a@x"), Some(1)); // ke-3 → kunci
        assert!(g.check("a@x").is_err());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(g.check("a@x").is_ok()); // lockout lewat
        // Counter di-reset → butuh 3 gagal lagi untuk kunci
        assert!(g.record_fail("a@x").is_none());
        assert!(g.record_fail("a@x").is_none());
        assert_eq!(g.record_fail("a@x"), Some(1));
    }
}
