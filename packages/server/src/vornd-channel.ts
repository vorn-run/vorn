import net from 'node:net'
import { EventEmitter } from 'node:events'

/**
 * The server's own channel to vornd, the native daemon.
 *
 * A local socket only this user can open (a Unix socket in the data
 * directory's `run/`, or a named pipe on Windows), which vornd names in
 * `run/vornd-app`. Each frame is a 4-byte little-endian length, then a kind
 * byte, then the payload; the length counts the kind byte. Kind 1 is JSON-RPC,
 * the same calls a client sends, and kind 2 a terminal's bytes frame.
 */

export const KIND_TEXT = 1
export const KIND_BINARY = 2
/** The largest frame either side accepts. */
export const MAX_FRAME = 128 * 1024 * 1024

/** The version of the channel this server speaks; `vornd:hello` reports vornd's. */
export const APP_PROTOCOL = 1

const REQUEST_TIMEOUT_MS = 10_000

interface Pending {
  resolve(result: unknown): void
  reject(err: Error): void
  timer: ReturnType<typeof setTimeout>
}

/** Splits a byte stream into frames, whatever sizes its chunks arrive in. */
export class FrameSplitter {
  private chunks: Buffer[] = []
  private held = 0

  push(chunk: Buffer): void {
    this.chunks.push(chunk)
    this.held += chunk.length
  }

  /** The next whole frame, or null while one is still arriving. Throws on a frame no peer sends. */
  next(): { kind: number; payload: Buffer } | null {
    if (this.held < 4) return null
    const head = this.chunks.length === 1 ? this.chunks[0]! : this.join()
    const len = head.readUInt32LE(0)
    if (len === 0 || len > MAX_FRAME) throw new Error(`a frame of ${len} bytes`)
    if (this.held < 4 + len) return null
    const all = this.join()
    const kind = all[4]!
    const payload = Buffer.from(all.subarray(5, 4 + len))
    const rest = all.subarray(4 + len)
    this.chunks = rest.length ? [rest] : []
    this.held = rest.length
    return { kind, payload }
  }

  private join(): Buffer {
    const all = this.chunks.length === 1 ? this.chunks[0]! : Buffer.concat(this.chunks)
    this.chunks = [all]
    return all
  }
}

/** One frame as it goes on the wire. */
export function encodeFrame(kind: number, payload: Uint8Array): Buffer {
  const out = Buffer.allocUnsafe(5 + payload.length)
  out.writeUInt32LE(payload.length + 1, 0)
  out[4] = kind
  out.set(payload, 5)
  return out
}

/**
 * A connected channel.
 *
 * Emits `notification` (method, params) for every call vornd sends, `frame`
 * (bytes) for every bytes frame, and `close` (why) once, when the channel ends.
 */
export class VorndChannel extends EventEmitter {
  private nextId = 1
  private readonly pending = new Map<number, Pending>()
  private closed = false

  private constructor(private readonly socket: net.Socket) {
    super()
    const frames = new FrameSplitter()
    socket.on('data', (chunk: Buffer) => {
      frames.push(chunk)
      try {
        for (let f = frames.next(); f; f = frames.next()) this.received(f.kind, f.payload)
      } catch (err) {
        this.end((err as Error).message)
      }
    })
    socket.on('error', (err) => this.end(err.message))
    socket.on('close', () => this.end('vornd closed the channel'))
  }

  /** Connects to `endpoint`, giving up after `timeoutMs`. */
  static connect(endpoint: string, timeoutMs = 2_000): Promise<VorndChannel> {
    return new Promise((resolve, reject) => {
      const socket = net.connect(endpoint)
      const timer = setTimeout(() => {
        socket.destroy()
        reject(new Error(`vornd did not answer on ${endpoint}`))
      }, timeoutMs)
      socket.once('error', (err) => {
        clearTimeout(timer)
        reject(err)
      })
      socket.once('connect', () => {
        clearTimeout(timer)
        socket.removeAllListeners('error')
        resolve(new VorndChannel(socket))
      })
    })
  }

  get isClosed(): boolean {
    return this.closed
  }

  request<T>(method: string, params?: unknown, timeoutMs = REQUEST_TIMEOUT_MS): Promise<T> {
    if (this.closed) return Promise.reject(new Error('the channel to vornd is closed'))
    const id = this.nextId++
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`vornd did not answer ${method}`))
      }, timeoutMs)
      this.pending.set(id, { resolve: resolve as (r: unknown) => void, reject, timer })
      this.send({ jsonrpc: '2.0', id, method, ...(params === undefined ? {} : { params }) })
    })
  }

  notify(method: string, params?: unknown): void {
    if (this.closed) return
    this.send({ jsonrpc: '2.0', method, ...(params === undefined ? {} : { params }) })
  }

  close(): void {
    this.socket.destroy()
    this.end('closed by this server')
  }

  private send(message: unknown): void {
    this.socket.write(encodeFrame(KIND_TEXT, Buffer.from(JSON.stringify(message))))
  }

  private received(kind: number, payload: Buffer): void {
    if (kind === KIND_BINARY) {
      this.emit('frame', new Uint8Array(payload.buffer, payload.byteOffset, payload.length))
      return
    }
    if (kind !== KIND_TEXT) return
    let message: {
      id?: number
      method?: string
      params?: unknown
      result?: unknown
      error?: { message?: string }
    }
    try {
      message = JSON.parse(payload.toString('utf8'))
    } catch {
      return
    }
    if (typeof message.method === 'string') {
      this.emit('notification', message.method, message.params)
      return
    }
    if (typeof message.id !== 'number') return
    const waiting = this.pending.get(message.id)
    if (!waiting) return
    this.pending.delete(message.id)
    clearTimeout(waiting.timer)
    if (message.error) waiting.reject(new Error(message.error.message ?? 'vornd refused'))
    else waiting.resolve(message.result)
  }

  private end(why: string): void {
    if (this.closed) return
    this.closed = true
    for (const p of this.pending.values()) {
      clearTimeout(p.timer)
      p.reject(new Error(`the channel to vornd closed: ${why}`))
    }
    this.pending.clear()
    this.emit('close', why)
  }
}
