import { IPC } from '@vornrun/shared/types'
import { clientRegistry } from '../broadcast'
import log from '../logger'

/**
 * What is selected in a terminal, which only the window drawing it knows.
 *
 * Every other host read is answered here. This one cannot be: the selection is
 * xterm's, in the renderer, and the server holds bytes rather than a screen. So
 * the request goes out as a notification and the answer comes back as one,
 * matched by id — the same shape the browser bridge uses, without needing the
 * one socket the desktop's main process holds.
 */

interface Pending {
  resolve: (text: string) => void
  timer: NodeJS.Timeout
}

/** Long enough for a window that is busy painting, short enough not to hold a footer's turn. */
const SELECTION_TIMEOUT_MS = 15_000

const pending = new Map<number, Pending>()
let nextId = 0

/** An unanswered request reads as no selection: a window that cannot say is a window with none. */
export function requestSelection(sessionId: string): Promise<string> {
  if (clientRegistry.size === 0) return Promise.resolve('')
  const requestId = ++nextId
  return new Promise<string>((resolve) => {
    const timer = setTimeout(() => {
      pending.delete(requestId)
      resolve('')
    }, SELECTION_TIMEOUT_MS)
    pending.set(requestId, { resolve, timer })
    clientRegistry.broadcast(IPC.EXTENSION_SELECTION_REQUEST, { requestId, sessionId }, sessionId)
  })
}

/** The first window to answer wins; the rest have nothing left to resolve. */
export function resolveSelection(requestId: number, text: string): void {
  const waiting = pending.get(requestId)
  if (!waiting) return
  pending.delete(requestId)
  clearTimeout(waiting.timer)
  waiting.resolve(typeof text === 'string' ? text : '')
}

export function abandonSelections(): void {
  for (const [id, waiting] of pending) {
    clearTimeout(waiting.timer)
    waiting.resolve('')
    pending.delete(id)
  }
  log.info('[extensions] released every pending selection request')
}
