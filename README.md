# Kode Klasifikasi Arsip — AI Chat Assistant

Asisten AI berbasis **Rust + React + TypeScript** untuk mencari kode klasifikasi arsip dinas menggunakan pencarian semantic (pgvector) dan penjelasan dari **Google Gemini**.

---

## Fitur

- **Pencarian Semantic** — Embedding 768 dimensi via Gemini `gemini-embedding-2` + pgvector cosine similarity
- **Penjelasan AI** — Gemini memilih kode terbaik dari top-10 hasil, merangking ulang, dan menjelaskan alasannya
- **Upload File PDF/DOCX** — Ekstrak teks langsung dari file: PDF via **pdf-inspector** (crate Rust in-process, tanpa dependensi sistem) dengan fallback pdf.js, DOCX via mammoth
- **Chrome Extension SRIKANDI** — Ekstensi MV3 (`srikandi-extension/`) untuk menganalisa naskah langsung di halaman SRIKANDI: inject tombol "Analisa dengan AI", baca file DOCX/PDF, tampilkan hasil (perihal, penjelasan AI, kode klasifikasi), isi form otomatis, dan feedback 👍 anonim — memakai API repo ini (lihat `srikandi-extension/README.md`)
- **Pemilihan Fungsi/Urusan** — Untuk setiap naskah (pendek maupun panjang), Gemini memilih salah satu dari Fungsi/Urusan induk (dibaca langsung dari database) + perihal inti yang dibersihkan dari nama orang, tempat/wilayah, dan keterangan waktu, lalu query embedding disusun sebagai `"FUNGSI > perihal"` agar hasil pencarian lebih akurat
- **Tanpa Login untuk Chat & Feedback Positif** — chat dan konfirmasi 👍 bisa dipakai tanpa akun Google; feedback positif dicatat **anonim** bila tidak login (identitas ikut tercatat bila sedang login). Login bersifat **opsional** — hanya wajib untuk mengirim **koreksi kode klasifikasi** dan menghapus feedback (admin)
- **Sesi Anonim (chat_id)** — setiap browser punya ID sesi acak (localStorage, UUID v4) yang dikirim bersama feedback; feedback anonim tetap bisa dikelompokkan per sesi chat untuk analisis. ID ini hanya angka acak di browser (bukan data perangkat/fingerprint teknis)
- **Multi-Key Rotasi** — Beberapa API key gratis dirotasi otomatis; saat satu key kena 429 rate limit, permintaan dialihkan ke key berikutnya
- **Pengaturan API Key (multi-key) di Frontend** — Menu Pengaturan untuk menyimpan banyak key per pengguna (localStorage), dengan tombol toggle lihat/sembunyikan; key dikirim berurutan ke backend dan dirotasi otomatis sebelum fallback ke key server
- **Browse Klasifikasi** — Halaman telusur (📋 Browse): cari kode klasifikasi (keyword ILIKE, **gratis tanpa kuota AI**), navigasi parent-child, dan breadcrumb — memakai endpoint `/api/browse/*` langsung ke PostgreSQL (tanpa Meilisearch)
- **Statistik Feedback dengan Filter** — Dashboard statistik bisa difilter perihal (kata kunci) & status (valid/ditolak/pending)
- **Hapus Feedback (Admin)** — Hanya email di `ADMIN_EMAILS` yang bisa menghapus feedback, dengan password secret `DELETE_SECRET`
- **Rate Limit Protection** — Cooldown timer di frontend + rate limiter di backend (10 detik per request)
- **Peringatan Naskah Rahasia** — UI menampilkan peringatan agar tidak mengunggah naskah rahasia atau naskah berisi informasi yang dikecualikan (istilah UU No. 14/2008 tentang Keterbukaan Informasi Publik)
- **Tailwind CSS UI** — Dark theme, responsive, typing indicator, status koneksi

---

## Arsitektur

```
┌──────────────┐  HTTP POST       ┌──────────────┐     ┌──────────────┐
│   Frontend   │ ── /api/chat ──→ │   Backend    │ ──→ │  PostgreSQL  │
│ React + Vite │ ── /api/extract │ Rust/Actix   │ ←── │  + pgvector  │
│   port 5173  │    -pdf (PDF)   │   port 3000  │     └──────────────┘
└──────────────┘                  └──────┬───────┘
                                         │
                                         ├── Gemini Embedding API
                                         ├── Gemini Chat API                                          └── pdf-inspector (PDF → Markdown, in-process)
```

---

## Keputusan Arsitektur (ADR)

Keputusan arsitektur penting didokumentasikan sebagai Architecture Decision Records di [`docs/adr/`](docs/adr/):

| ADR | Topik | Ringkasan |
|-----|-------|-----------|
| [ADR-0001](docs/adr/0001-arsitektur-postgresql-only.md) | Arsitektur PostgreSQL-only | Hapus Meilisearch; search selalu via pgvector; chat & feedback positif tanpa login |
| [ADR-0002](docs/adr/0002-browse-postgresql-keyword.md) | Browse Klasifikasi via PostgreSQL | Halaman browse pakai endpoint `/api/browse/*` langsung ke PostgreSQL — pencarian keyword ILIKE (gratis, tanpa kuota AI), bukan hybrid Meilisearch |

---

## Prasyarat

- **Rust** 1.96+
- **Node.js** 24+
- **PostgreSQL 17** + **pgvector** extension
- **Google Gemini API Key** (free tier cukup, beberapa key untuk rotasi)
- ~~poppler-utils~~ — **tidak diperlukan lagi** (ekstraksi PDF via crate `pdf-inspector`, pure Rust)

---

## Instalasi

### 1. Clone repository

```bash
git clone https://github.com/yud1p3/kode-klasifikasi-chat.git
cd kode-klasifikasi-chat
```

### 2. Database

```bash
# Buat database
sudo -u postgres psql -c "CREATE DATABASE klasifikasi_arsip;"
sudo -u postgres psql -d klasifikasi_arsip -c "CREATE EXTENSION IF NOT EXISTS vector;"

# Restore data (embedding sudah termasuk)
gunzip -c database/klasifikasi_arsip.sql.gz | psql -U postgres -d klasifikasi_arsip

# (fitur sesi anonim, versi baru) — kolom chat_id untuk mengaitkan feedback anonim ke sesi chat
psql -U postgres -d klasifikasi_arsip -c "ALTER TABLE klasifikasi_feedback ADD COLUMN IF NOT EXISTS chat_id text;"
```

### 3. Backend

```bash
cd backend
cp .env.example .env
# Edit .env — isi GEMINI_API_KEYS (multi-key, dipisah koma) atau GEMINI_API_KEY (single)
nano .env

# Build & run
cargo build --release
./target/release/kode-klasifikasi-chat
```

### 4. Frontend

```bash
cd frontend
npm install
npm run dev
```

Buka **http://localhost:5174**

---

## Konfigurasi (.env)

```env
DATABASE_URL=postgres://postgres:postgres@localhost:5432/klasifikasi_arsip
# Single key fallback (jika tidak pakai multi-key)
GEMINI_API_KEY=your-gemini-api-key
# Multi-key (disarankan): comma-separated, rotasi otomatis saat kena 429
GEMINI_API_KEYS=key1,key2,key3
HOST=0.0.0.0
PORT=3100

# --- Admin feedback (fitur hapus) ---
# Email admin yang berhak menghapus feedback (comma-separated, boleh lebih dari satu)
ADMIN_EMAILS=admin@dinas.go.id
# Password secret yang wajib dimasukkan admin untuk menghapus feedback.
# HARUS diisi (jangan default) agar fitur hapus aktif — simpan baik-baik.
DELETE_SECRET=ganti-dengan-password-kuat
# Anti brute-force (opsional, ada default): 5 percobaan password gagal
# per email admin → terkunci 15 menit (DELETE_MAX_ATTEMPTS / DELETE_LOCKOUT_SECS)
DELETE_MAX_ATTEMPTS=5
DELETE_LOCKOUT_SECS=900
```

Prioritas pembacaan: `GEMINI_API_KEYS` (multi-key) lebih diutamakan; `GEMINI_API_KEY` dipakai sebagai fallback jika `GEMINI_API_KEYS` tidak diisi.

Catatan: `GEMINI_API_KEYS` adalah key **server**. Pengguna juga bisa menyimpan key pribadi di menu **Pengaturan** frontend — dikirim bersama tiap request (`api_keys`) dan dicoba berurutan sebelum key server.

---

## API Endpoints

| Method | Path | Deskripsi |
|--------|------|-----------|
| GET | `/api/health` | Health check |
| POST | `/api/chat` | Chat klasifikasi (**tanpa login**; rate limit & kuota tetap berlaku) |
| POST | `/api/extract-pdf` | Ekstrak teks dari file PDF via pdf-inspector (multipart, field `file`; **tanpa login**) |
| GET | `/api/browse/roots` | Akar klasifikasi (level-1) dengan pagination `?offset=&limit=` → `{items, total}` |
| GET | `/api/browse/children` | Anak dari induk tertentu `?parent_id=<id>&offset=&limit=` → `{items, total}` |
| GET | `/api/browse/document` | Satu klasifikasi by id `?id=<id>` → objek tunggal (untuk membangun breadcrumb) |
| GET | `/api/browse/search` | Pencarian keyword `?q=<teks>&kode_prefix=<kode>&offset=&limit=` (ILIKE pada kode/deskripsi/path; `kode_prefix` membatasi hasil di dalam cabang tertentu) |
| GET | `/api/feedback/stats` | Statistik feedback (terbuka, tanpa login); filter opsional `?perihal=...&status=validated\|rejected\|pending` |
| DELETE | `/api/feedback/{id}` | Hapus feedback (khusus admin: `ADMIN_EMAILS` + body `{"password": "..."}` = `DELETE_SECRET`; anti brute-force: 5 gagal → 429 lockout 15 mnt) |
| GET | `/api/me` | Info user login + `is_admin` |

> **Model autentikasi:** chat & feedback positif (**👍**) tidak wajib login — feedback positif dicatat anonim bila tidak ada sesi. Mengirim **koreksi (✏️)** dan **menghapus feedback** wajib login Google (koreksi untuk akuntabilitas few-shot; hapus khusus admin `ADMIN_EMAILS` + `DELETE_SECRET`).

### POST `/api/chat`

Request:

```json
{"message": "Permohonan cuti tahunan pegawai"}
```

Response:

```json
{
  "results": [
    {"id": 123, "kode": "800.12.02", "deskripsi": "Cuti Tahunan", "path": "KEPEGAWAIAN > ...", "similarity": 0.69}
  ],
  "perihal": "Permohonan cuti tahunan",
  "explanation": "Kode terbaik adalah 800.12.02 - Cuti Tahunan. Alasan: ..."
}
```

`perihal` berisi perihal naskah hasil ekstraksi Gemini (untuk ditampilkan di UI dan disimpan bersama feedback).

Request juga bisa meminta **ringkasan naskah (isi ringkas)** — khusus Chrome extension SRIKANDI (versi web tidak memakai):

```json
{"message": "Permohonan cuti tahunan pegawai", "include_ringkasan": true}
```

Dengan `include_ringkasan: true`, respons menyertakan field opsional `ringkasan` (ringkasan isi dokumen dalam 2-3 kalimat). Tanpa opsi ini (perilaku default / web), field `ringkasan` tidak muncul — respons identik seperti sebelumnya.

Request bisa menyertakan key pengguna (multi-key, rotasi otomatis):

```json
{"message": "Permohonan cuti tahunan pegawai", "api_keys": ["AIza...1", "AIza...2"]}
```

(`api_key` tunggal legacy juga tetap didukung.)

### POST `/api/extract-pdf`

Menerima multipart `file` (PDF), mengembalikan:

```json
{"text": "isi teks hasil ekstraksi (Markdown)"}
```

Dipakai frontend sebagai jalur utama ekstraksi PDF (pdf-inspector menghasilkan Markdown terstruktur — heading, tabel — yang lebih baik untuk AI). pdf-inspector terbukti membaca PDF SRIKANDI bertanda tangan elektronik dengan kualitas setara poppler, lebih cepat, dan tanpa dependensi sistem. Sebelumnya dipakai anydoc (pembungkus pdf-inspector) — diganti ke pdf-inspector langsung karena hasil identik (anydoc mendelegasikan 100% pemrosesan PDF ke pdf-inspector) namun tanpa membawa parser docx/xls/pptx yang tidak dipakai (~147 paket dependensi).

---

## Chrome Extension — SRIKANDI

Tersedia juga **Chrome Extension** (MV3, vanilla JS) di folder [`srikandi-extension/`](srikandi-extension/) — versi "scraper" untuk aplikasi SRIKANDI (`srikandi.arsip.go.id`) dengan tujuan sama: analisa naskah (PDF/DOCX) → perihal, penjelasan AI, kode klasifikasi, dan isi form SRIKANDI otomatis.

- Memakai API repo ini secara langsung: `POST /api/chat` (sinkron, dengan `include_ringkasan:true` untuk isi ringkas), `POST /api/extract-pdf`, `GET /api/codes`, `POST /api/feedback`
- Chat & feedback 👍 **tanpa login** (anonim + `chat_id`); login Google & koreksi ✏️ diarahkan ke aplikasi web
- **Isi ringkas hanya untuk extension** — extension mengirim `include_ringkasan:true`; versi web tidak, sehingga tidak ada perubahan perilaku web
- Default API URL: `http://localhost:3100` (bisa diubah di Pengaturan popup)
- DOCX diekstrak client-side (mammoth); PDF diekstrak backend (pdf-inspector, Rust in-process)

Panduan lengkap: [`srikandi-extension/README.md`](srikandi-extension/README.md)

---

## Dataset

5.534 kode klasifikasi arsip dari Klasifikasi Arsip Nasional, mencakup:

- Fungsi/Urusan (klaster 1): Kepegawaian, Keuangan, Pendidikan, Kesehatan, dll
- Sub-klasifikasi hingga level 5
- Embedding 768 dimensi via `gemini-embedding-2`
- **45 root level-1**, semua relasi parent–child valid (0 anak yatim); ada **12 kode duplikat** (24 record) yang merupakan struktur asli SKKAD — lihat [docs/AUDIT_DATA.md](docs/AUDIT_DATA.md) untuk hasil audit lengkap & cara menjalankan ulang

---

## Tech Stack

| Layer | Teknologi |
|-------|-----------|
| Frontend | React 19 + TypeScript + Vite + Tailwind CSS v4 |
| Backend | Rust + Actix-web 4 + SQLx |
| Database | PostgreSQL 17 + pgvector 0.8 |
| AI | Google Gemini (embedding + chat) |

---

## License

MIT
