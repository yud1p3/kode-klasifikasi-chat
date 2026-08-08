#!/bin/bash
# ============================================================
# SETUP MEILISEARCH DI VPS (Ubuntu/Debian)
# Jalankan di VPS sebagai user siapdev (dengan sudo access)
# ============================================================
set -e

MEILI_VERSION="v1.46.1"
MEILI_BINARY="meilisearch-linux-amd64"
MEILI_URL="https://github.com/meilisearch/meilisearch/releases/download/${MEILI_VERSION}/${MEILI_BINARY}"
MASTER_KEY="tEVTGJUo6AjqKWmL5deKpLugqAzmmX4l-NpIf5xp7G0"

echo "=== Setup Meilisearch ${MEILI_VERSION} di VPS ==="

# 1. Download binary
echo "[1/6] Downloading Meilisearch ${MEILI_VERSION}..."
wget -q "${MEILI_URL}" -O meilisearch
chmod +x meilisearch
echo "       OK - $(du -h meilisearch | cut -f1)"

# 2. Buat user meilisearch (jika belum ada)
echo "[2/6] Creating meilisearch user..."
sudo useradd -m -s /usr/sbin/nologin meilisearch 2>/dev/null || echo "       User sudah ada, skip"

# 3. Setup direktori
echo "[3/6] Setting up directories..."
sudo mkdir -p /var/lib/meilisearch
sudo mkdir -p /var/log/meilisearch
sudo chown -R meilisearch:meilisearch /var/lib/meilisearch

# 4. Pindahkan binary
echo "[4/6] Installing binary..."
sudo mv meilisearch /usr/local/bin/meilisearch
sudo chown meilisearch:meilisearch /usr/local/bin/meilisearch

# 5. Buat systemd service
echo "[5/6] Creating systemd service..."
sudo tee /etc/systemd/system/meilisearch.service > /dev/null <<'SERVICEEOF'
[Unit]
Description=Meilisearch
After=systemd-user-sessions.service

[Service]
Type=simple
User=meilisearch
Group=meilisearch
Environment="HOME=/var/lib/meilisearch"
Environment="HUGGINGFACE_HUB_CACHE=/var/lib/meilisearch/huggingface_cache"
WorkingDirectory=/var/lib/meilisearch
ExecStart=/usr/local/bin/meilisearch \
  --api-key "${MASTER_KEY}" \
  --db-path /var/lib/meilisearch/data.ms \
  --log-path /var/log/meilisearch/logs.json \
  --http-addr 127.0.0.1:7700 \
  --env production

Restart=always
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
SERVICEEOF

# 6. Start service
echo "[6/6] Starting Meilisearch..."
sudo systemctl daemon-reload
sudo systemctl enable meilisearch
sudo systemctl start meilisearch

# 7. Install poppler-utils (pdftotext — ekstraksi PDF)
echo "[7/9] Installing poppler-utils (pdftotext)..."
sudo apt install -y -qq poppler-utils
echo "       OK"

# 8. Setup MiniLM embedder di index klasifikasi
echo "[8/9] Setting up MiniLM embedder..."
if curl -s -m 10 -o /dev/null -w "%{http_code}" http://127.0.0.1:7700/health | grep -q 200; then
    curl -s -X PATCH 'http://127.0.0.1:7700/indexes/klasifikasi/settings' \
      -H 'Content-Type: application/json' \
      -H "Authorization: Bearer ${MASTER_KEY}" \
      -d '{"embedders":{"miniLM":{"source":"huggingFace","model":"sentence-transformers/all-MiniLM-L6-v2"}}}' \
      -o /dev/null -w "       HTTP %{http_code}\n"
    echo "       ✅ MiniLM embedder siap"
else
    echo "       ⚠️  Meilisearch belum siap, setup MiniLM manually nanti"
fi

echo "[9/9] Final check..."
sleep 2
# Verifikasi
if sudo systemctl is-active --quiet meilisearch; then
    echo ""
    echo "✅ Meilisearch berjalan di port 7700"
    echo "   Service: meilisearch.service"
    echo "   DB path: /var/lib/meilisearch/data.ms"
    curl -s -m 5 http://127.0.0.1:7700/health
    echo ""
else
    echo "❌ ERROR: Meilisearch gagal start!"
    sudo journalctl -u meilisearch --no-pager -n 30
    exit 1
fi
