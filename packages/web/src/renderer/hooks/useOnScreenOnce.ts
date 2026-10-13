import { useEffect, useState } from 'react'

/**
 * How early a slot counts as on screen, so scrolling to a card finds it drawn
 * rather than filling in behind the scroll.
 */
const NEARLY_ON_SCREEN = '400px'

/**
 * True once the element has come near the viewport, and true forever after.
 *
 * A latch rather than a visibility tracker, which is what keeps it small. The
 * cost being avoided is paid once -- creating a terminal and pulling its
 * scrollback -- so there is nothing to undo when the element leaves again, no
 * detach path, and no churn while somebody scrolls a long board.
 *
 * That also makes the observer's known blind spot harmless. It cannot see
 * `visibility: hidden`, so a pane covered by a maximised sibling still reports
 * as intersecting; for a latch that is at worst attaching something a moment
 * early, which is what happens today anyway. `DeviceCard` needs the sharper
 * answer and pays for it with an ancestor walk and a `visibilitychange`
 * listener, because it is throttling a poll that repeats twice a second.
 *
 * Without an `IntersectionObserver` the answer is true immediately. jsdom has
 * none, and the alternative would report every element off screen under test.
 * Deliberately not `checkVisibility()` or `offsetParent` for the same reason:
 * both want layout that jsdom never performs.
 */
export function useOnScreenOnce(ref: React.RefObject<HTMLElement | null>): boolean {
  const [seen, setSeen] = useState(() => typeof IntersectionObserver === 'undefined')

  useEffect(() => {
    // Listed in the dependencies, so latching re-runs this and it returns here.
    // The cleanup disconnects on the way past, which is the point: a latched
    // slot must stop being watched.
    if (seen) return
    const el = ref.current
    if (!el || typeof IntersectionObserver === 'undefined') {
      setSeen(true)
      return
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return
        setSeen(true)
        io.disconnect()
      },
      { rootMargin: NEARLY_ON_SCREEN }
    )
    io.observe(el)
    return () => io.disconnect()
  }, [ref, seen])

  return seen
}
