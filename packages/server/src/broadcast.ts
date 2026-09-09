import type { WebSocket } from 'ws'
import { createNotification, type TerminalData } from '@vornrun/shared/protocol'
import { encodeTerminalFrame } from '@vornrun/shared/terminal-frame'
import log from './logger'

/**
 * Which pushes a socket wants.
 *
 * Every client used to get every notification, because the registry was a bare
 * set of sockets with nothing attached to them. That is right for a desktop, which
 * renders all of it, and wrong for a phone: `terminal:data` alone is every byte of
 * every PTY on the machine, and a phone renders none of it.
 *
 * Absent means everything, so nothing that exists today has to opt in and no
 * behaviour changes for a client that never asks.
 *
 * An entry is an exact notification name (`config:changed`), a namespace wildcard
 * (`session:*`), or one instance of a notification (`terminal:data#<id>`). The
 * wildcard is what keeps a shipped client correct: a phone that asked for
 * `session:*` also receives whatever is added to that namespace after it was
 * built.
 *
 * The instance form exists for `terminal:data`, where subscribing by name is
 * useless on a phone: it means every byte of every terminal on the machine when
 * what is wanted is the one on screen.
 */
export type TopicFilter = readonly string[] | undefined

class Subscription {
  private readonly exact = new Set<string>()
  private readonly prefixes: string[] = []

  constructor(topics: readonly string[]) {
    for (const topic of topics) {
      if (topic.endsWith('*')) this.prefixes.push(topic.slice(0, -1))
      else this.exact.add(topic)
    }
  }

  wants(method: string, scope: string | undefined): boolean {
    // The bare name still means every instance, so a desktop asking for
    // `terminal:data` is unaffected by the instance form existing.
    if (this.exact.has(method)) return true
    if (scope !== undefined && this.exact.has(`${method}#${scope}`)) return true
    for (const prefix of this.prefixes) if (method.startsWith(prefix)) return true
    return false
  }
}

/**
 * A filter, or `null` for "send everything".
 *
 * Malformed input reads as no filter rather than an empty one. Failing open costs
 * bandwidth; failing closed would silence a client completely, which looks like a
 * broken app rather than a bad parameter and is far harder to diagnose from the
 * other end of a phone.
 */
function subscriptionFrom(topics: TopicFilter): Subscription | null {
  if (topics === undefined) return null
  if (!Array.isArray(topics) || topics.some((t) => typeof t !== 'string')) {
    log.warn('[ws] ignoring a malformed topic list; this client will receive everything')
    return null
  }
  // An empty list widens to everything rather than narrowing to nothing, for
  // the same reason malformed input does. There is deliberately no way to
  // subscribe to nothing: a client that wants nothing can close the socket.
  if (topics.length === 0) return null
  return new Subscription(topics)
}

/**
 * The filter a client declares on the socket URL, as
 * `?topics=session:*,config:changed`.
 *
 * On the upgrade rather than in a frame because a filter that arrives later is a
 * filter that arrives too late: the socket is already in the broadcast set, and
 * on a busy machine that gap is enough PTY output to matter on a phone. It
 * happens on every reconnect, which on a mobile network is often.
 */
export function parseTopics(query: unknown): readonly string[] | undefined {
  const raw = (query as { topics?: unknown } | undefined)?.topics
  if (typeof raw !== 'string') return undefined
  const topics = raw
    .split(',')
    .map((topic) => topic.trim())
    .filter(Boolean)
  return topics.length > 0 ? topics : undefined
}

interface Client {
  subscription: Subscription | null
  /** Terminal output as a frame of bytes rather than a JSON string. */
  terminalBytes: boolean
}

/** Terminal output before the wire: the pty hands over text. */
export type TerminalText = Omit<TerminalData, 'data'> & { data: string }

function terminalFrame(payload: TerminalText): Uint8Array {
  return encodeTerminalFrame({
    id: payload.id,
    seq: payload.seq,
    data: Buffer.from(payload.data, 'utf-8')
  })
}

export class ClientRegistry {
  private clients = new Map<WebSocket, Client>()
  private lastActivity = Date.now()

  add(ws: WebSocket, topics?: TopicFilter): void {
    this.clients.set(ws, { subscription: subscriptionFrom(topics), terminalBytes: false })
    log.info(`[ws] client connected (total: ${this.clients.size})`)
  }

  remove(ws: WebSocket): void {
    this.clients.delete(ws)
    log.info(`[ws] client disconnected (total: ${this.clients.size})`)
  }

  /**
   * How long since a client last did anything.
   *
   * A timestamp rather than the count below, because MCP opens a fresh socket
   * for every RPC call: the count drops to zero between two calls of a working
   * agent, and anything sampling it at that instant reads a busy server as an
   * empty one. Recording *when* also fixes the opposite hole — these sockets
   * have no heartbeat and are only removed on close or error, so one half-open
   * connection would otherwise hold the count above zero for ever.
   */
  msSinceActivity(): number {
    return Date.now() - this.lastActivity
  }

  /**
   * Mark a client as having done something.
   *
   * Only real traffic: connecting is not activity, and neither is disconnecting.
   * Another Vorn deciding whether it may adopt this server opens a socket, reads
   * the greeting and closes it again — counting that would let the very launches
   * this feature exists to unblock keep the leftover alive for ever, one probe at
   * a time.
   */
  touch(): void {
    this.lastActivity = Date.now()
  }

  /**
   * Narrow or widen what an already-connected socket receives.
   *
   * Ignored for a socket that was never admitted, so this cannot be used to add
   * an unauthenticated connection to the broadcast set.
   */
  setTopics(ws: WebSocket, topics: TopicFilter, terminalBytes?: unknown): void {
    const client = this.clients.get(ws)
    if (!client) return
    // A field left out stays as it was: the desktop asks for bytes alone, the web client sends topics alone.
    if (topics !== undefined) client.subscription = subscriptionFrom(topics)
    if (terminalBytes !== undefined) client.terminalBytes = terminalBytes === true
  }

  /**
   * `scope` names which instance of `method` this is, for notifications that
   * have instances. Derived by the caller rather than dug out of `params` here,
   * so the registry keeps knowing nothing about any particular payload shape.
   */
  broadcast(method: string, params: unknown, scope?: string): void {
    // Each wire form is built on the first socket that wants it, and not at all otherwise.
    let text: string | undefined
    let frame: Uint8Array | undefined
    for (const [ws, client] of this.clients) {
      if (ws.readyState !== ws.OPEN) continue
      if (client.subscription && !client.subscription.wants(method, scope)) continue
      if (client.terminalBytes && method === 'terminal:data') {
        ws.send((frame ??= terminalFrame(params as TerminalText)))
      } else {
        ws.send((text ??= JSON.stringify(createNotification(method, params))))
      }
    }
  }

  get size(): number {
    return this.clients.size
  }
}

export const clientRegistry = new ClientRegistry()
