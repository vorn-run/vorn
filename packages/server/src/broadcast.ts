import type { WebSocket } from 'ws'
import { createNotification } from '@vornrun/shared/protocol'
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
 * An entry is either an exact notification name (`config:changed`) or a namespace
 * wildcard (`session:*`). The wildcard is what keeps a shipped client correct:
 * a phone that asked for `session:*` also receives whatever is added to that
 * namespace after it was built.
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

  wants(method: string): boolean {
    if (this.exact.has(method)) return true
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

export class ClientRegistry {
  private clients = new Map<WebSocket, Subscription | null>()

  add(ws: WebSocket, topics?: TopicFilter): void {
    this.clients.set(ws, subscriptionFrom(topics))
    log.info(`[ws] client connected (total: ${this.clients.size})`)
  }

  remove(ws: WebSocket): void {
    this.clients.delete(ws)
    log.info(`[ws] client disconnected (total: ${this.clients.size})`)
  }

  /**
   * Narrow or widen what an already-connected socket receives.
   *
   * Ignored for a socket that was never admitted, so this cannot be used to add
   * an unauthenticated connection to the broadcast set.
   */
  setTopics(ws: WebSocket, topics: TopicFilter): void {
    if (!this.clients.has(ws)) return
    this.clients.set(ws, subscriptionFrom(topics))
  }

  broadcast(method: string, params: unknown): void {
    // Serialised on the first socket that actually wants it. With only a
    // filtered client attached, an unwanted notification now costs a map walk
    // instead of a full JSON encode of the payload.
    let msg: string | undefined
    for (const [ws, subscription] of this.clients) {
      if (ws.readyState !== ws.OPEN) continue
      if (subscription && !subscription.wants(method)) continue
      msg ??= JSON.stringify(createNotification(method, params))
      ws.send(msg)
    }
  }

  get size(): number {
    return this.clients.size
  }
}

export const clientRegistry = new ClientRegistry()
