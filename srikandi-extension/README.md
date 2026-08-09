# 📦 Srikandi Extension — Analisa Naskah SRIKANDI

Chrome Extension (MV3) untuk aplikasi **SRIKANDI** (`srikandi.arsip.go.id`).
Dibuat dari project `srikandi-scraper` dan diadaptasi agar bekerja dengan
**backend Kode Klasifikasi** (repo ini — `kode-klasifikasi-chat`, Rust/Actix + pgvector).

Tujuan sama dengan aplikasi web: memperoleh **perihal**, **isi ringkas**, **penjelasan AI**, dan
**kode klasifikasi** dari naskah (PDF/DOCX) langsung di halaman SRIKANDI, lalu
mengisi form (Hal, Ringkasan, Klasifikasi) secara otomatis.

> 💡 **Isi ringkas hanya untuk extension.** Extension mengirim
> `include_ringkasan: true` pada `POST /api/chat` sehingga backend juga
> menghasilkan `ringkasan` (isi ringkas naskah). Versi web TIDAK mengirim opsi ini
> sehingga respons & perilakunya tidak berubah sama sekali.

> ⚠️ **Catatan:** extension ini **vanilla JavaScript** (tanpa build step).
> Renungan awal untuk popup React (Vite) tercatat di `CATATAN_DISKUSI_2026-07-08.md`,
> namun belum diimplementasikan.

> ⚠️ **Keamanan naskah:** JANGAN upload naskah rahasia atau naskah berisi informasi
> yang dikecualikan (rahasia negara, data pribadi, rahasia jabatan — istilah UU No.
> 14/2008 tentang Keterbukaan Informasi Publik). Isi naskah dikirim ke server API
> (Gemini) untuk dianalisa. Peringatan ini juga ditampilkan di popup dan di bawah
> tombol "Analisa dengan AI" di halaman SRIKANDI.

---

## Struktur

```
srikandi-extension/
├── extension/                    ← folder yang di-load sebagai extension
│   ├── manifest.json
│   ├── background.js             ← relay ke API backend (sinkron)
│   ├── content_script.js         ← inject tombol, baca file, modal hasil, isi form
│   ├── content_styles.css
│   ├── vendor/mammoth.browser.min.js  ← ekstraksi DOCX client-side
│   ├── popup/                    ← popup (pengaturan, riwayat feedback, tombol aksi)
│   └── icons/
├── README.md                     ← dokumen ini
├── PANDUAN_UPLOAD_CHROME_WEB_STORE.md
├── CATATAN_DISKUSI_2026-07-08.md
├── deploy-to-vps.sh              ⚠️ legacy: deploy API Golang browser-klasifikasi-arsip (TIDAK dipakai untuk repo ini)
├── setup-meilisearch-vps.sh      ⚠️ legacy: setup Meilisearch Golang (TIDAK dipakai)
└── PANDUAN_DEPLOY_VPS.md         ⚠️ legacy: panduan deploy Golang (TIDAK dipakai)
```

File bertanda ⚠️ adalah peninggalan dari project `browser-klasifikasi-arsip`
(Golang + Meilisearch) — **tidak berlaku** untuk backend baru (Rust + pgvector).
Bisa dihapus bila tidak dibutuhkan.

---

## Integrasi API — mapping endpoint

| Fungsi extension | Endpoint lama (Golang) | **Endpoint baru (kode-klasifikasi-chat)** |
|---|---|---|
| Analisa teks | `POST /api/analisa-dari-teks` (task_id + polling) | **`POST /api/chat`** `{message, include_ringkasan:true}` — sinkron, hasil langsung |
| Analisa file PDF | `POST /api/analisa-dari-file` (base64) | **`POST /api/extract-pdf`** (multipart) → `POST /api/chat` |
| Analisa file DOCX | server-side (Golang) | **client-side mammoth** → `POST /api/chat` |
| Feedback 👍 Setuju | `POST /api/analisa/feedback` (setuju_sub) | **`POST /api/feedback`** `{feedback_type:'positive'}` — anonim |
| Feedback ✏️ Koreksi | `POST /api/analisa/feedback` (koreksi_all) | **`POST /api/feedback`** `{feedback_type:'correction'}` + `Authorization: Bearer` (wajib login) |
| Login Google | `POST /api/auth/extension` (chrome.identity) | **Langsung di extension** — `chrome.identity.launchWebAuthFlow` + PKCE → `POST /api/auth/google` |

### Pemetaan hasil `/api/chat` → hasil extension

```jsonc
// Respons backend (karena extension mengirim include_ringkasan:true):
{ "results": [{ "id", "kode", "deskripsi", "path", "similarity" }, ...],
  "perihal": "...", "explanation": "...", "ringkasan": "..." }

// Disusun ulang oleh background.js (mapChatResult) menjadi:
{
  "perihal": "...",
  "isi_ringkas": "...",              // ringkasan naskah → field "Isi Ringkas" + form SRIKANDI
  "explanation": "...",              // ditampilkan sebagai "Penjelasan AI"
  "kode_klasifikasi": "800.12.02",   // kandidat teratas
  "klasifikasi_deskripsi": "...",
  "sub_klasifikasi": [ { "kode", "deskripsi", "path", "similarity" }, ... ], // 5 kandidat
  "kode_detil": "...",
  "detil_deskripsi": "..."
}
```

**Catatan:** `ringkasan` (isi ringkas) hanya dihasilkan bila request menyertakan
`include_ringkasan: true` — itulah yang dikirim extension. Versi web tidak
mengirimnya sehingga respons web tidak berubah.

---

## Model autentikasi (mengikuti backend baru)

| Aksi | Login? |
|---|---|
| Chat & analisa (teks/PDF/DOCX) | ❌ Tidak (rate limit & kuota berlaku) |
| Feedback 👍 Setuju | ❌ Tidak — **anonim**, tercatat dengan `chat_id` (ID sesi acak per browser) |
| Feedback ✏️ Koreksi | ✅ Wajib login Google (langsung dari extension) |
| Login Google | **Langsung di extension** — `launchWebAuthFlow` + PKCE, JWT disimpan di `chrome.storage.local` |

`chat_id` disimpan di `chrome.storage.local` (`srikandi_chat_id`) dan dikirim
bersama feedback — konsisten dengan fitur `chat_id` aplikasi web.

### Login Google & Koreksi — alur

1. Klik **✏️ Koreksi** di modal hasil → jika belum login, muncul tombol **"🔑 Login dengan Google"**.
2. `background.js` membuat pasangan PKCE (verifier/challenge) lalu membuka halaman Google via
   `chrome.identity.launchWebAuthFlow` (redirect ke `https://<EXTENSION_ID>.chromiumapp.org/`).
3. Setelah user menyetujui, extension menerima `code` → ditukar di **`POST /api/auth/google`**
   (client_secret tetap di server — extension TIDAK pernah memegang token Google, hanya JWT sesi backend).
4. JWT disimpan di `chrome.storage.local` (`ext_token` / `ext_user`); koreksi dikirim dengan
   `Authorization: Bearer <JWT>`.

### Konfigurasi sekali (agar login berfungsi)

Extension unpacked TETAP bisa login Google — **tidak perlu upload ke Chrome Web Store**.
Yang perlu disiapkan:

| # | Langkah | Keterangan |
|---|---|---|
| 1 | **Google Cloud Console** | Tambah redirect URI `https://<EXTENSION_ID>.chromiumapp.org/` ke **Authorized redirect URIs** di OAuth client yang dipakai aplikasi web (client yang sama; boleh banyak redirect URI). |
| 2 | **Backend `.env`** | Tambah ke `GOOGLE_REDIRECT_URI` (comma-separated): `,https://<EXTENSION_ID>.chromiumapp.org/` lalu restart backend. |
| 3 | **Reload extension** | Setelah manifest berubah, reload di `chrome://extensions`. |

> **Extension ID stabil (`key`):** manifest menyertakan field `"key"` (kunci publik, dibangkitkan
> dari keypair di `keys/srikandi-extension-private.pem` — **jangan di-commit**, sudah di-`.gitignore`).
> Dengan `key`, Extension ID terkunci permanen = **`egpmgoopjkaookacalkdnkgafeacpoje`** di semua mesin,
> sehingga redirect URI tidak berubah antar komputer.

**Redirect URI extension:** `https://egpmgoopjkaookacalkdnkgafeacpoje.chromiumapp.org/`

---

## Cara pakai

1. **API server aktif** — jalankan backend repo ini (lihat README utama):
   ```bash
   bash start-meili-app.sh
   # backend di http://localhost:3100
   ```
2. **Load extension** di Chrome: `chrome://extensions` → *Developer mode* →
   *Load unpacked* → pilih folder `srikandi-extension/extension`.
   (Di Windows: jalankan `sync-to-windows.sh` lalu load dari
   `C:\Users\yudi\srikandi-extension`.)
3. Buka halaman **Registrasi Naskah Keluar** di SRIKANDI, upload file
   DOCX/PDF, klik tombol **"Analisa dengan AI"**.
4. Periksa hasil → pilih **👍 Setuju** (anonim) atau **✏️ Koreksi** (login Google) → "Sisipkan ke Naskah".

### Pengaturan (popup ⚙️)

| Setelan | Keterangan |
|---|---|
| **Akun Google** | Login/logout Google (diperlukan untuk ✏️ Koreksi). |
| **Gemini API Keys** | Opsional, 1 key per baris. Dikirim sebagai `api_keys` (rotasi otomatis di backend). |
| **Server API** | URL backend. Default `http://localhost:3100`. |
| **Aplikasi Web** | Tombol buka aplikasi web Kode Klasifikasi (chat, statistik). URL dihitung otomatis: `localhost:3100` → `localhost:5174`; selain itu = origin API. |

---

## Verifikasi API (smoke test)

```bash
# Chat (tanpa login)
curl -s -X POST http://localhost:3100/api/chat \
  -H 'Content-Type: application/json' \
  -d '{"message":"Permohonan cuti tahunan pegawai"}'

# Feedback positif anonim
curl -s -X POST http://localhost:3100/api/feedback \
  -H 'Content-Type: application/json' \
  -d '{"message":"Permohonan cuti","kode_ai":"800.12.02","feedback_type":"positive","chat_id":"test-123"}'

# Cari kode
curl -s 'http://localhost:3100/api/codes?q=cuti'
```
