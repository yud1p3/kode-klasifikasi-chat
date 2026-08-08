# Catatan Diskusi — 8 Juli 2026

## Topik: Pengembangan Chrome Extension untuk Analisa Naskah SRIKANDI

---

### Latar Belakang
Project `browser-klasifikasi-arsip` (klas.site) sudah berjalan dengan:
- React SPA untuk browse & search klasifikasi arsip
- Golang API untuk analisa naskah (2-stage Gemini + feedback + sub-klasifikasi)
- Meilisearch sebagai database pencarian

Rencana: Kembangkan Chrome Extension untuk integrasi langsung dengan SRIKANDI.

---

### Keputusan Arsitektur

**1. Extension terpisah dari project browser-klasifikasi-arsip**
- ✅ Tidak mengubah kode existing (klas.site tetap jalan)
- ✅ Golang API endpoint yang sama (localhost:3001)
- ✅ Reuse komponen React untuk popup (adaptasi dari AnalisaNaskah.jsx)

**2. Alur kerja extension:**
1. User upload DOCX template di SRIKANDI
2. Content script inject tombol "Analisa dengan AI" di halaman
3. Klik → baca file dari hidden input → kirim ke Golang API
4. Tampilkan loading modal
5. Selesai → modal hasil (perihal, ringkasan, kode klasifikasi)
6. "Masukkan ke Konsep" → isi form SRIKANDI otomatis

**3. Teknis DOM SRIKANDI:**
- Hal (Perihal): `textarea[name="hal"]`
- Isi Ringkas: `textarea[name="ringkasan"]`
- Klasifikasi: React-Select (`input[role="combobox"][aria-autocomplete="list"]`)
- Upload file: `input[type="file"][accept*="docx"]` (react-dropzone)

**4. Golang API — endpoint baru (ditambahkan):**
- `POST /api/analisa-dari-teks` → `{"teks": "..."}`
- `POST /api/analisa-dari-file` → `{"fileName":"...","fileData":"base64...","fileExt":"docx"}`
- Keduanya reusable tanpa mengubah endpoint lama

---

### Struktur Extension

```
srikandi-scraper/extension/
├── manifest.json           ← MV3, inject ke srikandi.arsip.go.id
├── content_script.js       ← Inject tombol, modal, baca & isi form
├── content_styles.css      ← Styling overlay
├── background.js           ← Service worker: relay API
├── popup/
│   ├── popup.html/popup.css/popup.js   ← UI hasil analisa
└── icons/
```

### Yang sudah dibuat (8 Juli 2026, pagi):
1. ✅ manifest.json (MV3)
2. ✅ content_script.js — inject tombol, baca file, modal hasil, isi form (termasuk react-select)
3. ✅ content_styles.css
4. ✅ background.js — service worker + relay
5. ✅ popup/ — HTML + CSS + JS
6. ✅ icons/ (SVG)
7. ✅ Golang API — 2 endpoint baru + `runAnalysisPipeline()` + `convertPdfToTxt()`
8. ✅ Go build sukses (0 error)
9. ✅ `klas.site` dipastikan tidak terganggu — semua endpoint lama tetap ada

### Yang perlu dilanjutkan nanti:
- [ ] Test load extension di Chrome
- [ ] Uji coba inject button di SRIKANDI (perlu login SRIKANDI)
- [ ] Uji coba baca file dari react-dropzone
- [ ] Uji coba isi react-select otomatis
- [ ] Konfirmasi selector DOM Klasifikasi (react-select) di SRIKANDI live
- [ ] Setup Golang API key (env) untuk production
- [ ] Opsi: build popup dengan React (Vite) untuk komponen yang lebih kompleks
