// Test detektor deterministik "informasi yang dikecualikan" di background.js.
// Mengekstrak blok fungsi dari background.js (via marker) lalu menjalankan
// kasus uji — logika yang diuji SAMA dengan yang berjalan di extension.
//
// Jalankan dari folder extension/:  node test_deteksi.js

const fs = require('fs');
const path = require('path');

const BG = path.join(__dirname, 'background.js');
const src = fs.readFileSync(BG, 'utf8');

const START_MARK = '// ── DETEKSI INFORMASI DIKECUALIKAN (tanpa AI) ─────────────────';
const END_MARK = '// ── DETEKSI SELESAI ───────────────────────────────────────────';

const start = src.indexOf(START_MARK);
const end = src.indexOf(END_MARK);
if (start < 0 || end < 0 || end <= start) {
  console.error('❌ Marker deteksi tidak ditemukan di background.js');
  process.exit(1);
}

// new Function: scope terisolasi, fungsi dideklarasikan di dalam body lalu
// dikembalikan — tidak ada masalah leakage/redeclaration seperti eval biasa.
const code = src.slice(start, end) +
  '\nreturn { deteksiInformasiDikecualikan, countNIK, deteksiKodeRahasia, kodeKandidat };';
const { deteksiInformasiDikecualikan, countNIK, deteksiKodeRahasia, kodeKandidat } = new Function(code)();

if (typeof deteksiInformasiDikecualikan !== 'function' || typeof countNIK !== 'function' ||
    typeof deteksiKodeRahasia !== 'function' || typeof kodeKandidat !== 'function') {
  console.error('❌ Fungsi deteksi tidak terdefinisi');
  process.exit(1);
}

let pass = 0;
let fail = 0;

function expect(teks, harusDiblokir, alasanKunci) {
  const alasan = deteksiInformasiDikecualikan(teks);
  const blocked = alasan.length > 0;
  const ok = blocked === harusDiblokir &&
    (!alasanKunci || alasan.some((a) => a.includes(alasanKunci)));
  if (ok) {
    pass++;
    const label = harusDiblokir ? 'BLOKIR' : 'LULUS';
    console.log(`  ✅ [${label}] ${teks.slice(0, 60).replace(/\n/g, ' ')}`);
  } else {
    fail++;
    console.log(`  ❌ ${teks.slice(0, 60).replace(/\n/g, ' ')}`);
    console.log(`     harusDiblokir=${harusDiblokir}, alasan=${JSON.stringify(alasan)}`);
  }
}

console.log('== Kasus yang HARUS diblokir ==');
expect('NOMOR: 001/RAHASIA/2026\nSANGAT RAHASIA\nKepada Yth...', true, 'SANGAT RAHASIA');
expect('Dokumen ini memuat rahasia negara tentang perbatasan.', true, 'RAHASIA NEGARA');
expect('Sifat: Terbatas\nPerihal: Hasil pemeriksaan BPK', true, 'sifat/klasifikasi');
expect('LAPORAN\n\nRAHASIA\n\nPerihal: Evaluasi kinerja', true, 'stempel');
expect('Berisi informasi yang dikecualikan sesuai UU 14/2008.', true, 'informasi yang dikecualikan');
expect('Materi rahasia jabatan pejabat struktural.', true, 'rahasia jabatan');
expect('Koordinasi intelijen negara terkait perbatasan.', true, 'intelijen negara');
expect('Daftar data nasabah bank pembangunan daerah.', true, 'data nasabah');
expect('Riwayat rekam medis pasien rawat inap.', true, 'rekam medis');
expect('1. 3501010101010001\n2. 3501010101010002\n3. 3501010101010003', true, 'NIK massal');
expect('MEMO INTERNAL\nCONFIDENTIAL\nDistribution limited.', true, 'CONFIDENTIAL');
expect('Dokumen ini bersifat rahasia dan tidak untuk disebarluaskan.', true, 'sifat/klasifikasi');
expect('HANYA UNTUK INTERNAL — dilarang disebarluaskan.', true, 'hanya untuk internal');

console.log('== Kasus yang TIDAK boleh diblokir (anti false-positive) ==');
expect('Permohonan cuti tahunan. Bersama ini saya mohon izin cuti selama 12 hari karena alasan keluarga. Terlampir fotokopi KTP, NIK 3501010101010001. Kami memahami anggaran kami terbatas, mohon maklum.', false);
expect('Rapat ini membahas rahasia tim sukses pemilu.', false);
expect('Kami mohon maaf atas keterbatasan waktu yang kami miliki.', false);
expect('Undangan rapat koordinasi dinas perhubungan tanggal 12 Agustus 2026.', false);
expect('Dengan ini mengajukan permohonan pengadaan laptop untuk unit kerja.', false);
expect('Dokumen ini tidak bersifat rahasia dan dapat disebarluaskan kepada umum.', false);
expect('Surat ini bukan bersifat terbatas, mohon disebarluaskan.', false);
expect('Ref: 12345678901234567\nRef: 23456789012345678\nRef: 34567890123456789', false);

// ── Lapisan 2: kode klasifikasi sensitif per SKKAD ────────────────
const KODE_SET = new Set(['010.03', '010.04.02', '200.02.01', '800.05.04', '900.12.04']);

function expectKode(teks, harusDiblokir, alasanKunci) {
  const alasan = deteksiKodeRahasia(teks, KODE_SET);
  const blocked = alasan.length > 0;
  const ok = blocked === harusDiblokir &&
    (!alasanKunci || alasan.some((a) => a.includes(alasanKunci)));
  if (ok) {
    pass++;
    const label = harusDiblokir ? 'BLOKIR' : 'LULUS';
    console.log(`  ✅ [${label} (kode)] ${teks.slice(0, 60).replace(/\n/g, ' ')}`);
  } else {
    fail++;
    console.log(`  ❌ ${teks.slice(0, 60).replace(/\n/g, ' ')}`);
    console.log(`     harusDiblokir=${harusDiblokir}, alasan=${JSON.stringify(alasan)}`);
  }
}

console.log('== Kasus kode klasifikasi yang HARUS diblokir ==');
expectKode('Lampiran: 010.04.02 Rapat Pimpinan Eselon II dan III.', true, '010.04.02');
expectKode('Nomor: 001/010.03/2026/DISPENDA', true, '010.03'); // kode di nomor surat dinas

expectKode('Rahasia per SKKAD: kode 200.02.01 fasilitasi intelijen.', true, '200.02.01');

console.log('== Kasus kode yang TIDAK boleh diblokir (anti false-positive) ==');
expectKode('Anggaran Rp 20.000.000 untuk kegiatan pelatihan.', false);
expectKode('Dikeluarkan tanggal 10.03.2026 di Malang.', false); // tanggal, bukan kode

expectKode('Kode: 010.04.02 ada tapi kode lain: 999.99.99 tidak ada.', true, '010.04.02');

expectKode('Permohonan cuti tahunan pegawai.', false); // tidak ada kode sama sekali

// ── Test langsung kodeKandidat (sinkronisasi dengan Rust kode_kandidat) ──
function expectKandidat(teks, harusAda, kodeKunci) {
  const kandidat = kodeKandidat(teks);
  const ada = kandidat.includes(kodeKunci);
  const ok = ada === harusAda;
  if (ok) {
    pass++;
    console.log(`  ✅ [kandidat] ${teks.slice(0, 55).replace(/\n/g, ' ')} → ${JSON.stringify(kandidat)}`);
  } else {
    fail++;
    console.log(`  ❌ [kandidat] ${teks.slice(0, 55).replace(/\n/g, ' ')} → ${JSON.stringify(kandidat)}`);
    console.log(`     harusAda=${harusAda}, kodeKunci=${kodeKunci}`);
  }
}

console.log('== Ekstraksi kandidat kode (kodeKandidat) ==');
expectKandidat('Perihal: 010.04.02 Rapat Pimpinan', true, '010.04.02');
expectKandidat('No: 010 hanya tiga digit', false, '010'); // 3 digit tunggal bukan kode
expectKandidat('Tanggal 10.03.2026 di Malang', false, '10.03'); // tanggal → kandidat maximal diikuti digit
expectKandidat('Anggaran Rp 20.000.000', false, '20.000'); // uang → kandidat maximal 3 segmen

console.log(`\nHasil: ${pass} lolos, ${fail} gagal`);
process.exit(fail === 0 ? 0 : 1);
