// ─── Content Script: Analisa Naskah SRIKANDI ─────────────────────
// Inject tombol "Analisa dengan AI" pada halaman registrasi naskah,
// baca file yang di-upload, analisa via backend kode-klasifikasi-chat
// (POST /api/chat), tampilkan hasil dalam modal overlay, dan isi form otomatis.

(() => {
  'use strict';

  // ── Config ────────────────────────────────────────────────────
  const STORAGE_KEYS = {
    API_URL: 'api_base_url',
  };
  const DEFAULT_API_URL = 'http://localhost:3100';
  let API_BASE = DEFAULT_API_URL;
  const KLASIFIKASI_SELECTOR = 'input[role="combobox"][aria-autocomplete="list"]';

  // ── chat_id (sesi anonim per browser) ────────────────────────
  // Dipakai backend untuk mengaitkan feedback anonim ke sesi chat
  // (kolom chat_id di klasifikasi_feedback). Dibuat sekali & disimpan.
  const CHAT_ID_STORAGE = 'srikandi_chat_id';

  function makeChatId() {
    if (typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function') {
      const h = (n) => Array.from(crypto.getRandomValues(new Uint8Array(n)), b => b.toString(16).padStart(2, '0')).join('');
      return `${h(8)}-${h(4)}-4${h(3)}-${['8', '9', 'a', 'b'][Math.floor(Math.random() * 4)]}${h(3)}-${h(12)}`;
    }
    return `anon-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
  }

  let cachedChatId = '';
  chrome.storage.local.get([CHAT_ID_STORAGE]).then((r) => {
    cachedChatId = r[CHAT_ID_STORAGE] || makeChatId();
    if (!r[CHAT_ID_STORAGE]) {
      chrome.storage.local.set({ [CHAT_ID_STORAGE]: cachedChatId });
    }
  }).catch(() => {
    cachedChatId = makeChatId();
  });

  function getChatId() {
    return cachedChatId || makeChatId();
  }

  // Load API URL dari storage (async, fallback ke default)
  async function loadApiUrl() {
    try {
      const result = await chrome.storage.local.get([STORAGE_KEYS.API_URL]);
      if (result[STORAGE_KEYS.API_URL]) {
        API_BASE = result[STORAGE_KEYS.API_URL].replace(/\/+$/, '');
      }
    } catch (err) {
    }
  }

  // ── State ─────────────────────────────────────────────────────
  let analysisState = {
    status: 'idle', // idle | loading | done | error
    result: null,
    error: null,
    text: '', // teks naskah yang dianalisa (dipakai untuk feedback positif)
  };

  // ── DOM helpers ───────────────────────────────────────────────

  function isOnRegistrasiPage() {
    return window.location.pathname.includes('registrasi-naskah');
  }

  function getFileInput() {
    // File input untuk upload DOCX/PDF (react-dropzone) di SRIKANDI.
    // Beberapa varian accept ditemukan: wordprocessingml (DOCX), openxmlformats,
    // "docx" (snapshot lama), dan "application/pdf" (PDF murni). Selector
    // mencakup SEMUA varian agar DOCX maupun PDF selalu terdeteksi.
    return document.querySelector(
      'input[type="file"][accept*="wordprocessingml"],' +
      'input[type="file"][accept*="openxmlformats"],' +
      'input[type="file"][accept*="docx"],' +
      'input[type="file"][accept*="pdf"]'
    );
  }

  function getHalTextarea() {
    return document.querySelector('textarea[name="hal"]');
  }

  function getRingkasanTextarea() {
    return document.querySelector('textarea[name="ringkasan"]');
  }

  function getKlasifikasiInput() {
    // Cari react-select untuk field Klasifikasi (bukan Dikirimkan melalui)
    const allCombos = document.querySelectorAll('input[role="combobox"][aria-autocomplete="list"]');

    // 1. Cari berdasarkan label "Klasifikasi" di parent container
    for (const combo of allCombos) {
      let el = combo.parentElement;
      for (let i = 0; i < 8 && el; i++) {
        const text = el.textContent || '';
        if (text.includes('Klasifikasi') && !text.includes('Dikirimkan')) {
          return combo;
        }
        el = el.parentElement;
      }
    }

    // 2. Cari berdasarkan placeholder text (bukan "Dikirimkan")
    for (const combo of allCombos) {
      const placeholderId = combo.getAttribute('aria-describedby');
      if (placeholderId) {
        const placeholder = document.getElementById(placeholderId);
        if (placeholder && !placeholder.textContent.includes('Dikirimkan')) {
          return combo;
        }
      }
    }

    // 3. Fallback: cari dari label/teks "Klasifikasi" di halaman -> cari combobox sibling
    const labels = document.querySelectorAll('label, span, strong, div');
    for (const label of labels) {
      const txt = (label.textContent || '').trim();
      if (txt === 'Klasifikasi' || txt.startsWith('Klasifikasi')) {
        const container = label.closest('[class*="Mui"]') || label.parentElement;
        if (container) {
          const combo = container.querySelector('input[role="combobox"]');
          if (combo) {
            return combo;
          }
        }
      }
    }

    // 4. Last resort: ambil yang pertama
    return allCombos[0];
  }

  // ── Inject Button ─────────────────────────────────────────────

  function injectAnalisaButton() {
    if (document.getElementById('srikandi-ai-analisa-btn')) return;

    const btn = document.createElement('button');
    btn.id = 'srikandi-ai-analisa-btn';
    btn.type = 'button'; // PREVENT form submission!
    btn.innerHTML = `
      <span class="srikandi-ai-icon">🔍</span>
      Analisa dengan AI
    `;
    btn.className = 'srikandi-ai-btn';
    btn.addEventListener('click', handleAnalisaClick);

    let placed = false;

    // Strategy 1: Cari form card/container yang berisi file upload
    // SRIKANDI pakai MUI — cari container dengan class mengandung "file" atau "upload"
    const fileInput = getFileInput();
    if (!placed && fileInput) {
      // Cari container terdekat (MUI FormControl / div pembungkus)
      const formControl = fileInput.closest('[class*="Mui"]') ||
                          fileInput.closest('div[class]');
      if (formControl && formControl.parentElement) {
        // Inject setelah form control file upload
        formControl.parentElement.insertBefore(btn, formControl.nextSibling);
        placed = true;
      }
    }

    // Strategy 2: Cari elemen input dengan id="file" (MUI text field yg show filename)
    if (!placed) {
      const fileTextField = document.querySelector('input[id="file"][readonly]');
      if (fileTextField) {
        const muiContainer = fileTextField.closest('[class*="Mui"]') ||
                             fileTextField.closest('.MuiFormControl-root') ||
                             fileTextField.closest('[class*="form"]') ||
                             fileTextField.parentElement;
        if (muiContainer && muiContainer.parentElement) {
          muiContainer.parentElement.insertBefore(btn, muiContainer.nextSibling);
          placed = true;
        }
      }
    }

    // Strategy 3: Inject di atas tombol submit (jika ada)
    if (!placed) {
      const submitBtn = document.querySelector('button[type="submit"]');
      if (submitBtn && submitBtn.parentElement) {
        submitBtn.parentElement.insertBefore(btn, submitBtn);
        placed = true;
      }
    }

    // Strategy 4 (last resort): Inject setelah Hal field
    if (!placed) {
      const halField = getHalTextarea();
      if (halField && halField.parentElement) {
        halField.parentElement.insertBefore(btn, halField.nextSibling);
        placed = true;
      }
    }

    // Strategy 5 (ultra fallback): Append ke body
    if (!placed) {
      btn.style.position = 'fixed';
      btn.style.bottom = '20px';
      btn.style.right = '20px';
      btn.style.zIndex = '99999';
      document.body.appendChild(btn);
    }

    // Peringatan keamanan — tampil tepat di bawah tombol agar terlihat SEBELUM
    // analisa: naskah berlabel RAHASIA atau berisi informasi yang dikecualikan
    // (istilah UU No. 14/2008) tidak boleh dikirim ke layanan AI.
    const warn = document.createElement('div');
    warn.id = 'srikandi-ai-warning';
    warn.className = 'srikandi-ai-warning';
    warn.textContent =
      '⚠️ JANGAN UPLOAD NASKAH RAHASIA ATAU NASKAH BERISI INFORMASI YANG DIKECUALIKAN';
    if (btn.style.position === 'fixed') {
      // Fallback mode (tombol melayang): letakkan peringatan di bawah tombol
      warn.style.position = 'fixed';
      warn.style.bottom = '70px';
      warn.style.right = '20px';
      warn.style.maxWidth = '300px';
      warn.style.zIndex = '99999';
    }
    btn.after(warn);
  }

  // ── Read Uploaded File ───────────────────────────────────────

  async function readUploadedFile() {
    const fileInput = getFileInput();
    if (!fileInput || !fileInput.files || fileInput.files.length === 0) {
      return { found: false };
    }

    const file = fileInput.files[0];
    const ext = file.name.split('.').pop().toLowerCase();
    if (ext !== 'docx' && ext !== 'pdf') {
      return { found: false, error: `Format ${ext} tidak didukung. Gunakan DOCX atau PDF.` };
    }

    // Read file as ArrayBuffer (untuk DOCX client-side) + base64 (untuk PDF → backend)
    const buffer = await file.arrayBuffer();
    const bytes = new Uint8Array(buffer);
    let binary = '';
    for (let i = 0; i < bytes.length; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    const base64 = btoa(binary);

    return { found: true, name: file.name, data: base64, ext, buffer };
  }

  // ── Inject Modal Overlay: Hasil + Feedback ──────────────────────

  let modalFeedbackState = {
    mode: null,       // 'setuju' | 'koreksi' | null
    selectedSub: null,
    feedbackStatus: null, // 'sending' | 'sent' | 'rejected' | null
    feedbackError: null,
  };

  // Badge metadata SKKAD per kandidat: 🔒 klasifikasi keamanan + 🗓️ retensi + ♻️ penyusutan
  function buildSkadMetaHtml(item) {
    const parts = [];
    if (item.klasifikasi_keamanan && item.klasifikasi_keamanan !== '-') {
      const cls = item.klasifikasi_keamanan === 'Sangat Rahasia'
        ? 'sk-meta-badge sk-meta-red'
        : item.klasifikasi_keamanan === 'Rahasia'
          ? 'sk-meta-badge sk-meta-orange'
          : item.klasifikasi_keamanan === 'Terbatas'
            ? 'sk-meta-badge sk-meta-amber'
            : 'sk-meta-badge sk-meta-green';
      parts.push(`<span class="${cls}">🔒 ${escapeHtml(item.klasifikasi_keamanan)}</span>`);
    }
    if (item.retensi_aktif != null || item.retensi_inaktif != null) {
      parts.push(`<span class="sk-meta-badge sk-meta-gray">🗓️ Aktif ${item.retensi_aktif ?? '–'} th · Inaktif ${item.retensi_inaktif ?? '–'} th</span>`);
    }
    if (item.penyusutan_akhir && item.penyusutan_akhir !== '-') {
      parts.push(`<span class="sk-meta-badge sk-meta-gray">♻️ ${escapeHtml(item.penyusutan_akhir)}</span>`);
    }
    return parts.length > 0 ? `<div class="sk-meta-row">${parts.join('')}</div>` : '';
  }

  function buildSubKlasifikasiHtml(subItems, selectedKode) {
    if (!subItems || subItems.length === 0) return '';
    // Max 3 sub-klasifikasi teratas
    const limited = subItems.slice(0, 3);
    const selected = selectedKode || (limited[0]?.kode);
    return limited.map((item, idx) => {
      const isSelected = item.kode === selected;
      return `
        <label class="sk-item ${isSelected ? 'sk-item-selected' : ''}"
               data-kode="${escapeHtml(item.kode)}"
               data-deskripsi="${escapeHtml(item.deskripsi)}">
          <input type="radio" name="sk-select" value="${escapeHtml(item.kode)}"
                 data-deskripsi="${escapeHtml(item.deskripsi)}"
                 ${isSelected ? 'checked' : ''} />
          <span class="sk-kode">${escapeHtml(item.kode)}</span>
          <span class="sk-desc">${escapeHtml(item.deskripsi)}</span>
          ${item.path ? `<span class="sk-path">${escapeHtml(item.path)}</span>` : ''}
          ${buildSkadMetaHtml(item)}
        </label>
      `;
    }).join('');
  }

  function showResultModal(result) {
    removeExistingModal();

    // Reset feedback state
    modalFeedbackState = {
      mode: null,
      selectedSub: result.sub_klasifikasi?.[0] || null,
      feedbackStatus: null,
      feedbackError: null,
    };

    const subItems = result.sub_klasifikasi || [];
    const subHtml = buildSubKlasifikasiHtml(subItems, result.kode_detil);

    // Build dropdown options for Setuju mode
    const setujuOptions = subItems.map((item, idx) =>
      `<option value="${escapeHtml(item.kode)}" data-deskripsi="${escapeHtml(item.deskripsi)}" data-path="${escapeHtml(item.path || '')}" ${idx === 0 ? 'selected' : ''}>
        ${escapeHtml(item.kode)} &mdash; ${escapeHtml(item.deskripsi)}${item.path ? ` (${escapeHtml(item.path)})` : ''}
      </option>`
    ).join('');

    const overlay = document.createElement('div');
    overlay.id = 'srikandi-ai-modal-overlay';
    overlay.className = 'srikandi-ai-overlay';

    const modal = document.createElement('div');
    modal.className = 'srikandi-ai-modal';
    modal.id = 'srikandi-ai-result-modal';
    modal.innerHTML = `
      <div class="srikandi-ai-modal-header">
        <h3>✅ Hasil Analisa Naskah</h3>
        <button class="srikandi-ai-close" id="srikandi-ai-close-modal">&times;</button>
      </div>
      <div class="srikandi-ai-modal-body">
        <!-- Perihal -->
        <div class="sk-field">
          <label class="sk-field-label">Perihal:</label>
          <div class="sk-field-value">${escapeHtml(result.perihal || '(kosong)')}</div>
        </div>
        <!-- Isi Ringkas (dari ringkasan backend — khusus extension) -->
        <div class="sk-field">
          <label class="sk-field-label">Isi Ringkas:</label>
          <div class="sk-field-value sk-field-value-ml">${escapeHtml(result.isi_ringkas || '(kosong)')}</div>
        </div>
        <!-- Penjelasan AI -->
        <div class="sk-field">
          <label class="sk-field-label">Penjelasan AI:</label>
          <div class="sk-field-value sk-field-value-ml">${escapeHtml(result.explanation || '(kosong)')}</div>
        </div>
        <!-- Kode Fungsi/Urusan (hasil utama) + metadata SKKAD -->
        <div class="sk-field">
          <label class="sk-field-label">Kode Fungsi/Urusan:</label>
          <div class="sk-field-value sk-field-code">
            <span class="sk-badge-fungsi">${escapeHtml(result.kode_klasifikasi)}</span>
            ${result.klasifikasi_deskripsi ? escapeHtml(result.klasifikasi_deskripsi) : ''}
          </div>
          ${buildSkadMetaHtml(result.sub_klasifikasi?.[0] || result)}
        </div>
        <!-- Kode Subklasifikasi -->
        <div class="sk-field">
          <label class="sk-field-label">Kode Subklasifikasi:</label>
          ${subHtml ? `
          <div class="sk-numlist" id="sk-numlist">
            ${subItems.map((item, idx) => `
              <div class="sk-numitem ${item.kode === result.kode_detil ? 'sk-numitem-active' : ''}"
                   data-kode="${escapeHtml(item.kode)}"
                   data-deskripsi="${escapeHtml(item.deskripsi)}">
                <span class="sk-num">${idx + 1}.</span>
                <span class="sk-numkode">${escapeHtml(item.kode)}</span>
                <span class="sk-numdesc">${escapeHtml(item.deskripsi)}</span>
                ${item.path ? `<span class="sk-numpath">${escapeHtml(item.path)}</span>` : ''}
                ${buildSkadMetaHtml(item)}
              </div>
            `).join('')}
          </div>
          ` : `
          <div class="sk-field-value sk-field-code">
            <span class="sk-badge-fungsi">${escapeHtml(result.kode_detil || '')}</span>
            ${escapeHtml(result.detil_deskripsi || '')}
          </div>
          `}
        </div>
        <!-- Feedback -->
        <div class="sk-field">
          <label class="sk-field-label">Feedback:</label>
          <div class="sk-feedback-box">
            <!-- Radio: Setuju / Koreksi -->
            <div class="sk-fb-radio-group">
              <label class="sk-fb-radio ${subItems.length > 0 ? '' : 'sk-fb-disabled'}">
                <input type="radio" name="sk-fb-mode" value="setuju" ${subItems.length === 0 ? 'disabled' : ''}>
                <span class="sk-fb-radio-label">👍 Setuju</span>
              </label>
              <label class="sk-fb-radio">
                <input type="radio" name="sk-fb-mode" value="koreksi">
                <span class="sk-fb-radio-label">✏️ Koreksi</span>
              </label>
            </div>
            <!-- Panel: Setuju — pilih sub -->
            <div id="sk-fb-setuju-panel" class="sk-fb-panel" style="display:none">
              <select id="sk-fb-setuju-select" class="sk-fb-select">
                ${setujuOptions}
              </select>
            </div>
            <!-- Panel: Koreksi (butuh login Google) -->
            <div id="sk-fb-koreksi-panel" class="sk-fb-panel" style="display:none">
              <!-- State 1: belum login -->
              <div id="sk-fb-koreksi-login" class="sk-fb-koreksi-login" style="display:none">
                <p class="sk-fb-koreksi-info">🔐 Koreksi memerlukan login Google. Klik tombol di bawah, lalu setujui akses email & profil.</p>
                <button type="button" class="srikandi-ai-btn srikandi-ai-btn-primary" id="sk-btn-login-google">🔑 Login dengan Google</button>
              </div>
              <!-- State 2: sudah login → form koreksi -->
              <div id="sk-fb-koreksi-form" class="sk-fb-koreksi-form" style="display:none">
                <div class="sk-ac-wrap">
                  <input type="text" id="sk-fb-cari-kode" class="sk-fb-input" placeholder="Ketik kode atau deskripsi..." autocomplete="off" />
                  <div id="sk-fb-ac-dropdown" class="sk-ac-dropdown" style="display:none"></div>
                </div>
                <div id="sk-fb-selected-label" class="sk-fb-selected" style="display:none">
                  <span class="sk-fb-selected-badge">Terpilih:</span>
                  <span id="sk-fb-selected-kode" class="sk-fb-selected-kode"></span>
                  <span id="sk-fb-selected-desc" class="sk-fb-selected-desc"></span>
                </div>
                <textarea id="sk-fb-alasan" class="sk-fb-input sk-fb-input-alasan" rows="2" placeholder="Alasan koreksi (opsional)"></textarea>
              </div>
            </div>
            <!-- User identity (nama SRIKANDI yang akan tercatat) -->
            <div id="sk-fb-user-area" class="sk-fb-user" style="display:none">
              <span class="sk-fb-user-icon">👤</span>
              <span id="sk-fb-user-label-value" class="sk-fb-user-val" style="display:none"></span>
              <input type="text" id="sk-fb-user-name" class="sk-fb-input sk-fb-input-user" placeholder="Nama Anda" style="display:none" />
            </div>
            <!-- Status message -->
            <div id="sk-fb-status" class="sk-fb-status"></div>
          </div>
        </div>
      </div>
      <div class="srikandi-ai-modal-footer">
        <button class="srikandi-ai-btn srikandi-ai-btn-primary" id="sk-btn-sisipkan">
          📄 Sisipkan ke Naskah
        </button>
      </div>
    `;

    overlay.appendChild(modal);
    document.body.appendChild(overlay);

    // ── Init feedback state ──
    initFeedbackModal(result);
  }

  // ── New: Build sub-klasifikasi as numbered list (inline) ────────
  // (buildSubKlasifikasiHtml tetap digunakan untuk subItems, di atas)

  function initFeedbackModal(result) {
    const subItems = result.sub_klasifikasi || [];

    // Close modal — hanya via X button, bukan klik di luar overlay
    document.getElementById('srikandi-ai-close-modal').addEventListener('click', removeExistingModal);

    // Radio buttons: Setuju / Koreksi
    document.querySelectorAll('input[name="sk-fb-mode"]').forEach(radio => {
      radio.addEventListener('change', () => {
        const mode = radio.value;
        modalFeedbackState.mode = mode;
        modalFeedbackState.feedbackStatus = null;
        modalFeedbackState.feedbackError = null;

        // Toggle panels
        document.getElementById('sk-fb-setuju-panel').style.display = mode === 'setuju' ? 'block' : 'none';
        document.getElementById('sk-fb-koreksi-panel').style.display = mode === 'koreksi' ? 'block' : 'none';
        document.getElementById('sk-fb-status').innerHTML = '';

        if (mode === 'koreksi') {
          initKoreksiPanel();
        }
      });
    });

    // Single button: Sisipkan ke Naskah
    document.getElementById('sk-btn-sisipkan').addEventListener('click', () => handleSisipkan(result));

    // Tampilkan identitas pengguna SRIKANDI (nama yang akan tercatat di feedback)
    loadUserIdentity();
  }

  // Tampilkan nama pengguna SRIKANDI di area feedback. Bila tidak ter-scrape,
  // tampilkan input manual agar user bisa mengisi sendiri.
  async function loadUserIdentity() {
    const userName = await getUserName();

    const userArea = document.getElementById('sk-fb-user-area');
    const userNameLabel = document.getElementById('sk-fb-user-label-value');
    const userNameInput = document.getElementById('sk-fb-user-name');
    if (!userArea) return;

    userArea.style.display = 'flex';
    if (userName) {
      // Label read-only dari scrape SRIKANDI
      if (userNameLabel) {
        userNameLabel.textContent = userName;
        userNameLabel.style.display = 'inline';
      }
      if (userNameInput) userNameInput.style.display = 'none';
    } else {
      // Fallback: input manual
      if (userNameLabel) userNameLabel.style.display = 'none';
      if (userNameInput) {
        userNameInput.style.display = 'inline';
        userNameInput.placeholder = 'Masukkan nama Anda';
        userNameInput.readOnly = false;
      }
    }
  }

  // ── Handle: Sisipkan ke Naskah (feedback + isi form atomik) ────
  async function handleSisipkan(result) {
    if (modalFeedbackState.feedbackStatus === 'sending') return;

    const mode = modalFeedbackState.mode;
    if (!mode) {
      showFeedbackStatus('Pilih Setuju atau Koreksi terlebih dahulu', 'error');
      return;
    }

    if (mode === 'setuju') {
      const select = document.getElementById('sk-fb-setuju-select');
      const kode = select.value;
      const deskripsi = select.options[select.selectedIndex]?.dataset?.deskripsi || '';
      const selectedSub = { kode, deskripsi };

      // Feedback positif — tanpa login (anonim, tercatat dengan chat_id)
      showFeedbackStatus('⏳ Menyimpan feedback...', 'sending');
      const res = await submitFeedback(result, 'setuju', selectedSub, null);

      if (res.error) {
        showFeedbackStatus(`❌ ${res.error}`, 'error');
        return;
      }
      // res.success === true berarti berhasil
      showFeedbackStatus('✅ Feedback tersimpan', 'success');

      // Isi form SRIKANDI
      fillSrikandiForm(result);
      showFeedbackStatus('✅ Feedback tersimpan & form terisi!', 'success');
      setTimeout(() => removeExistingModal(), 1500);

    } else if (mode === 'koreksi') {
      // Koreksi butuh login — cek sesi extension
      const auth = await getExtLoginStatus();
      if (!auth.loggedIn) {
        showFeedbackStatus('🔐 Login Google diperlukan untuk mengirim koreksi.', 'error');
        initKoreksiPanel();
        return;
      }

      const kodeInput = document.getElementById('sk-fb-cari-kode');
      const raw = kodeInput?.value?.trim() || '';
      if (!raw) {
        showFeedbackStatus('Ketik atau pilih kode koreksi terlebih dahulu', 'error');
        return;
      }
      let kode, deskripsi;
      if (kodeInput._selectedKode) {
        kode = kodeInput._selectedKode;
        deskripsi = kodeInput._selectedDeskripsi || '(tanpa deskripsi)';
      } else {
        const parts = raw.split(' — ');
        kode = parts[0]?.trim() || raw;
        deskripsi = parts[1]?.trim() || '(tanpa deskripsi)';
      }
      const alasan = document.getElementById('sk-fb-alasan')?.value?.trim() || '';

      showFeedbackStatus('⏳ Mengirim koreksi ke AI Arsiparis...', 'sending');
      const res = await submitFeedback(result, 'koreksi', null, { kode, deskripsi }, alasan);

      if (res.error) {
        // 401 / sesi kedaluwarsa → tampilkan panel login
        const perluLogin = /login|sesi|401/i.test(res.error);
        showFeedbackStatus(`❌ ${res.error}`, 'error');
        if (perluLogin) initKoreksiPanel();
        return;
      }
      if (!res.success || !res.data || !res.data.valid) {
        showFeedbackStatus(`❌ Koreksi ditolak: ${res.data?.penjelasan || 'Alasan tidak diketahui'}`, 'error');
        return;
      }
      showFeedbackStatus('✅ Koreksi diterima', 'success');

      // Isi form SRIKANDI dengan kode hasil validasi
      result.kode_detil = res.data.kode_terbaik || kode;
      result.detil_deskripsi = deskripsi;
      fillSrikandiForm(result);
      showFeedbackStatus('✅ Koreksi diterima & form terisi!', 'success');
      setTimeout(() => removeExistingModal(), 1500);
    }
  }

  function showFeedbackStatus(msg, type) {
    const el = document.getElementById('sk-fb-status');
    if (!el) return;
    const cls = type === 'error' ? 'sk-fb-msg-error'
              : type === 'success' ? 'sk-fb-msg-success'
              : type === 'sending' ? 'sk-fb-msg-sending'
              : '';
    el.innerHTML = `<span class="${cls}">${escapeHtml(msg)}</span>`;
  }

  // (overlayClickClose dihapus — hanya close via X button atau setelah sukses sisip)

  // ── Panel Koreksi: cek login → tampilkan form / tombol login ──

  async function getExtLoginStatus() {
    try {
      const stored = await chrome.storage.local.get(['ext_token', 'ext_user']);
      return { loggedIn: !!stored['ext_token'], user: stored['ext_user'] || null };
    } catch (err) {
      return { loggedIn: false, user: null };
    }
  }

  async function initKoreksiPanel() {
    const loginDiv = document.getElementById('sk-fb-koreksi-login');
    const formDiv = document.getElementById('sk-fb-koreksi-form');
    if (!loginDiv || !formDiv) return;

    const auth = await getExtLoginStatus();
    if (auth.loggedIn) {
      loginDiv.style.display = 'none';
      formDiv.style.display = 'block';
      setupAutocomplete();
    } else {
      loginDiv.style.display = 'block';
      formDiv.style.display = 'none';
      const btnLogin = document.getElementById('sk-btn-login-google');
      if (btnLogin) {
        btnLogin.disabled = false;
        btnLogin.textContent = '🔑 Login dengan Google';
        // Hindari penumpukan listener (initKoreksiPanel bisa dipanggil berulang)
        if (btnLogin._loginHandler) {
          btnLogin.removeEventListener('click', btnLogin._loginHandler);
        }
        const handler = async () => {
          btnLogin.disabled = true;
          btnLogin.textContent = '⏳ Memproses...';
          const res = await chrome.runtime.sendMessage({ type: 'LOGIN_GOOGLE' }).catch(() => null);
          btnLogin.disabled = false;
          btnLogin.textContent = '🔑 Login dengan Google';
          if (res && res.success) {
            const name = (res.user && (res.user.name || res.user.email)) || 'selamat datang';
            showFeedbackStatus(`✅ Login berhasil — ${name}. Silakan pilih kode koreksi.`, 'success');
            await initKoreksiPanel();
          } else {
            showFeedbackStatus(`❌ Login gagal: ${(res && res.error) || 'Gagal terhubung ke server'}`, 'error');
          }
        };
        btnLogin._loginHandler = handler;
        btnLogin.addEventListener('click', handler);
      }
    }
  }

  // ── Autocomplete untuk input koreksi kode (GET /api/codes) ────

  let autocompleteTimeout = null;

  function setupAutocomplete() {
    const kodeInput = document.getElementById('sk-fb-cari-kode');
    const dropdown = document.getElementById('sk-fb-ac-dropdown');
    if (!kodeInput || !dropdown) return;

    // Hapus listener lama
    if (kodeInput._acListener) {
      kodeInput.removeEventListener('input', kodeInput._acListener);
    }
    if (kodeInput._acKeyListener) {
      kodeInput.removeEventListener('keydown', kodeInput._acKeyListener);
    }
    if (kodeInput._clickAwayHandler) {
      document.removeEventListener('mousedown', kodeInput._clickAwayHandler);
      delete kodeInput._clickAwayHandler;
    }

    let selectedIndex = -1;
    let searchSeq = 0; // anti-race-condition: ignore stale search results

    function closeDropdown() {
      dropdown.style.display = 'none';
      selectedIndex = -1;
    }

    function selectItem(el) {
      if (!el) return;
      if (autocompleteTimeout) {
        clearTimeout(autocompleteTimeout);
        autocompleteTimeout = null;
      }
      searchSeq++;
      kodeInput.value = `${el.dataset.kode} — ${el.dataset.deskripsi}`;
      kodeInput._selectedKode = el.dataset.kode;
      kodeInput._selectedDeskripsi = el.dataset.deskripsi;
      const selLabel = document.getElementById('sk-fb-selected-label');
      const selKode = document.getElementById('sk-fb-selected-kode');
      const selDesc = document.getElementById('sk-fb-selected-desc');
      if (selLabel && selKode && selDesc) {
        selKode.textContent = el.dataset.kode;
        selDesc.textContent = el.dataset.deskripsi;
        selLabel.style.display = 'flex';
      }
      closeDropdown();
    }

    function getItems() {
      return dropdown.querySelectorAll('.sk-ac-item');
    }

    function highlightItem(index) {
      const items = getItems();
      items.forEach((item, i) => {
        item.classList.toggle('sk-ac-highlighted', i === index);
      });
      if (items[index]) {
        items[index].scrollIntoView({ block: 'nearest' });
      }
      selectedIndex = index;
    }

    async function doSearch(query) {
      if (query.length < 2) {
        closeDropdown();
        return;
      }
      const mySeq = ++searchSeq;
      try {
        const data = await chrome.runtime.sendMessage({ type: 'SEARCH_SUGGESTIONS', query });
        if (mySeq !== searchSeq) return;
        if (!data || data.error || data.length === 0) {
          dropdown.innerHTML = '<div class="fb-ac-empty">Tidak ditemukan</div>';
          dropdown.style.display = 'block';
          selectedIndex = -1;
          return;
        }
        if (mySeq !== searchSeq) return;
        dropdown.innerHTML = data.map((item, idx) => `
          <div class="sk-ac-item" data-index="${idx}" data-kode="${escapeHtml(item.kode)}" data-deskripsi="${escapeHtml(item.deskripsi)}">
            <span class="sk-ac-kode">${escapeHtml(item.kode)}</span>
            <span class="sk-ac-desc">${escapeHtml(item.deskripsi)}</span>
          </div>
        `).join('');
        dropdown.style.display = 'block';
        selectedIndex = -1;
        dropdown.querySelectorAll('.sk-ac-item').forEach(el => {
          el.addEventListener('click', () => selectItem(el));
        });
      } catch (err) {
        closeDropdown();
      }
    }

    const inputHandler = () => {
      const val = kodeInput.value.trim();
      if (autocompleteTimeout) {
        clearTimeout(autocompleteTimeout);
        autocompleteTimeout = null;
      }
      searchSeq++;
      autocompleteTimeout = setTimeout(() => doSearch(val), 250);
    };
    kodeInput.addEventListener('input', inputHandler);
    kodeInput._acListener = inputHandler;

    const clickAwayHandler = (e) => {
      const wrap = kodeInput.closest('.sk-ac-wrap');
      if (wrap && !wrap.contains(e.target)) {
        closeDropdown();
      }
    };
    document.addEventListener('mousedown', clickAwayHandler);
    kodeInput._clickAwayHandler = clickAwayHandler;

    const keyHandler = (e) => {
      const isOpen = dropdown.style.display !== 'none';
      const items = getItems();
      const count = items.length;
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        if (!isOpen || count === 0) return;
        highlightItem(selectedIndex < count - 1 ? selectedIndex + 1 : 0);
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        if (!isOpen || count === 0) return;
        highlightItem(selectedIndex > 0 ? selectedIndex - 1 : count - 1);
      } else if (e.key === 'Enter') {
        if (isOpen && selectedIndex >= 0 && selectedIndex < count) {
          e.preventDefault();
          selectItem(items[selectedIndex]);
        }
      } else if (e.key === 'Escape') {
        closeDropdown();
      }
    };
    kodeInput.addEventListener('keydown', keyHandler);
    kodeInput._acKeyListener = keyHandler;
  }

  function showLoadingOverlay(message) {
    removeExistingModal();
    const overlay = document.createElement('div');
    overlay.id = 'srikandi-ai-modal-overlay';
    overlay.className = 'srikandi-ai-overlay';
    overlay.innerHTML = `
      <div class="srikandi-ai-modal srikandi-ai-modal-loading">
        <div class="srikandi-ai-modal-body" style="text-align:center;padding:40px">
          <div class="srikandi-ai-spinner"></div>
          <p style="margin-top:16px;color:#555">${escapeHtml(message)}</p>
        </div>
      </div>
    `;
    document.body.appendChild(overlay);
  }

  function showErrorOverlay(errorMsg) {
    removeExistingModal();
    const overlay = document.createElement('div');
    overlay.id = 'srikandi-ai-modal-overlay';
    overlay.className = 'srikandi-ai-overlay';
    overlay.innerHTML = `
      <div class="srikandi-ai-modal">
        <div class="srikandi-ai-modal-header">
          <h3 style="color:#ef4444">❌ Gagal Analisa</h3>
          <button class="srikandi-ai-close" id="srikandi-ai-close-modal">&times;</button>
        </div>
        <div class="srikandi-ai-modal-body" style="text-align:center;padding:30px">
          <p style="color:#666">${escapeHtml(errorMsg)}</p>
        </div>
        <div class="srikandi-ai-modal-footer" style="justify-content:center">
          <button class="srikandi-ai-btn srikandi-ai-btn-secondary" id="srikandi-ai-try-again">🔄 Coba Lagi</button>
        </div>
      </div>
    `;
    document.body.appendChild(overlay);
    document.getElementById('srikandi-ai-close-modal').addEventListener('click', removeExistingModal);
    document.getElementById('srikandi-ai-try-again').addEventListener('click', () => {
      removeExistingModal();
      handleAnalisaClick();
    });
  }

  function removeExistingModal() {
    const existing = document.getElementById('srikandi-ai-modal-overlay');
    if (existing) existing.remove();
  }

  // ── Main Handler ─────────────────────────────────────────────

  async function handleAnalisaClick() {
    if (analysisState.status === 'loading') {
      return; // Prevent double-click
    }

    // Check file
    const fileResult = await readUploadedFile();
    if (!fileResult.found) {
      // Maybe the form already has text content
      const halText = getHalTextarea()?.value?.trim();
      const ringkasanText = getRingkasanTextarea()?.value?.trim();

      if (halText || ringkasanText) {
        // Use form text as input
        const teks = [halText, ringkasanText].filter(Boolean).join('\n\n');
        startAnalysis(teks);
        return;
      }

      showErrorOverlay(
        'Tidak ada file yang terupload. Silakan upload file DOCX template naskah ' +
        'terlebih dahulu melalui form "File naskah" di atas.'
      );
      return;
    }

    analysisState.text = '';

    if (fileResult.ext === 'docx') {
      // DOCX: ekstrak teks client-side via mammoth (backend hanya ekstrak PDF)
      showLoadingOverlay('Membaca file DOCX...');
      analysisState.status = 'loading';
      if (typeof mammoth === 'undefined' || !mammoth.extractRawText) {
        analysisState.status = 'error';
        showErrorOverlay('Library ekstraksi DOCX (mammoth) tidak termuat. Muat ulang extension di chrome://extensions.');
        return;
      }
      mammoth.extractRawText({ arrayBuffer: fileResult.buffer })
        .then((extracted) => {
          const text = (extracted.value || '')
            .replace(/\n{3,}/g, '\n\n')
            .replace(/\s{2,}/g, ' ')
            .trim();
          if (!text) {
            analysisState.status = 'error';
            showErrorOverlay('Tidak ada teks yang bisa diekstrak dari DOCX.');
            return;
          }
          startAnalysis(text);
        })
        .catch((err) => {
          analysisState.status = 'error';
          showErrorOverlay('Gagal mengekstrak DOCX: ' + (err && err.message ? err.message : err));
        });
      return;
    }

    // PDF: kirim file ke backend (ekstraksi pdf-inspector) lalu analisa
    showLoadingOverlay('Membaca file...');
    analysisState.status = 'loading';

    chrome.runtime.sendMessage(
      {
        type: 'ANALISA_FILE',
        fileName: fileResult.name,
        fileData: fileResult.data,
        fileExt: fileResult.ext,
      },
      (response) => {
        if (chrome.runtime.lastError) {
          analysisState.status = 'error';
          showErrorOverlay('Gagal komunikasi dengan background: ' + chrome.runtime.lastError.message);
          return;
        }

        if (response.error) {
          analysisState.status = 'error';
          showErrorOverlay(response.error);
          return;
        }

        // API sinkron — hasil langsung (tanpa task_id/polling)
        if (!response.result) {
          analysisState.status = 'error';
          showErrorOverlay('Hasil tidak tersedia dari server.');
          return;
        }
        analysisState.status = 'done';
        analysisState.result = response.result;
        analysisState.text = response.text || '';
        showResultModal(response.result);
      }
    );
  }

  function startAnalysis(teks) {
    showLoadingOverlay('Menganalisa naskah dengan AI...');
    analysisState.status = 'loading';

    chrome.runtime.sendMessage(
      { type: 'ANALISA_TEKS', teks },
      (response) => {
        if (chrome.runtime.lastError) {
          analysisState.status = 'error';
          showErrorOverlay('Gagal komunikasi: ' + chrome.runtime.lastError.message);
          return;
        }
        if (response.error) {
          analysisState.status = 'error';
          showErrorOverlay(response.error);
          return;
        }
        // API sinkron — hasil langsung
        analysisState.status = 'done';
        analysisState.result = response.result;
        analysisState.text = teks;
        showResultModal(response.result);
      }
    );
  }

  // ── Fill SRIKANDI Form ───────────────────────────────────────

  function fillSrikandiForm(result) {
    const { perihal, isi_ringkas, kode_klasifikasi, kode_detil } = result;

    // 1. Isi Perihal (Hal)
    const halField = getHalTextarea();
    if (halField && perihal) {
      setNativeValue(halField, perihal);
    }

    // 2. Isi Ringkasan (isi ringkas dari backend — khusus extension)
    const ringkasanField = getRingkasanTextarea();
    if (ringkasanField && isi_ringkas) {
      setNativeValue(ringkasanField, isi_ringkas);
    }

    // 3. Isi Klasifikasi (react-select)
    // Coba dengan kode detil dulu (lebih spesifik), fallback ke kode fungsi
    const kodeTarget = kode_detil || kode_klasifikasi;
    if (kodeTarget) {
      setReactSelectValue(kodeTarget);
    }
  }

  function setNativeValue(element, value) {
    // Native setter untuk trigger React onChange
    const nativeSetter = Object.getOwnPropertyDescriptor(
      element instanceof HTMLTextAreaElement
        ? window.HTMLTextAreaElement.prototype
        : window.HTMLInputElement.prototype,
      'value'
    ).set;
    nativeSetter.call(element, value);

    // Dispatch events yang React dengarkan
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function setReactSelectValue(value) {
    const input = getKlasifikasiInput();
    if (!input) {
      return false;
    }

    // Focus the input
    input.focus();

    // Clear existing value first
    const nativeSetter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype, 'value'
    ).set;

    // Kosongkan dulu
    nativeSetter.call(input, '');
    input.dispatchEvent(new Event('input', { bubbles: true }));

    // Ketik nilai baru
    nativeSetter.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));

    // Wait for async options to load, then select matching option
    const maxWait = 3000; // ms
    const startTime = Date.now();

    function trySelect() {
      if (Date.now() - startTime > maxWait) {
        input.dispatchEvent(new KeyboardEvent('keydown', {
          key: 'Enter', code: 'Enter', keyCode: 13, which: 13,
          bubbles: true, cancelable: true
        }));
        return;
      }

      // Cari menu yang visible
      const menus = document.querySelectorAll('[class*="-menu"], [id$="-listbox"]');
      let targetMenu = null;

      for (const m of menus) {
        // Cari menu yang tidak hidden/display:none
        const style = window.getComputedStyle(m);
        if (style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0') {
          // Scope ke menu yang dekat dengan input kita (optional: cek parent)
          targetMenu = m;
          break;
        }
      }

      if (targetMenu) {
        const options = targetMenu.querySelectorAll('[class*="-option"]');

        // Cari option yang textnya mengandung kode kita
        for (const opt of options) {
          if (opt.textContent.includes(value)) {
            opt.click();
            return;
          }
        }

        // Jika tidak ada yang cocok, klik option pertama
        if (options.length > 0) {
          options[0].click();
          return;
        }
      }

      // Menu belum muncul, tunggu lagi
      setTimeout(trySelect, 200);
    }

    setTimeout(trySelect, 500);
  }

  // ── Listen for background messages ────────────────────────────

  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    if (message.type === 'ISI_FORM') {
      fillSrikandiForm(message.data);
      sendResponse({ success: true });
    }
    return true;
  });

  // ── Utilities ─────────────────────────────────────────────────

  function escapeHtml(text) {
    if (!text) return '';
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
  }

  // ── User Identity SRIKANDI ──────────────────────────────────
  // Scrape nama lengkap pengguna dari halaman SRIKANDI (avatar/navbar) lalu
  // cache di storage. Nama ini dikirim bersama feedback (positif & koreksi)
  // agar feedback dari extension tercatat atas nama pengguna SRIKANDI.

  const USER_STORAGE_KEY = 'srikandi_user_name';

  function scrapeUserName() {
    // 1. Cari img MUI Avatar dengan alt (ditemukan user: img.MuiAvatar-img[alt])
    const avatarImg = document.querySelector('img.MuiAvatar-img[alt]');
    if (avatarImg && avatarImg.alt && avatarImg.alt.trim().length > 0 && avatarImg.alt.trim().length < 100) {
      return avatarImg.alt.trim();
    }

    // 2. Cari img dengan class mengandung "avatar"/"profile" dan punya alt
    const imgSelectors = [
      'img[class*="avatar"][alt]',
      'img[class*="Avatar"][alt]',
      'img[class*="profile"][alt]',
      'img[class*="Profile"][alt]',
      'img[alt]:not([alt=""])',
    ];
    for (const sel of imgSelectors) {
      const el = document.querySelector(sel);
      if (el && el.alt && el.alt.trim().length > 0 && el.alt.trim().length < 100) {
        return el.alt.trim();
      }
    }

    // 3. Selector berbasis teks
    const selectors = [
      '[class*="user-name"]', '[class*="username"]',
      '[class*="profile-name"]', '[class*="display-name"]',
      '[class*="nama-user"]', '[class*="nama_pegawai"]',
      '[class*="MuiAvatar"] ~ [class*="Mui"]',
      'header [class*="MuiTypography"]',
      'nav [class*="MuiTypography"]',
      '[class*="toolbar"] [class*="MuiTypography"]',
    ];

    for (const sel of selectors) {
      const el = document.querySelector(sel);
      if (el && el.textContent.trim().length > 0 && el.textContent.trim().length < 100) {
        return el.textContent.trim();
      }
    }

    // 4. Fallback: cari text yang mengandung "Selamat datang" / "Hi" / "Halo"
    const bodyText = document.body.textContent || '';
    const welcomeMatch = bodyText.match(/(?:Selamat datang|Hi|Halo|Welcome)\s*,?\s*([A-Za-z\s.]+?)(?:[.!]|\s|$)/i);
    if (welcomeMatch) {
      const name = welcomeMatch[1].trim();
      if (name.length > 0 && name.length < 60) {
        return name;
      }
    }

    return '';
  }

  async function getUserName() {
    // Coba dari storage dulu (cache — scrape cukup sekali per browser)
    try {
      const stored = await chrome.storage.local.get([USER_STORAGE_KEY]);
      if (stored[USER_STORAGE_KEY]) {
        return stored[USER_STORAGE_KEY];
      }
    } catch (err) {
    }

    // Coba scrape dari halaman
    const scraped = scrapeUserName();
    if (scraped) {
      try {
        await chrome.storage.local.set({ [USER_STORAGE_KEY]: scraped });
      } catch (err) {}
      return scraped;
    }

    return ''; // Tidak ditemukan
  }

  // ── Feedback ─────────────────────────────────────────────────

  async function submitFeedback(result, mode, selectedSub, correctedItem, alasan) {
    // Backend kode-klasifikasi-chat:
    // - POSITIF (setuju): anonim tanpa login, dicatat dengan chat_id
    // - KOREKSI: butuh login — Authorization: Bearer JWT ditambahkan di background
    const naskah = (analysisState.text || '').trim();
    if (!naskah) {
      return { error: 'Data naskah tidak tersedia untuk feedback.' };
    }

    let payload;
    if (mode === 'setuju') {
      const kodeTerpilih = selectedSub?.kode || result.kode_klasifikasi || '';
      if (!kodeTerpilih) {
        return { error: 'Kode tidak tersedia untuk feedback.' };
      }
      payload = {
        message: naskah.slice(0, 1000), // backend menyimpan maks 1000 karakter
        kode_ai: kodeTerpilih,
        feedback_type: 'positive',
        perihal: result.perihal || '',
        chat_id: getChatId(),
      };
    } else if (mode === 'koreksi' && correctedItem?.kode) {
      payload = {
        message: naskah.slice(0, 1000),
        kode_ai: result.kode_klasifikasi || '',
        feedback_type: 'correction',
        kode_koreksi: correctedItem.kode,
        alasan: alasan || '',
        perihal: result.perihal || '',
        chat_id: getChatId(),
      };
    } else {
      return { error: 'Mode feedback tidak valid' };
    }

    // Sertakan API key Gemini dari storage (opsional — rotasi otomatis di backend)
    try {
      const stored = await chrome.storage.local.get(['gemini_api_keys']);
      if (stored['gemini_api_keys']) {
        const keys = stored['gemini_api_keys'].split('\n')
          .map(k => k.trim()).filter(k => k.length > 0);
        if (keys.length > 0) {
          payload.api_keys = keys;
        }
      }
    } catch (err) {}

    // Sertakan nama lengkap pengguna SRIKANDI — dipakai backend sebagai nama
    // tampilan feedback (positif & koreksi). Prioritas: input manual di modal
    // (bila user mengisi), fallback ke hasil scrape dari halaman SRIKANDI.
    let userName = '';
    const manualInput = document.getElementById('sk-fb-user-name');
    if (manualInput && manualInput.style.display !== 'none') {
      userName = manualInput.value.trim();
    }
    if (!userName) {
      userName = await getUserName();
    }
    if (userName) {
      payload.user_name = userName;
    }

    try {
      const res = await chrome.runtime.sendMessage({
        type: 'SUBMIT_FEEDBACK',
        payload,
      });
      return res || { error: 'Tidak ada response dari background' };
    } catch (err) {
      return { error: `Gagal kirim feedback: ${err.message}` };
    }
  }

  // ── SPA Navigation Detection ───────────────────────────────

  function setupSpaNavigationHandler() {

    let previousUrl = window.location.href;

    function checkForSpaNavigation() {
      const currentUrl = window.location.href;
      if (currentUrl !== previousUrl) {
        previousUrl = currentUrl;
        handleUrlChange();
      }
    }

    // Handle URL change
    function handleUrlChange() {
      // Hapus tombol yang sudah ada
      const existingBtn = document.getElementById('srikandi-ai-analisa-btn');
      if (existingBtn) existingBtn.remove();

      // Hapus peringatan keamanan yang menyertai tombol (hindari duplikat)
      const existingWarn = document.getElementById('srikandi-ai-warning');
      if (existingWarn) existingWarn.remove();

      // Hapus modal yang mungkin masih ada
      removeExistingModal();

      // Reset state
      analysisState = {
        status: 'idle',
        result: null,
        error: null,
        text: '',
      };

      if (isOnRegistrasiPage()) {
        // Tunggu form render (SPA loading)
        waitForFormAndInject();
      } else {
      }
    }

    // 1. Override history.pushState
    const originalPushState = history.pushState;
    history.pushState = function (...args) {
      originalPushState.apply(this, args);
      checkForSpaNavigation();
    };

    // 2. Override history.replaceState
    const originalReplaceState = history.replaceState;
    history.replaceState = function (...args) {
      originalReplaceState.apply(this, args);
      checkForSpaNavigation();
    };

    // 3. Listen for popstate (back/forward)
    window.addEventListener('popstate', checkForSpaNavigation);

    // 4. MutationObserver sebagai fallback
    const observer = new MutationObserver(() => {
      // Cek hanya jika URL tidak berubah (DOM berubah dalam page yg sama)
      if (isOnRegistrasiPage() && !document.getElementById('srikandi-ai-analisa-btn')) {
        // Cek apakah form sudah ada (mungkin SPA loading selesai)
        const halField = getHalTextarea();
        const fileInput = getFileInput();
        if (halField && fileInput) {
          injectAnalisaButton();
        }
      }
    });
    observer.observe(document.body, { childList: true, subtree: true });
  }

  // ── Wait for form and inject button ─────────────────────────

  function waitForFormAndInject() {

    const waitForForm = setInterval(() => {
      const halField = getHalTextarea();
      const fileInput = getFileInput();
      if (halField && fileInput) {
        clearInterval(waitForForm);
        injectAnalisaButton();
      }
    }, 1000);

    // Fallback: inject anyway after 10s
    setTimeout(() => {
      clearInterval(waitForForm);
      if (!document.getElementById('srikandi-ai-analisa-btn')) {
        injectAnalisaButton();
      }
    }, 10000);
  }

  // ── Init ────────────────────────────────────────────────────

  async function init() {

    // Load API URL dari storage sebelum melakukan API calls
    await loadApiUrl();

    // Setup SPA navigation handler (override history, listen popstate + DOM)
    setupSpaNavigationHandler();

    // Jika langsung di halaman registrasi, inject button
    if (isOnRegistrasiPage()) {
      waitForFormAndInject();
    }
  }

  // Start
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
