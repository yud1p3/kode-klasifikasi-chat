# ADR-0001: Arsitektur PostgreSQL-only — Hapus Meilisearch, Chat Tanpa Login

- **Status:** Accepted
- **Tanggal:** 2026-08-08
- **Konteks:**

Aplikasi Kode Klasifikasi Arsip (Rust/Actix-web + React/Vite) awalnya mencari kode klasifikasi via **pgvector** di PostgreSQL. Untuk keperluan benchmark, search sempat dimigrasi ke **Meilisearch** terpisah (port 7700) dengan embedder `userProvided`.

Timbul kebutuhan untuk:

1. **Menyederhanakan operasional** — cukup satu database & satu layanan search; tanpa service tambahan yang harus di-deploy, di-monitor, dan di-backup (terutama di VPS).
2. **Membuka akses tanpa login** — chat & feedback positif tidak lagi wajib login, sehingga ada traffic anonim yang harus tetap terlindungi rate limit & kuota.
3. **Menghilangkan redundansi** — PostgreSQL sebenarnya **sudah wajib ada** (few-shot feedback memakai `f.embedding <=> $1::vector` di tabel `klasifikasi_feedback`), jadi Meilisearch menambah kompleksitas tanpa menghilangkan dependensi Postgres.

Data pendukung: 5.534 kode klasifikasi dengan embedding 768 dimensi (`gemini-embedding-2`) tersimpan utuh di tabel `klasifikasi_embedding` — siap dipakai pgvector **tanpa re-index**.

- **Keputusan:**

1. **Hapus Meilisearch sepenuhnya** — modul `meili.rs`, `meili-benchmark/`, variabel `MEILI_*`/`SEARCH_BACKEND` (`.env`, `.env.vps`, start script, systemd), dan referensi di panduan/README.
2. **Search selalu via pgvector** — `search::similarity_search`: cosine similarity `1.0 - (embedding <=> '[...]'::vector)` pada tabel `klasifikasi_embedding`.
3. **Model autentikasi baru:**
   - **Chat & extract-pdf:** tanpa login (rate limit global 10 detik + kuota Gemini RPM/RPD tetap aktif).
   - **Feedback positif (👍):** tanpa login — dicatat **anonim**; identitas tercatat bila user sedang login (token valid).
   - **Koreksi (✏️):** wajib login (akuntabilitas, karena koreksi tervalidasi dipakai sebagai few-shot).
   - **Statistik & pencarian kode:** terbuka untuk semua.
   - **Hapus feedback:** tetap khusus admin (`ADMIN_EMAILS` + `DELETE_SECRET`).
4. **Kolom `chat_id`** — UUID acak per browser (localStorage) dikirim bersama feedback, agar feedback anonim tetap bisa dikelompokkan per sesi chat tanpa fingerprint perangkat.

- **Konsekuensi:**

**Positif:**
- Satu layanan lebih sedikit — deploy/monitor/backup VPS lebih sederhana (embedding ikut dalam `pg_dump`, tidak ada index terpisah yang dimigrasi).
- Kualitas search teruji — hasil `0.87` cosine similarity untuk "Permohonan cuti tahunan" (kode 800.12.02).
- Akses lebih terbuka — pengguna tidak perlu akun untuk bertanya & konfirmasi jawaban.

**Negatif / tradeoff:**
- Kehilangan mode **hybrid keyword + semantic** Meilisearch (tidak signifikan — mekanisme utama memang semantic).
- pgvector tanpa index ANN (brute-force) — tidak masalah untuk 5.534 baris (query < 1 ms).
- Traffic anonim berpotensi disalahgunakan — dilindungi rate limiter global + kuota free Gemini.
- Statistik kini publik — response `/api/feedback/stats` **tidak** mengekspos `chat_id` maupun identitas; anonim ditampilkan sebagai "Anonim".
