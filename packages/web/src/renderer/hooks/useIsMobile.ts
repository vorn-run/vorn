import { useState, useEffect } from 'react'

const MOBILE_BREAKPOINT = 768

/**
 * Returns true when the viewport width is below the mobile breakpoint (768px).
 * Uses matchMedia for efficient, debounced change detection.
 */
export function useIsMobile(): boolean {
  const [isMobile, setIsMobile] = useState(
    () => typeof window !== 'undefined' && window.innerWidth < MOBILE_BREAKPOINT
  )

  useEffect(() => {
    const mql = window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`)
    const handler = (e: MediaQueryListEvent) => setIsMobile(e.matches)
    // Safari <14 requires addListener/removeListener
    if (mql.addEventListener) {
      mql.addEventListener('change', handler)
      return () => mql.removeEventListener('change', handler)
    }
    mql.addListener(handler)
    return () => mql.removeListener(handler)
  }, [])

  return isMobile
}
