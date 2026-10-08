import { IPC } from '@vornrun/shared/types'
import log from './logger'
import { VORN_PEER_HEADER } from './vornd-relay'

/**
 * What vornd asks of this server, and tells it, once it answers the pairing
 * and token calls itself.
 *
 * vornd holds pairing and writes device tokens, and runs workflows and
 * artifacts, but the clients are this server's: some connect here directly,
 * and every socket a token opened is one of this server's. So vornd asks for
 * a broadcast when a phone asks to pair or collects its token, and as runs
 * move and artifacts change, and says which token it revoked so the sockets
 * holding it are closed. In turn it is told where this server is bound, which
 * decides the addresses a browser on the network can use, and when to read
 * again the names a browser may load the web client from.
 */

/** The broadcasts vornd may ask for: what pairing, runs, artifacts, extensions, connectors and the configuration announce. */
const BROADCASTS: ReadonlySet<string> = new Set([
  IPC.CONFIG_CHANGED,
  IPC.CONNECTOR_INSTALL_PROGRESS,
  IPC.CONNECTOR_CATALOG_CHANGED,
  IPC.PAIRING_REQUESTED,
  IPC.PAIRING_COLLECTED,
  IPC.WORKFLOW_RUN_UPDATED,
  IPC.WORKFLOW_GATE_RESOLVED,
  IPC.ARTIFACT_PUBLISHED,
  IPC.ARTIFACT_COMMENTS_CHANGED,
  IPC.EXTENSION_ACTIVATION,
  IPC.EXTENSION_FOOTER_ITEMS,
  IPC.EXTENSION_SELECTION_REQUEST,
  IPC.WIDGET_STATUS_UPDATE,
  IPC.SCRIPT_DATA,
  IPC.SCRIPT_EXIT,
  IPC.WIDGET_PERMISSION_REQUEST,
  IPC.WIDGET_PERMISSION_CANCELLED
])

/** What vornd last said of agents' hooks (`vornd:hooks`), which keeps this server from stopping as idle. */
const hooks = { lastAt: Date.now(), pending: 0 }

/** How long since a hook posted to vornd, and how many permission requests it holds open. */
export function hookActivity(): { msSinceHookActivity: number; pendingPermissions: number } {
  return { msSinceHookActivity: Date.now() - hooks.lastAt, pendingPermissions: hooks.pending }
}

export interface ReachDeps {
  /** vornd's channel: what it asks, when it is (re)subscribed, and telling it. */
  channel: {
    on(event: 'ask', listener: (method: string, params: unknown) => void): unknown
    on(event: 'subscribed', listener: () => void): unknown
    tell(method: string, params: unknown): Promise<boolean>
  }
  /** `scope` is the session a push is about, for the clients subscribed to one. */
  broadcast: (method: string, params: unknown, scope?: string) => void
  disconnectToken: (tokenId: string) => number
  /** The address this server is bound to now. */
  host: () => string
}

/** Wire vornd's asks to this server; answers what to call when the binding may have changed. */
export function linkReach(deps: ReachDeps): { hostChanged(): void } {
  const tellHost = (): void => {
    void deps.channel.tell('vornd:reach', { host: deps.host() })
  }
  deps.channel.on('subscribed', tellHost)
  deps.channel.on('ask', (method, params) => {
    const p = (params ?? {}) as Record<string, unknown>
    switch (method) {
      case 'vornd:broadcast':
        if (typeof p.method === 'string' && BROADCASTS.has(p.method)) {
          deps.broadcast(p.method, p.params, typeof p.scope === 'string' ? p.scope : undefined)
        } else {
          log.warn({ method: p.method }, '[vornd] refused to broadcast for vornd')
        }
        return
      case 'vornd:hooks':
        hooks.lastAt = Date.now()
        if (typeof p.pending === 'number') hooks.pending = p.pending
        return
      case 'vornd:tokenRevoked':
        if (typeof p.tokenId === 'string') deps.disconnectToken(p.tokenId)
        return
    }
  })
  return { hostChanged: tellHost }
}

/** Set by vornd on a pairing request it hands this server, which then answers it here. */
export const VORND_FORWARDED_HEADER = 'x-vornd-forwarded'

/** How long vornd gets to answer a phone's pairing request before this server does. */
const PAIR_RELAY_TIMEOUT_MS = 10_000

/**
 * Hand a phone's pairing request to vornd, which holds pairing once it answers
 * the `pairing` calls: the code the desktop shows was made there. Answers
 * vornd's status and body, or null when this server is to answer it itself:
 * vornd does not hold pairing, the request came from vornd, or vornd could not
 * be reached.
 */
export async function relayPairing(
  request: { url: string; body: unknown; ip: string; fromVornd: boolean },
  vorndPort: number | null
): Promise<{ status: number; body: unknown } | null> {
  if (vorndPort === null || request.fromVornd) return null
  try {
    const res = await fetch(`http://127.0.0.1:${vorndPort}${request.url}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', [VORN_PEER_HEADER]: request.ip },
      body: JSON.stringify(request.body ?? null),
      signal: AbortSignal.timeout(PAIR_RELAY_TIMEOUT_MS)
    })
    return { status: res.status, body: await res.json() }
  } catch (err) {
    log.warn({ err }, '[vornd] could not relay a pairing request to vornd; answering here')
    return null
  }
}
