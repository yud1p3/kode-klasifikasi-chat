# 🚀 Panduan Migrasi ke VPS Produksi — kode-klasifikasi-chat

Migrasi aplikasi chat **Kode Klasifikasi Arsip** (Rust + React + PostgreSQL pgvector) dari WSL ke VPS produksi.

> **Kabar baik:** seluruh konfigurasi aplikasi dibaca dari `.env` — **tidak ada perubahan kode yang diperlukan** untuk migrasi. Yang berubah hanya: nilai `.env`, data (database PostgreSQL — termasuk embedding search), nginx, dan redirect URI Google.

---

## 1. Gambaran Arsitektur di VPS

```
Pengguna → ngrok (https://<domain-vps>.ngrok-free.dev) → nginx :80
                                                          ├── / (statis)   → /var/www/kode-klasifikasi/
                                                          └── /api/*       → proxy 127.0.0.1:3000 (backend Rust)
                                                                             ├── PostgreSQL 17 + pgvector (search semantic)
                                                                             └── Gemini API (embedding + chat)
```

| Komponen | Dari WSL | Tujuan VPS | Cara |
|---|---|---|---|
| **Database** (5.534 kode + embedding + feedback) | `pg_dump` | PostgreSQL VPS | §3 (sekali) |
| **Backend binary** | `cargo build --release` | `~/apps/kode-klasifikasi/` | `deploy-to-vps.sh` |
| **Frontend dist** | `npm run build` | `/var/www/kode-klasifikasi/` | `deploy-to-vps.sh` |
| **Nginx conf** | `deploy/nginx-kode-klasifikasi-vps.conf` | `/etc/nginx/sites-available/` | `deploy-to-vps.sh` |
| **Systemd unit** | `deploy/kode-klasifikasi.service` | `/etc/systemd/system/` | `deploy-to-vps.sh` |
| **.env** | — (buat manual di VPS) | `~/apps/kode-klasifikasi/.env` | §5 |

---

## 2. Prasyarat di VPS (pertama kali)

```bash
# Dependencies aplikasi
sudo apt update && sudo apt install -y \
    postgresql-17 postgresql-17-pgvector \
    nginx curl

# Rust (untuk build — atau build di lokal lalu sync binary, lihat catatan §6)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

> **Catatan:** aplikasi **tidak lagi memakai Meilisearch** — pencarian semantic sepenuhnya via **pgvector di PostgreSQL** (embedding ikut di-restore pada §3). Meilisearch di VPS boleh dihentikan/nonaktifkan.

**Node.js 24+** hanya diperlukan bila ingin build frontend di VPS. Script deploy kita build di **lokal** (WSL), jadi tidak wajib di VPS.

---

## 3. Migrasi Database PostgreSQL (SEKALI)

> ⚠️ Pastikan versi **pgvector di VPS ≥ versi di WSL** (kolom `embedding` vector 768 dimensi). Bila perlu, hapus & rebuild kolom setelah restore.

### 3a. Di WSL — buat dump

```bash
cd ~/projects/kode-klasifikasi-chat
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

# Migrasi skema terbaru (kolom chat_id utk sesi anonim — tidak ada di dump lama):
sudo -u postgres psql -d klasifikasi_arsip -c "ALTER TABLE klasifikasi_feedback ADD COLUMN IF NOT EXISTS chat_id text;"
rm -f /tmp/klasifikasi_arsip.dump
```

### 3d. Migrasi kolom metadata SKKAD (guard rahasia + retensi/penyusutan)

Backend baru memakai kolom metadata per-kode dari perbup SKKAD untuk:
- **Guard naskah rahasia** — `deteksi_kode` mencocokkan kode klasifikasi yang tertulis di naskah dengan daftar kode berklasifikasi **Terbatas/Rahasia/Sangat Rahasia** (dari kolom `klasifikasi_keamanan`), diblokir sebelum dikirim ke Gemini.
- **Hasil klasifikasi** — retensi aktif/inaktif, penyusutan akhir, dan klasifikasi keamanan ditampilkan di UI web & extension.

Dump DB yang dibuat **setelah** migrasi lokal (perintah `pg_dump` di §3a) **sudah berisi kolom + datanya** — tidak perlu apa-apa. Bila Anda restore dari **dump lama**, jalankan migrasi berikut:

```bash
# 1. Tambah kolom (idempoten — aman dijalankan kapan pun)
sudo -u postgres psql -d klasifikasi_arsip <<'SQL'
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS parent_id integer;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS retensi_aktif integer;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS retensi_inaktif integer;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS penyusutan_akhir text;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS klasifikasi_keamanan text;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS pertimbangan text;
SQL

# 2. Backfill data dari CSV (kirim dulu dari WSL):
#    scp klasifikasi_arsip_lengkap.csv root@<IP-VPS>:/tmp/kkl-migrate/
#    (pastikan readable oleh user postgres: chmod 644 /tmp/kkl-migrate/klasifikasi_arsip_lengkap.csv)
#    PENTING: CSV hasil tools/gabung_skkad.py punya 10 kolom
#    (id,kode,deskripsi,path,parent_id,retensi_aktif,retensi_inaktif,
#    penyusutan_akhir,klasifikasi_keamanan,pertimbangan). \copy memetakan
#    kolom by-position, jadi temp table di bawah harus sama persis 10 kolom.
sudo -u postgres psql -d klasifikasi_arsip <<'SQL'
CREATE TEMP TABLE _tmp_skkad (
  id integer PRIMARY KEY,
  kode text,
  deskripsi text,
  path text,
  parent_id integer,
  retensi_aktif integer,
  retensi_inaktif integer,
  penyusutan_akhir text,
  klasifikasi_keamanan text,
  pertimbangan text
);
\copy _tmp_skkad FROM '/tmp/kkl-migrate/klasifikasi_arsip_lengkap.csv' WITH (FORMAT csv, HEADER true)
UPDATE klasifikasi_embedding e
SET parent_id = t.parent_id,
    retensi_aktif = t.retensi_aktif,
    retensi_inaktif = t.retensi_inaktif,
    penyusutan_akhir = t.penyusutan_akhir,
    klasifikasi_keamanan = t.klasifikasi_keamanan,
    pertimbangan = t.pertimbangan
FROM _tmp_skkad t
WHERE e.id = t.id;
SQL

# 3. Verifikasi — harus menampilkan 1.903+ kode terisi:
sudo -u postgres psql -d klasifikasi_arsip -tAc \
  "SELECT count(*) FROM klasifikasi_embedding WHERE klasifikasi_keamanan IN ('Rahasia','Sangat Rahasia','Terbatas')"
```

> **Script otomatis:** `deploy/db-setup-vps.sh` (STEP4b) sudah melakukan langkah 1 + 2 sekaligus bila file CSV ada di `/tmp/kkl-migrate/` — tinggal jalankan script itu.

---

## 4. Pencarian Semantic via pgvector (TANPA Meilisearch)

> Pencarian kode klasifikasi dilakukan **langsung di PostgreSQL** menggunakan ekstensi **pgvector** (cosine similarity pada kolom `embedding` tabel `klasifikasi_embedding`). Embedding 768 dimensi **sudah termasuk** dalam dump database pada §3 — tidak ada index terpisah yang perlu dimigrasi.

Setelah restore (§3), verifikasi bahwa kolom embedding terisi:

```bash
sudo -u postgres psql -d klasifikasi_arsip -tAc \
  "SELECT count(*) FROM klasifikasi_embedding WHERE embedding IS NOT NULL"
# → 5534 (semua baris punya embedding)
```

Backend membaca `DATABASE_URL` dari `.env` — tidak ada variabel Meilisearch lagi (`MEILI_HOST`, `MEILI_MASTER_KEY`, `SEARCH_BACKEND` sudah dihapus dari kode).

---

## 5. Konfigurasi .env di VPS

`deploy-to-vps.sh` sudah mengirim template ke `~/apps/kode-klasifikasi/.env.vps.example`. Di VPS:

```bash
cd ~/apps/kode-klasifikasi
cp .env.vps.example .env
nano .env
```

Variabel yang **WAJIB disesuaikan** (beda dari WSL):

| Variabel | Nilai di VPS |
|---|---|
| `DATABASE_URL` | kredensial PostgreSQL VPS |
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
REMOTE_HOME="apps/kode-klasifikasi"   # relatif ke $HOME VPS
WEBROOT="/var/www/kode-klasifikasi"
BACKEND_PORT="3000"
```

### 6b. Jalankan (dari WSL)

```bash
cd ~/projects/kode-klasifikasi-chat
eval $(ssh-agent -s) && ssh-add ~/.ssh/key-vps
bash deploy-to-vps.sh
```

Script otomatis: build frontend (API relatif) → build backend release → sync binary, dist, nginx conf, systemd unit → tampilkan langkah selanjutnya.

> **Catatan kompatibilitas binary:** binary Rust hasil build di WSL Debian bisa langsung jalan di VPS Debian/Ubuntu x86_64 (glibc sama). Bila VPS pakai distro lain / arsitektur beda → build di VPS saja: `cd ~/apps/kode-klasifikasi/backend && cargo build --release` (butuh Rust di VPS).

### 6c. Aktifkan nginx + systemd (di VPS)

> ⚠️ **PENTING — nonaktifkan situs lama dulu:** domain ngrok VPS
> (`liqueur-douche-defuse.ngrok-free.dev`) sebelumnya dipakai oleh aplikasi
> **browser-klasifikasi-arsip** (config `/etc/nginx/sites-available/klasifikasi-arsip.conf`
> atau `klas-arsip-webid.conf`, dengan `server_name` = domain yang sama persis).
> Karena config chatbot memakai `default_server` (`server_name _`), nginx tetap
> mencocokkan request ke aplikasi lama — yang tampil browser-klasifikasi, bukan chatbot.
> **Hapus symlink situs lama itu dulu** (lihat `ls /etc/nginx/sites-enabled/`):

```bash
# Lihat situs nginx yang aktif saat ini
ls -la /etc/nginx/sites-enabled/

# Nonaktifkan situs lama browser-klasifikasi (sesuaikan nama file-nya)
sudo rm -f /etc/nginx/sites-enabled/klasifikasi-arsip.conf
sudo rm -f /etc/nginx/sites-enabled/klas-arsip-webid.conf

# Lalu aktifkan situs chatbot + reload
sudo ln -sf /etc/nginx/sites-available/kode-klasifikasi.conf /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx

# Verifikasi: sekarang harusnya HTML chatbot (bukan browser-klasifikasi)
curl -s http://127.0.0.1/ | grep -o '<title>[^<]*</title>'

# Backend service
sudo systemctl enable --now kode-klasifikasi
sudo systemctl status kode-klasifikasi   # aktif & running
journalctl -u kode-klasifikasi -n 30 --no-pager   # cek log startup
```

Cek log startup — harusnya:
```
🔑 Loaded N Gemini API key(s)
🔎 Search backend: pgvector (PostgreSQL)
📊 Kuota free aktif: chat 10 RPM / 250 RPD · embed 100 RPM / 10000 RPD
🛡️  Kode klasifikasi sensitif dimuat: 1903 kode (Rahasia/Sangat Rahasia/Terbatas)
🔐 Auth Google AKTIF (redirect: https://<domain>.ngrok-free.dev/auth/callback)
👑 Admin feedback: ...
🔒 DELETE_SECRET terkonfigurasi (N karakter)
```

> Baris `🛡️ Kode klasifikasi sensitif dimuat` menandakan lapisan guard kode SKKAD aktif. Bila tidak muncul (hanya peringatan gagal muat), pastikan migrasi §3d sudah dijalankan — guard berbasis teks tetap aktif, lapisan kode nonaktif.

> **Catatan `default_server`:** bila VPS sudah punya situs lain yang listen di port 80 (mis. aplikasi lain), ubah `listen 80 default_server;` di config ini menjadi `listen 80;` + `server_name <domain-ngrok-vps>;`, agar tidak saling rebut. Atau nonaktifkan situs lama: `sudo rm /etc/nginx/sites-enabled/<situs-lama>`.
>
> **Kasus browser-klasifikasi:** jika situs lama punya `server_name` yang **sama persis** dengan domain ngrok, mengubah config ini jadi `server_name` saja **tidak cukup** — nginx tetap memakai salah satu (yang pertama di urutan `sites-enabled`). Solusinya hanya satu: **disable situs lama** (hapus symlink-nya), karena dua aplikasi tidak bisa berbagi domain yang sama di port 80.

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

# 3. Guard kode rahasia — endpoint daftar kode sensitif SKKAD (dipakai extension):
curl -s https://liqueur-douche-defuse.ngrok-free.dev/api/dikecualikan/kode-rahasia | head -c 200
# → {"kode":["010.03",...],"level":[...],"total":1903}

# 4. Login via browser → buka domain, klik "Masuk dengan Google"

# 5. Chat uji (setelah login) — pilih Fungsi/Urusan, upload PDF/DOCX, submit feedback
#    · naskah memuat kode berklasifikasi (mis. "Nomor: 001/010.03/2026/...") → ditolak 400
#    · hasil kandidat menampilkan badge 🔒 klasifikasi + 🗓️ retensi + ♻️ penyusutan
```

---

## 9. Update Aplikasi (Deploy Ulang)

```bash
# Dari WSL:
bash deploy-to-vps.sh

# Di VPS:
sudo systemctl restart kode-klasifikasi
```

Script `deploy-to-vps.sh` hanya meng-update binary, dist, dan config — database **tidak disentuh** (tidak perlu migrasi ulang).

---

## 10. Backup Rutin (cron di VPS)

```bash
# /etc/cron.d/kode-klasifikasi-backup
30 2 * * * root pg_dump -h 127.0.0.1 -U postgres -Fc -d klasifikasi_arsip -f /backup/db_$(date +\%Y\%m\%d).dump
# (Dulu ada dump Meilisearch di sini — sekarang tidak perlu: semua data sudah di PostgreSQL)
```

---

## 11. Troubleshooting

| Gejala | Cek |
|---|---|
| Backend tidak start | `journalctl -u kode-klasifikasi -n 30 --no-pager` — perhatikan baris `WARNING` (env belum diisi) |
| `Gagal pencarian` / search error | `DATABASE_URL` salah, atau kolom `embedding` belum terisi (cek §4) |
| Login gagal / redirect salah | URI di Google Console belum ditambah (§7); `redirect_uris` di `/api/auth/config` |
| Login 400 `redirect_uri_mismatch` | `GOOGLE_REDIRECT_URI` di `.env` belum berisi domain VPS |
| Feedback "secret salah" | Karakter `$`/`#`/spasi di `DELETE_SECRET` — bungkus `'...'` (§5) |
| Nginx 502 | Backend mati: `sudo systemctl status kode-klasifikasi`; port `BACKEND_PORT` cocok dengan proxy? |
| Frontend blank / API ke localhost | Build frontend tidak boleh punya `VITE_API_URL` (script sudah `unset`) — rebuild |
| Domain menampilkan aplikasi **lain** (mis. browser-klasifikasi) | Situs lama masih aktif dengan `server_name` domain yang sama — `ls /etc/nginx/sites-enabled/`, lalu hapus symlink situs lama (§6c) |

---

## 12. Migrasi penamaan (dari `kode-klasifikasi-meili` ke `kode-klasifikasi`)

Mulai commit ini, path VPS & nama unit diseragamkan: `kode-klasifikasi-meili` → **`kode-klasifikasi`** (folder app `~/apps/kode-klasifikasi/`, webroot `/var/www/kode-klasifikasi/`, unit `kode-klasifikasi.service`, nginx `kode-klasifikasi.conf`).

> ⚠️ **VPS yang sudah pernah di-deploy dengan nama lama tidak otomatis ikut berubah.** Deploy berikutnya (`deploy-to-vps.sh`) akan membuat folder/unit **baru** — yang lama tidak dihapus. Lakukan migrasi sekali saja di VPS (sambil service dimatikan):

```bash
# 1. Hentikan service lama & pindahkan data app (termasuk .env!)
sudo systemctl stop kode-klasifikasi-meili
mkdir -p ~/apps && mv ~/apps/kode-klasifikasi-meili ~/apps/kode-klasifikasi

# 2. Pindahkan webroot frontend
sudo mv /var/www/kode-klasifikasi-meili /var/www/kode-klasifikasi

# 3. Hapus unit + nginx conf lama, aktifkan yang baru (sudah dikirim deploy berikutnya)
sudo rm -f /etc/systemd/system/kode-klasifikasi-meili.service /etc/nginx/sites-available/kode-klasifikasi-meili.conf
sudo rm -f /etc/nginx/sites-enabled/kode-klasifikasi-meili.conf
sudo systemctl daemon-reload

# 4. Jalankan deploy ulang, lalu aktifkan unit baru
bash deploy-to-vps.sh          # dari WSL
sudo systemctl enable --now kode-klasifikasi
```
