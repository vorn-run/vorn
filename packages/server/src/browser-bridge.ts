import type { WebSocket } from 'ws'
import { createRequest } from '@vornrun/shared/protocol'
import type { RequestMethod, RequestMethods, RpcResponse } from '@vornrun/shared/protocol'
import log from './logger'

/**
 * Reverse RPC: the server asking the Electron main process a question.
 *
 * Every other method in this package is answered here, in the server process.
 * The browser ones cannot be: a `<webview>` guest and its CDP debugger exist
 * only in main, and there is no handle on either from out here. So the server
 * relays `browser:*` back over the *same socket* main opened to reach it, and
 * waits for main's response.
 *
 * The bridge socket is identified by the first client that registers itself as
 * main. That is deliberately narrow: renderer clients share this WS server, and
 * a request routed to one of them would hang until it timed out rather than
 * fail, which is a much worse way to learn the wiring is wrong.
 */

interface Pending {
  resolve: (v: unknown) => void
  reject: (e: Error) => void
  timer: NodeJS.Timeout
}

const DEFAULT_TIMEOUT_MS = 15_000

/**
 * Ids are negative so they cannot collide with the ids main assigns to its own
 * outbound requests on the same socket. Both directions share one id space;
 * without the split, a reply could be matched to the wrong pending call.
 */
let nextId = 0

class BrowserBridge {
  private socket: WebSocket | null = null
  private pending = new Map<number, Pending>()

  /** Called when main identifies itself, so we know which socket to ask. */
  setSocket(ws: WebSocket): void {
    this.socket = ws
    log.info('[browser-bridge] main process registered')
  }

  clearSocket(ws: WebSocket): void {
    if (this.socket !== ws) return
    this.socket = null
    for (const [id, p] of Array.from(this.pending)) {
      clearTimeout(p.timer)
      p.reject(new Error('Vorn main process disconnected'))
      this.pending.delete(id)
    }
  }

  get isConnected(): boolean {
    return this.socket !== null && this.socket.readyState === this.socket.OPEN
  }

  /** Resolve a reply that came back from main. Returns false if it wasn't ours. */
  handleResponse(msg: RpcResponse): boolean {
    const id = typeof msg.id === 'number' ? msg.id : Number(msg.id)
    const p = this.pending.get(id)
    if (!p) return false
    this.pending.delete(id)
    clearTimeout(p.timer)
    if (msg.error) p.reject(new Error(msg.error.message))
    else p.resolve(msg.result)
    return true
  }

  async request<M extends RequestMethod>(
    method: M,
    params: RequestMethods[M]['params'],
    timeoutMs = DEFAULT_TIMEOUT_MS
  ): Promise<RequestMethods[M]['result']> {
    const ws = this.socket
    if (!ws || ws.readyState !== ws.OPEN) {
      throw new Error('Vorn app is not running (no main process connected)')
    }
    const id = --nextId
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`Browser request timed out: ${method}`))
      }, timeoutMs)
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, timer })
      ws.send(JSON.stringify(createRequest(id, method, params)))
    })
  }
}

export const browserBridge = new BrowserBridge()
