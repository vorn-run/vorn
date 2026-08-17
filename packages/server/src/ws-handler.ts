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
  RUNTIME_PROTOCOL_VERSION
} from '@vornrun/shared/protocol'
import { clientRegistry } from './broadcast'
import { browserBridge } from './browser-bridge'
import log from './logger'

// The greeting never varies, so it is serialized once at module load rather
// than rebuilt per connection.
const HELLO_FRAME = JSON.stringify(
  createNotification('server:hello', {
    protocolVersion: RUNTIME_PROTOCOL_VERSION,
    capabilities: {}
  })
)

// Handler registry: method name → async handler function
type Handler = (params: unknown) => Promise<unknown> | unknown
const handlers = new Map<string, Handler>()

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
export function handleConnection(ws: WebSocket): void {
  clientRegistry.add(ws)

  // Announce the contract before anything is dispatched, so a client knows what
  // it is talking to without having to ask. Capabilities are empty until there
  // is something true to put in them.
  ws.send(HELLO_FRAME)

  ws.on('message', async (raw: Buffer) => {
    let msg: RpcRequest
    try {
      msg = JSON.parse(raw.toString())
    } catch {
      log.warn('[ws] received non-JSON message')
      return
    }

    const { id, method, params } = msg

    // A reply to something *we* asked main (browser:* reverse RPC). Responses
    // carry no `method`, so they must be recognised before the notification
    // branch below treats a method-less frame as junk.
    if (method === undefined && id !== undefined && id !== null) {
      if (browserBridge.handleResponse(msg as unknown as RpcResponse)) return
    }

    // Main identifying itself, so the reverse bridge knows which socket to use.
    if (method === 'bridge:identify') {
      browserBridge.setSocket(ws)
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

  ws.on('close', () => {
    clientRegistry.remove(ws)
    browserBridge.clearSocket(ws)
  })

  ws.on('error', (err) => {
    log.error({ err }, '[ws] socket error')
    clientRegistry.remove(ws)
    browserBridge.clearSocket(ws)
  })
}
