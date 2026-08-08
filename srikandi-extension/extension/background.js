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
// Backend baru mengembalikan { results[], perihal, explanation, ringkasan? } (sinkron).
// Extension mengirim include_ringkasan:true sehingga respons juga memuat ringkasan
// (isi ringkas) — versi web tidak mengirim, jadi tanpa ringkasan.
// Disusun ulang agar UI existing (modal & popup) bisa langsung memakai:
// perihal, isi_ringkas, explanation (penjelasan AI),
// kode_klasifikasi/klasifikasi_deskripsi (kandidat teratas) &
// sub_klasifikasi (daftar kandidat untuk pemilihan & feedback Setuju).

function mapChatResult(data) {
  const top = Array.isArray(data.results) ? data.results : [];
  const candidates = top.slice(0, 5).map(r => ({
    kode: r.kode || '',
    deskripsi: r.deskripsi || '',
    path: r.path || '',
    similarity: r.similarity || 0,
  }));
  return {
    perihal: data.perihal || '',
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

// ── Analisa Teks ──────────────────────────────────────────────
// POST /api/chat — hasil langsung (tanpa task_id & polling).

async function startAnalysis(teks) {
  try {
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
// PDF → POST /api/extract-pdf (poppler) → teks → POST /api/chat.
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
