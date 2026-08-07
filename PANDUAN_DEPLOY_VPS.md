# 🚀 Panduan Migrasi ke VPS Produksi — kode-klasifikasi-meili

Migrasi aplikasi chat **Kode Klasifikasi Arsip** (Rust + React + Meilisearch + PostgreSQL) dari WSL ke VPS produksi.

> **Kabar baik:** seluruh konfigurasi aplikasi dibaca dari `.env` — **tidak ada perubahan kode yang diperlukan** untuk migrasi. Yang berubah hanya: nilai `.env`, data (database + index Meilisearch), nginx, dan redirect URI Google.

---

## 1. Gambaran Arsitektur di VPS

```
Pengguna → ngrok (https://<domain-vps>.ngrok-free.dev) → nginx :80
                                                          ├── / (statis)   → /var/www/kode-klasifikasi-meili/
                                                          └── /api/*       → proxy 127.0.0.1:3000 (backend Rust)
                                                                             ├── PostgreSQL 17 + pgvector
                                                                             ├── Meilisearch :7700 (master key VPS)
                                                                             └── Gemini API (embedding + chat)
```

| Komponen | Dari WSL | Tujuan VPS | Cara |
|---|---|---|---|
| **Database** (5.534 kode + feedback) | `pg_dump` | PostgreSQL VPS | §3 (sekali) |
| **Index Meilisearch** | `POST /dumps` | Meilisearch VPS | §4 (sekali) |
| **Backend binary** | `cargo build --release` | `~/kode-klasifikasi-meili/` | `deploy-to-vps.sh` |
| **Frontend dist** | `npm run build` | `/var/www/kode-klasifikasi-meili/` | `deploy-to-vps.sh` |
| **Nginx conf** | `deploy/nginx-kode-klasifikasi-vps.conf` | `/etc/nginx/sites-available/` | `deploy-to-vps.sh` |
| **Systemd unit** | `deploy/kode-klasifikasi-meili.service` | `/etc/systemd/system/` | `deploy-to-vps.sh` |
| **.env** | — (buat manual di VPS) | `~/kode-klasifikasi-meili/.env` | §5 |

---

## 2. Prasyarat di VPS (pertama kali)

```bash
# Dependencies aplikasi
sudo apt update && sudo apt install -y \
    postgresql-17 postgresql-17-pgvector \
    poppler-utils nginx curl

# Rust (untuk build — atau build di lokal lalu sync binary, lihat catatan §6)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

**Meilisearch** sudah kamu punya di VPS — pastikan berjalan dengan master key:
```bash
curl http://127.0.0.1:7700/health   # {"status":"available"}
```

**Node.js 24+** hanya diperlukan bila ingin build frontend di VPS. Script deploy kita build di **lokal** (WSL), jadi tidak wajib di VPS.

---

## 3. Migrasi Database PostgreSQL (SEKALI)

> ⚠️ Pastikan versi **pgvector di VPS ≥ versi di WSL** (kolom `embedding` vector 768 dimensi). Bila perlu, hapus & rebuild kolom setelah restore.

### 3a. Di WSL — buat dump

```bash
cd ~/projects/kode-klasifikasi-meili
pg_dump -h 127.0.0.1 -U postgres -d klasifikasi_arsip -Fc -f /tmp/klasifikasi_arsip.dump
ls -lh /tmp/klasifikasi_arsip.dump   # ukuran bisa ratusan MB (embedding 5.534 baris)
```

### 3b. Kirim ke VPS

```bash
scp -i ~/.ssh/key-vps /tmp/klasifikasi_arsip.dump root@<IP-VPS>:/tmp/
```

### 3c. Di VPS — restore

```bash
# Buat database + extension vector (bila belum ada)
sudo -u postgres psql -c "CREATE DATABASE klasifikasi_arsip;"
sudo -u postgres psql -d klasifikasi_arsip -c "CREATE EXTENSION IF NOT EXISTS vector;"

# Restore (no-owner: role lokal mungkin beda)
pg_restore -h 127.0.0.1 -U postgres -d klasifikasi_arsip --no-owner --no-privileges /tmp/klasifikasi_arsip.dump

# Verifikasi
sudo -u postgres psql -d klasifikasi_arsip -c "SELECT COUNT(*) FROM klasifikasi_embedding;"   # 5.534
sudo -u postgres psql -d klasifikasi_arsip -c "SELECT COUNT(*) FROM klasifikasi_feedback;"
rm -f /tmp/klasifikasi_arsip.dump
```

---

## 4. Migrasi Index Meilisearch (SEKALI)

Karena master key di VPS **berbeda**, gunakan fitur **dump/import** (versi WSL & VPS sama — sudah dikonfirmasi).

> ⚠️ **Penting:** dump Meilisearch **menyimpan API keys** (termasuk master key WSL). Saat import, beri `--master-key <KEY-VPS>` agar instance VPS memakai master key barumu, lalu **hapus key sisa WSL** setelah running.

### 4a. Di WSL — buat dump

```bash
# Trigger dump (asinkron)
curl -X POST http://localhost:7700/dumps -H "Authorization: Bearer $MEILI_KEY_WSL"
# → {"taskUid": N} — cek statusnya:
curl -s "http://localhost:7700/tasks/N" -H "Authorization: Bearer $MEILI_KEY_WSL" | grep -o '"status":"[^"]*"'
# Tunggu sampai "succeeded", lalu cari file dump:
ls -lht /var/lib/meilisearch/dumps/ | head -3
```

### 4b. Kirim ke VPS

```bash
scp -i ~/.ssh/key-vps /var/lib/meilisearch/dumps/<nama>.dump root@<IP-VPS>:/tmp/
```

### 4c. Di VPS — import dengan master key BARU

```bash
# Stop meilisearch dulu
sudo systemctl stop meilisearch

# Import dump — instance VPS akan memakai master key VPS
sudo -u meilisearch meilisearch \
    --import-dump /tmp/<nama>.dump \
    --master-key "<MASTER-KEY-VPS>" \
    --env production \
    --http-addr 127.0.0.1:7700

# Setelah import selesai (proses berhenti), start service biasa:
sudo systemctl start meilisearch

# Bersihkan key sisa WSL (hanya sisakan key yang kita butuhkan):
curl http://127.0.0.1:7700/keys -H "Authorization: Bearer <MASTER-KEY-VPS>" | python3 -m json.tool
# → catat UID tiap key, lalu hapus yang bukan milik VPS:
curl -X DELETE http://127.0.0.1:7700/keys/<UID> -H "Authorization: Bearer <MASTER-KEY-VPS>"

# Verifikasi index + jumlah dokumen:
curl "http://127.0.0.1:7700/indexes/klasifikasi_embedding/stats" \
     -H "Authorization: Bearer <MASTER-KEY-VPS>" | python3 -m json.tool
# → numberOfDocuments ≈ 5.534, dan settings punya embedder "userProvided" 768 dims
```

> **Fallback bila import bermasalah** (versi beda/error): rebuild index langsung dari PostgreSQL yang sudah direstore, ikuti pola `meili-benchmark/index.go` (create index + settings `searchableAttributes: [kode, deskripsi, path]` + embedder `userProvided` 768 dims + push dokumen batch 500 dengan `_vectors`). Data ada di tabel `klasifikasi_embedding` (kolom: id, kode, deskripsi, path, embedding).

---

## 5. Konfigurasi .env di VPS

`deploy-to-vps.sh` sudah mengirim template ke `~/kode-klasifikasi-meili/.env.vps.example`. Di VPS:

```bash
cd ~/kode-klasifikasi-meili
cp .env.vps.example .env
nano .env
```

Variabel yang **WAJIB disesuaikan** (beda dari WSL):

| Variabel | Nilai di VPS |
|---|---|
| `DATABASE_URL` | kredensial PostgreSQL VPS |
| `MEILI_MASTER_KEY` | **master key Meilisearch VPS** |
| `GOOGLE_REDIRECT_URI` | `https://liqueur-douche-defuse.ngrok-free.dev/auth/callback` (boleh tambah URI lain, comma-separated) |
| `JWT_SECRET` | **baru** — 32+ karakter acak (jangan pakai dari WSL) |
| `DELETE_SECRET` | **baru** — hindari `$`, `#`, spasi, atau bungkus `'...'` |
| `ADMIN_EMAILS` | email admin produksi |

> **Catatan dotenv:** nilai yang mengandung `$` diinterpretasikan sebagai variabel (kena bug "secret salah" sebelumnya). Selalu bungkus dengan single-quote: `DELETE_SECRET='kalimat-acak-2026'`. `.env` dibaca otomatis oleh backend (dotenv) dari direktori kerja — **jangan** dipindah ke `EnvironmentFile` systemd.

---

## 6. Deploy Aplikasi

### 6a. Sesuaikan konfigurasi di `deploy-to-vps.sh`

Edit bagian `KONFIGURASI VPS` di bagian atas file:

```bash
VPS_USER="root"
VPS_IP="203.0.113.10"
SSH_KEY="$HOME/.ssh/key-vps"
SSH_PORT="22"
REMOTE_HOME="kode-klasifikasi-meili"
WEBROOT="/var/www/kode-klasifikasi-meili"
BACKEND_PORT="3000"
```

### 6b. Jalankan (dari WSL)

```bash
cd ~/projects/kode-klasifikasi-meili
eval $(ssh-agent -s) && ssh-add ~/.ssh/key-vps
bash deploy-to-vps.sh
```

Script otomatis: build frontend (API relatif) → build backend release → sync binary, dist, nginx conf, systemd unit → tampilkan langkah selanjutnya.

> **Catatan kompatibilitas binary:** binary Rust hasil build di WSL Debian bisa langsung jalan di VPS Debian/Ubuntu x86_64 (glibc sama). Bila VPS pakai distro lain / arsitektur beda → build di VPS saja: `cd ~/kode-klasifikasi-meili/backend && cargo build --release` (butuh Rust di VPS).

### 6c. Aktifkan nginx + systemd (di VPS)

```bash
# Nginx
sudo ln -sf /etc/nginx/sites-available/kode-klasifikasi-meili.conf /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx

# Backend service
sudo systemctl enable --now kode-klasifikasi-meili
sudo systemctl status kode-klasifikasi-meili   # aktif & running
journalctl -u kode-klasifikasi-meili -n 30 --no-pager   # cek log startup
```

Cek log startup — harusnya:
```
🔑 Loaded N Gemini API key(s)
🔎 Search backend: meili → http://127.0.0.1:7700 (index 'klasifikasi_embedding', hybrid=false)
🔐 Auth Google AKTIF (redirect: https://<domain>.ngrok-free.dev/auth/callback)
👑 Admin feedback: ...
🔒 DELETE_SECRET terkonfigurasi (N karakter)
```

> **Catatan `default_server`:** bila VPS sudah punya situs lain yang listen di port 80 (mis. aplikasi lain), ubah `listen 80 default_server;` di config ini menjadi `listen 80;` + `server_name <domain-ngrok-vps>;`, agar tidak saling rebut. Atau nonaktifkan situs lama: `sudo rm /etc/nginx/sites-enabled/<situs-lama>`.

### 6d. Ngrok di VPS

Tunnel ngrok VPS menunjuk ke **port 80** (nginx):

```bash
ngrok http 80
```

Gunakan URL `https://liqueur-douche-defuse.ngrok-free.dev` yang muncul.

---

## 7. Google Console — Redirect URI Baru

1. Buka [Google Cloud Console → Credentials](https://console.cloud.google.com/apis/credentials)
2. Klik OAuth Client ID aplikasi chat (client ID yang sama dengan WSL)
3. **Authorized redirect URIs** — tambahkan:
   ```
   https://liqueur-douche-defuse.ngrok-free.dev/auth/callback
   ```
4. Boleh biarkan URI localhost tetap ada (untuk dev) — frontend otomatis memilih URI sesuai origin.

---

## 8. Verifikasi End-to-End

```bash
# 1. Health via domain ngrok VPS
curl -s https://liqueur-douche-defuse.ngrok-free.dev/api/health
# → {"status":"ok"}

# 2. Auth config — redirect_uris harus berisi domain VPS
curl -s https://liqueur-douche-defuse.ngrok-free.dev/api/auth/config

# 3. Login via browser → buka domain, klik "Masuk dengan Google"

# 4. Chat uji (setelah login) — pilih Fungsi/Urusan, upload PDF/DOCX, submit feedback
```

---

## 9. Update Aplikasi (Deploy Ulang)

```bash
# Dari WSL:
bash deploy-to-vps.sh

# Di VPS:
sudo systemctl restart kode-klasifikasi-meili
```

Script `deploy-to-vps.sh` hanya meng-update binary, dist, dan config — database & index Meilisearch **tidak disentuh** (tidak perlu migrasi ulang).

---

## 10. Backup Rutin (cron di VPS)

```bash
# /etc/cron.d/kode-klasifikasi-backup
30 2 * * * root pg_dump -h 127.0.0.1 -U postgres -Fc -d klasifikasi_arsip -f /backup/db_$(date +\%Y\%m\%d).dump
35 2 * * * root curl -s -X POST http://127.0.0.1:7700/dumps -H "Authorization: Bearer <MASTER-KEY-VPS>" > /dev/null
```

---

## 11. Troubleshooting

| Gejala | Cek |
|---|---|
| Backend tidak start | `journalctl -u kode-klasifikasi-meili -n 30 --no-pager` — perhatikan baris `WARNING` (env belum diisi) |
| `Gagal pencarian (Meilisearch): 401` | `MEILI_MASTER_KEY` di `.env` ≠ master key VPS |
| Meilisearch `index tidak ditemukan` | Index belum di-import (§4) — cek `stats` index |
| Login gagal / redirect salah | URI di Google Console belum ditambah (§7); `redirect_uris` di `/api/auth/config` |
| Login 400 `redirect_uri_mismatch` | `GOOGLE_REDIRECT_URI` di `.env` belum berisi domain VPS |
| Feedback "secret salah" | Karakter `$`/`#`/spasi di `DELETE_SECRET` — bungkus `'...'` (§5) |
| Nginx 502 | Backend mati: `sudo systemctl status kode-klasifikasi-meili`; port `BACKEND_PORT` cocok dengan proxy? |
| Frontend blank / API ke localhost | Build frontend tidak boleh punya `VITE_API_URL` (script sudah `unset`) — rebuild |
