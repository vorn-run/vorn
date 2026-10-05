import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { createServer } from 'node:http'
import { createInterface } from 'node:readline'
import WebSocket, { WebSocketServer } from 'ws'
import { Terminal as Headless } from '@xterm/headless'
import type { Terminal } from '@xterm/xterm'
import type { RecordCursor } from '../../packages/shared/src/types'
import { decodeTerminalFrameV2, frameResume } from '../../packages/shared/src/terminal-frame'
import { swallowQueries } from '../../src/renderer/lib/vornd-replies'

/**
 * Sessions held by vornd, for tests that go through the real binaries.
 *
 * The conformance run (`yarn test:conformance`) builds vornd and vorn-sessiond
 * and names them in `VORN_CONFORMANCE_VORND` and `VORN_CONFORMANCE_SESSIOND`.
 * Until the app creates its sessions through vornd, a test starts them with
 * `vornd:spawn`, which vornd answers only with `--debug-spawn`.
 */

const vorndBinary = process.env.VORN_CONFORMANCE_VORND
const sessiondBinary =
  process.env.VORN_CONFORMANCE_SESSIOND ??
  (vorndBinary
    ? path.join(
        path.dirname(vorndBinary),
        process.platform === 'win32' ? 'vorn-sessiond.exe' : 'vorn-sessiond'
      )
    : undefined)

/** Whether this run can start vornd with a session holder: both binaries, on a Unix. */
export const vorndSessionsAvailable =
  process.platform !== 'win32' &&
  !!vorndBinary &&
  existsSync(vorndBinary) &&
  !!sessiondBinary &&
  existsSync(sessiondBinary)

const PATIENCE_MS = 15_000

/** Polls `check` until it holds. */
export async function until(what: string, check: () => boolean | Promise<boolean>): Promise<void> {
  const start = Date.now()
  while (!(await check())) {
    if (Date.now() - start > PATIENCE_MS) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 20))
  }
}

/**
 * A server for vornd to stand in front of. vornd opens a client's socket only
 * once the server accepted the same upgrade; for sessions vornd holds, that is
 * all the server does.
 */
export async function upstream(): Promise<{ port: number; close(): void }> {
  // Its health route answers too, so vornd's start-up check finds it up.
  const http = createServer((_, res) => res.end('ok'))
  const server = new WebSocketServer({ server: http })
  await new Promise<void>((resolve) => http.listen(0, '127.0.0.1', () => resolve()))
  const address = http.address()
  return {
    port: typeof address === 'object' && address ? address.port : 0,
    close: () => {
      server.close()
      http.close()
    }
  }
}

/** A vornd with a session holder under `home`. */
export class Vornd {
  private constructor(
    private readonly child: ChildProcess,
    readonly port: number
  ) {}

  /** With `desktopToken`, a connection that opens with it is the desktop's. */
  static async start(upstreamPort: number, home: string, desktopToken?: string): Promise<Vornd> {
    const child = spawn(
      vorndBinary!,
      [
        '--upstream',
        `127.0.0.1:${upstreamPort}`,
        '--sessiond',
        sessiondBinary!,
        '--home',
        home,
        '--debug-spawn'
      ],
      {
        stdio: ['ignore', 'pipe', 'inherit'],
        env: {
          ...process.env,
          VORND_LOG: process.env.VORND_LOG ?? 'warn',
          ...(desktopToken ? { VORND_DESKTOP_TOKEN: desktopToken } : {})
        }
      }
    )
    const port = await new Promise<number>((resolve, reject) => {
      const lines = createInterface({ input: child.stdout! })
      lines.once('line', (line) => resolve((JSON.parse(line) as { port: number }).port))
      child.once('exit', (code) => reject(new Error(`vornd exited with ${code}`)))
    })
    const v = new Vornd(child, port)
    await until('vornd to hold sessions', async () => (await v.report()).connected === true)
    return v
  }

  async report(): Promise<{
    connected: boolean
    sessions: Array<{ session: string; state: string }>
  }> {
    const res = await fetch(`http://127.0.0.1:${this.port}/vornd/sessions`)
    return res.ok ? res.json() : { connected: false, sessions: [] }
  }

  /** The pid of the session holder it keeps, which outlives it. */
  async sessiondPid(): Promise<number | null> {
    const res = await fetch(`http://127.0.0.1:${this.port}/vornd/health`)
    const health = (await res.json()) as { sessiond?: { current?: { pid: number } } }
    return health.sessiond?.current?.pid ?? null
  }

  /** Killed, as a crash would: no last checkpoint, no goodbye. */
  async kill(): Promise<void> {
    if (this.child.exitCode !== null || this.child.signalCode !== null) return
    await new Promise<void>((resolve) => {
      this.child.once('exit', () => resolve())
      this.child.kill('SIGKILL')
    })
  }
}

/** A data directory for one test, and its cleanup. */
export function home(): { dir: string; remove(): void } {
  const dir = mkdtempSync(path.join(tmpdir(), 'vornd-sessions-'))
  return { dir, remove: () => rmSync(dir, { recursive: true, force: true }) }
}

interface Pending {
  resolve(v: unknown): void
  reject(e: Error): void
}

/**
 * A bytes client the way the renderer is one: xterm.js fed the attach answer,
 * then every frame and `terminal:resized` in order, its query replies left to
 * vornd and everything else it sends going back as `terminal:write`.
 */
export class BytesClient {
  readonly term = new Headless({ allowProposedApi: true, cols: 80, rows: 24, scrollback: 10_000 })
  cursor: RecordCursor | null = null
  /** Every `terminal:resized`, as it came. */
  resized: Array<{
    cols: number
    rows: number
    rseq: number
    owner?: string | null
    reason?: string | null
  }> = []
  /** This connection's name in vornd, from the attach answer. */
  name: string | null = null
  resyncs: string[] = []
  exits: number[] = []
  /** Set when a frame did not start at the cursor: a byte lost or doubled. */
  broken: string | null = null
  private ws!: WebSocket
  private nextId = 0
  private pending = new Map<number, Pending>()
  /** Written into the terminal in order, even when frames come faster than xterm parses. */
  private queue: Promise<void> = Promise.resolve()
  session = ''

  constructor() {
    swallowQueries(this.term as unknown as Terminal, () => this.session !== '')
    this.term.onData((data) => {
      if (this.session) this.notify('terminal:write', { id: this.session, data })
    })
  }

  /** Opens with `token` as its bearer credential, as the desktop does, when given one. */
  async connect(port: number, token?: string): Promise<void> {
    this.ws = new WebSocket(
      `ws://127.0.0.1:${port}/ws`,
      token ? { headers: { Authorization: `Bearer ${token}` } } : {}
    )
    this.ws.binaryType = 'nodebuffer'
    this.ws.on('message', (raw: Buffer, isBinary: boolean) => this.receive(raw, isBinary))
    await new Promise<void>((resolve, reject) => {
      this.ws.once('open', () => resolve())
      this.ws.once('error', reject)
    })
  }

  close(): void {
    this.ws?.close()
  }

  call<T = unknown>(method: string, params: unknown): Promise<T> {
    const id = ++this.nextId
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject })
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  notify(method: string, params: unknown): void {
    this.ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
  }

  /** Starts a session in vornd's holder; answers its id. */
  async spawn(argv: string[], cols = 80, rows = 24): Promise<string> {
    return (await this.call<{ id: string }>('vornd:spawn', { argv, cols, rows })).id
  }

  /** Attaches `session`, from this client's cursor when `resume` is set. */
  async attach(
    session: string,
    resume = false
  ): Promise<{ continued: boolean; cursor: RecordCursor; resync?: string; replies?: string }> {
    this.session = session
    const params = resume && this.cursor ? { id: session, cursor: this.cursor } : { id: session }
    const answer = await this.call<{
      data: string
      continued: boolean
      cursor: RecordCursor
      cols?: number
      rows?: number
      resync?: string
      replies?: string
      client?: string
    }>('terminal:attach', params)
    this.name = answer.client ?? null
    if (!answer.continued) {
      this.enqueue(() => {
        this.term.reset()
        if (answer.cols && answer.rows) this.term.resize(answer.cols, answer.rows)
      })
      this.enqueueWrite(answer.data)
    }
    this.cursor = answer.cursor
    return answer
  }

  /** Everything the terminal shows, scrollback included, once all written is parsed. */
  async text(): Promise<string> {
    await this.queue
    const buf = this.term.buffer.active
    const lines: string[] = []
    for (let y = 0; y < buf.length; y++) {
      const line = buf.getLine(y)
      if (!line) continue
      if (line.isWrapped && lines.length) lines[lines.length - 1] += line.translateToString(true)
      else lines.push(line.translateToString(true))
    }
    while (lines.length && lines[lines.length - 1] === '') lines.pop()
    return lines.join('\n')
  }

  private enqueue(f: () => void): void {
    this.queue = this.queue.then(f)
  }

  private enqueueWrite(data: string | Uint8Array): void {
    this.queue = this.queue.then(
      () => new Promise<void>((resolve) => this.term.write(data, resolve))
    )
  }

  private receive(raw: Buffer, isBinary: boolean): void {
    if (isBinary) {
      const frame = decodeTerminalFrameV2(new Uint8Array(raw))
      if (!frame || frame.id !== this.session) return
      const at = this.cursor
      if (!at || frame.firstRseq !== at.nextRseq || frame.startOffset !== at.nextOffset) {
        this.broken ??= `frame ${frame.firstRseq}@${frame.startOffset} after ${JSON.stringify(at)}`
      }
      this.cursor = frameResume(frame)
      this.enqueueWrite(frame.data)
      return
    }
    const msg = JSON.parse(raw.toString()) as {
      id?: number
      method?: string
      params?: Record<string, unknown>
      result?: unknown
      error?: { message: string }
    }
    if (msg.id !== undefined) {
      const p = this.pending.get(msg.id)
      this.pending.delete(msg.id)
      if (msg.error) p?.reject(new Error(msg.error.message))
      else p?.resolve(msg.result)
      return
    }
    const params = msg.params ?? {}
    if (params.id !== this.session) return
    if (msg.method === 'terminal:resized') {
      const resized = params as BytesClient['resized'][number]
      this.resized.push(resized)
      const { cols, rows, rseq } = resized
      if (this.cursor) this.cursor = { ...this.cursor, nextRseq: rseq + 1 }
      this.enqueue(() => this.term.resize(cols, rows))
    } else if (msg.method === 'terminal:resync') {
      this.resyncs.push(String(params.reason))
    } else if (msg.method === 'terminal:exit') {
      this.exits.push(Number(params.exitCode))
    }
  }
}

/** A process by pid, ended if it is still there. */
export function killPid(pid: number | null): void {
  if (!pid) return
  try {
    process.kill(pid, 'SIGTERM')
  } catch {
    /* already gone */
  }
}
