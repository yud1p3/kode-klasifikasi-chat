import { useEffect, useRef, useCallback } from 'react'

interface UseInfiniteScrollOptions {
  onLoadMore: () => void
  hasMore?: boolean
  isLoading?: boolean
  rootMargin?: string
}

/**
 * IntersectionObserver sentinel untuk infinite scroll. Mengembalikan ref callback
 * yang dipasang ke elemen sentinel di bawah daftar; saat elemen terlihat dan
 * masih ada data (hasMore) serta tidak sedang memuat, onLoadMore dipanggil.
 */
export function useInfiniteScroll({
  onLoadMore,
  hasMore = true,
  isLoading = false,
  rootMargin = '200px',
}: UseInfiniteScrollOptions) {
  const observerRef = useRef<IntersectionObserver | null>(null)

  const setSentinel = useCallback(
    (node: HTMLElement | null) => {
      if (observerRef.current) {
        observerRef.current.disconnect()
      }
      if (node && hasMore && !isLoading) {
        observerRef.current = new IntersectionObserver(
          (entries) => {
            if (entries[0].isIntersecting) {
              onLoadMore()
            }
          },
          { rootMargin, threshold: 0 }
        )
        observerRef.current.observe(node)
      }
    },
    [onLoadMore, hasMore, isLoading, rootMargin]
  )

  useEffect(() => {
    return () => {
      if (observerRef.current) {
        observerRef.current.disconnect()
      }
    }
  }, [])

  return { setSentinel }
}
