use std::collections::HashMap;
use serde_json::Value;

const EMBED_MODEL: &str = "gemini-embedding-2";
const CHAT_MODEL: &str = "gemini-flash-lite-latest";

const API_KEY: &str = "AQ.Ab8RN6K86aO73a9xXW_lre4rSwSyY4r9OLEylq5AGxzPWsBYTw";

pub async fn embed_text(_api_key: &str, text: &str) -> anyhow::Result<Vec<f64>> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
        EMBED_MODEL, API_KEY
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

/// Kirim hasil similarity search ke Gemini untuk di-rerank.
/// Gemini mengurutkan ulang berdasarkan: relevansi → spesifisitas path → similarity.
/// Return: (reranked_results, explanation_text)
pub async fn rerank_and_explain(
    _api_key: &str,
    message: &str,
    results: &[super::ClassificationResult],
) -> anyhow::Result<(Vec<super::ClassificationResult>, String)> {
    if results.is_empty() {
        return Ok((vec![], "Tidak ada hasil yang cocok.".into()));
    }

    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        CHAT_MODEL, API_KEY
    );

    // Bangun daftar kandidat untuk prompt
    let mut candidates = String::new();
    for (i, r) in results.iter().enumerate() {
        candidates.push_str(&format!(
            "{}. Kode: {} | Deskripsi: {} | Path: {} | Similarity: {:.4}\n",
            i, r.kode, r.deskripsi, r.path, r.similarity
        ));
    }

    let prompt = format!(
        "Kamu adalah AI Arsiparis. Tugasmu mengurutkan ulang (rerank) daftar kandidat kode klasifikasi arsip.\n\n\
         Perihal naskah: {}\n\n\
         Daftar kandidat (hasil pencarian semantic, belum terurut sempurna):\n{}\n\
         ATURAN PENGURUTAN ULANG (PRIORITAS BERURUT):\n\
         1. RELEVANSI — Kode yang paling cocok dengan perihal naskah diberi peringkat lebih tinggi.\n\
         2. SPESIFISITAS PATH — **ATURAN PALING PENTING**: Jika kode A adalah prefix kode B\
            (misal A=041.03 dan B=041.03.01 atau B=041.03.01.01), maka B HARUS di atas A.\
            JANGAN PERNAH tempatkan kode pendek (induk) di atas kode panjang (anak).\n\
            Contoh benar: 041.03.01.01 > 041.03.01 > 041.03\n\
            Contoh salah: 041.03 > 041.03.01.01 (INI DILARANG)\n\
         3. SIMILARITY SCORE — Tiebreaker terakhir jika kode tidak memiliki hubungan prefix.\n\n\
         Keluarkan HANYA JSON valid (tanpa markdown code block):\n\
         {{\"reranked\":[{{\"rank\":1,\"kode\":\"XXX.XX\",\"alasan_singkat\":\"...\"}}],\
         \"explanation\":\"Penjelasan 1-2 kalimat kenapa peringkat 1 adalah yang terbaik\"}}",
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

            // Bersihkan markdown code fence jika ada
            let cleaned = raw_text
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            let parsed: Value = serde_json::from_str(cleaned).unwrap_or(Value::Null);

            let explanation = parsed["explanation"]
                .as_str()
                .unwrap_or("Hasil diurutkan berdasarkan relevansi dan spesifisitas.")
                .to_string();

            // Bangun map kode → rank dari respons Gemini
            let mut rank_map: HashMap<String, usize> = HashMap::new();
            if let Some(arr) = parsed["reranked"].as_array() {
                for (i, item) in arr.iter().enumerate() {
                    if let Some(kode) = item["kode"].as_str() {
                        rank_map.insert(kode.to_string(), i);
                    }
                }
            }

            // Step 1: Urutkan berdasarkan rank Gemini
            let mut reranked = results.to_vec();
            reranked.sort_by_key(|r| {
                rank_map.get(&r.kode).copied().unwrap_or(usize::MAX)
            });

            // Step 2: Deterministic specificity enforcement
            // Hard rule: child (kode lebih spesifik) HARUS di atas parent.
            // Loop sampai tidak ada lagi parent-child yang terbalik.
            loop {
                let mut swapped = false;
                for i in 0..reranked.len() {
                    for j in (i + 1)..reranked.len() {
                        // Jika results[j] adalah child dari results[i] (prefix match),
                        // dan child ada di bawah parent → swap.
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

            Ok((reranked, explanation))
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
