import { EventEmitter } from 'node:events'
import { StringDecoder } from 'node:string_decoder'
import type { RecordCursor } from '@vornrun/shared/types'
import type { ManagedPty } from './handoff/adopted-pty'
import log from './logger'
import { claimEffect } from './effect-receipts'
import {
  vorndLink,
  type LinkEffect,
  type LinkReceiver,
  type LinkRecord,
  type LinkSpawn,
  type Listing,
  type VorndLink
} from './vornd-link'

/**
 * A terminal or piped agent that lives in vornd's session holder rather than in
 * this process: the Native daemon switch's process backend.
 *
 * It is a `ManagedPty`, so `pty-manager` drives it exactly as it drives a
 * node-pty: bytes out of `onData`, input into `write`, `resize`, `kill`. What
 * differs is where the process is. It belongs to the session holder, which
 * outlives this server, so a server that restarts finds it again in vornd's
 * list (`VorndProcess.adopt`) instead of losing it.
 *
 * Output arrives as records, each at its place in the session's log. A record
 * at or below the cursor this process has reached is one it already has, which
 * is what a vornd restart's replay sends; a record beyond it means output was
 * lost while nothing was linked, and is logged and taken.
 *
 * Effects carry the cursor they reflect: a status is applied only when it is
 * newer than the last one applied. The exit is a state of the session and comes
 * once, after every byte before it; as the trigger that ends a workflow step it
 * is claimed in `effect_receipts` first, so a repeat after a restart of this
 * server or of vornd starts nothing.
 */

/** How long a spawn may keep finding the name still held by the run before it. */
const SPAWN_RETRY_MS = 5_000
const SPAWN_RETRY_STEP_MS = 100

/** How long an exit effect waits for the records it follows before it is taken alone. */
const EXIT_WAIT_MS = 2_000

/** A status effect's place, to apply only newer ones. */
interface EffectPlace {
  epoch: number
  rseq: number
  index: number
}

function newer(a: EffectPlace, than: EffectPlace | null): boolean {
  if (!than) return true
  // Another epoch is another run of the session under the same name.
  if (a.epoch !== than.epoch) return true
  return a.rseq > than.rseq || (a.rseq === than.rseq && a.index > than.index)
}

/** The exit as listeners hear it. */
export interface VorndExit {
  exitCode: number
  signal?: number
  /** The raw code, null when the program was ended by a signal. */
  code: number | null
  /** The effect id of the exit, for receivers that act on it once. */
  effectId: string
  /** False when an earlier life of this server already acted on this exit. */
  first: boolean
}

export class VorndProcess implements ManagedPty, LinkReceiver {
  pid = 0
  readonly id: string
  readonly piped: boolean
  private cursor: RecordCursor | null
  private readonly decoder = new StringDecoder('utf8')
  private readonly events = new EventEmitter()
  /** Input written before the session exists, sent once it does. */
  private queued: string[] = []
  private spawned = false
  private ended = false
  private killWhenSpawned: string | null = null
  private lastStatus: EffectPlace | null = null
  /** Settles once the holder has answered the spawn, whichever way. */
  readonly ready: Promise<void>
  private settleReady: () => void = () => {}
  private exitTimer: ReturnType<typeof setTimeout> | null = null
  /** The size vornd last accepted, so an identical re-fit is not sent again. */
  private size: [number, number] | null
  /** A size asked for while vornd could not take it: sent once it can. */
  private pendingSize: [number, number] | null = null
  /**
   * The spawn whose answer was lost with the link: whether the program
   * started is learned from vornd's list once a vornd links again.
   */
  private orphaned: LinkSpawn | null = null
  private saidGap = false

  private constructor(
    private readonly link: VorndLink,
    id: string,
    piped: boolean,
    cursor: RecordCursor | null,
    size: [number, number] | null
  ) {
    this.id = id
    this.piped = piped
    this.cursor = cursor
    this.size = size
    // Several listeners per process is normal: pty-manager, a remote host's
    // password prompt, the SSH marker.
    this.events.setMaxListeners(0)
    this.ready = new Promise((resolve) => (this.settleReady = resolve))
    link.register(id, this)
  }

  /** Start `spec` in the session holder under `spec.id`. */
  static spawn(spec: LinkSpawn, link: VorndLink = vorndLink): VorndProcess {
    const p = new VorndProcess(
      link,
      spec.id,
      spec.piped === true,
      null,
      spec.piped ? null : [spec.cols ?? 80, spec.rows ?? 24]
    )
    void p.start(spec)
    return p
  }

  /**
   * A session the holder already has, found in vornd's list after this server
   * or vornd restarted. Records before `cursor` are what the screen this server
   * rebuilt already shows, or what it can no longer use; the ones after it are
   * taken.
   */
  static adopt(
    found: { id: string; kind: 'pty' | 'piped'; pid: number; cursor: RecordCursor | null },
    size: [number, number] | null,
    link: VorndLink = vorndLink
  ): VorndProcess {
    const p = new VorndProcess(link, found.id, found.kind === 'piped', found.cursor, size)
    p.pid = found.pid
    p.spawned = true
    p.settleReady()
    return p
  }

  /** Whether the session holder has started it, and so whether vornd's list should name it. */
  get started(): boolean {
    return this.spawned
  }

  get exited(): boolean {
    return this.ended
  }

  private async start(spec: LinkSpawn): Promise<void> {
    const deadline = Date.now() + SPAWN_RETRY_MS
    for (;;) {
      try {
        const done = await this.link.spawn(spec)
        this.pid = done.pid
        // Records may have come before the answer did.
        this.cursor ??= { epoch: done.epoch, nextRseq: 0, nextOffset: 0 }
        break
      } catch (err) {
        const message = (err as Error).message
        // The link went before vornd answered: the holder may well have
        // started the program. Not a failure; vornd's list says, once linked.
        if (/vornd is not linked/.test(message) && !this.ended) {
          log.warn({ id: this.id }, '[vornd] the link went during a spawn; waiting for vornd')
          this.orphaned = spec
          this.settleReady()
          return
        }
        // A resumed session's name is released once vornd has every record of
        // the run before it; that is moments away.
        if (/still held/.test(message) && Date.now() < deadline && !this.ended) {
          await new Promise((r) => setTimeout(r, SPAWN_RETRY_STEP_MS))
          continue
        }
        log.warn({ id: this.id, err }, '[vornd] could not start a session in the session holder')
        this.emitData(`\r\n[vorn] could not start this session through vornd: ${message}\r\n`)
        this.finish({ code: 1, signal: null }, null)
        this.settleReady()
        return
      }
    }
    this.holderStarted()
  }

  /** The holder has the program: send what waited for it. */
  private holderStarted(): void {
    this.spawned = true
    this.settleReady()
    this.events.emit('spawned', this.pid)
    if (this.killWhenSpawned) {
      this.kill(this.killWhenSpawned)
      return
    }
    for (const data of this.queued) this.link.notify('vornd:write', { id: this.id, data })
    this.queued = []
    this.flushSize()
  }

  /**
   * vornd's list, after a vornd linked (or its session holder came back):
   * what this process should make of it. A spawn whose answer was lost takes
   * the session if vornd has it and spawns again if not; a started one that
   * vornd no longer has has ended, as `ended` says or with no code; one it
   * has gets a size that could not be sent while nothing was linked.
   */
  reconcile(listing: Listing): void {
    if (this.ended) return
    const found = listing.sessions.find((s) => s.id === this.id)
    if (this.orphaned) {
      const spec = this.orphaned
      this.orphaned = null
      if (found) {
        this.pid = found.pid
        this.holderStarted()
      } else {
        void this.start(spec)
      }
      return
    }
    if (!this.spawned) return
    if (!found) {
      const ended = listing.ended.find((e) => e.id === this.id)
      log.warn({ id: this.id }, '[vornd] vornd no longer holds this session; it has ended')
      this.lost(ended?.exited ?? null)
      return
    }
    this.flushSize()
  }

  write(data: string): void {
    if (this.ended || !data) return
    if (!this.spawned) {
      this.queued.push(data)
      return
    }
    if (!this.link.notify('vornd:write', { id: this.id, data })) {
      // Input is at most once: what was typed while nothing was linked is gone,
      // and the person can see it was.
      log.warn({ id: this.id }, '[vornd] input dropped: vornd is not linked')
    }
  }

  resize(cols: number, rows: number): void {
    if (this.ended || this.piped) return
    this.pendingSize = [cols, rows]
    this.flushSize()
  }

  /**
   * Sends the size asked for, once vornd can take it. The record that comes
   * back is what moves the session's size; this only remembers what vornd
   * accepted, so a size lost while nothing was linked is sent again rather
   * than taken for done.
   */
  private flushSize(): void {
    const want = this.pendingSize
    if (!want || !this.spawned) return
    if (this.size && this.size[0] === want[0] && this.size[1] === want[1]) {
      this.pendingSize = null
      return
    }
    if (!this.link.notify('vornd:resize', { id: this.id, cols: want[0], rows: want[1] })) return
    this.size = want
    this.pendingSize = null
  }

  kill(signal?: string): void {
    if (this.ended) return
    const name = (signal ?? (this.piped ? 'SIGTERM' : 'SIGHUP')).replace(/^SIG/i, '').toLowerCase()
    if (!this.spawned) {
      this.killWhenSpawned = name
      return
    }
    if (!this.link.notify('vornd:signal', { id: this.id, signal: name })) {
      log.warn({ id: this.id }, '[vornd] could not signal a session: vornd is not linked')
    }
  }

  /** The holder reads on its own; there is no reader here to hold still. */
  pause(): void {}
  resume(): void {}

  onData(listener: (data: string) => void): { dispose(): void } {
    return this.on('data', listener)
  }

  onExit(listener: (event: { exitCode: number; signal?: number } & Partial<VorndExit>) => void): {
    dispose(): void
  } {
    return this.on('exit', listener)
  }

  /** The holder started the program, as `pid`. */
  onSpawned(listener: (pid: number) => void): { dispose(): void } {
    return this.on('spawned', listener)
  }

  /** A resize recorded in the session's log, from any client: the session's size from here on. */
  onResize(listener: (cols: number, rows: number) => void): { dispose(): void } {
    return this.on('resize', listener)
  }

  /** An agent status vornd's analysis reported, as a `NATIVE_STATUS` code, newest only. */
  onStatus(listener: (status: number) => void): { dispose(): void } {
    return this.on('status', listener)
  }

  private on<A extends unknown[]>(
    event: string,
    listener: (...args: A) => void
  ): { dispose(): void } {
    const l = listener as (...args: unknown[]) => void
    this.events.on(event, l)
    return { dispose: () => this.events.off(event, l) }
  }

  private emitData(data: string): void {
    if (data) this.events.emit('data', data)
  }

  records(records: LinkRecord[]): void {
    for (const r of records) {
      if (this.ended) return
      const at = this.cursor
      if (at && r.epoch === at.epoch && r.rseq < at.nextRseq) continue
      if (at && r.epoch === at.epoch && r.rseq > at.nextRseq && !this.saidGap) {
        this.saidGap = true
        log.warn(
          { id: this.id, from: at.nextRseq, to: r.rseq },
          '[vornd] records missing from this server’s copy of a session'
        )
      }
      const len = r.data ? Buffer.byteLength(r.data, 'base64') : (r.gap ?? 0)
      this.cursor = { epoch: r.epoch, nextRseq: r.rseq + 1, nextOffset: r.offset + len }
      if (r.data !== undefined) {
        this.emitData(this.decoder.write(Buffer.from(r.data, 'base64')))
      } else if (r.resize) {
        const [cols, rows] = r.resize
        this.size = [cols, rows]
        this.events.emit('resize', cols, rows)
      } else if (r.gap !== undefined) {
        this.emitData(this.decoder.end())
      } else if (r.exit) {
        this.emitData(this.decoder.end())
        this.finish(r.exit, `${this.id}:${r.epoch}:${r.rseq}:0`)
      }
    }
  }

  effect(e: LinkEffect): void {
    if (this.ended) return
    if (e.kind === 'status' && typeof e.status === 'number') {
      if (!newer(e, this.lastStatus)) return
      this.lastStatus = { epoch: e.epoch, rseq: e.rseq, index: e.index }
      this.events.emit('status', e.status)
    } else if (e.kind === 'exit' && !this.exitTimer) {
      // The exit effect comes before the records of its batch, and the exit
      // record after every byte. Taken alone only if the records never come.
      const exit = { code: e.code ?? null, signal: e.signal ?? null }
      this.exitTimer = setTimeout(() => this.finish(exit, e.effect), EXIT_WAIT_MS)
      this.exitTimer.unref?.()
    }
  }

  /**
   * vornd no longer has this session, and did not say how it ended: its holder
   * went away. The session ends here with no code.
   */
  lost(exited: { code: number | null; signal: number | null } | null): void {
    if (this.ended) return
    this.finish(exited ?? { code: null, signal: null }, null)
  }

  private finish(
    exit: { code: number | null; signal: number | null },
    effectId: string | null
  ): void {
    if (this.ended) return
    this.ended = true
    if (this.exitTimer) clearTimeout(this.exitTimer)
    this.exitTimer = null
    this.link.unregister(this.id, this)
    const first = effectId ? claimEffect(effectId, 'trigger') : true
    const event: VorndExit = {
      // node-pty's convention: a signalled program reports code 0 and its signal.
      exitCode: exit.code ?? 0,
      ...(exit.signal !== null ? { signal: exit.signal } : {}),
      code: exit.code,
      effectId: effectId ?? '',
      first
    }
    this.events.emit('exit', event)
  }
}

/**
 * A piped agent over the link, shaped like the part of a `ChildProcess` that
 * `headless-manager` uses: `stdout` data, `exit` with the code (null when
 * signalled), `kill`. stdin is given at spawn and closed by vornd, so there is
 * none here.
 */
export class VorndChild extends EventEmitter {
  readonly stdout = new EventEmitter()
  readonly stderr = new EventEmitter()
  readonly stdin = null
  private readonly proc: VorndProcess

  constructor(spec: LinkSpawn, link: VorndLink = vorndLink) {
    super()
    this.proc = VorndProcess.spawn({ ...spec, piped: true }, link)
    this.proc.onData((data) => this.stdout.emit('data', Buffer.from(data)))
    this.proc.onExit((e) => {
      // An exit an earlier life of this server already acted on is not acted on again.
      if (e.first === false) {
        log.info({ id: spec.id }, '[vornd] an agent’s exit was already handled; not again')
        return
      }
      this.emit('exit', e.code ?? (e.signal !== undefined ? null : 1))
    })
  }

  get pid(): number {
    return this.proc.pid
  }

  /** See `VorndProcess.reconcile`; an agent vornd no longer has exits with 1. */
  reconcile(listing: Listing): void {
    this.proc.reconcile(listing)
  }

  kill(signal?: NodeJS.Signals | number): boolean {
    this.proc.kill(typeof signal === 'string' ? signal : 'SIGTERM')
    return true
  }
}
