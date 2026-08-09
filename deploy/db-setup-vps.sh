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

# ── STEP4b: migrasi kolom metadata SKKAD (guard + retensi + penyusutan) ──
# Kolom ini dipakai backend BARU (dikecualikan::deteksi_kode, endpoint
# /api/dikecualikan/kode-rahasia, dan metadata retensi/penyusutan di hasil).
# Dump DB yang dibuat SETELAH migrasi lokal sudah berisi kolom+data; blok ini
# aman (IF NOT EXISTS) untuk dump lama yang belum punya.
echo "== STEP4b: migrasi kolom metadata SKKAD =="
# Direktori dipakai untuk log & CSV backfill — jamin ada (defensif, terutama saat run ulang)
mkdir -p /tmp/kkl-migrate
# Output ke log (bukan /dev/null) agar pesan error yang ditampilkan benar-benar ada:
sudo -n -u postgres psql -d klasifikasi_arsip -v ON_ERROR_STOP=1 \
  > /tmp/kkl-migrate/step4b_alter.log 2>&1 <<'SQL' && echo '✅ kolom metadata SKKAD siap (ALTER TABLE)' \
  || { echo '⚠️  ALTER TABLE metadata SKKAD gagal:'; tail -5 /tmp/kkl-migrate/step4b_alter.log; }
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS parent_id integer;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS retensi_aktif integer;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS retensi_inaktif integer;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS penyusutan_akhir text;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS klasifikasi_keamanan text;
ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS pertimbangan text;
SQL

# Backfill data metadata SKKAD bila CSV disediakan (untuk dump LAMA yang
# kolomnya kosong). Kirim file ke VPS: scp klasifikasi_arsip_lengkap.csv
# root@VPS:/tmp/kkl-migrate/  lalu jalankan ulang script ini.
# PENTING — format CSV (dari tools/gabung_skkad.py) persis 10 kolom:
#   id,kode,deskripsi,path,parent_id,retensi_aktif,retensi_inaktif,
#   penyusutan_akhir,klasifikasi_keamanan,pertimbangan
# \copy memetakan kolom by-position (bukan by-nama), jadi temp table harus
# punya 10 kolom dengan urutan SAMA seperti header CSV. Di COPY format CSV,
# string kosong otomatis jadi NULL (aman untuk record tanpa data skkad).
# CSV harus readable oleh user postgres (\copy jalan sebagai postgres):
#   chmod 644 /tmp/kkl-migrate/klasifikasi_arsip_lengkap.csv
if [ -f /tmp/kkl-migrate/klasifikasi_arsip_lengkap.csv ]; then
  echo '   • CSV SKKAD ditemukan — backfill metadata...'
  # Satu sesi psql: buat temp table → \copy → UPDATE → (temp drop saat sesi tutup)
  sudo -n -u postgres psql -d klasifikasi_arsip -v ON_ERROR_STOP=1 \
    > /tmp/kkl-migrate/step4b_backfill.log 2>&1 <<'SQL' && echo '✅ backfill metadata SKKAD selesai' \
    || { echo '⚠️  backfill gagal — log:'; tail -8 /tmp/kkl-migrate/step4b_backfill.log; }
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
else
  echo 'ℹ️  CSV SKKAD tidak ditemukan di /tmp/kkl-migrate/ — kolom baru kosong'
  echo '   (dump DB baru sudah berisi data; hanya perlu CSV bila restore dari dump lama)'
fi

# Verifikasi kolom metadata
sudo -n -u postgres psql -d klasifikasi_arsip -tAc \
  "SELECT 'metadata_terisi: ' || count(*) || ' / ' || (SELECT count(*) FROM klasifikasi_embedding) FROM klasifikasi_embedding WHERE klasifikasi_keamanan IS NOT NULL AND klasifikasi_keamanan <> ''"
echo -n "embedding_rows: " && sudo -n -u postgres psql -d klasifikasi_arsip -tAc "SELECT count(*) FROM klasifikasi_embedding"
echo -n "feedback_rows:  " && sudo -n -u postgres psql -d klasifikasi_arsip -tAc "SELECT count(*) FROM klasifikasi_feedback"
echo -n "vector_ext:     " && sudo -n -u postgres psql -d klasifikasi_arsip -tAc "SELECT extversion FROM pg_extension WHERE extname='vector'"
echo "== ALL_DONE =="
