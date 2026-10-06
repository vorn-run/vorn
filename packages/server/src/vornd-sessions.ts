import { EventEmitter } from 'node:events'
import { decodeTerminalFrameV2 } from '@vornrun/shared/terminal-frame'
import type { VorndStatus } from '@vornrun/shared/types'
import log from './logger'
import { claimEffect, pruneEffectReceipts } from './database'
import { APP_PROTOCOL, VorndChannel } from './vornd-channel'

/**
 * Sessions started and held by vornd, the native daemon, instead of by this
 * server.
 *
 * This server starts vornd (`vornd-process.ts`) and connects to the channel
 * it opens. Every terminal and headless agent is started in vornd, which keeps
 * it in its session holder, so it outlives this server and the app. This
 * server keeps what it owns, the session's name, group, agent and workflow,
 * and is told what the output meant as effects: the agent's status, the
 * shell's directory, a notification, the exit. It reads the output itself only
 * of the sessions that need it (a remote host's login, a headless agent), by
 * attaching to them as any client does.
 *
 * An effect may be told twice, after vornd or this server restarts. Status and
 * directory are states and setting one again changes nothing. A notification
 * and an exit are acted on once, by the receipt each leaves in the database.
 */

/** A session as vornd reports it. */
export interface HeldSession {
  id: string
  kind: 'pty' | 'piped'
  pid: number
  status: EffectNote | null
  cwd: EffectNote | null
  exit: EffectNote | null
}

/** One effect, as `vornd:effect` carries it. */
export interface EffectNote {
  effectId: string
  id: string
  epoch: number
  rseq: number
  index: number
  kind: 'status' | 'cwd' | 'exit' | 'notify'
  status?: number
  cwd?: string
  exitCode?: number
  title?: string
  body?: string
}

interface AttachAnswer {
  live?: boolean
  continued?: boolean
  exitCode?: number
  cursor?: { epoch: number; nextRseq: number; nextOffset: number }
}

interface Subscribed {
  connected: boolean
  sessions: HeldSession[]
  ended: HeldSession[]
  notices: EffectNote[]
}

/** What vornd is asked to start. */
export interface VorndSpawn {
  argv: string[]
  cwd: string
  env: Record<string, string>
  cols?: number
  rows?: number
  /** A process on pipes rather than a terminal: a headless agent. */
  piped?: boolean
}

/** How an exit reached this server, and whether it has been acted on before. */
export interface VorndExit {
  exitCode: number
  signal?: number
  /** Acted on already, by this server or one before it: only state follows. */
  repeated?: boolean
}

/** How long a notification's receipt is kept: longer than vornd keeps one to tell again. */
export const NOTICE_RECEIPT_MS = 24 * 60 * 60 * 1000

/** How long an exit waits for the output before it, on a session being read. */
const EXIT_WAIT_MS = 3_000

/** How long a spawn waits for vornd's session holder before asking anyway. */
export const HOLDER_WAIT_MS = 10_000

/**
 * A session vornd holds, as this server's terminals use one.
 *
 * Returned at once, as `spawnPty` must: the spawn is answered later, and what
 * is written before then waits for it.
 */
export class VorndPty extends EventEmitter {
  pid = 0
  /** The record log this server knows of the session: what it reads output from. */
  epoch: number | null = null
  private ready = false
  private queued: string[] = []
  /** A signal sent before vornd knew the session, sent once it does. */
  private queuedSignal: string | null = null
  private ended = false
  private readonly dataListeners = new Set<(data: string) => void>()
  private readonly exitListeners = new Set<(event: VorndExit) => void>()
  private readonly decoder = new TextDecoder()
  /** The next record this server has not read, while it reads. */
  private nextRseq = 0
  private exitSeen: EffectNote | null = null
  private exitTimer: ReturnType<typeof setTimeout> | undefined

  constructor(
    private readonly owner: VorndSessions,
    readonly id: string,
    /** Whether this server reads the session's output. */
    readonly watched: boolean
  ) {
    super()
  }

  /** The spawn was answered, or the session was found held. */
  started(pid: number, epoch: number): void {
    this.pid = pid
    this.epoch = epoch
    this.ready = true
    this.emit('started', pid)
    for (const data of this.queued.splice(0)) this.owner.write(this.id, data)
    if (this.queuedSignal) this.owner.kill(this.id, this.queuedSignal)
    this.queuedSignal = null
    if (this.watched) this.owner.watch(this)
  }

  write(data: string): void {
    if (this.ended || data.length === 0) return
    if (!this.ready) {
      this.queued.push(data)
      return
    }
    this.owner.write(this.id, data)
  }

  /** The size is vornd's to decide, from the people watching. */
  resize(): void {}

  kill(signal = 'SIGHUP'): void {
    if (this.ended) return
    // Before the spawn is answered vornd does not know the name yet.
    if (!this.ready) {
      this.queuedSignal = signal
      return
    }
    this.owner.kill(this.id, signal)
  }

  /** A headless agent has had its prompt. */
  closeStdin(): void {
    if (!this.ready) {
      this.queued.push('')
      return
    }
    this.owner.closeStdin(this.id)
  }

  onData(listener: (data: string) => void): { dispose(): void } {
    this.dataListeners.add(listener)
    return { dispose: () => this.dataListeners.delete(listener) }
  }

  onExit(listener: (event: VorndExit) => void): { dispose(): void } {
    this.exitListeners.add(listener)
    return { dispose: () => this.exitListeners.delete(listener) }
  }

  /** Nothing to hold still: vornd keeps the session whatever this server does. */
  pause(): void {}
  resume(): void {}

  get isEnded(): boolean {
    return this.ended
  }

  /** Where reading resumes after vornd or its holder starts again. */
  readCursor(): { epoch: number; nextRseq: number; nextOffset: number } | null {
    return this.epoch === null
      ? null
      : { epoch: this.epoch, nextRseq: this.nextRseq, nextOffset: 0 }
  }

  /**
   * @internal The answer to reading from `readCursor`. One that could not
   * continue from there starts from a screen instead, and reading goes on
   * from where that screen ends: what was printed in between is not read.
   */
  attached(answer: AttachAnswer | null): void {
    if (!answer || answer.live === false) {
      if (typeof answer?.exitCode === 'number') this.exitNotice(answer.exitCode)
      return
    }
    if (answer.continued || !answer.cursor) return
    this.epoch = answer.cursor.epoch
    this.nextRseq = answer.cursor.nextRseq
  }

  /** @internal Bytes read from the session's records. */
  frame(epoch: number, lastRseq: number, bytes: Uint8Array): void {
    if (this.ended || epoch !== this.epoch || lastRseq < this.nextRseq) return
    this.nextRseq = lastRseq + 1
    const data = this.decoder.decode(bytes, { stream: true })
    if (!data) return
    for (const listener of this.dataListeners) listener(data)
  }

  /** @internal The exit effect. A session being read waits for its last output. */
  exitEffect(note: EffectNote): void {
    if (this.ended) return
    this.exitSeen = note
    if (!this.watched) return this.finish(note.exitCode ?? 0)
    clearTimeout(this.exitTimer)
    this.exitTimer = setTimeout(() => this.finish(note.exitCode ?? 0), EXIT_WAIT_MS)
  }

  /** @internal `terminal:exit`: after the last of the output, for a session being read. */
  exitNotice(exitCode: number): void {
    this.finish(this.exitSeen?.exitCode ?? exitCode)
  }

  /** @internal The session ended, and this is the last of it. */
  finish(exitCode: number): void {
    if (this.ended) return
    this.ended = true
    clearTimeout(this.exitTimer)
    this.queued = []
    this.queuedSignal = null
    const tail = this.decoder.decode()
    if (tail) for (const listener of this.dataListeners) listener(tail)
    // One exit per run of the session: its effect id names it, and a session
    // that ended unseen has the run's epoch to go by. One that never started
    // has no run to tell twice: a resume reusing its id may fail again.
    const receipt =
      this.exitSeen?.effectId ?? (this.epoch === null ? null : `${this.id}/${this.epoch}/exit`)
    const repeated = receipt !== null && !claim(receipt, 'exit')
    this.owner.forget(this)
    for (const listener of this.exitListeners) listener({ exitCode, repeated })
  }
}

/** Takes a receipt; answers false when one was taken before. A database that fails acts. */
function claim(effectId: string, kind: string): boolean {
  try {
    return claimEffect(effectId, kind)
  } catch (err) {
    log.warn({ err, effectId }, '[vornd] could not record a receipt; acting on the effect')
    return true
  }
}

/**
 * The channel to vornd and every session started through it.
 *
 * Emits `held` (HeldSession[]) with the sessions vornd holds that no
 * terminal here stands for, after each subscription, so the terminals of a
 * previous run can be taken on again; and `notify` (id, title, body) once per
 * notification.
 */
export class VorndSessions extends EventEmitter {
  private channel: VorndChannel | null = null
  private connecting: Promise<boolean> | null = null
  /** Whether vornd last said its session holder is connected. */
  private holderUp = false
  private readonly ptys = new Map<string, VorndPty>()

  /** What starts vornd, and says whether it is coming. */
  private launcher: VorndLauncher | null = null

  /** Whether the channel to vornd is up. */
  inUse(): boolean {
    return this.channel !== null && !this.channel.isClosed
  }

  /** Who starts vornd for this server; a spawn while it starts waits for it. */
  setLauncher(launcher: VorndLauncher | null): void {
    this.launcher = launcher
  }

  /** The session vornd holds under `id`, if one does. */
  get(id: string): VorndPty | undefined {
    return this.ptys.get(id)
  }

  /**
   * Connect to vornd's channel at `endpoint` and take stock of what it holds.
   * Answers whether the channel is up.
   */
  connect(endpoint: string): Promise<boolean> {
    const before = this.connecting ?? Promise.resolve(false)
    const attempt = before.then(() => this.open(endpoint))
    const tracked: Promise<boolean> = attempt.finally(() => {
      if (this.connecting === tracked) this.connecting = null
    })
    this.connecting = tracked
    return attempt
  }

  private async open(endpoint: string): Promise<boolean> {
    let channel: VorndChannel
    try {
      channel = await VorndChannel.connect(endpoint)
      const hello = await channel.request<{ protocol?: number }>('vornd:hello')
      if (hello?.protocol !== APP_PROTOCOL) {
        channel.close()
        throw new Error(
          `vornd speaks protocol ${String(hello?.protocol)}, and this server ${APP_PROTOCOL}`
        )
      }
    } catch (err) {
      log.error({ err, endpoint }, '[vornd] could not reach vornd')
      return false
    }
    const old = this.channel
    this.channel = channel
    old?.close()
    channel.on('notification', (method: string, params: unknown) =>
      this.notified(channel, method, params)
    )
    channel.on('frame', (bytes: Uint8Array) => this.framed(bytes))
    channel.on('close', (why: string) => {
      if (this.channel !== channel) return
      this.channel = null
      this.holderUp = false
      log.warn(`[vornd] the channel to vornd closed (${why}); its sessions carry on there`)
    })
    log.info({ endpoint }, '[vornd] connected to vornd')
    await this.subscribe(channel)
    return !channel.isClosed
  }

  /** Take stock: every session vornd holds, those that ended, the notifications kept. */
  private async subscribe(channel: VorndChannel): Promise<void> {
    let state: Subscribed
    try {
      state = await channel.request<Subscribed>('vornd:subscribe')
    } catch (err) {
      log.warn({ err }, '[vornd] could not subscribe to vornd')
      return
    }
    if (this.channel !== channel) return
    this.emit('subscribed')
    this.holderTold(state.connected)
    try {
      pruneEffectReceipts('notify', Date.now() - NOTICE_RECEIPT_MS)
    } catch {
      // Kept a while longer.
    }
    const held = new Map(state.sessions.map((s) => [s.id, s]))
    const ended = new Map(state.ended.map((s) => [s.id, s]))
    for (const pty of [...this.ptys.values()]) {
      const now = held.get(pty.id)
      if (now) {
        if (pty.epoch === null || !pty.pid)
          pty.started(now.pid, now.status?.epoch ?? pty.epoch ?? 0)
        else if (pty.watched) this.watch(pty)
        this.states(now)
        continue
      }
      // Its spawn waited for this connect and is not answered yet: the
      // answer says how it went.
      if (pty.epoch === null) continue
      // Not held: it ended, maybe while nothing here was connected. Without
      // a holder vornd cannot tell, and nothing is said until it can.
      if (!state.connected) continue
      const how = ended.get(pty.id)?.exit
      if (how) pty.exitEffect(how)
      else pty.finish(0)
    }
    for (const note of state.notices) this.notice(note)
    const strangers = state.sessions.filter((s) => !this.ptys.has(s.id))
    if (strangers.length) this.emit('held', strangers)
  }

  /** A held session's latest states, told again: setting one twice changes nothing. */
  private states(held: HeldSession): void {
    if (held.status) this.effect(held.status)
    if (held.cwd) this.effect(held.cwd)
    if (held.exit) this.effect(held.exit)
  }

  /**
   * Start a session in vornd under `id`. Answered at once; a spawn that fails
   * ends the session with exit code 1. While vornd is starting the spawn waits
   * for it; when vornd cannot be used at all, this throws and says why.
   */
  spawn(id: string, spec: VorndSpawn, watched: boolean): VorndPty {
    if (!this.inUse() && !this.connecting && !this.launcher?.starting) {
      const state = this.launcher?.state
      const why = state?.state === 'failed' ? state.detail : 'vornd is not running'
      throw new Error(`Terminals cannot start: ${why}`)
    }
    const pty = new VorndPty(this, id, watched)
    this.ptys.set(id, pty)
    const fail = (err: Error): void => {
      log.warn({ err, id }, '[vornd] vornd could not start this session')
      pty.finish(1)
    }
    this.whenConnected()
      .then((up) => {
        if (!up) throw new Error('vornd is not running')
        return up.request<{ id: string; pid: number; epoch: number }>('vornd:spawn', {
          ...spec,
          name: id
        })
      })
      .then((s) => pty.started(s.pid, s.epoch))
      .catch(fail)
    return pty
  }

  /**
   * The channel, once a start or connect in flight is done and vornd's
   * session holder is up; null when there is none. vornd answers before its
   * holder connects, and a spawn asked of it then would fail.
   */
  private async whenConnected(): Promise<VorndChannel | null> {
    if (!this.inUse()) {
      await this.launcher?.ready()
      await this.connecting
    }
    if (!this.inUse()) return null
    if (!this.holderUp) await this.holderWait(HOLDER_WAIT_MS)
    return this.inUse() ? this.channel : null
  }

  private holderTold(up: boolean): void {
    this.holderUp = up
    if (up) this.emit('holder')
  }

  /** Until vornd says its holder is up, or `ms` pass: then the spawn is asked anyway. */
  private holderWait(ms: number): Promise<void> {
    return new Promise((resolve) => {
      const done = (): void => {
        clearTimeout(timer)
        this.off('holder', done)
        resolve()
      }
      const timer = setTimeout(done, ms)
      timer.unref?.()
      this.on('holder', done)
    })
  }

  /**
   * Take on a session vornd already holds, from a previous run of this
   * server. `wire` listens to it before its latest states are told.
   */
  adopt(held: HeldSession, watched: boolean, wire?: (pty: VorndPty) => void): VorndPty {
    const pty = new VorndPty(this, held.id, watched)
    this.ptys.set(held.id, pty)
    wire?.(pty)
    pty.started(held.pid, held.status?.epoch ?? held.cwd?.epoch ?? 0)
    this.states(held)
    return pty
  }

  /** @internal */
  forget(pty: VorndPty): void {
    if (this.ptys.get(pty.id) === pty) this.ptys.delete(pty.id)
  }

  /** Stop following a session without ending it, for a server on its way out. */
  release(id: string): void {
    this.ptys.delete(id)
  }

  /** @internal */
  write(id: string, data: string): void {
    const channel = this.channel
    if (!channel) {
      log.warn({ id }, '[vornd] input dropped: vornd is not connected')
      return
    }
    // An empty write is the queued end of input.
    if (data === '') channel.notify('vornd:closeStdin', { id })
    else channel.notify('terminal:write', { id, data })
  }

  /** @internal */
  kill(id: string, signal: string): void {
    const sig = SIGNALS[signal] ?? 'hup'
    this.channel?.request('vornd:kill', { id, signal: sig }).catch((err: Error) => {
      log.warn({ err, id }, '[vornd] could not signal a session')
    })
  }

  /** @internal */
  closeStdin(id: string): void {
    this.channel?.request('vornd:closeStdin', { id }).catch((err: Error) => {
      log.warn({ err, id }, '[vornd] could not end input')
    })
  }

  /** The last `lines` lines a session printed, as vornd's model of its screen has them. */
  async readOutput(id: string, lines?: number): Promise<string[]> {
    const channel = this.channel
    if (!channel) return []
    const out = await channel.request<unknown>('terminal:readOutput', {
      id,
      ...(lines === undefined ? {} : { lines })
    })
    return Array.isArray(out) ? out.filter((l): l is string => typeof l === 'string') : []
  }

  /** @internal Read a session's output from where this server last read. */
  watch(pty: VorndPty): void {
    const channel = this.channel
    const cursor = pty.readCursor()
    if (!channel || !cursor) return
    channel
      .request<AttachAnswer>('terminal:attach', { id: pty.id, cursor })
      .then((answer) => pty.attached(answer))
      .catch((err: Error) => {
        log.warn({ err, id: pty.id }, '[vornd] could not read a session')
      })
  }

  private framed(bytes: Uint8Array): void {
    const frame = decodeTerminalFrameV2(bytes)
    if (!frame) return
    this.ptys.get(frame.id)?.frame(frame.epoch, frame.lastRseq, frame.data)
  }

  private notified(channel: VorndChannel, method: string, params: unknown): void {
    const p = (params ?? {}) as Record<string, unknown>
    const id = typeof p.id === 'string' ? p.id : ''
    switch (method) {
      case 'vornd:effect':
        this.effect(p as unknown as EffectNote)
        return
      case 'vornd:activity':
        this.ptys.get(id)?.emit('activity')
        return
      case 'vornd:connected':
        // The holder came back: what it holds may have changed.
        if (channel === this.channel) this.holderTold(true)
        void this.subscribe(channel)
        return
      case 'terminal:exit':
        this.ptys.get(id)?.exitNotice(typeof p.exitCode === 'number' ? p.exitCode : 0)
        return
      case 'terminal:resync': {
        const pty = this.ptys.get(id)
        if (pty?.watched) this.watch(pty)
        return
      }
      default:
        // What vornd asks of this server beyond its sessions.
        if (method.startsWith('vornd:')) this.emit('ask', method, params)
    }
  }

  /** Tell vornd `method` with `params`; false when there is no channel or it refused. */
  async tell(method: string, params: unknown): Promise<boolean> {
    const channel = this.channel
    if (!channel || channel.isClosed) return false
    try {
      await channel.request(method, params)
      return true
    } catch (err) {
      log.warn({ err, method }, '[vornd] vornd refused a call')
      return false
    }
  }

  private effect(note: EffectNote): void {
    if (note.kind === 'notify') return this.notice(note)
    const pty = this.ptys.get(note.id)
    if (!pty) return
    switch (note.kind) {
      case 'status':
        pty.emit('status', note.status ?? 0, note)
        return
      case 'cwd':
        if (note.cwd) pty.emit('cwd', note.cwd)
        return
      case 'exit':
        pty.exitEffect(note)
        return
    }
  }

  /** Shown once, whichever connection tells it and however often. */
  private notice(note: EffectNote): void {
    if (!claim(note.effectId, 'notify')) return
    this.emit('notify', note.id, note.title ?? '', note.body ?? '')
  }

  /** Close the channel, for tests and a server on its way out. */
  close(): void {
    const channel = this.channel
    this.channel = null
    channel?.close()
    this.ptys.clear()
  }
}

const SIGNALS: Record<string, string> = {
  SIGHUP: 'hup',
  SIGTERM: 'term',
  SIGKILL: 'kill',
  SIGINT: 'int'
}

/** What starts vornd for this server: `VorndKeeper`. */
export interface VorndLauncher {
  readonly state: VorndStatus
  /** Whether a start is in flight. */
  readonly starting: boolean
  /** Resolves when a start in flight is done. */
  ready(): Promise<void>
}

export const vorndSessions = new VorndSessions()
