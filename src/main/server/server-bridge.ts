import WebSocket from 'ws'
import { EventEmitter } from 'node:events'
import type { RpcResponse, RpcNotification, ServerIdentity } from '@vornrun/shared/protocol'
import { createRequest, createNotification } from '@vornrun/shared/protocol'
import log from '../logger'

interface PendingRequest {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timeout: NodeJS.Timeout | undefined
  method: string
}

/**
 * WebSocket client that bridges the standalone server to the Electron main process.
 *
 * - `request(method, params)` sends a JSON-RPC request and returns a Promise.
 * - `notify(method, params)` sends a fire-and-forget notification.
 * - Emits 'server-notification' for each push from the server.
 * - Emits 'connected' / 'disconnected' for lifecycle.
 */
export class ServerBridge extends EventEmitter {
  private ws: WebSocket | null = null
  private nextId = 0
  private pending = new Map<number, PendingRequest>()
  private url: string
  private credential: string | undefined
  private reconnectTimer: NodeJS.Timeout | null = null
  private shouldReconnect = true
  private inbound = new Map<string, (params: unknown) => unknown>()
  /**
   * Who the server said it is, once it has said so.
   *
   * Follows the greeting, before authentication, and only on loopback — so a
   * caller deciding whether to adopt this server can read it without first
   * proving who it is, and a stranger on the network never sees it at all.
   */
  private identity: ServerIdentity | null = null
  /** From the greeting, which every socket receives. Undefined until it lands. */
  private helloVersion: number | undefined

  get serverIdentity(): ServerIdentity | null {
    return this.identity
  }

  get serverHelloVersion(): number | undefined {
    return this.helloVersion
  }

  constructor(url: string, credential?: string) {
    super()
    this.url = url
    this.credential = credential
  }

  /** Where this bridge is pointed, so a caller can tell whether it has moved. */
  target(): string {
    return this.url
  }

  /**
   * Point at a different port and reconnect there.
   *
   * A restarted server usually comes back on the port it had, and the reconnect
   * loop finds it without help. This is for when it does not — the old port
   * taken in the moment between the two — where the loop would otherwise retry a
   * stale address forever, which looks exactly like a server that never came
   * back.
   */
  retarget(url: string): void {
    if (url === this.url) return
    this.url = url

    // The old socket's listeners come off before it is closed. `close` is
    // asynchronous, so its handler would otherwise run after the new socket
    // exists and set `this.ws = null` on it -- leaving the bridge holding no
    // reference to a connection that is open, rejecting whatever was in flight,
    // and scheduling a reconnect on top of the one just made.
    const previous = this.ws
    this.ws = null
    if (previous) {
      previous.removeAllListeners()
      // One listener goes straight back on. An EventEmitter with nothing
      // listening for `error` throws when one is emitted, and a socket being
      // torn down is exactly when that happens -- closing mid-CONNECTING, or a
      // reconnect that was already failing. Detaching everything would turn a
      // socket's last gasp into an uncaught exception, which is the failure this
      // whole branch is about. It has nothing left to tell us; it just must not
      // throw on the way out.
      previous.on('error', () => {})
      previous.close()
      // Said here rather than left to each request's own timeout: they were sent
      // on a socket that is gone, and the answer is never coming.
      this.rejectAllPending('Server moved')
    }

    // A reconnect already queued would fire into the new socket and early-return,
    // but it would also be a second attempt nobody asked for.
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }

    this.connect()
  }

  connect(): void {
    if (this.ws) return

    // Sent on the upgrade rather than as a first message, so the socket is
    // authenticated before it sends anything. Re-supplied here on purpose:
    // `connect()` runs again on every reconnect.
    this.ws = new WebSocket(this.url, {
      headers: this.credential ? { Authorization: `Bearer ${this.credential}` } : {}
    })

    this.ws.on('open', () => {
      log.info('[bridge] connected to server')
      // Tell the server which socket is main's. Browser tools arrive at the
      // server but can only be answered here — a `<webview>` guest is
      // unreachable from that process — so it needs to know where to relay them.
      //
      // Sent as a request, not a notification: the server can refuse the claim,
      // and a silent refusal would leave main believing it holds the bridge while
      // every browser and device tool times out after 15 seconds.
      this.request<{ ok: boolean }>('bridge:identify')
        .then((result) => {
          if (!result?.ok) {
            log.error(
              '[bridge] refused the browser bridge claim — browser and device tools will not work'
            )
          }
        })
        .catch((err) => log.warn({ err }, '[bridge] could not claim the browser bridge'))
      this.emit('connected')
    })

    this.ws.on('message', (raw: Buffer) => {
      try {
        const msg = JSON.parse(raw.toString())
        if ('method' in msg && 'id' in msg && msg.id !== undefined && msg.id !== null) {
          // An inbound *request*: the server asking us something only main can
          // answer. Distinguished from a response by carrying a method.
          void this.handleInbound(msg as { id: number | string; method: string; params?: unknown })
        } else if ('id' in msg && msg.id !== undefined) {
          this.handleResponse(msg as RpcResponse)
        } else if ('method' in msg) {
          if (msg.method === 'server:hello') {
            const hello = (msg as RpcNotification).params as { protocolVersion?: number }
            this.helloVersion = hello?.protocolVersion
            log.info({ hello }, '[bridge] server protocol')
          }
          if (msg.method === 'server:identity') {
            this.identity = (msg as RpcNotification).params as ServerIdentity
            this.emit('identity', this.identity)
          }
          this.emit('server-notification', msg.method, (msg as RpcNotification).params)
        }
      } catch {
        log.warn('[bridge] failed to parse server message')
      }
    })

    this.ws.on('close', () => {
      log.info('[bridge] disconnected from server')
      this.ws = null
      this.rejectAllPending('Server disconnected')
      this.emit('disconnected')
      if (this.shouldReconnect) {
        this.scheduleReconnect()
      }
    })

    this.ws.on('error', (err) => {
      log.error({ err }, '[bridge] WebSocket error')
    })
  }

  async request<T = unknown>(method: string, params?: unknown, timeoutMs = 30_000): Promise<T> {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error(`Cannot send request: not connected (method=${method})`)
    }

    const id = ++this.nextId
    return new Promise<T>((resolve, reject) => {
      // A zero deadline waits as long as the server takes; a disconnect still rejects.
      const timeout =
        timeoutMs > 0
          ? setTimeout(() => {
              this.pending.delete(id)
              reject(new Error(`Request timed out: ${method} (id=${id})`))
            }, timeoutMs)
          : undefined

      this.pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
        timeout,
        method
      })

      this.ws!.send(JSON.stringify(createRequest(id, method as never, params as never)))
    })
  }

  notify(method: string, params?: unknown): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      log.warn(`[bridge] cannot notify: not connected (method=${method})`)
      return
    }
    this.ws.send(JSON.stringify(createNotification(method, params)))
  }

  close(): void {
    this.shouldReconnect = false
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.rejectAllPending('Bridge closing')
    if (this.ws) {
      this.ws.close()
      this.ws = null
    }
  }

  get isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN
  }

  /**
   * Answer a method only main can answer.
   *
   * Registered by `registerInboundHandlers` at startup. Anything unregistered
   * is refused rather than ignored: a silent drop would leave the server's
   * caller waiting out its full timeout for a method that will never exist.
   */
  handle(method: string, handler: (params: unknown) => unknown): void {
    this.inbound.set(method, handler)
  }

  private async handleInbound(msg: {
    id: number | string
    method: string
    params?: unknown
  }): Promise<void> {
    const send = (body: Record<string, unknown>): void => {
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.ws.send(JSON.stringify({ jsonrpc: '2.0', id: msg.id, ...body }))
      }
    }
    const handler = this.inbound.get(msg.method)
    if (!handler) {
      send({ error: { code: -32601, message: `Method not found in main: ${msg.method}` } })
      return
    }
    try {
      send({ result: await handler(msg.params) })
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      log.warn({ err, method: msg.method }, '[bridge] inbound handler error')
      send({ error: { code: -32000, message } })
    }
  }

  private handleResponse(msg: RpcResponse): void {
    const id = typeof msg.id === 'number' ? msg.id : parseInt(String(msg.id), 10)
    const pending = this.pending.get(id)
    if (!pending) return

    this.pending.delete(id)
    clearTimeout(pending.timeout)

    if (msg.error) {
      pending.reject(new Error(msg.error.message))
    } else {
      pending.resolve(msg.result)
    }
  }

  private rejectAllPending(reason: string): void {
    for (const [id, pending] of Array.from(this.pending)) {
      clearTimeout(pending.timeout)
      pending.reject(new Error(`${reason} (method=${pending.method}, id=${id})`))
    }
    this.pending.clear()
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      log.info('[bridge] attempting reconnect...')
      this.connect()
    }, 2000)
  }
}
