interface SearchBarProps {
  value: string
  onChange: (v: string) => void
  placeholder?: string
  isLoading?: boolean
  disabled?: boolean
}

/**
 * Pencarian kata kunci untuk halaman browse. Tanpa slider semantik — pencarian
 * ILIKE langsung ke PostgreSQL (gratis, tanpa kuota AI).
 */
export function SearchBar({
  value,
  onChange,
  placeholder = 'Cari kode klasifikasi...',
  isLoading = false,
  disabled = false,
}: SearchBarProps) {
  return (
    <div className="relative">
      <label htmlFor="browse-search-input" className="sr-only">
        Pencarian klasifikasi
      </label>
      <div className="relative">
        <input
          id="browse-search-input"
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          disabled={disabled}
          className={`
            w-full rounded-xl border px-4 pl-11 pr-20 py-2.5
            text-sm bg-gray-800/80 shadow-sm transition-all
            border-gray-700 placeholder-gray-500 text-gray-100
            focus:border-violet-500 focus:ring-2 focus:ring-violet-500/30 focus:outline-none
            ${disabled ? 'opacity-50 cursor-not-allowed' : ''}
          `}
        />
        {/* Ikon cari */}
        <div className="absolute left-3.5 top-1/2 -translate-y-1/2 text-gray-500">
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
        </div>

        {/* Kontrol kanan */}
        <div className="absolute right-2.5 top-1/2 -translate-y-1/2 flex items-center gap-1">
          {value && !disabled && !isLoading && (
            <button
              type="button"
              onClick={() => onChange('')}
              className="p-1.5 text-gray-500 hover:text-gray-300 transition-colors"
              aria-label="Hapus pencarian"
            >
              <svg className="w-4.5 h-4.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          )}

          {isLoading && (
            <div className="p-1">
              <div className="w-5 h-5 border-2 border-violet-500 border-t-transparent rounded-full animate-spin" />
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
