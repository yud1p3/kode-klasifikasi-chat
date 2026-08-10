#!/usr/bin/env python3
"""Backfill embedding untuk feedback validated yang embedding-nya NULL.

Latar belakang: kolom embedding di klasifikasi_feedback dipakai fetch_fewshot
untuk mencari feedback serupa (few-shot di prompt rerank). Sebelum perbaikan
2026-08-09, feedback POSITIF tidak pernah di-embed sehingga tidak pernah muncul
di few-shot — meski arsiparis sudah mengonfirmasi kode benar. Script ini
menghitung embedding untuk feedback lama yang kosong.

Konsistensi: teks yang di-embed DISELARASKAN dengan backend build_embed_query
(main.rs): selalu "FUNGSI > perihal_inti" via Gemini select_fungsi, sehingga
feedback lama dicocokkan dalam ruang embedding yang sama dengan query chat.
Prompt select_fungsi di bawah adalah SALINAN dari backend/src/gemini.rs —
jaga sinkron bila prompt itu diubah.

Pemakaian (sekali / saat migrasi data):
    python3 tools/backfill_feedback_embedding.py        # baca backend/.env
    DATABASE_URL=... GEMINI_API_KEYS=... python3 tools/backfill_feedback_embedding.py

Idempoten: hanya memproses feedback status='validated' dengan embedding IS NULL.
"""
import json
import os
import re
import sys
import time
from pathlib import Path

import psycopg2
import requests

BASE = Path(__file__).resolve().parent.parent
EMBED_MODEL = "gemini-embedding-2"
CHAT_MODEL = "gemini-2.5-flash"
DIM = 768

# ---------- 1. Baca konfigurasi (backend/.env fallback env) ----------

def load_env(path: Path):
    if not path.exists():
        return
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        os.environ.setdefault(key.strip(), val.strip().strip('"').strip("'"))

load_env(BASE / "backend" / ".env")

DATABASE_URL = os.environ.get("DATABASE_URL", "postgres://postgres:postgres@localhost:5432/klasifikasi_arsip")
API_KEYS = [
    k.strip()
    for k in os.environ.get("GEMINI_API_KEYS", os.environ.get("GEMINI_API_KEY", "")).split(",")
    if k.strip()
]
if not API_KEYS:
    print("❌ GEMINI_API_KEYS tidak ditemukan di env / backend/.env")
    sys.exit(1)

# ---------- 2. Daftar fungsi (konsisten dengan build_embed_query) ----------

# ---------- 3. select_fungsi — SALINAN backend/src/gemini.rs ----------

PROMPT_SELECT_FUNGSI = (
    "Anda arsiparis. Dari teks naskah dinas berikut, tentukan SATU Fungsi/Urusan yang paling sesuai dengan SUBSTANSI MASALAH naskah (bukan bentuk surat), lalu tuliskan DUA varian perihal:\n\n"
    "- perihal_lengkap: perihal naskah LENGKAP apa adanya (maks 1 kalimat, boleh memuat tanggal/tahun/nama/tempat sebagaimana tertulis di naskah).\n"
    "- perihal_inti: versi BERSIH dari perihal_lengkap, hanya substansi. WAJIB BUANG dari perihal_inti: 1) NAMA ORANG (contoh: \"usulan kenaikan pangkat atas nama Bambang\" cukup \"usulan kenaikan pangkat\"), 2) TEMPAT/WILAYAH/UNIT: kota, kabupaten, kecamatan, desa, instansi, alamat (contoh: \"bimbingan teknis SRIKANDI di Kecamatan Kesamben\" cukup \"bimbingan teknis SRIKANDI\"), 3) KETERANGAN WAKTU & NOMOR: tanggal, bulan, tahun, periode, nomor surat (contoh: \"realisasi anggaran triwulan 2 tahun 2026\" cukup \"realisasi anggaran triwulan\"), 4) BENTUK DOKUMEN: kata seperti \"standar operasional prosedur\", \"SOP\", \"juknis\", \"laporan\", \"surat edaran\", \"undangan\", \"berita acara\", \"memo\" BUKAN substansi — buang (contoh: \"standar operasional prosedur inovasi baper\" cukup \"inovasi layanan baper\"). PERTAHANKAN istilah substantif seperti \"triwulan\", \"semester\", \"tahun anggaran\" bila menjadi sifat naskah; cukup hilangkan angka/penunjuk spesifiknya. Tulis perihal_inti dalam HURUF KECIL.\n\n"
    "ATURAN PENTING — JANGAN TERTIPU BENTUK DOKUMEN:\n\n"
    "1. Bentuk dokumen (SOP, surat, juknis, laporan, undangan, berita acara, memo, surat edaran) BUKAN penentu klasifikasi. Klasifikasikan berdasarkan SUBSTANSI/ISI, bukan jenis dokumen. Contoh: \"SOP pelayanan perpustakaan\" → PERPUSTAKAAN (bukan ORGANISASI DAN KETATALAKSANAAN); \"juknis bantuan operasional sekolah\" → PENDIDIKAN (bukan KETATAUSAHAAN).\n\n"
    "2. Jangan tertipu NAMA INSTANSI. Kop surat \"Dinas Perpustakaan dan Kearsipan\" TIDAK otomatis berarti KEARSIPAN — lihat isi: bila substansi layanan perpustakaan (perpustakaan, pustaka, pojok baca, literasi baca, layanan perpustakaan) → PERPUSTAKAAN.\n\n"
    "3. Perhatikan SUBSTANSI kata kunci dalam teks: kata \"perpustakaan\", \"pustaka\", \"pojok baca\", \"literasi baca\", \"bahan pustaka\" jelas mengarah ke PERPUSTAKAAN; kata \"kearsipan\", \"arsip\", \"pengelolaan arsip\" mengarah ke KEARSIPAN.\n\n"
    "ATURAN PEMILIHAN FUNGSI — PALING PENTING:\n\n"
    "Fungsi/Urusan adalah KLASTER TINGKAT ATAS (level 1, kode 3 digit).\n"
    "1. Pilih SATU nama PERSIS dari Daftar Fungsi/Urusan di bawah — salin apa adanya (huruf besar/kecil sama persis). JANGAN mengubah atau menyingkat nama.\n"
    "2. JANGAN memilih sub-urusan level 2/3 (mis. \"PEMBINAAN KEARSIPAN\", \"PENGELOLAAN ARSIP\" adalah anak dari KEARSIPAN — BUKAN pilihan). Hanya nama yang TERTULIS PERSIS di daftar yang boleh dipilih.\n"
    "3. Bila substansi masuk kategori tertentu, pilih klaster induknya. Contoh: bimbingan konsultasi kearsipan → KEARSIPAN; pengelolaan simpul jaringan SIKN/JIKN → KEARSIPAN; SOP perpustakaan → PERPUSTAKAAN.\n\n"
    "Daftar Fungsi/Urusan:\n{daftar}\n\n"
    "Teks naskah:\n{text}\n\n"
    "Keluarkan HANYA JSON valid: {{\"fungsi\":\"NAMA PERSIS DARI DAFTAR\",\"perihal_inti\":\"perihal inti huruf kecil\",\"perihal_lengkap\":\"perihal lengkap apa adanya\"}}"
)


def call_gemini_chat(prompt: str) -> dict:
    last_err = None
    for key in API_KEYS:
        url = f"https://generativelanguage.googleapis.com/v1beta/models/{CHAT_MODEL}:generateContent?key={key}"
        body = {
            "contents": [{"parts": [{"text": prompt}]}],
            "generationConfig": {"temperature": 0.0, "maxOutputTokens": 8192,
                                 "thinkingConfig": {"thinkingBudget": 0}},
        }
        try:
            r = requests.post(url, json=body, timeout=60)
            if r.status_code == 429:
                last_err = "429 rate limit"
                time.sleep(3)
                continue
            r.raise_for_status()
            raw = r.json()["candidates"][0]["content"]["parts"][0]["text"]
            cleaned = raw.strip().strip("```json").strip("```").strip()
            return json.loads(cleaned)
        except Exception as e:  # noqa: BLE001
            last_err = str(e)[:200]
            continue
    raise RuntimeError(f"Semua key gagal: {last_err}")


def call_embed(text: str):
    last_err = None
    for key in API_KEYS:
        url = f"https://generativelanguage.googleapis.com/v1beta/models/{EMBED_MODEL}:embedContent?key={key}"
        body = {"content": {"parts": [{"text": text}]}, "outputDimensionality": DIM}
        try:
            r = requests.post(url, json=body, timeout=60)
            if r.status_code == 429:
                last_err = "429 rate limit"
                time.sleep(3)
                continue
            r.raise_for_status()
            return r.json()["embedding"]["values"]
        except Exception as e:  # noqa: BLE001
            last_err = str(e)[:200]
            continue
    raise RuntimeError(f"Semua key gagal: {last_err}")


def _norm(s: str) -> str:
    return " ".join(s.lower().split())


def _overlap_score(a: str, b: str) -> int:
    """Skor overlap token (salinan overlap_score di gemini.rs): jumlah panjang
    token yang saling menjadi substring (min 4 huruf), dihitung dua arah."""
    score = 0
    for tok in a.split():
        n = len(tok)
        if n >= 4 and tok in b:
            score += n
    for tok in b.split():
        n = len(tok)
        if n >= 4 and tok in a:
            score += n
    return score


def validate_fungsi(fungsi: str, daftar_list: list) -> str:
    """Petakan nama fungsi hasil model ke nama kanonik level-1 (salinan
    validate_fungsi di backend/src/gemini.rs). Strategi:
    1. Cocok persis (case-insensitive, normalisasi spasi).
    2. Overlap token (mis. "PEMBINAAN KEARSIPAN" → "KEARSIPAN";
       "PENGELOLAAN ARSIP" → "KEARSIPAN" via token "arsip" ⊂ "kearsipan").
       Skor tertinggi menang (>= 4).
    3. Tidak ada kecocokan → kosong (caller fallback ke teks asli)."""
    f = fungsi.strip()
    if not f:
        return ""
    f_norm = _norm(f)
    # 1) Cocok persis
    for d in daftar_list:
        if _norm(d) == f_norm:
            return d.strip()
    # 2) Overlap token (skor tertinggi menang)
    best = None
    best_score = 0
    for d in daftar_list:
        s = _overlap_score(f_norm, _norm(d))
        if s >= 4 and s > best_score:
            best_score = s
            best = d.strip()
    if best:
        print(f"   ⚠️ select_fungsi: '{fungsi}' di luar daftar 45 → dipetakan ke '{best}'")
        return best
    print(f"   ⚠️ select_fungsi: '{fungsi}' tidak cocok daftar 45 → fungsi kosong")
    return ""


def fetch_fungsi_list(cursor) -> list:
    cursor.execute(
        "SELECT DISTINCT trim(deskripsi) FROM klasifikasi_embedding WHERE LENGTH(kode) = 3 ORDER BY 1"
    )
    return [r[0] for r in cursor.fetchall()]


def select_fungsi(text: str, daftar: str, daftar_list: list):
    """Kembalikan (fungsi, perihal_inti) — salinan build_embed_query main.rs."""
    prompt = PROMPT_SELECT_FUNGSI.format(daftar=daftar, text=text[:3000])
    data = call_gemini_chat(prompt)
    fungsi = validate_fungsi(str(data.get("fungsi", "")), daftar_list)
    inti = str(data.get("perihal_inti", "")).strip().lower()
    if fungsi and inti:
        return f"{fungsi} > {inti}"
    return text


def build_embed_query(text: str, daftar: str, daftar_list: list) -> str:
    """select_fungsi → "FUNGSI > perihal_inti"; gagal/field kosong → fallback
    teks asli (sama seperti build_embed_query di main.rs)."""
    try:
        return select_fungsi(text, daftar, daftar_list)
    except Exception as e:  # noqa: BLE001
        print(f"   ⚠️ select_fungsi gagal, pakai teks asli: {e}")
        return text


# ---------- 4. Backfill ----------

def main():
    print(f"🔑 {len(API_KEYS)} Gemini key(s) dimuat")
    conn = psycopg2.connect(DATABASE_URL)
    conn.autocommit = True
    cur = conn.cursor()

    daftar_list = fetch_fungsi_list(cur)
    daftar = ", ".join(daftar_list)
    if not daftar_list:
        print("❌ Daftar fungsi kosong — DB tidak punya data klasifikasi?")
        sys.exit(1)

    cur.execute(
        "SELECT id, perihal, naskah FROM klasifikasi_feedback "
        "WHERE status = 'validated' AND embedding IS NULL "
        "AND (feedback_type = 'positive' OR feedback_type = 'correction') "
        "ORDER BY id"
    )
    rows = cur.fetchall()
    if not rows:
        print("✅ Tidak ada feedback validated tanpa embedding — tidak ada yang perlu dibackfill.")
        return

    print(f"🔄 Backfill {len(rows)} feedback validated tanpa embedding...")
    ok = 0
    fail = 0
    for (fid, perihal, naskah) in rows:
        # Teks sumber: perihal bila ada (lebih bersih), fallback potongan naskah
        src = (perihal or "").strip() or (naskah or "").strip()
        if not src:
            print(f"   ⏭️  id={fid}: perihal & naskah kosong — dilewati")
            continue
        try:
            embed_query = build_embed_query(src, daftar, daftar_list)
            vec = call_embed(embed_query)
            if len(vec) != DIM:
                raise RuntimeError(f"dimensi embedding {len(vec)} ≠ {DIM}")
            vec_str = "[" + ",".join(repr(float(v)) for v in vec) + "]"
            cur.execute(
                "UPDATE klasifikasi_feedback SET embedding = %s::vector WHERE id = %s",
                (vec_str, fid),
            )
            ok += 1
            print(f"   ✅ id={fid} embed_query={embed_query[:70]}...")
        except Exception as e:  # noqa: BLE001
            fail += 1
            print(f"   ❌ id={fid} gagal: {e}")

    print(f"\n📊 Selesai: {ok} di-backfill, {fail} gagal")
    print("   (fetch_fewshot sekarang akan menemukan feedback ini untuk naskah serupa)")


if __name__ == "__main__":
    main()
