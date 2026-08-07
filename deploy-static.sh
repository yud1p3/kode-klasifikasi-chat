#!/usr/bin/env bash
# ============================================================
#  Deploy frontend statis kode-klasifikasi-meili ke nginx
#  (domain ngrok stamina-deepen-activist.ngrok-free.dev)
#
#  Build → salin dist ke /var/www/kode-klasifikasi-meili → reload nginx
#  Catatan: build TANPA VITE_API_URL agar API memakai path relatif
#  (same-origin, /api/* diproxy nginx ke backend :3100).
#
#  Pemakaian: bash deploy-static.sh
# ============================================================
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEBROOT=/var/www/kode-klasifikasi-meili

echo "== 1/3 Build frontend (relatif API) =="
(
  cd "$DIR/frontend"
  unset VITE_API_URL
  npm run build
)

echo "== 2/3 Salin dist → $WEBROOT =="
sudo mkdir -p "$WEBROOT"
sudo cp -r "$DIR/frontend/dist/." "$WEBROOT/"
sudo chmod -R a+rX "$WEBROOT"

echo "== 3/3 Reload nginx =="
sudo nginx -t
sudo systemctl reload nginx 2>/dev/null || sudo nginx -s reload

echo "✅ Deploy selesai — cek: https://stamina-deepen-activist.ngrok-free.dev"
