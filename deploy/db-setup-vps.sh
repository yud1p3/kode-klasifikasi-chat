#!/usr/bin/env bash
# ============================================================
# SETUP DATABASE VPS — kode-klasifikasi-meili
# Jalankan di VPS:  bash db-setup-vps.sh   (bisa via nohup)
#   • Install pgvector (apt; fallback build dari source)
#   • Buat role kklas + database klasifikasi_arsip + ext vector
#   • Restore dump  /tmp/kkl-migrate/klasifikasi_arsip.sql
#   • GRANT akses ke kklas + verifikasi jumlah baris
# Butuh: sudoers NOPASSWD (systemctl/apt/psql-postgres) — lihat PANDUAN_DEPLOY_VPS.md
# ============================================================
set -u
export DEBIAN_FRONTEND=noninteractive

echo "== STEP1: install pgvector =="
sudo -n apt-get install -y -qq postgresql-16-pgvector >/dev/null 2>&1
if [ -f /usr/share/postgresql/16/extension/vector.control ]; then
  echo '✅ pgvector terpasang (apt)'
else
  echo '⏳ apt tidak punya pgvector — build dari source...'
  sudo -n apt-get install -y -qq build-essential git postgresql-server-dev-16 >/dev/null 2>&1
  rm -rf /tmp/pgvector-src
  git clone --depth 1 https://github.com/pgvector/pgvector.git /tmp/pgvector-src >/dev/null 2>&1
  (cd /tmp/pgvector-src && make -j"$(nproc)" >/dev/null 2>&1 && sudo -n make install >/dev/null 2>&1)
fi
ls -l /usr/share/postgresql/16/extension/vector.control && echo '✅ vector.control tersedia' || echo '❌ vector.control TIDAK ada'

echo "== STEP2: role kklas + database + extension =="
if ! sudo -n -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname='kklas'" | grep -q 1; then
  DB_PASS=$(openssl rand -base64 18 | tr -d '/+=' | head -c 24)
  sudo -n -u postgres psql <<SQL
CREATE ROLE kklas LOGIN PASSWORD '$DB_PASS';
CREATE DATABASE klasifikasi_arsip OWNER kklas;
SQL
  sudo -n -u postgres psql -d klasifikasi_arsip -c "CREATE EXTENSION IF NOT EXISTS vector" >/dev/null
  umask 077
  echo "$DB_PASS" > /tmp/kkl-migrate/dbpass.txt
  echo '✅ role kklas + database dibuat (password di /tmp/kkl-migrate/dbpass.txt, chmod 600)'
else
  echo 'ℹ️  role kklas sudah ada'
fi

echo "== STEP3: restore dump =="
grep -vE 'CREATE EXTENSION IF NOT EXISTS vector|COMMENT ON EXTENSION vector|ALTER EXTENSION vector' \
  /tmp/kkl-migrate/klasifikasi_arsip.sql > /tmp/kkl-migrate/klasifikasi_arsip_clean.sql
sudo -n -u postgres psql -d klasifikasi_arsip -v ON_ERROR_STOP=0 -f /tmp/kkl-migrate/klasifikasi_arsip_clean.sql > /tmp/kkl-migrate/restore.log 2>&1
echo "⚠️  pesan error di restore (boleh kosong):"
grep -iE 'ERROR|FATAL' /tmp/kkl-migrate/restore.log | grep -viE 'does not exist|already exists' | head -5 || echo '  (tidak ada error signifikan)'

echo "== STEP4: grants + verifikasi =="
sudo -n -u postgres psql -d klasifikasi_arsip -c "GRANT ALL ON ALL TABLES IN SCHEMA public TO kklas; GRANT ALL ON ALL SEQUENCES IN SCHEMA public TO kklas; GRANT USAGE, CREATE ON SCHEMA public TO kklas;" >/dev/null
# Migrasi skema terbaru (kolom chat_id utk sesi anonim — dump lama belum punya):
sudo -n -u postgres psql -d klasifikasi_arsip -c "ALTER TABLE klasifikasi_feedback ADD COLUMN IF NOT EXISTS chat_id text" >/dev/null 2>&1 && echo '✅ chat_id column siap'
echo -n "embedding_rows: " && sudo -n -u postgres psql -d klasifikasi_arsip -tAc "SELECT count(*) FROM klasifikasi_embedding"
echo -n "feedback_rows:  " && sudo -n -u postgres psql -d klasifikasi_arsip -tAc "SELECT count(*) FROM klasifikasi_feedback"
echo -n "vector_ext:     " && sudo -n -u postgres psql -d klasifikasi_arsip -tAc "SELECT extversion FROM pg_extension WHERE extname='vector'"
echo "== ALL_DONE =="
