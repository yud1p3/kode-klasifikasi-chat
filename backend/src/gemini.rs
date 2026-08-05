use std::collections::HashMap;
use serde_json::Value;

const EMBED_MODEL: &str = "gemini-embedding-2";
const CHAT_MODEL: &str = "gemini-flash-lite-latest";

/// POST ke Gemini tanpa retry lokal.
/// Jika kena 429, langsung return error agar caller (try_all) bisa switch key dengan cepat.
/// Retry dan rotasi ditangani sepenuhnya oleh KeyRotator::try_all().
async fn post(client: &reqwest::Client, url: &str, body: &serde_json::Value) -> anyhow::Result<reqwest::Response> {
    let resp = client.post(url).json(body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Gemini {} ({}): {}", status.as_str(), status, body.chars().take(500).collect::<String>());
    }
    Ok(resp)
}

pub async fn embed_text(api_key: &str, text: &str) -> anyhow::Result<Vec<f64>> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
        EMBED_MODEL, api_key
    );
    let body = serde_json::json!({
        "content": { "parts": [{"text": text}] },
        "outputDimensionality": 768
    });

    let resp = post(&client, &url, &body).await?;
    let json: Value = resp.json().await?;
    let values = json["embedding"]["values"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing embedding.values"))?;
    Ok(values.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
}

/// Pilih fungsi/urusan (1 dari 45) + perihal dari naskah, utk menyusun query embedding.
/// Kembalikan (fungsi, perihal).
pub async fn select_fungsi(api_key: &str, text: &str) -> anyhow::Result<(String, String)> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        CHAT_MODEL, api_key
    );
    let daftar = "AGRARIA, BENCANA, KECELAKAAN DAN KONDISI BAHAYA, BINA PEMBANGUNAN DAERAH, HUBUNGAN MASYARAKAT, HUKUM, KEARSIPAN, KEBUDAYAAN, KELAUTAN DAN PERIKANAN, KEPEGAWAIAN, KEPENDUDUKAN DAN KELUARGA BERENCANA, KEPENDUDUKAN DAN PENCATATAN SIPIL, KESATUAN BANGSA DAN POLITIK, KESEHATAN, KETATAUSAHAAN DAN KERUMAHTANGGAAN, KETENAGAKERJAAN, KEUANGAN, KOMUNIKASI DAN INFROMATIKA, KOPERASI DAN UKM, LINGKUNGAN HIDUP, ORGANISASI DAN KETATALAKSANAAN, OTONOMI DAERAH, PARIWISATA, PEKERJAAN UMUM, PEMBERDAYAAN MASYARAKAT DAN DESA, PEMBERDAYAAN PEREMPUAN DAN PERLINDUNGAN ANAK, PEMERINTAHAN DAERAH, PEMUDA DAN OLAHRAGA, PENANAMAN MODAL, PENDIDIKAN, PENDIDIKAN DAN PELATIHAN, PENELITIAN, PENGKAJIAN, PENGEMBANGAN, PEREKAYASAAN, PENERAPAN, SERTA PENDAYAGUNAAN ILMU PENGETAHUAN DAN TEKNOLOGI, PENGADAAN, PERDAGANGAN, PERENCANAAN PEMBANGUNAN, PERHUBUNGAN, PERINDUSTRIAN, PERLENGKAPAN/PERALATAN/KEKAYAAN DAERAH, PERPUSTAKAAN, PERSANDIAN, PERTANIAN, PERUMAHAN RAKYAT, POLISI PAMONG PRAJA DAN PELINDUNGAN MASYARAKAT, SOSIAL, STATISTIK, TRANSMIGRASI";
    let prompt = format!(
        "Anda arsiparis. Dari teks naskah dinas berikut, tentukan SATU Fungsi/Urusan yang paling sesuai dengan SUBSTANSI MASALAH naskah (bukan bentuk surat), dan Tuliskan perihal singkat naskah.\n\nDaftar Fungsi/Urusan:\n{daftar}\n\nTeks naskah:\n{text}\n\nKeluarkan HANYA JSON valid: {{\"fungsi\":\"NAMA PERSIS DARI DAFTAR\",\"perihal\":\"perihal singkat\"}}",
        daftar = daftar, text = &text.chars().take(3000).collect::<String>()
    );
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": prompt}]}],
        "generationConfig": { "temperature": 0.1, "maxOutputTokens": 300 }
    });
    let resp = post(&client, &url, &body).await?;
    let json: Value = resp.json().await?;
    let raw = json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str().unwrap_or("{\"fungsi\":\"\",\"perihal\":\"\"}")
        .trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let f = grab_field(raw, "fungsi");
    let p = grab_field(raw, "perihal");
    Ok((f, p))
}

/// Ambil kode + deskripsi kandidat terbaik pertama (untuk fallback explanation).
/// Ekstrak field dari raw JSON yang korup via pencarian substring manual
/// (tahan karakter aneh/terpotong, tanpa dependency regex).
/// Ambil nilai string dari key JSON tunggal (tahan JSON korup).
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

fn extract_fields_tolerant(raw: &str) -> (String, String, Vec<String>) {
    fn grab(raw: &str, key: &str) -> String {
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
    // Ambil semua nilai "kode":"..." secara berurutan
    let mut kodes: Vec<String> = Vec::new();
    let needle = "\"kode\"";
    let mut rest = raw;
    while let Some((_, after)) = rest.split_once(needle) {
        if let Some(start) = after.find('"') {
            let body = &after[start + 1..];
            if let Some(end) = body.find('"') {
                kodes.push(body[..end].to_string());
                rest = &body[end + 1..];
                continue;
            }
        }
        break;
    }
    (grab(raw, "perihal"), grab(raw, "explanation"), kodes)
}

fn reranked_first_kode(results: &[super::ClassificationResult]) -> (String, String) {
    if let Some(r) = results.first() {
        (r.kode.clone(), r.deskripsi.clone())
    } else {
        (String::new(), String::new())
    }
}

pub async fn rerank_and_explain(
    api_key: &str,
    message: &str,
    results: &[super::ClassificationResult],
) -> anyhow::Result<(Vec<super::ClassificationResult>, String, String)> {
    if results.is_empty() {
        return Ok((vec![], "Tidak ada hasil yang cocok.".into(), String::new()));
    }

    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        CHAT_MODEL, api_key
    );

    let mut candidates = String::new();
    for (i, r) in results.iter().enumerate() {
        candidates.push_str(&format!(
            "{}. Kode: {} | Deskripsi: {} | Path: {} | Similarity: {:.4}\n",
            i, r.kode, r.deskripsi, r.path, r.similarity
        ));
    }

    let prompt = format!(
        "Kamu adalah AI Arsiparis. Di bawah ini adalah teks naskah dinas (bisa teks lengkap\n\
         dokumen, atau perihal singkat).\n\n\
         ===== TEKS NASKAH =====\n\
         {}\n\n\
         ===== DAFTAR KANDIDAT =====\n\
         {}\n\n\
         TUGAS KAMU (3 langkah berurutan):\n\n\
         LANGKAH 1 - EKSTRAK PERIHAL: Baca teks naskah. Cari baris Perihal: / Hal:\n\
         atau simpulkan dari isi dokumen. Hasilnya PERIHAL NASKAH (maks 1 kalimat).\n\
         Abaikan kop surat, kop dinas, alamat, salam pembuka.\n\
         LANGKAH 2 - RERANK KANDIDAT: Urutkan ulang semua kandidat berdasarkan:\n\
         1. RELEVANSI - kecocokan dengan PERIHAL NASKAH.\n\
         2. SPESIFISITAS PATH - **ATURAN PALING PENTING**: jika kode A adalah prefix\n\
            kode B (misal A=041.03 dan B=041.03.01), maka B HARUS di atas A.\n\
            Contoh benar: 041.03.01.01 > 041.03.01 > 041.03\n\
         3. SIMILARITY SCORE - tiebreaker terakhir.\n\
         LANGKAH 3 - JELASKAN: Tulis penjelasan dengan format PERSIS satu kalimat:\n\
         \"Perihal: <perihal>. Kode klasifikasi <kode terpilih> dipilih dengan alasan <alasan>.\"\n\
         <alasan> = 2-3 kalimat fokus kecocokan isi naskah dengan kode/deskripsi terpilih,\n\
         dalam bahasa yang mudah dimengerti pengelola arsip.\n\
         JANGAN menyebut aturan \"spesifisitas path\", \"prefix\", atau aturan teknis pengurutan.\n\n\
         Keluarkan HANYA JSON valid (tanpa markdown code block):\n\
         {{\"perihal\":\"...\",\"reranked\":[{{\"rank\":1,\"kode\":\"XXX.XX\"}}],\"explanation\":\"...\"}}",
        message, candidates
    );

    let body = serde_json::json!({
        "contents": [{ "parts": [{"text": prompt}] }],
        "generationConfig": { "temperature": 0.1, "maxOutputTokens": 2048 }
    });

    let resp = post(&client, &url, &body).await?;
    let json: Value = resp.json().await?;
    let raw_text = json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("{}");

    let cleaned = raw_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    // Parsing toleran: serde gagal total bila JSON korup (karakter aneh / terpotong).
    // Fallback ke ekstraksi regex per-field supaya perihal/reranked/explanation
    // tetap terbaca meski satu bagian rusak.
    let parsed: Value = serde_json::from_str(cleaned).unwrap_or(Value::Null);

    let mut perihal = parsed["perihal"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_default();

    let mut explanation_raw = parsed["explanation"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_default();

    let mut rank_map: HashMap<String, usize> = HashMap::new();
    if let Some(arr) = parsed["reranked"].as_array() {
        for (i, item) in arr.iter().enumerate() {
            if let Some(kode) = item["kode"].as_str() {
                rank_map.insert(kode.to_string(), i);
            }
        }
    }

    // Bila JSON korup, ekstrak manual via regex (tahan terhadap karakter aneh).
    if perihal.is_empty() || explanation_raw.is_empty() || rank_map.is_empty() {
        eprintln!("[rerank] JSON tidak lengkap ({} chars). Raw: {}", cleaned.chars().count(), cleaned.chars().take(800).collect::<String>());
        let (p, e, kodes) = extract_fields_tolerant(cleaned);
        if perihal.is_empty() { perihal = p; }
        if explanation_raw.is_empty() { explanation_raw = e; }
        if rank_map.is_empty() {
            for (i, k) in kodes.into_iter().enumerate() {
                rank_map.entry(k).or_insert(i);
            }
        }
    }

    // Fallback: explanation selalu ikut pola kesepakatan
    // "Perihal: X. Kode klasifikasi Y dipilih dengan alasan Z."
    let explanation = if !explanation_raw.is_empty() {
        explanation_raw
    } else {
        let (kode, deskripsi) = reranked_first_kode(results);
        let p = if perihal.is_empty() { "Naskah".to_string() } else { perihal.clone() };
        format!(
            "Perihal: {}. Kode klasifikasi {} ({}) dipilih dengan alasan deskripsinya paling sesuai dengan isi naskah.",
            p, kode, deskripsi
        )
    };

    let mut reranked = results.to_vec();
    reranked.sort_by_key(|r| rank_map.get(&r.kode).copied().unwrap_or(usize::MAX));

    // Insertion sort: pastikan kode lebih spesifik selalu di atas kode parent-nya
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..reranked.len() - 1 {
            if reranked[i+1].kode.starts_with(&format!("{}.{}", reranked[i].kode, ""))
                && !reranked[i].kode.ends_with(&reranked[i+1].kode)
            {
                reranked.swap(i, i+1);
                changed = true;
            }
        }
    }

    Ok((reranked, explanation, perihal))
}
