# 📦 Panduan Upload Ekstensi ke Chrome Web Store

> **Ekstensi:** Analisa Naskah SRIKANDI
> **Folder source:** `kode-klasifikasi-chat/srikandi-extension/extension`
> **Folder di Windows:** `C:\Users\yudi\srikandi-extension`
> **Versi manifest:** MV3

---

## ⚠️ 0. Baca Dulu — Prasyarat Paling Penting: Backend Publik

Ekstensi ini **membutuhkan server API** "Kode Klasifikasi" (backend Rust `kode-klasifikasi-chat`, PostgreSQL-only / pgvector). Default API saat ini: `http://localhost:3100`.

Chrome Web Store menuntut ekstensi berfungsi **tanpa konfigurasi internal oleh pengguna**. Ekstensi yang bergantung server `localhost`:

1. **Hampir pasti DITOLAK** review dengan alasan *Minimal Functionality* — reviewer tidak bisa menjalankan backend Anda.
2. **Tidak berguna bagi pengguna lain** — tidak ada orang lain yang punya backend Anda di mesinnya.

### ✅ Yang harus dilakukan sebelum upload

**Deploy backend ke server publik dengan HTTPS.** Dua opsi:

| Opsi | Cocok untuk | Catatan |
|---|---|---|
| **VPS + domain + SSL** (rekomendasi) | Produksi / store | Backend resmi instansi; lihat bagian 1 di bawah |
| **Tunnel sementara** (ngrok / Cloudflare Tunnel) | Uji coba / unlisted | URL berubah tiap restart (kecuali domain statis berbayar) — kurang ideal untuk store |

Setelah backend publik, lakukan **4 perubahan di ekstensi**:
1. `extension/background.js` → ganti `DEFAULT_API_URL` (baris ~11) dari `http://localhost:3100` ke `https://api.domain-anda.com`
2. `extension/manifest.json` → tambahkan domain baru ke `host_permissions` (bagian 6)
3. `extension/background.js` → fungsi `webAppUrl()` (baris ~352): untuk domain non-localhost ia mengembalikan `API_BASE` apa adanya (asumsi web app & API share origin via nginx) — verifikasi asumsi ini untuk domain Anda
4. `extension/background.js` → pesan error `KNOWN_ERRORS` (baris ~166) masih menyebut "backend kode-klasifikasi-chat di localhost:3100" — ganti teksnya agar sesuai domain publik

---

## 1. Deploy Backend ke VPS (ringkas)

Script deploy untuk backend **baru** (Rust, PostgreSQL-only) ada di repo `kode-klasifikasi-chat`:
- `deploy/` → `deploy-to-vps.sh`, `kode-klasifikasi.service`, `nginx-kode-klasifikasi-vps.conf`, `db-setup-vps.sh`
- `start-app.sh` → jalankan lokal / referensi environment

```bash
# Setelah service berjalan & nginx dikonfigurasi:
sudo apt install -y certbot python3-certbot-nginx
sudo certbot --nginx -d api.domain-anda.com

# Verifikasi
curl https://api.domain-anda.com/api/health   # → {"status":"ok"}
```

> ⚠️ **Catatan:** panduan deploy arsitektur **lama** (Go API + Meilisearch) di folder ini sudah **dihapus**. Backend sekarang **PostgreSQL-only** — ikuti README & `deploy/` di `kode-klasifikasi-chat`.

---

## 2. Login Google (OAuth PKCE) — Redirect URI Wajib Terdaftar

Ekstensi memakai `chrome.identity.launchWebAuthFlow` → browser me-redirect ke:
```
https://<EXTENSION_ID>.chromiumapp.org/
```

**Extension ID stabil** selama `key` di `manifest.json` dipertahankan — dan **ID unpacked = ID store** (Chrome Web Store menghitung ID item dari kunci publik di package). Jadi login Google tetap jalan setelah publish.

Langkah:
1. **Cek ID extension saat ini**: `chrome://extensions` → aktifkan *Developer mode* → lihat ID. (Contoh: `egpmgoopjkaookacalkdnkgafeacpoje`)
2. **Google Cloud Console** (project OAuth yang sama dengan `GOOGLE_CLIENT_ID` di `backend/.env`):
   - *OAuth consent screen* → pastikan status **Production** (mode *Testing* hanya bertahan 7 hari & hanya untuk akun test)
   - *Credentials* → OAuth Client ID → *Authorized redirect URIs* → tambahkan `https://<EXTENSION_ID>.chromiumapp.org/`
3. **Backend `.env`**: pastikan `GOOGLE_REDIRECT_URI` menyertakan `https://<EXTENSION_ID>.chromiumapp.org/` (sudah ada sejak pengujian; verifikasi tetap sebelum deploy VPS)
4. Restart backend setelah mengubah `.env`

---

## 3. Daftar Developer Account

1. Buka **[Chrome Web Store Developer Dashboard](https://chrome.google.com/webstore/devconsole)**
2. Login dengan Google Account
3. Bayar **$5 (sekali seumur hidup)** — registrasi developer
4. Isi data profil developer (nama, email publik)

---

## 4. Privacy Policy — WAJIB

Ekstensi **mengumpulkan & mengirim data pengguna** ke server Anda:
- Teks naskah (isi dokumen) yang dianalisa
- Nama lengkap pengguna SRIKANDI (di-scrape dari halaman)
- Nama & email Google (hanya saat user login untuk koreksi)

Karena itu Chrome Web Store **mewajibkan URL Privacy Policy** di form listing. Siapkan halaman publik (GitHub Pages / Notion / halaman instansi) yang menjelaskan minimal:
- Data apa yang dikumpulkan & dikirim (ke server instansi)
- Tujuan (analisa klasifikasi arsip, feedback/koreksi)
- Tidak dibagikan ke pihak ketiga; disimpan di server instansi
- Cara kontak / permintaan penghapusan data

---

## 5. Siapkan Material Listing

| Item | Spesifikasi | Status |
|---|---|---|
| **File ZIP ekstensi** | Isi folder `extension` (lihat bagian 7) | 🔲 Buat |
| **Deskripsi** | Maks 16.000 karakter (contoh di bawah) | 🔲 Tulis |
| **Screenshot (1280×800)** | Minimal 1, maks 5 — modal hasil analisa, form terisi | 🔲 Ambil |
| **Small promo tile** | 440×280 px (opsional untuk tampil di kategori) | 🔲 |
| **Marquee promo tile** | 1400×560 px (opsional) | 🔲 |
| **Icon 128×128** | **PNG** (wajib untuk listing) — konversi dari SVG | 🔲 Konversi |
| **Privacy policy URL** | Lihat bagian 4 | 🔲 |

### Konversi ikon SVG → PNG

Chrome Web Store menerima **PNG** untuk listing. Ikon saat ini `.svg`:

```bash
# Di WSL (ImageMagick) — jalankan dari folder extension/icons:
for s in 16 48 128; do
  convert -background none icon${s}.svg icon${s}.png
done
```

Kalau ImageMagick tidak tersedia: gunakan Inkscape, atau generator online (cari "svg to png").

### Contoh deskripsi

```
Analisa Naskah SRIKANDI adalah ekstensi Chrome yang membantu Arsiparis
mengklasifikasikan naskah dinas di aplikasi SRIKANDI secara otomatis
menggunakan AI.

Fitur:
- 🔍 Tombol "Analisa dengan AI" di halaman registrasi naskah
- 📄 Baca file DOCX/PDF langsung dari form upload (DOCX di browser, PDF via server)
- 🤖 Perihal, Isi Ringkas, Penjelasan AI, & Kode Klasifikasi (semantic search)
- 👍 Feedback positif anonim (tercatat dengan sesi chat)
- ✏️ Koreksi kode dengan login Google (OAuth PKCE)
- 📝 Isi form SRIKANDI otomatis dengan hasil analisa

Cara pakai:
1. Pastikan server API instansi aktif (lihat repositori)
2. Buka halaman registrasi naskah di SRIKANDI
3. Upload file DOCX/PDF seperti biasa
4. Klik "Analisa dengan AI", periksa hasil, beri feedback
5. Klik "Sisipkan ke Naskah" — form terisi otomatis

Kategori: Productivity
Bahasa: Bahasa Indonesia
```

---

## 6. Update `host_permissions` di manifest.json

Default saat ini:
```json
"host_permissions": [
  "https://srikandi.arsip.go.id/*",
  "http://localhost:*/*",
  "http://127.0.0.1:*/*"
],
"optional_host_permissions": [
  "https://*/*",
  "http://*/*"
]
```

**Tambahkan domain API publik** (agar review cepat & instalasi mulus, sebaiknya juga hapus `http://localhost` / `http://127.0.0.1` pada build store — reviewer bisa mempertanyakan alasan mengakses localhost):
```json
"host_permissions": [
  "https://srikandi.arsip.go.id/*",
  "https://api.domain-anda.com/*"
]
```

> Ekstensi memakai `optional_host_permissions` untuk domain lain, jadi pengguna tetap bisa ganti URL API sendiri lewat popup.

---

## 7. Buat File ZIP — JANGAN Ikutkan Private Key!

> ⛔ **KRITIS:** folder `keys/` berisi **private key** (`srikandi-extension-private.pem`) yang menentukan identitas ekstensi. **JANGAN PERNAH** ikutkan ke ZIP / commit / publikasikan. Kalau bocor, orang lain bisa membangun ekstensi dengan identitas yang sama dengan milik Anda.

ZIP harus berisi **isi folder `extension`** (manifest.json di root ZIP), **tanpa** folder `keys`:

```powershell
# Di Windows PowerShell (folder srikandi-extension):
cd C:\Users\yudi
# 1. Copy isi folder extension ke folder bersih
Copy-Item srikandi-extension\extension\* srikandi-extension-package\ -Recurse
# 2. Pastikan tidak ada keys/ di dalam package
Remove-Item srikandi-extension-package\keys -Recurse -Force -ErrorAction SilentlyContinue
# 3. ZIP isi folder package (manifest.json di root)
Compress-Archive -Path srikandi-extension-package\* -DestinationPath srikandi-extension.zip
```

Struktur ZIP yang benar:
```
srikandi-extension.zip
├── manifest.json          ← WAJIB di root
├── background.js
├── content_script.js
├── content_styles.css
├── popup/                 (popup.html, popup.js, popup.css)
├── icons/                 (icon16/48/128 — PNG)
└── vendor/                (mammoth.browser.min.js)
```

---

## 8. Upload ke Dashboard

1. Klik **New Item**
2. Upload ZIP dari bagian 7
3. Isi form:
   - **Name**: Analisa Naskah SRIKANDI
   - **Description**: contoh di bagian 5
   - **Category**: Productivity
   - **Language**: Bahasa Indonesia
4. Upload **screenshot** (minimal 1, format 1280×800 atau 640×400)
5. Upload **icon PNG 128×128** + promo tiles
6. Isi **Privacy Policy URL** (bagian 4)
7. Pilih visibilitas: **Unlisted** (uji coba dulu) atau **Public**

---

## 9. Review & Publikasi

1. Setelah submit, Google mereview **1–3 hari kerja**
2. Status di dashboard: *Pending Review* → *Published* / *Rejected*
3. Kalau **ditolak**, baca alasannya, perbaiki, submit ulang

### Alasan penolakan umum & solusinya

| Alasan | Solusi |
|---|---|
| **Minimal functionality** | Backend publik HTTPS wajib (bagian 0) — ini penyebab paling umum |
| **Broad host permissions** | Pakai `optional_host_permissions`; hapus `localhost` di build store |
| **Privacy** | Sediakan privacy policy URL (bagian 4) |
| **Deceptive installation** | UI harus jujur; jangan teknik menipu |
| **Single purpose violation** | Pastikan semua fitur terkait satu tujuan (klasifikasi naskah) |

---

## 10. Update Versi

1. Naikkan `"version"` di `manifest.json` (misal `1.2.0` → `1.2.1`)
2. Zip ulang (bagian 7)
3. Dashboard → item → **Upload Updated Package**
4. Review ulang (biasanya lebih cepat)

---

## ✅ Checklist Sebelum Upload

- [ ] Backend publik HTTPS aktif & `curl https://api…/api/health` OK
- [ ] `DEFAULT_API_URL` di `background.js` diubah ke URL publik
- [ ] `host_permissions` memuat domain API (localhost dihapus)
- [ ] Redirect URI `https://<ID>.chromiumapp.org/` terdaftar di Google Cloud + `GOOGLE_REDIRECT_URI` backend
- [ ] Ikon PNG (16/48/128) dibuat
- [ ] Screenshot & promo tiles siap
- [ ] Privacy policy URL siap & diisi
- [ ] ZIP bersih (isi folder `extension`, **tanpa** `keys/`)
- [ ] Versi di `manifest.json` sudah sesuai
