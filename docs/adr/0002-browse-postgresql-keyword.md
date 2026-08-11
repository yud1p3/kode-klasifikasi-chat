# ADR-0002: Browse Klasifikasi via PostgreSQL Langsung — Pencarian Keyword ILIKE, Tanpa Meilisearch

- **Status:** Accepted
- **Tanggal:** 2026-08-11
- **Konteks:**

Aplikasi lama `browser-klasifikasi-arsip` punya halaman browse klasifikasi yang mencari kode dengan slider "semantik" (hybrid Meilisearch), navigasi parent-child, breadcrumb, dan infinite scroll. Fitur ini perlu disalin ke aplikasi `kode-klasifikasi-chat`, yang sudah mengadopsi arsitektur **PostgreSQL-only** (lihat ADR-0001) — Meilisearch dihapus, pencarian semantik lewat pgvector.

Muncul dua pilihan untuk pencarian di halaman browse:

1. **Semantik via pgvector + embedding Gemini** — konsisten dengan chat, tapi **menghabiskan kuota Gemini** (embedding per query) pada fitur yang bersifat eksplorasi/penjelajahan.
2. **Keyword murni (ILIKE) di PostgreSQL** — gratis, tanpa kuota AI, cukup untuk mencari kode/deskripsi/path.

Pengguna memilih **keyword saja (gratis)** untuk menghindari biaya operasional kuota AI pada fitur penjelajahan.

Selain itu, dataset `klasifikasi_embedding` (5.534 baris SKKAD) sudah memiliki relasi parent-child yang sahih setelah audit & perbaikan data (`tools/perbaiki_parent_id.py`, 28 record diselaraskan ke CSV sumber) — sehingga navigasi induk-anak bisa diandalkan.

- **Keputusan:**

1. **Backend — modul baru `browse.rs`** dengan 4 endpoint yang membaca langsung dari PostgreSQL:
   - `GET /api/browse/roots` — klasifikasi level-1 (45 root, filter `LENGTH(kode) = 3`), pagination `offset/limit` (default 20, maks 100).
   - `GET /api/browse/children?parent_id=<id>` — anak langsung dari induk, diurutkan by kode, dengan flag `has_children` (EXISTS subquery) agar UI tahu apakah tombol "Sub Klas" perlu muncul.
   - `GET /api/browse/document?id=<id>` — satu record (untuk membangun breadcrumb dari hasil pencarian).
   - `GET /api/browse/search?q=<teks>&kode_prefix=<kode>` — ILIKE pada `kode`, `deskripsi`, dan `path`; `kode_prefix` membatasi hasil di dalam cabang (pola kode `prefix` atau `prefix.%`); diurutkan `LENGTH(kode), kode` (umum dulu). Semua input lewat **bind parameter** (aman SQL injection).
2. **Frontend — port dari aplikasi lama ke TypeScript** dengan penyesuaian skema DB tujuan:
   - Hooks: `useDebounce` (500 ms — request hanya dikirim setelah user berhenti mengetik, plus guard race-condition `searchSeqRef`) dan `useInfiniteScroll`.
   - Komponen: `BrowseView`, `Breadcrumb`, `SearchBar`, `ClassificationCard` (dark theme, kolom `path`, `retensi`, `penyusutan_akhir`, `klasifikasi_keamanan`; **tanpa** "Deskripsi Lengkap" karena tidak ada di skema tujuan).
   - Integrasi: menu sidebar "📋 Browse" di `App.tsx` (view baru).
   - Tombol **"Salin kode"** di kartu (fallback `execCommand` untuk non-HTTPS) dan **indikator "Mengetik…"** selama jeda debounce.
3. **Data** — audit menyeluruh (0 anak yatim, 0 self-cycle, relasi valid) didokumentasikan di `docs/AUDIT_DATA.md`; 12 kode duplikat (24 record) adalah struktur asli SKKAD dan **dibiarkan** (dibedakan via deskripsi, navigasi memakai `id` unik).

- **Konsekuensi:**

**Positif:**
- **Gratis** — pencarian browse tidak memakai kuota Gemini; ILIKE pada 5.534 baris sangat cepat (tanpa index khusus pun < 50 ms).
- **Satu database** — konsisten dengan ADR-0001; tidak ada service tambahan untuk di-deploy/monitor di VPS.
- **Data sahih** — navigasi induk-anak dapat diandalkan setelah audit & perbaikan `parent_id`.
- **Ramah arsiparis** — salin kode sekali klik sangat membantu saat menghadapi kode duplikat.

**Negatif / tradeoff:**
- Tanpa pencarian semantik — user harus mengetik kata yang persis muncul di `kode`/`deskripsi`/`path`; sinonim bahasa alami tidak tertangkap (bisa dicari nanti via chat).
- Duplikat kode SKKAD tampil sebagai kartu terpisah — membingungkan secara visual, tapi merupakan data asli (didokumentasikan di `docs/AUDIT_DATA.md`).
- ILIKE tanpa index trigram (`pg_trgm`) masih cukup untuk 5.534 baris; jika dataset tumbuh besar, tambahkan index `GIN (kode gin_trgm_ops)`.
