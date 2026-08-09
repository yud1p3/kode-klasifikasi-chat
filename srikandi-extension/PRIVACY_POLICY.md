# Kebijakan Privasi — Ekstensi Chrome "Analisa Naskah SRIKANDI"

> **Tanggal efektif:** [ISI TANGGAL, mis. 8 Agustus 2026]
> **Pengelola:** [ISI NAMA INSTANSI / DINAS, mis. Dinas Perpustakaan dan Kearsipan Kabupaten Blitar]
> **Kontak:** [ISI EMAIL KONTAK, mis. kearsipan@blitarkab.go.id]

---

## 1. Pendahuluan

Ekstensi Chrome **"Analisa Naskah SRIKANDI"** membantu arsiparis mengklasifikasikan naskah dinas di aplikasi SRIKANDI secara otomatis menggunakan kecerdasan buatan (AI). Ekstensi ini dikembangkan dan dikelola oleh **[NAMA INSTANSI]** dan memerlukan server API milik instansi untuk bekerja.

Kebijakan Privasi ini menjelaskan data apa saja yang dikumpulkan, bagaimana data digunakan, dan hak Anda sebagai pengguna. Dengan memasang dan menggunakan ekstensi ini, Anda dianggap telah membaca dan menyetujui kebijakan ini.

> ⚠️ **Tanggung jawab kerahasiaan naskah ada pada pengguna.** Ekstensi ini **tidak** menyaring klasifikasi keamanan naskah sebelum dikirim ke layanan AI. Sebelum menekan "Analisa dengan AI", pastikan naskah yang akan dianalisa **bukan** naskah rahasia dan **tidak** berisi informasi yang dikecualikan (rahasia negara, data pribadi, rahasia jabatan, dan sebagainya — Pasal 17 UU No. 14/2008 tentang Keterbukaan Informasi Publik). Mengirim naskah berklasifikasi ke layanan AI adalah tanggung jawab pengguna sepenuhnya.

---

## 2. Data yang Dikumpulkan

### 2.1 Data yang dikirim ke server instansi

| Data | Kapan dikumpulkan | Tujuan |
|---|---|---|
| **Isi naskah dokumen** (teks hasil ekstraksi dari file DOCX/PDF) | Saat Anda menekan tombol "Analisa dengan AI" | Menghasilkan perihal, isi ringkas, dan kode klasifikasi |
| **Nama lengkap pengguna SRIKANDI** | Dibaca dari halaman SRIKANDI yang sedang Anda gunakan; dikirim bersama umpan balik (feedback) | Mencatat siapa yang memberi feedback / koreksi |
| **Kode klasifikasi & perihal hasil analisa** | Saat Anda memberi feedback (👍 Setuju / ✏️ Koreksi) | Mencatat feedback untuk perbaikan kualitas AI |
| **ID sesi perangkat** (`chat_id`) | Dibuat acak sekali per perangkat/browser, disimpan di `chrome.storage.local` | Mengaitkan feedback ke sesi analisa tanpa perlu login |

### 2.2 Data yang dikumpulkan hanya saat Anda login (opsional)

| Data | Kapan dikumpulkan | Tujuan |
|---|---|---|
| **Nama & alamat email akun Google** | HANYA saat Anda memilih "Login dengan Google" untuk mengirim **koreksi** | Akuntabilitas koreksi; menampilkan identitas Anda pada riwayat feedback |
| **Token sesi** (JWT dari server instansi) | Setelah login berhasil | Menjaga sesi login Anda; **tidak** berisi password atau token Google |

Login Google bersifat **opsional** — memberi feedback positif dan menggunakan fitur analisa **tidak memerlukan login**.

### 2.3 Data yang disimpan di perangkat Anda (browser)

- `api_base_url` — alamat server API yang Anda atur
- `srikandi_user_name` — nama pengguna SRIKANDI (hasil pembacaan, untuk cache)
- `srikandi_chat_id` — ID sesi anonim
- `ext_token` / `ext_user` — token & info sesi (hanya jika Anda login)
- `gemini_api_keys` — **API Key Gemini milik Anda** (opsional, jika Anda memilih menyimpannya untuk memakai kuota sendiri)

Seluruh data di atas hanya tersimpan di browser Anda (storage lokal ekstensi) dan **tidak pernah dikirim** ke pihak lain kecuali server instansi.

---

## 3. Cara Data Digunakan

Data yang dikumpulkan digunakan **hanya untuk**:

1. Menjalankan fitur inti: analisa naskah (perihal, isi ringkas, kode klasifikasi) dan mengisi formulir SRIKANDI secara otomatis.
2. Mencatat feedback dan koreksi arsiparis — yang kemudian dipakai untuk **memperbaiki kualitas klasifikasi AI** (sebagai contoh pembelajaran / few-shot).
3. Statistik internal instansi (mis. jumlah analisa, kode yang paling sering dikoreksi).
4. Menjaga keamanan layanan (pembatasan pemakaian, pencegahan penyalahgunaan).

Data **tidak digunakan** untuk iklan, profil komersial, atau tujuan lain di luar klasifikasi arsip.

---

## 4. Pemrosesan oleh Layanan AI Pihak Ketiga

Untuk menghasilkan analisa, teks naskah dikirim ke **layanan Google Gemini** (via API resmi) menggunakan kunci API milik instansi atau kunci API yang Anda sediakan. Perlu diketahui:

- Teks yang dikirim adalah **isi naskah dinas** yang Anda analisa.
- Google menggunakan data untuk memproses permintaan Anda saat itu; **kebijakan Google** berlaku untuk pemrosesan ini (lihat Kebijakan Privasi Google).
- Data **tidak dijual** dan tidak digunakan untuk iklan.

> ⚠️ **JANGAN menganalisa naskah rahasia atau naskah berisi informasi yang dikecualikan.**
>
> Kerahasiaan naskah **sepenuhnya merupakan tanggung jawab pengguna dan instansi pengguna**. Ekstensi menampilkan peringatan (di popup dan di bawah tombol "Analisa dengan AI"), namun peringatan tersebut **tidak menggantikan** kewajiban pengguna untuk memastikan naskah yang dianalisa adalah naskah biasa (non-rahasia). "Informasi yang dikecualikan" mencakup antara lain rahasia negara, informasi yang dapat membahayakan keamanan/pertahanan negara, hak pribadi (data pribadi), rahasia jabatan, dan rahasia bisnis (Pasal 17 UU No. 14/2008 jo. PP No. 61/2010).
>
> Jika kebijakan instansi melarang pengiriman isi naskah ke layanan eksternal, gunakan server instansi dengan konfigurasi yang sesuai, atau jangan gunakan fitur analisa AI untuk dokumen tersebut.

---

## 5. Penyimpanan & Keamanan

- Data feedback disimpan di **server milik instansi** (basis data PostgreSQL) dengan akses terbatas.
- Naskah yang disimpan di server **dipotong** (maksimal ±1.000 karakter) — cukup untuk substansi klasifikasi.
- Komunikasi antara ekstensi dan server dilakukan melalui **HTTPS** (terenkripsi).
- Akses ke fitur admin (mis. menghapus feedback) dibatasi: hanya admin instansi yang berwenang, dengan perlindungan tambahan (verifikasi password & pencegahan percobaan berulang).
- Kami menerapkan upaya keamanan yang wajar untuk melindungi data, namun tidak ada metode transmisi atau penyimpanan yang 100% aman.

---

## 6. Berbagi Data

Kami **tidak menjual, menyewakan, atau membagikan** data pribadi Anda kepada pihak ketiga, kecuali:

1. **Penyedia layanan AI** (Google Gemini) — hanya untuk memproses permintaan analisa Anda (lihat bagian 4).
2. **Aparat penegak hukum / pihak berwenang** — bila diwajibkan oleh hukum yang berlaku.
3. **Pihak lain dengan persetujuan Anda.**

Akses ke data feedback di server hanya diberikan kepada pejabat/petugas instansi yang berwenang.

---

## 7. Retensi & Penghapusan Data

- Data feedback disimpan **selama diperlukan** untuk keperluan klasifikasi dan perbaikan kualitas AI, sesuai ketentuan kearsipan yang berlaku.
- Anda dapat **meminta penghapusan** data feedback yang tercatat atas nama Anda dengan menghubungi kontak di bagian 11.
- Data di perangkat Anda (nama, sesi, chat_id, API key) dapat dihapus kapan saja dengan: menghapus data ekstensi di `chrome://extensions` → Detail → "Hapus data", atau dengan mencopot pemasangan ekstensi.

---

## 8. Hak Anda

Sesuai peraturan perundang-undangan yang berlaku (antara lain UU Perlindungan Data Pribadi), Anda berhak untuk:

- **Mengakses** data Anda yang tersimpan;
- **Memperbaiki / mengoreksi** data yang tidak akurat;
- **Menghapus** data Anda;
- **Menarik persetujuan** penggunaan data — dengan konsekuensi fitur terkait tidak dapat digunakan (mis. koreksi).

Untuk menggunakan hak-hak tersebut, hubungi kontak pada bagian 11. Kami akan menindaklanjuti dalam waktu yang wajar.

---

## 9. Perubahan Kebijakan

Kebijakan ini dapat diperbarui sewaktu-waktu. Perubahan yang bersifat material akan diumumkan melalui halaman kebijakan ini dengan tanggal efektif yang baru. Dengan terus menggunakan ekstensi setelah perubahan, Anda dianggap menyetujui kebijakan yang diperbarui.

---

## 10. Kontak

Untuk pertanyaan, permintaan penghapusan data, atau pengaduan terkait privasi, hubungi:

- **Instansi:** [NAMA INSTANSI / DINAS]
- **Email:** [EMAIL KONTAK]
- **Alamat:** [ALAMAT INSTANSI]

---

*Terakhir diperbarui: [TANGGAL]*
