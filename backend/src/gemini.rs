use std::collections::HashMap;
use serde_json::Value;

const EMBED_MODEL: &str = "gemini-embedding-2";
const CHAT_MODEL: &str = "gemini-2.5-flash";

/// POST ke Gemini tanpa retry lokal.
/// Jika kena 429, langsung return error agar caller (try_all) bisa switch key dengan cepat.
/// Retry dan rotasi ditangani sepenuhnya oleh KeyRotator::try_all().
async fn post(client: &reqwest::Client, url: &str, body: &serde_json::Value) -> anyhow::Result<reqwest::Response> {
    let resp = client.post(url).json(body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Gemini {} ({}): {}", status.as_str(), status, body.chars().take(500).collect::<String>());
    }
    Ok(resp)
}

pub async fn embed_text(api_key: &str, text: &str) -> anyhow::Result<Vec<f64>> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
        EMBED_MODEL, api_key
    );
    let body = serde_json::json!({
        "content": { "parts": [{"text": text}] },
        "outputDimensionality": 768
    });

    let resp = post(&client, &url, &body).await?;
    let json: Value = resp.json().await?;
    let values = json["embedding"]["values"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing embedding.values"))?;
    Ok(values.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
}

/// Pilih fungsi/urusan (1 dari 45 klaster-1) + DUA varian perihal dari naskah.
/// Kembalikan (fungsi, perihal_inti, perihal_lengkap):
/// - perihal_inti: bersih (tanpa nama orang/tempat/waktu), huruf kecil → query embedding
/// - perihal_lengkap: detail apa adanya → tampilan UI & feedback
///
/// `daftar_fungsi` = string nama-nama fungsi/urusan (dipisah koma) untuk prompt;
/// `daftar_fungsi_list` = daftar nama KANONIK (level-1, kode 3 digit) untuk
/// validasi hasil model — bila model mengembalikan nama di luar daftar
/// (mis. sub-urusan "PEMBINAAN KEARSIPAN"), dipetakan kembali ke nama kanonik
/// ("KEARSIPAN") agar query embedding selalu berada di ruang level-1.
pub async fn select_fungsi(
    api_key: &str,
    text: &str,
    daftar_fungsi: &str,
    daftar_fungsi_list: &[String],
) -> anyhow::Result<(String, String, String)> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        CHAT_MODEL, api_key
    );
    let daftar = daftar_fungsi;
    // Tanpa few-shot: saat pemilihan fungsi, PERIHAL naskah belum terbentuk
    // (select_fungsi-lah yang menghasilkan perihal), jadi contoh perihal-mirip
    // dari feedback belum bisa dicocokkan secara bermakna.
    let prompt = format!(
        "Anda arsiparis. Dari teks naskah dinas berikut, tentukan SATU Fungsi/Urusan yang paling sesuai dengan SUBSTANSI MASALAH naskah (bukan bentuk surat), lalu tuliskan DUA varian perihal:\n\n\
         - perihal_lengkap: perihal naskah LENGKAP apa adanya (maks 1 kalimat, boleh memuat tanggal/tahun/nama/tempat sebagaimana tertulis di naskah).\n\
         - perihal_inti: versi BERSIH dari perihal_lengkap, hanya substansi. WAJIB BUANG dari perihal_inti: 1) NAMA ORANG (contoh: \"usulan kenaikan pangkat atas nama Bambang\" cukup \"usulan kenaikan pangkat\"), 2) TEMPAT/WILAYAH/UNIT: kota, kabupaten, kecamatan, desa, instansi, alamat (contoh: \"bimbingan teknis SRIKANDI di Kecamatan Kesamben\" cukup \"bimbingan teknis SRIKANDI\"), 3) KETERANGAN WAKTU & NOMOR: tanggal, bulan, tahun, periode, nomor surat (contoh: \"realisasi anggaran triwulan 2 tahun 2026\" cukup \"realisasi anggaran triwulan\"), 4) BENTUK DOKUMEN: kata seperti \"standar operasional prosedur\", \"SOP\", \"juknis\", \"laporan\", \"surat edaran\", \"undangan\", \"berita acara\", \"memo\" BUKAN substansi — buang (contoh: \"standar operasional prosedur inovasi baper\" cukup \"inovasi layanan baper\"). PERTAHANKAN istilah substantif seperti \"triwulan\", \"semester\", \"tahun anggaran\" bila menjadi sifat naskah; cukup hilangkan angka/penunjuk spesifiknya. Tulis perihal_inti dalam HURUF KECIL.\n\n\
         ATURAN PENTING — JANGAN TERTIPU BENTUK DOKUMEN:\n\
         1. Bentuk dokumen (SOP, surat, juknis, laporan, undangan, berita acara, memo, surat edaran) BUKAN penentu klasifikasi. Klasifikasikan berdasarkan SUBSTANSI/ISI, bukan jenis dokumen. Contoh: \"SOP pelayanan perpustakaan\" → PERPUSTAKAAN (bukan ORGANISASI DAN KETATALAKSANAAN); \"juknis bantuan operasional sekolah\" → PENDIDIKAN (bukan KETATAUSAHAAN).\n\
         2. Jangan tertipu NAMA INSTANSI. Kop surat \"Dinas Perpustakaan dan Kearsipan\" TIDAK otomatis berarti KEARSIPAN — lihat isi: bila substansi layanan perpustakaan (perpustakaan, pustaka, pojok baca, literasi baca, layanan perpustakaan) → PERPUSTAKAAN.\n\
         3. Perhatikan SUBSTANSI kata kunci dalam teks: kata \"perpustakaan\", \"pustaka\", \"pojok baca\", \"literasi baca\", \"bahan pustaka\" jelas mengarah ke PERPUSTAKAAN; kata \"kearsipan\", \"arsip\", \"pengelolaan arsip\" mengarah ke KEARSIPAN.\n\n\
         ATURAN PEMILIHAN FUNGSI — PALING PENTING:\n\
         Fungsi/Urusan adalah KLASTER TINGKAT ATAS (level 1, kode 3 digit).\n\
         1. Pilih SATU nama PERSIS dari Daftar Fungsi/Urusan di bawah — salin apa adanya (huruf besar/kecil sama persis). JANGAN mengubah atau menyingkat nama.\n\
         2. JANGAN memilih sub-urusan level 2/3 (mis. \"PEMBINAAN KEARSIPAN\", \"PENGELOLAAN ARSIP\" adalah anak dari KEARSIPAN — BUKAN pilihan). Hanya nama yang TERTULIS PERSIS di daftar yang boleh dipilih.\n\
         3. Bila substansi masuk kategori tertentu, pilih klaster induknya. Contoh: bimbingan konsultasi kearsipan → KEARSIPAN; pengelolaan simpul jaringan SIKN/JIKN → KEARSIPAN; SOP perpustakaan → PERPUSTAKAAN.\n\n\
         Daftar Fungsi/Urusan:\n{daftar}\n\n\
         Teks naskah:\n{text}\n\n\
         Keluarkan HANYA JSON valid: {{\"fungsi\":\"NAMA PERSIS DARI DAFTAR\",\"perihal_inti\":\"perihal inti huruf kecil\",\"perihal_lengkap\":\"perihal lengkap apa adanya\"}}",
        daftar = daftar, text = &text.chars().take(3000).collect::<String>()
    );
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": prompt}]}],
        // temperature 0: deterministik — hasil select_fungsi (yang menentukan
        // query embedding & fungsi) harus konsisten antar panggilan.
        "generationConfig": { "temperature": 0.0, "maxOutputTokens": 8192, "thinkingConfig": { "thinkingBudget": 0 } }
    });
    let resp = post(&client, &url, &body).await?;
    let json: Value = resp.json().await?;
    let raw = json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str().unwrap_or("{\"fungsi\":\"\",\"perihal_inti\":\"\",\"perihal_lengkap\":\"\"}")
        .trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let f = grab_field(raw, "fungsi");
    let pi = grab_field(raw, "perihal_inti");
    let pl = grab_field(raw, "perihal_lengkap");
    // Validasi: model bisa mengembalikan nama di luar daftar 45 level-1
    // (mis. sub-urusan "PEMBINAAN KEARSIPAN"). Petakan kembali ke nama
    // kanonik agar query embedding selalu di ruang level-1 — kalau tidak
    // cocok sama sekali, fungsi dikosongkan (caller fallback ke teks asli).
    let fungsi = validate_fungsi(&f, daftar_fungsi_list);
    // perihal_inti selalu lowercase & trim (hardening: instruksi prompt saja
    // tidak cukup — model sesekali bisa mengembalikan kapital/whitespace).
    Ok((fungsi, pi.trim().to_lowercase(), pl.trim().to_string()))
}

/// Petakan nama fungsi hasil model ke nama KANONIK dari daftar 45 level-1.
/// Hardening: prompt saja tidak cukup — model sesekali mengembalikan nama
/// sub-urusan (level 2/3) seperti "PEMBINAAN KEARSIPAN". Strategi:
/// 1. Cocok persis (case-insensitive) → pakai nama kanonik daftar.
/// 2. Nama hasil mengandung salah satu nama daftar (mis. "PEMBINAAN KEARSIPAN"
///    mengandung "KEARSIPAN") → petakan ke nama kanonik terpanjang yang cocok.
/// 3. Tidak ada kecocokan → kembalikan string kosong (caller fallback ke teks asli).
fn validate_fungsi(fungsi: &str, daftar: &[String]) -> String {
    let f = fungsi.trim();
    if f.is_empty() {
        return String::new();
    }
    // 1) Cocok persis (abaikan case, normalisasi spasi)
    let f_norm = f.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    for d in daftar {
        let d_norm = d.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
        if d_norm == f_norm {
            return d.trim().to_string();
        }
    }
    // 2) Overlap kata (token saling substring):
    //    - hasil memuat nama kanonik ("PEMBINAAN KEARSIPAN" memuat "KEARSIPAN")
    //    - token hasil muncul sebagai substring nama kanonik ("PENGELOLAAN ARSIP"
    //      → token "arsip" ada di "KEARSIPAN"; "KETATAUSAHAAN" → token ada di
    //      "KETATAUSAHAAN DAN KERUMAHTANGGAAN"). Skor tertinggi menang.
    let mut best: Option<(usize, &str)> = None;
    for d in daftar {
        let d_norm = d.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
        let score = overlap_score(&f_norm, &d_norm);
        if score >= 4 {
            if best.map_or(true, |(bs, _)| score > bs) {
                best = Some((score, d.trim()));
            }
        }
    }
    if let Some((_, canon)) = best {
        eprintln!(
            "⚠️  select_fungsi: '{}' di luar daftar 45 → dipetakan ke '{}'",
            f, canon
        );
        return canon.to_string();
    }
    eprintln!("⚠️  select_fungsi: '{}' tidak cocok daftar 45 → fungsi kosong", f);
    String::new()
}

/// Skor overlap kata antara dua string ternormalisasi: jumlah panjang token
/// yang SALING menjadi substring (minimal 4 huruf untuk hindari kecocokan
/// semu seperti "dan"). Token dihitung dua arah — cukup sebagai peringkat
/// relatif untuk memilih nama kanonik terdekat.
fn overlap_score(a: &str, b: &str) -> usize {
    let mut score = 0;
    for tok in a.split_whitespace() {
        let n = tok.chars().count();
        if n >= 4 && b.contains(tok) {
            score += n;
        }
    }
    for tok in b.split_whitespace() {
        let n = tok.chars().count();
        if n >= 4 && a.contains(tok) {
            score += n;
        }
    }
    score
}

/// Ambil kode + deskripsi kandidat terbaik pertama (untuk fallback explanation).
/// Ekstrak field dari raw JSON yang korup via pencarian substring manual
/// (tahan karakter aneh/terpotong, tanpa dependency regex).
/// Ambil nilai string dari key JSON tunggal (tahan JSON korup).
fn grab_field(raw: &str, key: &str) -> String {
    let needle = format!("\"{}\"", key);
    if let Some(rest) = raw.split_once(&needle).map(|(_, r)| r) {
        if let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            if let Some(end) = after.find('"') {
                return after[..end].to_string();
            }
        }
    }
    String::new()
}

fn extract_fields_tolerant(raw: &str) -> (String, String, Vec<String>) {
    fn grab(raw: &str, key: &str) -> String {
        let needle = format!("\"{}\"", key);
        if let Some(rest) = raw.split_once(&needle).map(|(_, r)| r) {
            if let Some(start) = rest.find('"') {
                let after = &rest[start + 1..];
                if let Some(end) = after.find('"') {
                    return after[..end].to_string();
                }
            }
        }
        String::new()
    }
    // Ambil semua nilai "kode":"..." secara berurutan
    let mut kodes: Vec<String> = Vec::new();
    let needle = "\"kode\"";
    let mut rest = raw;
    while let Some((_, after)) = rest.split_once(needle) {
        if let Some(start) = after.find('"') {
            let body = &after[start + 1..];
            if let Some(end) = body.find('"') {
                kodes.push(body[..end].to_string());
                rest = &body[end + 1..];
                continue;
            }
        }
        break;
    }
    (grab(raw, "perihal"), grab(raw, "explanation"), kodes)
}

fn reranked_first_kode(results: &[super::ClassificationResult]) -> (String, String) {
    if let Some(r) = results.first() {
        (r.kode.clone(), r.deskripsi.clone())
    } else {
        (String::new(), String::new())
    }
}

/// Samakan kode terpilih dalam penjelasan dengan kode peringkat teratas SETELAH
/// reorder spesifisitas path. Pola penjelasan: "... Kode klasifikasi <kode>
/// dipilih dengan alasan ...". Bila <kode> bukan kode peringkat teratas baru
/// (mis. Gemini memilih induk padahal anaknya ada di kandidat), ganti <kode>
/// tersebut. Kalimat penjelasan lainnya dibiarkan (alasan tetap relevan).
fn fix_explanation_kode(explanation: &str, top_kode: &str) -> String {
    const MARK: &str = "Kode klasifikasi ";
    if let Some((before, rest)) = explanation.split_once(MARK) {
        // Ambil token kode sampai spasi berikutnya (format kode: digit + titik)
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let old_kode = &rest[..end];
        if !old_kode.is_empty() && old_kode != top_kode {
            let mut out = String::with_capacity(explanation.len() + 8);
            out.push_str(before);
            out.push_str(MARK);
            out.push_str(top_kode);
            out.push_str(&rest[end..]);
            return out;
        }
    }
    explanation.to_string()
}

pub async fn rerank_and_explain(
    api_key: &str,
    message: &str,
    fewshot: &str,
    perihal_hint: &str,
    need_ringkasan: bool,
    results: &[super::ClassificationResult],
) -> anyhow::Result<(Vec<super::ClassificationResult>, String, String, String)> {
    if results.is_empty() {
        return Ok((vec![], "Tidak ada hasil yang cocok.".into(), String::new(), String::new()));
    }

    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        CHAT_MODEL, api_key
    );

    let mut candidates = String::new();
    for (i, r) in results.iter().enumerate() {
        candidates.push_str(&format!(
            "{}. Kode: {} | Deskripsi: {} | Path: {} | Similarity: {:.4}\n",
            i, r.kode, r.deskripsi, r.path, r.similarity
        ));
    }

    // Few-shot: koreksi arsiparis tervalidasi pada naskah serupa (jika ada)
    let fewshot_section = if fewshot.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", fewshot)
    };

    // Perihal lengkap dari select_fungsi — dipakai sebagai acuan perihal di
    // LANGKAH 1 agar kalimat penjelasan konsisten dengan perihal tampilan.
    let perihal_hint_section = if perihal_hint.trim().is_empty() {
        String::new()
    } else {
        format!("===== PERIHAL NASKAH (hasil ekstraksi awal) =====\n{}\n\n", perihal_hint)
    };

    // Instruksi ringkasan naskah (isi ringkas) — HANYA bila diminta.
    // Dipakai Chrome extension SRIKANDI; versi web tidak meminta sehingga
    // tidak ada biaya kuota/latensi tambahan (tetap 1 panggilan Gemini yang sama).
    let ringkasan_inst = if need_ringkasan {
        "LANGKAH 4 - RINGKAS NASKAH: Tulis \"isi ringkas\" naskah (ringkasan isi dokumen) dalam 2-3 kalimat padat, fokus substansi masalah & keputusan penting. PERTAHANKAN keterangan nama orang, tempat, dan waktu sebagaimana tertulis di naskah.\n"
    } else {
        ""
    };
    let ringkasan_schema = if need_ringkasan {
        ",\"ringkasan\":\"...\""
    } else {
        ""
    };

    let prompt = format!(
        "Kamu adalah AI Arsiparis. Di bawah ini adalah teks naskah dinas (bisa teks lengkap\n\
         dokumen, atau perihal singkat).\n\n\
         ===== TEKS NASKAH =====\n\
         {}\n\n\
         ===== DAFTAR KANDIDAT =====\n\
         {}\n\n\
         {}\
         {}\
         TUGAS KAMU (langkah berurutan):\n\n\
         LANGKAH 1 - TETAPKAN PERIHAL: Pakai PERIHAL NASKAH dari bagian ===== PERIHAL\n\
         NASKAH ===== di atas bila terisi (jangan ekstrak ulang). Bila kosong, cari baris\n\
         Perihal: / Hal: atau simpulkan dari isi dokumen. Hasilnya PERIHAL NASKAH\n\
         (maks 1 kalimat). Gunakan persis apa adanya, tanpa mengubah huruf besar/kecil.\n\
         Abaikan kop surat, kop dinas, alamat, salam pembuka.\n\
         LANGKAH 2 - RERANK KANDIDAT: Urutkan ulang semua kandidat berdasarkan:\n\
         1. RELEVANSI - kecocokan dengan PERIHAL NASKAH.\n\
         2. SPESIFISITAS PATH - **ATURAN PALING PENTING**: jika kode A adalah prefix\n\
            kode B (misal A=041.03 dan B=041.03.01), maka B HARUS di atas A.\n\
            Contoh benar: 041.03.01.01 > 041.03.01 > 041.03\n\
         3. SIMILARITY SCORE - tiebreaker terakhir.\n\
         Tulis HANYA 3 kandidat terbaik di array reranked.\n\
         LANGKAH 3 - JELASKAN: Tulis penjelasan dengan format PERSIS satu kalimat:\n\
         \"Perihal: <perihal>. Kode klasifikasi <kode terpilih> dipilih dengan alasan <alasan>.\"\n\
         <alasan> = 2-3 kalimat fokus kecocokan isi naskah dengan kode/deskripsi terpilih,\n\
         dalam bahasa yang mudah dimengerti pengelola arsip.\n\
         JANGAN menyebut aturan \"spesifisitas path\", \"prefix\", atau aturan teknis pengurutan.\n\n\
         {}\
         Keluarkan HANYA JSON valid (tanpa markdown code block):\n\
         {{\"perihal\":\"...\",\"reranked\":[{{\"rank\":1,\"kode\":\"XXX.XX\"}}],\"explanation\":\"...\"{}}}",
        message, candidates, fewshot_section, perihal_hint_section, ringkasan_inst, ringkasan_schema
    );

    let body = serde_json::json!({
        "contents": [{ "parts": [{"text": prompt}] }],
        // temperature 0: deterministik — konsisten dengan select_fungsi, agar
        // naskah yang sama selalu menghasilkan urutan kandidat yang sama.
        "generationConfig": { "temperature": 0.0, "maxOutputTokens": 8192, "thinkingConfig": { "thinkingBudget": 0 } }
    });

    let resp = post(&client, &url, &body).await?;
    let json: Value = resp.json().await?;
    let raw_text = json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("{}");

    let cleaned = raw_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    // Parsing toleran: serde gagal total bila JSON korup (karakter aneh / terpotong).
    // Fallback ke ekstraksi regex per-field supaya perihal/reranked/explanation
    // tetap terbaca meski satu bagian rusak.
    let parsed: Value = serde_json::from_str(cleaned).unwrap_or(Value::Null);

    let mut perihal = parsed["perihal"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_default();

    let mut explanation_raw = parsed["explanation"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_default();

    let mut ringkasan = parsed["ringkasan"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_default();

    let mut rank_map: HashMap<String, usize> = HashMap::new();
    if let Some(arr) = parsed["reranked"].as_array() {
        for (i, item) in arr.iter().enumerate() {
            if let Some(kode) = item["kode"].as_str() {
                rank_map.insert(kode.to_string(), i);
            }
        }
    }

    // Bila JSON korup, ekstrak manual via regex (tahan terhadap karakter aneh).
    if perihal.is_empty() || explanation_raw.is_empty() || rank_map.is_empty() {
        eprintln!("[rerank] JSON tidak lengkap ({} chars). Raw: {}", cleaned.chars().count(), cleaned.chars().take(800).collect::<String>());
        let (p, e, kodes) = extract_fields_tolerant(cleaned);
        if perihal.is_empty() { perihal = p; }
        if explanation_raw.is_empty() { explanation_raw = e; }
        if ringkasan.is_empty() { ringkasan = grab_field(cleaned, "ringkasan"); }
        if rank_map.is_empty() {
            for (i, k) in kodes.into_iter().enumerate() {
                rank_map.entry(k).or_insert(i);
            }
        }
    }

    // Fallback: explanation selalu ikut pola kesepakatan
    // "Perihal: X. Kode klasifikasi Y dipilih dengan alasan Z."
    let explanation = if !explanation_raw.is_empty() {
        explanation_raw
    } else {
        let (kode, deskripsi) = reranked_first_kode(results);
        let p = if perihal.is_empty() { "Naskah".to_string() } else { perihal.clone() };
        format!(
            "Perihal: {}. Kode klasifikasi {} ({}) dipilih dengan alasan deskripsinya paling sesuai dengan isi naskah.",
            p, kode, deskripsi
        )
    };

    let mut reranked = results.to_vec();
    reranked.sort_by_key(|r| rank_map.get(&r.kode).copied().unwrap_or(usize::MAX));

    // Pastikan kode lebih spesifik (ANAK) SELALU di atas induknya — aturan
    // "spesifisitas path". Versi lama hanya menukar pasangan BERSEBELAHAN,
    // sehingga induk yang dipisah kode lain dari anaknya tidak pernah tertukar
    // (contoh gagal lama: [045.02.02, 045.02.01.02, 045.02.02.03] — anak 045.02.02.03
    // tidak berdekatan dengan induk 045.02.02, jadi aturan tak dieksekusi).
    // Algoritma baru: scan berulang; setiap kali induk terletak di atas
    // keturunannya (kode anak = induk + "." + ...), pindahkan anak itu TEPAT
    // ke atas induknya. Hanya menyentuh relasi induk–anak; urutan relatif
    // kode yang tak berelasi dipertahankan (stabil) mengikuti ranking Gemini.
    reranked = reorder_specific_first(reranked);

    // Konsistensi penjelasan: setelah reorder spesifisitas path, kode terpilih
    // yang disebut di penjelasan harus sama dengan peringkat teratas baru
    // (bila Gemini memilih induk padahal anaknya ada di kandidat, aturan path
    // yang menang — perbarui penjelasan agar tidak menyesatkan pengguna).
    let top_kode = reranked.first().map(|r| r.kode.as_str()).unwrap_or("");
    let explanation = if top_kode.is_empty() {
        explanation
    } else {
        fix_explanation_kode(&explanation, top_kode)
    };

    Ok((reranked, explanation, perihal, ringkasan))
}

/// Pindahkan kode ANAK ke atas induknya — aturan "spesifisitas path".
/// Iteratif: scan dari atas; setiap kali ada keturunan (kode = induk + "." + ...)
/// yang terletak DI BAWAH induknya, pindahkan anak itu tepat ke atas induk.
/// Berhenti saat stabil. Stabil: relasi induk–anak tidak disentuh setelah satu
/// pass penuh tanpa perpindahan; urutan kode tak berelasi dipertahankan.
fn reorder_specific_first(mut reranked: Vec<super::ClassificationResult>) -> Vec<super::ClassificationResult> {
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..reranked.len() {
            let prefix = format!("{}.", reranked[i].kode);
            if let Some(j) = (i + 1..reranked.len()).find(|&j| reranked[j].kode.starts_with(&prefix)) {
                let item = reranked.remove(j);
                reranked.insert(i, item);
                changed = true;
                break;
            }
        }
    }
    reranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(kode: &str) -> super::super::ClassificationResult {
        super::super::ClassificationResult {
            id: 0,
            kode: kode.to_string(),
            deskripsi: String::new(),
            path: String::new(),
            similarity: 0.0,
            retensi_aktif: None,
            retensi_inaktif: None,
            penyusutan_akhir: None,
            klasifikasi_keamanan: None,
        }
    }

    fn kodes(v: &[super::super::ClassificationResult]) -> Vec<String> {
        v.iter().map(|x| x.kode.clone()).collect()
    }

    /// Kasus nyata dari pengujian PDF di VPS: induk 045.02.02 di atas anak
    /// 045.02.02.03 yang TIDAK berdekatan (dipisah 045.02.01.02). Versi lama
    /// (hanya tukar pasangan bersebelahan) gagal memperbaiki urutan ini.
    #[test]
    fn anak_di_atas_induk_walaupun_tidak_berdekatan() {
        let input = vec![r("045.02.02"), r("045.02.01.02"), r("045.02.02.03")];
        let out = reorder_specific_first(input);
        assert_eq!(kodes(&out), vec!["045.02.02.03", "045.02.02", "045.02.01.02"]);
    }

    /// Urutan yang sudah benar (anak di atas induk) TIDAK diubah.
    #[test]
    fn urutan_benar_tidak_diubah() {
        let input = vec![r("041.03.01.01"), r("041.03.01"), r("041.03")];
        let out = reorder_specific_first(input);
        assert_eq!(kodes(&out), vec!["041.03.01.01", "041.03.01", "041.03"]);
    }

    /// Kode yang tak berelasi tetap mempertahankan urutan relatifnya.
    #[test]
    fn kode_tak_berelasi_tetap_stabil() {
        let input = vec![r("045.02.02"), r("045.02.02.03"), r("045.02.01.02")];
        let out = reorder_specific_first(input);
        // 045.02.02.03 sudah di atas induknya; sisanya tetap di urutan awal
        assert_eq!(kodes(&out), vec!["045.02.02.03", "045.02.02", "045.02.01.02"]);
    }

    /// Berantai 3 tingkat: semua anak harus naik di atas induknya.
    #[test]
    fn rantai_tiga_tingkat() {
        let input = vec![r("041.03"), r("041.03.01"), r("041.03.01.01")];
        let out = reorder_specific_first(input);
        assert_eq!(kodes(&out), vec!["041.03.01.01", "041.03.01", "041.03"]);
    }

    /// Interleaved: cucu (041.03.01.01) berada di ANTARA induk dan anak
    /// (041.03 di atas, 041.03.01 di bawah). Algoritma harus stabil tanpa
    /// osilasi dan tetap menghasilkan urutan terdalam-di-atas.
    #[test]
    fn cucu_di_antara_induk_dan_anak() {
        let input = vec![r("041.03"), r("041.03.01.01"), r("041.03.01")];
        let out = reorder_specific_first(input);
        assert_eq!(kodes(&out), vec!["041.03.01.01", "041.03.01", "041.03"]);
    }

    /// fix_explanation_kode: kode di penjelasan disamakan dengan peringkat teratas.
    #[test]
    fn penjelasan_kode_ikut_peringkat_teratas() {
        let e = "Perihal: Surat Tugas. Kode klasifikasi 045.02.02 dipilih dengan alasan relevan.";
        assert_eq!(
            fix_explanation_kode(e, "045.02.02.03"),
            "Perihal: Surat Tugas. Kode klasifikasi 045.02.02.03 dipilih dengan alasan relevan."
        );
    }

    /// fix_explanation_kode: kode sudah sama → teks tidak berubah.
    #[test]
    fn penjelasan_kode_sama_tidak_diubah() {
        let e = "Perihal: X. Kode klasifikasi 045.02.02 dipilih dengan alasan Y.";
        assert_eq!(fix_explanation_kode(e, "045.02.02"), e);
    }

    // ---------- validate_fungsi ----------

    fn daftar45() -> Vec<String> {
        vec![
            "KEARSIPAN".to_string(),
            "PERPUSTAKAAN".to_string(),
            "PENDIDIKAN".to_string(),
            "KETATAUSAHAAN DAN KERUMAHTANGGAAN".to_string(),
        ]
    }

    /// Hasil model persis di daftar → dipertahankan.
    #[test]
    fn fungsi_persis_di_daftar_dipertahankan() {
        assert_eq!(validate_fungsi("KEARSIPAN", &daftar45()), "KEARSIPAN");
    }

    /// Hasil model = sub-urusan (level-2) → dipetakan ke nama kanonik level-1.
    #[test]
    fn sub_urusan_dipetakan_ke_level1() {
        // Kasus nyata: model mengembalikan PEMBINAAN KEARSIPAN (anak KEARSIPAN)
        assert_eq!(validate_fungsi("PEMBINAAN KEARSIPAN", &daftar45()), "KEARSIPAN");
        assert_eq!(validate_fungsi("PENGELOLAAN ARSIP", &daftar45()), "KEARSIPAN");
    }

    /// Case & whitespace tidak masalah — normalisasi sebelum cocok.
    #[test]
    fn fungsi_normalisasi_case_dan_spasi() {
        assert_eq!(validate_fungsi("  kearsipan ", &daftar45()), "KEARSIPAN");
    }

    /// Tidak ada kecocokan sama sekali → kosong (caller fallback ke teks asli).
    #[test]
    fn fungsi_tak_kenal_dikosongkan() {
        assert_eq!(validate_fungsi("ADMINISTRASI UMUM", &daftar45()), "");
        assert_eq!(validate_fungsi("", &daftar45()), "");
    }
}
