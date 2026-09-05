import { useEffect, useState } from 'react'

/** The same reach as the attach latch, so a card subscribes before it is drawn. */
const NEARLY_ON_SCREEN = '400px'

/**
 * Whether the element is near the viewport right now, both edges reported.
 *
 * `useOnScreenOnce` is a latch because attaching is paid once. This is a
 * tracker because what it drives -- which terminals' bytes this client asks the
 * server for -- is cheap to change and expensive to leave on. Without an
 * observer the answer is true, as the latch answers, so nothing is hidden under
 * test or on a surface that cannot measure.
 */
export function useOnScreen(ref: React.RefObject<HTMLElement | null>): boolean {
  const [near, setNear] = useState(() => typeof IntersectionObserver === 'undefined')

  useEffect(() => {
    const el = ref.current
    if (!el || typeof IntersectionObserver === 'undefined') {
      setNear(true)
      return
    }
    const io = new IntersectionObserver(
      (entries) => setNear(entries.some((e) => e.isIntersecting)),
      { rootMargin: NEARLY_ON_SCREEN }
    )
    io.observe(el)
    return () => io.disconnect()
  }, [ref])

  return near
}
