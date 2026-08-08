// ─── Popup Script: Analisa Naskah SRIKANDI ────────────────────
// Popup = panel status/pengaturan + riwayat feedback.
// Hasil analisa naskah TIDAK ditampilkan di popup — tampil di modal
// pada halaman SRIKANDI (content script). Login & koreksi diarahkan
// ke aplikasi web (backend kode-klasifikasi-chat mewajibkan login utk koreksi).

(() => {
  'use strict';

  let API_BASE = 'http://localhost:3100';

  // ── DOM refs ─────────────────────────────────────────────────
  const $ = (id) => document.getElementById(id);
  const btnSettings = $('btn-settings');
  const settingsPanel = $('settings-panel');
  const inputApiKeys = $('input-api-keys');
  const btnToggleKey = $('btn-toggle-key-visibility');
  const btnSaveKey = $('btn-save-key');
  const btnClearKey = $('btn-clear-key');
  const apiKeyStatus = $('api-key-status');
  const inputApiUrl = $('input-api-url');
  const btnSaveUrl = $('btn-save-url');
  const btnResetUrl = $('btn-reset-url');
  const apiUrlStatus = $('api-url-status');
  const btnBukaWebApp = $('btn-buka-web-app');
  const btnBukaSrikandi = $('btn-buka-srikandi');
  const btnOpenSrikandi = $('btn-open-srikandi');
  const btnOpenWeb = $('btn-open-web');
  const loginUserInfo = $('login-user-info');
  const loginNotLoggedIn = $('login-not-logged-in');
  const loginUserName = $('login-user-name');
  const loginUserEmail = $('login-user-email');
  const btnLoginGoogle = $('btn-login-google');
  const btnLogoutGoogle = $('btn-logout-google');

  // ── API Key Management ───────────────────────────────────────

  const STORAGE_KEY = 'gemini_api_keys'; // sama dengan key di web app

  async function loadApiKeys() {
    try {
      const result = await chrome.storage.local.get([STORAGE_KEY]);
      const raw = result[STORAGE_KEY] || '';
      inputApiKeys.value = raw;
      updateApiKeyStatus(raw);
    } catch (err) {
    }
  }

  async function saveApiKeys(raw) {
    try {
      const trimmed = raw.trim();
      await chrome.storage.local.set({ [STORAGE_KEY]: trimmed });
      updateApiKeyStatus(trimmed);
    } catch (err) {
      apiKeyStatus.textContent = '❌ Gagal simpan';
    }
  }

  async function clearApiKeys() {
    try {
      await chrome.storage.local.remove(STORAGE_KEY);
      inputApiKeys.value = '';
      updateApiKeyStatus('');
    } catch (err) {
    }
  }

  function updateApiKeyStatus(raw) {
    const keys = (raw || '').split('\n').map(k => k.trim()).filter(k => k.length > 0);
    if (keys.length > 0) {
      apiKeyStatus.textContent = `✅ ${keys.length} key tersimpan`;
      apiKeyStatus.style.color = '#059669';
    } else {
      apiKeyStatus.textContent = '⏺ Belum diset (pakai key server default)';
      apiKeyStatus.style.color = '#888';
    }
  }

  // ── API URL Management ───────────────────────────────────────

  const API_URL_STORAGE_KEY = 'api_base_url';
  const DEFAULT_API_URL = 'http://localhost:3100';

  async function loadApiUrl() {
    try {
      const result = await chrome.storage.local.get([API_URL_STORAGE_KEY]);
      const url = result[API_URL_STORAGE_KEY] || DEFAULT_API_URL;
      API_BASE = url.replace(/\/+$/, '');
      inputApiUrl.value = url;
      updateApiUrlStatus(url);
      return url;
    } catch (err) {
      inputApiUrl.value = DEFAULT_API_URL;
      API_BASE = DEFAULT_API_URL;
      return DEFAULT_API_URL;
    }
  }

  async function saveApiUrl(url) {
    try {
      const trimmed = url.trim().replace(/\/+$/, ''); // hapus trailing slash
      API_BASE = trimmed; // update global
      await chrome.storage.local.set({ [API_URL_STORAGE_KEY]: trimmed });
      updateApiUrlStatus(trimmed);
    } catch (err) {
      apiUrlStatus.textContent = '❌ Gagal simpan';
      apiUrlStatus.style.color = '#dc2626';
    }
  }

  async function resetApiUrl() {
    try {
      await chrome.storage.local.remove(API_URL_STORAGE_KEY);
      inputApiUrl.value = DEFAULT_API_URL;
      updateApiUrlStatus(DEFAULT_API_URL);
    } catch (err) {
    }
  }

  function updateApiUrlStatus(url) {
    if (url && url !== DEFAULT_API_URL) {
      apiUrlStatus.textContent = `✅ ${url}`;
      apiUrlStatus.style.color = '#059669';
    } else {
      apiUrlStatus.textContent = `⏺ Default (${DEFAULT_API_URL})`;
      apiUrlStatus.style.color = '#888';
    }
  }

  // ── Login Google (via background: launchWebAuthFlow + PKCE) ────

  function updateLoginUI(user) {
    if (user) {
      loginUserInfo.style.display = 'flex';
      loginNotLoggedIn.style.display = 'none';
      loginUserName.textContent = user.name || '';
      loginUserEmail.textContent = user.email || '';
    } else {
      loginUserInfo.style.display = 'none';
      loginNotLoggedIn.style.display = 'block';
    }
  }

  async function checkLoginStatus() {
    try {
      const stored = await chrome.storage.local.get(['ext_token', 'ext_user']);
      updateLoginUI(stored['ext_token'] ? (stored['ext_user'] || null) : null);
    } catch (err) {
      updateLoginUI(null);
    }
  }

  btnLoginGoogle.addEventListener('click', async () => {
    btnLoginGoogle.disabled = true;
    btnLoginGoogle.textContent = '⏳ Memproses...';
    try {
      const res = await chrome.runtime.sendMessage({ type: 'LOGIN_GOOGLE' });
      if (res && res.success) {
        updateLoginUI(res.user);
      } else {
        alert('❌ Login gagal: ' + ((res && res.error) || 'Gagal terhubung ke server'));
      }
    } catch (err) {
      alert('❌ Login gagal: ' + err.message);
    } finally {
      btnLoginGoogle.disabled = false;
      btnLoginGoogle.textContent = '🔑 Login dengan Google';
    }
  });

  btnLogoutGoogle.addEventListener('click', async () => {
    try {
      await chrome.runtime.sendMessage({ type: 'LOGOUT' });
    } catch (err) {}
    updateLoginUI(null);
  });

  // ── URL Aplikasi Web (login & koreksi) ───────────────────────
  // Heuristik: bila API di localhost, web app dev ada di :5174;
  // selain itu (VPS) web app & API share origin via nginx proxy.

  function getWebAppUrl() {
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

  // ── Feedback Riwayat (dari /api/feedback/stats) ────────────────

  function getCategoryLabel(entry) {
    if (entry.feedback_type === 'correction') {
      if (entry.status === 'validated') return { label: 'REVISI DITERIMA', cls: 'diterima' };
      if (entry.status === 'rejected') return { label: 'DITOLAK', cls: '' };
      return { label: 'PENDING', cls: '' };
    }
    return { label: 'SETUJU', cls: 'setuju' };
  }

  function formatDate(ts) {
    if (!ts) return '';
    try {
      const d = new Date(ts);
      return d.toLocaleDateString('id-ID', {
        day: 'numeric', month: 'short', year: 'numeric',
        hour: '2-digit', minute: '2-digit',
      });
    } catch { return ts; }
  }

  function escapeHtml(text) {
    if (!text) return '';
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
  }

  async function fetchFeedback() {
    const container = $('fb-list-container');
    container.innerHTML = '<div class="fb-list-placeholder">⏳ Memuat data...</div>';

    try {
      const resp = await fetch(`${API_BASE}/api/feedback/stats`);
      const data = await resp.json();

      const recent = (data && Array.isArray(data.recent)) ? data.recent : [];
      if (recent.length === 0) {
        container.innerHTML = '<div class="fb-list-empty">📭 Belum ada feedback</div>';
        return;
      }

      renderFeedbackList(recent, container);
    } catch (err) {
      container.innerHTML = `<div class="fb-list-error">❌ Gagal memuat: ${escapeHtml(err.message)}</div>`;
    }
  }

  function renderFeedbackList(entries, container) {
    let html = '';
    for (const entry of entries) {
      const cat = getCategoryLabel(entry);
      const date = formatDate(entry.waktu);

      html += '<div class="fb-list-item">';
      html += '  <div class="fb-item-header">';
      html += `    <span class="fb-item-type ${cat.cls}">${cat.label}</span>`;
      html += `    <span class="fb-item-date">${escapeHtml(date)}</span>`;
      html += '  </div>';

      if (entry.perihal) {
        html += `  <div class="fb-item-perihal">${escapeHtml(entry.perihal)}</div>`;
      }

      html += '  <div class="fb-item-detail">';
      if (entry.user_name) {
        html += `👤 ${escapeHtml(entry.user_name)}<br>`;
      }
      html += `📄 Kode AI: <strong>${escapeHtml(entry.kode_ai || '-')}</strong>`;
      if (entry.feedback_type === 'correction') {
        html += ` → <span class="fb-kode-baru">${escapeHtml(entry.kode_koreksi || '-')}</span>`;
      }
      if (entry.penjelasan) {
        html += `<br>📝 ${escapeHtml(entry.penjelasan)}`;
      }
      html += '  </div>';
      html += '</div>';
    }

    container.innerHTML = html;
  }

  // ── Tab switching ─────────────────────────────────────────────

  function setupTabSwitching() {
    document.querySelectorAll('.settings-tab').forEach(tab => {
      tab.addEventListener('click', () => {
        document.querySelectorAll('.settings-tab').forEach(t => t.classList.remove('active'));
        document.querySelectorAll('.tab-content').forEach(t => t.style.display = 'none');

        tab.classList.add('active');
        const targetId = tab.dataset.tab;
        const target = $(targetId);
        if (target) {
          target.style.display = 'block';
        }

        if (targetId === 'feedback-tab-content') {
          fetchFeedback();
        }
      });
    });
  }

  // ── Event Listeners ──────────────────────────────────────────

  // Settings toggle
  btnSettings.addEventListener('click', () => {
    const isOpen = settingsPanel.style.display !== 'none';
    settingsPanel.style.display = isOpen ? 'none' : 'block';
  });

  // Toggle API key visibility (gunakan CSS untuk mask/unmask textarea)
  let keysVisible = false;
  btnToggleKey.addEventListener('click', () => {
    keysVisible = !keysVisible;
    inputApiKeys.style.WebkitTextSecurity = keysVisible ? 'none' : 'disc';
    btnToggleKey.textContent = keysVisible ? '🙈' : '👁️';
  });

  // Save API keys
  btnSaveKey.addEventListener('click', () => {
    saveApiKeys(inputApiKeys.value);
  });

  // Clear API keys
  btnClearKey.addEventListener('click', clearApiKeys);

  // ── API URL event listeners ──

  // Save API URL
  btnSaveUrl.addEventListener('click', () => {
    saveApiUrl(inputApiUrl.value);
  });

  // Reset API URL
  btnResetUrl.addEventListener('click', resetApiUrl);

  // Enter to save URL
  inputApiUrl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      saveApiUrl(inputApiUrl.value);
    }
  });

  // Auto-save on blur
  inputApiUrl.addEventListener('blur', () => {
    saveApiUrl(inputApiUrl.value);
  });

  // ── Buka aplikasi web (login & koreksi) ──
  btnBukaWebApp.addEventListener('click', () => {
    chrome.tabs.create({ url: getWebAppUrl() });
  });

  // Tombol di footer / body (bila ada)
  if (btnOpenWeb) {
    btnOpenWeb.addEventListener('click', () => {
      chrome.tabs.create({ url: getWebAppUrl() });
    });
  }
  if (btnOpenSrikandi) {
    btnOpenSrikandi.addEventListener('click', () => {
      chrome.tabs.create({ url: 'https://srikandi.arsip.go.id/pembuatan-naskah-keluar/registrasi-naskah-keluar' });
    });
  }
  if (btnBukaSrikandi) {
    btnBukaSrikandi.addEventListener('click', () => {
      chrome.tabs.create({ url: 'https://srikandi.arsip.go.id/pembuatan-naskah-keluar/registrasi-naskah-keluar' });
    });
  }

  // ── Keyboard shortcuts ──

  // Ctrl+Enter / Cmd+Enter to save from textarea
  inputApiKeys.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
      e.preventDefault();
      saveApiKeys(inputApiKeys.value);
    }
  });

  // Auto-save on blur (leave textarea)
  inputApiKeys.addEventListener('blur', () => {
    saveApiKeys(inputApiKeys.value);
  });

  // ── Init ─────────────────────────────────────────────────────

  async function init() {
    await loadApiKeys();
    await loadApiUrl();
    await checkLoginStatus();
    setupTabSwitching();
  }

  init();
})();
