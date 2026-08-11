# Audit Data Klasifikasi — Relasi & Duplikat

Dokumen ini merangkum hasil audit data tabel `klasifikasi_embedding` (5.534 baris, dataset SKKAD 2026-07-02) setelah fitur **Browse Klasifikasi** diintegrasikan ke aplikasi ini (Agustus 2026). Audit membandingkan tiga sumber: **PostgreSQL** (sumber data aplikasi ini), **indeks Meilisearch** aplikasi lama `browser-klasifikasi-arsip`, dan **CSV sumber** `klasifikasi_arsip_2026-07-02.csv`.

---

## 1. Relasi parent–child: sudah benar ✓

Pemeriksaan integritas relasi di PostgreSQL:

| Pemeriksaan | Hasil |
|---|---|
| Anak yatim (parent_id menunjuk id yang tidak ada) | **0** |
| Self-cycle (parent_id = id sendiri) | **0** |
| Induk level > anak (relasi terbalik) | **0** |
| `parent_id=0` dengan kode dalam (level > 1) | **0** |
| Root dengan kode ≠ 3 digit | **0** |

Struktur final:

- **45 root** — semua kode level-1 3 digit (010 s/d 900); satu-satunya yang memakai `NULL` adalah `590` AGRARIA (setara `parent_id=0` di Meilisearch, keduanya berarti root)
- **5.489 anak** dengan induk valid → total **5.534** = seluruh dataset
- Setiap record dapat ditelusuri naik ke akar (breadcrumb) dan setiap induk menampilkan anak-anaknya — tanpa titik putus

### Riwayat perbaikan

Perbandingan PG vs Meilisearch (yang sudah dibetulkan di aplikasi lama) menemukan **28 record** dengan `parent_id` salah di PostgreSQL:

- **8 record** anomali `parent_id=0` untuk kode dalam (mis. `590.01` yang tadinya tidak terhubung ke `590`)
- **20 record** menunjuk induk yang keliru (mengikuti pola kode yang salah); Meilisearch & CSV sudah benar

Semua **28 record** diselaraskan ke `parent_id` yang benar dari **CSV sumber** (identik dengan Meilisearch) lewat script `tools/perbaiki_parent_id.py`. Setelah perbaikan, PG vs Meilisearch tersisa 1 perbedaan yang bukan masalah nyata: `590` memakai `NULL` (PG) vs `0` (Meili) — keduanya berarti root.

> **Sengaja tidak diubah** — `185597` (`200.05.01.01.02`) dan `185624` (`330.01.01.02`): konsisten di ketiga sumber (PG = Meili = CSV asli). Ini struktur SKKAD asli yang memang tidak mengikuti pola kode (mis. `330.01.01` memang tidak ada di dataset, sehingga `330.01.01.02` menunjuk `330.01`).

---

## 2. Duplikat kode: 12 kode / 24 record — asli SKKAD, dibiarkan

**12 kode duplikat** (24 record, ~0,4% dari dataset). Semuanya **duplikat sungguhan**: kode sama di bawah **induk yang sama** dengan deskripsi berbeda.

| Kode | Deskripsi A | Deskripsi B |
|---|---|---|
| `027.04.02.01` | Penyelenggaraan Diklat | Sistem informasi |
| `440.03.03.04` | Pengendalian Filariasis dan Kecacingan | Pengendalian Penyakit Tidak Menular |
| `440.03.03.04.01` | Filariasis | Pengendalian Penyakit Jantung dan Pembuluh Darah |
| `440.03.03.04.02` | Kecacingan | Pengendalian Diabetes Melitus dan Penyakit Metabolik |
| `440.03.03.04.03` | Schistosomiasis | Pengendalian Penyakit Kanker |
| `440.03.03.05` | Pengendalian Vektor | Penyehatan Lingkungan |
| `440.03.03.05.02.01` | Higiene sanitasi dan Bangunan Umum | Dampak perubahan iklim terhadap kesehatan |
| `440.04.01.01.01` | Gerakan Nasional Sadar Gizi | Pemantauan Pertumbuhan Anak (posyandu) |
| `510.02.01.05.01` | Pengecer | Pemasok |
| `523.02.02.01.01` | rancang bangun kapal perikanan | kelaikan kapal perikanan |
| `523.03.05.04.01` | kelembagaan | ketenagakerjaan |
| `662.02.02.05` | Zoonosis dan Kesejahteraan Hewan | Ternak Kambing Perah |

**Keputusan: dibiarkan apa adanya.** Alasan:

- Duplikat ini **konsisten di ketiga sumber** (PostgreSQL = Meilisearch = CSV) — artinya ini **struktur asli SKKAD**, bukan kesalahan impor maupun kualitas data lokal
- Indeks Meilisearch yang sudah dibetulkan di aplikasi lama **tidak** membersihkan duplikat kode — kedua entri memang sah dan sering punya anak sendiri-sendiri yang valid
- Dampak di UI minimal: kedua kartu tampil berdampingan di halaman browse dan dibedakan dari judul deskripsinya — sama seperti aplikasi lama
- Navigasi aman: semua operasi (React key, breadcrumb, tombol Induk/Sub Klas) memakai `id` unik, bukan kode

---

## 3. Cara menjalankan ulang audit / perbaikan

Script `tools/perbaiki_parent_id.py` menyelaraskan `parent_id` ke CSV sumber:

```bash
# Dry-run (default): tampilkan rencana tanpa mengubah database
python3 tools/perbaiki_parent_id.py

# Terapkan perbaikan
python3 tools/perbaiki_parent_id.py --apply
```

> Ditujukan untuk database **lokal**. Untuk VPS, jalankan dengan kredensial/koneksi database produksi (sesuaikan koneksi di bagian atas script atau set `DATABASE_URL`).

Query audit mandiri (contoh — anak yatim):

```sql
SELECT e.id, e.kode, e.parent_id
FROM klasifikasi_embedding e
LEFT JOIN klasifikasi_embedding p ON p.id = e.parent_id
WHERE e.parent_id IS NOT NULL AND e.parent_id != 0 AND p.id IS NULL;
```
