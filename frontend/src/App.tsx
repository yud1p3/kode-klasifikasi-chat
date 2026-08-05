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

interface Message {
  role: 'user' | 'assistant'
  content: string
  results?: ClassificationResult[]
  isRateLimit?: boolean
}

function formatCooldown(s: number): string {
  if (s >= 60) {
    const m = Math.floor(s / 60)
    const sec = s % 60
    return sec > 0 ? `${m} menit ${sec} detik` : `${m} menit`
  }
  return `${s} detik`
}

const EXAMPLE_QUERIES = [
  'Permohonan cuti tahunan pegawai',
  'Pengadaan laptop untuk unit kerja',
  'Laporan keuangan triwulan III',
]

function App() {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [loading, setLoading] = useState(false)
  const [extracting, setExtracting] = useState(false)
  const [apiAvailable, setApiAvailable] = useState<boolean | null>(null)
  const [cooldown, setCooldown] = useState<number | null>(null)
  const chatEndRef = useRef<HTMLDivElement>(null)
  const cooldownRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const API_BASE = import.meta.env.VITE_API_URL || 'http://localhost:3000'

  useEffect(() => {
    fetch(`${API_BASE}/api/health`)
      .then(r => r.json())
      .then(() => setApiAvailable(true))
      .catch(() => setApiAvailable(false))
  }, [])

  useEffect(() => {
    chatEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

  // Cleanup cooldown timer
  useEffect(() => {
    return () => {
      if (cooldownRef.current) clearInterval(cooldownRef.current)
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

    setExtracting(true)
    try {
      let text = ''
      const ext = file.name.split('.').pop()?.toLowerCase()

      if (ext === 'pdf') {
        // Prioritaskan ekstraksi via backend (poppler): benar untuk PDF SRIKANDI
        // yang ToUnicode-nya rusak (pdf.js menghasilkan karakter garbled).
        const fd = new FormData()
        fd.append('file', file)
        try {
          const r = await fetch(`${API_BASE}/api/extract-pdf`, { method: 'POST', body: fd })
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
    } finally {
      setExtracting(false)
    }
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!input.trim() || loading || cooldown !== null) return

    const userMsg: Message = { role: 'user', content: input }
    setMessages(prev => [...prev, userMsg])
    setInput('')
    setLoading(true)

    try {
      const resp = await fetch(`${API_BASE}/api/chat`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message: userMsg.content })
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
        setLoading(false)
        return
      }

      const data: ChatResponse = await resp.json()
      const assistantMsg: Message = {
        role: 'assistant',
        content: data.explanation,
        results: data.results?.slice(0, 3)
      }
      setMessages(prev => [...prev, assistantMsg])
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

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSubmit(e as unknown as React.FormEvent)
    }
  }

  const isInputDisabled = loading || cooldown !== null

  return (
    <div className="flex flex-col h-screen max-w-4xl mx-auto bg-gray-900 shadow-2xl">
      {/* Header */}
      <header className="shrink-0 px-6 py-4 bg-gray-950 border-b border-gray-800 flex items-center gap-3 flex-wrap">
        <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-violet-500 to-blue-600 flex items-center justify-center text-sm font-bold">
          K
        </div>
        <div className="flex flex-col gap-0.5">
          <h1 className="text-base font-semibold text-white leading-tight">
            Kode Klasifikasi Arsip
          </h1>
          <span className="text-xs text-gray-500">AI Arsiparis — Pencarian Semantic</span>
        </div>
        <div className="ml-auto flex items-center gap-2">
          {cooldown !== null && (
            <span className="text-xs px-2.5 py-1 rounded-full bg-amber-950 border border-amber-800 text-amber-400">
              ⏳ {cooldown}s
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
                      </div>
                      <div className="mt-1 ml-0 text-xs text-gray-500">
                        {r.path}
                      </div>
                    </div>
                  ))}
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

        <form onSubmit={handleSubmit} className="flex gap-3 items-end">
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
  )
}

export default App
