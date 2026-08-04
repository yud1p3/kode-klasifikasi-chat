# Kode Klasifikasi Arsip — AI Chat Assistant

Asisten AI berbasis **Rust + React + TypeScript** untuk mencari kode klasifikasi arsip dinas menggunakan pencarian semantic (pgvector) dan penjelasan dari **Google Gemini**.

---

## Fitur

- **Pencarian Semantic** — Embedding 768 dimensi via Gemini `text-embedding-004` + pgvector cosine similarity
- **Penjelasan AI** — Gemini memilih kode terbaik dari top-3 hasil dan menjelaskan alasannya
- **Rate Limit Protection** — Cooldown timer di frontend + rate limiter di backend (free tier API key)
- **Tailwind CSS UI** — Dark theme, responsive, typing indicator, status koneksi

---

## Arsitektur

```
┌──────────────┐     HTTP POST      ┌──────────────┐     ┌──────────────┐
│   Frontend   │ ──── /api/chat ──→ │   Backend    │ ──→ │  PostgreSQL  │
│ React + Vite │                    │ Rust/Actix   │ ←── │  + pgvector  │
│   port 5173  │                    │   port 3000  │     └──────────────┘
└──────────────┘                    └──────┬───────┘
                                          │
                                          ├── Gemini Embedding API
                                          └── Gemini Chat API
```

---

## Prasyarat

- **Rust** 1.96+
- **Node.js** 24+
- **PostgreSQL 17** + **pgvector** extension
- **Google Gemini API Key** (free tier cukup)

---

## Instalasi

### 1. Clone repository

```bash
git clone https://github.com/yudi-pwt/kode-klasifikasi-chat.git
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
# Edit .env — isi GEMINI_API_KEY
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
GEMINI_API_KEY=your-gemini-api-key
HOST=0.0.0.0
PORT=3000
```

---

## API Endpoints

| Method | Path | Deskripsi |
|--------|------|-----------|
| GET | `/api/health` | Health check |
| POST | `/api/chat` | Chat klasifikasi |

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
  "explanation": "Kode terbaik adalah 800.12.02 - Cuti Tahunan. Alasan: ..."
}
```

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
