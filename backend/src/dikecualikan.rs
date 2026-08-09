//! Deteksi deterministik (TANPA AI) indikasi naskah berisi informasi yang
//! dikecualikan / berklasifikasi rahasia — dijalankan SEBELUM teks dikirim ke
//! Gemini. Aturan disusun KONSERVATIF untuk meminimalkan false-positive pada
//! naskah dinas biasa: satu NIK/KTP pemohon adalah hal normal dan TIDAK
//! diblokir; kata "rahasia"/"terbatas" lowercase biasa juga TIDAK diblokir.
//!
//! Dasar hukum: Pasal 17 UU No. 14/2008 tentang Keterbukaan Informasi Publik
//! jo. PP No. 61/2010 (informasi publik yang dikecualikan).

/// Normalisasi: huruf kapital + seluruh deretan whitespace jadi satu spasi.
/// Dipakai untuk aturan berbasis frasa (case-insensitive).
fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.extend(c.to_uppercase());
            prev_space = false;
        }
    }
    out
}

/// Token kata ASCII (alphanumeric) dari teks ASLI — dipakai untuk deteksi
/// stempel kapital (RAHASIA/TERBATAS ditulis kapital berdiri sendiri).
fn tokens(teks: &str) -> impl Iterator<Item = &str> + '_ {
    teks.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
}

/// Ambil maks `len` karakter setelah kemunculan pertama `kw` di teks ternormalisasi.
fn window_after(n: &str, kw: &str, len: usize) -> String {
    match n.find(kw) {
        Some(i) => n[i + kw.len()..].chars().take(len).collect(),
        None => String::new(),
    }
}

/// Hitung jumlah run angka sepanjang tepat 16 digit (pola NIK) pada teks asli.
fn count_nik(teks: &str) -> usize {
    let mut count = 0usize;
    let mut run = 0usize;
    for c in teks.chars() {
        if c.is_ascii_digit() {
            run += 1;
        } else {
            if run == 16 {
                count += 1;
            }
            run = 0;
        }
    }
    if run == 16 {
        count += 1;
    }
    count
}

fn push_alasan(alasan: &mut Vec<String>, r: &str) {
    if !alasan.iter().any(|a| a == r) {
        alasan.push(r.to_string());
    }
}

/// Ekstrak kandidat kode klasifikasi dari teks naskah.
///
/// Pola: segmen digit 1–3 karakter yang dipisah titik, MINIMAL 2 segmen
/// (dataset SKKAD tidak punya kode 3-digit berklasifikasi). Scan "maximal
/// munch" dengan batas kata: kode dianggap utuh hanya bila diapit karakter
/// non-digit. Ini menghindari false-positive pada angka biasa (mis.
/// "20.000.000" atau nomor surat "010.03/2026" → "010.03" bukan batas kata
/// karena diikuti titik+digit).
fn kode_kandidat(teks: &str) -> Vec<String> {
    let b = teks.as_bytes();
    let n = b.len();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < n {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // Batas kiri: karakter sebelum digit awal bukan digit (kode diawali kata)
        if i > 0 && b[i - 1].is_ascii_digit() {
            i += 1;
            continue;
        }
        // Segmen pertama: 1–3 digit
        let mut j = i;
        let mut seg_len = 0usize;
        while j < n && b[j].is_ascii_digit() && seg_len < 3 {
            j += 1;
            seg_len += 1;
        }
        let mut kode = String::new();
        kode.push_str(&teks[i..j]);
        let mut segmen = 1usize;
        let mut k = j;
        // Segmen berikutnya: '.' + 1–3 digit
        while k < n && b[k] == b'.' && k + 1 < n && b[k + 1].is_ascii_digit() {
            let s = k + 1;
            let mut m = s;
            let mut slen = 0usize;
            while m < n && b[m].is_ascii_digit() && slen < 3 {
                m += 1;
                slen += 1;
            }
            kode.push('.');
            kode.push_str(&teks[s..m]);
            segmen += 1;
            k = m;
        }
        // Batas kanan: karakter setelah kode bukan digit
        let boundary = k >= n || !b[k].is_ascii_digit();
        if segmen >= 2 && boundary {
            out.push(kode);
        }
        i = k;
    }
    out
}

/// Deteksi kode klasifikasi SENSITIF (per SKKAD: Rahasia/Sangat Rahasia/Terbatas)
/// yang tertulis di dalam teks naskah. `kode_sensitif` adalah daftar kode yang
/// dimuat dari DB (kolom klasifikasi_keamanan) — satu-satunya sumber kebenaran.
/// Mengembalikan alasan blokir; kosong = aman.
pub fn deteksi_kode(teks: &str, kode_sensitif: &std::collections::HashSet<String>) -> Vec<String> {
    let mut alasan = Vec::new();
    for kode in kode_kandidat(teks) {
        if kode_sensitif.contains(&kode) {
            push_alasan(&mut alasan, &format!("kode klasifikasi '{}' berklasifikasi keamanan per SKKAD", kode));
        }
    }
    alasan
}

/// Deteksi indikasi informasi yang dikecualikan pada isi naskah.
/// Mengembalikan daftar alasan (kosong = aman untuk dianalisa).
pub fn deteksi(teks: &str) -> Vec<String> {
    let mut alasan = Vec::new();
    let n = norm(teks);

    // ── Tier 1: label klasifikasi naskah dinas ──
    for p in ["SANGAT RAHASIA", "RAHASIA NEGARA"] {
        if n.contains(p) {
            push_alasan(&mut alasan, &format!("label '{}'", p));
        }
    }
    // Pola kop naskah dinas: SIFAT/KLASIFIKASI … RAHASIA/TERBATAS (case-insensitive).
    // Catatan: "bersifat rahasia" adalah frasa klasifikasi asli → diblokir.
    // Guard negasi: "tidak/bukan bersifat rahasia" TIDAK dianggap label.
    for kw in ["SIFAT", "KLASIFIKASI", "KLASSIFIKASI"] {
        if let Some(i) = n.find(kw) {
            let after = window_after(&n, kw, 60);
            let pos = [after.find("RAHASIA"), after.find("TERBATAS")]
                .into_iter()
                .flatten()
                .min();
            if let Some(p) = pos {
                // Cek ~20 karakter sebelum posisi label di teks PENUH (mencakup
                // teks sebelum kata kunci, mis. "tidak bersifat rahasia").
                let label_pos = i + kw.len() + p;
                let before: String = n[..label_pos].chars().rev().take(20).collect();
                let before = before.chars().rev().collect::<String>().to_uppercase();
                if !(before.contains("TIDAK") || before.contains("BUKAN")) {
                    push_alasan(&mut alasan, &format!("sifat/klasifikasi '{} …'", kw));
                    break;
                }
            }
        }
    }
    // Stempel kapital berdiri sendiri (case-sensitive terhadap teks asli).
    // Kata lowercase ("rahasia tim sukses") TIDAK dianggap label.
    let mut ada_stempel_rahasia = false;
    let mut ada_stempel_terbatas = false;
    for t in tokens(teks) {
        if t == "RAHASIA" {
            ada_stempel_rahasia = true;
        }
        if t == "TERBATAS" {
            ada_stempel_terbatas = true;
        }
    }
    if ada_stempel_rahasia {
        push_alasan(&mut alasan, "stempel 'RAHASIA'");
    }
    if ada_stempel_terbatas {
        push_alasan(&mut alasan, "stempel 'TERBATAS'");
    }
    // Padanan Inggris yang sering dipakai di naskah
    for p in ["TOP SECRET", "CONFIDENTIAL", "RESTRICTED", "FOR INTERNAL USE"] {
        if n.contains(p) {
            push_alasan(&mut alasan, &format!("label '{}'", p));
        }
    }

    // ── Tier 2: frasa eksplisit "informasi yang dikecualikan" & rahasia khusus ──
    if n.contains("INFORMASI YANG DIKECUALIKAN") {
        push_alasan(&mut alasan, "frasa 'informasi yang dikecualikan'");
    }
    for p in [
        "RAHASIA JABATAN",
        "RAHASIA DINAS",
        "RAHASIA DAGANG",
        "RAHASIA PERUSAHAAN",
        "RAHASIA BANK",
        "RAHASIA MEDIS",
        "RAHASIA KORESPONDENSI",
        "RAHASIA KOMERSIAL",
    ] {
        if n.contains(p) {
            push_alasan(&mut alasan, &format!("frasa '{}'", p.to_lowercase()));
        }
    }
    for p in ["DATA NASABAH", "REKAM MEDIS", "HANYA UNTUK INTERNAL", "TIDAK UNTUK DIPUBLIKASIKAN"] {
        if n.contains(p) {
            push_alasan(&mut alasan, &format!("frasa '{}'", p.to_lowercase()));
        }
    }

    // ── Tier 3: istilah keamanan negara ──
    for p in [
        "INTELIJEN NEGARA",
        "KEAMANAN NEGARA",
        "PERTAHANAN NEGARA",
        "OPERASI MILITER",
        "MATA-MATA",
        "PENYADAPAN",
        "PERSENJATAAN",
        "AMUNISI",
    ] {
        if n.contains(p) {
            push_alasan(&mut alasan, &format!("istilah '{}'", p.to_lowercase()));
        }
    }

    // ── Tier 4: data pribadi massal ──
    let nik = count_nik(teks);
    if nik >= 3 {
        push_alasan(&mut alasan, &format!("daftar NIK massal ({} NIK)", nik));
    }
    for kw in ["DAFTAR NIK", "DAFTAR NPWP", "DATA PENDUDUK"] {
        let after = window_after(&n, kw, 100);
        if after.contains("RAHASIA") || after.contains("TERBATAS") {
            push_alasan(&mut alasan, &format!("'{}' dengan klasifikasi rahasia", kw.to_lowercase()));
            break;
        }
    }

    alasan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alasan_singkat(teks: &str) -> Vec<String> {
        deteksi(teks)
    }

    #[test]
    fn label_sangat_rahasia_diblokir() {
        let a = alasan_singkat("NOMOR: 001/RAHASIA/2026\nSANGAT RAHASIA\nKepada Yth. ...");
        assert!(!a.is_empty());
    }

    #[test]
    fn rahasia_negara_lowercase_diblokir() {
        let a = alasan_singkat("Dokumen ini memuat rahasia negara tentang perbatasan.");
        assert!(a.iter().any(|x| x.contains("RAHASIA NEGARA")));
    }

    #[test]
    fn sifat_terbatas_diblokir() {
        let a = alasan_singkat("Sifat: Terbatas\nPerihal: Hasil pemeriksaan BPK");
        assert!(a.iter().any(|x| x.contains("sifat/klasifikasi")));
    }

    #[test]
    fn stempel_rahasia_kapital_diblokir() {
        let a = alasan_singkat("LAPORAN\n\nRAHASIA\n\nPerihal: Evaluasi kinerja");
        assert!(a.iter().any(|x| x.contains("stempel 'RAHASIA'")));
    }

    #[test]
    fn frasa_informasi_dikecualikan_diblokir() {
        let a = alasan_singkat("Berisi informasi yang dikecualikan sesuai UU 14/2008.");
        assert!(a.iter().any(|x| x.contains("informasi yang dikecualikan")));
    }

    #[test]
    fn rahasia_jabatan_diblokir() {
        let a = alasan_singkat("Materi rahasia jabatan pejabat struktural.");
        assert!(a.iter().any(|x| x.contains("rahasia jabatan")));
    }

    #[test]
    fn intelijen_negara_diblokir() {
        let a = alasan_singkat("Koordinasi intelijen negara terkait perbatasan.");
        assert!(a.iter().any(|x| x.contains("intelijen negara")));
    }

    #[test]
    fn data_nasabah_diblokir() {
        let a = alasan_singkat("Daftar data nasabah bank pembangunan daerah.");
        assert!(a.iter().any(|x| x.contains("data nasabah")));
    }

    #[test]
    fn nik_massal_diblokir() {
        let teks = "1. 3501010101010001\n2. 3501010101010002\n3. 3501010101010003";
        let a = alasan_singkat(teks);
        assert!(a.iter().any(|x| x.contains("NIK massal")));
    }

    #[test]
    fn naskah_biasa_tidak_diblokir() {
        // Surat permohonan normal: satu NIK pemohon + kata "terbatas" lowercase
        let teks = "Permohonan cuti tahunan. Bersama ini saya mohon izin cuti selama 12 hari \
                    karena alasan keluarga. Terlampir fotokopi KTP, NIK 3501010101010001. \
                    Kami memahami anggaran kami terbatas, mohon maklum.";
        assert!(deteksi(teks).is_empty());
    }

    #[test]
    fn kata_rahasia_lowercase_tidak_diblokir() {
        let a = alasan_singkat("Rapat ini membahas rahasia tim sukses pemilu.");
        assert!(a.is_empty());
    }

    #[test]
    fn frasa_inggris_confidential_diblokir() {
        let a = alasan_singkat("MEMO INTERNAL\nCONFIDENTIAL\nDistribution limited.");
        assert!(a.iter().any(|x| x.contains("CONFIDENTIAL")));
    }

    #[test]
    fn bersifat_rahasia_diblokir() {
        // "bersifat rahasia" memuat "sifat … rahasia" → harus diblokir
        let a = alasan_singkat("Dokumen ini bersifat rahasia dan tidak untuk disebarluaskan.");
        assert!(!a.is_empty());
    }

    #[test]
    fn negasi_tidak_bersifat_rahasia_tidak_diblokir() {
        // Kalimat yang MENYATAKAN naskah tidak rahasia tidak boleh diblokir
        let teks = "Dokumen ini tidak bersifat rahasia dan dapat disebarluaskan kepada umum.";
        assert!(deteksi(teks).is_empty());
    }

    #[test]
    fn negasi_bukan_terbatas_tidak_diblokir() {
        let teks = "Surat ini bukan bersifat terbatas, mohon disebarluaskan.";
        assert!(deteksi(teks).is_empty());
    }

    #[test]
    fn run_angka_17_digit_tidak_dihitung_sebagai_nik() {
        // Bukan NIK (bukan 16 digit persis) — tidak boleh memicu blokir NIK massal
        let teks = "Ref: 12345678901234567\nRef: 23456789012345678\nRef: 34567890123456789";
        assert!(deteksi(teks).is_empty());
    }

    // ── Kode klasifikasi sensitif (per SKKAD) ──

    fn set(kodes: &[&str]) -> std::collections::HashSet<String> {
        kodes.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn kode_kandidat_ekstrak_pola_minimal_2_segmen() {
        let k = kode_kandidat("Perihal: 010.04.02 Rapat Pimpinan");
        assert!(k.contains(&"010.04.02".to_string()));
        // 3 digit tunggal (bukan kode) tidak ikut
        let k2 = kode_kandidat("No: 010 hanya tiga digit");
        assert!(!k2.iter().any(|x| x == "010"));
    }

    #[test]
    fn kode_sensitif_di_teks_diblokir() {
        let s = set(&["010.04.02"]);
        let a = deteksi_kode("Lampiran: 010.04.02 Rapat Pimpinan Eselon II", &s);
        assert!(a.iter().any(|x| x.contains("010.04.02")));
    }

    #[test]
    fn kode_non_sensitif_tidak_diblokir() {
        // 010.04.02 ada di teks tapi tidak ada di daftar sensitif → aman
        let s = set(&["010.03"]);
        let a = deteksi_kode("Kode: 010.04.02 Rapat", &s);
        assert!(a.is_empty());
    }

    #[test]
    fn angka_uang_tidak_dianggap_kode() {
        // "20.000.000" cocok pola numerik tapi tidak ada di daftar kode
        let s = set(&["010.03", "200.000"]);
        let a = deteksi_kode("Anggaran Rp 20.000.000 untuk kegiatan.", &s);
        assert!(a.is_empty());
    }

    #[test]
    fn kode_di_nomor_surat_diblokir() {
        // Format nomor surat dinas memuat kode klasifikasi (…/010.03/…).
        // 010.03 Terbatas per SKKAD → surat itu memang berklasifikasi → blokir.
        let s = set(&["010.03"]);
        let a = deteksi_kode("Nomor: 001/010.03/2026/DISPENDA", &s);
        assert!(a.iter().any(|x| x.contains("010.03")));
    }

    #[test]
    fn tanggal_tidak_dianggap_kode() {
        // "10.03.2026" (10 Maret 2026) — kandidat maximal "10.03.202" diikuti
        // digit '6' → bukan batas kata, tidak jadi kode utuh → aman
        let s = set(&["010.03"]);
        let a = deteksi_kode("Dikeluarkan tanggal 10.03.2026 di Malang.", &s);
        assert!(a.is_empty());
    }

    #[test]
    fn kode_dengan_batas_kata_diblokir() {
        // "Kode: 010.03;" — titik koma = batas kata → kode utuh → blokir
        let s = set(&["010.03"]);
        let a = deteksi_kode("Kode klasifikasi: 010.03; lampiran terlampir.", &s);
        assert!(a.iter().any(|x| x.contains("010.03")));
    }
}
