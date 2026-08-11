use actix_web::{web, HttpResponse};
use serde::Serialize;
use sqlx::PgPool;
use std::collections::HashMap;

/// Item klasifikasi untuk fitur browse (pengganti dokumen Meilisearch).
/// Field mengikuti aplikasi browser-klasifikasi-arsip, disesuaikan dengan
/// kolom tabel `klasifikasi_embedding` (PostgreSQL):
/// - `deskripsi` di DB tujuan sudah memuat teks lengkap (bukan cuma judul),
///   jadi tidak perlu kolom `deskripsi_lengkap` terpisah.
/// - `klaster` (level) dihitung dari jumlah titik pada kode (010 → 1, dst).
/// - `penyusutan_id` (angka) diganti `penyusutan_akhir` (teks, dari SKKAD).
#[derive(Debug, Serialize)]
pub struct BrowseItem {
    pub id: i32,
    pub kode: String,
    pub deskripsi: String,
    pub path: String,
    pub parent_id: Option<i32>,
    /// Level (klaster): 1 = fungsi/urusan induk, 2 = sub-klas, dst.
    pub level: i32,
    pub has_children: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retensi_aktif: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retensi_inaktif: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub penyusutan_akhir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub klasifikasi_keamanan: Option<String>,
}

/// Baris mentah hasil query browse (SELECT + EXISTS subquery untuk has_children).
type Row = (
    i32,
    String,
    String,
    String,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    Option<String>,
    Option<String>,
    bool,
);

fn row_to_item(r: Row) -> BrowseItem {
    let level = r.1.split('.').count() as i32;
    BrowseItem {
        id: r.0,
        kode: r.1,
        deskripsi: r.2,
        path: r.3,
        parent_id: r.4,
        level,
        has_children: r.9,
        retensi_aktif: r.5,
        retensi_inaktif: r.6,
        penyusutan_akhir: r.7,
        klasifikasi_keamanan: r.8,
    }
}

/// Kolom + EXISTS subquery has_children — dipakai semua query browse agar
/// deklarasi kolom konsisten di satu tempat.
const BROWSE_SELECT: &str = r#"SELECT e.id, e.kode, e.deskripsi, e.path, e.parent_id,
       e.retensi_aktif, e.retensi_inaktif, e.penyusutan_akhir, e.klasifikasi_keamanan,
       EXISTS (SELECT 1 FROM klasifikasi_embedding c WHERE c.parent_id = e.id)
FROM klasifikasi_embedding e"#;

fn parse_offset_limit(query: &HashMap<String, String>) -> (i64, i64) {
    let offset = query.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0).max(0);
    let limit = query
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
        .clamp(1, 100);
    (offset, limit)
}

/// Akar (level-1) = Fungsi/Urusan induk: kode 3 digit (45 item).
/// TIDAK memakai `parent_id IS NULL OR parent_id = 0` karena beberapa record
/// anomali SKKAD punya parent_id=0 padahal bukan level-1 (mis. 590.01) —
/// definisi kode 3 digit konsisten dengan daftar fungsi di build_embed_query.
pub async fn roots(
    state: web::Data<crate::AppState>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let (offset, limit) = parse_offset_limit(&query);
    match browse_paged(&state.db, "WHERE LENGTH(e.kode) = 3", &[], offset, limit).await {
        Ok((items, total)) => HttpResponse::Ok().json(serde_json::json!({ "items": items, "total": total })),
        Err(e) => HttpResponse::InternalServerError().json(crate::ErrorResponse {
            error: format!("Gagal memuat akar klasifikasi: {e}"),
            retry_after_secs: None,
        }),
    }
}

/// Anak dari parent_id tertentu (navigasi parent-child).
pub async fn children(
    state: web::Data<crate::AppState>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let (offset, limit) = parse_offset_limit(&query);
    let parent_id: i64 = match query.get("parent_id").and_then(|v| v.parse().ok()) {
        Some(v) => v,
        None => {
            return HttpResponse::BadRequest().json(crate::ErrorResponse {
                error: "Parameter parent_id wajib diisi (angka)".into(),
                retry_after_secs: None,
            });
        }
    };
    let cond = "WHERE e.parent_id = $1";
    match browse_paged(&state.db, cond, &[parent_id], offset, limit).await {
        Ok((items, total)) => HttpResponse::Ok().json(serde_json::json!({ "items": items, "total": total })),
        Err(e) => HttpResponse::InternalServerError().json(crate::ErrorResponse {
            error: format!("Gagal memuat sub-klasifikasi: {e}"),
            retry_after_secs: None,
        }),
    }
}

/// Satu dokumen by id (dipakai untuk membangun breadcrumb dari hasil pencarian).
pub async fn document(
    state: web::Data<crate::AppState>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let id: i64 = match query.get("id").and_then(|v| v.parse().ok()) {
        Some(v) => v,
        None => {
            return HttpResponse::BadRequest().json(crate::ErrorResponse {
                error: "Parameter id wajib diisi (angka)".into(),
                retry_after_secs: None,
            });
        }
    };
    let sql = format!("{BROWSE_SELECT} WHERE e.id = $1");
    match sqlx::query_as::<_, Row>(&sql).bind(id).fetch_optional(&state.db).await {
        Ok(Some(row)) => HttpResponse::Ok().json(row_to_item(row)),
        Ok(None) => HttpResponse::NotFound().json(crate::ErrorResponse {
            error: "Klasifikasi tidak ditemukan.".into(),
            retry_after_secs: None,
        }),
        Err(e) => HttpResponse::InternalServerError().json(crate::ErrorResponse {
            error: format!("Gagal memuat klasifikasi: {e}"),
            retry_after_secs: None,
        }),
    }
}

/// Pencarian teks (keyword): ILIKE pada kode/deskripsi/path — pengganti
/// `textSearch` Meilisearch yang TIDAK memakai embedding (gratis, tanpa kuota).
/// Mendukung filter `kode_prefix` agar pencarian bisa dibatasi di dalam cabang
/// klasifikasi tertentu (mis. prefix "010" hanya menelusuri kode di bawahnya).
pub async fn search(
    state: web::Data<crate::AppState>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let q = query.get("q").cloned().unwrap_or_default().trim().to_string();
    if q.chars().count() < 2 {
        return HttpResponse::Ok().json(serde_json::json!({ "items": [], "total": 0 }));
    }
    let (offset, limit) = parse_offset_limit(&query);
    let kode_prefix = query.get("kode_prefix").cloned().unwrap_or_default().trim().to_string();

    // Pola ILIKE: kode bisa dicari persis (010.02), deskripsi/path dengan substring.
    // Prefix kode dibatasi: kode = prefix ATAU kode diawali "prefix." (hindari
    // mencocokkan "0100" saat prefix "010" — kode selalu bertitik setelah level 1).
    let pattern = format!("%{q}%");
    let prefix_bind: Option<String> = if kode_prefix.is_empty() { None } else { Some(kode_prefix) };

    let where_sql = "WHERE (e.kode ILIKE $1 OR e.deskripsi ILIKE $1 OR e.path ILIKE $1)
        AND ($2::text IS NULL OR e.kode = $2 OR e.kode LIKE $2 || '.%')";

    let sql = format!("{BROWSE_SELECT} {where_sql} ORDER BY LENGTH(e.kode), e.kode LIMIT $3 OFFSET $4");
    let count_sql = format!(
        "SELECT count(*) FROM klasifikasi_embedding e {where_sql}"
    );

    let items = sqlx::query_as::<_, Row>(&sql)
        .bind(&pattern)
        .bind(&prefix_bind)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await;
    let total = sqlx::query_scalar::<_, i64>(&count_sql)
        .bind(&pattern)
        .bind(&prefix_bind)
        .fetch_one(&state.db)
        .await;

    match (items, total) {
        (Ok(rows), Ok(n)) => HttpResponse::Ok().json(serde_json::json!({
            "items": rows.into_iter().map(row_to_item).collect::<Vec<_>>(),
            "total": n
        })),
        (Err(e), _) | (_, Err(e)) => HttpResponse::InternalServerError().json(crate::ErrorResponse {
            error: format!("Gagal pencarian: {e}"),
            retry_after_secs: None,
        }),
    }
}

/// Eksekusi query browse dengan pagination: WHERE clause (dengan placeholder
/// $1..$n untuk bind) + ORDER BY kode. Mengembalikan (items, total).
async fn browse_paged(
    db: &PgPool,
    where_sql: &str,
    binds: &[i64],
    offset: i64,
    limit: i64,
) -> anyhow::Result<(Vec<BrowseItem>, i64)> {
    // Placeholder bind dimulai dari $1 (parent_id) → pagination pakai nomor berikutnya.
    let n = binds.len();
    let data_sql = format!(
        "{BROWSE_SELECT} {where_sql} ORDER BY e.kode LIMIT ${} OFFSET ${}",
        n + 1,
        n + 2
    );
    let count_sql = format!("SELECT count(*) FROM klasifikasi_embedding e {where_sql}");

    let mut q = sqlx::query_as::<_, Row>(&data_sql);
    for b in binds {
        q = q.bind(*b);
    }
    let rows = q.bind(limit).bind(offset).fetch_all(db).await?;

    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in binds {
        cq = cq.bind(*b);
    }
    let total = cq.fetch_one(db).await?;

    Ok((rows.into_iter().map(row_to_item).collect(), total))
}
