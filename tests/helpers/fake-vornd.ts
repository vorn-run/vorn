import fs from 'node:fs'
import net from 'node:net'
import path from 'node:path'
import {
  FrameSplitter,
  encodeFrame,
  KIND_BINARY,
  KIND_TEXT,
  APP_PROTOCOL
} from '../../packages/server/src/vornd-channel'
import type { EffectNote, HeldSession } from '../../packages/server/src/vornd-sessions'
import { encodeTerminalFrameV2 } from '../../packages/shared/src/terminal-frame'

/**
 * A stand-in for vornd's channel for the server, on a socket in `dataDir`,
 * announced there as vornd announces its own. It answers what a test sets and
 * can tell the server anything vornd can, as often as a test likes, which the
 * real vornd does only around a crash.
 */
let endpoints = 0

export class FakeVornd {
  readonly endpoint: string
  private server!: net.Server
  private conns = new Set<net.Socket>()
  /** What `vornd:subscribe` answers. */
  state: { connected: boolean; sessions: HeldSession[]; ended: HeldSession[]; notices: unknown[] } =
    { connected: true, sessions: [], ended: [], notices: [] }
  /** What `terminal:readOutput` answers. */
  output: string[] = []
  /** Every call the server made, in order. */
  calls: Array<{ method: string; params: Record<string, unknown> }> = []
  nextPid = 100
  /** What `vornd:hello` reports. */
  protocol = APP_PROTOCOL

  constructor(private readonly dataDir: string) {
    // Each its own endpoint: two made in the same millisecond must not share one.
    const n = ++endpoints
    this.endpoint =
      process.platform === 'win32'
        ? `\\\\.\\pipe\\vorn-app-test-${process.pid}-${n}`
        : path.join(dataDir, `app-${n}.sock`)
  }

  async start(): Promise<void> {
    this.server = net.createServer((socket) => {
      this.conns.add(socket)
      const frames = new FrameSplitter()
      socket.on('data', (chunk: Buffer) => {
        frames.push(chunk)
        for (let f = frames.next(); f; f = frames.next()) {
          if (f.kind === KIND_TEXT) this.answer(socket, JSON.parse(f.payload.toString()))
        }
      })
      socket.on('close', () => this.conns.delete(socket))
    })
    await new Promise<void>((resolve) => this.server.listen(this.endpoint, () => resolve()))
    fs.mkdirSync(path.join(this.dataDir, 'run'), { recursive: true })
    fs.writeFileSync(
      path.join(this.dataDir, 'run', 'vornd-app'),
      JSON.stringify({ pid: process.pid, endpoint: this.endpoint, protocol: APP_PROTOCOL })
    )
  }

  /** The calls made with `method`. */
  made(method: string): Array<Record<string, unknown>> {
    return this.calls.filter((c) => c.method === method).map((c) => c.params)
  }

  /** Tells every connection `method`. */
  send(method: string, params: unknown): void {
    const frame = encodeFrame(KIND_TEXT, Buffer.from(JSON.stringify({ method, params })))
    for (const c of this.conns) c.write(frame)
  }

  /** Sends record `rseq` of `id`'s output, as a bytes frame. */
  sendOutput(id: string, epoch: number, rseq: number, data: string): void {
    const bytes = encodeTerminalFrameV2({
      id,
      epoch,
      firstRseq: rseq,
      lastRseq: rseq,
      startOffset: 0,
      data: new TextEncoder().encode(data)
    })
    const frame = encodeFrame(KIND_BINARY, bytes)
    for (const c of this.conns) c.write(frame)
  }

  /** Drops every connection, as a vornd that died. */
  dropAll(): void {
    for (const c of this.conns) c.destroy()
  }

  async stop(): Promise<void> {
    this.dropAll()
    await new Promise<void>((resolve) => this.server.close(() => resolve()))
  }

  private answer(
    socket: net.Socket,
    msg: { id?: number; method: string; params?: Record<string, unknown> }
  ): void {
    this.calls.push({ method: msg.method, params: msg.params ?? {} })
    let result: unknown = null
    if (msg.method === 'vornd:hello') result = { protocol: this.protocol, build: 'test' }
    else if (msg.method === 'vornd:subscribe') result = this.state
    else if (msg.method === 'vornd:spawn') {
      result = { id: msg.params?.name, pid: this.nextPid++, epoch: 7 }
    } else if (msg.method === 'terminal:attach') {
      result = { live: true, continued: true, cursor: msg.params?.cursor }
    } else if (msg.method === 'terminal:readOutput') result = this.output
    if (msg.id === undefined) return
    socket.write(
      encodeFrame(KIND_TEXT, Buffer.from(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result })))
    )
  }
}

/** An effect as vornd tells it, in epoch 7. */
export function effect(
  id: string,
  kind: EffectNote['kind'],
  rseq: number,
  extra: object
): EffectNote {
  return { effectId: `${id}/7/${rseq}/0`, id, epoch: 7, rseq, index: 0, kind, ...extra }
}
