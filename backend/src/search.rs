use sqlx::PgPool;

use crate::ClassificationResult;

pub async fn similarity_search(
    db: &PgPool,
    embedding: &[f64],
    limit: i64,
) -> anyhow::Result<Vec<ClassificationResult>> {
    let emb_str = embedding
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let query = format!(
        r#"SELECT id, kode, deskripsi, path,
           1.0 - (embedding <=> '[{}]'::vector) AS similarity
        FROM klasifikasi_embedding
        WHERE embedding IS NOT NULL
        ORDER BY embedding <=> '[{}]'::vector
        LIMIT {}"#,
        emb_str, emb_str, limit
    );

    let rows = sqlx::query_as::<_, (i32, String, String, String, f64)>(&query)
        .fetch_all(db)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, kode, deskripsi, path, similarity)| ClassificationResult {
            id,
            kode,
            deskripsi,
            path,
            similarity,
        })
        .collect())
}
