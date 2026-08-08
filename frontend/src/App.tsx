import { useState, useRef, useEffect, useCallback } from 'react'
import * as pdfjsLib from 'pdfjs-dist'
import mammoth from 'mammoth'

// Set PDF.js worker untuk versi 5.x
pdfjsLib.GlobalWorkerOptions.workerSrc = new URL('pdfjs-dist/build/pdf.worker.min.mjs', import.meta.url).toString()


interface ClassificationResult {
  id: number
  kode: string
  deskripsi: string
  path: string
  similarity: number
}

interface ChatResponse {
  results: ClassificationResult[]
  explanation: string
  perihal: string
}

interface ErrorResponse {
  error: string
  retry_after_secs?: number
}

interface ModelQuotaInfo {
  rpm_used: number
  rpm_limit: number
  rpd_used: number
  rpd_limit: number
  minute_reset_secs: number
  day_reset_secs: number
}

interface QuotaStats {
  enabled: boolean
  chat: ModelQuotaInfo
  embed: ModelQuotaInfo
  overall_pct: number
}

function formatDuration(s: number): string {
  if (s >= 3600) {
    const h = Math.floor(s / 3600)
    const m = Math.round((s % 3600) / 60)
    return m > 0 ? `${h} jam ${m} mnt` : `${h} jam`
  }
  if (s >= 60) {
    const m = Math.floor(s / 60)
    const sec = s % 60
    return sec > 0 ? `${m} mnt ${sec} dtk` : `${m} mnt`
  }
  return `${s} dtk`
}

interface Message {
  role: 'user' | 'assistant'
  content: string
  results?: ClassificationResult[]
  isRateLimit?: boolean
  query?: string // teks naskah asli (untuk feedback)
  perihal?: string // perihal naskah dari hasil rerank AI (untuk validasi & statistik)
}

// ---------- Auth (Google OAuth PKCE) ----------

interface AuthUser {
  sub: string
  email: string
  name: string
  is_admin?: boolean
}

interface StatsFilter {
  status: string // '' | 'validated' | 'rejected' | 'pending'
  perihal: string
}

interface AuthConfig {
  enabled: boolean
  client_id: string
  redirect_uri: string // URI pertama (kompatibilitas)
  redirect_uris: string[]
}

interface FeedbackResult {
  valid: boolean
  kode_terbaik: string | null
  penjelasan: string
}

interface FeedbackState {
  type: 'positive' | 'correction'
  sending?: boolean
  result?: FeedbackResult
}

interface CorrectionFormState {
  kode: string
  alasan: string
  open?: boolean
  suggestions?: { kode: string; deskripsi: string; path: string }[]
  selected?: { kode: string; deskripsi: string; path: string } | null
  search?: string
  loading?: boolean
}

interface RecentFeedback {
  id: number
  feedback_type: 'positive' | 'correction'
  kode_ai: string
  kode_koreksi: string
  status: string
  user_name: string
  user_email: string
  penjelasan: string
  perihal: string
  naskah: string
  waktu: string
}

interface FeedbackStats {
  total: number
  positive: number
  correction: number
  correction_valid: number
  correction_rejected: number
  top_kode: { kode: string; count: number }[]
  top_user: { user: string; count: number }[]
  recent: RecentFeedback[]
}

function base64UrlEncode(bytes: Uint8Array): string {
  let bin = ''
  bytes.forEach(b => { bin += String.fromCharCode(b) })
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

async function pkcePair() {
  const verifierBytes = new Uint8Array(32)
  crypto.getRandomValues(verifierBytes)
  const verifier = base64UrlEncode(verifierBytes)
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier))
  const challenge = base64UrlEncode(new Uint8Array(digest))
  return { verifier, challenge }
}

const EXAMPLE_QUERIES = [
  'Permohonan cuti tahunan pegawai',
  'Pengadaan laptop untuk unit kerja',
  'Laporan keuangan triwulan III',
]

// ---------- ID Sesi Chat (chat_id) ----------
// ID unik per browser, dibuat sekali & disimpan di localStorage. Dikirim bersama
// feedback agar feedback — termasuk yang ANONIM — tetap bisa dikaitkan ke sesi
// chat yang sama (tanpa login & tanpa fingerprint teknis yang invasif).

const CHAT_ID_STORAGE = 'kk_chat_id'

function randomHex(len: number): string {
  const arr = new Uint8Array(len)
  crypto.getRandomValues(arr)
  return Array.from(arr, b => b.toString(16).padStart(2, '0')).join('')
}

/** Muat chat_id dari localStorage; buat & simpan bila belum ada (UUID v4-like). */
function loadChatId(): string {
  let id = localStorage.getItem(CHAT_ID_STORAGE)
  if (!id) {
    // crypto.getRandomValues tersedia di semua browser modern; fallback ringan
    // untuk embedder eksotis yang tidak punya crypto sama sekali.
    if (typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function') {
      id = `${randomHex(8)}-${randomHex(4)}-4${randomHex(3)}-${['8', '9', 'a', 'b'][Math.floor(Math.random() * 4)]}${randomHex(3)}-${randomHex(12)}`
    } else {
      id = `anon-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
    }
    localStorage.setItem(CHAT_ID_STORAGE, id)
  }
  return id
}

const CHAT_ID = loadChatId()

// ---------- Pengaturan API Key (multi-key, toggle lihat/sembunyikan) ----------

const KEYS_STORAGE = 'gemini_api_keys'

/** Muat daftar key dari localStorage + migrasi key tunggal lama (gemini_api_key). */
function loadSavedKeys(): string[] {
  let arr: string[] = []
  try { arr = JSON.parse(localStorage.getItem(KEYS_STORAGE) || '[]') } catch { arr = [] }
  if (!Array.isArray(arr)) arr = []
  const legacy = (localStorage.getItem('gemini_api_key') || '').trim()
  if (legacy && !arr.includes(legacy)) arr = [legacy, ...arr]
  if (arr.length > 0) localStorage.setItem(KEYS_STORAGE, JSON.stringify(arr))
  localStorage.removeItem('gemini_api_key')
  return arr
}

function ApiKeySettings({ keys, onSave }: {
  keys: string[]
  onSave: (keys: string[]) => void
}) {
  const [newKey, setNewKey] = useState('')
  const [showAll, setShowAll] = useState(false)
  const [showNew, setShowNew] = useState(false)

  const addKey = () => {
    const k = newKey.trim()
    if (!k) return
    if (!keys.includes(k)) onSave([...keys, k])
    setNewKey('')
  }

  const removeKey = (k: string) => {
    if (!window.confirm('Hapus API Key ini dari browser?')) return
    onSave(keys.filter(x => x !== k))
  }

  const maskKey = (k: string) => (k.length <= 8 ? '••••••••' : '••••••••' + k.slice(-4))

  const EyeIcon = ({ off }: { off: boolean }) => (
    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
      {off ? (
        <path strokeLinecap="round" strokeLinejoin="round" d="M3.98 8.223A10.477 10.477 0 0 0 1.934 12C3.226 16.338 7.244 19.5 12 19.5c.993 0 1.953-.138 2.863-.395M6.228 6.228A10.451 10.451 0 0 1 12 4.5c4.756 0 8.773 3.162 10.065 7.498a10.522 10.522 0 0 1-4.293 5.774M6.228 6.228 3 3m3.228 3.228 3.65 3.65m7.894 7.894L21 21m-3.228-3.228-3.65-3.65m0 0a3 3 0 1 0-4.243-4.243m4.242 4.242L9.88 9.88" />
      ) : (
        <path strokeLinecap="round" strokeLinejoin="round" d="M2.036 12.322a1.012 1.012 0 0 1 0-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178Z" />
      )}
      <path strokeLinecap="round" strokeLinejoin="round" d={off ? '' : 'M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z'} />
    </svg>
  )

  return (
    <main className="flex-1 overflow-y-auto px-6 py-6">
      <div className="max-w-2xl space-y-5">
        {/* Kartu daftar key */}
        <div className="rounded-xl border border-gray-800 bg-gray-950 overflow-hidden">
          <div className="px-4 py-3 border-b border-gray-800 flex items-center justify-between gap-3">
            <div className="min-w-0">
              <h3 className="text-xs font-semibold text-gray-300 uppercase tracking-wider">API Key Gemini ({keys.length})</h3>
              <p className="text-[10px] text-gray-600 mt-0.5">
                Tersimpan hanya di browser ini (localStorage). Diprioritaskan di atas key server & dirotasi otomatis.
              </p>
            </div>
            {keys.length > 0 && (
              <button
                type="button"
                onClick={() => setShowAll(!showAll)}
                className="shrink-0 text-xs px-3 py-1.5 rounded-lg border border-gray-700 text-gray-400 hover:text-white hover:border-gray-500 transition-colors"
              >
                {showAll ? '🙈 Sembunyikan' : '👁️ Lihat Semua'}
              </button>
            )}
          </div>

          {keys.length === 0 ? (
            <div className="px-4 py-8 text-center text-sm text-gray-500">
              Belum ada API Key tersimpan. Tambahkan key Gemini di bawah — boleh lebih dari satu untuk rotasi saat quota habis.
            </div>
          ) : (
            <ul className="divide-y divide-gray-800/60">
              {keys.map((k, i) => (
                <li key={i} className="flex items-center gap-2 px-4 py-2.5">
                  <span className="text-[10px] text-gray-600 shrink-0">#{i + 1}</span>
                  <code className="flex-1 text-xs text-gray-300 font-mono truncate">
                    {showAll ? k : maskKey(k)}
                  </code>
                  <button
                    type="button"
                    onClick={() => setShowAll(!showAll)}
                    title={showAll ? 'Sembunyikan key' : 'Lihat key'}
                    className="shrink-0 p-1.5 rounded-lg text-gray-500 hover:text-cyan-400 hover:bg-gray-800 transition-colors"
                  >
                    <EyeIcon off={!showAll} />
                  </button>
                  <button
                    type="button"
                    onClick={() => removeKey(k)}
                    title="Hapus key"
                    className="shrink-0 p-1.5 rounded-lg text-gray-500 hover:text-red-400 hover:bg-gray-800 transition-colors"
                  >
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="m14.74 9-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 0 1-2.244 2.077H8.084a2.25 2.25 0 0 1-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 0 0-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 0 1 3.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 0 0-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 0 0-7.5 0" />
                    </svg>
                  </button>
                </li>
              ))}
            </ul>
          )}

          {/* Form tambah key */}
          <div className="px-4 py-3 border-t border-gray-800 bg-gray-900/50 flex flex-col sm:flex-row gap-2">
            <div className="relative flex-1">
              <input
                type={showNew ? 'text' : 'password'}
                value={newKey}
                onChange={(e) => setNewKey(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addKey() } }}
                placeholder="Tempel API Key baru... (mis. AIzaSy...)"
                className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 pr-10 text-xs text-white placeholder-gray-500 focus:outline-none focus:border-violet-500"
              />
              <button
                type="button"
                onClick={() => setShowNew(!showNew)}
                title={showNew ? 'Sembunyikan' : 'Lihat'}
                className="absolute right-1.5 top-1/2 -translate-y-1/2 p-1 text-gray-500 hover:text-cyan-400 transition-colors"
              >
                {showNew ? '🙈' : '👁️'}
              </button>
            </div>
            <button
              type="button"
              onClick={addKey}
              disabled={!newKey.trim()}
              className="shrink-0 px-4 py-2 rounded-lg bg-violet-600 text-white text-xs font-medium hover:bg-violet-700 disabled:opacity-40 transition-colors"
            >
              + Tambah Key
            </button>
            {keys.length > 0 && (
              <button
                type="button"
                onClick={() => { if (window.confirm('Hapus SEMUA API Key yang tersimpan di browser ini?')) onSave([]) }}
                className="shrink-0 px-3 py-2 rounded-lg text-xs text-gray-500 hover:text-red-400 transition-colors"
              >
                Kosongkan semua
              </button>
            )}
          </div>
        </div>

        {/* Info cara kerja */}
        <div className="rounded-xl border border-gray-800 bg-gray-950 p-4 text-xs text-gray-500 space-y-2 leading-relaxed">
          <h4 className="text-xs font-semibold text-gray-300 uppercase tracking-wider">💡 Cara kerja multi-key</h4>
          <p>1. Key Anda dikirim bersama tiap permintaan klasifikasi dan dicoba <strong className="text-gray-300">berurutan</strong> (key #1, lalu #2, dst).</p>
          <p>2. Saat satu key habis kuota (rate limit), backend otomatis beralih ke key berikutnya — lalu fallback ke key server bila semua key pengguna gagal.</p>
          <p>3. Key dari project Google yang sama tetap berbagi quota yang sama; rotasi paling efektif bila tiap key berasal dari project berbeda.</p>
          <p className="text-amber-400/80">⚠️ Key disimpan mentah di localStorage perangkat ini. Jangan simpan key pada perangkat publik/bersama.</p>
        </div>
      </div>
    </main>
  )
}

// ---------- Dashboard Statistik Feedback ----------

function StatsDashboard({ stats, loading, onRefresh, filter, onApplyFilter, onClearFilter, canDelete, onDeleteClick }: {
  stats: FeedbackStats | null
  loading: boolean
  onRefresh: () => void
  filter: StatsFilter
  onApplyFilter: (f: StatsFilter) => void
  onClearFilter: () => void
  canDelete: boolean
  onDeleteClick: (r: RecentFeedback) => void
}) {
  // Input perihal lokal + debounce 300ms agar tidak membanjiri server per ketikan
  const [perihalInput, setPerihalInput] = useState(filter.perihal)
  const perihalTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    setPerihalInput(filter.perihal)
  }, [filter.perihal])

  useEffect(() => () => {
    if (perihalTimerRef.current) clearTimeout(perihalTimerRef.current)
  }, [])

  const onPerihalChange = (v: string) => {
    setPerihalInput(v)
    if (perihalTimerRef.current) clearTimeout(perihalTimerRef.current)
    perihalTimerRef.current = setTimeout(() => {
      onApplyFilter({ ...filter, perihal: v })
    }, 300)
  }
  if (loading && !stats) {
    return (
      <main className="flex-1 overflow-y-auto px-6 py-10 flex items-start justify-center">
        <div className="flex gap-1.5 mt-10">
          <span className="w-2 h-2 rounded-full bg-gray-500 animate-bounce" style={{ animationDelay: '0ms' }} />
          <span className="w-2 h-2 rounded-full bg-gray-500 animate-bounce" style={{ animationDelay: '150ms' }} />
          <span className="w-2 h-2 rounded-full bg-gray-500 animate-bounce" style={{ animationDelay: '300ms' }} />
        </div>
      </main>
    )
  }

  if (!stats) {
    return (
      <main className="flex-1 overflow-y-auto px-6 py-10">
        <div className="text-center text-sm text-gray-400">
          Belum ada data statistik.{' '}
          <button onClick={onRefresh} className="text-violet-400 hover:underline">Muat ulang</button>
        </div>
      </main>
    )
  }

  const pending = Math.max(0, stats.correction - stats.correction_valid - stats.correction_rejected)
  const maxKode = Math.max(...stats.top_kode.map(k => k.count), 1)
  const maxUser = Math.max(...stats.top_user.map(u => u.count), 1)

  const statusBadge = (s: string) => {
    if (s === 'validated') return <span className="text-[10px] px-2 py-0.5 rounded-full bg-emerald-950 border border-emerald-800 text-emerald-400 whitespace-nowrap">✅ valid</span>
    if (s === 'rejected') return <span className="text-[10px] px-2 py-0.5 rounded-full bg-red-950 border border-red-800 text-red-400 whitespace-nowrap">✖️ ditolak</span>
    return <span className="text-[10px] px-2 py-0.5 rounded-full bg-amber-950 border border-amber-800 text-amber-400 whitespace-nowrap">⏳ pending</span>
  }

  // Kolom Pengguna: badge nama tampilan (SRIKANDI dari extension / nama Google),
  // tooltip berisi email (bila ada). Fallback: email saja, lalu "Anonim".
  const userCell = (r: RecentFeedback) => {
    const name = (r.user_name || '').trim()
    const email = (r.user_email || '').trim()
    if (name && email) {
      return (
        <span
          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-violet-950 border border-violet-800 text-violet-300 whitespace-nowrap max-w-[160px]"
          title={`${name} · ${email}`}
        >
          <span className="truncate">👤 {name}</span>
        </span>
      )
    }
    if (name) {
      return <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-violet-950 border border-violet-800 text-violet-300 whitespace-nowrap" title={name}>👤 {name}</span>
    }
    if (email) {
      return <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-cyan-950 border border-cyan-800 text-cyan-400 whitespace-nowrap max-w-[160px]" title={email}><span className="truncate">✉️ {email}</span></span>
    }
    return <span className="text-gray-600 italic">Anonim</span>
  }

  return (
    <main className="flex-1 overflow-y-auto px-6 py-6 space-y-5">
      {/* Aksi: filter + refresh */}
      <div className="rounded-xl border border-gray-800 bg-gray-950 px-4 py-3 flex flex-wrap items-center gap-2">
        <div className="flex items-center gap-2">
          <label className="text-[10px] text-gray-500 uppercase tracking-wider">Status</label>
          <select
            value={filter.status}
            onChange={(e) => onApplyFilter({ ...filter, status: e.target.value })}
            className="text-xs bg-gray-800 border border-gray-700 rounded-lg px-2 py-1.5 text-gray-300 focus:outline-none focus:border-violet-500"
          >
            <option value="">Semua Status</option>
            <option value="validated">✅ Valid</option>
            <option value="rejected">✖️ Ditolak</option>
            <option value="pending">⏳ Pending</option>
          </select>
        </div>
        <div className="flex items-center gap-2 flex-1 min-w-[180px]">
          <label className="text-[10px] text-gray-500 uppercase tracking-wider">Perihal</label>
          <input
            value={perihalInput}
            onChange={(e) => onPerihalChange(e.target.value)}
            placeholder="Cari perihal / naskah..."
            className="flex-1 text-xs bg-gray-800 border border-gray-700 rounded-lg px-3 py-1.5 text-white placeholder-gray-600 focus:outline-none focus:border-violet-500"
          />
        </div>
        {(filter.status !== '' || filter.perihal.trim() !== '') && (
          <button
            type="button"
            onClick={onClearFilter}
            className="text-xs px-3 py-1.5 rounded-lg text-amber-400 hover:text-amber-300 transition-colors"
          >
            ✕ Reset filter
          </button>
        )}
        <button
          onClick={onRefresh}
          disabled={loading}
          className="text-xs px-3 py-1.5 rounded-lg border border-gray-700 text-gray-400 hover:text-white hover:border-gray-500 transition-colors disabled:opacity-50 ml-auto"
        >
          {loading ? 'Memuat...' : '🔄 Muat Ulang'}
        </button>
      </div>

      {(filter.status !== '' || filter.perihal.trim() !== '') && (
        <p className="text-[10px] text-gray-500">
          Menampilkan hasil <span className="text-gray-300">difilter</span>
          {filter.status && <> · status <span className="text-cyan-400">{filter.status}</span></>}
          {filter.perihal.trim() && <> · perihal mengandung <span className="text-cyan-400">“{filter.perihal.trim()}”</span></>}
        </p>
      )}

      {/* Kartu KPI */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <div className="rounded-xl border border-gray-800 bg-gray-950 p-4">
          <div className="text-[10px] text-gray-500 font-medium uppercase tracking-wider">Total Feedback</div>
          <div className="mt-1 text-2xl font-bold text-white">{stats.total}</div>
        </div>
        <div className="rounded-xl border border-emerald-900 bg-emerald-950/20 p-4">
          <div className="text-[10px] text-gray-500 font-medium uppercase tracking-wider">👍 Positif</div>
          <div className="mt-1 text-2xl font-bold text-emerald-400">{stats.positive}</div>
        </div>
        <div className="rounded-xl border border-cyan-900 bg-cyan-950/20 p-4">
          <div className="text-[10px] text-gray-500 font-medium uppercase tracking-wider">✏️ Koreksi Valid</div>
          <div className="mt-1 text-2xl font-bold text-cyan-400">{stats.correction_valid}</div>
        </div>
        <div className="rounded-xl border border-red-900 bg-red-950/20 p-4">
          <div className="text-[10px] text-gray-500 font-medium uppercase tracking-wider">✖️ Koreksi Ditolak</div>
          <div className="mt-1 text-2xl font-bold text-red-400">{stats.correction_rejected}</div>
          {pending > 0 && <div className="mt-0.5 text-[10px] text-amber-400">+ {pending} pending</div>}
        </div>
      </div>

      {/* Grafik top kode & top pengguna */}
      <div className="grid sm:grid-cols-2 gap-4">
        <div className="rounded-xl border border-gray-800 bg-gray-950 p-4 space-y-3">
          <h3 className="text-xs font-semibold text-gray-300 uppercase tracking-wider">Top Kode Hasil Koreksi</h3>
          {stats.top_kode.length === 0 && <p className="text-xs text-gray-600">Belum ada koreksi yang tervalidasi.</p>}
          {stats.top_kode.map(k => (
            <div key={k.kode} className="space-y-1">
              <div className="flex items-center justify-between text-xs">
                <span className="font-mono text-cyan-400">{k.kode}</span>
                <span className="text-gray-500">{k.count}×</span>
              </div>
              <div className="h-2 rounded-full bg-gray-800 overflow-hidden">
                <div
                  className="h-full rounded-full bg-gradient-to-r from-cyan-700 to-cyan-400 transition-all duration-500"
                  style={{ width: `${(k.count / maxKode) * 100}%` }}
                />
              </div>
            </div>
          ))}
        </div>
        <div className="rounded-xl border border-gray-800 bg-gray-950 p-4 space-y-3">
          <h3 className="text-xs font-semibold text-gray-300 uppercase tracking-wider">Pengguna Teraktif</h3>
          {stats.top_user.length === 0 && <p className="text-xs text-gray-600">Belum ada feedback.</p>}
          {stats.top_user.map(u => (
            <div key={u.user} className="space-y-1">
              <div className="flex items-center justify-between text-xs gap-2">
                <span className="inline-flex items-center gap-1 text-gray-300 truncate" title={u.user}>
                  <span className="shrink-0">{u.user === 'anonim' ? '👻' : '👤'}</span>
                  <span className="truncate">{u.user}</span>
                </span>
                <span className="text-gray-500 shrink-0">{u.count}×</span>
              </div>
              <div className="h-2 rounded-full bg-gray-800 overflow-hidden">
                <div
                  className="h-full rounded-full bg-gradient-to-r from-violet-700 to-violet-400 transition-all duration-500"
                  style={{ width: `${(u.count / maxUser) * 100}%` }}
                />
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Tabel feedback terbaru */}
      <div className="rounded-xl border border-gray-800 bg-gray-950 overflow-hidden">
        <div className="px-4 py-3 border-b border-gray-800 flex items-center justify-between">
          <h3 className="text-xs font-semibold text-gray-300 uppercase tracking-wider">Feedback Terbaru</h3>
          <span className="text-[10px] text-gray-600">maks. 20 entri{canDelete ? ' · hapus butuh password secret (admin)' : ''}</span>
        </div>
        <div className="overflow-x-auto">
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left text-gray-500 border-b border-gray-800">
                <th className="px-4 py-2 font-medium whitespace-nowrap">Waktu</th>
                <th className="px-4 py-2 font-medium whitespace-nowrap">Pengguna</th>
                <th className="px-4 py-2 font-medium">Perihal / Naskah</th>
                <th className="px-4 py-2 font-medium whitespace-nowrap">Kode AI</th>
                <th className="px-4 py-2 font-medium whitespace-nowrap">Koreksi</th>
                <th className="px-4 py-2 font-medium whitespace-nowrap">Status</th>
                <th className="px-4 py-2 font-medium">Catatan Validasi</th>
                {canDelete && <th className="px-4 py-2 font-medium whitespace-nowrap">Aksi</th>}
              </tr>
            </thead>
            <tbody>
              {stats.recent.map(r => (
                <tr key={r.id} className="border-b border-gray-800/60 last:border-0 hover:bg-gray-900/60 transition-colors">
                  <td className="px-4 py-2.5 text-gray-500 whitespace-nowrap">{r.waktu}</td>
                  <td className="px-4 py-2.5">{userCell(r)}</td>
                  <td className="px-4 py-2.5 text-gray-400 max-w-[220px] truncate" title={r.perihal || r.naskah}>{(r.perihal || r.naskah) || '—'}</td>
                  <td className="px-4 py-2.5 font-mono text-cyan-400 whitespace-nowrap">{r.kode_ai}</td>
                  <td className="px-4 py-2.5">
                    {r.feedback_type === 'correction'
                      ? <span className="font-mono text-amber-400 whitespace-nowrap">{r.kode_koreksi || '—'}</span>
                      : <span className="text-gray-600">—</span>}
                  </td>
                  <td className="px-4 py-2.5">{statusBadge(r.status)}</td>
                  <td className="px-4 py-2.5 text-gray-400 max-w-[240px] truncate" title={r.penjelasan}>{r.penjelasan || '—'}</td>
                  {canDelete && (
                    <td className="px-4 py-2.5">
                      <button
                        type="button"
                        onClick={() => onDeleteClick(r)}
                        title="Hapus feedback (admin)"
                        className="p-1.5 rounded-lg text-gray-600 hover:text-red-400 hover:bg-red-950/40 transition-colors"
                      >
                        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="m14.74 9-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 0 1-2.244 2.077H8.084a2.25 2.25 0 0 1-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 0 0-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 0 1 3.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 0 0-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 0 0-7.5 0" />
                        </svg>
                      </button>
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </main>
  )
}

function App() {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [loading, setLoading] = useState(false)
  const [apiAvailable, setApiAvailable] = useState<boolean | null>(null)
  const [cooldown, setCooldown] = useState<number | null>(null)
  const [userApiKeys, setUserApiKeys] = useState<string[]>(() => loadSavedKeys())
  const saveKeys = (keys: string[]) => {
    setUserApiKeys(keys)
    localStorage.setItem(KEYS_STORAGE, JSON.stringify(keys))
  }
  const chatEndRef = useRef<HTMLDivElement>(null)
  const cooldownRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [copiedKode, setCopiedKode] = useState<string | null>(null)
  const copiedTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [quota, setQuota] = useState<QuotaStats | null>(null)
  const [authConfig, setAuthConfig] = useState<AuthConfig | null>(null)
  const [token, setToken] = useState(() => localStorage.getItem('kk_token') || '')
  const [user, setUser] = useState<AuthUser | null>(() => {
    try { return JSON.parse(localStorage.getItem('kk_user') || 'null') } catch { return null }
  })
  const [feedbackMap, setFeedbackMap] = useState<Record<number, FeedbackState>>({})
  const [correctionForm, setCorrectionForm] = useState<Record<number, CorrectionFormState>>({})
  const kodeSearchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [view, setView] = useState<'chat' | 'stats' | 'settings'>('chat')
  const [stats, setStats] = useState<FeedbackStats | null>(null)
  const [statsLoading, setStatsLoading] = useState(false)
  const [statsFilter, setStatsFilter] = useState<StatsFilter>({ status: '', perihal: '' })
  const [isAdmin, setIsAdmin] = useState(() => localStorage.getItem('kk_is_admin') === 'true')
  const [deleteTarget, setDeleteTarget] = useState<RecentFeedback | null>(null)
  const [deletePassword, setDeletePassword] = useState('')
  const [deleting, setDeleting] = useState(false)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [deleteLockout, setDeleteLockout] = useState<number | null>(null)
  const deleteLockoutRef = useRef<ReturnType<typeof setInterval> | null>(null)
  // VITE_API_URL diisi saat dev (mis. http://localhost:3100). Bila kosong (build
  // statis via nginx) → relatif ('') sehingga /api/* lewat proxy nginx (same-origin).
  const API_BASE = (import.meta.env.VITE_API_URL as string) || ''

  // Keluar: hapus sesi lokal. Didefinisikan lebih awal (useCallback stabil)
  // agar bisa dipakai oleh fetchQuota/fetchStats saat token kedaluwarsa (401)
  // tanpa merusak memoization interval polling.
  const logout = useCallback(() => {
    localStorage.removeItem('kk_token')
    localStorage.removeItem('kk_user')
    localStorage.removeItem('kk_is_admin')
    setToken('')
    setUser(null)
    setIsAdmin(false)
    setView('chat')
  }, [])

  useEffect(() => {
    fetch(`${API_BASE}/api/health`)
      .then(r => r.json())
      .then(() => setApiAvailable(true))
      .catch(() => setApiAvailable(false))
  }, [])

  // Kuota free Gemini (RPM/RPD) — diambil saat load + polling tiap 30 detik
  const fetchQuota = useCallback(async () => {
    try {
      const r = await fetch(`${API_BASE}/api/quota`)
      if (r.ok) setQuota(await r.json())
      else if (r.status === 401) logout() // defensif: sesi kedaluwarsa → layar login
    } catch { /* server offline */ }
  }, [API_BASE, logout])

  useEffect(() => {
    fetchQuota()
    const t = setInterval(fetchQuota, 30000)
    return () => clearInterval(t)
  }, [fetchQuota])

  // Konfigurasi auth
  useEffect(() => {
    fetch(`${API_BASE}/api/auth/config`)
      .then(r => r.json())
      .then((c: AuthConfig) => setAuthConfig(c))
      .catch(() => {})
  }, [API_BASE])

  // Callback Google OAuth: tukar code → token JWT
  useEffect(() => {
    if (window.location.pathname !== '/auth/callback') return
    const params = new URLSearchParams(window.location.search)
    const code = params.get('code')
    const state = params.get('state')
    const stored = sessionStorage.getItem('kk_oauth_state')
    if (code && state && state === stored) {
      const verifier = sessionStorage.getItem('kk_oauth_verifier') || ''
      // Redirect URI yang tadi dipakai saat membuka URL login (harus sama saat tukar code)
      const redirect_uri = sessionStorage.getItem('kk_oauth_redirect') || ''
      fetch(`${API_BASE}/api/auth/google`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code, code_verifier: verifier, redirect_uri })
      }).then(r => r.json()).then((res: { token?: string; user?: AuthUser }) => {
        if (res.token && res.user) {
          localStorage.setItem('kk_token', res.token)
          localStorage.setItem('kk_user', JSON.stringify(res.user))
          setToken(res.token)
          setUser(res.user)
        }
      }).catch(() => {})
    }
    sessionStorage.removeItem('kk_oauth_state')
    sessionStorage.removeItem('kk_oauth_verifier')
    sessionStorage.removeItem('kk_oauth_redirect')
    window.history.replaceState({}, '', '/')
  }, [API_BASE])

  const authHeaders = useCallback((json = true): HeadersInit => {
    const h: Record<string, string> = {}
    if (json) h['Content-Type'] = 'application/json'
    if (token) h['Authorization'] = `Bearer ${token}`
    return h
  }, [token])

  const fetchStats = useCallback(async (f?: StatsFilter) => {
    const filter = f ?? statsFilter
    setStatsLoading(true)
    try {
      const params = new URLSearchParams()
      if (filter.status) params.set('status', filter.status)
      if (filter.perihal.trim()) params.set('perihal', filter.perihal.trim())
      const qs = params.toString()
      const r = await fetch(`${API_BASE}/api/feedback/stats${qs ? `?${qs}` : ''}`, { headers: authHeaders(false) })
      if (r.ok) setStats(await r.json())
      else if (r.status === 401) logout() // sesi kedaluwarsa → layar login
    } catch { /* server offline */ }
    setStatsLoading(false)
  }, [API_BASE, logout, statsFilter, authHeaders])

  const applyStatsFilter = (f: StatsFilter) => {
    setStatsFilter(f)
    fetchStats(f)
  }

  const clearStatsFilter = () => applyStatsFilter({ status: '', perihal: '' })

  // Sinkronkan status admin dari server (untuk tombol hapus feedback).
  // Bila token tidak valid/kedaluwarsa (401) → bersihkan sesi basi di localStorage
  // agar UI jujur (chat tetap bisa dipakai tanpa login).
  useEffect(() => {
    if (!token) return
    fetch(`${API_BASE}/api/me`, { headers: { Authorization: `Bearer ${token}` } })
      .then(r => {
        if (!r.ok) {
          if (r.status === 401) logout()
          return null
        }
        return r.json()
      })
      .then((u: (AuthUser & { is_admin?: boolean }) | null) => {
        if (u) {
          localStorage.setItem('kk_is_admin', String(!!u.is_admin))
          setIsAdmin(!!u.is_admin)
        }
      })
      .catch(() => {})
  }, [token, API_BASE, logout])

  const startDeleteLockout = useCallback((seconds: number) => {
    setDeleteLockout(seconds)
    if (deleteLockoutRef.current) clearInterval(deleteLockoutRef.current)
    deleteLockoutRef.current = setInterval(() => {
      setDeleteLockout(prev => {
        if (prev === null || prev <= 1) {
          if (deleteLockoutRef.current) clearInterval(deleteLockoutRef.current)
          return null
        }
        return prev - 1
      })
    }, 1000)
  }, [])

  const closeDeleteModal = useCallback(() => {
    setDeleteTarget(null)
    setDeletePassword('')
    setDeleteError(null)
    if (deleteLockoutRef.current) clearInterval(deleteLockoutRef.current)
    setDeleteLockout(null)
  }, [])

  const confirmDelete = async () => {
    if (!deleteTarget || !deletePassword.trim() || deleteLockout !== null) return
    setDeleting(true)
    setDeleteError(null)
    try {
      const r = await fetch(`${API_BASE}/api/feedback/${deleteTarget.id}`, {
        method: 'DELETE',
        headers: authHeaders(),
        body: JSON.stringify({ password: deletePassword })
      })
      if (r.ok) {
        closeDeleteModal()
        fetchStats()
      } else {
        const err = await r.json().catch(() => null) as ErrorResponse | null
        if (r.status === 401) {
          logout()
          setDeleteTarget(null)
          return
        }
        if (r.status === 429 && err?.retry_after_secs) {
          // Terkunci anti brute-force → tampilkan countdown sisa waktu
          startDeleteLockout(err.retry_after_secs)
        } else {
          setDeleteError(err?.error || `Gagal menghapus (HTTP ${r.status})`)
        }
      }
    } catch {
      setDeleteError('Gagal terhubung ke server')
    }
    setDeleting(false)
  }

  const login = async () => {
    if (!authConfig?.enabled) return
    const { verifier, challenge } = await pkcePair()
    const state = base64UrlEncode(crypto.getRandomValues(new Uint8Array(16)))
    sessionStorage.setItem('kk_oauth_verifier', verifier)
    sessionStorage.setItem('kk_oauth_state', state)
    // Pilih redirect URI yang cocok dengan origin saat ini (localhost vs domain publik),
    // fallback ke URI pertama. Disimpan agar sama saat tukar code di callback.
    const redirect_uri = (authConfig.redirect_uris.find(u => u.startsWith(window.location.origin))
      || authConfig.redirect_uris[0]
      || '').trim()
    sessionStorage.setItem('kk_oauth_redirect', redirect_uri)
    const url = new URL('https://accounts.google.com/o/oauth2/v2/auth')
    url.searchParams.set('client_id', authConfig.client_id)
    url.searchParams.set('redirect_uri', redirect_uri)
    url.searchParams.set('response_type', 'code')
    url.searchParams.set('scope', 'openid email profile')
    url.searchParams.set('code_challenge', challenge)
    url.searchParams.set('code_challenge_method', 'S256')
    url.searchParams.set('state', state)
    window.location.href = url.toString()
  }

  // ---------- Feedback ----------

  const sendPositive = async (msgIdx: number, msg: Message) => {
    if (!msg.query || !msg.results?.[0]) return
    setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'positive', sending: true } }))
    try {
      const resp = await fetch(`${API_BASE}/api/feedback`, {
        method: 'POST',
        headers: authHeaders(),
        body: JSON.stringify({
          message: msg.query,
          kode_ai: msg.results[0].kode,
          feedback_type: 'positive',
          perihal: msg.perihal || '',
          api_keys: userApiKeys.length ? userApiKeys : undefined,
          chat_id: CHAT_ID
        })
      })
      if (!resp.ok) {
        const err = await resp.json().catch(() => null) as ErrorResponse | null
        if (resp.status === 401) logout()
        setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'positive', result: { valid: false, kode_terbaik: null, penjelasan: err?.error || `Gagal mengirim feedback (HTTP ${resp.status}).` } } }))
        return
      }
      const data: FeedbackResult = await resp.json()
      setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'positive', result: data } }))
    } catch {
      setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'positive', result: { valid: false, kode_terbaik: null, penjelasan: 'Gagal mengirim feedback.' } } }))
    }
  }

  const searchKode = (msgIdx: number, q: string) => {
    // Ketikan baru membatalkan kode yang sedang dipilih (info deskripsi/path jadi basi)
    setCorrectionForm(prev => ({ ...prev, [msgIdx]: { ...(prev[msgIdx] || { kode: '', alasan: '' }), kode: q, search: q, loading: true, selected: null } }))
    if (kodeSearchTimerRef.current) clearTimeout(kodeSearchTimerRef.current)
    if (q.trim().length < 2) {
      setCorrectionForm(prev => ({ ...prev, [msgIdx]: { ...(prev[msgIdx] || { kode: '', alasan: '' }), suggestions: [], loading: false, selected: null } }))
      return
    }
    // Debounce 250ms agar tidak membanjiri server per ketikan
    kodeSearchTimerRef.current = setTimeout(async () => {
      try {
        const resp = await fetch(`${API_BASE}/api/codes?q=${encodeURIComponent(q)}`, { headers: authHeaders(false) })
        if (resp.ok) {
          const suggestions: { kode: string; deskripsi: string; path: string }[] = await resp.json()
          setCorrectionForm(prev => ({ ...prev, [msgIdx]: { ...(prev[msgIdx] || { kode: '', alasan: '' }), suggestions, loading: false } }))
        }
      } catch { /* server offline */ }
    }, 250)
  }

  const submitCorrection = async (msgIdx: number, msg: Message) => {
    const form = correctionForm[msgIdx]
    if (!form?.kode.trim() || !msg.query || !msg.results?.length) return
    // Koreksi wajib login (akuntabilitas) — pengaman ganda selain validasi backend
    if (!user && authConfig?.enabled) {
      setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'correction', result: { valid: false, kode_terbaik: null, penjelasan: '🔐 Silakan login terlebih dahulu untuk mengirim koreksi.' } } }))
      return
    }
    setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'correction', sending: true } }))
    try {
      const resp = await fetch(`${API_BASE}/api/feedback`, {
        method: 'POST',
        headers: authHeaders(),
        body: JSON.stringify({
          message: msg.query,
          kode_ai: msg.results[0].kode,
          feedback_type: 'correction',
          kode_koreksi: form.kode.trim(),
          alasan: form.alasan,
          perihal: msg.perihal || '',
          api_keys: userApiKeys.length ? userApiKeys : undefined,
          candidates: msg.results.slice(0, 3).map(r => ({ kode: r.kode, deskripsi: r.deskripsi, path: r.path })),
          chat_id: CHAT_ID
        })
      })
      if (!resp.ok) {
        const err = await resp.json().catch(() => null) as ErrorResponse | null
        if (resp.status === 401) logout()
        setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'correction', result: { valid: false, kode_terbaik: null, penjelasan: err?.error || `Gagal mengirim koreksi (HTTP ${resp.status}).` } } }))
        return
      }
      const data: FeedbackResult = await resp.json()
      setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'correction', result: data } }))
      setCorrectionForm(prev => ({ ...prev, [msgIdx]: { ...(prev[msgIdx] || { kode: '', alasan: '' }), suggestions: [] } }))
    } catch {
      setFeedbackMap(prev => ({ ...prev, [msgIdx]: { type: 'correction', result: { valid: false, kode_terbaik: null, penjelasan: 'Gagal mengirim koreksi.' } } }))
    }
  }

  useEffect(() => {
    chatEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

  // Cleanup timer
  useEffect(() => {
    return () => {
      if (cooldownRef.current) clearInterval(cooldownRef.current)
      if (copiedTimeoutRef.current) clearTimeout(copiedTimeoutRef.current)
      if (kodeSearchTimerRef.current) clearTimeout(kodeSearchTimerRef.current)
      if (deleteLockoutRef.current) clearInterval(deleteLockoutRef.current)
    }
  }, [])

  const startCooldown = useCallback((seconds: number) => {
    setCooldown(seconds)
    if (cooldownRef.current) clearInterval(cooldownRef.current)
    cooldownRef.current = setInterval(() => {
      setCooldown(prev => {
        if (prev === null || prev <= 1) {
          if (cooldownRef.current) clearInterval(cooldownRef.current)
          return null
        }
        return prev - 1
      })
    }, 1000)
  }, [])

  const handleFileUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return

    try {
      let text = ''
      const ext = file.name.split('.').pop()?.toLowerCase()

      if (ext === 'pdf') {
        // Prioritaskan ekstraksi via backend (poppler): benar untuk PDF SRIKANDI
        // yang ToUnicode-nya rusak (pdf.js menghasilkan karakter garbled).
        const fd = new FormData()
        fd.append('file', file)
        try {
          const r = await fetch(`${API_BASE}/api/extract-pdf`, { method: 'POST', headers: authHeaders(false), body: fd })
          if (r.ok) {
            const j = await r.json()
            if (j.text && j.text.trim().length > 0) {
              text = j.text
            }
          }
        } catch { /* backend tak terjangkau — lanjut fallback pdf.js */ }

        if (!text) {
          const arrayBuffer = await file.arrayBuffer()
          const pdf = await pdfjsLib.getDocument({ data: new Uint8Array(arrayBuffer) }).promise
          const pages: string[] = []
          for (let i = 1; i <= pdf.numPages; i++) {
            const page = await pdf.getPage(i)
            const content = await page.getTextContent()
            const pageText = content.items
              .map((item: any) => 'str' in item ? item.str : '')
              .join(' ')
            pages.push(pageText)
          }
          text = pages.join('\n')
        }
      } else if (ext === 'docx') {
        const arrayBuffer = await file.arrayBuffer()
        const result = await mammoth.extractRawText({ arrayBuffer })
        text = result.value
      } else {
        setMessages(prev => [...prev, {
          role: 'assistant',
          content: '❌ Format file tidak didukung. Gunakan PDF atau DOCX saja.',
          isRateLimit: false
        }])
        e.target.value = ''
        return
      }

      const cleaned = text.replace(/\n{3,}/g, '\n\n').replace(/\s{2,}/g, ' ').trim()
      if (!cleaned || cleaned.length < 5) {
        setMessages(prev => [...prev, {
          role: 'assistant',
          content: '❌ Tidak ada teks yang berhasil diekstrak dari file.',
          isRateLimit: false
        }])
        e.target.value = ''
        return
      }

      setInput(cleaned)
      // Tampilkan pesan bahwa file sedang dianalisa
      setMessages(prev => [...prev, {
        role: 'assistant',
        content: `📄 File "${file.name}" berhasil diekstrak ke area chat, silahkan kirim untuk mencari kode klasifikasi`,
        isRateLimit: false
      }])
      e.target.value = ''
    } catch {
      setMessages(prev => [...prev, {
        role: 'assistant',
        content: '❌ Gagal mengekstrak teks dari file.',
        isRateLimit: false
      }])
    }
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!input.trim() || loading || cooldown !== null) return

    const userMsg: Message = { role: 'user', content: input }
    setMessages(prev => [...prev, userMsg])
    setInput('')
    setLoading(true)
    const startTime = Date.now()

    try {
      const resp = await fetch(`${API_BASE}/api/chat`, {
        method: 'POST',
        headers: authHeaders(),
        body: JSON.stringify({ message: userMsg.content, api_keys: userApiKeys.length ? userApiKeys : undefined })
      })

      if (resp.status === 429) {
        const err: ErrorResponse = await resp.json()
        const wait = err.retry_after_secs || 12
        setMessages(prev => [...prev, {
          role: 'assistant',
          content: `⏳ ${err.error}`,
          isRateLimit: true
        }])
        startCooldown(wait)
        fetchQuota()
        setLoading(false)
        return
      }

      // Tangani status selain 200 (401 sesi berakhir, 400/500, dll).
      // Tanpa ini, respons error {error: ...} di-parse sebagai ChatResponse
      // dan field explanation yang tidak ada tampil sebagai "undefined".
      if (!resp.ok) {
        const err = await resp.json().catch(() => null) as ErrorResponse | null
        if (resp.status === 401) {
          // Token kedaluwarsa/tidak valid → reset sesi agar muncul layar login
          logout()
          setMessages(prev => [...prev, {
            role: 'assistant',
            content: '🔐 Sesi login sudah berakhir. Silakan masuk ulang dengan akun Google.',
            isRateLimit: false
          }])
        } else {
          setMessages(prev => [...prev, {
            role: 'assistant',
            content: `❌ ${err?.error || `Terjadi kesalahan (HTTP ${resp.status})`}`,
            isRateLimit: false
          }])
        }
        setLoading(false)
        return
      }

      const data: ChatResponse = await resp.json()
      // Keterangan waktu proses (end-to-end: klik kirim → respons diterima)
      const elapsedSec = (Date.now() - startTime) / 1000
      const elapsedText = elapsedSec.toFixed(1).replace('.', ',')
      const assistantMsg: Message = {
        role: 'assistant',
        content: `${data.explanation}\n\n⏱️ Diproses dalam ${elapsedText} detik`,
        results: data.results?.slice(0, 3),
        query: userMsg.content,
        perihal: data.perihal || ''
      }
      setMessages(prev => [...prev, assistantMsg])
      fetchQuota()
    } catch {
      setMessages(prev => [...prev, {
        role: 'assistant',
        content: 'Maaf, terjadi kesalahan koneksi ke server.',
        isRateLimit: false
      }])
    } finally {
      setLoading(false)
    }
  }

  const copyKode = async (kode: string) => {
    try {
      await navigator.clipboard.writeText(kode)
    } catch {
      // Fallback untuk non-secure context (execCommand)
      const ta = document.createElement('textarea')
      ta.value = kode
      ta.style.position = 'fixed'
      ta.style.opacity = '0'
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      document.body.removeChild(ta)
    }
    setCopiedKode(kode)
    if (copiedTimeoutRef.current) clearTimeout(copiedTimeoutRef.current)
    copiedTimeoutRef.current = setTimeout(() => setCopiedKode(null), 1500)
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSubmit(e as unknown as React.FormEvent)
    }
  }

  const isInputDisabled = loading || cooldown !== null

  // Chat & feedback positif terbuka untuk semua (tanpa login). Login bersifat
  // OPSIONAL — dibutuhkan hanya untuk mengirim KOREKSI & menghapus feedback (admin).
  return (
    <div className="flex h-screen bg-gray-900">
      {/* Sidebar kiri: brand, menu, akun */}
      <aside className="shrink-0 w-14 md:w-60 bg-gray-950 border-r border-gray-800 flex flex-col">
        {/* Brand */}
        <div className="px-3 md:px-4 py-4 flex items-center gap-3 border-b border-gray-800">
          <div className="w-9 h-9 shrink-0 rounded-lg bg-gradient-to-br from-violet-500 to-blue-600 flex items-center justify-center text-sm font-bold">
            K
          </div>
          <div className="hidden md:block min-w-0">
            <h1 className="text-sm font-semibold text-white leading-tight truncate">Kode Klasifikasi Arsip</h1>
            <span className="text-[10px] text-gray-500">AI Arsiparis · Semantic</span>
          </div>
        </div>

        {/* Navigasi utama */}
        <nav className="flex-1 px-2 md:px-3 py-4 space-y-1">
          <button
            type="button"
            onClick={() => setView('chat')}
            title="Chat"
            className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition-colors ${
              view === 'chat'
                ? 'bg-violet-950/60 text-violet-300 border border-violet-800/50'
                : 'text-gray-400 hover:bg-gray-900 hover:text-white border border-transparent'
            }`}
          >
            <span className="text-base">💬</span>
            <span className="hidden md:inline">Chat</span>
          </button>
          <button
            type="button"
            onClick={() => { setView('stats'); fetchStats() }}
            title="Statistik"
            className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition-colors ${
              view === 'stats'
                ? 'bg-violet-950/60 text-violet-300 border border-violet-800/50'
                : 'text-gray-400 hover:bg-gray-900 hover:text-white border border-transparent'
            }`}
          >
            <span className="text-base">📊</span>
            <span className="hidden md:inline">Statistik</span>
          </button>
          <button
            type="button"
            onClick={() => setView('settings')}
            title="Pengaturan"
            className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition-colors ${
              view === 'settings'
                ? 'bg-violet-950/60 text-violet-300 border border-violet-800/50'
                : 'text-gray-400 hover:bg-gray-900 hover:text-white border border-transparent'
            }`}
          >
            <span className="text-base">⚙️</span>
            <span className="hidden md:inline">Pengaturan</span>
          </button>
        </nav>

        {/* Kuota, API key, pengguna */}
        <div className="px-2 md:px-3 py-4 border-t border-gray-800 space-y-3">
          {quota?.enabled && (
            <div
              title={`Kuota free Gemini\nChat: ⚡ ${quota.chat.rpm_used}/${quota.chat.rpm_limit} RPM · 📅 ${quota.chat.rpd_used}/${quota.chat.rpd_limit} RPD\nEmbed: ⚡ ${quota.embed.rpm_used}/${quota.embed.rpm_limit} RPM · 📅 ${quota.embed.rpd_used}/${quota.embed.rpd_limit} RPD`}
              className={`hidden md:flex text-[10px] px-2.5 py-1.5 rounded-lg border items-center justify-center transition-colors ${
                quota.overall_pct >= 90
                  ? 'bg-red-950 border-red-800 text-red-400'
                  : quota.overall_pct >= 70
                    ? 'bg-amber-950 border-amber-800 text-amber-400'
                    : 'bg-emerald-950 border-emerald-800 text-emerald-400'
              }`}
            >
              📅 {quota.chat.rpd_used}/{quota.chat.rpd_limit} · ⚡ {quota.chat.rpm_used}/{quota.chat.rpm_limit}
            </div>
          )}
          {user && (
            <div className="flex items-center gap-2">
              <div className="w-8 h-8 shrink-0 rounded-full bg-violet-600 flex items-center justify-center text-xs font-bold">
                {(user.name || user.email || '?')[0].toUpperCase()}
              </div>
              <div className="hidden md:block min-w-0 flex-1">
                <div className="text-xs text-gray-300 truncate">{user.name}</div>
                <div className="text-[10px] text-gray-500 truncate">{user.email}</div>
              </div>
              <button
                onClick={logout}
                title="Keluar"
                className="shrink-0 flex items-center gap-1 text-xs text-gray-500 hover:text-red-400 transition-colors px-2 py-1.5 rounded-lg hover:bg-gray-900"
              >
                <span>⏻</span>
                <span className="hidden md:inline">Keluar</span>
              </button>
            </div>
          )}

          {/* Login OPSIONAL — dibutuhkan untuk kirim koreksi & hapus feedback (admin) */}
          {!user && authConfig?.enabled && (
            <button
              onClick={login}
              title="Masuk dengan Google (opsional)"
              className="w-full flex items-center justify-center md:justify-start gap-2.5 px-2.5 py-2 rounded-lg border border-gray-700 text-gray-300 hover:text-white hover:border-violet-600 hover:bg-gray-900 transition-colors"
            >
              <svg className="w-4 h-4 shrink-0" viewBox="0 0 24 24">
                <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.27-4.74 3.27-8.1z" />
                <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" />
                <path fill="#FBBC05" d="M5.84 14.1c-.22-.66-.35-1.36-.35-2.1s.13-1.44.35-2.1V7.06H2.18A10.96 10.96 0 0 0 1 12c0 1.77.43 3.45 1.18 4.94l3.66-2.84z" />
                <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.06l3.66 2.84c.87-2.6 3.3-4.52 6.16-4.52z" />
              </svg>
              <span className="hidden md:inline text-xs font-medium">Masuk dengan Google</span>
              <span className="hidden md:inline text-[10px] text-gray-500 ml-1">opsional</span>
            </button>
          )}
        </div>
      </aside>

      {/* Area utama */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Top bar ringkas */}
        <header className="shrink-0 px-5 py-3 bg-gray-950 border-b border-gray-800 flex items-center gap-3">
          <div className="flex flex-col gap-0.5 min-w-0">
            <h1 className="text-sm font-semibold text-white leading-tight">
              {view === 'stats' ? '📊 Statistik Feedback' : view === 'settings' ? '⚙️ Pengaturan API Key' : '💬 Chat Asisten'}
            </h1>
            <span className="hidden sm:block text-[10px] text-gray-500">
              {view === 'stats'
                ? 'Umpan balik pengguna terhadap hasil klasifikasi'
                : view === 'settings'
                  ? 'Kelola API Key Gemini — multi-key dengan rotasi otomatis'
                  : 'Cari kode klasifikasi arsip dengan AI'}
            </span>
          </div>
          <div className="ml-auto flex items-center gap-2">
          {cooldown !== null && (
            <span className="text-xs px-2.5 py-1 rounded-full bg-amber-950 border border-amber-800 text-amber-400">
              ⏳ {formatDuration(cooldown)}
            </span>
          )}
          {apiAvailable === null && (
            <span className="text-xs px-2.5 py-1 rounded-full bg-gray-800 text-gray-400">Menghubungkan...</span>
          )}
          {apiAvailable === true && (
            <span className="text-xs px-2.5 py-1 rounded-full bg-emerald-950 border border-emerald-800 text-emerald-400">Terhubung</span>
          )}
          {apiAvailable === false && (
            <span className="text-xs px-2.5 py-1 rounded-full bg-red-950 border border-red-800 text-red-400">Server Offline</span>
          )}
        </div>
      </header>

      {/* Chat selalu di-mount (hanya disembunyikan saat di Statistik) agar sesi,
          posisi scroll, dan form koreksi yang sedang terbuka tetap terjaga */}
      <div className={`flex-1 flex flex-col min-h-0 ${view === 'stats' || view === 'settings' ? 'hidden' : ''}`}>
      {/* Chat Area */}
      <main className="flex-1 overflow-y-auto px-6 py-5 space-y-5">
        {messages.length === 0 && (
          <div className="flex flex-col items-center justify-center h-full text-center max-w-md mx-auto gap-6">
            <div className="w-16 h-16 rounded-2xl bg-gradient-to-br from-violet-500/20 to-blue-600/20 border border-violet-500/30 flex items-center justify-center">
              <svg className="w-8 h-8 text-violet-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="m21 21-5.197-5.197m0 0A7.5 7.5 0 1 0 5.196 5.196a7.5 7.5 0 0 0 10.607 10.607Z" />
              </svg>
            </div>
            <div className="space-y-2">
              <h2 className="text-xl font-semibold text-white">
                Asisten Kode Klasifikasi Arsip
              </h2>
              <p className="text-sm text-gray-400 leading-relaxed">
                Masukkan <strong className="text-gray-200">perihal</strong> naskah dinas,
                atau <strong className="text-gray-200">upload file</strong> PDF/DOCX,{' '}
                dan AI akan mencari kode klasifikasi paling sesuai.
              </p>
              {authConfig?.enabled && (
                <p className="text-[10px] text-gray-600">
                  Bisa dipakai tanpa login · 👍 feedback positif anonim · ✏️ koreksi butuh login
                </p>
              )}
            </div>
            <div className="w-full space-y-2">
              <p className="text-xs text-gray-500 font-medium uppercase tracking-wider">Contoh</p>
              {EXAMPLE_QUERIES.map(q => (
                <button
                  key={q}
                  onClick={() => setInput(q)}
                  className="w-full text-left px-4 py-2.5 rounded-xl border border-gray-800 bg-gray-900 hover:bg-gray-800 hover:border-gray-700 text-sm text-gray-300 transition-colors"
                >
                  {q}
                </button>
              ))}
            </div>
          </div>
        )}

        {messages.map((msg, i) => (
          <div key={i} className={`flex gap-3 ${msg.role === 'user' ? 'flex-row-reverse' : 'flex-row'}`}>
            <div className={`shrink-0 w-8 h-8 rounded-full flex items-center justify-center text-xs font-bold ${
              msg.role === 'user' ? 'bg-blue-600 text-white' : msg.isRateLimit ? 'bg-amber-600 text-white' : 'bg-violet-600 text-white'
            }`}>
              {msg.role === 'user' ? 'A' : msg.isRateLimit ? '⏳' : 'AI'}
            </div>

            <div className={`max-w-[80%] space-y-3 ${msg.role === 'user' ? 'items-end' : 'items-start'}`}>
              <div className={`rounded-2xl px-4 py-3 text-sm leading-relaxed ${
                msg.role === 'user'
                  ? 'bg-blue-600 text-white rounded-br-md'
                  : msg.isRateLimit
                    ? 'bg-amber-900/50 border border-amber-800 text-amber-200 rounded-bl-md'
                    : 'bg-gray-800 text-gray-200 rounded-bl-md'
              }`}>
                {msg.content}
              </div>

              {msg.results && msg.results.length > 0 && (
                <div className="bg-gray-850 rounded-xl border border-gray-700 overflow-hidden">
                  {msg.results.map((r, j) => (
                    <div key={j} className={`px-4 py-3 ${
                      j === 0 ? 'bg-violet-950/30' : ''
                    }`}>
                      <div className="flex items-center gap-3">
                        <span className="shrink-0 font-mono font-semibold text-cyan-400 text-xs">
                          {r.kode}
                        </span>
                        <span className="flex-1 text-gray-300">{r.deskripsi}</span>
                        <button
                          type="button"
                          onClick={() => copyKode(r.kode)}
                          title={copiedKode === r.kode ? 'Tersalin!' : 'Salin kode klasifikasi'}
                          className={`shrink-0 p-1.5 rounded-lg transition-all duration-200 ${
                            copiedKode === r.kode
                              ? 'text-emerald-400 bg-emerald-950/40 scale-110'
                              : 'text-gray-500 hover:text-cyan-400 hover:bg-gray-700/60 active:scale-90'
                          }`}
                        >
                          {copiedKode === r.kode ? (
                            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                              <path strokeLinecap="round" strokeLinejoin="round" d="m5 13 4 4L19 7" />
                            </svg>
                          ) : (
                            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                              <path strokeLinecap="round" strokeLinejoin="round" d="M15.666 3.888A2.25 2.25 0 0 0 13.5 2.25h-3c-1.03 0-1.9.693-2.166 1.638m7.332 0c.055.194.084.4.084.612v0a.75.75 0 0 1-.75.75H9a.75.75 0 0 1-.75-.75v0c0-.212.03-.418.084-.612m7.332 0c.646.049 1.288.11 1.927.184 1.1.128 1.907 1.077 1.907 2.185V19.5a2.25 2.25 0 0 1-2.25 2.25H6.75A2.25 2.25 0 0 1 4.5 19.5V6.257c0-1.108.806-2.057 1.907-2.185a48.208 48.208 0 0 1 1.927-.184" />
                            </svg>
                          )}
                        </button>
                      </div>
                      <div className="mt-1 ml-0 text-xs text-gray-500">
                        {r.path}
                      </div>
                    </div>
                  ))}
                </div>
              )}

              {/* Feedback: 👍 atau ✏️ Koreksi (divalidasi AI) */}
              {msg.role === 'assistant' && msg.results && msg.results.length > 0 && msg.query && (
                <div className="flex flex-wrap items-center gap-2">
                  {!feedbackMap[i]?.result ? (
                    <>
                      <button
                        type="button"
                        onClick={() => sendPositive(i, msg)}
                        disabled={feedbackMap[i]?.sending}
                        title="Klasifikasi ini benar"
                        className={`text-xs px-2.5 py-1 rounded-full border transition-colors ${
                          feedbackMap[i]?.type === 'positive'
                            ? 'bg-emerald-950 border-emerald-700 text-emerald-400'
                            : 'border-gray-700 text-gray-400 hover:text-emerald-400 hover:border-emerald-700'
                        }`}
                      >
                        👍 Benar
                      </button>
                      <button
                        type="button"
                        onClick={() => setCorrectionForm(prev => ({ ...prev, [i]: { ...(prev[i] || { kode: '', alasan: '' }), open: !prev[i]?.open } }))}
                        title="Koreksi kode klasifikasi"
                        className="text-xs px-2.5 py-1 rounded-full border border-gray-700 text-gray-400 hover:text-amber-400 hover:border-amber-700 transition-colors"
                      >
                        ✏️ Koreksi
                      </button>
                    </>
                  ) : (
                    <div className={`text-xs px-3 py-1.5 rounded-lg border ${
                      feedbackMap[i].result?.valid
                        ? 'bg-emerald-950/60 border-emerald-800 text-emerald-300'
                        : 'bg-amber-950/60 border-amber-800 text-amber-300'
                    }`}>
                      {feedbackMap[i].result?.valid ? '✅ ' : '⚠️ '}{feedbackMap[i].result?.penjelasan}
                    </div>
                  )}
                </div>
              )}

              {/* Form koreksi */}
              {correctionForm[i]?.open && !feedbackMap[i]?.result && (
                <div className="bg-gray-900 border border-amber-800/50 rounded-xl p-3 space-y-2">
                  <p className="text-xs text-amber-300 font-medium">Koreksi kode klasifikasi (divalidasi AI sebelum dipakai){authConfig?.enabled ? ' · butuh login' : ''}</p>
                  <div className="relative">
                    <input
                      value={correctionForm[i]?.kode || ''}
                      onChange={(e) => searchKode(i, e.target.value)}
                      placeholder="Ketik kode (mis. 800.12.03) atau kata kunci..."
                      className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-xs text-white placeholder-gray-500 focus:outline-none focus:border-amber-600"
                    />
                    {correctionForm[i]?.suggestions && correctionForm[i].suggestions!.length > 0 && (
                      <div className="absolute z-10 mt-1 w-full bg-gray-800 border border-gray-700 rounded-lg overflow-hidden shadow-xl max-h-40 overflow-y-auto">
                        {correctionForm[i].suggestions!.map(s => (
                          <button
                            key={s.kode}
                            type="button"
                            onClick={() => setCorrectionForm(prev => ({ ...prev, [i]: { ...(prev[i] || { kode: '', alasan: '' }), kode: s.kode, suggestions: [], selected: { kode: s.kode, deskripsi: s.deskripsi, path: s.path } } }))}
                            className="w-full text-left px-3 py-2 hover:bg-gray-700 text-xs"
                          >
                            <span className="font-mono text-amber-400">{s.kode}</span>
                            <span className="ml-2 text-gray-300">{s.deskripsi}</span>
                          </button>
                        ))}
                      </div>
                    )}
                  </div>

                  {/* Info kode terpilih: deskripsi + path lengkap agar user yakin */}
                  {correctionForm[i]?.selected && (
                    <div className="rounded-lg border border-emerald-800/50 bg-emerald-950/20 px-3 py-2 space-y-0.5">
                      <div className="flex items-start gap-2">
                        <span className="shrink-0 font-mono text-xs font-semibold text-amber-400">{correctionForm[i].selected!.kode}</span>
                        <span className="text-xs text-gray-200">{correctionForm[i].selected!.deskripsi}</span>
                      </div>
                      <div className="text-[10px] text-gray-500 leading-relaxed">
                        Path: {correctionForm[i].selected!.path}
                      </div>
                    </div>
                  )}
                  <textarea
                    value={correctionForm[i]?.alasan || ''}
                    onChange={(e) => setCorrectionForm(prev => ({ ...prev, [i]: { ...(prev[i] || { kode: '', alasan: '' }), alasan: e.target.value } }))}
                    placeholder="Alasan (opsional)..."
                    rows={2}
                    className="w-full resize-none rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-xs text-white placeholder-gray-500 focus:outline-none focus:border-amber-600"
                  />
                  <div className="flex gap-2 justify-end">
                    <button
                      type="button"
                      onClick={() => setCorrectionForm(prev => ({ ...prev, [i]: { ...(prev[i] || { kode: '', alasan: '' }), open: false } }))}
                      className="text-xs px-3 py-1.5 rounded-lg text-gray-400 hover:text-white transition-colors"
                    >
                      Batal
                    </button>
                    {!user && authConfig?.enabled ? (
                      <button
                        type="button"
                        onClick={login}
                        className="text-xs px-3 py-1.5 rounded-lg bg-violet-600 text-white hover:bg-violet-700 transition-colors"
                      >
                        🔐 Login untuk Kirim Koreksi
                      </button>
                    ) : (
                      <button
                        type="button"
                        onClick={() => submitCorrection(i, msg)}
                        disabled={feedbackMap[i]?.sending || !(correctionForm[i]?.kode || '').trim()}
                        className="text-xs px-3 py-1.5 rounded-lg bg-amber-600 text-white hover:bg-amber-700 disabled:opacity-40 transition-colors"
                      >
                        {feedbackMap[i]?.sending ? 'Memvalidasi AI...' : 'Kirim Koreksi'}
                      </button>
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>
        ))}

        {loading && (
          <div className="flex gap-3">
            <div className="shrink-0 w-8 h-8 rounded-full bg-violet-600 flex items-center justify-center text-xs font-bold text-white">AI</div>
            <div className="bg-gray-800 rounded-2xl rounded-bl-md px-5 py-4">
              <div className="flex gap-1.5">
                <span className="w-2 h-2 rounded-full bg-gray-500 animate-bounce" style={{ animationDelay: '0ms' }} />
                <span className="w-2 h-2 rounded-full bg-gray-500 animate-bounce" style={{ animationDelay: '150ms' }} />
                <span className="w-2 h-2 rounded-full bg-gray-500 animate-bounce" style={{ animationDelay: '300ms' }} />
              </div>
            </div>
          </div>
        )}

        <div ref={chatEndRef} />
      </main>

      {/* Input Area */}
      <footer className="shrink-0 border-t border-gray-800 bg-gray-950 px-6 py-4">
        {/* Warning about confidential documents */}
        {messages.length > 0 && messages[messages.length - 1].role === 'assistant' && messages[messages.length - 1].content.includes('tidak dapat melakukan reranking') && (
          <div className="mb-3 p-3 bg-amber-950/50 border border-amber-800/50 rounded-xl text-xs text-amber-300">
            ⚠️ Gemini gagal melakukan reranking. Hasil ditampilkan berdasarkan similarity semantic. Pastikan API key valid dan database pgvector terisi data.
          </div>
        )}

        <form onSubmit={handleSubmit} className="flex gap-3 items-center">
          <div className="relative flex-1">
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Ketik perihal naskah... atau upload file PDF/DOCX"
              disabled={isInputDisabled}
              rows={2}
              className="w-full resize-none rounded-xl border border-gray-700 bg-gray-800 px-4 py-3 pr-12 text-sm text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-violet-600 focus:border-transparent"
            />
          </div>

          <div className="flex gap-2 items-center">
            {/* Tombol upload file — selalu terlihat */}
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              title="Upload PDF/DOCX"
              disabled={isInputDisabled}
              className={`shrink-0 w-10 h-10 rounded-xl border border-gray-700 bg-gray-800 flex items-center justify-center transition-colors ${
                isInputDisabled
                  ? 'text-gray-600 cursor-not-allowed opacity-50'
                  : 'text-gray-400 hover:bg-gray-700 hover:text-white'
              }`}
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13" />
              </svg>
            </button>

            {loading ? (
              <button type="button" disabled className="w-10 h-10 rounded-xl bg-violet-600 text-white flex items-center justify-center opacity-50">
                <svg className="animate-spin h-5 w-5" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
              </button>
            ) : (
              <button
                type="submit"
                disabled={!input.trim() || isInputDisabled}
                className="shrink-0 w-10 h-10 rounded-xl bg-violet-600 text-white flex items-center justify-center hover:bg-violet-700 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
              >
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
                </svg>
              </button>
            )}

            <input
              ref={fileInputRef}
              type="file"
              accept=".pdf,.docx"
              onChange={handleFileUpload}
              className="hidden"
            />
          </div>
        </form>

        <p className="mt-2 text-center text-xs font-semibold text-red-400">
          ⚠️ JANGAN UPLOAD NASKAH BERSIFAT RAHASIA ATAU BERISI INFORMASI SENSITIF
        </p>
        </footer>
      </div>

      {view === 'stats' && (
        <StatsDashboard
          stats={stats}
          loading={statsLoading}
          onRefresh={fetchStats}
          filter={statsFilter}
          onApplyFilter={applyStatsFilter}
          onClearFilter={clearStatsFilter}
          canDelete={isAdmin}
          onDeleteClick={(r) => {
            setDeleteTarget(r)
            setDeletePassword('')
            setDeleteError(null)
            if (deleteLockoutRef.current) clearInterval(deleteLockoutRef.current)
            setDeleteLockout(null)
          }}
        />
      )}

      {view === 'settings' && (
        <ApiKeySettings
          keys={userApiKeys}
          onSave={saveKeys}
        />
      )}

      {/* Modal hapus feedback (admin + password secret) */}
      {deleteTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <div className="w-full max-w-md rounded-2xl border border-gray-700 bg-gray-900 p-5 space-y-4 shadow-2xl">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <h3 className="text-sm font-semibold text-white">Hapus Feedback #{deleteTarget.id}</h3>
                <p className="text-xs text-gray-500 mt-0.5 truncate">
                  {deleteTarget.perihal || deleteTarget.naskah || '—'}
                </p>
              </div>
              <button
                type="button"
                onClick={closeDeleteModal}
                className="shrink-0 p-1.5 rounded-lg text-gray-500 hover:text-white hover:bg-gray-800 transition-colors"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
                </svg>
              </button>
            </div>

            <div className="rounded-lg border border-red-900/60 bg-red-950/30 px-3 py-2 text-xs text-red-300">
              ⚠️ Tindakan ini permanen dan tidak dapat dibatalkan. Hanya admin yang dapat menghapus, dengan memasukkan password secret server.
            </div>

            {deleteLockout !== null ? (
              <div className="rounded-lg border border-amber-800/60 bg-amber-950/40 px-3 py-2.5 text-xs text-amber-300 flex items-center gap-2">
                <span>🔒 Terlalu banyak percobaan password.</span>
                <span className="ml-auto font-semibold tabular-nums whitespace-nowrap">
                  Coba lagi dalam {formatDuration(deleteLockout)}
                </span>
              </div>
            ) : deleteError ? (
              <div className="text-xs text-red-400">{deleteError}</div>
            ) : null}

            <div>
              <label className="block text-[10px] text-gray-500 uppercase tracking-wider mb-1">Password secret (DELETE_SECRET)</label>
              <input
                type="password"
                value={deletePassword}
                onChange={(e) => setDeletePassword(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter' && deletePassword.trim() && !deleting && deleteLockout === null) confirmDelete() }}
                placeholder="Masukkan password admin..."
                autoFocus={deleteLockout === null}
                disabled={deleting || deleteLockout !== null}
                className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-xs text-white placeholder-gray-600 focus:outline-none focus:border-red-500 disabled:opacity-50"
              />
            </div>

            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={closeDeleteModal}
                disabled={deleting}
                className="text-xs px-4 py-2 rounded-lg text-gray-400 hover:text-white transition-colors disabled:opacity-50"
              >
                Batal
              </button>
              <button
                type="button"
                onClick={confirmDelete}
                disabled={!deletePassword.trim() || deleting || deleteLockout !== null}
                className="text-xs px-4 py-2 rounded-lg bg-red-600 text-white hover:bg-red-700 disabled:opacity-40 transition-colors"
              >
                {deleteLockout !== null
                  ? '🔒 Terkunci'
                  : deleting
                    ? 'Menghapus...'
                    : '🗑 Hapus'}
              </button>
            </div>
          </div>
        </div>
      )}
      </div>
    </div>
  )
}

export default App
