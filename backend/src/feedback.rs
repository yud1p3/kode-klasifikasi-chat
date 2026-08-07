use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

/// Kandidat top-3 yang tadi dipertimbangkan AI (dikirim frontend dari hasil chat).
#[derive(Debug, Deserialize, Clone)]
pub struct Candidate {
    pub kode: String,
    pub deskripsi: String,
    pub path: String,
}

#[derive(Debug)]
pub struct ValidationResult {
    pub valid: bool,
    pub kode_terbaik: String,
    pub penjelasan: String,
    /// Raw JSON dari Gemini (untuk debugging)
    pub raw: String,
}

/// Cari (deskripsi, path) sebuah kode di dataset. None bila kode tidak ada.
pub async fn lookup_kode(db: &PgPool, kode: &str) -> Result<Option<(String, String)>> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT deskripsi, path FROM klasifikasi_embedding WHERE kode = $1 LIMIT 1",
    )
    .bind(kode)
    .fetch_optional(db)
    .await?;
    Ok(row)
}

/// Validasi koreksi pengguna via Gemini. Prompt menyertakan PATH lengkap untuk
/// kode AI & kode koreksi (agar AI paham konteks fungsi/urusan), plus top-3
/// kandidat yang tadi dipertimbangkan AI.
pub async fn validate_correction(
    api_key: &str,
    message: &str,
    perihal: &str,
    ai: (&str, &str, &str),      // (kode, deskripsi, path) kode AI
    kor: (&str, &str, &str),     // (kode, deskripsi, path) kode koreksi pengguna
    alasan: &str,
    candidates: &[Candidate],
) -> Result<ValidationResult> {
    let mut kandidat_text = String::new();
    for (i, c) in candidates.iter().enumerate() {
        kandidat_text.push_str(&format!(
            "{}. Kode: {} | Deskripsi: {} | Path: {}\n",
            i + 1,
            c.kode,
            c.deskripsi,
            c.path
        ));
    }
    let alasan_txt = if alasan.trim().is_empty() { "tidak ada".to_string() } else { alasan.to_string() };

    let prompt = format!(
        "Anda arsiparis ahli klasifikasi arsip dinas. Evaluasi sebuah KOREKSI klasifikasi yang diajukan pengguna.\n\n\
         ===== TEKS NASKAH =====\n\
         PERIHAL: {}\n\
         {}\n\n\
         ===== KODE YANG DIPILIH AI =====\n\
         Kode: {} | Deskripsi: {}\n\
         Path: {}\n\n\
         ===== KOREKSI PENGGUNA =====\n\
         Kode: {} | Deskripsi: {}\n\
         Path: {}\n\
         Alasan pengguna: {}\n\n\
         ===== KANDIDAT AI (top-3 hasil pencarian semantic) =====\n\
         {}\n\n\
         TUGAS:\n\
         1. Pastikan kode koreksi sesuai SUBSTANSI naskah (bukan sekadar kemiripan kata),\n\
            dengan memperhatikan FUNGSI/URUSAN pada path. Koreksi yang fungsi/urusannya\n\
            keliru atau kodenya jelas tidak berhubungan dengan naskah = TIDAK valid.\n\
         2. Bandingkan dengan kode AI dan kandidat: apakah koreksi pengguna lebih tepat,\n\
            atau setara namun lebih spesifik (posisinya lebih dalam pada hirarki)?\n\
         3. Tentukan kode terbaik final — pilih DARI daftar yang disebutkan di atas\n\
            (kode AI, kode koreksi, atau salah satu kandidat).\n\n\
         Keluarkan HANYA JSON valid (tanpa markdown code block):\n\
         {{\"valid\": true|false, \"kode_terbaik\": \"xxx.xx\", \"penjelasan\": \"1-2 kalimat, bahasa mudah dipahami arsiparis\"}}",
        if perihal.trim().is_empty() { "(tidak diketahui)".to_string() } else { perihal.trim().to_string() },
        message.chars().take(2000).collect::<String>(),
        ai.0, ai.1, ai.2,
        kor.0, kor.1, kor.2,
        alasan_txt,
        if kandidat_text.is_empty() { "(tidak ada)".to_string() } else { kandidat_text }
    );

    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key={}",
        api_key
    );
    let body = json!({
        "contents": [{ "parts": [{"text": prompt}] }],
        "generationConfig": { "temperature": 0.1, "maxOutputTokens": 8192, "thinkingConfig": { "thinkingBudget": 0 } }
    });
    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Gemini {} ({}): {}", status.as_str(), status, text.chars().take(300).collect::<String>());
    }
    let resp_json: Value = resp.json().await?;
    let raw = resp_json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("{}")
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    // Parsing toleran (tahan JSON korup)
    let parsed: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let mut valid = parsed["valid"].as_bool().unwrap_or(false);
    let mut kode_terbaik = parsed["kode_terbaik"].as_str().unwrap_or_default().to_string();
    let mut penjelasan = parsed["penjelasan"].as_str().unwrap_or_default().to_string();

    if parsed.is_null() || kode_terbaik.is_empty() || penjelasan.is_empty() {
        // Fallback ekstraksi manual
        if kode_terbaik.is_empty() {
            kode_terbaik = grab_field(raw, "kode_terbaik");
        }
        if penjelasan.is_empty() {
            penjelasan = grab_field(raw, "penjelasan");
        }
        if !raw.contains("valid") {
            valid = false;
        }
    }

    Ok(ValidationResult { valid, kode_terbaik, penjelasan, raw: raw.to_string() })
}

fn grab_field(raw: &str, key: &str) -> String {
    let needle = format!("\"{}\"", key);
    if let Some(rest) = raw.split_once(&needle).map(|(_, r)| r) {
        if let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            if let Some(end) = after.find('"') {
                return after[..end].to_string();
            }
        }
    }
    String::new()
}

/// Satu contoh koreksi tervalidasi untuk few-shot, lengkap dengan deskripsi &
/// path kode awal & kode benar (dari dataset) agar Gemini punya konteks hirarki.
pub struct FewShotExample {
    pub teks: String,       // perihal (atau potongan naskah)
    pub kode_ai: String,
    pub kode_terbaik: String,
    pub ai_deskripsi: String,
    pub ai_path: String,
    pub kb_deskripsi: String,
    pub kb_path: String,
}

/// Ambil hingga 5 koreksi tervalidasi yang NASKAHNYA PALING MIRIP dengan embedding
/// query saat ini (pgvector cosine). Dipakai sebagai few-shot di prompt rerank.
pub async fn fetch_fewshot(db: &PgPool, embedding: &[f64]) -> Result<Vec<FewShotExample>> {
    let emb_str = embedding
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let rows = sqlx::query_as::<_, (String, String, String, String, String, String, String)>(
        "SELECT COALESCE(NULLIF(f.perihal,''), LEFT(f.naskah, 120)),
                f.kode_ai, f.kode_terbaik,
                COALESCE(ai.deskripsi,''), COALESCE(ai.path,''),
                COALESCE(kb.deskripsi,''), COALESCE(kb.path,'')
         FROM klasifikasi_feedback f
         LEFT JOIN klasifikasi_embedding ai ON ai.kode = f.kode_ai
         LEFT JOIN klasifikasi_embedding kb ON kb.kode = f.kode_terbaik
         WHERE f.status = 'validated' AND f.feedback_type = 'correction'
           AND f.kode_terbaik IS NOT NULL AND f.kode_terbaik <> f.kode_ai
           AND f.embedding IS NOT NULL
         ORDER BY f.embedding <=> $1::vector
         LIMIT 5",
    )
    .bind(format!("[{}]", emb_str))
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| FewShotExample {
            teks: r.0,
            kode_ai: r.1,
            kode_terbaik: r.2,
            ai_deskripsi: r.3,
            ai_path: r.4,
            kb_deskripsi: r.5,
            kb_path: r.6,
        })
        .collect())
}

/// Format teks few-shot untuk disisipkan ke prompt rerank.
/// Menyertakan deskripsi & path kode awal & kode benar agar Gemini paham
/// konteks fungsi/urusan dari tiap contoh koreksi (dipotong agar prompt ringkas).
pub fn format_fewshot(examples: &[FewShotExample]) -> String {
    if examples.is_empty() {
        return String::new();
    }
    let mut out = String::from("===== CONTOH KOREKSI ARSIPARIS (kasus serupa sebelumnya) =====\n");
    for (i, e) in examples.iter().enumerate() {
        let teks: String = e.teks.chars().take(120).collect();
        let ai_d: String = e.ai_deskripsi.chars().take(100).collect();
        let kb_d: String = e.kb_deskripsi.chars().take(100).collect();
        let ai_p: String = e.ai_path.chars().take(200).collect();
        let kb_p: String = e.kb_path.chars().take(200).collect();
        out.push_str(&format!(
            "{}. Naskah: \"{}\". Kode awal (keliru): {} — {}. Path: {}. Kode benar setelah koreksi arsiparis: {} — {}. Path: {}.\n",
            i + 1, teks, e.kode_ai, ai_d, ai_p, e.kode_terbaik, kb_d, kb_p
        ));
    }
    out.push_str("Gunakan contoh ini sebagai panduan: bila naskah saat ini serupa dengan suatu contoh, prioritaskan kode benar yang telah dikoreksi arsiparis tersebut.\n");
    out
}

/// Bangun klausa WHERE dari filter statistik. Mengembalikan (klausa "WHERE ...",
/// klausa "AND ...") — yang kedua untuk query yang sudah punya WHERE sendiri.
/// Aman dari SQL injection: status di-whitelist, perihal di-escape kutipnya.
fn build_stats_filter(perihal: Option<&str>, status: Option<&str>) -> (String, String) {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = status {
        let s = s.trim().to_lowercase();
        if matches!(s.as_str(), "validated" | "rejected" | "pending") {
            parts.push(format!("status = '{s}'"));
        }
    }
    if let Some(p) = perihal {
        let p = p.trim();
        if !p.is_empty() {
            // Escape kutip tunggal agar aman dimasukkan ke literal SQL
            let esc = p.replace('\'', "''");
            parts.push(format!("(perihal ILIKE '%{esc}%' OR naskah ILIKE '%{esc}%')"));
        }
    }
    if parts.is_empty() {
        return (String::new(), String::new());
    }
    let joined = parts.join(" AND ");
    (format!("WHERE {joined}"), format!(" AND {joined}"))
}

/// Statistik feedback untuk UI/admin. Mendukung filter opsional perihal (kata
/// kunci pada perihal/naskah) dan status (validated|rejected|pending).
pub async fn fetch_stats(db: &PgPool, perihal: Option<&str>, status: Option<&str>) -> Result<Value> {
    let (where_sql, and_sql) = build_stats_filter(perihal, status);

    let totals = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
        &format!(
            "SELECT
               count(*) AS total,
               count(*) FILTER (WHERE feedback_type = 'positive'),
               count(*) FILTER (WHERE feedback_type = 'correction'),
               count(*) FILTER (WHERE feedback_type = 'correction' AND status = 'validated'),
               count(*) FILTER (WHERE feedback_type = 'correction' AND status = 'rejected')
             FROM klasifikasi_feedback\n             {where_sql}"
        ),
    )
    .fetch_one(db)
    .await?;

    let top_kode = sqlx::query_as::<_, (String, i64)>(
        &format!(
            "SELECT COALESCE(kode_terbaik, kode_koreksi) AS k, count(*) AS c
             FROM klasifikasi_feedback
             WHERE feedback_type = 'correction' AND status = 'validated'{and_sql}
             GROUP BY 1 ORDER BY 2 DESC LIMIT 5"
        ),
    )
    .fetch_all(db)
    .await?;

    let top_user = sqlx::query_as::<_, (String, i64)>(
        &format!(
            "SELECT COALESCE(NULLIF(user_email,''), user_name, 'anonim') AS u, count(*) AS c
             FROM klasifikasi_feedback\n             {where_sql}
             GROUP BY 1 ORDER BY 2 DESC LIMIT 5"
        ),
    )
    .fetch_all(db)
    .await?;

    // Feedback terbaru (untuk tabel di dashboard) — waktu diformat di SQL agar
    // tidak perlu dependency chrono. Zona Asia/Jakarta (server dinas lokal).
    let recent = sqlx::query_as::<_, (i64, String, String, String, String, String, String, String, String, String, String)>(
        &format!(
            "SELECT id, feedback_type, kode_ai,
                    COALESCE(kode_koreksi, ''),
                    status,
                    COALESCE(NULLIF(user_name,''), 'Anonim'),
                    COALESCE(NULLIF(user_email,''), '-'),
                    COALESCE(validasi_penjelasan, ''),
                    COALESCE(NULLIF(perihal,''), LEFT(naskah, 120)),
                    LEFT(naskah, 120),
                    to_char(created_at AT TIME ZONE 'Asia/Jakarta', 'YYYY-MM-DD HH24:MI')
             FROM klasifikasi_feedback\n             {where_sql}
             ORDER BY id DESC LIMIT 20"
        ),
    )
    .fetch_all(db)
    .await?;

    Ok(json!({
        "total": totals.0,
        "positive": totals.1,
        "correction": totals.2,
        "correction_valid": totals.3,
        "correction_rejected": totals.4,
        "top_kode": top_kode.iter().map(|(k, c)| json!({"kode": k, "count": c})).collect::<Vec<_>>(),
        "top_user": top_user.iter().map(|(u, c)| json!({"user": u, "count": c})).collect::<Vec<_>>(),
        "recent": recent.iter().map(|r| json!({
            "id": r.0,
            "feedback_type": r.1,
            "kode_ai": r.2,
            "kode_koreksi": r.3,
            "status": r.4,
            "user_name": r.5,
            "user_email": r.6,
            "penjelasan": r.7,
            "perihal": r.8,
            "naskah": r.9,
            "waktu": r.10
        })).collect::<Vec<_>>()
    }))
}

/// Pencarian kode untuk dropdown koreksi (ILIKE, ringan). Menyertakan path lengkap
/// agar user bisa memverifikasi posisi kode di hirarki sebelum memilih.
pub async fn search_codes(db: &PgPool, q: &str) -> Result<Vec<(String, String, String)>> {
    let pattern = format!("%{}%", q.trim());
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT kode, deskripsi, path FROM klasifikasi_embedding
         WHERE kode ILIKE $1 OR deskripsi ILIKE $1
         ORDER BY LENGTH(kode) LIMIT 20",
    )
    .bind(pattern)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_fewshot_menyertakan_deskripsi_dan_path() {
        let ex = vec![FewShotExample {
            teks: "laporan realisasi anggaran triwulan".into(),
            kode_ai: "900.03.10".into(),
            kode_terbaik: "900.03.10.01".into(),
            ai_deskripsi: "LRA".into(),
            ai_path: "KEUANGAN > LAPORAN".into(),
            kb_deskripsi: "Laporan Realisasi Anggaran (LRA)".into(),
            kb_path: "KEUANGAN > LAPORAN > LRA".into(),
        }];
        let out = format_fewshot(&ex);
        assert!(out.contains("Naskah: \"laporan realisasi anggaran triwulan\""));
        assert!(out.contains("Kode awal (keliru): 900.03.10 — LRA. Path: KEUANGAN > LAPORAN."));
        assert!(out.contains(
            "Kode benar setelah koreksi arsiparis: 900.03.10.01 — Laporan Realisasi Anggaran (LRA). Path: KEUANGAN > LAPORAN > LRA."
        ));
    }

    #[test]
    fn format_fewshot_kosong_menghasilkan_string_kosong() {
        assert_eq!(format_fewshot(&[]), "");
    }
}
