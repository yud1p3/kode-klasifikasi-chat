use std::collections::HashMap;
use serde_json::Value;

const EMBED_MODEL: &str = "gemini-embedding-2";
const CHAT_MODEL: &str = "gemini-flash-lite-latest";

pub async fn embed_text(_api_key: &str, text: &str) -> anyhow::Result<Vec<f64>> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
        EMBED_MODEL, _api_key
    );
    let body = serde_json::json!({
        "content": { "parts": [{"text": text}] },
        "outputDimensionality": 768
    });
    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let err_body = resp.text().await?;
        anyhow::bail!("Gemini embed API error: {}", err_body);
    }
    let json: Value = resp.json().await?;
    let values = json["embedding"]["values"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing embedding.values"))?;
    Ok(values.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
}

pub async fn rerank_and_explain(
    _api_key: &str,
    message: &str,
    results: &[super::ClassificationResult],
) -> anyhow::Result<(Vec<super::ClassificationResult>, String, String)> {
    if results.is_empty() {
        return Ok((vec![], "Tidak ada hasil yang cocok.".into(), String::new()));
    }

    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        CHAT_MODEL, _api_key
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
         LANGKAH 3 - JELASKAN: Beri penjelasan 2-3 kalimat kenapa peringkat 1 terbaik.\n\
         Penjelasan HARUS fokus pada kecocokan isi naskah dengan kode/deskripsi terpilih.\n\
         JANGAN menyebut aturan \"spesifisitas path\", \"prefix\", atau aturan teknis\n\
         pengurutan. Gunakan bahasa yang mudah dimengerti pengelola arsip.\n\n\
         Keluarkan HANYA JSON valid (tanpa markdown code block):\n\
         {{\"perihal\":\"...\",\"reranked\":[{{\"rank\":1,\"kode\":\"XXX.XX\"}}],\"explanation\":\"...\"}}",
        message, candidates
    );

    let body = serde_json::json!({
        "contents": [{ "parts": [{"text": prompt}] }],
        "generationConfig": { "temperature": 0.1, "maxOutputTokens": 1024 }
    });

    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
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

            let parsed: Value = serde_json::from_str(cleaned).unwrap_or(Value::Null);

            let perihal = parsed["perihal"]
                .as_str()
                .unwrap_or("")
                .to_string();

            let explanation = parsed["explanation"]
                .as_str()
                .unwrap_or("Kode terbaik dipilih berdasarkan kecocokan dengan isi naskah.")
                .to_string();

            let mut rank_map: HashMap<String, usize> = HashMap::new();
            if let Some(arr) = parsed["reranked"].as_array() {
                for (i, item) in arr.iter().enumerate() {
                    if let Some(kode) = item["kode"].as_str() {
                        rank_map.insert(kode.to_string(), i);
                    }
                }
            }

            let mut reranked = results.to_vec();
            reranked.sort_by_key(|r| {
                rank_map.get(&r.kode).copied().unwrap_or(usize::MAX)
            });

            loop {
                let mut swapped = false;
                for i in 0..reranked.len() {
                    for j in (i + 1)..reranked.len() {
                        if reranked[j].kode.starts_with(&format!("{}.", reranked[i].kode)) {
                            reranked.swap(i, j);
                            swapped = true;
                            break;
                        }
                    }
                    if swapped {
                        break;
                    }
                }
                if !swapped {
                    break;
                }
            }

            Ok((reranked, explanation, perihal))
        }
        Ok(resp) => {
            let err_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Gemini API error: {}", err_body);
        }
        Err(e) => {
            anyhow::bail!("Gemini connection error: {}", e);
        }
    }
}
