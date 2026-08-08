# 📦 Panduan Upload Ekstensi ke Chrome Web Store

> **Ekstensi:** Analisa Naskah SRIKANDI
> **Folder:** `C:\Users\yudi\srikandi-extension`
> **Versi manifest:** MV3

---

## 1. Daftar Developer Account

1. Buka **[Chrome Web Store Developer Dashboard](https://chrome.google.com/webstore/devconsole)**
2. Login dengan **Google Account**
3. Bayar **$5 (sekali seumur hidup)** — registrasi developer
4. Isi data profil developer (nama, email publik)

---

## 2. Siapkan Material

| Item | Spesifikasi | Status |
|---|---|---|
| **File ZIP ekstensi** | Folder extension di-zip | 🔲 Buat sebelum upload |
| **Deskripsi (English/Indonesia)** | Maks 16.000 karakter | 🔲 Tulis |
| **Screenshot (1280×800)** | Minimal 1, maks 5 | 🔲 Screenshot modal hasil analisa |
| **Small promo tile** | 440×280 pixel | 🔲 Bisa dari screenshot |
| **Marquee promo tile** | 1400×560 pixel | 🔲 Opsional |
| **Icon 128×128** | Wajib, PNG | ✅ Ada di `icons/icon128.svg` |

### Detail deskripsi (contoh)

```
Analisa Naskah SRIKANDI adalah ekstensi Chrome yang membantu 
Arsiparis Nasional dalam mengklasifikasikan naskah dinas 
di aplikasi SRIKANDI secara otomatis menggunakan AI.

Fitur:
- 🔍 Inject tombol "Analisa dengan AI" di halaman registrasi naskah
- 📄 Baca file DOCX/PDF langsung dari form upload (DOCX diekstrak di browser)
- 🤖 Analisis otomatis: Perihal, Isi Ringkas, Penjelasan AI, Kode Klasifikasi
- 🏷️ Kode Klasifikasi (top-5 kandidat hasil semantic search pgvector)
- 👍 Feedback positif anonim (tanpa login, tercatat dengan sesi chat_id)
- ✏️ Koreksi kode dengan login Google langsung dari ekstensi (OAuth PKCE)
- 📝 Isi form SRIKANDI otomatis dengan hasil analisa

Cara pakai:
1. Pastikan API server aktif — backend "Kode Klasifikasi" (kode-klasifikasi-chat)
   di localhost:3100 (lihat README repo)
2. Buka halaman registrasi naskah di SRIKANDI
3. Upload file DOCX/PDF seperti biasa
4. Klik tombol "Analisa dengan AI"
5. Periksa hasil, beri feedback 👍 Setuju
6. Klik "Sisipkan ke Naskah" — form terisi otomatis

Catatan: 
- Ekstensi ini memerlukan server API terpisah yang berjalan di localhost 
  atau server lokal (backend kode-klasifikasi-chat, tanpa Meilisearch).
- Login Google & koreksi kode (✏️) tersedia langsung di ekstensi 
  (chrome.identity.launchWebAuthFlow + PKCE); pastikan redirect URI 
  https://<EXTENSION_ID>.chromiumapp.org/ terdaftar di OAuth client 
  Google Cloud dan GOOGLE_REDIRECT_URI backend (lihat README).

Kategori: Productivity
Bahasa: Bahasa Indonesia
```

---

## 3. Upload ke Dashboard

1. Klik **New Item**
2. Upload file ZIP — pilih folder `C:\Users\yudi\srikandi-extension`, zip
3. Isi semua field di form:
   - **Name**: Analisa Naskah SRIKANDI
   - **Description**: Copy dari atas
   - **Category**: Productivity
   - **Language**: Bahasa Indonesia
4. Upload **screenshot** (minimal 1)
5. Upload **icon** — konversi `icon128.svg` ke PNG 128×128
6. **Pilih visibilitas**: Public (atau Unlisted untuk testing)

---

## 4. Host Permissions — Perhatian!

Pada **manifest.json**, `host_permissions` sudah diatur agar tidak terlalu luas:

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

Default API backend: `http://localhost:3100` (kode-klasifikasi-chat).
Chrome akan minta **izin tambahan** hanya saat pengguna mengganti URL API 
ke domain lain (via `optional_host_permissions`). Saat instalasi, 
hanya domain SRIKANDI dan API default yang diminta.

**Jika ada domain API baru**, tambahkan ke `host_permissions` sebelum build ZIP.

---

## 5. Review & Publikasi

1. Setelah submit, Google akan **mereview** dalam 1–3 hari kerja
2. Cek dashboard untuk perubahan status:
   - **Pending Review** — sedang diperiksa
   - **Published** — sudah live
   - **Rejected** — akan ada alasannya, perbaiki lalu submit ulang

### Alasan penolakan umum & solusinya

| Alasan | Solusi |
|---|---|
| **Broad host permissions** | Sudah diatasi dengan `optional_host_permissions` |
| **Minimal functonality** | Pastikan ekstensi punya fungsi jelas tanpa server — beri pesan error informatif |
| **Deceptive installation** | Jangan pakai teknik menipu, UI harus jujur |
| **Privacy** | Pastikan tidak mengumpulkan data pribadi tanpa izin |

---

## 6. Update Versi

Untuk update:

1. Naikkan `"version"` di `manifest.json` (misal `1.0.0` → `1.0.1`)
2. Zip ulang folder extension
3. Buka dashboard → pilih item → **Upload Updated Package**
4. Chrome akan review ulang (biasanya lebih cepat)

---

## 7. ZIP Extension

```powershell
# Di Windows PowerShell:
cd C:\Users\yudi
Compress-Archive -Path srikandi-extension\* -DestinationPath srikandi-extension.zip
```
