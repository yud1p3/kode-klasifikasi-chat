#!/usr/bin/env bash
# ============================================================
# DEPLOY KE VPS PRODUKSI — kode-klasifikasi-chat
# Jalankan di WSL/LOCAL (bukan di VPS)
#
# Mengirim:
#   • backend release binary  (~/<REMOTE_HOME>/kode-klasifikasi-chat)
#   • frontend dist           (/var/www/kode-klasifikasi/)
#   • nginx config            (/etc/nginx/sites-available/)
#   • systemd unit            (/etc/systemd/system/)
#
# Catatan: database PostgreSQL di-migrasi
# SATU KALI (lihat PANDUAN_DEPLOY_VPS.md §3) — script ini
# hanya untuk deploy/update aplikasi.
#
# Cara pakai:
#   eval $(ssh-agent -s) && ssh-add ~/.ssh/key-vps
#   bash deploy-to-vps.sh
# ============================================================
set -euo pipefail

# ── KONFIGURASI VPS (SESUAIKAN) ─────────────────────────────
VPS_USER="siapdev"                          # user SSH VPS (root / siapdev / dsb)
VPS_IP="192.168.181.6"                      # IP atau hostname VPS
SSH_KEY="$HOME/.ssh/hermes_key"                # SSH private key (sudah di ssh-agent)
SSH_PORT="22"
REMOTE_HOME="apps/kode-klasifikasi"   # folder app di $HOME VPS
WEBROOT="/var/www/kode-klasifikasi"
BACKEND_PORT="3000"                         # port backend Rust di VPS (harus sama dgn nginx conf)
# ────────────────────────────────────────────────────────────

LOCAL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SSH_OPTS=(-i "$SSH_KEY" -o Port="$SSH_PORT")
# rsync memakai var env RSYNC_RSH untuk command ssh (bukan -e '$@')
# agar opsi ssh tidak salah diinterpretasikan oleh rsync.
export RSYNC_RSH="ssh -i $SSH_KEY -o Port=$SSH_PORT"

echo "╔══════════════════════════════════════════════════════════╗"
echo "║  DEPLOY kode-klasifikasi-chat KE VPS                    ║"
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

# ── Stop service lama (agar port tidak rebutan, jika sudah ada) ─
echo "────────────────────────────────────────────────────────────"
echo " [0/4] Hentikan service lama di VPS (jika sudah ada) ..."
echo "────────────────────────────────────────────────────────────"
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" "
  sudo systemctl stop kode-klasifikasi 2>/dev/null || true
  sudo systemctl disable kode-klasifikasi-meili 2>/dev/null || true
  sudo systemctl stop kode-klasifikasi-meili 2>/dev/null || true
  echo '✅ Service lama dihentikan'
" || true
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

# Create app directory (no sudo needed)
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" "mkdir -p ~/$REMOTE_HOME" \
    || { echo "❌ Gagal membuat ~/$REMOTE_HOME"; exit 1; }

# Create webroot with sudo (try with sudo first, then handle no-password sudo)
echo "   • Membuat webroot directory ($WEBROOT)..."
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" "sudo mkdir -p $WEBROOT && sudo chown $VPS_USER:$VPS_USER $WEBROOT" \
    || ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" "sudo -S mkdir -p $WEBROOT && sudo -S chown $VPS_USER:$VPS_USER $WEBROOT" \
    || { echo "⚠️  Perhatian: Tidak bisa membuat $WEBROOT dengan sudo. Pastikan siapdev bisa menggunakan sudo tanpa password untuk mkdir & chown, atau hubungi admin VPS."; exit 1; }

echo "   • Backend binary → ~/$REMOTE_HOME/"
rsync -avz \
    "$LOCAL_DIR/backend/target/release/kode-klasifikasi-chat" \
    "$VPS_USER@$VPS_IP":~/$REMOTE_HOME/

echo "   • Frontend dist → $WEBROOT/"
rsync -avz \
    "$LOCAL_DIR/frontend/dist/" \
    "$VPS_USER@$VPS_IP":$WEBROOT/

echo "   • Nginx config (substitusi port)"
sed "s|__BACKEND_PORT__|$BACKEND_PORT|g" \
    "$LOCAL_DIR/deploy/nginx-kode-klasifikasi-vps.conf" > /tmp/nginx-kode-klasifikasi-vps.conf
scp "${SSH_OPTS[@]}" /tmp/nginx-kode-klasifikasi-vps.conf "$VPS_USER@$VPS_IP":/tmp/
rm -f /tmp/nginx-kode-klasifikasi-vps.conf
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" \
    "sudo mv /tmp/nginx-kode-klasifikasi-vps.conf /etc/nginx/sites-available/kode-klasifikasi.conf"

echo "   • Systemd unit (substitusi user/path)"
sed -e "s|__VPS_USER__|$VPS_USER|g" \
    -e "s|__REMOTE_HOME__|$REMOTE_HOME|g" \
    "$LOCAL_DIR/deploy/kode-klasifikasi.service" > /tmp/kode-klasifikasi.service
scp "${SSH_OPTS[@]}" /tmp/kode-klasifikasi.service "$VPS_USER@$VPS_IP":/tmp/
rm -f /tmp/kode-klasifikasi.service
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" \
    "sudo mv /tmp/kode-klasifikasi.service /etc/systemd/system/ && sudo systemctl daemon-reload"

echo "   • Template .env VPS → ~/$REMOTE_HOME/"
scp "${SSH_OPTS[@]}" "$LOCAL_DIR/backend/.env.vps.example" "$VPS_USER@$VPS_IP":~/$REMOTE_HOME/
echo "✅ Sync selesai"
echo

# ── 4/4 Aktivasi service & nginx di VPS ──────────────────────
echo "────────────────────────────────────────────────────────────"
echo " [4/4] Aktifkan service & nginx di VPS ..."
echo "────────────────────────────────────────────────────────────"

echo "   • Mengaktifkan systemd unit..."
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" "
  sudo systemctl enable kode-klasifikasi 2>/dev/null
  sudo systemctl start kode-klasifikasi 2>&1
  echo '✅ Service start: \$(systemctl is-active kode-klasifikasi)'
" || echo "⚠️  Gagal start service. Jalankan 'sudo systemctl start kode-klasifikasi' manual"

echo "   • Mengaktifkan nginx site..."
ssh "${SSH_OPTS[@]}" "$VPS_USER@$VPS_IP" "
  sudo ln -sf /etc/nginx/sites-available/kode-klasifikasi.conf /etc/nginx/sites-enabled/ 2>/dev/null
  sudo nginx -t 2>&1 && sudo systemctl reload nginx 2>&1 && echo '✅ Nginx OK' || echo '⚠️ Nginx config error'
" || echo "⚠️  Gagal reload nginx. Periksa konfigurasi manual"

echo
echo "╔══════════════════════════════════════════════════════════╗"
echo "║  ✅ DEPLOY SELESAI                                         ║"
echo "╚══════════════════════════════════════════════════════════╝"
echo ""
echo "1. (PERTAMA KALI) Buat .env backend:"
echo "   ssh $VPS_USER@$VPS_IP"
echo "   cd ~/$REMOTE_HOME && cp .env.vps.example .env && nano .env"
echo "   # isi: DATABASE_URL VPS, GOOGLE_REDIRECT_URI, JWT_SECRET, dll"
echo ""
echo "2. (PERTAMA KALI) Google Console — tambahkan redirect URI:"
echo "   https://liqueur-douche-defuse.ngrok-free.dev/auth/callback"
echo ""
echo "3. Verifikasi:"
echo "   curl http://127.0.0.1:$BACKEND_PORT/api/health"
echo "   curl https://<domain>/api/health"
echo ""
echo "4. Log startup:"
echo "   ssh $VPS_USER@$VPS_IP 'journalctl -u kode-klasifikasi -n 20 --no-pager'
