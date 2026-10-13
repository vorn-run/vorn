import { useEffect, type RefObject } from 'react'

export function useOutsideClick(
  ref: RefObject<HTMLElement | null>,
  enabled: boolean,
  onOutside: () => void
): void {
  useEffect(() => {
    if (!enabled) return
    const handler = (e: MouseEvent): void => {
      // A missing anchor counts as "outside" so the popover still closes if
      // its trigger unmounts while open (e.g. last item was dismissed).
      if (!ref.current || !ref.current.contains(e.target as Node)) onOutside()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [ref, enabled, onOutside])
}
