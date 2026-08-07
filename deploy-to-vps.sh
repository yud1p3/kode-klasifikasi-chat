#!/usr/bin/env bash
# ============================================================
# DEPLOY KE VPS PRODUKSI — kode-klasifikasi-meili
# Jalankan di WSL/LOCAL (bukan di VPS)
#
# Mengirim:
#   • backend release binary  (~/<REMOTE_HOME>/kode-klasifikasi-chat)
#   • frontend dist           (/var/www/kode-klasifikasi-meili/)
#   • nginx config            (/etc/nginx/sites-available/)
#   • systemd unit            (/etc/systemd/system/)
#
# Catatan: database PostgreSQL & index Meilisearch di-migrasi
# SATU KALI (lihat PANDUAN_DEPLOY_VPS.md §3 & §4) — script ini
# hanya untuk deploy/update aplikasi.
#
# Cara pakai:
#   eval $(ssh-agent -s) && ssh-add ~/.ssh/key-vps
#   bash deploy-to-vps.sh
# ============================================================
set -euo pipefail

# ── KONFIGURASI VPS (SESUAIKAN) ─────────────────────────────
VPS_USER="root"                            # user SSH VPS (root / siapdev / dsb)
VPS_IP="203.0.113.10"                      # IP atau hostname VPS
SSH_KEY="$HOME/.ssh/key-vps"               # SSH private key
SSH_PORT="22"
REMOTE_HOME="kode-klasifikasi-meili"       # folder app di $HOME VPS (nama bebas)
WEBROOT="/var/www/kode-klasifikasi-meili"
BACKEND_PORT="3000"                        # port backend Rust di VPS (harus sama dgn nginx conf)
# ────────────────────────────────────────────────────────────

LOCAL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SSH_OPTS=(-i "$SSH_KEY" -p "$SSH_PORT")
RSYNC_E="-e ssh -i $SSH_KEY -p $SSH_PORT"

echo "╔══════════════════════════════════════════════════════════╗"
echo "║  DEPLOY kode-klasifikasi-meili KE VPS                    ║"
echo "║  $VPS_USER@$VPS_IP"
echo "╚══════════════════════════════════════════════════════════╝"
echo

# ── Cek SSH key & agent ─────────────────────────────────────
[ -f "$SSH_KEY" ] || { echo "❌ SSH key tidak ditemukan: $SSH_KEY"; exit 1; }
if ! ssh-add -l >/dev/null 2>&1; then
    echo "⚠️  SSH key belum di ssh-agent. Jalankan:"
    echo "   eval \$(ssh-agent -s)"
    echo "   ssh-add $SSH_KEY"
    echo
    exit 1
fi

# Test koneksi
echo -n "Menguji koneksi SSH... "
ssh "${SSH_OPTS[@]}" -o StrictHostKeyChecking=accept-new "$VPS_USER@$VPS_IP" "echo OK" \
    || { echo "❌ Gagal konek ke VPS. Cek SSH key & jaringan."; exit 1; }
echo "✅ OK"
echo

# ── 1/4 Build Frontend (API relatif — TANPA VITE_API_URL) ───
echo "────────────────────────────────────────────────────────────"
echo " [1/4] Build Frontend (Vite, API relatif) ..."
echo "────────────────────────────────────────────────────────────"
(
  cd "$LOCAL_DIR/frontend"
  unset VITE_API_URL
  npm run build
)
echo "✅ Frontend build selesai"
echo

# ── 2/4 Build Backend Release ────────────────────────────────
echo "────────────────────────────────────────────────────────────"
echo " [2/4] Build Backend Rust (release) ..."
echo "────────────────────────────────────────────────────────────"
(
  cd "$LOCAL_DIR/backend"
  cargo build --release
)
echo "✅ Backend build selesai ($(du -h "$LOCAL_DIR/backend/target/release/kode-klasifikasi-chat" | cut -f1))"
echo

# ── 3/4 Sync ke VPS ──────────────────────────────────────────
echo "────────────────────────────────────────────────────────────"
echo " [3/4] Sync binary + dist + config ke VPS ..."
echo "────────────────────────────────────────────────────────────"
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" "mkdir -p ~/$REMOTE_HOME $WEBROOT"

echo "   • Backend binary → ~/$REMOTE_HOME/"
rsync -avz $RSYNC_E \
    "$LOCAL_DIR/backend/target/release/kode-klasifikasi-chat" \
    "$VPS_USER@$VPS_IP":~/$REMOTE_HOME/

echo "   • Frontend dist → $WEBROOT/"
rsync -avz $RSYNC_E \
    "$LOCAL_DIR/frontend/dist/" \
    "$VPS_USER@$VPS_IP":$WEBROOT/

echo "   • Nginx config (substitusi port)"
sed "s/__BACKEND_PORT__/$BACKEND_PORT/g" \
    "$LOCAL_DIR/deploy/nginx-kode-klasifikasi-vps.conf" > /tmp/nginx-kode-klasifikasi-vps.conf
scp "${SSH_OPTS[@]}" /tmp/nginx-kode-klasifikasi-vps.conf "$VPS_USER@$VPS_IP":/tmp/
rm -f /tmp/nginx-kode-klasifikasi-vps.conf
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" \
    "sudo mv /tmp/nginx-kode-klasifikasi-vps.conf /etc/nginx/sites-available/kode-klasifikasi-meili.conf"

echo "   • Systemd unit (substitusi user/path)"
sed -e "s/__VPS_USER__/$VPS_USER/g" \
    -e "s/__REMOTE_HOME__/$REMOTE_HOME/g" \
    "$LOCAL_DIR/deploy/kode-klasifikasi-meili.service" > /tmp/kode-klasifikasi-meili.service
scp "${SSH_OPTS[@]}" /tmp/kode-klasifikasi-meili.service "$VPS_USER@$VPS_IP":/tmp/
rm -f /tmp/kode-klasifikasi-meili.service
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" \
    "sudo mv /tmp/kode-klasifikasi-meili.service /etc/systemd/system/ && sudo systemctl daemon-reload"

echo "   • Template .env VPS → ~/$REMOTE_HOME/"
scp "${SSH_OPTS[@]}" "$LOCAL_DIR/backend/.env.vps.example" "$VPS_USER@$VPS_IP":~/$REMOTE_HOME/
echo "✅ Sync selesai"
echo

# ── 4/4 Petunjuk langkah selanjutnya ─────────────────────────
echo "╔══════════════════════════════════════════════════════════╗"
echo "║  ✅ DEPLOY SELESAI — LANGKAH DI VPS                       ║"
echo "╚══════════════════════════════════════════════════════════╝"
echo ""echo "1. Buat .env backend (template sudah terkirim, lihat PANDUAN_DEPLOY_VPS.md §5):"
        echo "   cd ~/$REMOTE_HOME && cp .env.vps.example .env && nano .env"
        echo "   # isi: MEILI_MASTER_KEY VPS, GOOGLE_REDIRECT_URI ngrok VPS, JWT_SECRET baru, dst"
echo ""
echo "2. Aktifkan nginx + service:"
echo "   sudo ln -sf /etc/nginx/sites-available/kode-klasifikasi-meili.conf /etc/nginx/sites-enabled/"
echo "   sudo nginx -t && sudo systemctl reload nginx"
echo "   sudo systemctl enable --now kode-klasifikasi-meili"
echo "   sudo systemctl status kode-klasifikasi-meili"
echo ""
echo "3. Google Console — tambahkan redirect URI:"
echo "   https://liqueur-douche-defuse.ngrok-free.dev/auth/callback"
echo ""
echo "4. Verifikasi:"
echo "   curl http://127.0.0.1:$BACKEND_PORT/api/health"
echo "   curl http://127.0.0.1:7700/health"
