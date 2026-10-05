import type { WebSocket } from 'ws'
import type { ManagedPty } from '../../packages/server/src/handoff/adopted-pty'
import {
  setNativeDaemonOverride,
  spawnPipedHere,
  spawnTerminalHere,
  type AgentProcess
} from '../../packages/server/src/process-backend'
import { nativeCore } from '../../packages/server/src/native-core'
import {
  vorndLink,
  type LinkEffect,
  type LinkRecord,
  type Listing
} from '../../packages/server/src/vornd-link'

/**
 * vornd and its session holder in this process, for tests: the link protocol
 * as the server sees it, over whatever node-pty and child_process the test file
 * mocked.
 *
 * Each session's output is numbered into records the way sessiond numbers
 * them, and analyzed the way vornd analyzes it (with the same analyzer the
 * server would load), so status comes back as effects with their place. Every
 * record and effect is kept, so a test can replay them as a restarted vornd
 * would, or drop the link and bring a new one up.
 */

interface Held {
  id: string
  kind: 'pty' | 'piped'
  proc: ManagedPty | AgentProcess
  epoch: number
  rseq: number
  offset: number
  cols: number
  rows: number
  exited: { code: number | null; signal: number | null } | null
  log: Array<{ records?: LinkRecord[]; effect?: LinkEffect }>
  analyzer: { append(data: string, analyze: boolean): number; free(): void } | null
  status: number
}

/** A socket the link can hold: open until closed. */
class FakeSocket {
  readonly OPEN = 1
  readyState = 1
  private onClose: Array<() => void> = []
  constructor(private readonly onSend: (text: string) => void) {}
  send(text: string): void {
    if (this.readyState === this.OPEN) this.onSend(text)
  }
  once(event: string, cb: () => void): void {
    if (event === 'close') this.onClose.push(cb)
  }
  close(): void {
    if (this.readyState !== this.OPEN) return
    this.readyState = 3
    for (const cb of this.onClose.splice(0)) cb()
  }
}

export class LoopbackVornd {
  readonly sessions = new Map<string, Held>()
  /** Every call the server made, in order. */
  readonly calls: Array<{ method: string; params: Record<string, unknown> }> = []
  /** Answer spawns after this many ms, rather than at once. */
  spawnDelayMs = 0
  /** Withhold the records and effects of the exit, as a vornd that died before sending them. */
  holdExits = false
  /** Send nothing until the server asks to follow, as vornd does; otherwise follow at once. */
  waitForFollow = false
  private socket: FakeSocket | null = null
  private nextEpoch = 1000
  private following = false

  /** Link to the server, as vornd does on start. */
  link(): void {
    this.following = !this.waitForFollow
    const socket = new FakeSocket((text) => this.receive(text))
    this.socket = socket
    vorndLink.attach(socket as unknown as WebSocket)
  }

  /** The link drops: vornd died, or the app closed. The sessions go on. */
  unlink(): void {
    this.socket?.close()
    this.socket = null
  }

  private reply(id: unknown, result: unknown, error?: string): void {
    if (typeof id !== 'number') return
    const frame = error
      ? { jsonrpc: '2.0', id, error: { code: -32000, message: error } }
      : { jsonrpc: '2.0', id, result }
    // At once: the server queues what it writes before a spawn is answered, so
    // the order is the same either way, and a test that writes right after a
    // spawn sees the write land without waiting a turn.
    vorndLink.receive(frame)
  }

  private receive(text: string): void {
    const msg = JSON.parse(text) as { id?: number; method: string; params: Record<string, unknown> }
    this.calls.push({ method: msg.method, params: msg.params })
    const p = msg.params
    const held = typeof p?.id === 'string' ? this.sessions.get(p.id) : undefined
    try {
      switch (msg.method) {
        case 'vornd:list':
          return this.reply(msg.id, this.listing())
        case 'vornd:follow':
          this.following = true
          return this.reply(msg.id, { ok: true })
        case 'vornd:spawn': {
          const answer = (): void => {
            try {
              this.reply(msg.id, this.spawn(p))
            } catch (err) {
              this.reply(msg.id, null, (err as Error).message)
            }
          }
          if (this.spawnDelayMs) setTimeout(answer, this.spawnDelayMs)
          else answer()
          return
        }
        case 'vornd:write':
          if (held && !held.exited) (held.proc as ManagedPty).write(String(p.data))
          return this.reply(msg.id, null)
        case 'vornd:resize':
          if (held && held.kind === 'pty' && !held.exited) {
            const cols = Number(p.cols)
            const rows = Number(p.rows)
            ;(held.proc as ManagedPty).resize(cols, rows)
            held.cols = cols
            held.rows = rows
            this.record(held, { resize: [cols, rows] }, 0)
          }
          return this.reply(msg.id, null)
        case 'vornd:signal':
          if (held && !held.exited) {
            const name = `SIG${String(p.signal).toUpperCase()}`
            held.proc.kill(name as NodeJS.Signals)
          }
          return this.reply(msg.id, null)
        case 'vornd:closeStdin':
          if (held?.kind === 'piped') (held.proc as AgentProcess).stdin?.end()
          return this.reply(msg.id, null)
        default:
          return this.reply(msg.id, null, `vornd does not answer ${msg.method}`)
      }
    } catch (err) {
      this.reply(msg.id, null, (err as Error).message)
    }
  }

  private spawn(p: Record<string, unknown>): { id: string; pid: number; epoch: number } {
    const id = String(p.id)
    const existing = this.sessions.get(id)
    if (existing && !existing.exited) throw new Error(`session ${id} is still held`)
    const argv = p.argv as string[]
    const env = p.env as Record<string, string>
    const cwd = String(p.cwd)
    const piped = p.piped === true
    const epoch = this.nextEpoch++
    const cols = Number(p.cols ?? 80)
    const rows = Number(p.rows ?? 24)
    const proc = piped
      ? spawnPipedHere(argv[0]!, argv.slice(1), {
          cwd,
          env,
          stdio: ['pipe', 'pipe', 'pipe'],
          windowsHide: true,
          // What vornd does with `shell`: the platform's shell runs the line.
          shell: p.shell === true
        })
      : spawnTerminalHere({ file: argv[0]!, args: argv.slice(1), cwd, env, cols, rows })
    const Analyzer = nativeCore()?.Analyzer
    const held: Held = {
      id,
      kind: piped ? 'piped' : 'pty',
      proc,
      epoch,
      rseq: 0,
      offset: 0,
      cols,
      rows,
      exited: null,
      log: [],
      analyzer: Analyzer ? new Analyzer() : null,
      status: 0
    }
    this.sessions.set(id, held)
    const out = (data: string): void => this.output(held, data)
    if (piped) {
      const child = proc as AgentProcess
      child.stdout?.on('data', (chunk: Buffer) => out(chunk.toString()))
      child.stderr?.on('data', (chunk: Buffer) => out(chunk.toString()))
      child.on('exit', (code) => this.exit(held, { code, signal: null }))
      if (typeof p.stdin === 'string') {
        child.stdin?.on('error', () => {})
        if (p.stdin) child.stdin?.write(p.stdin)
        child.stdin?.end()
      }
    } else {
      const pty = proc as ManagedPty
      pty.onData(out)
      pty.onExit(({ exitCode, signal }) =>
        this.exit(held, { code: signal ? null : exitCode, signal: signal ?? null })
      )
    }
    return { id, pid: proc.pid ?? 0, epoch }
  }

  /** Output as the holder records it and vornd analyzes it. */
  output(held: Held, data: string): void {
    if (held.exited) return
    const bytes = Buffer.from(data)
    const effects: LinkEffect[] = []
    const status = held.analyzer?.append(data, true) ?? 0
    if (status && status !== held.status) {
      held.status = status
      effects.push(this.effectFor(held, held.rseq, 0, { kind: 'status', status }))
    }
    for (const e of effects) this.effect(held, e)
    this.record(held, { data: bytes.toString('base64') }, bytes.length)
  }

  private exit(held: Held, exit: { code: number | null; signal: number | null }): void {
    if (held.exited) return
    held.exited = exit
    held.analyzer?.free()
    if (this.holdExits) return
    this.effect(held, this.effectFor(held, held.rseq, 0, { kind: 'exit', ...exit }))
    this.record(held, { exit }, 0)
  }

  effectFor(
    held: { id: string; epoch: number },
    rseq: number,
    index: number,
    rest: Omit<LinkEffect, 'id' | 'effect' | 'epoch' | 'rseq' | 'index'>
  ): LinkEffect {
    return {
      id: held.id,
      effect: `${held.id}:${held.epoch}:${rseq}:${index}`,
      epoch: held.epoch,
      rseq,
      index,
      ...rest
    }
  }

  private record(
    held: Held,
    rest: Omit<LinkRecord, 'epoch' | 'rseq' | 'offset'>,
    len: number
  ): void {
    const r: LinkRecord = { epoch: held.epoch, rseq: held.rseq, offset: held.offset, ...rest }
    held.rseq += 1
    held.offset += len
    held.log.push({ records: [r] })
    this.deliver({ method: 'vornd:records', params: { id: held.id, records: [r] } })
  }

  /** An effect for a session, sent as vornd sends it and kept for replays. */
  effect(held: Held, e: LinkEffect): void {
    held.log.push({ effect: e })
    this.deliver({ method: 'vornd:effect', params: e })
  }

  private deliver(frame: { method: string; params: unknown }): void {
    if (!this.following || !this.socket) return
    vorndLink.receive({ jsonrpc: '2.0', ...frame } as Parameters<typeof vorndLink.receive>[0])
  }

  /**
   * What a vornd restarted from a checkpoint at `fromRseq` sends again: every
   * record and effect of the session from there, with the same places and ids.
   */
  replay(id: string, fromRseq: number): void {
    const held = this.sessions.get(id)
    if (!held) return
    for (const entry of held.log) {
      if (entry.records) {
        const records = entry.records.filter((r) => r.rseq >= fromRseq)
        if (records.length) this.deliver({ method: 'vornd:records', params: { id, records } })
      } else if (entry.effect && entry.effect.rseq >= fromRseq) {
        this.deliver({ method: 'vornd:effect', params: entry.effect })
      }
    }
  }

  listing(): Listing {
    const sessions = [...this.sessions.values()]
      .filter((h) => !h.exited || this.holdExits)
      .map((h) => ({
        id: h.id,
        kind: h.kind,
        pid: h.proc.pid ?? 0,
        epoch: h.epoch,
        state: 'live',
        cursor: { epoch: h.epoch, nextRseq: h.rseq, nextOffset: h.offset },
        cols: h.kind === 'pty' ? h.cols : null,
        rows: h.kind === 'pty' ? h.rows : null,
        exited: null
      }))
    const ended = [...this.sessions.values()]
      .filter((h) => h.exited && !this.holdExits)
      .map((h) => ({ id: h.id, exited: h.exited! }))
    return { connected: true, sessions, ended }
  }

  /** Forget a session, as a holder that died would. */
  drop(id: string): void {
    this.sessions.delete(id)
  }

  /** Back to the switch as configured, with nothing linked. */
  uninstall(): void {
    this.unlink()
    for (const held of this.sessions.values()) {
      if (!held.exited) {
        try {
          held.proc.kill()
        } catch {
          // A fake that cannot be killed is already gone.
        }
      }
    }
    this.sessions.clear()
    setNativeDaemonOverride(null)
  }
}

/** The switch on and a loopback vornd linked: every new session starts through it. */
export function installLoopbackVornd(): LoopbackVornd {
  setNativeDaemonOverride(true)
  const v = new LoopbackVornd()
  v.link()
  return v
}
