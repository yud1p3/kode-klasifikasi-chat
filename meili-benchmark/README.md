# Meili Benchmark — pgvector vs Meilisearch

Tool Go standalone untuk menguji performa pencarian embedding:
**PostgreSQL pgvector** vs **Meilisearch vector & hybrid search**, dengan dataset
kode klasifikasi arsip (5.534 dokumen, embedding 768-dims dari Gemini).

## Cara pakai

```bash
# 1. Build
go build -o meili-benchmark .

# 2. Index ulang + benchmark (pertama kali / setelah data berubah)
./meili-benchmark -meili-key "$MEILI_MASTER_KEY" -force -hybrid

# 3. Benchmark saja (tanpa reindex)
./meili-benchmark -meili-key "$MEILI_MASTER_KEY" -skip-index -hybrid
```

### Kredensial (prioritas: flag > env > default)

| Flag            | Env              | Default                                       |
|-----------------|------------------|-----------------------------------------------|
| `-db-url`       | `DATABASE_URL`   | `postgres://postgres:postgres@localhost:5432/klasifikasi_arsip` |
| `-gemini-key`   | `GEMINI_API_KEY` / `GEMINI_API_KEYS` (dipakai key pertama) | — |
| `-meili-host`   | `MEILI_HOST`     | `http://localhost:7700`                       |
| `-meili-key`    | `MEILI_MASTER_KEY` | — (wajib)                                   |

### Flag lain

| Flag            | Fungsi                                              |
|-----------------|-----------------------------------------------------|
| `-force`        | Hapus & buat ulang index Meilisearch + set settings |
| `-skip-index`   | Lewati indexing, langsung benchmark                 |
| `-index`        | Nama index (default `klasifikasi_embedding`)        |
| `-topk`         | Jumlah hasil top-k (default 10)                     |
| `-runs`         | Iterasi per query per engine (default 10)           |
| `-queries`      | File query benchmark (satu per baris)               |
| `-q`            | Batasi jumlah query (default: 17 query bawaan)      |
| `-hybrid`       | Uji tambahan hybrid search (semanticRatio 0.5)      |
| `-verbose`      | Rincian per query                                   |

## Metodologi

- Query di-embed **sekali** via Gemini (`gemini-embedding-2`, 768 dims) — sama untuk semua engine.
- **Warmup** 2 iterasi per engine (tidak dihitung) sebelum timing.
- Urutan pengukuran pgvector/Meilisearch **bergantian** per query (hindari bias urutan).
- Latensi = **median** dari `-runs` iterasi per query (round-trip end-to-end).
- Kualitas: **top-1 sama**, **Jaccard** top-3 dan top-10 antara engine (berdasarkan `kode`).
- pgvector: exact kNN (seq scan, tanpa index HNSW). Meilisearch: HNSW (approximate).
- Catatan: pada dataset kecil ini, selisih latensi didominasi overhead tetap
  (HTTP + serialisasi JSON vektor 768-dims), bukan kecepatan komputasi.

## Hasil ringkas (Agustus 2026, dataset 5.534 dokumen)

| Engine            | p50     | p95     | Ukuran    |
|-------------------|---------|---------|-----------|
| pgvector (exact)  | ~28 ms  | ~32 ms  | 23 MB     |
| meili vector      | ~44 ms  | ~44 ms  | ~173 MB   |
| meili hybrid 0.5  | ~44 ms  | ~48 ms  | —         |

- Top-1 identik dengan pgvector: **100%** (17/17 query) untuk vector maupun hybrid.
- Jaccard top-10 vs pgvector: **0.97** (perbedaan kecil dari HNSW approximate).
- Meilisearch lebih lambat di dataset kecil karena overhead tetap per request;
  keunggulannya muncul di skala besar, QPS tinggi, typo-tolerance, filter/facet,
  dan hybrid keyword+semantic.
