import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'events'
import { RUNTIME_PROTOCOL_VERSION } from '@vornrun/shared/protocol'

vi.mock('../packages/server/src/broadcast', () => ({
  clientRegistry: { add: vi.fn(), remove: vi.fn() }
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

let registerMethod: typeof import('../packages/server/src/ws-handler').registerMethod
let registerNotification: typeof import('../packages/server/src/ws-handler').registerNotification
let handleConnection: typeof import('../packages/server/src/ws-handler').handleConnection
let clientRegistry: typeof import('../packages/server/src/broadcast').clientRegistry

function createMockWs() {
  const emitter = new EventEmitter()
  const ws = Object.assign(emitter, {
    send: vi.fn(),
    readyState: 1,
    OPEN: 1
  })
  return ws as unknown as import('ws').WebSocket
}

function sendMessage(ws: ReturnType<typeof createMockWs>, msg: object) {
  ;(ws as unknown as EventEmitter).emit('message', Buffer.from(JSON.stringify(msg)))
}

type Frame = {
  method?: string
  id?: number | string
  result?: unknown
  error?: { code: number; message: string }
  params?: { protocolVersion?: number; capabilities?: Record<string, number> }
}

function sentFrames(ws: import('ws').WebSocket): Frame[] {
  return (ws.send as ReturnType<typeof vi.fn>).mock.calls.map((c) => JSON.parse(c[0]) as Frame)
}

/**
 * What the handler sent in reply to something — everything except the
 * `server:hello` it opens every connection with. Tests assert on replies rather
 * than on `calls[0]` so that adding another unprompted frame later does not
 * silently shift every index in this file.
 */
function replies(ws: import('ws').WebSocket): Frame[] {
  return sentFrames(ws).filter((f) => f.method !== 'server:hello')
}

beforeEach(async () => {
  vi.clearAllMocks()
  vi.resetModules()
  const wsHandler = await import('../packages/server/src/ws-handler')
  const broadcast = await import('../packages/server/src/broadcast')
  registerMethod = wsHandler.registerMethod
  registerNotification = wsHandler.registerNotification
  handleConnection = wsHandler.handleConnection
  clientRegistry = broadcast.clientRegistry
})

describe('handleConnection', () => {
  it('adds client to registry on connect', () => {
    const ws = createMockWs()
    handleConnection(ws)
    expect(clientRegistry.add).toHaveBeenCalledWith(ws)
  })

  it('removes client on close', () => {
    const ws = createMockWs()
    handleConnection(ws)
    ;(ws as unknown as EventEmitter).emit('close')
    expect(clientRegistry.remove).toHaveBeenCalledWith(ws)
  })

  it('returns -32601 for unknown method', async () => {
    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', id: 1, method: 'unknown:method' })

    await vi.waitFor(() => expect(replies(ws).length).toBeGreaterThan(0))
    const response = replies(ws)[0]
    expect(response.error.code).toBe(-32601)
  })

  it('dispatches to registered method handler', async () => {
    registerMethod('config:load' as never, (() => ({ ok: true })) as never)

    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', id: 2, method: 'config:load' })

    await vi.waitFor(() => expect(replies(ws).length).toBeGreaterThan(0))
    const response = replies(ws)[0]
    expect(response.id).toBe(2)
    expect(response.result).toEqual({ ok: true })
  })

  it('dispatches fire-and-forget notification (no id)', async () => {
    const handler = vi.fn()
    registerNotification('terminal:write', handler)

    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', method: 'terminal:write', params: { data: 'hi' } })

    await vi.waitFor(() => expect(handler).toHaveBeenCalled())
    expect(handler).toHaveBeenCalledWith({ data: 'hi' })
    // No response should be sent for notifications. The handshake frame is
    // not a response, so it is excluded rather than counted here.
    expect(replies(ws)).toHaveLength(0)
  })

  it('returns -32000 when handler throws', async () => {
    registerMethod(
      'test:error' as never,
      (() => {
        throw new Error('boom')
      }) as never
    )

    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', id: 3, method: 'test:error' })

    await vi.waitFor(() => expect(replies(ws).length).toBeGreaterThan(0))
    const response = replies(ws)[0]
    expect(response.error.code).toBe(-32000)
    expect(response.error.message).toBe('boom')
  })
})

/**
 * The connect-time handshake.
 *
 * Clients update on their own schedule once anything but this desktop connects,
 * so a client has to be able to learn what it is talking to without asking. The
 * cost of shipping that late is that every client predating it cannot negotiate.
 */
describe('handshake', () => {
  it('announces the protocol before anything else is sent', () => {
    const ws = createMockWs()
    handleConnection(ws)

    const first = sentFrames(ws)[0]
    expect(first.method).toBe('server:hello')
    expect(first.params?.protocolVersion).toBe(RUNTIME_PROTOCOL_VERSION)
  })

  it('advertises no capabilities while none are true', () => {
    const ws = createMockWs()
    handleConnection(ws)

    // Authentication is not enforced yet, so it must not be advertised: a later
    // client would read the claim and gate on a boundary that is not there.
    expect(sentFrames(ws)[0].params?.capabilities).toEqual({})
  })

  it('greets every connection, not just the first', () => {
    const a = createMockWs()
    const b = createMockWs()
    handleConnection(a)
    handleConnection(b)

    expect(sentFrames(a)[0].method).toBe('server:hello')
    expect(sentFrames(b)[0].method).toBe('server:hello')
  })
})

/**
 * The reverse-RPC bridge's half of the socket handler.
 *
 * `browser:*` calls are answered by Electron main, not here, so this file has
 * to recognise two extra kinds of frame: main introducing itself, and main's
 * reply to something the server asked it.
 */
describe('bridge frames', () => {
  it('registers the socket main identifies itself on, and acknowledges', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const ws = createMockWs()
    handleConnection(ws)

    sendMessage(ws, { jsonrpc: '2.0', id: 7, method: 'bridge:identify' })

    await vi.waitFor(() => expect(replies(ws).length).toBeGreaterThan(0))
    // Acknowledged, so main knows the bridge is live rather than assuming it.
    const response = replies(ws)[0]
    expect(response.id).toBe(7)
    expect(response.result).toEqual({ ok: true })
    expect(browserBridge.isConnected).toBe(true)
  })

  it('routes a reply from main to the bridge instead of treating it as junk', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', method: 'bridge:identify' })

    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' })
    const asked = replies(ws).at(-1)!

    // A response carries no `method`. Without the bridge branch running first,
    // the notification branch below it reads a method-less frame as garbage and
    // the caller waits out its whole timeout for a reply already in hand.
    sendMessage(ws, { jsonrpc: '2.0', id: asked.id, result: { nodes: [] } })
    await expect(inflight).resolves.toEqual({ nodes: [] })
  })

  it('drops the bridge when main’s socket closes', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', method: 'bridge:identify' })
    expect(browserBridge.isConnected).toBe(true)
    ;(ws as unknown as EventEmitter).emit('close')

    // Left registered, every later browser call would be sent into a dead
    // socket and time out rather than saying the app is not running.
    expect(browserBridge.isConnected).toBe(false)
  })

  it('drops the bridge when main’s socket errors', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', method: 'bridge:identify' })
    ;(ws as unknown as EventEmitter).emit('error', new Error('reset'))

    expect(browserBridge.isConnected).toBe(false)
    expect(clientRegistry.remove).toHaveBeenCalledWith(ws)
  })
})
