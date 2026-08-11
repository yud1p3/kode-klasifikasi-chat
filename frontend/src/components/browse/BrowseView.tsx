import { useState, useEffect, useCallback, useRef } from 'react'
import { useDebounce } from '../../hooks/useDebounce'
import { useInfiniteScroll } from '../../hooks/useInfiniteScroll'
import { Breadcrumb, type BreadcrumbItem } from './Breadcrumb'
import { SearchBar } from './SearchBar'
import { ClassificationCard, type BrowseItem } from './ClassificationCard'

const LIMIT = 20

interface BrowseResponse {
  items: BrowseItem[]
  total: number
}

interface BrowseViewProps {
  apiBase: string
}

// ---------- Komponen state kosong ----------

function EmptyState({ icon, title, description, action }: {
  icon?: React.ReactNode
  title: string
  description?: string
  action?: { label: string; onClick: () => void }
}) {
  return (
    <div className="flex flex-col items-center justify-center py-16 px-4 text-center">
      {icon && <div className="mb-4 text-gray-600">{icon}</div>}
      <h3 className="text-lg font-semibold text-gray-200 mb-2">{title}</h3>
      {description && <p className="text-gray-500 mb-4 max-w-md">{description}</p>}
      {action && (
        <button
          onClick={action.onClick}
          className="px-4 py-2 text-sm font-medium text-white bg-violet-600 rounded-lg hover:bg-violet-700 transition-colors focus:outline-none focus:ring-2 focus:ring-violet-400/50"
        >
          {action.label}
        </button>
      )}
    </div>
  )
}

function LoadingSkeleton({ count = 5 }: { count?: number }) {
  return (
    <div className="grid gap-3 grid-cols-1 md:grid-cols-2 lg:grid-cols-3">
      {Array.from({ length: count }).map((_, i) => (
        <div key={i} className="rounded-xl border border-gray-800 bg-gray-900 p-5 space-y-3 animate-pulse">
          <div className="w-20 h-6 rounded-md bg-gray-800" />
          <div className="h-4 rounded bg-gray-800 w-3/4" />
          <div className="h-4 rounded bg-gray-800 w-1/2" />
          <div className="flex gap-2 pt-2">
            <div className="w-16 h-7 rounded-lg bg-gray-800" />
            <div className="w-16 h-7 rounded-lg bg-gray-800" />
          </div>
        </div>
      ))}
    </div>
  )
}

/**
 * Halaman browse klasifikasi: cari kode (keyword), navigasi parent-child, dan
 * breadcrumb — memakai endpoint /api/browse/* (PostgreSQL langsung, tanpa
 * Meilisearch). Pencarian memakai ILIKE (gratis, tanpa kuota AI).
 */
export function BrowseView({ apiBase }: BrowseViewProps) {
  // State navigasi
  const [currentPath, setCurrentPath] = useState<BrowseItem[]>([])
  const [currentParentId, setCurrentParentId] = useState<number | null>(null)
  const [currentLevel, setCurrentLevel] = useState(1)

  // State data
  const [items, setItems] = useState<BrowseItem[]>([])
  const [hasMore, setHasMore] = useState(true)
  const [isLoading, setIsLoading] = useState(false)
  const [isLoadingMore, setIsLoadingMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [totalHits, setTotalHits] = useState(0)
  const offsetRef = useRef(0)

  // State pencarian
  const [searchQuery, setSearchQuery] = useState('')
  const [isSearching, setIsSearching] = useState(false)
  const [searchResults, setSearchResults] = useState<BrowseItem[]>([])
  const [searchHasMore, setSearchHasMore] = useState(false)
  const [searchTotalHits, setSearchTotalHits] = useState(0)
  const searchOffsetRef = useRef(0)
  // Urutan permintaan pencarian — mengabaikan respons yang sudah kedaluwarsa
  // (race condition saat user mengetik cepat: respons lama tidak menimpa yang baru)
  const searchSeqRef = useRef(0)

  const isSearchView = searchQuery.trim().length >= 2

  const clearSearchState = useCallback(() => {
    setSearchQuery('')
    setSearchResults([])
    setSearchHasMore(false)
    setSearchTotalHits(0)
    setIsSearching(false)
    searchOffsetRef.current = 0
  }, [])

  // UI state
  const [expandedItems, setExpandedItems] = useState<Set<number>>(new Set())

  // Query debounced
  const debouncedQuery = useDebounce(searchQuery, 500)
  // Benar-benar sedang menunggu jeda debounce (user mengetik, request belum dikirim)
  const isDebouncePending = searchQuery.trim().length >= 2 && searchQuery !== debouncedQuery && !isSearching

  // ---------- Fetch helpers ----------

  const fetchJson = useCallback(async (path: string): Promise<BrowseResponse> => {
    const r = await fetch(`${apiBase}${path}`)
    if (!r.ok) {
      const err = await r.json().catch(() => null) as { error?: string } | null
      throw new Error(err?.error || `Gagal memuat data (HTTP ${r.status})`)
    }
    return await r.json() as BrowseResponse
  }, [apiBase])

  // Endpoint /api/browse/document mengembalikan OBJEK TUNGGAL (bukan {items,total})
  const fetchDocument = useCallback(async (id: number): Promise<BrowseItem | null> => {
    try {
      const r = await fetch(`${apiBase}/api/browse/document?id=${id}`)
      if (!r.ok) return null
      return await r.json() as BrowseItem
    } catch (e) {
      console.error('Fetch document error:', e)
      return null
    }
  }, [apiBase])

  const loadItems = useCallback(async (parentId: number | null, offset = 0, append = false) => {
    try {
      if (!append) setIsLoading(true)
      else setIsLoadingMore(true)
      setError(null)

      const path = parentId === null
        ? `/api/browse/roots?offset=${offset}&limit=${LIMIT}`
        : `/api/browse/children?parent_id=${parentId}&offset=${offset}&limit=${LIMIT}`
      const result = await fetchJson(path)

      const newItems = result?.items || []
      const total = result?.total || 0

      if (append) {
        setItems(prev => [...prev, ...newItems])
      } else {
        setItems(newItems)
        offsetRef.current = 0
      }
      offsetRef.current = offset + newItems.length
      setHasMore(offsetRef.current < total)
      setTotalHits(total)
    } catch (err) {
      console.error('Load items error:', err)
      setError(err instanceof Error ? err.message : 'Gagal memuat data')
    } finally {
      setIsLoading(false)
      setIsLoadingMore(false)
    }
  }, [fetchJson])

  const loadMoreItems = useCallback(() => {
    if (isLoadingMore || !hasMore || isSearchView) return
    loadItems(currentParentId, offsetRef.current, true)
  }, [isLoadingMore, hasMore, isSearchView, loadItems, currentParentId])

  // ---------- Pencarian ----------

  const handleSearch = useCallback(async (query: string) => {
    if (!query.trim()) {
      setSearchResults([])
      setSearchHasMore(false)
      setSearchTotalHits(0)
      setIsSearching(false)
      return
    }

    setIsSearching(true)
    searchOffsetRef.current = 0
    const seq = ++searchSeqRef.current

    try {
      // Cari di semua turunan memakai prefix kode (bukan hanya anak langsung)
      const currentKode = currentPath.length > 0 ? currentPath[currentPath.length - 1].kode : ''
      const qs = new URLSearchParams({ q: query, limit: String(LIMIT), offset: '0' })
      if (currentKode) qs.set('kode_prefix', currentKode)
      const result = await fetchJson(`/api/browse/search?${qs.toString()}`)
      if (seq !== searchSeqRef.current) return // respons lama — abaikan

      const hits = result?.items || []
      const total = result?.total || 0
      setSearchResults(hits)
      setSearchTotalHits(total)
      searchOffsetRef.current = hits.length
      setSearchHasMore(searchOffsetRef.current < total)
    } catch (err) {
      if (seq !== searchSeqRef.current) return
      console.error('Search error:', err)
      setError(err instanceof Error ? err.message : 'Gagal pencarian')
    } finally {
      if (seq === searchSeqRef.current) setIsSearching(false)
    }
  }, [fetchJson, currentPath])

  const loadMoreSearchResults = useCallback(async () => {
    if (!searchHasMore || isSearching || !debouncedQuery.trim()) return

    setIsSearching(true)
    const seq = searchSeqRef.current // load-more tidak menaikkan seq — pakai seq pencarian aktif
    try {
      const currentKode = currentPath.length > 0 ? currentPath[currentPath.length - 1].kode : ''
      const qs = new URLSearchParams({
        q: debouncedQuery,
        limit: String(LIMIT),
        offset: String(searchOffsetRef.current),
      })
      if (currentKode) qs.set('kode_prefix', currentKode)
      const result = await fetchJson(`/api/browse/search?${qs.toString()}`)
      if (seq !== searchSeqRef.current) return // pencarian baru dimulai — abaikan

      const newHits = result?.items || []
      const total = result?.total || 0
      setSearchResults(prev => [...prev, ...newHits])
      searchOffsetRef.current += newHits.length
      setSearchTotalHits(prev => Math.max(prev, total))
      setSearchHasMore(searchOffsetRef.current < total)
    } catch (err) {
      if (seq !== searchSeqRef.current) return
      console.error('Load more search error:', err)
    } finally {
      if (seq === searchSeqRef.current) setIsSearching(false)
    }
  }, [searchHasMore, isSearching, debouncedQuery, fetchJson, currentPath])

  // ---------- Infinite scroll ----------

  const { setSentinel } = useInfiniteScroll({
    onLoadMore: loadMoreItems,
    hasMore: isSearchView ? false : hasMore,
    isLoading: isSearchView ? false : isLoadingMore,
  })

  const { setSentinel: setSearchSentinel } = useInfiniteScroll({
    onLoadMore: loadMoreSearchResults,
    hasMore: searchHasMore,
    isLoading: false,
  })

  // ---------- Navigasi breadcrumb ----------

  const handleBreadcrumbClick = useCallback((item: BreadcrumbItem, index: number) => {
    if (item.disabled) return

    if (index === 0) {
      setCurrentPath([])
      setCurrentParentId(null)
      setCurrentLevel(1)
      clearSearchState()
      loadItems(null)
    } else {
      const newPath = currentPath.slice(0, index)
      setCurrentPath(newPath)
      const targetNode = currentPath[index - 1]
      setCurrentParentId(targetNode.id)
      setCurrentLevel(index + 1)
      clearSearchState()
      loadItems(targetNode.id)
    }
  }, [currentPath, loadItems, clearSearchState])

  // Klik "Induk" — kembali ke induk (juga dari hasil pencarian)
  const handleIndukClick = useCallback(async (hit: BrowseItem) => {
    if (!hit.parent_id || hit.parent_id === 0) {
      setCurrentPath([])
      setCurrentParentId(null)
      setCurrentLevel(1)
      clearSearchState()
      loadItems(null)
      return
    }

    const parentId = hit.parent_id

    // Coba cari induk di currentPath (berlaku untuk navigasi normal)
    const parentIndex = currentPath.findIndex(n => n.id === parentId)
    if (parentIndex >= 0) {
      const newPath = currentPath.slice(0, parentIndex + 1)
      setCurrentPath(newPath)
      setCurrentLevel(parentIndex + 2)
      setCurrentParentId(parentId)
      clearSearchState()
      loadItems(parentId)
      return
    }

    // Induk tidak ada di currentPath (mis. dari hasil pencarian) — bangun path
    // dengan menelusuri nenek moyang hingga akar.
    try {
      const parentDoc = await fetchDocument(parentId)
      if (parentDoc) {
        const newPath: BrowseItem[] = []
        let current = parentDoc
        // Batas pengaman: maksimal 10 level (mencegah loop tak berujung)
        let guard = 0
        while (current && guard < 10) {
          newPath.unshift({ id: current.id, kode: current.kode, deskripsi: current.deskripsi, path: current.path, parent_id: current.parent_id, level: current.level, has_children: current.has_children })
          if (!current.parent_id || current.parent_id === 0) break
          const anc = await fetchDocument(current.parent_id)
          if (!anc) break
          current = anc
          guard++
        }
        setCurrentPath(newPath)
        setCurrentLevel(newPath.length + 1)
        setCurrentParentId(parentId)
      } else {
        // Fallback: naik satu level dari path saat ini
        if (currentPath.length > 0) {
          const newPath = currentPath.slice(0, -1)
          setCurrentPath(newPath)
          setCurrentLevel(newPath.length + 1)
          setCurrentParentId(newPath.length > 0 ? newPath[newPath.length - 1].id : null)
        } else {
          setCurrentPath([])
          setCurrentParentId(null)
          setCurrentLevel(1)
        }
      }
    } catch (err) {
      console.error('Failed to fetch parent for breadcrumb:', err)
      setCurrentParentId(parentId)
    }
    clearSearchState()
    loadItems(parentId)
  }, [currentPath, loadItems, clearSearchState, fetchDocument])

  // Klik "Sub Klas" — telusuri turunan (dari navigasi ataupun hasil pencarian)
  const handleSubKlasClick = useCallback(async (hit: BrowseItem) => {
    // Sudah ada di currentPath (navigasi normal)
    const alreadyInPath = currentPath.some(n => n.id === hit.id)
    if (alreadyInPath) {
      setCurrentParentId(hit.id)
      setCurrentLevel(currentPath.length + 1)
      clearSearchState()
      await loadItems(hit.id)
      return
    }

    // Dari hasil pencarian: bangun seluruh rantai nenek moyang
    try {
      const newPath: BrowseItem[] = []
      let current: BrowseItem = hit
      let guard = 0
      while (current && guard < 10) {
        newPath.unshift({ id: current.id, kode: current.kode, deskripsi: current.deskripsi, path: current.path, parent_id: current.parent_id, level: current.level, has_children: current.has_children })
        if (!current.parent_id || current.parent_id === 0) break
        const anc = await fetchDocument(current.parent_id)
        if (!anc) break
        current = anc
        guard++
      }

      setCurrentPath(newPath)
      setCurrentLevel(newPath.length + 1)
      setCurrentParentId(hit.id)
    } catch (err) {
      console.error('Failed to build breadcrumb for sub klas:', err)
      setCurrentPath(prev => [...prev, { id: hit.id, kode: hit.kode, deskripsi: hit.deskripsi, path: hit.path, parent_id: hit.parent_id, level: hit.level, has_children: hit.has_children }])
      setCurrentParentId(hit.id)
      setCurrentLevel(prev => prev + 1)
    }
    clearSearchState()
    await loadItems(hit.id)
  }, [loadItems, clearSearchState, currentPath, fetchDocument])

  // ---------- Toggle detail ----------

  const handleDetailToggle = useCallback((id: number, expanded: boolean) => {
    setExpandedItems(prev => {
      const next = new Set(prev)
      if (expanded) next.add(id)
      else next.delete(id)
      return next
    })
  }, [])

  // ---------- Breadcrumb items ----------

  const breadcrumbItems: BreadcrumbItem[] = [
    { label: 'Home', query: '', disabled: false },
    ...currentPath.map((node, idx) => ({
      label: node.deskripsi ? `${node.kode} - ${node.deskripsi}` : node.kode,
      query: `nav:${node.id}`,
      disabled: idx === currentPath.length - 1,
    })),
  ]

  // ---------- Load awal ----------

  useEffect(() => {
    if (currentParentId === null && !isSearchView) {
      loadItems(null)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // ---------- Auto-search saat debounced query berubah ----------

  useEffect(() => {
    if (debouncedQuery && debouncedQuery.length >= 2) {
      handleSearch(debouncedQuery)
    } else if (!debouncedQuery) {
      setSearchResults([])
      setSearchHasMore(false)
      setSearchTotalHits(0)
      setIsSearching(false)
    }
  }, [debouncedQuery, handleSearch])

  // ---------- Tampilan ----------

  const displayItems = isSearchView ? searchResults : items
  const displayHasMore = isSearchView ? searchHasMore : hasMore
  const displayTotalHits = isSearchView ? searchTotalHits : totalHits
  const displaySetSentinel = isSearchView ? setSearchSentinel : setSentinel
  const displayIsLoadingMore = isSearchView ? isSearching : isLoadingMore

  const getItemLevel = (hit: BrowseItem) => hit.level || currentLevel

  return (
    <main className="flex-1 overflow-y-auto px-6 py-5">
      <div className="max-w-6xl mx-auto">
        {/* Breadcrumb */}
        <Breadcrumb items={breadcrumbItems} onItemClick={handleBreadcrumbClick} />

        {/* Jumlah klasifikasi */}
        {displayTotalHits > 0 && (
          <div className="text-xs text-gray-500 mb-3">
            <span className="font-mono text-violet-400 font-semibold">{displayTotalHits.toLocaleString('id-ID')}</span>
            <span className="ml-1">klasifikasi</span>
          </div>
        )}

        {/* Search bar */}
        <div className="mb-4">
          <SearchBar
            value={searchQuery}
            onChange={setSearchQuery}
            placeholder={currentParentId
              ? `Cari di dalam ${currentPath[currentPath.length - 1]?.kode}...`
              : 'Cari kode klasifikasi (min. 2 karakter)...'}
            isLoading={isLoading || isSearching}
          />

          {/* Indikator jeda debounce — request akan dikirim sebentar lagi */}
          {isDebouncePending && (
            <p className="mt-1.5 flex items-center gap-1.5 text-xs text-gray-500" role="status" aria-live="polite">
              <span className="inline-flex gap-0.5" aria-hidden="true">
                <span className="w-1 h-1 rounded-full bg-violet-400 animate-bounce" style={{ animationDelay: '0ms' }} />
                <span className="w-1 h-1 rounded-full bg-violet-400 animate-bounce" style={{ animationDelay: '150ms' }} />
                <span className="w-1 h-1 rounded-full bg-violet-400 animate-bounce" style={{ animationDelay: '300ms' }} />
              </span>
              Mengetik…
            </p>
          )}
        </div>

        {/* Loading */}
        {(isLoading || isSearching) && displayItems.length === 0 && (
          <LoadingSkeleton count={5} />
        )}

        {/* Error */}
        {error && !isLoading && !isSearching && displayItems.length === 0 && (
          <EmptyState
            title="Terjadi Kesalahan"
            description={error}
            action={{ label: 'Coba Lagi', onClick: () => loadItems(currentParentId) }}
            icon={
              <svg className="w-16 h-16 mx-auto text-red-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
            }
          />
        )}

        {/* Initial state */}
        {!isLoading && !isSearching && displayItems.length === 0 && !error && !searchQuery && currentParentId === null && (
          <EmptyState
            title="Selamat Datang di Browser Klasifikasi"
            description="Mulai dengan menelusuri klasifikasi induk, atau gunakan pencarian untuk menemukan klasifikasi spesifik."
            action={{ label: 'Lihat Klasifikasi Induk', onClick: () => loadItems(null) }}
            icon={
              <svg className="w-16 h-16 mx-auto text-violet-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
              </svg>
            }
          />
        )}

        {/* Tidak ada hasil pencarian */}
        {!isLoading && !isSearching && displayItems.length === 0 && searchQuery && !error && (
          <EmptyState
            title="Tidak ditemukan hasil"
            description={`Pencarian untuk "${searchQuery}" tidak mengembalikan hasil apapun. Coba perbaiki kata kunci.`}
            action={{ label: 'Hapus Pencarian', onClick: () => setSearchQuery('') }}
            icon={
              <svg className="w-16 h-16 mx-auto text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9.172 16.172a4 4 0 015.656 0M9 10h.01M15 10h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
            }
          />
        )}

        {/* Grid hasil */}
        {displayItems.length > 0 && (
          <>
            <div className="grid gap-3 grid-cols-1 md:grid-cols-2 lg:grid-cols-3" role="list" aria-label={isSearchView ? `Hasil pencarian: ${searchQuery}` : 'Daftar klasifikasi'}>
              {displayItems.map(hit => (
                <ClassificationCard
                  key={hit.id}
                  hit={hit}
                  onIndukClick={handleIndukClick}
                  onSubKlasClick={handleSubKlasClick}
                  isSelected={expandedItems.has(hit.id)}
                  onSelectToggle={handleDetailToggle}
                  hasChildren={hit.has_children}
                  level={getItemLevel(hit)}
                />
              ))}
            </div>

            {/* Sentinel muat lebih banyak */}
            <div ref={displaySetSentinel} className="h-4" aria-hidden="true">
              {displayHasMore && !displayIsLoadingMore && (
                <div className="flex justify-center py-4">
                  <button
                    onClick={() => displayHasMore && (isSearchView ? loadMoreSearchResults() : loadMoreItems())}
                    disabled={displayIsLoadingMore}
                    className="px-6 py-2 text-sm font-medium text-violet-300 bg-violet-950/50 border border-violet-700/60 rounded-lg hover:bg-violet-900/50 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
                  >
                    {displayIsLoadingMore ? (
                      <>
                        <div className="w-4 h-4 border-2 border-violet-400 border-t-transparent rounded-full animate-spin" />
                        Memuat...
                      </>
                    ) : (
                      'Muat Lebih Banyak'
                    )}
                  </button>
                </div>
              )}
              {!displayHasMore && displayItems.length > 0 && (
                <p className="text-center text-sm text-gray-500 py-4">
                  {isSearchView
                    ? `Menampilkan ${displayItems.length} dari ${displayTotalHits} hasil`
                    : 'Semua data telah dimuat'}
                </p>
              )}
            </div>
          </>
        )}

        {/* Navigasi tanpa anak */}
        {!isLoading && !isSearching && displayItems.length === 0 && currentParentId && !searchQuery && !error && (
          <EmptyState
            title="Tidak ada sub-klasifikasi"
            description={`Klasifikasi ${currentPath[currentPath.length - 1]?.kode} tidak memiliki anak.`}
            icon={
              <svg className="w-16 h-16 mx-auto text-gray-700" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2" />
              </svg>
            }
          />
        )}
      </div>
    </main>
  )
}
