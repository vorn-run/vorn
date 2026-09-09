import WebSocket from 'ws'
import type { HandoffRequest, HandoffResult } from '@vornrun/shared/protocol'
import log from '../logger'

/**
 * Asking for a handoff over a socket of its own, without adopting anything.
 *
 * The main bridge cannot carry this request in the one case it matters most. A
 * release that changes `RUNTIME_PROTOCOL_VERSION` makes the incumbent
 * unadoptable, and `launchServer` refuses before it ever holds a bridge to ask
 * with -- so the terminals would sit on a server the new app cannot speak to,
 * which is the outcome the frozen handoff contract exists to prevent.
 *
 * So this speaks only the frozen part: a JSON-RPC frame naming `server:handoff`,
 * a bearer credential on the upgrade, and one reply. No greeting is read, no
 * capabilities are negotiated, and no version is compared -- a server that
 * disagrees about every other message still understands this one.
 */

/** Long, because the far side starts a whole server before it answers. */
const REPLY_TIMEOUT_MS = 90_000
const OPEN_TIMEOUT_MS = 10_000

export async function askForHandoff(
  target: string,
  credential: string,
  request: HandoffRequest
): Promise<HandoffResult> {
  const socket = new WebSocket(target, { headers: { authorization: `Bearer ${credential}` } })

  try {
    await once(socket, OPEN_TIMEOUT_MS, 'the server did not accept a connection')
    return await reply(socket, request)
  } finally {
    try {
      socket.close()
    } catch {
      // Already gone, which a successful handoff arranges moments later anyway.
    }
  }
}

function once(socket: WebSocket, timeoutMs: number, why: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(why)), timeoutMs)
    socket.once('open', () => {
      clearTimeout(timer)
      resolve()
    })
    socket.once('error', (err) => {
      clearTimeout(timer)
      reject(err)
    })
  })
}

function reply(socket: WebSocket, request: HandoffRequest): Promise<HandoffResult> {
  return new Promise((resolve, reject) => {
    const id = 1
    const timer = setTimeout(() => reject(new Error('the server never answered')), REPLY_TIMEOUT_MS)
    const settle = (fn: () => void): void => {
      clearTimeout(timer)
      socket.off('message', onMessage)
      fn()
    }

    const onMessage = (raw: WebSocket.RawData): void => {
      let frame: { id?: unknown; result?: unknown; error?: { message?: string } }
      try {
        frame = JSON.parse(String(raw))
      } catch {
        // Greetings and notifications share this socket; anything unreadable is not ours.
        return
      }
      if (frame.id !== id) return
      if (frame.error) return settle(() => reject(new Error(frame.error?.message ?? 'refused')))
      settle(() => resolve(frame.result as HandoffResult))
    }

    socket.on('message', onMessage)
    // The socket closing before the reply is the handoff having gone wrong on the
    // far side, not having succeeded: a successful one replies first, then exits.
    socket.once('close', () => settle(() => reject(new Error('the connection closed first'))))
    socket.send(JSON.stringify({ jsonrpc: '2.0', id, method: 'server:handoff', params: request }))
    log.info('[handoff] asked over a direct socket, without adopting')
  })
}
