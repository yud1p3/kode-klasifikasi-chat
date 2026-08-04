import { useState, useRef, useEffect, useCallback } from 'react'

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

const EXAMPLE_QUERIES = [
  'Permohonan cuti tahunan pegawai',
  'Pengadaan laptop untuk unit kerja',
  'Laporan keuangan triwulan III',
]

function App() {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [loading, setLoading] = useState(false)
  const [apiAvailable, setApiAvailable] = useState<boolean | null>(null)
  const [cooldown, setCooldown] = useState<number | null>(null)
  const chatEndRef = useRef<HTMLDivElement>(null)
  const cooldownRef = useRef<ReturnType<typeof setInterval> | null>(null)
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
                dan AI akan mencari kode klasifikasi paling sesuai berdasarkan{' '}
                <em className="text-violet-400">vector similarity</em> di database pgvector.
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
              {/* Message bubble */}
              <div className={`rounded-2xl px-4 py-3 text-sm leading-relaxed ${
                msg.role === 'user'
                  ? 'bg-blue-600 text-white rounded-br-md'
                  : msg.isRateLimit
                    ? 'bg-amber-900/50 border border-amber-800 text-amber-200 rounded-bl-md'
                    : 'bg-gray-800 text-gray-200 rounded-bl-md'
              }`}>
                {msg.content}
              </div>

              {/* Results Table */}
              {msg.results && msg.results.length > 0 && (
                <div className="rounded-xl border border-gray-700 overflow-hidden bg-gray-900">
                  <div className="px-4 py-2 bg-gray-950 border-b border-gray-700 text-xs font-semibold text-gray-400 uppercase tracking-wider">
                    Top 3 Hasil Semantic
                  </div>
                  {msg.results.map((r, j) => (
                    <div
                      key={j}
                      className={`flex items-center gap-3 px-4 py-2.5 text-sm border-b border-gray-800 last:border-b-0 hover:bg-gray-850 transition-colors ${
                        j === 0 ? 'bg-violet-950/30' : ''
                      }`}
                    >
                      <span className="shrink-0 font-mono font-semibold text-cyan-400 w-28 text-xs">
                        {r.kode}
                      </span>
                      <span className="flex-1 text-gray-300 truncate">{r.deskripsi}</span>
                      <span className={`shrink-0 text-xs font-semibold tabular-nums ${
                        r.similarity > 0.7 ? 'text-emerald-400' : r.similarity > 0.5 ? 'text-amber-400' : 'text-gray-500'
                      }`}>
                        {(r.similarity * 100).toFixed(1)}%
                      </span>
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

      {/* Footer Input */}
      <footer className="shrink-0 px-6 py-4 bg-gray-950 border-t border-gray-800">
        {cooldown !== null && (
          <div className="mb-3 px-4 py-2 rounded-xl bg-amber-950/50 border border-amber-800 text-sm text-amber-300 flex items-center gap-2">
            <svg className="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 6v6h4.5m4.5 0a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" />
            </svg>
            <span className="flex-1">
              API Key gratis dibatasi per menit. Silakan tunggu.
            </span>
            <span className="font-mono font-bold text-amber-100 tabular-nums">{cooldown}s</span>
          </div>
        )}
        <form onSubmit={handleSubmit} className="flex gap-3">
          <textarea
            value={input}
            onChange={e => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={cooldown !== null ? `Tunggu ${cooldown}s...` : 'Ketik perihal naskah di sini...'}
            rows={1}
            disabled={isInputDisabled}
            className="flex-1 resize-none rounded-xl border border-gray-700 bg-gray-900 px-4 py-3 text-sm text-gray-100 placeholder-gray-500 outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500/50 transition-colors font-sans disabled:opacity-40"
          />
          <button
            type="submit"
            disabled={isInputDisabled || !input.trim()}
            className="shrink-0 px-5 py-3 rounded-xl bg-violet-600 hover:bg-violet-500 disabled:opacity-30 disabled:cursor-not-allowed text-white text-sm font-semibold transition-colors"
          >
            {loading ? (
              <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
              </svg>
            ) : cooldown !== null ? (
              <span className="font-mono tabular-nums">{cooldown}</span>
            ) : (
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5" />
              </svg>
            )}
          </button>
        </form>
      </footer>
    </div>
  )
}

export default App
