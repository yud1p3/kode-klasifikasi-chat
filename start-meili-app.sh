#!/usr/bin/env bash
# ============================================================
#  Start/Stop Aplikasi Kode Klasifikasi — versi Meilisearch
#  Backend :3100 (search via Meilisearch) + Frontend :5174
#  Dipakai: bash start-meili-app.sh [start|stop|status|restart]
# ============================================================
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_LOG=/tmp/meili-backend.log
FRONTEND_LOG=/tmp/meili-frontend.log
BACKEND_PID=/tmp/meili-backend.pid
FRONTEND_PID=/tmp/meili-frontend.pid

is_alive() {
  local pidfile="$1"
  [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null
}

start_one() {
  local name="$1" workdir="$2" cmd="$3" log="$4" pidfile="$5"
  if is_alive "$pidfile"; then
    echo "⚠️  $name sudah berjalan (pid $(cat "$pidfile"))"
    return 0
  fi
  # setsid: dilepas dari process group shell agar tidak ikut mati.
  # `exec` memastikan PID setsid == PID proses sebenarnya (tersimpan di pidfile).
  setsid bash -c "cd '$workdir' && exec $cmd" </dev/null >"$log" 2>&1 &
  echo $! > "$pidfile"
  echo "▶️  $name di-start (pid $(cat "$pidfile"), log: $log)"
}

stop_one() {
  local name="$1" pidfile="$2"
  if is_alive "$pidfile"; then
    kill "$(cat "$pidfile")" 2>/dev/null && echo "⏹️  $name dihentikan" || echo "⚠️  $name gagal dihentikan"
  else
    echo "$name tidak berjalan"
  fi
  rm -f "$pidfile"
}

start() {
  echo "== Memulai aplikasi Meilisearch =="
  start_one "backend :3100" "$DIR/backend" "./target/debug/kode-klasifikasi-chat" "$BACKEND_LOG" "$BACKEND_PID"
  start_one "frontend :5174" "$DIR/frontend" "env VITE_API_URL=http://localhost:3100 ./node_modules/.bin/vite --port 5174 --strictPort" "$FRONTEND_LOG" "$FRONTEND_PID"
  sleep 4
  status
}

stop() {
  echo "== Menghentikan aplikasi Meilisearch =="
  stop_one "backend :3100" "$BACKEND_PID"
  stop_one "frontend :5174" "$FRONTEND_PID"
}

status() {
  echo "== Status =="
  curl -s -m 3 http://localhost:3100/api/health >/dev/null 2>&1 && echo "✅ backend     : http://localhost:3100 (Meilisearch)" || echo "❌ backend     : :3100 tidak merespon"
  curl -s -m 3 http://localhost:5174 >/dev/null 2>&1 && echo "✅ frontend    : http://localhost:5174" || echo "❌ frontend    : :5174 tidak merespon"
  curl -s -m 3 http://localhost:7700/health >/dev/null 2>&1 && echo "✅ meilisearch : http://localhost:7700" || echo "❌ meilisearch : :7700 tidak merespon"
}

case "${1:-start}" in
  start)   start ;;
  stop)    stop ;;
  status)  status ;;
  restart) stop; sleep 1; start ;;
  *) echo "Gunakan: $0 [start|stop|status|restart]"; exit 1 ;;
esac
