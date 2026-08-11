#!/usr/bin/env python3
"""Perbaiki parent_id di klasifikasi_embedding agar selaras dengan data SKKAD.

Latar belakang: beberapa record di klasifikasi_embedding (PostgreSQL) punya
parent_id yang TIDAK konsisten dengan data SKKAD — sebagian bernilai tidak
valid (0/NULL padahal kode bertitik), sebagian menunjuk induk yang salah.
Sumber kebenaran: CSV SKKAD sumber (mis. klasifikasi_arsip_2026-07-02.csv)
yang isinya SAMA dengan indeks Meilisearch index 'klasifikasi' pada aplikasi
browser-klasifikasi-arsip (data yang sudah pernah dikoreksi manual).

Dua lapis perbaikan:
  1. parent_id INVALID (0/NULL) pada kode bertitik → cari induk dari pola kode
     (kode minus segmen terakhir), hanya bila kode induk UNIK di dataset.
  2. parent_id berbeda dari CSV sumber → ikuti CSV, TAPI hanya bila induk di
     CSV sesuai pola kode (kode induk = kode minus segmen terakhir). Ini
     mencegah menyentuh record yang struktur SKKAD-nya memang tidak mengikuti
     pola kode ketat (mis. 185597, 185624 — konsisten di semua sumber).

Pemakaian:
  python3 tools/perbaiki_parent_id.py [--csv PATH]           # dry-run
  python3 tools/perbaiki_parent_id.py --apply [--csv PATH]   # terapkan
  # PATH default: /home/yudi/klasifikasi_arsip_2026-07-02.csv
"""
import argparse
import csv
import os
import sys

import psycopg2

DB_URL = os.environ.get(
    'DATABASE_URL',
    'postgres://postgres:postgres@localhost:5432/klasifikasi_arsip',
)
DEFAULT_CSV = '/home/yudi/klasifikasi_arsip_2026-07-02.csv'


def parent_kode(kode: str) -> str | None:
    """Kode induk = kode tanpa segmen terakhir. None bila kode level-1."""
    if '.' not in kode:
        return None
    return kode.rsplit('.', 1)[0]


def muat_csv(path: str) -> dict[int, int]:
    """CSV SKKAD (delimiter ';') → {id: parent_id}. Nilai 0/NULL → 0 (root)."""
    out: dict[int, int] = {}
    with open(path, newline='', encoding='utf-8') as f:
        reader = csv.DictReader(f, delimiter=';')
        for row in reader:
            try:
                rid = int(row['id'])
                pid = row['parent_id'].strip()
                out[rid] = int(pid) if pid not in ('', '0') else 0
            except (ValueError, KeyError):
                continue
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description='Perbaiki parent_id klasifikasi_embedding (selaras SKKAD)')
    parser.add_argument('--apply', action='store_true', help='Jalankan perubahan (default: dry-run)')
    parser.add_argument('--csv', default=DEFAULT_CSV, help='Path CSV SKKAD sumber')
    args = parser.parse_args()

    if not os.path.exists(args.csv):
        print(f'❌ CSV tidak ditemukan: {args.csv}')
        return 1

    csv_parent = muat_csv(args.csv)
    print(f'📄 CSV sumber: {args.csv} ({len(csv_parent)} baris)')

    conn = psycopg2.connect(DB_URL)
    conn.autocommit = True
    cur = conn.cursor()
    cur.execute("SELECT id, kode, parent_id FROM klasifikasi_embedding")
    rows = cur.fetchall()
    pg = {r[0]: {'kode': r[1], 'parent_id': r[2]} for r in rows}
    print(f'🗄️  PostgreSQL: {len(pg)} baris\n')

    # Hitung kemunculan kode (deteksi duplikat) + map kode unik → id
    from collections import Counter
    kode_cnt = Counter(info['kode'] for info in pg.values())
    kode_to_id = {
        info['kode']: i for i, info in pg.items() if kode_cnt[info['kode']] == 1
    }

    rencana: list[tuple[int, str, int | None, str]] = []  # id, kode, parent_baru, sumber
    dilewati: list[tuple[int, str, str]] = []

    for rid, kode, cur_p_raw in rows:
        if '.' not in kode:
            continue  # level-1 tidak punya induk
        pk = parent_kode(kode)
        cur_p = cur_p_raw or 0

        # ── Lapis 1: parent_id invalid (0/NULL) → dari pola kode ──
        if cur_p == 0:
            if pk in kode_to_id:
                rencana.append((rid, kode, kode_to_id[pk], 'pola kode (invalid)'))
            else:
                dilewati.append((rid, kode, f'parent kode {pk} duplikat/tidak ada'))
            continue

        # ── Lapis 2: beda dari CSV → ikuti CSV bila sesuai pola kode ──
        csv_p = csv_parent.get(rid)
        if csv_p is None or csv_p == cur_p:
            continue
        # Verifikasi induk di CSV sesuai pola kode
        if pk in kode_to_id and csv_p == kode_to_id[pk]:
            rencana.append((rid, kode, csv_p, 'CSV sumber'))
        else:
            dilewati.append((rid, kode, f'CSV parent {csv_p} ≠ pola kode ({pk}) — struktur SKKAD asli, tidak diubah'))

    if not rencana:
        print('✅ Tidak ada parent_id yang perlu diperbaiki.')
        cur.close()
        conn.close()
        return 0

    print(f'Rencana perbaikan ({len(rencana)} record):\n')
    for rid, kode, pid, sumber in rencana:
        print(f'  id={rid:<8} kode={kode:<22} parent_id → {str(pid):<8} ({sumber})')

    if dilewati:
        print(f'\nℹ️  {len(dilewati)} record dilewati (tidak diubah):')
        for rid, kode, alasan in dilewati:
            print(f'  id={rid:<8} kode={kode:<22} {alasan}')

    if not args.apply:
        print('\n(dry-run — tidak ada perubahan. Jalankan dengan --apply untuk menerapkan)')
        cur.close()
        conn.close()
        return 0

    print('\nMenerapkan perubahan...')
    n = 0
    for rid, _kode, pid, _sumber in rencana:
        cur.execute('UPDATE klasifikasi_embedding SET parent_id = %s WHERE id = %s', (pid, rid))
        n += cur.rowcount
    print(f'✅ {n} record diperbarui.')

    # Verifikasi
    cur.execute("""
        SELECT count(*) FROM klasifikasi_embedding
        WHERE position('.' in kode) > 0 AND (parent_id IS NULL OR parent_id = 0)
    """)
    print(f'  Sisa parent_id invalid (0/NULL pada kode bertitik): {cur.fetchone()[0]}')

    cur.close()
    conn.close()
    return 0


if __name__ == '__main__':
    sys.exit(main())
