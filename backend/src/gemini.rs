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

pub async fn explain_classification(
    _api_key: &str,
    message: &str,
    results: &[super::ClassificationResult],
) -> anyhow::Result<String> {
    // If API is at quota, fallback to top-1 result
    let top = match results.first() {
        Some(r) => format!("{} - {}", r.kode, r.deskripsi),
        None => return Ok("Tidak ada hasil yang cocok.".into()),
    };

    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        CHAT_MODEL, API_KEY
    );

    let mut candidates = String::new();
    for (i, r) in results.iter().take(3).enumerate() {
        candidates.push_str(&format!("{}. {} - {}\n", i + 1, r.kode, r.deskripsi));
    }

    let prompt = format!(
        "Perihal: {}\nKandidat:\n{}\nPilih kode terbaik. Jawab pendek: Kode X - deskripsi. Alasan 1 kalimat.",
        message, candidates
    );

    let body = serde_json::json!({
        "contents": [{ "parts": [{"text": prompt}] }],
        "generationConfig": { "temperature": 0.1, "maxOutputTokens": 512 }
    });

    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            let json: Value = resp.json().await?;
            let text = json["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .unwrap_or(&top);
            Ok(text.to_string())
        }
        Ok(resp) => {
            let err_body = resp.text().await.unwrap_or_default();
            eprintln!("Explain API error: {}", err_body);
            // Fallback: use top result
            Ok(format!("Kode terbaik: {}. Pencarian semantic menghasilkan 3 kandidat, kode ini memiliki similarity tertinggi.", top))
        }
        Err(e) => {
            eprintln!("Explain error: {}", e);
            Ok(format!("Kode terbaik: {}. Pencarian semantic menghasilkan 3 kandidat, kode ini memiliki similarity tertinggi.", top))
        }
    }
}
