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
  RPC_NOT_AUTHENTICATED
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

// Handler registry: method name → async handler function
type Handler = (params: unknown) => Promise<unknown> | unknown
const handlers = new Map<string, Handler>()

registerCapability('auth', 1)

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
 */
export function registerNotification(method: string, handler: (params: unknown) => void): void {
  handlers.set(`notify:${method}`, handler as Handler)
}

/**
 * Handle a new WebSocket connection. Sets up message parsing,
 * request dispatching, and cleanup on close.
 */
export function handleConnection(ws: WebSocket, credential?: string): void {
  // Announce the contract first, so a client that has to authenticate by message
  // knows that it must before it is refused for not having.
  ws.send(helloFrame())

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
    clientRegistry.add(ws)
    if (authTimer) {
      clearTimeout(authTimer)
      authTimer = null
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
    authTimer = setTimeout(() => {
      authTimer = null
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
      // Only the process holding the per-launch secret can be main. Every socket
      // reaching this line is authenticated, so without this a remote client with
      // a valid device token could claim the bridge during main's reconnect window
      // and receive every screenshot, page read and app-install request.
      const claimed = session.kind === 'bootstrap' && browserBridge.setSocket(ws)
      if (!claimed) {
        log.warn('[ws] refused a second bridge:identify while one is live')
      }
      if (id !== undefined && id !== null) {
        ws.send(JSON.stringify(createResponse(id, { ok: claimed })))
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
    }
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
