/**
 * A grid-mode connection to vornd's `/grid` WebSocket: the screen as cells,
 * not bytes. Opt-in (see `isGridEnabled` in env.ts); the bytes path in
 * api-shim.ts stays the default. A dropped socket is not reopened: the
 * caller opens a new connection and attaches again, resuming from its mirror.
 */

import { CLOSE_CREDENTIAL_REJECTED } from '@vornrun/shared/protocol'
import type { Uint } from './cbor'
import {
  CAPS,
  ClientKind,
  FrameReader,
  PROTO_MAJOR,
  PROTO_MINOR,
  decodeServer,
  encodeClient,
  type ClientMsg,
  type InputEvent,
  type ServerMsg,
  type Size
} from './protocol'

/** The part of a WebSocket this uses, so a test can stand in for one. */
export interface GridSocket {
  binaryType: string
  readonly readyState: number
  send(data: string | Uint8Array): void
  close(code?: number, reason?: string): void
  onopen: ((ev: unknown) => void) | null
  onmessage: ((ev: { data: unknown }) => void) | null
  onclose: ((ev: { code: number; reason: string }) => void) | null
  onerror: ((ev: unknown) => void) | null
}

export interface GridConnectionOptions {
  /** The device or local token, sent as the first message. */
  token: string
  /** Names this build in the Hello. */
  build?: string
  onMessage: (msg: ServerMsg) => void
  /** The token was refused; another will not be accepted on this socket. */
  onAuthRequired?: () => void
  onClose?: (code: number, reason: string) => void
  openSocket?: (url: string) => GridSocket
}

export class GridConnection {
  private ws: GridSocket
  private reader = new FrameReader()
  private opts: GridConnectionOptions
  private admitted = false
  private inputSeq = 0
  private closed = false

  constructor(url: string, opts: GridConnectionOptions) {
    this.opts = opts
    this.ws = (opts.openSocket ?? ((u: string) => new WebSocket(u) as unknown as GridSocket))(url)
    this.ws.binaryType = 'arraybuffer'
    const auth = JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'auth:authenticate',
      params: { token: opts.token }
    })
    // A browser cannot set headers on the upgrade, so the token goes first.
    this.ws.onopen = () => this.ws.send(auth)
    this.ws.onmessage = (ev) => this.receive(ev.data)
    this.ws.onclose = (ev) => {
      this.closed = true
      if (ev.code === CLOSE_CREDENTIAL_REJECTED) opts.onAuthRequired?.()
      opts.onClose?.(ev.code, ev.reason)
    }
    this.ws.onerror = () => {}
  }

  /** Whether vornd took the token and the grid protocol is running. */
  get ready(): boolean {
    return this.admitted && !this.closed
  }

  attach(session: string, view: Size, visible = true, historyTail = 0): void {
    this.send({ type: 'attach', session, view, visible, historyTail })
  }

  detach(sid: number): void {
    this.send({ type: 'detach', sid })
  }

  /** Sends one input event; returns its sequence number, echoed in `inputAck`. */
  input(sid: number, event: InputEvent): number {
    const inputSeq = ++this.inputSeq
    this.send({ type: 'input', sid, inputSeq, event })
    return inputSeq
  }

  /** Reports this view's size; vornd resizes the session if this view owns it. */
  resize(sid: number, size: Size): void {
    this.send({ type: 'viewport', sid, size })
  }

  /** Makes this view's size the session's. */
  takeSize(sid: number): void {
    this.send({ type: 'takeSize', sid })
  }

  send(msg: ClientMsg): void {
    if (!this.ready) throw new Error('the grid connection is not open')
    this.ws.send(encodeClient(msg))
  }

  close(): void {
    this.ws.close(1000)
  }

  private receive(data: unknown): void {
    if (typeof data === 'string') {
      this.control(data)
      return
    }
    if (!(data instanceof ArrayBuffer) && !ArrayBuffer.isView(data)) return
    const bytes =
      data instanceof ArrayBuffer
        ? new Uint8Array(data)
        : new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
    try {
      this.reader.push(bytes)
      for (let f = this.reader.next(); f; f = this.reader.next()) {
        const msg = decodeServer(f.kind, f.payload)
        if (!msg) continue
        // Two frames may be unacknowledged; acking each one keeps them coming.
        if (msg.type === 'snapshot') this.ack(msg.sid, msg.snap.rev)
        else if (msg.type === 'delta') this.ack(msg.sid, msg.delta.rev)
        this.opts.onMessage(msg)
      }
    } catch {
      // vornd would close a peer that broke framing; this side does the same.
      this.ws.close(1002, 'protocol error')
    }
  }

  private ack(sid: number, rev: Uint): void {
    this.ws.send(encodeClient({ type: 'ack', sid, rev }))
  }

  private control(text: string): void {
    let msg: { method?: string }
    try {
      msg = JSON.parse(text)
    } catch {
      return
    }
    if (msg.method !== 'auth:ok' || this.admitted) return
    this.admitted = true
    this.send({
      type: 'hello',
      major: PROTO_MAJOR,
      minor: PROTO_MINOR,
      caps: CAPS.ALL,
      client: ClientKind.Web,
      build: this.opts.build ?? 'web'
    })
  }
}
