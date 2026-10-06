import type { HeadlessSession, TerminalSession } from '@vornrun/shared/types'
import { heldWorkspaces, onHoldsChanged } from './workspace-holds'

/**
 * What this server tells vornd of its session records.
 *
 * While vornd runs native work (Settings › Experimental › Native server, or a
 * per-group setting), it keeps a copy of this server's terminal and headless
 * records, and answers the calls that read them from it in shadow mode, so the
 * two can be compared. The copy is only as good as what it is told: every
 * place that changes a record, here or anywhere that holds one, says so here.
 */

/** The record cursor of the effect that set a state: `{epoch, rseq, index}`, compared in that order. */
export interface Stamp {
  epoch: number
  rseq: number
  index: number
}

/** The two kinds of session record this server keeps. */
export type RecordKind = 'terminal' | 'headless'

/** Where `RecordFeed` reads the whole registry from, for a snapshot. */
export interface RecordSource {
  /** Every terminal record, as `terminal:listActive` lists them. */
  terminals(): TerminalSession[]
  /** The order the terminals are listed in. */
  order(): string[]
  /** The terminals whose programs ended: their records are this server's whole again. */
  ended?(): string[]
}

/** The channel a `RecordFeed` writes to: vornd's, through `VorndSessions`. */
export interface RecordSink {
  /** Whether vornd wants the records now: it runs native work, and is connected. */
  wants(): boolean
  send(params: Record<string, unknown>): void
}

/**
 * What this server tells vornd's copy of its session records (`vornd:record`).
 *
 * The pty and headless managers call it after every change they make to a
 * record, and so does every other place that changes one in place. Each call
 * is cheap and safe to repeat: a record is sent only when its JSON differs from
 * what was last sent, and nothing at all is done unless vornd asked (`wants`),
 * so with the Native server switch off this costs one boolean.
 *
 * A status or exit set by an effect vornd told is sent with that effect's
 * cursor (`statusAt`, `exitAt`), so vornd keeps the newer of two states told
 * out of order. One set any other way (a hook, a timer, the person typing)
 * clears it: it is the newest word on the session.
 *
 * A terminal whose program ended is sent with `ended`: while vornd decides the
 * statuses (`vornd-sessions.ts`), that is what gives the record back to this
 * server, status included.
 */
export class RecordFeed {
  private terminalSource: RecordSource | null = null
  private headlessSource: (() => HeadlessSession[]) | null = null
  /** What was last sent of each record, by kind and id. */
  private readonly told = new Map<string, string>()
  private readonly stamps = new Map<string, { statusAt?: Stamp; exitAt?: Stamp }>()
  private toldOrder = ''
  private toldHolds = ''

  private sink: RecordSink | null = null

  constructor() {
    onHoldsChanged(() => this.holds())
  }

  /** Where the records go from now on. */
  attach(sink: RecordSink | null): void {
    this.sink = sink
  }

  private wants(): boolean {
    return this.sink?.wants() === true
  }

  private send(params: Record<string, unknown>): void {
    this.sink?.send(params)
  }

  /** Where a snapshot reads the terminals from: the pty manager. */
  setTerminalSource(source: RecordSource): void {
    this.terminalSource = source
  }

  /** Where a snapshot reads the headless agents from: the headless manager. */
  setHeadlessSource(source: () => HeadlessSession[]): void {
    this.headlessSource = source
  }

  /** The effect that set this terminal's status, or null for anything else. */
  statusAt(id: string, at: Stamp | null): void {
    this.stamp(`terminal/${id}`, 'statusAt', at)
  }

  /** The effect that ended this session, or null when none was told. */
  exitAt(kind: RecordKind, id: string, at: Stamp | null): void {
    this.stamp(`${kind}/${id}`, 'exitAt', at)
  }

  /** A terminal record was created or changed; `ended` once its program has. */
  terminal(session: TerminalSession, ended = false): void {
    this.upsert('terminal', session, ended)
  }

  /** A headless agent's record was created or changed. */
  headless(session: HeadlessSession): void {
    this.upsert('headless', session)
  }

  /** The server let go of a record. */
  remove(kind: RecordKind, id: string): void {
    const key = `${kind}/${id}`
    this.stamps.delete(key)
    if (!this.wants()) return
    if (!this.told.delete(key)) return
    this.send({ op: 'remove', kind, id })
  }

  /** The order the terminals are listed in changed. */
  order(ids: readonly string[]): void {
    if (!this.wants()) return
    const json = JSON.stringify(ids)
    if (json === this.toldOrder) return
    this.toldOrder = json
    this.send({ op: 'order', order: ids })
  }

  /** The workspaces held while a session is prepared changed. */
  holds(): void {
    if (!this.wants()) return
    const holds = heldWorkspaces()
    const json = JSON.stringify(holds)
    if (json === this.toldHolds) return
    this.toldHolds = json
    this.send({ op: 'holds', holds })
  }

  /** Everything, for a vornd that has just connected: what it held of a previous run goes. */
  snapshot(): void {
    if (!this.wants()) return
    const terminals = this.terminalSource?.terminals() ?? []
    const headless = this.headlessSource?.() ?? []
    const order = this.terminalSource?.order() ?? []
    const ended = this.terminalSource?.ended?.() ?? []
    const holds = heldWorkspaces()
    this.told.clear()
    for (const s of terminals) {
      const key = `terminal/${s.id}`
      this.told.set(key, this.fingerprint('terminal', s, ended.includes(s.id)))
    }
    for (const s of headless) this.told.set(`headless/${s.id}`, this.fingerprint('headless', s))
    this.toldOrder = JSON.stringify(order)
    this.toldHolds = JSON.stringify(holds)
    this.send({
      op: 'snapshot',
      terminals,
      headless,
      order,
      holds,
      ...(ended.length ? { ended } : {})
    })
  }

  private stamp(key: string, field: 'statusAt' | 'exitAt', at: Stamp | null): void {
    if (!this.wants()) return
    const stamps = this.stamps.get(key) ?? {}
    if (at) stamps[field] = at
    else delete stamps[field]
    if (stamps.statusAt || stamps.exitAt) this.stamps.set(key, stamps)
    else this.stamps.delete(key)
  }

  private fingerprint(kind: RecordKind, record: object, ended = false): string {
    const stamps = this.stamps.get(`${kind}/${(record as { id: string }).id}`)
    return JSON.stringify(record) + (stamps ? JSON.stringify(stamps) : '') + (ended ? '/ended' : '')
  }

  private upsert(kind: RecordKind, record: TerminalSession | HeadlessSession, ended = false): void {
    if (!this.wants()) return
    const key = `${kind}/${record.id}`
    const fingerprint = this.fingerprint(kind, record, ended)
    if (this.told.get(key) === fingerprint) return
    this.told.set(key, fingerprint)
    this.send({ op: 'upsert', kind, record, ...this.stamps.get(key), ...(ended ? { ended } : {}) })
  }
}

/** The server's one feed, which `vorndSessions` sends on. */
export const sessionFeed = new RecordFeed()
