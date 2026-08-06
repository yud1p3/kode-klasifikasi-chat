use anyhow::bail;
use serde_json::json;

use crate::ClassificationResult;

/// Pencarian semantic via Meilisearch (menggantikan pgvector).
/// Data di-index dari tabel klasifikasi_embedding dengan embedder `userProvided`
/// (lihat meili-benchmark/index.go). Embedding query dihasilkan Gemini,
/// lalu dikirim sebagai vektor ke endpoint /search.
pub struct MeiliSearch {
    client: reqwest::Client,
    host: String,
    key: String,
    index: String,
    /// true → hybrid (keyword + semantic, semanticRatio 0.5); false → murni vector
    hybrid: bool,
}

impl MeiliSearch {
    pub fn new(host: String, key: String, index: String, hybrid: bool) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("reqwest client"),
            host,
            key,
            index,
            hybrid,
        }
    }

    pub async fn similarity_search(
        &self,
        query_text: &str,
        embedding: &[f64],
        limit: i64,
    ) -> anyhow::Result<Vec<ClassificationResult>> {
        let url = format!("{}/indexes/{}/search", self.host, self.index);

        let mut body = json!({
            "vector": embedding,
            "limit": limit,
            "showRankingScore": true,
            "hybrid": { "embedder": "userProvided", "semanticRatio": 1.0 }
        });
        if self.hybrid {
            body["hybrid"]["semanticRatio"] = json!(0.5);
            body["q"] = json!(query_text);
        }

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("Meilisearch {} ({}): {}", status.as_str(), status, text.chars().take(300).collect::<String>());
        }

        let json: serde_json::Value = resp.json().await?;
        let hits = json["hits"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Meilisearch: field hits tidak ditemukan"))?;

        Ok(hits
            .iter()
            .map(|h| ClassificationResult {
                id: h["id"].as_i64().unwrap_or(0) as i32,
                kode: h["kode"].as_str().unwrap_or_default().to_string(),
                deskripsi: h["deskripsi"].as_str().unwrap_or_default().to_string(),
                path: h["path"].as_str().unwrap_or_default().to_string(),
                // CATATAN: ini _rankingScore Meilisearch (skala algoritma sendiri, 0-1),
                // BUKAN cosine similarity seperti di search.rs (pgvector). Dipakai hanya
                // sebagai tiebreaker oleh Gemini rerank; frontend tidak menampilkan nilai ini.
                similarity: h["_rankingScore"].as_f64().unwrap_or(0.0),
            })
            .collect())
    }
}
