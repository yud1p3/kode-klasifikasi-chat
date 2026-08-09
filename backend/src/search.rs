use sqlx::PgPool;

use crate::ClassificationResult;

/// Baris mentah hasil query klasifikasi_embedding (9 kolom). Dipakai dua query
/// pencarian (similarity_search & fetch_by_kodes) sehingga decode-nya disatukan.
type Row = (
    i32,
    String,
    String,
    String,
    Option<i32>,
    Option<i32>,
    Option<String>,
    Option<String>,
    f64,
);

fn row_to_result(
    (id, kode, deskripsi, path, retensi_aktif, retensi_inaktif, penyusutan_akhir, klasifikasi_keamanan, similarity): Row,
) -> ClassificationResult {
    ClassificationResult {
        id,
        kode,
        deskripsi,
        path,
        retensi_aktif,
        retensi_inaktif,
        penyusutan_akhir,
        klasifikasi_keamanan,
        similarity,
    }
}

/// Format vektor embedding untuk literal pgvector (dipakai sebagai parameter $n::vector).
fn emb_param(embedding: &[f64]) -> String {
    format!(
        "[{}]",
        embedding.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
    )
}

/// Ambil record klasifikasi untuk sekumpulan kode tertentu (dipakai injeksi
/// few-shot: kode hasil koreksi/konfirmasi arsiparis yang TIDAK lolos top-N
/// pencarian semantic). Similarity dihitung terhadap embedding query yang sama
/// dengan similarity_search agar konsisten — caller bisa memfilter berdasarkan
/// nilainya (mis. hanya inject yang cukup relevan).
pub async fn fetch_by_kodes(
    db: &PgPool,
    embedding: &[f64],
    kodes: &[String],
) -> anyhow::Result<Vec<ClassificationResult>> {
    if kodes.is_empty() {
        return Ok(vec![]);
    }
    let rows = sqlx::query_as::<_, Row>(
        r#"SELECT id, kode, deskripsi, path,
           retensi_aktif, retensi_inaktif, penyusutan_akhir, klasifikasi_keamanan,
           1.0 - (embedding <=> $1::vector) AS similarity
        FROM klasifikasi_embedding
        WHERE kode = ANY($2) AND embedding IS NOT NULL"#,
    )
    .bind(emb_param(embedding))
    .bind(kodes)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(row_to_result).collect())
}

pub async fn similarity_search(
    db: &PgPool,
    embedding: &[f64],
    limit: i64,
) -> anyhow::Result<Vec<ClassificationResult>> {
    let emb = emb_param(embedding);
    let query = format!(
        r#"SELECT id, kode, deskripsi, path,
           retensi_aktif, retensi_inaktif, penyusutan_akhir, klasifikasi_keamanan,
           1.0 - (embedding <=> '{}'::vector) AS similarity
        FROM klasifikasi_embedding
        WHERE embedding IS NOT NULL
        ORDER BY embedding <=> '{}'::vector
        LIMIT {}"#,
        emb, emb, limit
    );

    // Kolom metadata SKKAD bisa NULL (record tanpa data di skkad, mis. id 187823)
    let rows = sqlx::query_as::<_, Row>(&query).fetch_all(db).await?;

    Ok(rows.into_iter().map(row_to_result).collect())
}
