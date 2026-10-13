import { PHONE_BASE_TOPICS, terminalTopic } from '../../shared/topics'
import { isWeb } from './platform'

/** Long enough to fold a scroll's worth of edges into one message. */
const SETTLE_MS = 100

const visible = new Set<string>()
let timer: ReturnType<typeof setTimeout> | null = null

/**
 * Which terminals' bytes this client asks the server for.
 *
 * The desktop asks for nothing and so is sent everything; only the web client
 * narrows its socket, and it narrows it to the cards that are on screen. The
 * set is pushed whole each time, because `subscribe:set` replaces the filter
 * rather than editing it.
 */
function schedule(): void {
  if (!isWeb) return
  if (timer) clearTimeout(timer)
  timer = setTimeout(() => {
    timer = null
    const api = window.api as { setTopics?: (topics: readonly string[]) => Promise<void> }
    if (typeof api?.setTopics !== 'function') return
    void api.setTopics([...PHONE_BASE_TOPICS, ...[...visible].map(terminalTopic)]).catch(() => {})
  }, SETTLE_MS)
}

export function markVisible(id: string): void {
  if (visible.has(id)) return
  visible.add(id)
  schedule()
}

export function markHidden(id: string): void {
  if (!visible.delete(id)) return
  schedule()
}

/** Test-only. */
export function resetVisible(): void {
  visible.clear()
  if (timer) clearTimeout(timer)
  timer = null
}
