import type { WebSocket } from 'ws'
import type {
  RpcRequest,
  RpcResponse,
  RequestMethod,
  RequestMethods
} from '@vornrun/shared/protocol'
import {
  createResponse,
  createErrorResponse,
  createNotification,
  RUNTIME_PROTOCOL_VERSION,
  CLOSE_UNAUTHENTICATED,
  CLOSE_CREDENTIAL_REJECTED,
  RPC_NOT_AUTHENTICATED,
  type ClientNotification,
  type ClientNotifications,
  type ServerIdentity
} from '@vornrun/shared/protocol'
import { authenticateCredential, AUTH_TIMEOUT_MS, type Authenticated } from './ws-auth'
import { clientRegistry } from './broadcast'
import { browserBridge } from './browser-bridge'
import log from './logger'

/**
 * What this server can do, declared by the code that implements it rather than
 * written out at the greeting. `auth` is registered below, next to the check
 * that enforces it — co-location, not a guarantee: nothing mechanically ties the
 * two together, so a capability removed here still needs its check removed.
 */
const capabilities = new Map<string, number>()

export function registerCapability(name: string, version: number): void {
  capabilities.set(name, version)
  helloFrameCache = null
}

let helloFrameCache: string | null = null

/**
 * Who this server is, for a desktop deciding whether to adopt it.
 *
 * Set once at startup by the entry point, which is the only place that knows the
 * resolved data directory. Left unset in tests and in any embedding that does not
 * call it, and a hello without identity simply is not adoptable — the launcher
 * spawns its own rather than guessing.
 */
let identity: ServerIdentity | null = null

export function setServerIdentity(next: ServerIdentity): void {
  identity = next
}

function helloFrame(): string {
  // Built on first use, not at module load: another module may still be
  // registering a capability at import time. It never varies after that.
  helloFrameCache ??= JSON.stringify(
    createNotification('server:hello', {
      protocolVersion: RUNTIME_PROTOCOL_VERSION,
      capabilities: Object.fromEntries(capabilities)
    })
  )
  return helloFrameCache
}

/**
 * Who this server is — sent only to a peer on this machine.
 *
 * `dataDir` names the user's home directory, so it carries the account name,
 * and with remote access enabled the server binds `0.0.0.0` where the Origin
 * allowlist does not apply to a peer that simply sends no Origin at all. The
 * one caller with any use for these fields is a desktop on this machine
 * deciding whether to adopt this server rather than start a second one.
 */
function identityFrame(): string | null {
  if (!identity) return null
  // Built per connection rather than cached: the session count is the point of
  // sending it, and a count fixed at boot would be a lie by the second socket.
  // The frame goes only to loopback peers, so this is not a per-request cost on
  // anything remote.
  return JSON.stringify(
    createNotification('server:identity', { ...identity, sessions: liveSessionCount?.() })
  )
}

/**
 * How many terminals are live, when somebody has told us how to ask.
 *
 * A function rather than a number because the count changes; injected rather
 * than imported because `ws-handler` has no business knowing about PTYs, and a
 * direct import would put the socket layer downstream of the terminal layer.
 */
let liveSessionCount: (() => number) | null = null

export function setLiveSessionCount(fn: () => number): void {
  liveSessionCount = fn
}

/**
 * Whether a peer address is this machine talking to itself.
 *
 * IPv4-mapped IPv6 (`::ffff:127.0.0.1`) is what a dual-stack listener actually
 * reports for a v4 loopback connection, so matching only `127.0.0.1` would fail
 * open in the common case -- and failing open here means withholding nothing.
 */
/**
 * Where a connection came in.
 *
 * A unix peer has no address at all -- there is nothing to put in one, and
 * `isLoopbackAddress(undefined)` is correctly false. Passing a synthetic
 * `'127.0.0.1'` would make it true by lying, in the one place whose whole job is
 * to decide who may be told about this machine. So the transport is named
 * instead, and a unix peer is judged on what is actually true of it: it reached a
 * socket inside a directory this user owns, which is stronger evidence of being
 * on this machine than any address could be.
 */
export type Peer = { transport: 'unix' } | { transport: 'tcp'; address?: string }

/** Whether this peer is provably on the same machine, and may hear who we are. */
export function isSameMachine(peer: Peer | undefined): boolean {
  if (!peer) return false
  return peer.transport === 'unix' || isLoopbackAddress(peer.address)
}

export function isLoopbackAddress(address: string | undefined): boolean {
  if (!address) return false
  const bare = address.startsWith('::ffff:') ? address.slice('::ffff:'.length) : address
  return bare === '::1' || bare === '127.0.0.1' || bare.startsWith('127.')
}

// Handler registry: method name → async handler function
type Handler = (params: unknown) => Promise<unknown> | unknown
const handlers = new Map<string, Handler>()

registerCapability('auth', 1)

// Declared so a client knows the server will honour a topic list before it sends
// one. Without the check, an older server silently drops `subscribe:set` and the
// client believes it is filtered while receiving everything.
registerCapability('subscribe', 1)

/**
 * Register a method handler. Called during server startup to wire up
 * manager methods to the WS protocol.
 */
export function registerMethod<M extends RequestMethod>(
  method: M,
  handler: (
    params: RequestMethods[M]['params']
  ) => Promise<RequestMethods[M]['result']> | RequestMethods[M]['result']
): void {
  handlers.set(method, handler as Handler)
}

/**
 * Register a fire-and-forget notification handler (no response sent).
 *
 * Typed against `ClientNotifications` the way `registerMethod` is against
 * `RequestMethods`. It used to take a bare `string` and `unknown`, so a handler
 * could not destructure its own parameters without an implicit `any`, and the
 * payload shape was checked on neither side of the wire — for `terminal:write`,
 * whose params reach a PTY.
 */
export function registerNotification<N extends ClientNotification>(
  method: N,
  handler: (params: ClientNotifications[N]) => void
): void {
  handlers.set(`notify:${method}`, handler as Handler)
}

/**
 * Handle a new WebSocket connection. Sets up message parsing,
 * request dispatching, and cleanup on close.
 */
/**
 * Which live sockets are holding which device token.
 *
 * `clientRegistry` keeps a bare set of sockets with no identity attached, so
 * there was no way to answer "who is using this token" — and revoking one only
 * took effect whenever that device next happened to reconnect. For a lost phone
 * that is the wrong answer: the point of revoking is that it stops now.
 *
 * A Set per token because one device can hold several sockets at once — MCP opens
 * a fresh connection per call.
 */
const socketsByToken = new Map<string, Set<WebSocket>>()

/**
 * How many sockets may sit unauthenticated at once.
 *
 * Each one holds a slot for the full grace window while it proves nothing, so
 * without a ceiling anyone who can reach the port can hold every slot open with a
 * loop that connects and says nothing — no credential needed, which is the point.
 * The number is far above any real client: a browser uses one, and MCP opens one
 * per call but authenticates on the upgrade and so never counts here.
 */
const MAX_PENDING_SOCKETS = 64

let pendingSockets = 0

function trackToken(tokenId: string, ws: WebSocket): void {
  const existing = socketsByToken.get(tokenId)
  if (existing) existing.add(ws)
  else socketsByToken.set(tokenId, new Set([ws]))
}

function untrackToken(tokenId: string, ws: WebSocket): void {
  const sockets = socketsByToken.get(tokenId)
  if (!sockets) return
  sockets.delete(ws)
  if (sockets.size === 0) socketsByToken.delete(tokenId)
}

/**
 * Close every socket authenticated with this token.
 *
 * Closed with CLOSE_CREDENTIAL_REJECTED rather than a generic code on purpose:
 * that is the one the web client turns into a request for a new token, where
 * anything else it simply retries — which for a revoked token is an endless loop
 * against a door that will not open again.
 */
export function disconnectToken(tokenId: string): number {
  const sockets = socketsByToken.get(tokenId)
  if (!sockets) return 0
  const count = sockets.size
  for (const ws of [...sockets]) {
    try {
      ws.close(CLOSE_CREDENTIAL_REJECTED, 'token revoked')
    } catch {
      // Already gone; `teardown` on its close event clears the entry.
    }
  }
  socketsByToken.delete(tokenId)
  if (count > 0) log.info({ tokenId, count }, '[ws] closed sockets for revoked token')
  return count
}

/** Test seam — module state outlives a single connection. */
export function resetTokenTracking(): void {
  socketsByToken.clear()
  pendingSockets = 0
}

export function handleConnection(
  ws: WebSocket,
  credential?: string,
  initialTopics?: readonly string[],
  peer?: Peer
): void {
  // Announce the contract first, so a client that has to authenticate by message
  // knows that it must before it is refused for not having.
  ws.send(helloFrame())
  if (isSameMachine(peer)) {
    const frame = identityFrame()
    if (frame) ws.send(frame)
  }

  let authTimer: NodeJS.Timeout | null = null

  /**
   * Who this socket authenticated as, or null.
   *
   * Per-connection state in a local, so it dies with the socket rather than
   * needing to be purged from a process-wide map on every exit path. `kind` and
   * `tokenId` are carried because they cannot be recovered later: only the
   * desktop may claim the browser bridge, and revoking a device token has to be
   * able to find the sockets holding it.
   */
  let session: Authenticated | null = null

  /** Admit the socket: only from here does it receive broadcasts. */
  const admit = (result: Authenticated): void => {
    session = result
    // Topics from the upgrade rather than a later frame, so a filtered client is
    // never briefly unfiltered. `subscribe:set` can only run after admission, and
    // in that gap a busy machine can push a lot of PTY output at a phone that
    // asked for none — on every reconnect, which on a mobile network is often.
    clientRegistry.add(ws, initialTopics)
    if (result.tokenId) trackToken(result.tokenId, ws)
    if (authTimer) {
      clearTimeout(authTimer)
      authTimer = null
      pendingSockets -= 1
    }
  }

  const refuse = (reason: string, code = CLOSE_UNAUTHENTICATED): void => {
    log.warn({ reason }, '[ws] refusing unauthenticated socket')
    ws.close(code, reason)
  }

  // A credential on the upgrade authenticates before the first frame. Node
  // clients use this path; MCP in particular opens a fresh connection per RPC
  // call, so a mandatory round-trip would tax every one of them.
  const upgraded = authenticateCredential(credential)
  if (upgraded) {
    admit(upgraded)
  } else if (credential !== undefined) {
    // Offered one and it was wrong. Distinct from offering none: waiting out the
    // grace window would answer with a timeout ten seconds later, and a timeout is
    // the code a client retries rather than the one that says the token is bad.
    // MCP opens a connection per call, so a stale token would otherwise leave one
    // socket parked for ten seconds on every single RPC.
    refuse('credential rejected', CLOSE_CREDENTIAL_REJECTED)
  } else {
    // Browsers cannot set headers, so they get a window to send one message.
    // Bounded, or an unauthenticated socket could sit open indefinitely.
    if (pendingSockets >= MAX_PENDING_SOCKETS) {
      log.warn({ pendingSockets }, '[ws] too many sockets waiting to authenticate')
      refuse('too many pending connections')
      return
    }
    pendingSockets += 1
    authTimer = setTimeout(() => {
      authTimer = null
      pendingSockets -= 1
      refuse('authentication timeout')
    }, AUTH_TIMEOUT_MS)
  }

  ws.on('message', async (raw: Buffer) => {
    let msg: RpcRequest
    try {
      msg = JSON.parse(raw.toString())
    } catch {
      log.warn('[ws] received non-JSON message')
      return
    }

    const { id, method, params } = msg
    // A frame from a socket that has proved itself counts as somebody being out
    // there, and the idle watch stays up for it. Two things do not count.
    //
    // An unauthenticated frame, because anything on this machine can open a
    // socket to loopback: counting those would let arbitrary local traffic pin a
    // server nobody is using, without ever proving it is a client. Same rule the
    // hook endpoint follows, and for the same reason.
    //
    // And `bridge:identify`, even authenticated, because `ServerBridge` sends it
    // from its own `open` handler -- so it arrives on every connection including
    // the one another Vorn opens purely to ask whether it may adopt this server.
    // Counting that would let a user blocked by a leftover reset its clock on
    // every launch attempt, so the leftover never leaves and the launches never
    // stop being blocked.
    if (session && method !== 'bridge:identify') clientRegistry.touch()

    // Everything below this line requires an authenticated socket. The one
    // exception is the credential itself.
    if (!session) {
      const deny = (message: string, reason: string, code: number): void => {
        if (id !== undefined && id !== null) {
          ws.send(JSON.stringify(createErrorResponse(id, RPC_NOT_AUTHENTICATED, message)))
        }
        refuse(reason, code)
      }

      if (method !== 'auth:authenticate') {
        return deny('Not authenticated', 'method before authentication', CLOSE_UNAUTHENTICATED)
      }

      const token = (params as { token?: string } | undefined)?.token
      const result = authenticateCredential(token)
      if (!result) {
        // A distinct code from the timeout: a client should discard a rejected
        // credential, but keep one whose socket merely stalled.
        return deny('Authentication failed', 'invalid credential', CLOSE_CREDENTIAL_REJECTED)
      }

      admit(result)
      ws.send(JSON.stringify(createNotification('auth:ok', { userId: result.userId })))
      if (id !== undefined && id !== null) {
        ws.send(JSON.stringify(createResponse(id, { ok: true })))
      }
      return
    }

    // A reply to something *we* asked main (browser:* reverse RPC). Responses
    // carry no `method`, so they must be recognised before the notification
    // branch below treats a method-less frame as junk.
    //
    // Only from the socket that actually holds the bridge: request ids are
    // negative and sequential, so any other socket could otherwise resolve a
    // pending bridge request by guessing one.
    if (method === undefined && id !== undefined && id !== null) {
      if (
        browserBridge.isBridgeSocket(ws) &&
        browserBridge.handleResponse(msg as unknown as RpcResponse)
      ) {
        return
      }
    }

    // Main identifying itself, so the reverse bridge knows which socket to use.
    if (method === 'bridge:identify') {
      // Any authenticated socket may claim this, and the reason it can is that the
      // claim is not an escalation: a device token already reaches `terminal:create`
      // and `script:execute`, so anything the bridge could reveal — a screenshot, a
      // page read — its holder could already take with a shell.
      //
      // It used to be restricted to the bootstrap credential, on the reasoning that
      // only the process holding the per-launch secret can be main. That stopped
      // being true when the desktop learned to connect to a server on another
      // machine: it authenticates there with a device token, so the restriction
      // silently cost host mode its browser and device panes while the connection
      // itself looked healthy.
      //
      // `setSocket` refusing while a live holder exists is what still matters, and
      // it is unchanged: one holder, first to ask, and a dead one is replaceable.
      const claimed = browserBridge.setSocket(ws)
      if (!claimed) {
        log.warn('[ws] refused a second bridge:identify while one is live')
      }
      if (id !== undefined && id !== null) {
        ws.send(JSON.stringify(createResponse(id, { ok: claimed })))
      }
      return
    }

    // Changing what this socket receives. Handled here rather than through
    // `registerNotification` because it is the socket that is being configured,
    // and a registered handler is given only its params.
    if (method === 'subscribe:set') {
      const topics = (params as { topics?: readonly string[] } | undefined)?.topics
      clientRegistry.setTopics(ws, topics)
      if (id !== undefined && id !== null) {
        ws.send(JSON.stringify(createResponse(id, { ok: true })))
      }
      return
    }

    // Fire-and-forget notification (no id)
    if (id === undefined || id === null) {
      const notifHandler = handlers.get(`notify:${method}`)
      if (notifHandler) {
        try {
          notifHandler(params)
        } catch (err) {
          log.error({ err, method }, '[ws] notification handler error')
        }
      }
      return
    }

    // Request-response
    const handler = handlers.get(method)
    if (!handler) {
      ws.send(JSON.stringify(createErrorResponse(id, -32601, `Method not found: ${method}`)))
      return
    }

    try {
      const result = await handler(params)
      ws.send(JSON.stringify(createResponse(id, result)))
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      log.error({ err, method }, '[ws] handler error')
      ws.send(JSON.stringify(createErrorResponse(id, -32000, message)))
    }
  })

  const teardown = (): void => {
    if (authTimer) {
      clearTimeout(authTimer)
      authTimer = null
      // Released here as well as on admission and timeout: a socket that simply
      // goes away mid-window would otherwise leak its slot, and enough of those
      // would close the door on everyone.
      pendingSockets -= 1
    }
    if (session?.tokenId) untrackToken(session.tokenId, ws)
    session = null
    clientRegistry.remove(ws)
    browserBridge.clearSocket(ws)
  }

  ws.on('close', teardown)

  ws.on('error', (err) => {
    log.error({ err }, '[ws] socket error')
    teardown()
  })
}
