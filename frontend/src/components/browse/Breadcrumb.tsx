export interface BreadcrumbItem {
  label: string
  query: string
  disabled: boolean
}

interface BreadcrumbProps {
  items: BreadcrumbItem[]
  onItemClick: (item: BreadcrumbItem, index: number) => void
  className?: string
}

export function Breadcrumb({ items, onItemClick, className = '' }: BreadcrumbProps) {
  if (!items || items.length === 0) return null

  return (
    <nav className={`mb-4 text-sm ${className}`} aria-label="Breadcrumb">
      <ol className="flex flex-wrap items-center gap-1.5 text-gray-400">
        {items.map((item, index) => (
          <li key={index} className="flex items-center gap-1.5">
            {index > 0 && (
              <span className="text-gray-600">
                /
              </span>
            )}
            <button
              onClick={() => onItemClick(item, index)}
              disabled={item.disabled}
              className={`
                px-2 py-1 rounded transition-colors
                font-medium whitespace-nowrap
                ${item.disabled
                  ? 'text-gray-100 font-bold cursor-default'
                  : 'text-gray-400 hover:text-violet-300 hover:bg-gray-800 focus:outline-none focus:ring-2 focus:ring-violet-500/40'
                }
                ${index === items.length - 1 && !item.disabled
                  ? 'text-violet-300 font-bold bg-gray-800/80'
                  : ''
                }
              `}
            >
              {item.label}
            </button>
          </li>
        ))}
      </ol>
    </nav>
  )
}
