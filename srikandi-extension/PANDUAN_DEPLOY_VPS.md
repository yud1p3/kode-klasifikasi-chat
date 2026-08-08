# 🚀 Panduan Deploy ke VPS — browser-klasifikasi-arsip

## Komponen Yang Dikirim

| Komponen | Lokasi Lokal | Tujuan VPS |
|---|---|---|
| **API Binary** (Go) | `api/ringkas-api` | `~/ringkas-api/ringkas-api` |
| **Feedback data** | `api/feedback.jsonl` | `~/ringkas-api/feedback.jsonl` |
| **Klasifikasi CSV** | `~/klasifikasi_arsip_*.csv` | `~/ringkas-api/` |
| **Klasifikasi NDJSON** | `~/klasifikasi_arsip_*.ndjson` | `~/ringkas-api/` |
| **Frontend** (React build) | `dist/` | `/var/www/klas-arsip-webid/` |
| **Nginx config** | `nginx-klas-arsip-vps.conf` | `/etc/nginx/sites-available/` |

---

## 0. Dependencies — Install di VPS (pertama kali)

Tools pendukung yang harus ada di VPS sebelum service berjalan:

```bash
# 1. pdftotext — ekstraksi teks dari PDF
sudo apt install -y poppler-utils

# 2. MiniLM embedder — Meilisearch hybrid search
#    Model akan di-download otomatis oleh Meilisearch saat pertama
#    kali digunakan (sekitar 90MB).
#    Cukup set environment HUGGINGFACE_HUB_CACHE di service meilisearch.
#
#    Tambahkan embedder ke index klasifikasi:
curl -X PATCH 'http://127.0.0.1:7700/indexes/klasifikasi/settings' \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer MASTER_KEY_ANDA' \
  -d '{
    "embedders": {
      "miniLM": {
        "source": "huggingFace",
        "model": "sentence-transformers/all-MiniLM-L6-v2"
      }
    }
  }'

# 3. (Opsional) LibreOffice — konversi DOCX ke PDF (fallback)
#    Saat ini DOCX langsung diekstrak via internal parser, tidak perlu.
```

> **Catatan MiniLM:** Model `all-MiniLM-L6-v2` (~90MB) akan di-download
> ke `$HUGGINGFACE_HUB_CACHE` saat query hybrid pertama kali dijalankan.
> Pastikan VPS punya koneksi internet dan cukup disk space.

---

## 1. Deploy ke VPS

```bash
# Dari WSL/Local — jalankan sekali:
eval $(ssh-agent -s) && ssh-add ~/siap_key.pem

# Deploy semua komponen (build dulu, lalu sync):
bash ~/deploy-to-vps.sh
```

Script akan otomatis:
1. ✅ Build frontend (Vite)
2. ✅ Build Go API binary
3. ✅ Sync binary + data ke VPS
4. ✅ Sync frontend + nginx config
5. ✅ Tampilkan langkah selanjutnya

---

## 2. Di VPS — Setup Pertama Kali

### 2a. Setup Nginx

```bash
# Enable site
sudo ln -sf /etc/nginx/sites-available/klas-arsip-webid.conf /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

### 2b. Setup Meilisearch

```bash
# Install Meilisearch (baris perintah dari setup-meilisearch-vps.sh)
# atau jalankan script:
#   bash ~/ringkas-api/setup-meilisearch-vps.sh   # (kalau di-copy)

# Index data klasifikasi:
curl -X POST 'http://127.0.0.1:7700/indexes/klasifikasi/documents' \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer MASTER_KEY_ANDA' \
  --data-binary @~/ringkas-api/klasifikasi_*_bersih.ndjson
```

### 2c. Setup ringkas-api Service

```bash
sudo tee /etc/systemd/system/ringkas-api.service << 'SERVICE'
[Unit]
Description=Ringkas API — Analisa Naskah dengan Gemini
After=network.target

[Service]
Type=simple
User=siapdev
WorkingDirectory=/home/siapdev/ringkas-api
ExecStart=/home/siapdev/ringkas-api/ringkas-api
Environment=GEMINI_API_KEY=isi-api-key-gemini
Environment=GEMINI_MODEL=gemini-2.5-flash
Environment=GOOGLE_CLIENT_ID=isi-client-id-google
Environment=MEILI_HOST=http://127.0.0.1:7700
Environment=MEILI_KEY=isi-master-key-meilisearch
Environment=API_PORT=3001
Environment=HYBRID_SEMANTIC_RATIO=0.3
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
SERVICE

sudo systemctl daemon-reload
sudo systemctl enable --now ringkas-api
sudo systemctl status ringkas-api
```

### 2d. Verifikasi

```bash
# Cek API
curl http://127.0.0.1:3001/api/config

# Cek frontend
curl -s http://127.0.0.1/ | head -5

# Cek Meilisearch
curl http://127.0.0.1:7700/health
```

---

## 3. Update (Deploy Ulang)

```bash
# Dari Local:
eval $(ssh-agent -s) && ssh-add ~/siap_key.pem
bash ~/deploy-to-vps.sh

# Di VPS — restart service:
sudo systemctl restart ringkas-api
```

---

## 4. Setup SSL (Domain Publik)

```bash
sudo apt install certbot python3-certbot-nginx
sudo certbot --nginx -d domain-anda.com
```

---

## 5. Troubleshooting

| Masalah | Cek |
|---|---|
| API tidak bisa diakses | `sudo systemctl status ringkas-api` |
| | `journalctl -u ringkas-api -n 30 --no-pager` |
| | `curl http://127.0.0.1:3001/api/config` |
| Frontend 404 | `sudo nginx -t` |
| | `ls -la /var/www/klas-arsip-webid/` |
| Meilisearch error | `sudo journalctl -u meilisearch -n 30` |
| | `curl http://127.0.0.1:7700/health` |
