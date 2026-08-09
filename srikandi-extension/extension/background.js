// ─── Background Script: Analisa Naskah SRIKANDI ───────────────
// Service worker untuk:
// 1. Meneruskan analisa teks/file ke API Kode Klasifikasi (Rust — sinkron, hasil langsung)
// 2. Relay message content ↔ popup
//
// Adaptasi dari API Golang lama (task_id + polling) ke backend
// kode-klasifikasi-chat: POST /api/chat, /api/extract-pdf, /api/codes, /api/feedback.
// Chat & feedback positif TANPA login (anonim). Koreksi butuh login → diarahkan
// ke aplikasi web (bukan dari extension).

const DEFAULT_API_URL = 'http://localhost:3100';
let API_BASE = DEFAULT_API_URL;

// ── Helper: baca konfigurasi dari storage ─────────────────────

async function loadConfig() {
  try {
    const result = await chrome.storage.local.get(['api_base_url']);
    if (result['api_base_url']) {
      API_BASE = result['api_base_url'].replace(/\/+$/, '');
    }
  } catch (err) {
  }
}

// ── Helper: baca API Key Gemini dari storage (multi-line) ─────

const STORAGE_KEY = 'gemini_api_keys';

async function getApiKeysFromStorage() {
  try {
    const result = await chrome.storage.local.get([STORAGE_KEY]);
    const raw = result[STORAGE_KEY] || '';
    // Parse multi-line: 1 key per baris, trim, filter kosong, unik
    const keys = [...new Set(
      raw.split('\n')
        .map(k => k.trim())
        .filter(k => k.length > 0)
    )];
    return keys;
  } catch (err) {
    return [];
  }
}

// ── Helper: OAuth PKCE (login Google via launchWebAuthFlow) ────

function base64UrlEncode(bytes) {
  let bin = '';
  bytes.forEach(b => { bin += String.fromCharCode(b) });
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

async function pkcePair() {
  const verifierBytes = new Uint8Array(32);
  crypto.getRandomValues(verifierBytes);
  const verifier = base64UrlEncode(verifierBytes);
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier));
  const challenge = base64UrlEncode(new Uint8Array(digest));
  return { verifier, challenge };
}

function randomState() {
  const arr = new Uint8Array(16);
  crypto.getRandomValues(arr);
  return Array.from(arr, b => b.toString(16).padStart(2, '0')).join('');
}

// ── Konfigurasi auth & sesi extension ──────────────────────────

const AUTH_TOKEN_KEY = 'ext_token';
const AUTH_USER_KEY = 'ext_user';

async function getAuthConfig() {
  try {
    const r = await fetchWithTimeout(`${API_BASE}/api/auth/config`, { method: 'GET' });
    if (!r.ok) return null;
    return await r.json(); // { enabled, client_id, redirect_uri(s) }
  } catch (err) {
    return null;
  }
}

/// Header JSON + Bearer token bila user sedang login (opsional).
async function getAuthHeaders() {
  const headers = { 'Content-Type': 'application/json' };
  try {
    const stored = await chrome.storage.local.get([AUTH_TOKEN_KEY]);
    if (stored[AUTH_TOKEN_KEY]) {
      headers['Authorization'] = `Bearer ${stored[AUTH_TOKEN_KEY]}`;
    }
  } catch (err) {}
  return headers;
}

async function getLoginStatus() {
  try {
    const stored = await chrome.storage.local.get([AUTH_TOKEN_KEY, AUTH_USER_KEY]);
    return {
      loggedIn: !!stored[AUTH_TOKEN_KEY],
      user: stored[AUTH_USER_KEY] || null,
    };
  } catch (err) {
    return { loggedIn: false, user: null };
  }
}

// Login Google via chrome.identity.launchWebAuthFlow + PKCE.
// Code ditukar di backend /api/auth/google (client_secret tetap di server,
// extension tidak pernah memegang token Google — hanya JWT sesi backend).
async function loginGoogle() {
  const cfg = await getAuthConfig();
  if (!cfg || !cfg.enabled) {
    return { success: false, error: 'Auth Google nonaktif di server (GOOGLE_CLIENT_ID kosong).' };
  }
  const { verifier, challenge } = await pkcePair();
  const state = randomState();
  const redirectUri = `https://${chrome.runtime.id}.chromiumapp.org/`;

  const url = new URL('https://accounts.google.com/o/oauth2/v2/auth');
  url.searchParams.set('client_id', cfg.client_id);
  url.searchParams.set('redirect_uri', redirectUri);
  url.searchParams.set('response_type', 'code');
  url.searchParams.set('scope', 'openid email profile');
  url.searchParams.set('code_challenge', challenge);
  url.searchParams.set('code_challenge_method', 'S256');
  url.searchParams.set('state', state);

  try {
    const resultUrl = await chrome.identity.launchWebAuthFlow({ url: url.toString(), interactive: true });
    const ru = new URL(resultUrl);
    const code = ru.searchParams.get('code');
    const st = ru.searchParams.get('state');
    if (!code) {
      return { success: false, error: 'Login dibatalkan atau gagal (tidak ada authorization code).' };
    }
    if (st && st !== state) {
      return { success: false, error: 'State tidak cocok — silakan coba lagi.' };
    }
    const resp = await fetchWithTimeout(`${API_BASE}/api/auth/google`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ code, code_verifier: verifier, redirect_uri: redirectUri }),
    });
    const data = await resp.json().catch(() => null);
    if (!resp.ok) {
      return { success: false, error: data?.error || `Login gagal (HTTP ${resp.status}).` };
    }
    await chrome.storage.local.set({ [AUTH_TOKEN_KEY]: data.token, [AUTH_USER_KEY]: data.user });
    return { success: true, token: data.token, user: data.user };
  } catch (err) {
    // User menutup jendela login / flow dibatalkan
    return { success: false, error: `Login dibatalkan: ${err.message}` };
  }
}

async function logoutExtension() {
  await chrome.storage.local.remove([AUTH_TOKEN_KEY, AUTH_USER_KEY]);
  return { success: true };
}

// ── Helper: normalize error message ────────────────────────────

const KNOWN_ERRORS = [
  { pattern: /429|quota|rate.limit|exceeded|terlalu banyak|tunggu/i, msg: '⚠️ Kuota API terpakai habis / rate limit. Tunggu beberapa detik lalu coba lagi.' },
  { pattern: /403|unauthorized|forbidden/i, msg: '❌ Koneksi ke server ditolak. Periksa URL Server API di Pengaturan.' },
  { pattern: /Failed to fetch|NetworkError|ERR_CONNECTION_REFUSED|ERR_NAME_NOT_RESOLVED|connect/i, msg: '❌ Tidak dapat terhubung ke server. Pastikan API server menyala (backend kode-klasifikasi-chat di localhost:3100).' },
  { pattern: /502|503|504/, msg: '⚠️ Server sedang sibuk. Coba lagi nanti.' },
];

function normalizeError(errText, status) {
  if (!errText && status) return `Server error (${status})`;
  for (const known of KNOWN_ERRORS) {
    if (known.pattern.test(errText) || known.pattern.test(String(status))) {
      return known.msg;
    }
  }
  // Ambil 200 karakter pertama saja dari pesan mentah
  const clean = errText.replace(/[{}\"\\]/g, '').trim();
  return clean.length > 200 ? clean.slice(0, 200) + '...' : clean;
}

// ── Helper: fetch with timeout ────────────────────────────────

async function fetchWithTimeout(url, options = {}, timeoutMs = 60000) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { ...options, signal: controller.signal });
    clearTimeout(timeout);
    return response;
  } catch (err) {
    clearTimeout(timeout);
    throw err;
  }
}

// ── Mapping hasil /api/chat → bentuk hasil extension ──────────
// Backend baru mengembalikan { results[], perihal, perihal_inti?, explanation, ringkasan? } (sinkron).
// Extension mengirim include_ringkasan:true sehingga respons juga memuat ringkasan
// (isi ringkas) — versi web tidak mengirim, jadi tanpa ringkasan.
// Disusun ulang agar UI existing (modal & popup) bisa langsung memakai:
// perihal, isi_ringkas, explanation (penjelasan AI),
// kode_klasifikasi/klasifikasi_deskripsi (kandidat teratas) &
// sub_klasifikasi (daftar kandidat untuk pemilihan & feedback Setuju).

function mapChatResult(data) {
  const top = Array.isArray(data.results) ? data.results : [];
  // Hanya 3 kandidat teratas yang ditampilkan (permintaan user)
  const candidates = top.slice(0, 3).map(r => ({
    kode: r.kode || '',
    deskripsi: r.deskripsi || '',
    path: r.path || '',
    similarity: r.similarity || 0,
    // Metadata SKKAD (opsional — dari kolom baru klasifikasi_embedding)
    retensi_aktif: r.retensi_aktif ?? null,
    retensi_inaktif: r.retensi_inaktif ?? null,
    penyusutan_akhir: r.penyusutan_akhir || null,
    klasifikasi_keamanan: r.klasifikasi_keamanan || null,
  }));
  return {
    perihal: data.perihal || '',
    perihal_inti: data.perihal_inti || '', // perihal bersih (untuk embedding feedback)
    explanation: data.explanation || '',
    isi_ringkas: data.ringkasan || '', // ringkasan naskah (khusus extension, dari include_ringkasan)
    kode_klasifikasi: top[0]?.kode || '',
    klasifikasi_deskripsi: top[0]?.deskripsi || '',
    sub_klasifikasi: candidates, // daftar kandidat (untuk UI & feedback Setuju)
    kode_detil: candidates[0]?.kode || '',
    detil_deskripsi: candidates[0]?.deskripsi || '',
    raw_results: top,
  };
}

// ── DETEKSI INFORMASI DIKECUALIKAN (tanpa AI) ─────────────────
// Blok ini dipisahkan marker agar bisa diuji via `node test_deteksi.js`.
// Logika harus SAMA dengan backend/src/dikecualikan.rs. Aturan konservatif:
// satu NIK/KTP pemohon normal TIDAK diblokir; kata "rahasia"/"terbatas"
// lowercase biasa TIDAK dianggap label.

function normTeks(s) {
  return String(s || '').toUpperCase().replace(/\s+/g, ' ');
}

function countNIK(teks) {
  // Hitung run angka PERSIS 16 digit (pola NIK) — diselaraskan dengan
  // backend/src/dikecualikan.rs (run 17+ digit tidak dihitung).
  const s = String(teks || '');
  let count = 0;
  let run = 0;
  for (const ch of s) {
    if (ch >= '0' && ch <= '9') {
      run++;
    } else {
      if (run === 16) count++;
      run = 0;
    }
  }
  if (run === 16) count++;
  return count;
}

function deteksiInformasiDikecualikan(teks) {
  const alasan = [];
  const push = (r) => { if (!alasan.includes(r)) alasan.push(r); };
  const n = normTeks(teks);

  // Tier 1: label klasifikasi naskah dinas
  for (const p of ['SANGAT RAHASIA', 'RAHASIA NEGARA']) {
    if (n.includes(p)) push(`label '${p}'`);
  }
  // Pola kop naskah dinas: SIFAT/KLASIFIKASI … RAHASIA/TERBATAS.
  // "bersifat rahasia" = frasa klasifikasi asli → diblokir.
  // Guard negasi: "tidak/bukan bersifat rahasia" TIDAK dianggap label.
  for (const kw of ['SIFAT', 'KLASIFIKASI', 'KLASSIFIKASI']) {
    const i = n.indexOf(kw);
    if (i >= 0) {
      const after = n.slice(i + kw.length, i + kw.length + 60);
      const posRa = after.indexOf('RAHASIA');
      const posTe = after.indexOf('TERBATAS');
      const pos = posRa < 0 ? posTe : (posTe < 0 ? posRa : Math.min(posRa, posTe));
      if (pos >= 0) {
        // Cek ~20 karakter sebelum posisi label di teks PENUH (mencakup teks
        // sebelum kata kunci, mis. "tidak bersifat rahasia") untuk negasi.
        const labelPos = i + kw.length + pos;
        const before = n.slice(Math.max(0, labelPos - 20), labelPos).toUpperCase();
        if (!(before.includes('TIDAK') || before.includes('BUKAN'))) {
          push(`sifat/klasifikasi '${kw} …'`);
          break;
        }
      }
    }
  }
  // Stempel kapital berdiri sendiri (case-sensitive terhadap teks asli)
  const tokens = String(teks || '').split(/[^A-Za-z0-9]+/).filter(Boolean);
  if (tokens.includes('RAHASIA')) push("stempel 'RAHASIA'");
  if (tokens.includes('TERBATAS')) push("stempel 'TERBATAS'");
  for (const p of ['TOP SECRET', 'CONFIDENTIAL', 'RESTRICTED', 'FOR INTERNAL USE']) {
    if (n.includes(p)) push(`label '${p}'`);
  }

  // Tier 2: frasa eksplisit "informasi yang dikecualikan" & rahasia khusus
  if (n.includes('INFORMASI YANG DIKECUALIKAN')) push("frasa 'informasi yang dikecualikan'");
  for (const p of ['RAHASIA JABATAN', 'RAHASIA DINAS', 'RAHASIA DAGANG', 'RAHASIA PERUSAHAAN', 'RAHASIA BANK', 'RAHASIA MEDIS', 'RAHASIA KORESPONDENSI', 'RAHASIA KOMERSIAL']) {
    if (n.includes(p)) push(`frasa '${p.toLowerCase()}'`);
  }
  for (const p of ['DATA NASABAH', 'REKAM MEDIS', 'HANYA UNTUK INTERNAL', 'TIDAK UNTUK DIPUBLIKASIKAN']) {
    if (n.includes(p)) push(`frasa '${p.toLowerCase()}'`);
  }

  // Tier 3: istilah keamanan negara
  for (const p of ['INTELIJEN NEGARA', 'KEAMANAN NEGARA', 'PERTAHANAN NEGARA', 'OPERASI MILITER', 'MATA-MATA', 'PENYADAPAN', 'PERSENJATAAN', 'AMUNISI']) {
    if (n.includes(p)) push(`istilah '${p.toLowerCase()}'`);
  }

  // Tier 4: data pribadi massal
  const nik = countNIK(teks);
  if (nik >= 3) push(`daftar NIK massal (${nik} NIK)`);
  for (const kw of ['DAFTAR NIK', 'DAFTAR NPWP', 'DATA PENDUDUK']) {
    const i = n.indexOf(kw);
    if (i >= 0) {
      const after = n.slice(i + kw.length, i + kw.length + 100);
      if (after.includes('RAHASIA') || after.includes('TERBATAS')) {
        push(`'${kw.toLowerCase()}' dengan klasifikasi rahasia`);
        break;
      }
    }
  }

  return alasan;
}

// Ekstrak kandidat kode klasifikasi dari teks (pola segmen digit 1–3 dipisah
// titik, MINIMAL 2 segmen; batas kata non-digit di kedua sisi). Diselaraskan
// dengan backend/src/dikecualikan.rs (kode_kandidat). "10.03.2026" (tanggal)
// → kandidat maximal "10.03.202" diikuti digit → bukan kode utuh → aman.
function kodeKandidat(teks) {
  const s = String(teks || '');
  const out = [];
  let i = 0;
  const n = s.length;
  while (i < n) {
    const ch = s[i];
    if (ch < '0' || ch > '9') {
      i++;
      continue;
    }
    // Batas kiri: karakter sebelum digit awal bukan digit
    if (i > 0 && s[i - 1] >= '0' && s[i - 1] <= '9') {
      i++;
      continue;
    }
    // Segmen pertama: 1–3 digit
    let j = i;
    let segLen = 0;
    while (j < n && s[j] >= '0' && s[j] <= '9' && segLen < 3) {
      j++;
      segLen++;
    }
    let kode = s.slice(i, j);
    let segmen = 1;
    let k = j;
    // Segmen berikutnya: '.' + 1–3 digit
    while (k < n && s[k] === '.' && k + 1 < n && s[k + 1] >= '0' && s[k + 1] <= '9') {
      let m = k + 1;
      let slen = 0;
      while (m < n && s[m] >= '0' && s[m] <= '9' && slen < 3) {
        m++;
        slen++;
      }
      kode += '.' + s.slice(k + 1, m);
      segmen++;
      k = m;
    }
    // Batas kanan: karakter setelah kode bukan digit
    const boundary = k >= n || !(s[k] >= '0' && s[k] <= '9');
    if (segmen >= 2 && boundary) {
      out.push(kode);
    }
    i = k;
  }
  return out;
}

// Deteksi kode klasifikasi SENSITIF (per SKKAD: Rahasia/Sangat Rahasia/Terbatas)
// yang tertulis di dalam teks naskah. `kodeSet` = Set kode sensitif (dari
// endpoint /api/dikecualikan/kode-rahasia, di-cache). Logika identik dengan
// backend/src/dikecualikan.rs (deteksi_kode).
function deteksiKodeRahasia(teks, kodeSet) {
  const alasan = [];
  const seen = new Set();
  for (const kode of kodeKandidat(teks)) {
    if (kodeSet && kodeSet.has(kode) && !seen.has(kode)) {
      seen.add(kode);
      alasan.push(`kode klasifikasi '${kode}' berklasifikasi keamanan per SKKAD`);
    }
  }
  return alasan;
}
// ── DETEKSI SELESAI ───────────────────────────────────────────

// ── Daftar kode klasifikasi sensitif (per SKKAD) ───────────────
// Di-cache di chrome.storage.local agar guard lokal tidak bergantung server
// (fallback: bila daftar belum tersedia, lapisan kode nonaktif — aturan teks
// tetap jalan). Disinkronkan dari endpoint /api/dikecualikan/kode-rahasia.

const KODE_SENSITIF_STORAGE = 'srikandi_kode_sensitif';
const KODE_SENSITIF_TTL_MS = 24 * 60 * 60 * 1000; // refresh harian

async function readKodeSensitifCache() {
  try {
    const stored = await chrome.storage.local.get([KODE_SENSITIF_STORAGE]);
    const data = stored[KODE_SENSITIF_STORAGE];
    if (data && Array.isArray(data.kode) && data.kode.length > 0) {
      return data;
    }
  } catch (err) {}
  return null;
}

async function getKodeSensitif() {
  // 1. Cache valid (masih dalam TTL) → pakai langsung (tanpa request)
  const cached = await readKodeSensitifCache();
  if (cached && (!cached.ts || (Date.now() - cached.ts) < KODE_SENSITIF_TTL_MS)) {
    return new Set(cached.kode);
  }

  // 2. Cache kosong/kedaluwarsa → muat dari server, simpan cache baru
  try {
    const resp = await fetchWithTimeout(`${API_BASE}/api/dikecualikan/kode-rahasia`, {
      method: 'GET',
      headers: { 'Accept': 'application/json' },
    });
    if (resp.ok) {
      const body = await resp.json();
      if (body && Array.isArray(body.kode) && body.kode.length > 0) {
        await chrome.storage.local.set({
          [KODE_SENSITIF_STORAGE]: { kode: body.kode, ts: Date.now() },
        });
        return new Set(body.kode);
      }
    }
  } catch (err) {}

  // 3. Fallback: pakai cache lama (walau kedaluwarsa) kalau ada; else Set kosong
  if (cached) {
    return new Set(cached.kode);
  }
  return new Set();
}

// ── Analisa Teks ──────────────────────────────────────────────
// POST /api/chat — hasil langsung (tanpa task_id & polling).

async function startAnalysis(teks) {
  try {
    // 1. Deteksi deterministik (tanpa AI) indikasi informasi yang dikecualikan.
    //    Lapis 1: aturan teks (label, frasa, NIK massal).
    //    Lapis 2: kode klasifikasi sensitif per SKKAD yang tertulis di naskah
    //    (daftar di-cache dari backend; bila offline, lapisan ini nonaktif).
    //    Catatan: placeholder template SRIKANDI (${...}) sengaja TIDAK diblokir
    //    — naskah asli SRIKANDI memang memuat variabel template (mis. ${perihal}),
    //    dan teks tetap dikirim apa adanya (keputusan user 2026-08-09).
    const alasan = deteksiInformasiDikecualikan(teks);
    const kodeSensitif = await getKodeSensitif();
    alasan.push(...deteksiKodeRahasia(teks, kodeSensitif));
    if (alasan.length > 0) {
      return { error: 'Demi keamanan, analisa dibatalkan. Naskah ini terdeteksi mengandung informasi yang dikecualikan: ' + alasan.join('; ') + '. Jangan kirim naskah rahasia atau naskah berisi informasi yang dikecualikan (Pasal 17 UU No. 14/2008) ke layanan AI.' };
    }
    const keys = await getApiKeysFromStorage();
    const body = { message: teks, include_ringkasan: true };
    if (keys.length > 0) {
      body.api_keys = keys;
    }

    const response = await fetchWithTimeout(`${API_BASE}/api/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      // Baca body SEKALI lalu coba parse JSON (body tidak bisa dibaca dua kali)
      const raw = await response.text().catch(() => '');
      let err = null;
      try { err = JSON.parse(raw); } catch { err = null; }
      return { error: normalizeError(err?.error || raw, response.status) };
    }

    const data = await response.json();
    return { result: mapChatResult(data), text: teks };
  } catch (err) {
    return { error: `Gagal hubungi server: ${err.message}` };
  }
}

// ── Analisa File (PDF) ────────────────────────────────────────
// PDF → POST /api/extract-pdf (pdf-inspector) → teks → POST /api/chat.
// DOCX diekstrak client-side di content script (mammoth) → ANALISA_TEKS.

async function startAnalysisWithFile(fileName, fileBase64, fileExt) {
  try {
    if (fileExt !== 'pdf') {
      return { error: `Format ${fileExt} tidak didukung backend. Gunakan PDF (diekstrak server) atau DOCX (diekstrak di halaman).` };
    }

    // 1. Ekstrak teks PDF via backend
    const binary = atob(fileBase64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    const blob = new Blob([bytes], { type: 'application/pdf' });
    const fd = new FormData();
    fd.append('file', blob, fileName);

    const extractResp = await fetchWithTimeout(`${API_BASE}/api/extract-pdf`, {
      method: 'POST',
      body: fd,
    });

    if (!extractResp.ok) {
      // Baca body SEKALI lalu coba parse JSON (body tidak bisa dibaca dua kali)
      const raw = await extractResp.text().catch(() => '');
      let err = null;
      try { err = JSON.parse(raw); } catch { err = null; }
      return { error: normalizeError(err?.error || raw, extractResp.status) };
    }

    const extracted = await extractResp.json();
    const text = (extracted.text || '').trim();
    if (!text) {
      return { error: 'Tidak ada teks yang bisa diekstrak dari PDF.' };
    }

    // 2. Analisa teks → hasil klasifikasi
    return await startAnalysis(text);
  } catch (err) {
    return { error: `Gagal hubungi server: ${err.message}` };
  }
}

// ── Cari Kode (autocomplete koreksi) ──────────────────────────
// GET /api/codes?q= → [{ kode, deskripsi, path }]

async function searchSuggestions(query) {
  try {
    const q = (query || '').trim();
    if (q.length < 2) return [];
    const response = await fetchWithTimeout(
      `${API_BASE}/api/codes?q=${encodeURIComponent(q)}`,
      { method: 'GET', headers: { 'Accept': 'application/json' } }
    );
    if (!response.ok) return [];
    return await response.json();
  } catch (err) {
    return [];
  }
}

// ── Submit Feedback ────────────────────────────────────────────
// POST /api/feedback — positif (anonim, pakai chat_id) & koreksi
// (butuh login — Authorization: Bearer JWT dari sesi extension).

async function submitFeedbackProxy(payload) {
  try {
    const response = await fetchWithTimeout(`${API_BASE}/api/feedback`, {
      method: 'POST',
      headers: await getAuthHeaders(),
      body: JSON.stringify(payload),
    });
    const data = await response.json().catch(() => null);
    if (!response.ok) {
      // 401 = sesi kedaluwarsa/ditolak → bersihkan token agar UI kembali
      // menampilkan tombol login (hindari loop 401 tanpa jalan keluar).
      if (response.status === 401) {
        await chrome.storage.local.remove([AUTH_TOKEN_KEY, AUTH_USER_KEY]);
      }
      return { error: normalizeError(data?.error || `Server error (${response.status})`, response.status) };
    }
    return { success: true, data };
  } catch (err) {
    return { error: `Gagal kirim feedback: ${err.message}` };
  }
}

// ── URL Aplikasi Web (untuk login & koreksi) ──────────────────
// Heuristik: bila API di localhost, web app dev ada di port 5174;
// selain itu (VPS) web app & API share origin via nginx proxy.

function webAppUrl() {
  try {
    const u = new URL(API_BASE);
    if (u.hostname === 'localhost' || u.hostname === '127.0.0.1') {
      u.port = '5174';
      return u.toString();
    }
    return API_BASE;
  } catch (err) {
    return API_BASE;
  }
}

// ── Find SRIKANDI tab ─────────────────────────────────────────

async function findSrikandiTab() {
  const tabs = await chrome.tabs.query({
    url: ['https://srikandi.arsip.go.id/*'],
    currentWindow: true,
  });

  if (tabs.length === 0) {
    // Try without currentWindow restriction
    const allTabs = await chrome.tabs.query({
      url: ['https://srikandi.arsip.go.id/*'],
    });
    return allTabs[0] || null;
  }
  return tabs[0];
}

// ── Message Handler ────────────────────────────────────────────

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  // Pastikan konfigurasi termuat sebelum proses message
  const handleMessage = async () => {
    await loadConfig();

    switch (message.type) {
      case 'ANALISA_TEKS':
        return await startAnalysis(message.teks);

      case 'ANALISA_FILE':
        return await startAnalysisWithFile(message.fileName, message.fileData, message.fileExt);

      case 'ISI_FORM': {
        const tab = await findSrikandiTab();
        if (!tab) {
          return { error: 'Tab SRIKANDI tidak ditemukan' };
        }
        return new Promise((resolve) => {
          chrome.tabs.sendMessage(tab.id, { type: 'ISI_FORM', data: message.data }, (resp) => {
            if (chrome.runtime.lastError) {
              resolve({ error: chrome.runtime.lastError.message });
              return;
            }
            resolve(resp || { success: true });
          });
        });
      }

      case 'SEARCH_SUGGESTIONS':
        return await searchSuggestions(message.query);

      case 'LOGIN_GOOGLE':
        return await loginGoogle();

      case 'LOGOUT':
        return await logoutExtension();

      case 'GET_LOGIN_STATUS':
        return await getLoginStatus();

      case 'SUBMIT_FEEDBACK':
        return await submitFeedbackProxy(message.payload);

      case 'OPEN_WEB_APP': {
        await chrome.tabs.create({ url: webAppUrl() });
        return { success: true };
      }

      default:
        return { error: 'Unknown message type: ' + message.type };
    }
  };

  handleMessage().then(sendResponse);
  return true;
});

// ── Extension installed ────────────────────────────────────────

chrome.runtime.onInstalled.addListener(async () => {
  await loadConfig();
});

// Load config on startup
loadConfig();
