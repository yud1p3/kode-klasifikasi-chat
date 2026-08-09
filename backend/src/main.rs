use actix_cors::Cors;
use actix_multipart::Multipart;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, middleware};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc;
use std::time::Duration;

mod auth;
mod dikecualikan;
mod feedback;
mod gemini;
mod key_rotator;
mod quota;
mod search;

use key_rotator::KeyRotator;

const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(10);

/// Ambang similarity PERIHAL (embedding feedback vs query) — sesuai kesepakatan
/// "hanya memilih perihal yang mirip". Dipakai untuk:
/// 1) menyertakan contoh ke few-shot select_fungsi, dan
/// 2) meng-inject kode_terbaik-nya ke daftar kandidat rerank.
/// Kalibrasi: feedback relevan 0.88–1.00; agak mirip ~0.72; tidak mirip
/// 0.61–0.66 → ambang 0.70 menyeimbangkan keduanya.
const FEWSHOT_PERIHAL_SIM_THRESHOLD: f64 = 0.70;

/// Ambang similarity KODE DATASET vs query untuk injeksi ke daftar kandidat
/// (lapisan kedua setelah FEWSHOT_PERIHAL_SIM_THRESHOLD). Kandidat top-10
/// relevan biasanya 0.65–0.85; kode tak berhubungan umumnya di bawah 0.5.
const FEWSHOT_INJECT_SIM_THRESHOLD: f64 = 0.5;

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
    /// Minta ringkasan naskah (isi ringkas) — HANYA dipakai Chrome extension SRIKANDI.
    /// Versi web tidak mengirim ini sehingga respons tetap tanpa `ringkasan` (perilaku tidak berubah).
    #[serde(default)]
    include_ringkasan: bool,
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

/// Pilih nama tampilan feedback: nama SRIKANDI (dari extension) bila ada & tidak
/// kosong, fallback ke nama Google. Normalisasi: trim + batasi 100 karakter.
fn display_name(srikandi_name: Option<&str>, google_name: Option<&str>) -> Option<String> {
    let s = srikandi_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(100).collect());
    let g = google_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(100).collect());
    s.or(g)
}

#[derive(Debug, Serialize, Clone)]
struct ClassificationResult {
    id: i32,
    kode: String,
    deskripsi: String,
    path: String,
    similarity: f64,
    /// Metadata SKKAD (kolom baru di klasifikasi_embedding). Opsional: NULL
    /// bila record tidak punya data skkad. Diteruskan ke UI (web & extension).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retensi_aktif: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retensi_inaktif: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    penyusutan_akhir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    klasifikasi_keamanan: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    results: Vec<ClassificationResult>,
    perihal: String,
    explanation: String,
    /// Ringkasan naskah (isi ringkas) — opsional; hanya muncul bila diminta (include_ringkasan=true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ringkasan: Option<String>,
    /// Perihal inti (bersih, tanpa nama/tempat/waktu) hasil select_fungsi.
    /// Diteruskan ke klien agar dipakai saat submit feedback — embedding feedback
    /// tetap memakai perihal_inti tanpa perlu memanggil Gemini lagi.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    perihal_inti: Option<String>,
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
    /// Daftar kode klasifikasi berklasifikasi keamanan SENSITIF per SKKAD
    /// (Rahasia / Sangat Rahasia / Terbatas), dimuat dari DB saat startup.
    /// Dipakai lapisan guard tambahan: kode yang tertulis di dalam naskah
    /// dicocokkan ke daftar ini (deteksi deterministik, tanpa AI).
    kode_sensitif: std::collections::HashSet<String>,
}

impl AppState {
    fn is_admin(&self, email: &str) -> bool {
        self.admin_emails.iter().any(|e| e.eq_ignore_ascii_case(email.trim()))
    }
}

/// Muat daftar kode berklasifikasi keamanan sensitif dari DB (kolom
/// klasifikasi_keamanan: Rahasia/Sangat Rahasia/Terbatas). Dipakai guard
/// deterministik sebelum teks dikirim ke Gemini. Bila query gagal, server
/// TETAP berjalan (guard berbasis teks tetap aktif; lapisan kode nonaktif).
async fn load_kode_sensitif(db: &PgPool) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    match sqlx::query_scalar::<_, String>(
        "SELECT kode FROM klasifikasi_embedding
         WHERE klasifikasi_keamanan IN ('Rahasia', 'Sangat Rahasia', 'Terbatas')
           AND kode IS NOT NULL AND kode <> ''",
    )
    .fetch_all(db)
    .await
    {
        Ok(rows) => {
            for k in rows {
                set.insert(k.trim().to_string());
            }
            println!("🛡️  Kode klasifikasi sensitif dimuat: {} kode (Rahasia/Sangat Rahasia/Terbatas)", set.len());
        }
        Err(e) => eprintln!("⚠️  Gagal muat daftar kode sensitif dari DB: {e} — lapisan kode nonaktif"),
    }
    set
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

/// Ekstrak baris "Perihal:" / "Hal:" dari teks mentah (bila ada) sebagai query
/// retrieval few-shot fungsi — jauh lebih selaras dengan perihal feedback daripada
/// seluruh teks mentah (kop/footer bisa mendominasi embedding). Fallback: 3000
/// karakter pertama teks (tanpa mengubah perilaku lama).
fn perihal_mentah(message: &str) -> String {
    for line in message.lines().take(80) {
        let l = line.trim();
        let lower = l.to_lowercase();
        let pos = if lower.starts_with("perihal:") {
            Some(8)
        } else if lower.starts_with("hal:") {
            Some(4)
        } else {
            None
        };
        if let Some(p) = pos {
            let v = l[p..].trim();
            if !v.is_empty() {
                return v.chars().take(300).collect();
            }
        }
    }
    message.chars().take(3000).collect()
}

/// Susun teks untuk embedding yang KONSISTEN dengan query chat:
/// SELALU "FUNGSI > perihal_inti" via Gemini select_fungsi, baik naskah
/// pendek maupun panjang. perihal_inti dibersihkan dari nama orang,
/// tempat/wilayah, dan keterangan waktu — sesuai struktur path dataset yang
/// tidak pernah memuat keterangan semacam itu. Dikembalikan juga
/// perihal_lengkap (detail apa adanya) untuk ditampilkan di UI & feedback.
/// Fallback ke teks asli bila gagal / rate limit.
/// Catatan: setiap pemanggilan select_fungsi menghabiskan 1 kuota chat.
async fn build_embed_query(state: &AppState, api_keys: &[String], message: &str, fewshot_fungsi: &str) -> (String, String, String) {
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
            let ff = fewshot_fungsi.to_string();
            async move { gemini::select_fungsi(&key, &msg, &df, &ff).await }
        })
        .await
    {
        Ok(((fungsi, perihal_inti, perihal_lengkap), _)) if !fungsi.is_empty() && !perihal_inti.is_empty() => {
            eprintln!("Fungsi terpilih: {fungsi} | Perihal inti: {perihal_inti}");
            state.quota.record_chat(1);
            (format!("{} > {}", fungsi, perihal_inti), perihal_lengkap, perihal_inti)
        }
        Ok(((_fungsi, _perihal_inti, perihal_lengkap), _)) => {
            // Gemini tetap dipanggil (kuota terpakai) meski field inti kosong
            state.quota.record_chat(1);
            eprintln!("select_fungsi: field inti kosong, pakai teks asli");
            (message.to_string(), perihal_lengkap, String::new())
        }
        Err(e) => {
            eprintln!("select_fungsi gagal, pakai teks asli: {e}");
            (message.to_string(), String::new(), String::new())
        }
    }
}

/// Susun teks embedding feedback TANPA memanggil select_fungsi (Gemini): fungsi
/// diambil dari level-1 PATH kode yang dikonfirmasi/dikoreksi arsiparis
/// (deterministik, gratis); perihal diprioritaskan perihal_inti (hasil chat,
/// sudah bersih) → perihal lengkap → baris "Perihal:"/"Hal:" → teks mentah.
/// Menghemat 1 chat (select_fungsi) + 1 embed (few-shot) per feedback, namun
/// tetap berada di ruang "FUNGSI > perihal" yang sama dengan query chat.
/// Bila kode tidak ditemukan di dataset → fallback teks mentah (perilaku lama).
async fn embed_text_feedback(state: &AppState, kode: &str, perihal_inti: &str, perihal: &str, msg: &str) -> String {
    let fungsi = match feedback::lookup_kode(&state.db, kode).await {
        Ok(Some((_d, path))) => path.split('>').next().map(str::trim).unwrap_or("").to_string(),
        _ => String::new(),
    };
    if !fungsi.is_empty() {
        let p = if !perihal_inti.trim().is_empty() {
            perihal_inti.trim().to_string()
        } else if !perihal.trim().is_empty() {
            perihal.trim().to_string()
        } else {
            let m = perihal_mentah(msg);
            if m.chars().count() > 5 {
                m
            } else {
                msg.chars().take(3000).collect()
            }
        };
        return format!("{} > {}", fungsi, p.chars().take(300).collect::<String>());
    }
    msg.chars().take(3000).collect()
}

/// Hitung teks few-shot untuk select_fungsi: embed perihal mentah (baris
/// "Perihal:"/"Hal:" bila ada) → cari feedback dengan perihal mirip → filter
/// ambang FEWSHOT_PERIHAL_SIM_THRESHOLD → format ringkas. HANYA dipakai jalur
/// chat (jalur submit feedback menyusun embedding tanpa Gemini — lihat
/// embed_text_feedback). Bila gagal / tak ada yang mirip → String kosong
/// (perilaku tanpa panduan).
async fn fewshot_fungsi_text(state: &AppState, api_keys: &[String], message: &str) -> String {
    let raw_trunc = perihal_mentah(message);
    match state
        .key_rotator
        .try_all_prefer(api_keys, |key| {
            let t = raw_trunc.clone();
            async move { gemini::embed_text(&key, &t).await }
        })
        .await
    {
        Ok((emb, _)) => {
            state.quota.record_embed(1);
            match feedback::fetch_fewshot(&state.db, &emb).await {
                Ok(ex) => {
                    let mirip: Vec<feedback::FewShotExample> = ex
                        .into_iter()
                        .filter(|e| e.similarity >= FEWSHOT_PERIHAL_SIM_THRESHOLD)
                        .collect();
                    if !mirip.is_empty() {
                        let kodes: Vec<&str> = mirip.iter().map(|e| e.kode_terbaik.as_str()).collect();
                        eprintln!("🌐 Few-shot fungsi: {} contoh perihal mirip: {}", mirip.len(), kodes.join(", "));
                    }
                    feedback::format_fewshot_fungsi(&mirip)
                }
                Err(e) => {
                    eprintln!("fewshot fungsi fetch error: {e}");
                    String::new()
                }
            }
        }
        Err(e) => {
            eprintln!("embed few-shot fungsi gagal (dilewati): {e}");
            String::new()
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
    // Pengaman: tolak naskah berindikasi informasi yang dikecualikan / berlabel
    // rahasia SEBELUM menyentuh Gemini (deteksi deterministik, tanpa AI).
    // Berlaku untuk semua klien (web & extension).
    // Lapis 1: aturan teks (label, frasa, NIK massal).
    // Lapis 2: kode klasifikasi sensitif per SKKAD yang tertulis di dalam naskah.
    let mut alasan = dikecualikan::deteksi(message);
    alasan.extend(dikecualikan::deteksi_kode(message, &state.kode_sensitif));
    if !alasan.is_empty() {
        eprintln!("🚫 Chat dibatalkan (informasi dikecualikan): {}", alasan.join("; "));
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: format!(
                "Demi keamanan, analisa dibatalkan. Naskah ini terdeteksi mengandung informasi yang dikecualikan: {}. Jangan kirim naskah rahasia atau naskah berisi informasi yang dikecualikan (Pasal 17 UU No. 14/2008) ke layanan AI.",
                alasan.join("; ")
            ),
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
    // Estimasi call: 2 chat (select_fungsi + rerank) + 2 embed
    // (teks mentah utk few-shot fungsi + query "FUNGSI > perihal_inti").
    let chat_calls_estimate: u32 = 2;
    if let Err((wait, why)) = state.quota.check(chat_calls_estimate, 2) {
        eprintln!("⏳ Kuota free block: {why}, tunggu {wait} detik");
        return HttpResponse::TooManyRequests().json(ErrorResponse {
            error: format!(
                "Kuota free terpakai habis ({why}). Mohon tunggu {wait} detik sebelum mencoba lagi."
            ),
            retry_after_secs: Some(wait),
        });
    }

    // Few-shot untuk select_fungsi (lihat fewshot_fungsi_text): pemilihan FUNGSI
    // ikut terpandu validasi arsiparis. Bila gagal / tak ada yang mirip → dilewati.
    let fewshot_fungsi = fewshot_fungsi_text(&state, &keys, message).await;

    // Susun teks query embedding — dipakai juga saat menyimpan feedback
    // (build_embed_query) agar few-shot dicocokkan dalam ruang embedding yang sama.
    let (embed_query, perihal_lengkap, perihal_inti) = build_embed_query(&state, &keys, message, &fewshot_fungsi).await;

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
    let fewshot_examples = match feedback::fetch_fewshot(&state.db, &embedding).await {
        Ok(examples) => examples,
        Err(e) => {
            eprintln!("fewshot fetch error: {e}");
            Vec::new()
        }
    };
    let fewshot_text = feedback::format_fewshot(&fewshot_examples);

    // Pencarian semantic: pgvector (PostgreSQL) — satu-satunya backend search
    let mut results = match search::similarity_search(&state.db, &embedding, 10).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Search error: {e}");
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Gagal pencarian: {e}"),
                retry_after_secs: None,
            });
        }
    };

    // Injeksi few-shot: kode yang dikonfirmasi/dikoreksi arsiparis (kode_terbaik)
    // yang TIDAK lolos top-N pencarian tetap dimasukkan ke daftar kandidat (dengan
    // deskripsi & path asli dari dataset) bila cukup relevan dengan query. Tanpa
    // ini, rerank hanya bisa memilih dari top-N semantic — sehingga feedback baru
    // efektif bila kode hasil koreksinya kebetulan masuk kolam kandidat.
    {
        let existing: HashSet<String> = results.iter().map(|r| r.kode.clone()).collect();
        let mut ingin: Vec<String> = Vec::new();
        for ex in &fewshot_examples {
            let k = ex.kode_terbaik.trim();
            if !k.is_empty()
                && ex.similarity >= FEWSHOT_PERIHAL_SIM_THRESHOLD
                && !existing.contains(k)
                && !ingin.iter().any(|x| x == k)
            {
                ingin.push(k.to_string());
            }
        }
        if !ingin.is_empty() {
            match search::fetch_by_kodes(&state.db, &embedding, &ingin).await {
                Ok(extra) => {
                    let mut injected: Vec<String> = Vec::new();
                    for r in extra {
                        if r.similarity >= FEWSHOT_INJECT_SIM_THRESHOLD {
                            injected.push(r.kode.clone());
                            results.push(r);
                        }
                    }
                    if !injected.is_empty() {
                        eprintln!("🔁 Few-shot inject: kode arsiparis ditambahkan ke kandidat: {}", injected.join(", "));
                    }
                }
                Err(e) => eprintln!("fewshot inject error: {e}"),
            }
        }
    }

    // Ringkasan (isi ringkas) hanya diminta oleh Chrome extension SRIKANDI
    let need_ring = body.include_ringkasan;
    let (reranked, explanation, perihal_rerank, ringkasan) = match state.key_rotator.try_all_prefer(&keys, |key| {
        let msg = message.to_string();
        let fs = fewshot_text.clone();
        let res = results.clone();
        // Perihal tampilan (perihal_lengkap) diteruskan agar penjelasan
        // "Perihal: X" konsisten dengan yang ditampilkan di UI.
        let ph = perihal_lengkap.clone();
        async move { gemini::rerank_and_explain(&key, &msg, &fs, &ph, need_ring, &res).await }
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

    // Ringkasan opsional: None bila kosong (biar respons web tidak berubah)
    let ringkasan = if ringkasan.trim().is_empty() {
        None
    } else {
        Some(ringkasan.trim().to_string())
    };

    let perihal_inti = if perihal_inti.trim().is_empty() {
        None
    } else {
        Some(perihal_inti.trim().to_string())
    };
    HttpResponse::Ok().json(ChatResponse { results: reranked, perihal, explanation, ringkasan, perihal_inti })
}


/// Hapus pola penanda halaman "N / N" berulang yang menempel di AKHIR teks
/// (sisa footer TCPDF, mis. "...BSSN).  1 / 1 1 / 1"). Footer selalu di ujung
/// dokumen, jadi hanya menyentuh suffix — tidak mengganggu substansi naskah.
fn strip_trailing_page_markers(mut s: String) -> String {
    loop {
        let t = s.trim_end();
        let b = t.as_bytes();
        let n = b.len();
        // Pola akhir: ... <digit> ' ' '/' ' ' <digit>
        if n >= 5 && b[n - 1].is_ascii_digit() && b[n - 2] == b' ' && b[n - 3] == b'/'
            && b[n - 4] == b' ' && b[n - 5].is_ascii_digit()
        {
            s = t[..n - 5].trim_end().to_string();
        } else {
            break;
        }
    }
    s
}

/// Bersihkan boilerplate footer TCPDF (www.tcpdf.org) yang kadang dobel di
/// naskah SRIKANDI (mis. "Powered by TCPDF ... 1 / 1 1 / 1"). Hanya menyentuh
/// footer, bukan substansi naskah.
fn bersihkan_footer_tcpdf(s: &str) -> String {
    let tanpa_tcpdf = s.replace("Powered by TCPDF (www.tcpdf.org)", "");
    strip_trailing_page_markers(tanpa_tcpdf)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Ekstrak teks PDF via pdf-inspector (crate Rust in-process, tanpa poppler).
/// Output GFM Markdown terstruktur (heading, tabel) — lebih baik untuk AI
/// daripada teks polos; tetap dikembalikan sebagai `{text}` agar kontrak API
/// dengan extension TIDAK berubah.
/// Dipakai sebagai pengganti anydoc (yang membungkus pdf-inspector untuk PDF)
/// — hasil IDENTIK karena anydoc mendelegasikan 100% pemrosesan PDF ke
/// pdf-inspector, namun tanpa membawa parser docx/xls/pptx yang tidak kita
/// pakai (~147 paket dependensi). Terbukti membaca PDF SRIKANDI bertanda
/// tangan elektronik dengan kualitas setara, cepat, tanpa dependensi sistem
/// (`poppler-utils`), dan tanpa spawn proses.
async fn extract_pdf(mut payload: Multipart) -> HttpResponse {
    use actix_web::web::BytesMut;
    use futures::StreamExt;

    // Terbuka untuk semua (bagian dari alur chat tanpa login).
    // Baca seluruh bytes multipart ke memory — pdf-inspector bekerja in-memory,
    // tanpa temp file (tidak seperti pdftotext yang butuh path file).
    // Batas ukuran 20 MB: lindungi memory dari upload berlebihan (PDF naskah
    // dinas jarang melewati beberapa MB).
    const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
    let mut buf = BytesMut::new();
    while let Some(Ok(mut field)) = payload.next().await {
        while let Some(Ok(chunk)) = field.next().await {
            buf.extend_from_slice(&chunk);
            if buf.len() > MAX_UPLOAD_BYTES {
                return HttpResponse::PayloadTooLarge().json(serde_json::json!({
                    "error": "Ukuran file melebihi batas 20 MB."
                }));
            }
        }
    }
    if buf.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({"error": "File kosong"}));
    }

    // Parse PDF di thread pool (web::block): parse sinkron pdf-inspector tidak
    // memblokir runtime async Actix, dan panic di library pihak ketiga
    // (pdf-inspector masih v0.1.x) tidak menjatuhkan seluruh server.
    let bytes = buf.to_vec();
    // web::block mengembalikan Result<Result<PagesExtractionResult, PdfError>, BlockingError>:
    // lapisan luar = status thread pool (panic/join), lapisan dalam = hasil pdf-inspector.
    match web::block(move || pdf_inspector::extract_pages_markdown_mem(&bytes, None)).await {
        Ok(Ok(res)) => {
            // Gabung markdown per halaman (halaman scan → markdown kosong)
            let md = res
                .pages
                .iter()
                .map(|p| p.markdown.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let text = bersihkan_footer_tcpdf(&md);
            if text.is_empty() {
                // Semua halaman tanpa teks: PDF scan (image-only) atau format tak dikenal
                HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Tidak ada teks yang bisa diekstrak dari PDF. Bila ini PDF hasil scan, tidak ada lapisan teks yang bisa dibaca — unggah naskah digital asli atau file DOCX."
                }))
            } else {
                HttpResponse::Ok().json(serde_json::json!({"text": text}))
            }
        }
        Ok(Err(e)) => match e {
            // File bukan PDF / format tidak dikenali
            pdf_inspector::PdfError::NotAPdf(hint) => HttpResponse::BadRequest().json(serde_json::json!({
                "error": format!("File yang diunggah bukan PDF atau formatnya tidak dikenali ({hint}).")
            })),
            // PDF terenkripsi / berpassword
            pdf_inspector::PdfError::Encrypted => HttpResponse::BadRequest().json(serde_json::json!({
                "error": "PDF terenkripsi atau diproteksi kata sandi. Buka proteksinya terlebih dahulu, lalu unggah ulang."
            })),
            // PDF rusak / struktur tidak valid — user bisa coba file lain
            pdf_inspector::PdfError::Parse(msg) => HttpResponse::BadRequest().json(serde_json::json!({
                "error": format!("PDF rusak atau tidak valid: {msg}. Coba buka dengan aplikasi PDF lalu simpan ulang.")
            })),
            pdf_inspector::PdfError::InvalidStructure => HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Struktur PDF tidak valid. Coba buka dengan aplikasi PDF lalu simpan ulang, atau gunakan file lain."
            })),
            // Error IO / lainnya: tak terduga
            other => HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Gagal ekstrak PDF: {other}")
            })),
        },
        Err(block_err) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Terjadi kesalahan internal saat memproses PDF: {block_err}")
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
    // Penting: bila redirect_uri DIKIRIM tapi TIDAK diizinkan, TOLAK dengan pesan
    // jelas — jangan fallback senyap ke URI lain. Google memvalidasi redirect_uri
    // saat exchange code (harus SAMA dengan yang dipakai membangun URL login),
    // jadi mengganti diam-diam memicu error 'redirect_uri_mismatch' yang
    // menyesatkan (mis. extension Chrome mengirim https://<id>.chromiumapp.org/).
    let redirect_uri = match &body.redirect_uri {
        Some(uri) if !uri.trim().is_empty() && !state.auth.is_allowed_redirect(uri) => {
            eprintln!(
                "🔐 Login ditolak: redirect_uri tidak diizinkan: {uri:?} | diizinkan: {:?}",
                state.auth.redirect_uris
            );
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: format!(
                    "redirect_uri tidak terdaftar di server: {uri}. Pastikan URI ini ada di GOOGLE_REDIRECT_URI backend dan Google Cloud Console (Authorized redirect URIs)."
                ),
                retry_after_secs: None,
            });
        }
        _ => body
            .redirect_uri
            .clone()
            .filter(|u| state.auth.is_allowed_redirect(u))
            .unwrap_or_else(|| state.auth.redirect_uris.first().cloned().unwrap_or_default()),
    };
    eprintln!(
        "🔐 Login Google: client_id {}…, redirect_uri dipakai: {redirect_uri}",
        state.auth.client_id.chars().take(12).collect::<String>()
    );
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
    /// Perihal inti (bersih, tanpa nama/tempat/waktu) hasil select_fungsi saat chat.
    /// Dipakai menyusun embedding feedback tanpa memanggil Gemini lagi.
    #[serde(default)]
    perihal_inti: Option<String>,
    /// Nama lengkap pengguna SRIKANDI (di-scrape extension dari halaman SRIKANDI).
    /// Dipakai sebagai nama tampilan feedback dari extension; identitas login
    /// Google tetap tercatat terpisah di user_sub/user_email bila user login.
    #[serde(default)]
    user_name: Option<String>,
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
    // Pengaman: tolak feedback berindikasi informasi yang dikecualikan — feedback
    // juga memproses naskah via Gemini (few-shot & validasi koreksi).
    // Lapis 1: aturan teks. Lapis 2: kode klasifikasi sensitif per SKKAD.
    let mut alasan = dikecualikan::deteksi(msg);
    alasan.extend(dikecualikan::deteksi_kode(msg, &state.kode_sensitif));
    if !alasan.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: format!(
                "Demi keamanan, feedback dibatalkan. Naskah ini terdeteksi mengandung informasi yang dikecualikan: {}",
                alasan.join("; ")
            ),
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

    // Perihal inti (bersih) dari hasil chat — dipakai embedding feedback (prioritas).
    let perihal_inti: String = body
        .perihal_inti
        .clone()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(300)
        .collect();

    // Feedback positif: TANPA WAJIB LOGIN — dicatat anonim bila user tidak
    // login; identitas tetap tercatat bila user kebetulan sedang login.
    // Nama tampilan: nama SRIKANDI (dari extension) bila ada, fallback nama Google.
    if ftype == "positive" {
        let user = auth::optional_user(&req, &state.auth);
        let (sub, email, name) = match &user {
            Some(u) => (Some(u.sub.clone()), Some(u.email.clone()), Some(u.name.clone())),
            None => (None, None, None),
        };
        let name = display_name(body.user_name.as_deref(), name.as_deref());

        // Embedding naskah (best-effort) — dipakai few-shot untuk query serupa.
        // Disusun TANPA memanggil select_fungsi (hemat 1 chat + 1 embed): fungsi
        // dari level-1 path kode yang dikonfirmasi, perihal_inti dari hasil chat
        // (bila ada). Tetap di ruang "FUNGSI > perihal" yang sama dengan query.
        // Pengaman: cek kuota proaktif (branch ini anonim & tanpa rate limiter).
        // Bila kuota tidak cukup, embedding dilewati — feedback TETAP tersimpan
        // (embedding NULL = tidak muncul di few-shot, tidak fatal).
        let mut emb_store: Option<String> = None;
        if state.quota.check(0, 1).is_ok() {
            let embed_text = embed_text_feedback(&state, &body.kode_ai, &perihal_inti, &perihal, msg).await;
            if let Ok((emb, _)) = state
                .key_rotator
                .try_all_prefer(&keys, |key| {
                    let t = embed_text.clone();
                    async move { gemini::embed_text(&key, &t).await }
                })
                .await
            {
                state.quota.record_embed(1);
                emb_store = Some(format!(
                    "[{}]",
                    emb.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
                ));
            }
        }

        let res = sqlx::query(
            "INSERT INTO klasifikasi_feedback (naskah, kode_ai, feedback_type, status, kode_terbaik, perihal, user_sub, user_email, user_name, chat_id, embedding)
             VALUES ($1, $2, 'positive', 'validated', $3, $4, $5, $6, $7, $8,
                     CASE WHEN $9::text IS NULL THEN NULL ELSE $9::vector END)",
        )
        .bind(&naskah_store)
        .bind(&body.kode_ai)
        .bind(&body.kode_ai)
        .bind(&perihal)
        .bind(sub)
        .bind(email)
        .bind(name)
        .bind(chat_id)
        .bind(emb_store.as_deref())
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
    // Nama tampilan: nama SRIKANDI (dari extension) bila ada, fallback nama Google.
    // Identitas login tetap terlacak di user_sub/user_email.
    let fb_display_name = display_name(body.user_name.as_deref(), Some(&user.name));

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
    // Estimasi: koreksi = 1 chat (validasi Gemini) + 1 embed (query feedback),
    // karena embedding feedback disusun tanpa select_fungsi (fungsi dari path).
    let chat_calls_estimate: u32 = 1;
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
            .bind(&fb_display_name)
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
    // Disusun TANPA select_fungsi: fungsi dari level-1 path kode_terbaik,
    // perihal_inti dari hasil chat (bila ada). Ruang sama dengan query chat.
    let mut emb_store: Option<String> = None;
    if status == "validated" {
        let embed_text = embed_text_feedback(&state, &kode_terbaik, &perihal_inti, &perihal, msg).await;
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
    .bind(&fb_display_name)
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

/// Verifikasi izin hapus feedback — 4 lapis: admin (ADMIN_EMAILS), fitur aktif
/// (DELETE_SECRET), anti brute-force (DeleteGuard), dan password secret
/// (constant-time). Dipakai bersama oleh hapus tunggal (DELETE /api/feedback/{id})
/// dan hapus massal (POST /api/feedback/bulk-delete) agar lapisan keamanan
/// identik di kedua jalur.
/// Mengembalikan Err(HttpResponse) bila ditolak (response siap kirim);
/// Ok(()) bila lolos semua lapisan.
async fn verify_delete_allowed(
    state: &web::Data<AppState>,
    req: &HttpRequest,
    password: &str,
) -> Result<(), HttpResponse> {
    let user = match auth::require_user(req, &state.auth) {
        Ok(u) => u,
        Err(r) => return Err(r),
    };
    // Lapis 1: hanya admin yang boleh menghapus
    if !state.is_admin(&user.email) {
        return Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "Hanya admin yang dapat menghapus feedback.".into(),
            retry_after_secs: None,
        }));
    }
    // Lapis 2: fitur hapus harus dikonfigurasi (DELETE_SECRET)
    if state.delete_secret.is_empty() {
        return Err(HttpResponse::ServiceUnavailable().json(ErrorResponse {
            error: "Fitur hapus feedback nonaktif: DELETE_SECRET belum dikonfigurasi di server.".into(),
            retry_after_secs: None,
        }));
    }
    // Lapis 3: anti brute-force — blokir bila email ini sedang terkunci
    if let Err(wait) = state.delete_guard.check(&user.email) {
        return Err(HttpResponse::TooManyRequests().json(ErrorResponse {
            error: format!(
                "Terlalu banyak percobaan password yang gagal. Tunggu {wait} detik sebelum mencoba lagi."
            ),
            retry_after_secs: Some(wait),
        }));
    }
    // Lapis 4: verifikasi password secret (constant-time)
    if !constant_time_eq(&state.delete_secret, password) {
        // Catat kegagalan; bila batas tercapai → kunci email ini
        if let Some(wait) = state.delete_guard.record_fail(&user.email) {
            let dur = if wait >= 60 {
                format!("{:.0} menit", wait as f64 / 60.0)
            } else {
                format!("{wait} detik")
            };
            eprintln!("🔒 Admin {} terkunci selama {dur} ({} percobaan gagal)", user.email, state.delete_guard.max_attempts);
            return Err(HttpResponse::TooManyRequests().json(ErrorResponse {
                error: format!(
                    "Terlalu banyak percobaan password ({}) — terkunci selama {dur}.",
                    state.delete_guard.max_attempts
                ),
                retry_after_secs: Some(wait),
            }));
        }
        return Err(HttpResponse::Forbidden().json(ErrorResponse {
            error: "Password secret salah.".into(),
            retry_after_secs: None,
        }));
    }
    state.delete_guard.record_success(&user.email);
    Ok(())
}

/// Hapus feedback tunggal — hanya admin (ADMIN_EMAILS) dengan password secret (DELETE_SECRET).
async fn delete_feedback(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<HashMap<String, String>>,
) -> HttpResponse {
    let password = body.get("password").cloned().unwrap_or_default();
    if let Err(resp) = verify_delete_allowed(&state, &req, &password).await {
        return resp;
    }
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

/// Payload hapus massal feedback — 3 mode (salah satu wajib diisi):
/// - ids:    ID feedback spesifik (checkbox multi-select di dashboard)
/// - status/perihal: hapus SEMUA feedback yang cocok filter (sama seperti
///   filter statistik: status divalidasi whitelist, perihal di-escape kutip)
/// - all:    hapus seluruh feedback
#[derive(Debug, Deserialize)]
struct BulkDeleteFeedbackRequest {
    password: String,
    #[serde(default)]
    ids: Vec<i64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    perihal: Option<String>,
    #[serde(default)]
    all: bool,
}

/// Hapus feedback secara massal — proteksi 4 lapis SAMA seperti hapus tunggal
/// (admin + DELETE_SECRET + anti brute-force + password constant-time), via
/// verify_delete_allowed. SQL dibangun dinamis namun aman: ids bertipe i64
/// (bind parameter), status di-whitelist, perihal di-escape kutip tunggal
/// (pola yang sama dengan fetch_stats).
async fn bulk_delete_feedback(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<BulkDeleteFeedbackRequest>,
) -> HttpResponse {
    if let Err(resp) = verify_delete_allowed(&state, &req, &body.password).await {
        return resp;
    }

    // Batasi jumlah ID per request (cegah abuse); buang ID <= 0
    let ids: Vec<i64> = body.ids.iter().copied().filter(|i| *i > 0).take(2000).collect();
    let status = body.status.as_deref().map(str::trim).unwrap_or("").to_lowercase();
    let status_valid = matches!(status.as_str(), "validated" | "rejected" | "pending");
    let perihal = body.perihal.as_deref().map(str::trim).unwrap_or("");
    if !body.all && ids.is_empty() && !status_valid && perihal.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Tentukan kriteria hapus: ids, filter status/perihal, atau all=true.".into(),
            retry_after_secs: None,
        });
    }
    // Mode eksklusif: all=true TIDAK boleh dicampur kriteria lain (ids/filter)
    // — mencegah permintaan "hapus semua" yang tak sengaja menyertakan id.
    if body.all && (!ids.is_empty() || status_valid || !perihal.is_empty()) {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "all=true tidak boleh dikombinasikan dengan ids atau filter status/perihal.".into(),
            retry_after_secs: None,
        });
    }

    // Bangun SQL dinamis. Hanya ids yang di-bind ($1::bigint[]); status &
    // perihal disisipkan aman (whitelist / escape), konsisten dengan fetch_stats.
    let mut sql = String::from("DELETE FROM klasifikasi_feedback");
    let mut conds: Vec<String> = Vec::new();
    if !ids.is_empty() {
        conds.push("id = ANY($1::bigint[])".to_string());
    }
    if status_valid {
        conds.push(format!("status = '{status}'"));
    }
    if !perihal.is_empty() {
        let esc = perihal.replace('\'', "''");
        conds.push(format!("(perihal ILIKE '%{esc}%' OR naskah ILIKE '%{esc}%')"));
    }
    if !conds.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conds.join(" AND "));
    }

    let mut q = sqlx::query(&sql);
    let ids_len = ids.len();
    if !ids.is_empty() {
        q = q.bind(ids);
    }
    match q.execute(&state.db).await {
        Ok(r) => {
            let n = r.rows_affected();
            eprintln!("🗑️ Admin menghapus {n} feedback (massal): all={} ids={} status={} perihal={}",
                body.all, ids_len, if status_valid { &status } else { "-" }, if perihal.is_empty() { "-" } else { perihal });
            HttpResponse::Ok().json(serde_json::json!({ "deleted": n }))
        }
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

/// Daftar kode klasifikasi berklasifikasi keamanan sensitif (per SKKAD) —
/// dipakai Chrome extension untuk lapisan guard lokal SEBELUM teks dikirim
/// ke API (daftar di-cache di chrome.storage.local, fallback ke aturan teks).
async fn kode_rahasia(state: web::Data<AppState>) -> HttpResponse {
    let mut v: Vec<String> = state.kode_sensitif.iter().cloned().collect();
    v.sort();
    HttpResponse::Ok().json(serde_json::json!({
        "kode": v,
        "level": ["Rahasia", "Sangat Rahasia", "Terbatas"],
        "total": v.len(),
    }))
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

    let kode_sensitif = load_kode_sensitif(&db).await;

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
        kode_sensitif,
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
            // Harus DIDAFTARKAN sebelum /api/feedback/{id}: "bulk-delete" tidak
            // lolos ekstraksi Path<i64>, sehingga bila terdaftar setelahnya
            // request akan kena 400 (bukan fallthrough ke route ini).
            .route("/api/feedback/bulk-delete", web::post().to(bulk_delete_feedback))
            .route("/api/feedback/{id}", web::delete().to(delete_feedback))
            .route("/api/codes", web::get().to(codes_search))
            .route("/api/dikecualikan/kode-rahasia", web::get().to(kode_rahasia))
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
    fn perihal_mentah_mengambil_baris_perihal() {
        let s = "PEMERINTAH KABUPATEN BLITAR\nDINAS PERPUSTAKAAN DAN KEARSIPAN\nPerihal: Undangan Pendampingan Pengelolaan Resiko\nisi naskah...";
        assert_eq!(perihal_mentah(s), "Undangan Pendampingan Pengelolaan Resiko");
    }

    #[test]
    fn perihal_mentah_dukung_hal_dan_fallback() {
        assert_eq!(perihal_mentah("kop surat\nHal: Permohonan cuti tahunan\nisi"), "Permohonan cuti tahunan");
        // Tanpa Perihal/Hal → fallback 3000 karakter pertama
        let tanpa = perihal_mentah("isi naskah tanpa perihal");
        assert_eq!(tanpa, "isi naskah tanpa perihal");
    }

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
    fn footer_tcpdf_dobel_dibersihkan() {
        let s = "Isi naskah\n\nDokumen ini telah ditandatangani secara elektronik menggunakan sertifikat elektronik yang diterbitkan oleh Balai Besar Sertifikasi Elektronik (BSrE), Badan Siber dan Sandi Negara (BSSN). Powered by TCPDF (www.tcpdf.org)Powered by TCPDF (www.tcpdf.org) 1 / 1 1 / 1";
        let hasil = bersihkan_footer_tcpdf(s);
        assert!(!hasil.contains("Powered by TCPDF"));
        assert!(!hasil.contains("1 / 1"));
        assert!(hasil.contains("BSSN).")); // teks sertifikat tetap utuh
    }

    #[test]
    fn footer_bersih_tidak_berubah() {
        let s = "Permohonan reset MFA akun pengguna.";
        assert_eq!(bersihkan_footer_tcpdf(s), s);
    }

    #[test]
    fn footer_strip_tidak_menyentuh_teks_awal() {
        // Penanda "N / N" di TENGAH teks (bukan akhir) tidak boleh dihapus
        let s = "Pertemuan 1 / 1 peserta hadir.";
        assert!(bersihkan_footer_tcpdf(s).contains("1 / 1"));
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
