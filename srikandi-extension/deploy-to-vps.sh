#!/bin/bash
# ============================================================
# DEPLOY LENGKAP KE VPS — browser-klasifikasi-arsip
# Jalankan di WSL/LOCAL (bukan di VPS)
#
# Cara pakai:
#   eval $(ssh-agent -s) && ssh-add ~/siap_key.pem
#   bash deploy-to-vps.sh
# ============================================================
set -e

VPS_USER="siapdev"
VPS_IP="192.168.181.6"
SSH_KEY="$HOME/siap_key.pem"
LOCAL_PROJECT="$HOME/projects/browser-klasifikasi-arsip"

echo "╔══════════════════════════════════════════════════════════╗"
echo "║     DEPLOY browser-klasifikasi-arsip KE VPS             ║"
echo "║     $VPS_USER@$VPS_IP                         ║"
echo "╚══════════════════════════════════════════════════════════╝"
echo ""

# ── Cek SSH key ──────────────────────────────────────────────
ssh-add -l 2>/dev/null || {
    echo "⚠️  SSH key belum di ssh-agent. Jalankan:"
    echo "   eval \$(ssh-agent -s)"
    echo "   ssh-add $SSH_KEY   # masukkan passphrase"
    echo ""
    exit 1
}

# Test koneksi
echo -n "Menguji koneksi SSH... "
ssh -o StrictHostKeyChecking=accept-new "$VPS_USER@$VPS_IP" "echo OK" || {
    echo "❌ Gagal konek ke VPS. Cek SSH key dan koneksi jaringan."
    exit 1
}
echo "✅ OK"
echo ""

# ── 1. Build Frontend ────────────────────────────────────────
echo "────────────────────────────────────────────────────────────"
echo " [1/5] Build Frontend (Vite) ..."
echo "────────────────────────────────────────────────────────────"
cd "$LOCAL_PROJECT"
npm run build 2>&1
echo "✅ Frontend build selesai"
echo ""

# ── 2. Build Go API Binary ────────────────────────────────────
echo "────────────────────────────────────────────────────────────"
echo " [2/5] Build Go API Binary..."
echo "────────────────────────────────────────────────────────────"
cd "$LOCAL_PROJECT/api"
go build -o ringkas-api . 2>&1
echo "✅ Go build selesai ($(du -h ringkas-api | cut -f1))"
echo ""

# ── 3. Sync ke VPS ────────────────────────────────────────────
echo "────────────────────────────────────────────────────────────"
echo " [3/5] Sync API Binary ke VPS..."
echo "────────────────────────────────────────────────────────────"
ssh "$VPS_USER@$VPS_IP" "mkdir -p ~/ringkas-api"
rsync -avz --progress -e ssh \
    "$LOCAL_PROJECT/api/ringkas-api" \
    "$VPS_USER@$VPS_IP":~/ringkas-api/
echo "✅ API binary terkirim"
echo ""

echo "────────────────────────────────────────────────────────────"
echo " [4/6] Sync Feedback data ke VPS..."
echo "────────────────────────────────────────────────────────────"
rsync -avz --progress -e ssh \
    "$LOCAL_PROJECT/api/feedback.jsonl" \
    "$VPS_USER@$VPS_IP":~/ringkas-api/
echo "✅ Feedback terkirim"
echo ""

echo "────────────────────────────────────────────────────────────"
echo " [5/6] Sync Meilisearch database (full) ke VPS..."
echo "────────────────────────────────────────────────────────────"
MEILI_LOCAL="/var/lib/meilisearch/data.ms"
echo "   Sumber: $MEILI_LOCAL ($(sudo du -sh $MEILI_LOCAL | cut -f1))"
# Siapkan direktori di VPS
ssh "$VPS_USER@$VPS_IP" "sudo mkdir -p /var/lib/meilisearch && sudo chown siapdev:siapdev /var/lib/meilisearch -R"
# Stop Meilisearch di VPS dulu sebelum sync data
ssh "$VPS_USER@$VPS_IP" "sudo systemctl stop meilisearch 2>/dev/null; echo OK"
# Sync full database (butuh sudo untuk baca file milik user meilisearch)
sudo rsync -avz --progress -e ssh \
    "$MEILI_LOCAL/" \
    "$VPS_USER@$VPS_IP":/var/lib/meilisearch/data.ms/
# Kembalikan ownership
ssh "$VPS_USER@$VPS_IP" "sudo chown -R meilisearch:meilisearch /var/lib/meilisearch && sudo systemctl start meilisearch 2>/dev/null; echo OK"
echo "✅ Database Meilisearch terkirim (full, termasuk klasifikasi + feedback + embedder)"
echo ""

echo "────────────────────────────────────────────────────────────"
echo " [6/6] Sync Frontend + Nginx Config ke VPS..."
echo "────────────────────────────────────────────────────────────"
ssh "$VPS_USER@$VPS_IP" "sudo mkdir -p /var/www/klas-arsip-webid && sudo chown siapdev:siapdev /var/www/klas-arsip-webid"
rsync -avz --progress -e ssh \
    "$LOCAL_PROJECT/dist/" \
    "$VPS_USER@$VPS_IP":/var/www/klas-arsip-webid/

# Nginx config
scp "$LOCAL_PROJECT/nginx-klas-arsip-vps.conf" "$VPS_USER@$VPS_IP":/tmp/
ssh "$VPS_USER@$VPS_IP" "sudo mv /tmp/nginx-klas-arsip-vps.conf /etc/nginx/sites-available/klas-arsip-webid.conf && sudo chown root:root /etc/nginx/sites-available/klas-arsip-webid.conf"
echo "✅ Frontend + nginx config terkirim"
echo ""

echo "╔══════════════════════════════════════════════════════════╗"
echo "║     ✅ DEPLOY SELESAI !                                   ║"
echo "╚══════════════════════════════════════════════════════════╝"
echo ""
echo "=== LANGKAH SELANJUTNYA DI VPS ==="
echo "0. Install dependencies (pertama kali):"
echo "   sudo apt install -y poppler-utils"
echo ""
echo "1. Setup Nginx site (pertama kali):"
echo "   sudo ln -sf /etc/nginx/sites-available/klas-arsip-webid.conf /etc/nginx/sites-enabled/"
echo "   sudo nginx -t && sudo systemctl reload nginx"
echo ""
echo "2. Setup Meilisearch (pertama kali):"
echo "   bash ~/ringkas-api/setup-meilisearch-vps.sh"
echo "   (install binary + systemd service + poppler-utils + MiniLM)"
echo "   ⚠️  Jangan index ulang data — database sudah di-copy langung dari lokal"
echo ""
echo "3. Setup ringkas-api service (pertama kali):"
echo "   sudo tee /etc/systemd/system/ringkas-api.service << 'SERVICE'"
echo "   [Unit]"
echo "   Description=Ringkas API — Analisa Naskah dengan Gemini"
echo "   After=network.target"
echo ""
echo "   [Service]"
echo "   Type=simple"
echo "   User=siapdev"
echo "   WorkingDirectory=/home/siapdev/ringkas-api"
echo "   ExecStart=/home/siapdev/ringkas-api/ringkas-api"
echo "   Environment=GEMINI_API_KEY=isi-dengan-api-key"
echo "   Environment=GEMINI_MODEL=gemini-2.5-flash"
echo "   Environment=GOOGLE_CLIENT_ID=isi-dengan-client-id"
echo "   Environment=MEILI_HOST=http://127.0.0.1:7700"
echo "   Environment=MEILI_KEY=isi-dengan-master-key-meili"
echo "   Environment=API_PORT=3001"
echo "   Environment=HYBRID_SEMANTIC_RATIO=0.3"
echo "   Restart=always"
echo "   RestartSec=5"
echo ""
echo "   [Install]"
echo "   WantedBy=multi-user.target"
echo "   SERVICE"
echo ""
echo "   sudo systemctl daemon-reload"
echo "   sudo systemctl enable --now ringkas-api"
echo "   sudo systemctl status ringkas-api"
echo ""
echo "4. Setup SSL (kalau pakai domain publik):"
echo "   sudo apt install certbot python3-certbot-nginx"
echo "   sudo certbot --nginx -d domain-anda.com"
echo ""
echo "5. Cek API sudah running:"
echo "   curl http://127.0.0.1:3001/api/config"
echo ""
echo "=== SELESAI ==="
