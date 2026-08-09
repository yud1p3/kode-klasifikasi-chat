#!/usr/bin/env python3
"""Simpan kolom metadata SKKAD dari klasifikasi_arsip_lengkap.csv ke tabel klasifikasi_embedding.

- ALTER TABLE: tambah kolom parent_id, retensi_aktif, retensi_inaktif,
  penyusutan_akhir, klasifikasi_keamanan, pertimbangan (idempoten)
- UPDATE: join by id; nilai kosong di CSV -> NULL di DB
- Aman dijalankan ulang (tidak merusak data yang sudah ada)
"""
import csv
import os
import psycopg2

DB_URL = os.environ.get('DATABASE_URL',
                        'postgres://postgres:postgres@localhost:5432/klasifikasi_arsip')
CSV_PATH = 'klasifikasi_arsip_lengkap.csv'

# Kolom: (nama_db, tipe)  -- tipe untuk ALTER TABLE
NEW_COLUMNS = [
    ('parent_id', 'integer'),
    ('retensi_aktif', 'integer'),
    ('retensi_inaktif', 'integer'),
    ('penyusutan_akhir', 'text'),
    ('klasifikasi_keamanan', 'text'),
    ('pertimbangan', 'text'),
]

# Kolom CSV yang akan di-update (sama nama dengan kolom DB)
DATA_COLUMNS = [c[0] for c in NEW_COLUMNS]


def to_null(v):
    """String kosong -> None, selain itu dikembalikan apa adanya (strip)."""
    if v is None:
        return None
    s = str(v).strip()
    return s if s else None


def main():
    # 1. Baca CSV
    with open(CSV_PATH, newline='', encoding='utf-8') as f:
        rows = list(csv.DictReader(f))
    print(f'CSV dibaca: {len(rows)} baris')

    # 2. Koneksi
    conn = psycopg2.connect(DB_URL)
    conn.autocommit = False
    cur = conn.cursor()

    try:
        # 3. ALTER TABLE (idempoten)
        for col, typ in NEW_COLUMNS:
            cur.execute(f'ALTER TABLE klasifikasi_embedding ADD COLUMN IF NOT EXISTS {col} {typ}')
        print('ALTER TABLE selesai (kolom baru siap)')

        # 4. UPDATE per baris (parameterized; nilai kosong -> NULL)
        set_clause = ', '.join(f'{c} = %s' for c in DATA_COLUMNS)
        sql = f'UPDATE klasifikasi_embedding SET {set_clause} WHERE id = %s'
        updated = 0
        for r in rows:
            try:
                row_id = int(r['id'])
            except (TypeError, ValueError):
                continue
            vals = [to_null(r.get(c)) for c in DATA_COLUMNS]
            cur.execute(sql, (*vals, row_id))
            updated += cur.rowcount

        conn.commit()
        print(f'UPDATE selesai: {updated} baris ter-update')
    except Exception as e:
        conn.rollback()
        print(f'ERROR: {e}')
        raise
    finally:
        cur.close()
        conn.close()


if __name__ == '__main__':
    main()
