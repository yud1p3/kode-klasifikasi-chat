import { useState, useEffect, useRef } from 'react'

export interface BrowseItem {
  id: number
  kode: string
  deskripsi: string
  path: string
  parent_id: number | null
  level: number
  has_children: boolean
  retensi_aktif?: number | null
  retensi_inaktif?: number | null
  penyusutan_akhir?: string | null
  klasifikasi_keamanan?: string | null
}

interface ClassificationCardProps {
  hit: BrowseItem
  onIndukClick: (hit: BrowseItem) => void
  onSubKlasClick: (hit: BrowseItem) => void
  isSelected?: boolean
  onSelectToggle?: (id: number, expanded: boolean) => void
  hasChildren?: boolean
  level?: number
}

// Warna badge kode berdasarkan level (klaster) — gelap sesuai tema aplikasi.
const LEVEL_BADGE: Record<number, { bg: string; text: string }> = {
  1: { bg: 'bg-blue-950/80 border-blue-700/60', text: 'text-blue-300' },
  2: { bg: 'bg-indigo-950/80 border-indigo-700/60', text: 'text-indigo-300' },
  3: { bg: 'bg-fuchsia-950/80 border-fuchsia-700/60', text: 'text-fuchsia-300' },
  4: { bg: 'bg-amber-950/80 border-amber-700/60', text: 'text-amber-300' },
  5: { bg: 'bg-emerald-950/80 border-emerald-700/60', text: 'text-emerald-300' },
  6: { bg: 'bg-cyan-950/80 border-cyan-700/60', text: 'text-cyan-300' },
  7: { bg: 'bg-rose-950/80 border-rose-700/60', text: 'text-rose-300' },
}

export function ClassificationCard({
  hit,
  onIndukClick,
  onSubKlasClick,
  isSelected = false,
  onSelectToggle,
  hasChildren = false,
  level = 1,
}: ClassificationCardProps) {
  const [isDetailOpen, setIsDetailOpen] = useState(isSelected)
  const [copied, setCopied] = useState(false)
  const copyTimerRef = useRef<number | null>(null)

  // Sinkron dengan state seleksi parent
  useEffect(() => {
    setIsDetailOpen(isSelected)
  }, [isSelected])

  // Bersihkan timer saat komponen dilepas
  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current)
    }
  }, [])

  // Reset status "Tersalin" jika kode berubah (mis. daftar diganti)
  useEffect(() => {
    setCopied(false)
    if (copyTimerRef.current !== null) {
      window.clearTimeout(copyTimerRef.current)
      copyTimerRef.current = null
    }
  }, [hit.kode])

  const copyViaFallback = (): boolean => {
    // Fallback utk environment non-HTTPS / non-secure context
    const textarea = document.createElement('textarea')
    textarea.value = hit.kode
    textarea.style.position = 'fixed'
    textarea.style.opacity = '0'
    document.body.appendChild(textarea)
    textarea.select()
    const ok = document.execCommand('copy')
    document.body.removeChild(textarea)
    return ok
  }

  const handleCopyClick = async (e: React.MouseEvent) => {
    e.stopPropagation()
    let ok = false
    if (navigator.clipboard && window.isSecureContext) {
      try {
        await navigator.clipboard.writeText(hit.kode)
        ok = true
      } catch {
        // Izin clipboard ditolak — coba jalur fallback
        ok = copyViaFallback()
      }
    } else {
      ok = copyViaFallback()
    }
    if (!ok) {
      console.error('Gagal menyalin kode:', hit.kode)
      return
    }
    setCopied(true)
    if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current)
    copyTimerRef.current = window.setTimeout(() => {
      setCopied(false)
      copyTimerRef.current = null
    }, 2000)
  }

  const handleDetailToggle = () => {
    const newState = !isDetailOpen
    setIsDetailOpen(newState)
    onSelectToggle?.(hit.id, newState)
  }

  const handleIndukClick = (e: React.MouseEvent) => {
    e.stopPropagation()
    onIndukClick(hit)
  }

  const handleSubKlasClick = (e: React.MouseEvent) => {
    e.stopPropagation()
    onSubKlasClick(hit)
  }

  const handleCardClick = (e: React.MouseEvent) => {
    // Jangan toggle bila mengklik tombol
    if ((e.target as HTMLElement).closest('button')) return
    handleDetailToggle()
  }

  const formatRetensi = (aktif?: number | null, inaktif?: number | null) => {
    const parts: string[] = []
    if (aktif !== undefined && aktif !== null) parts.push(`Aktif: ${aktif} tahun`)
    if (inaktif !== undefined && inaktif !== null) parts.push(`Inaktif: ${inaktif} tahun`)
    return parts.length > 0 ? parts.join(' | ') : '-'
  }

  const badge = LEVEL_BADGE[level] ?? LEVEL_BADGE[5]
  const penyusutan = hit.penyusutan_akhir && hit.penyusutan_akhir !== '-' ? hit.penyusutan_akhir : null

  return (
    <div
      className={`group rounded-xl border shadow-sm transition-all cursor-pointer ${
        isDetailOpen
          ? 'border-violet-500 shadow-lg shadow-violet-500/10 bg-gray-900/80'
          : 'border-gray-800 bg-gray-900 hover:border-violet-600/60 hover:shadow-lg hover:shadow-violet-500/5'
      }`}
      onClick={handleCardClick}
      style={{ animationDelay: `${level * 50}ms` }}
    >
      {/* Konten utama kartu */}
      <div className="flex flex-col md:flex-row gap-3 p-5">
        {/* Badge kode */}
        <div className={`inline-flex w-auto shrink-0 items-center justify-center rounded-lg px-3 py-1 font-mono text-sm font-bold border ${badge.bg} ${badge.text}`}>
          {hit.kode}
        </div>

        {/* Deskripsi & aksi */}
        <div className="flex-1 min-w-0">
          <h3 className="font-semibold text-gray-100 pr-4 break-words leading-snug">{hit.deskripsi}</h3>

          {/* Tombol aksi */}
          <div className="mt-3 flex flex-wrap gap-2">
            {/* Salin kode — berguna utk arsiparis, terutama saat ada kode duplikat */}
            <button
              onClick={handleCopyClick}
              className={`inline-flex items-center gap-1 px-3 py-1.5 text-xs font-medium rounded-lg transition-all focus:outline-none focus:ring-2 ${
                copied
                  ? 'text-emerald-300 bg-emerald-950/60 border border-emerald-700/60 focus:ring-emerald-600/40'
                  : 'text-gray-300 bg-gray-800 border border-gray-700 hover:bg-gray-700 hover:text-white focus:ring-gray-600'
              }`}
              title={copied ? 'Kode tersalin!' : 'Salin kode ke clipboard'}
              aria-label={copied ? `Kode ${hit.kode} tersalin` : `Salin kode ${hit.kode}`}
            >
              {copied ? (
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
              ) : (
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <rect x="9" y="9" width="13" height="13" rx="2" strokeWidth={2} />
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" />
                </svg>
              )}
              {copied ? 'Tersalin' : 'Salin kode'}
            </button>

            {hit.parent_id && hit.parent_id !== 0 && (
              <button
                onClick={handleIndukClick}
                className="inline-flex items-center gap-1 px-3 py-1.5 text-xs font-medium text-gray-300 bg-gray-800 border border-gray-700 rounded-lg hover:bg-gray-700 hover:text-white transition-all focus:outline-none focus:ring-2 focus:ring-gray-600"
                title="Kembali ke induk"
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 19l-7-7m0 0l7-7m-7 7h18" />
                </svg>
                Induk
              </button>
            )}

            <button
              onClick={handleDetailToggle}
              className={`inline-flex items-center gap-1 px-3 py-1.5 text-xs font-medium rounded-lg transition-all focus:outline-none focus:ring-2 ${
                isDetailOpen
                  ? 'text-violet-300 bg-violet-950/60 border border-violet-700/60 focus:ring-violet-600/40'
                  : 'text-gray-300 bg-gray-800 border border-gray-700 hover:bg-gray-700 hover:text-white focus:ring-gray-600'
              }`}
              aria-expanded={isDetailOpen}
              aria-controls={`detail-${hit.id}`}
            >
              <svg
                className="w-3.5 h-3.5 transition-transform duration-200"
                style={{ transform: isDetailOpen ? 'rotate(180deg)' : 'rotate(0deg)' }}
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
              {isDetailOpen ? 'Tutup Detail ▲' : 'Detail ▼'}
            </button>

            {hasChildren && (
              <button
                onClick={handleSubKlasClick}
                className="inline-flex items-center gap-1 px-3 py-1.5 text-xs font-medium text-white bg-violet-600 border border-violet-600 rounded-lg hover:bg-violet-700 hover:border-violet-700 transition-all focus:outline-none focus:ring-2 focus:ring-violet-400/50"
                title="Lihat sub-klasifikasi"
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                </svg>
                Sub Klas
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Panel detail yang dapat diperluas */}
      {isDetailOpen && (
        <div
          id={`detail-${hit.id}`}
          className="px-5 pb-5 pt-0"
          role="region"
          aria-label={`Detail ${hit.kode}`}
        >
          <div className="border-t border-gray-800 pt-4 mt-2">
            <h4 className="text-sm font-bold text-gray-200 uppercase tracking-wide mb-3">
              Informasi Lengkap
            </h4>

            {/* Path lengkap — satu-satunya konteks hirarki (deskripsi sudah tampil di judul kartu) */}
            {hit.path && (
              <div className="mb-4">
                <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">Path</p>
                <p className="text-gray-300 text-xs leading-relaxed break-words bg-gray-800/60 p-3 rounded-lg border border-gray-800">
                  {hit.path}
                </p>
              </div>
            )}

            {/* Grid metadata */}
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
              <div className="bg-gray-800/50 p-3 rounded-lg border border-gray-800">
                <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">Level (Klaster)</p>
                <p className="text-gray-100 font-mono text-sm">{level}</p>
              </div>

              {hit.parent_id != null && (
                <div className="bg-gray-800/50 p-3 rounded-lg border border-gray-800">
                  <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">Parent ID</p>
                  <p className="text-gray-100 font-mono text-sm">{hit.parent_id}</p>
                </div>
              )}

              <div className="bg-gray-800/50 p-3 rounded-lg border border-gray-800">
                <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">Penyusutan</p>
                <p className="text-gray-100 font-medium">
                  {penyusutan ?? '-'}
                  {penyusutan && <span className="text-gray-500 font-normal ml-1">(akhir)</span>}
                </p>
              </div>

              {(hit.retensi_aktif != null || hit.retensi_inaktif != null) && (
                <div className="bg-gray-800/50 p-3 rounded-lg border border-gray-800">
                  <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">Retensi</p>
                  <p className="text-gray-100 font-mono text-sm">{formatRetensi(hit.retensi_aktif, hit.retensi_inaktif)}</p>
                </div>
              )}

              {hit.klasifikasi_keamanan && hit.klasifikasi_keamanan !== '-' && (
                <div className="bg-gray-800/50 p-3 rounded-lg border border-gray-800">
                  <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">Klasifikasi Keamanan</p>
                  <p className="text-gray-100 font-medium">
                    {hit.klasifikasi_keamanan === 'Sangat Rahasia'
                      ? '🔴 Sangat Rahasia'
                      : hit.klasifikasi_keamanan === 'Rahasia'
                        ? '🟠 Rahasia'
                        : hit.klasifikasi_keamanan === 'Terbatas'
                          ? '🟡 Terbatas'
                          : hit.klasifikasi_keamanan}
                  </p>
                </div>
              )}

              <div className="bg-gray-800/50 p-3 rounded-lg border border-gray-800">
                <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">ID Dokumen</p>
                <p className="text-gray-100 font-mono text-xs">{hit.id}</p>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
