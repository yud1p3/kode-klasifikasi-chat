#!/usr/bin/env python3
"""Gabungkan kolom metadata dari skkad.xlsx ke klasifikasi_arsip.csv.

- Join kunci: `id` (unik di kedua file; 5533/5534 match)
- Resolve: penyusutan_akhir_id & klasifikasi_keamanan_id -> teks nama (sheet Keterangan)
- Nilai khusus: string 'null' -> kosong; artifact '_x000D_' dibersihkan
- Record CSV tanpa pasangan di skkad (mis. 187823) -> kolom baru kosong
- Output: klasifikasi_arsip_lengkap.csv (file asli TIDAK ditimpa)
"""
import pandas as pd
import re

BASE = '.'

# ---------- 1. Baca file ----------
csv = pd.read_csv(f'{BASE}/klasifikasi_arsip.csv', dtype=str)
sk = pd.read_excel(f'{BASE}/skkad.xlsx', sheet_name='skkad', dtype=str)

# ---------- 2. Mapping resolve ID -> teks (sheet Keterangan) ----------
PENYUSUTAN = {'0': '-', '1': 'Musnah', '2': 'Permanen', '4': 'Dinilai Kembali'}
KLASIFIKASI = {'0': '-', '1': 'Terbuka', '2': 'Rahasia', '3': 'Sangat Rahasia', '4': 'Terbatas'}


def clean(v):
    """Normalisasi nilai: NaN/None/'null' -> '', artifact _x000D_ dibuang, strip."""
    if v is None:
        return ''
    s = str(v)
    if s.strip().lower() == 'null' or s.strip() == 'nan':
        return ''
    s = re.sub(r'_x000D_', '', s, flags=re.IGNORECASE)
    return s.strip()


# ---------- 3. Siapkan kolom tambahan dari skkad ----------
sk_extra = sk[['id', 'parent_id', 'retensi_aktif', 'retensi_inaktif',
               'penyusutan_akhir_id', 'klasifikasi_keamanan_id', 'pertimbangan']].copy()
sk_extra['penyusutan_akhir'] = sk_extra['penyusutan_akhir_id'].map(PENYUSUTAN).fillna('')
sk_extra['klasifikasi_keamanan'] = sk_extra['klasifikasi_keamanan_id'].map(KLASIFIKASI).fillna('')
for c in ['parent_id', 'retensi_aktif', 'retensi_inaktif', 'pertimbangan']:
    sk_extra[c] = sk_extra[c].map(clean)

# ---------- 4. Merge (left join by id, pertahankan urutan & isi CSV) ----------
result = csv.merge(
    sk_extra[['id', 'parent_id', 'retensi_aktif', 'retensi_inaktif',
              'penyusutan_akhir', 'klasifikasi_keamanan', 'pertimbangan']],
    on='id', how='left'
)
for c in ['parent_id', 'retensi_aktif', 'retensi_inaktif',
          'penyusutan_akhir', 'klasifikasi_keamanan', 'pertimbangan']:
    result[c] = result[c].fillna('')

# ---------- 5. Tulis output ----------
out = f'{BASE}/klasifikasi_arsip_lengkap.csv'
result.to_csv(out, index=False, lineterminator='\r\n', encoding='utf-8')

# ---------- 6. Ringkasan ----------
print('=== HASIL MERGE ===')
print('Baris output:', len(result))
print('Kolom:', list(result.columns))
print()
print('Klasifikasi keamanan (teks):')
print(result['klasifikasi_keamanan'].value_counts(dropna=False).to_string())
print()
print('Penyusutan akhir (teks):')
print(result['penyusutan_akhir'].value_counts(dropna=False).to_string())
print()
print('pertimbangan terisi:', (result['pertimbangan'] != '').sum(), '/', len(result))
print()
print('Record tanpa data skkad (kolom baru kosong):',
      (result['klasifikasi_keamanan'] == '').sum())
print()
print('Contoh baris (3 pertama):')
print(result.head(3).to_string())
print()
print('Contoh kode 440.02.02.04.01 (duplikat di skkad):')
print(result[result['kode'] == '440.02.02.04.01'].to_string())
print()
print('Record 187823 (AGRARIA, tanpa skkad):')
print(result[result['id'] == '187823'].to_string())
print()
print('File ditulis ke:', out)
