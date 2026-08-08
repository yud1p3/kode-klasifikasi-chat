use std::collections::HashMap;
use serde_json::Value;

const EMBED_MODEL: &str = "gemini-embedding-2";
const CHAT_MODEL: &str = "gemini-2.5-flash";

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

/// Pilih fungsi/urusan (1 dari 45 klaster-1) + DUA varian perihal dari naskah.
/// Kembalikan (fungsi, perihal_inti, perihal_lengkap):
/// - perihal_inti: bersih (tanpa nama orang/tempat/waktu), huruf kecil → query embedding
/// - perihal_lengkap: detail apa adanya → tampilan UI & feedback
pub async fn select_fungsi(api_key: &str, text: &str, daftar_fungsi: &str) -> anyhow::Result<(String, String, String)> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        CHAT_MODEL, api_key
    );
    let daftar = daftar_fungsi;
    let prompt = format!(
        "Anda arsiparis. Dari teks naskah dinas berikut, tentukan SATU Fungsi/Urusan yang paling sesuai dengan SUBSTANSI MASALAH naskah (bukan bentuk surat), lalu tuliskan DUA varian perihal:\n\n\
         - perihal_lengkap: perihal naskah LENGKAP apa adanya (maks 1 kalimat, boleh memuat tanggal/tahun/nama/tempat sebagaimana tertulis di naskah).\n\
         - perihal_inti: versi BERSIH dari perihal_lengkap, hanya substansi. WAJIB BUANG dari perihal_inti: 1) NAMA ORANG (contoh: \"usulan kenaikan pangkat atas nama Bambang\" cukup \"usulan kenaikan pangkat\"), 2) TEMPAT/WILAYAH/UNIT: kota, kabupaten, kecamatan, desa, instansi, alamat (contoh: \"bimbingan teknis SRIKANDI di Kecamatan Kesamben\" cukup \"bimbingan teknis SRIKANDI\"), 3) KETERANGAN WAKTU & NOMOR: tanggal, bulan, tahun, periode, nomor surat (contoh: \"laporan realisasi anggaran triwulan 2 tahun 2026\" cukup \"laporan realisasi anggaran triwulan\"). PERTAHANKAN istilah substantif seperti \"triwulan\", \"semester\", \"tahun anggaran\" bila menjadi sifat naskah; cukup hilangkan angka/penunjuk spesifiknya. Tulis perihal_inti dalam HURUF KECIL.\n\n\
         Daftar Fungsi/Urusan:\n{daftar}\n\n\
         Teks naskah:\n{text}\n\n\
         Keluarkan HANYA JSON valid: {{\"fungsi\":\"NAMA PERSIS DARI DAFTAR\",\"perihal_inti\":\"perihal inti huruf kecil\",\"perihal_lengkap\":\"perihal lengkap apa adanya\"}}",
        daftar = daftar, text = &text.chars().take(3000).collect::<String>()
    );
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": prompt}]}],
        "generationConfig": { "temperature": 0.1, "maxOutputTokens": 8192, "thinkingConfig": { "thinkingBudget": 0 } }
    });
    let resp = post(&client, &url, &body).await?;
    let json: Value = resp.json().await?;
    let raw = json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str().unwrap_or("{\"fungsi\":\"\",\"perihal_inti\":\"\",\"perihal_lengkap\":\"\"}")
        .trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let f = grab_field(raw, "fungsi");
    let pi = grab_field(raw, "perihal_inti");
    let pl = grab_field(raw, "perihal_lengkap");
    // perihal_inti selalu lowercase & trim (hardening: instruksi prompt saja
    // tidak cukup — model sesekali bisa mengembalikan kapital/whitespace).
    Ok((f.trim().to_string(), pi.trim().to_lowercase(), pl.trim().to_string()))
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
    fewshot: &str,
    perihal_hint: &str,
    need_ringkasan: bool,
    results: &[super::ClassificationResult],
) -> anyhow::Result<(Vec<super::ClassificationResult>, String, String, String)> {
    if results.is_empty() {
        return Ok((vec![], "Tidak ada hasil yang cocok.".into(), String::new(), String::new()));
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

    // Few-shot: koreksi arsiparis tervalidasi pada naskah serupa (jika ada)
    let fewshot_section = if fewshot.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", fewshot)
    };

    // Perihal lengkap dari select_fungsi — dipakai sebagai acuan perihal di
    // LANGKAH 1 agar kalimat penjelasan konsisten dengan perihal tampilan.
    let perihal_hint_section = if perihal_hint.trim().is_empty() {
        String::new()
    } else {
        format!("===== PERIHAL NASKAH (hasil ekstraksi awal) =====\n{}\n\n", perihal_hint)
    };

    // Instruksi ringkasan naskah (isi ringkas) — HANYA bila diminta.
    // Dipakai Chrome extension SRIKANDI; versi web tidak meminta sehingga
    // tidak ada biaya kuota/latensi tambahan (tetap 1 panggilan Gemini yang sama).
    let ringkasan_inst = if need_ringkasan {
        "LANGKAH 4 - RINGKAS NASKAH: Tulis \"isi ringkas\" naskah (ringkasan isi dokumen) dalam 2-3 kalimat padat, fokus substansi masalah & keputusan penting. PERTAHANKAN keterangan nama orang, tempat, dan waktu sebagaimana tertulis di naskah.\n"
    } else {
        ""
    };
    let ringkasan_schema = if need_ringkasan {
        ",\"ringkasan\":\"...\""
    } else {
        ""
    };

    let prompt = format!(
        "Kamu adalah AI Arsiparis. Di bawah ini adalah teks naskah dinas (bisa teks lengkap\n\
         dokumen, atau perihal singkat).\n\n\
         ===== TEKS NASKAH =====\n\
         {}\n\n\
         ===== DAFTAR KANDIDAT =====\n\
         {}\n\n\
         {}\
         {}\
         TUGAS KAMU (langkah berurutan):\n\n\
         LANGKAH 1 - TETAPKAN PERIHAL: Pakai PERIHAL NASKAH dari bagian ===== PERIHAL\n\
         NASKAH ===== di atas bila terisi (jangan ekstrak ulang). Bila kosong, cari baris\n\
         Perihal: / Hal: atau simpulkan dari isi dokumen. Hasilnya PERIHAL NASKAH\n\
         (maks 1 kalimat). Gunakan persis apa adanya, tanpa mengubah huruf besar/kecil.\n\
         Abaikan kop surat, kop dinas, alamat, salam pembuka.\n\
         LANGKAH 2 - RERANK KANDIDAT: Urutkan ulang semua kandidat berdasarkan:\n\
         1. RELEVANSI - kecocokan dengan PERIHAL NASKAH.\n\
         2. SPESIFISITAS PATH - **ATURAN PALING PENTING**: jika kode A adalah prefix\n\
            kode B (misal A=041.03 dan B=041.03.01), maka B HARUS di atas A.\n\
            Contoh benar: 041.03.01.01 > 041.03.01 > 041.03\n\
         3. SIMILARITY SCORE - tiebreaker terakhir.\n\
         Tulis HANYA 3 kandidat terbaik di array reranked.\n\
         LANGKAH 3 - JELASKAN: Tulis penjelasan dengan format PERSIS satu kalimat:\n\
         \"Perihal: <perihal>. Kode klasifikasi <kode terpilih> dipilih dengan alasan <alasan>.\"\n\
         <alasan> = 2-3 kalimat fokus kecocokan isi naskah dengan kode/deskripsi terpilih,\n\
         dalam bahasa yang mudah dimengerti pengelola arsip.\n\
         JANGAN menyebut aturan \"spesifisitas path\", \"prefix\", atau aturan teknis pengurutan.\n\n\
         {}\
         Keluarkan HANYA JSON valid (tanpa markdown code block):\n\
         {{\"perihal\":\"...\",\"reranked\":[{{\"rank\":1,\"kode\":\"XXX.XX\"}}],\"explanation\":\"...\"{}}}",
        message, candidates, fewshot_section, perihal_hint_section, ringkasan_inst, ringkasan_schema
    );

    let body = serde_json::json!({
        "contents": [{ "parts": [{"text": prompt}] }],
        "generationConfig": { "temperature": 0.1, "maxOutputTokens": 8192, "thinkingConfig": { "thinkingBudget": 0 } }
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

    let mut ringkasan = parsed["ringkasan"]
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
        if ringkasan.is_empty() { ringkasan = grab_field(cleaned, "ringkasan"); }
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

    Ok((reranked, explanation, perihal, ringkasan))
}
