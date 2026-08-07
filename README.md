# Kode Klasifikasi Arsip — AI Chat Assistant

Asisten AI berbasis **Rust + React + TypeScript** untuk mencari kode klasifikasi arsip dinas menggunakan pencarian semantic (pgvector) dan penjelasan dari **Google Gemini**.

---

## Fitur

- **Pencarian Semantic** — Embedding 768 dimensi via Gemini `gemini-embedding-2` + pgvector cosine similarity
- **Penjelasan AI** — Gemini memilih kode terbaik dari top-10 hasil, merangking ulang, dan menjelaskan alasannya
- **Upload File PDF/DOCX** — Ekstrak teks langsung dari file: PDF via poppler (`pdftotext`) dengan fallback pdf.js, DOCX via mammoth
- **Pemilihan Fungsi/Urusan** — Untuk setiap naskah (pendek maupun panjang), Gemini memilih salah satu dari Fungsi/Urusan induk (dibaca langsung dari database) + perihal inti yang dibersihkan dari nama orang, tempat/wilayah, dan keterangan waktu, lalu query embedding disusun sebagai `"FUNGSI > perihal"` agar hasil pencarian lebih akurat
- **Multi-Key Rotasi** — Beberapa API key gratis dirotasi otomatis; saat satu key kena 429 rate limit, permintaan dialihkan ke key berikutnya
- **Pengaturan API Key (multi-key) di Frontend** — Menu Pengaturan untuk menyimpan banyak key per pengguna (localStorage), dengan tombol toggle lihat/sembunyikan; key dikirim berurutan ke backend dan dirotasi otomatis sebelum fallback ke key server
- **Statistik Feedback dengan Filter** — Dashboard statistik bisa difilter perihal (kata kunci) & status (valid/ditolak/pending)
- **Hapus Feedback (Admin)** — Hanya email di `ADMIN_EMAILS` yang bisa menghapus feedback, dengan password secret `DELETE_SECRET`
- **Rate Limit Protection** — Cooldown timer di frontend + rate limiter di backend (10 detik per request)
- **Peringatan Naskah Sensitif** — UI menampilkan peringatan agar tidak mengunggah naskah rahasia/berisi informasi sensitif
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
                                         ├── Gemini Chat API
                                         └── poppler (pdftotext)
```

---

## Prasyarat

- **Rust** 1.96+
- **Node.js** 24+
- **PostgreSQL 17** + **pgvector** extension
- **Google Gemini API Key** (free tier cukup, beberapa key untuk rotasi)
- **poppler-utils** (untuk ekstraksi PDF di backend)

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

Buka **http://localhost:5173**

---

## Konfigurasi (.env)

```env
DATABASE_URL=postgres://postgres:postgres@localhost:5432/klasifikasi_arsip
# Single key fallback (jika tidak pakai multi-key)
GEMINI_API_KEY=your-gemini-api-key
# Multi-key (disarankan): comma-separated, rotasi otomatis saat kena 429
GEMINI_API_KEYS=key1,key2,key3
HOST=0.0.0.0
PORT=3000

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
| POST | `/api/chat` | Chat klasifikasi |
| POST | `/api/extract-pdf` | Ekstrak teks dari file PDF via poppler (multipart, field `file`) |
| GET | `/api/feedback/stats` | Statistik feedback; filter opsional `?perihal=...&status=validated\|rejected\|pending` |
| DELETE | `/api/feedback/{id}` | Hapus feedback (khusus admin: `ADMIN_EMAILS` + body `{"password": "..."}` = `DELETE_SECRET`; anti brute-force: 5 gagal → 429 lockout 15 mnt) |
| GET | `/api/me` | Info user login + `is_admin` |

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

Request bisa menyertakan key pengguna (multi-key, rotasi otomatis):

```json
{"message": "Permohonan cuti tahunan pegawai", "api_keys": ["AIza...1", "AIza...2"]}
```

(`api_key` tunggal legacy juga tetap didukung.)

### POST `/api/extract-pdf`

Menerima multipart `file` (PDF), mengembalikan:

```json
{"text": "isi teks hasil ekstraksi poppler"}
```

Dipakai frontend sebagai jalur utama ekstraksi PDF karena menangani PDF SRIKANDI dengan tabel ToUnicode rusak (pdf.js menghasilkan karakter garbled, poppler membaca benar).

---

## Dataset

5.534 kode klasifikasi arsip dari Klasifikasi Arsip Nasional, mencakup:

- Fungsi/Urusan (klaster 1): Kepegawaian, Keuangan, Pendidikan, Kesehatan, dll
- Sub-klasifikasi hingga level 5
- Embedding 768 dimensi via `gemini-embedding-2`

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
