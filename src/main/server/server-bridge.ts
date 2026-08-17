import WebSocket from 'ws'
import { EventEmitter } from 'node:events'
import type { RpcResponse, RpcNotification } from '@vornrun/shared/protocol'
import { createRequest, createNotification } from '@vornrun/shared/protocol'
import log from '../logger'

interface PendingRequest {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timeout: NodeJS.Timeout
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
  private reconnectTimer: NodeJS.Timeout | null = null
  private shouldReconnect = true
  private inbound = new Map<string, (params: unknown) => unknown>()

  constructor(url: string) {
    super()
    this.url = url
  }

  connect(): void {
    if (this.ws) return

    this.ws = new WebSocket(this.url)

    this.ws.on('open', () => {
      log.info('[bridge] connected to server')
      // Tell the server which socket is main's. Browser tools arrive at the
      // server but can only be answered here — a `<webview>` guest is
      // unreachable from that process — so it needs to know where to relay them.
      this.notify('bridge:identify')
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
            // Recorded in the log only. Pass B adds the field that gates on a
            // capability, at the point where something actually reads it.
            log.info({ hello: (msg as RpcNotification).params }, '[bridge] server protocol')
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
      const timeout = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`Request timed out: ${method} (id=${id})`))
      }, timeoutMs)

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
