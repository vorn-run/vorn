import { EventEmitter } from 'node:events'
import os from 'node:os'
import { decodeTerminalFrameV2 } from '@vornrun/shared/terminal-frame'
import type {
  AgentStatus,
  HeadlessSession,
  RestoredSession,
  TerminalSession,
  VorndStatus
} from '@vornrun/shared/types'
import log from './logger'
import { claimEffect, pruneEffectReceipts } from './database'
import { APP_PROTOCOL, VorndChannel } from './vornd-channel'
import {
  sessionFeed,
  type RecordFeed,
  type RecordKind,
  type RecordSink,
  type Stamp
} from './session-feed'

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
 *
 * vornd keeps a copy of this server's session records: this server feeds it
 * (`session-feed.ts`) and follows the copy's changes (`SessionMirror`).
 *
 * The copy decides each terminal's status: from what its screen shows, its output going quiet,
 * what is written to it and what the agent's hooks report. This server then
 * tells vornd what only it sees (`hookStatus`, `input`, `patch`) and takes each
 * status from the copy's changes (`mirrored`); the rest of the records stay its own.
 *
 * vornd also answers the clients' calls that create, close, rename,
 * regroup and reorder terminals, in its copy, and says so
 * in the changes it tells (`native`): a terminal it created and whether its
 * program started, one it closed, the order a client set. This server follows
 * them as it follows its own calls, telling clients, saving the records and
 * running what hangs off a session. The conversations being started are then
 * claimed in vornd (`claim`), so its creates and this server's own starts (a
 * resume, a create for a remote host) check one set of claims, and vornd is told
 * when this server winds down (`tellClosing`), when it creates nothing new.
 *
 * Likewise the clients' calls that start and stop headless agents: vornd starts the agent on pipes in its holder, puts the
 * record in its copy and tells it (`native`, `created`), then its program's
 * start and, from the session's exit effect, how it ended. This server follows
 * the agent as one it started itself: it reads the output for the clients and
 * the workflow waiting on it, and lets the record go a while after the exit.
 *
 * vornd also owns the records between runs:
 * it writes its copy down and reads it back when it starts, so a terminal the
 * holder still holds after this server restarts is live again in the copy
 * before this server connects (`adopted`), and the rest are offered to resume
 * from vornd's list (`restored`), which vornd answers `sessions:restored`,
 * `sessions:resume` and `sessions:clear` from. A session it starts again under
 * its id is told `resumed`. This server then saves nothing to its own session
 * records: it takes them on from the copy (`takeOnHeld` in `register-methods`),
 * and hands the ones its database still has to vornd once (`vornd:carry`).
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

/** What `vornd:hello` says. */
interface Hello {
  protocol?: number
  scripts?: 'native' | 'shadow' | null
}

/** When this machine came up. Uptime counts through sleep, so a laptop closed overnight is not a reboot. */
function bootTime(): number {
  return Date.now() - os.uptime() * 1000
}

/** Whether this server is winding down, as vornd is told it. */
export interface Closing {
  draining: boolean
  handingOver: boolean
}

/** How often whether this server is winding down is looked at again while vornd creates terminals. */
const CLOSING_CHECK_MS = 5_000

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
  /** vornd's copy of the session records, when it keeps one. */
  registry?: RegistrySnapshot
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

  /** The record cursor of the exit effect, once vornd has told it. */
  get exitAt(): Stamp | null {
    const note = this.exitSeen
    return note ? { epoch: note.epoch, rseq: note.rseq, index: note.index } : null
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
 * previous run can be taken on again; and `notify` (id, title, body, effectId)
 * once per notification.
 */
export class VorndSessions extends EventEmitter {
  private channel: VorndChannel | null = null
  private connecting: Promise<boolean> | null = null
  /** Whether vornd last said its session holder is connected. */
  private holderUp = false
  private readonly ptys = new Map<string, VorndPty>()

  /** What starts vornd, and says whether it is coming. */
  private launcher: VorndLauncher | null = null

  /** Whether vornd runs the project scripts, or compares what this server runs, as its `vornd:hello` said. */
  private scriptWork: Hello['scripts'] = null

  /** Whether this server is winding down, as vornd needs to know while it creates terminals. */
  private closingSource: (() => Closing) | null = null
  private toldClosing = ''
  private closingTimer: ReturnType<typeof setInterval> | undefined

  /** Where `sessionFeed` sends this server's session records: this channel, when vornd wants them. */
  readonly recordSink: RecordSink = {
    wants: () => this.inUse(),
    send: (params) => this.channel?.notify('vornd:record', params)
  }

  /** What tells vornd's copy of the records; set for the server's own channel only. */
  private feed: RecordFeed | null = null

  /** Feeds vornd's copy of the session records through this channel. */
  feedRecords(feed: RecordFeed): void {
    this.feed = feed
    feed.attach(this.recordSink)
  }

  /**
   * vornd's copy of the session records, as its changes are told. Each
   * terminal record it takes is emitted as `mirrored`, for the pty manager to
   * take the statuses from.
   */
  readonly mirror = new SessionMirror(
    () => this.resyncMirror(),
    (record, how) => this.emit('mirrored', record, how),
    (note) => this.emit('native', note)
  )

  /** Whether vornd runs the project scripts (`native`) or compares the plans of this server's (`shadow`). */
  scriptMode(): 'native' | 'shadow' | null {
    return this.inUse() ? (this.scriptWork ?? null) : null
  }

  /**
   * Claim `transcriptId` for session `sessionId` in vornd. Answers the session
   * already starting on it, or undefined when the claim is taken; a channel
   * that fails takes it, as a claim here would.
   */
  async claim(transcriptId: string, sessionId: string): Promise<string | undefined> {
    const channel = this.channel
    if (!channel) return undefined
    try {
      const answer = await channel.request<{ holder?: string | null }>('vornd:claim', {
        transcriptId,
        sessionId
      })
      return answer?.holder ?? undefined
    } catch (err) {
      log.warn({ err, transcriptId }, '[vornd] could not claim a conversation in vornd')
      return undefined
    }
  }

  /** Let go of a claim, or with no transcript every claim `sessionId` holds. */
  unclaim(sessionId: string, transcriptId?: string): void {
    this.channel?.notify('vornd:unclaim', { sessionId, ...(transcriptId ? { transcriptId } : {}) })
  }

  /** `sessionId`'s workspace is being prepared: its claims do not lapse meanwhile. */
  preparing(sessionId: string): void {
    this.channel?.notify('vornd:preparing', { sessionId })
  }

  prepared(sessionId: string): void {
    this.channel?.notify('vornd:prepared', { sessionId })
  }

  /** Where whether this server is winding down is read, for vornd. */
  setClosingSource(source: () => Closing): void {
    this.closingSource = source
  }

  /** Tell vornd whether this server is winding down, if that changed since it was told. */
  tellClosing(): void {
    const source = this.closingSource
    if (!source || !this.inUse()) return
    const closing = source()
    const json = JSON.stringify(closing)
    if (json === this.toldClosing) return
    this.toldClosing = json
    this.channel?.notify('vornd:draining', closing)
  }

  /**
   * Tells vornd whether this server is winding down on every subscription,
   * and looks again every few seconds: losing the endpoint is noticed by
   * looking, and vornd creates terminals without asking this server first.
   */
  private watchClosing(): void {
    this.toldClosing = ''
    this.tellClosing()
    if (this.closingTimer) return
    this.closingTimer = setInterval(() => this.tellClosing(), CLOSING_CHECK_MS)
    this.closingTimer.unref?.()
  }

  /** A status an agent's hook reported, and whether its hooks report the status from now on. */
  hookStatus(id: string, status: AgentStatus | null, promote: boolean): void {
    this.channel?.notify('vornd:hookStatus', { id, ...(status ? { status } : {}), promote })
  }

  /** Something was written to a terminal: an idle or waiting one runs again. */
  input(id: string): void {
    this.channel?.notify('vornd:input', { id })
  }

  /** Set fields vornd decides of a terminal's record (`hookSessionId`); null takes one away. */
  patch(id: string, fields: Partial<Record<PatchField, string | boolean | null>>): void {
    const baseRev = this.mirror.terminal(id)?.rev
    this.channel?.notify('vornd:patch', {
      id,
      fields,
      ...(baseRev === undefined ? {} : { baseRev })
    })
  }

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
    let hello: Hello
    try {
      channel = await VorndChannel.connect(endpoint)
      hello = await channel.request<Hello>('vornd:hello')
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
    this.scriptWork = hello?.scripts ?? null
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
    // Before the subscription, which then answers with the copy as fed.
    this.feed?.snapshot()
    await this.subscribe(channel)
    return !channel.isClosed
  }

  /** Take stock: every session vornd holds, those that ended, the notifications kept. */
  private async subscribe(channel: VorndChannel): Promise<void> {
    let state: Subscribed
    try {
      // When this machine came up, for which offered sessions a reboot ended.
      state = await channel.request<Subscribed>('vornd:subscribe', { bootTime: bootTime() })
    } catch (err) {
      log.warn({ err }, '[vornd] could not subscribe to vornd')
      return
    }
    if (this.channel !== channel) return
    this.emit('subscribed')
    this.holderTold(state.connected)
    if (state.registry) this.mirror.load(state.registry)
    // vornd owns the records: the database's records of the last run are handed over once.
    this.emit('restores')
    this.watchClosing()
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
   * A session vornd is starting itself, for a client's create: followed here
   * as one this server asked for, and started when vornd says its program is
   * up. `watched` for one whose output this server reads: a headless agent.
   */
  follow(id: string, watched = false): VorndPty {
    const pty = new VorndPty(this, id, watched)
    this.ptys.set(id, pty)
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
      case 'vornd:session':
        if (channel === this.channel) this.mirror.apply(p as unknown as SessionNote)
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

  /** Ask vornd `method` with `params`: its answer, or null with no channel. Throws when it refuses. */
  async ask<T>(method: string, params: unknown): Promise<T | null> {
    const channel = this.channel
    if (!channel || channel.isClosed) return null
    return channel.request<T>(method, params)
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
    this.emit('notify', note.id, note.title ?? '', note.body ?? '', note.effectId)
  }

  /**
   * vornd's copy of the session records, whole; null without a channel. Asked on the same channel the records go out on, so the answer
   * reflects every record sent before it.
   */
  async registry(): Promise<RegistrySnapshot | null> {
    const channel = this.channel
    if (!channel) return null
    return channel.request<RegistrySnapshot>('vornd:registry')
  }

  /** The mirror missed a change: it starts again from the copy whole. */
  private resyncMirror(): void {
    this.registry()
      .then((snapshot) => {
        if (snapshot) this.mirror.load(snapshot)
      })
      .catch((err: Error) => log.warn({ err }, '[vornd] could not read the session registry'))
  }

  /**
   * Hand vornd the records this server's database kept of its last run, for
   * it to offer; answers how many it took. A vornd
   * that read its own records back takes none.
   */
  async carry(terminals: TerminalSession[]): Promise<number | null> {
    const channel = this.channel
    if (!channel) return null
    try {
      const answer = await channel.request<{ carried?: number }>('vornd:carry', { terminals })
      return answer?.carried ?? 0
    } catch (err) {
      log.warn({ err }, '[vornd] could not hand the last run’s session records to vornd')
      return null
    }
  }

  /** Take stock of what vornd holds again: after a handover it took, what it adopted from it. */
  restock(): void {
    const channel = this.channel
    if (channel) void this.subscribe(channel)
  }

  /** Close the channel, for tests and a server on its way out. */
  close(): void {
    const channel = this.channel
    this.channel = null
    clearInterval(this.closingTimer)
    this.closingTimer = undefined
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

/** vornd's copy of the session records, whole, at one revision. */
export interface RegistrySnapshot {
  /** Which vornd: drawn when it started. Revisions of another one mean nothing here. */
  gen: string
  rev: number
  terminals: TerminalSession[]
  headless: HeadlessSession[]
  order: string[]
  holds: Record<string, number>
  /** The workspaces vornd holds itself while it prepares a session, when it holds any. */
  nativeHolds?: Record<string, number>
  /** The sessions of earlier runs still offered to resume, when vornd owns the records. */
  restored?: RestoredSession[]
}

/** The fields of a terminal's record `vornd:patch` may set. */
export type PatchField =
  | 'displayName'
  | 'renamedByPerson'
  | 'groupId'
  | 'hookSessionId'
  | 'agentSessionId'
  | 'statusSource'

/** One change to vornd's copy, as `vornd:session` tells it. */
export interface SessionNote {
  gen: string
  rev: number
  op: 'upsert' | 'remove' | 'order' | 'holds' | 'snapshot' | 'restored'
  kind?: RecordKind
  record?: TerminalSession | HeadlessSession
  id?: string
  order?: string[]
  holds?: Record<string, number>
  terminals?: TerminalSession[]
  headless?: HeadlessSession[]
  nativeHolds?: Record<string, number>
  /** The sessions of earlier runs still offered, as they are now. */
  restored?: RestoredSession[]
  /** A change vornd made itself, for a client's call, rather than one this server told it. */
  native?: boolean
  /** With `native`: the terminal or headless agent vornd created. */
  created?: boolean
  /** With `created`: a session of the last run the holder still held, live again in the copy. */
  adopted?: boolean
  /** With `created`: a session started again under the id it had. */
  resumed?: boolean
  /** With a `remove`: let go of quietly, its conversation running elsewhere. */
  released?: boolean
  /** With `native`: its program is up. */
  started?: { pid: number; epoch: number }
  /** With `native`: its program could not be started, and why. */
  failed?: string
  /** With `native`: the order a client set. */
  reordered?: boolean
  /** With `native`: its worktree's branch was renamed, or the worktree moved, for a client. */
  moved?: boolean
}

/**
 * How a terminal record reached the mirror: whole in a snapshot (`load`), as a
 * change vornd made to its fields for a client (`native`), or any other way.
 */
export type MirroredHow = 'load' | 'native' | 'note'

/** What `SessionMirror.apply` did with a note. */
export type MirrorOutcome = 'applied' | 'stale' | 'resync' | 'waiting'

/**
 * vornd's copy of the session records, followed here.
 *
 * Changes are applied in revision order. A note from another generation (vornd
 * restarted) or one that skips a revision means something was missed, and the
 * mirror asks for the copy whole (`resync`); notes that arrive meanwhile wait
 * and are applied on top of it. A note at or below the revision held was
 * applied already, and is dropped.
 *
 * Records are frozen outside production, so code that changes one in place
 * instead of asking vornd fails where it is written.
 */
export class SessionMirror {
  private gen: string | null = null
  private rev = 0
  private readonly terminalRecords = new Map<string, TerminalSession>()
  private readonly headlessRecords = new Map<string, HeadlessSession>()
  private terminalOrder: string[] = []
  private heldDirs: Record<string, number> = {}
  private nativeDirs: Record<string, number> = {}
  private restoredList: RestoredSession[] = []
  /** Notes waiting for a snapshot; null once one has been loaded and nothing is missing. */
  private waiting: SessionNote[] | null = []

  /**
   * @param resync Asks for the copy whole, after a note was missed.
   * @param took Each terminal record taken, from a note or a snapshot, once it is in.
   * @param native Each change vornd made itself, and every change to the
   *   workspaces it holds, once it is in.
   */
  constructor(
    private readonly resync: () => void,
    private readonly took: (record: TerminalSession, how: MirroredHow) => void = () => {},
    private readonly native: (note: SessionNote) => void = () => {}
  ) {}

  /** Where the mirror stands: null until it has loaded a snapshot. */
  get revision(): { gen: string; rev: number } | null {
    return this.gen === null ? null : { gen: this.gen, rev: this.rev }
  }

  /** Starts again from the copy whole, then applies what waited for it. */
  load(snapshot: RegistrySnapshot): void {
    this.gen = snapshot.gen
    this.rev = snapshot.rev
    this.terminalRecords.clear()
    for (const r of snapshot.terminals) this.terminalRecords.set(r.id, freeze(r))
    this.headlessRecords.clear()
    for (const r of snapshot.headless) this.headlessRecords.set(r.id, freeze(r))
    this.terminalOrder = [...snapshot.order]
    this.heldDirs = { ...snapshot.holds }
    this.nativeDirs = { ...snapshot.nativeHolds }
    this.restoredList = (snapshot.restored ?? []).map(freeze)
    const waited = (this.waiting ?? [])
      .filter((n) => n.gen === snapshot.gen && n.rev > snapshot.rev)
      .sort((a, b) => a.rev - b.rev)
    this.waiting = null
    for (const r of this.terminalRecords.values()) this.took(r, 'load')
    this.native({
      gen: snapshot.gen,
      rev: snapshot.rev,
      op: 'holds',
      holds: this.heldDirs,
      nativeHolds: this.nativeDirs
    })
    for (const note of waited) this.apply(note)
  }

  apply(note: SessionNote): MirrorOutcome {
    if (this.waiting) {
      this.waiting.push(note)
      return 'waiting'
    }
    if (note.op === 'snapshot' && note.terminals && note.headless) {
      if (note.gen === this.gen && note.rev <= this.rev) return 'stale'
      this.load({
        gen: note.gen,
        rev: note.rev,
        terminals: note.terminals,
        headless: note.headless,
        order: note.order ?? [],
        holds: note.holds ?? {},
        nativeHolds: note.nativeHolds,
        restored: note.restored
      })
      return 'applied'
    }
    if (note.gen !== this.gen) return this.missed(note)
    if (note.rev <= this.rev) return 'stale'
    if (note.rev !== this.rev + 1) return this.missed(note)
    let took: TerminalSession | undefined
    switch (note.op) {
      case 'upsert':
        if (note.kind === 'terminal' && note.record) {
          took = freeze(note.record as TerminalSession)
          this.terminalRecords.set(took.id, took)
        } else if (note.kind === 'headless' && note.record)
          this.headlessRecords.set(note.record.id, freeze(note.record as HeadlessSession))
        else return this.missed(note)
        break
      case 'remove':
        if (note.kind === 'terminal' && note.id) this.terminalRecords.delete(note.id)
        else if (note.kind === 'headless' && note.id) this.headlessRecords.delete(note.id)
        else return this.missed(note)
        break
      case 'order':
        this.terminalOrder = [...(note.order ?? [])]
        break
      case 'holds':
        this.heldDirs = { ...(note.holds ?? {}) }
        this.nativeDirs = { ...note.nativeHolds }
        break
      case 'restored':
        this.restoredList = (note.restored ?? []).map(freeze)
        break
      default:
        return this.missed(note)
    }
    this.rev = note.rev
    if (took) this.took(took, fieldsSet(note) ? 'native' : 'note')
    if (note.native || note.op === 'holds') this.native(note)
    return 'applied'
  }

  /** The terminals, listed as the pty manager lists them: the order first, then the rest. */
  terminals(): TerminalSession[] {
    const listed: TerminalSession[] = []
    for (const id of this.terminalOrder) {
      const r = this.terminalRecords.get(id)
      if (r) listed.push(r)
    }
    const ordered = new Set(this.terminalOrder)
    for (const r of this.terminalRecords.values()) if (!ordered.has(r.id)) listed.push(r)
    return listed
  }

  headless(): HeadlessSession[] {
    return [...this.headlessRecords.values()]
  }

  terminal(id: string): TerminalSession | undefined {
    return this.terminalRecords.get(id)
  }

  headlessRecord(id: string): HeadlessSession | undefined {
    return this.headlessRecords.get(id)
  }

  /** The sessions of earlier runs vornd still offers to resume. */
  restored(): RestoredSession[] {
    return [...this.restoredList]
  }

  /** Sessions at work in a worktree: terminals not idle, agents still running. */
  activeInWorktree(worktreePath: string): { count: number; sessionIds: string[] } {
    const sessionIds = [
      ...[...this.terminalRecords.values()]
        .filter((s) => s.worktreePath === worktreePath && s.status !== 'idle')
        .map((s) => s.id),
      ...[...this.headlessRecords.values()]
        .filter((s) => s.worktreePath === worktreePath && s.status === 'running')
        .map((s) => s.id)
    ]
    return { count: sessionIds.length, sessionIds }
  }

  /** The workspaces held while a session is prepared. */
  holds(): Readonly<Record<string, number>> {
    return this.heldDirs
  }

  /** The workspaces vornd holds itself while it prepares a session. */
  nativeHolds(): Readonly<Record<string, number>> {
    return this.nativeDirs
  }

  private missed(note: SessionNote): MirrorOutcome {
    this.waiting = [note]
    this.resync()
    return 'resync'
  }
}

/** Whether a note is vornd setting a terminal's fields for a client: a rename, a group. */
function fieldsSet(note: SessionNote): boolean {
  return (
    note.native === true &&
    note.op === 'upsert' &&
    !note.created &&
    note.started === undefined &&
    note.failed === undefined
  )
}

/** Frozen outside production: a record from the mirror is vornd's, not this server's to edit. */
function freeze<T extends object>(record: T): T {
  return process.env.NODE_ENV === 'production' ? record : Object.freeze(record)
}

export const vorndSessions = new VorndSessions()
vorndSessions.feedRecords(sessionFeed)
