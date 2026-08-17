import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'events'
import { RUNTIME_PROTOCOL_VERSION } from '@vornrun/shared/protocol'

/** The desktop's per-launch secret: the only credential that may claim the bridge. */
const GOOD_TOKEN = 'valid-credential'
/** A remote client's device token — authenticated, but not main. */
const DEVICE_TOKEN = 'device-credential'
const CLOSE_UNAUTHENTICATED = 4001
const CLOSE_CREDENTIAL_REJECTED = 4002

vi.mock('../packages/server/src/broadcast', () => ({
  clientRegistry: { add: vi.fn(), remove: vi.fn() }
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
// Credential *verification* is covered by tests/ws-auth.test.ts. Stubbed here so
// this file stays about what the socket does before and after it is authenticated.
vi.mock('../packages/server/src/ws-auth', () => ({
  authenticateCredential: (raw?: string) =>
    raw === GOOD_TOKEN
      ? { userId: 'owner-1', kind: 'bootstrap' }
      : raw === DEVICE_TOKEN
        ? { userId: 'owner-1', kind: 'device', tokenId: 'tok-1' }
        : null,
  AUTH_TIMEOUT_MS: 10_000
}))

let registerMethod: typeof import('../packages/server/src/ws-handler').registerMethod
let registerNotification: typeof import('../packages/server/src/ws-handler').registerNotification
let handleConnection: typeof import('../packages/server/src/ws-handler').handleConnection
let clientRegistry: typeof import('../packages/server/src/broadcast').clientRegistry

function createMockWs() {
  const emitter = new EventEmitter()
  const ws = Object.assign(emitter, {
    send: vi.fn(),
    close: vi.fn(),
    readyState: 1,
    OPEN: 1
  })
  return ws as unknown as import('ws').WebSocket
}

/** Connect and authenticate, which is what every test below the handshake wants. */
function connectAuthed(ws: ReturnType<typeof createMockWs>): void {
  handleConnection(ws, GOOD_TOKEN)
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
    connectAuthed(ws)
    expect(clientRegistry.add).toHaveBeenCalledWith(ws)
  })

  it('removes client on close', () => {
    const ws = createMockWs()
    connectAuthed(ws)
    ;(ws as unknown as EventEmitter).emit('close')
    expect(clientRegistry.remove).toHaveBeenCalledWith(ws)
  })

  it('returns -32601 for unknown method', async () => {
    const ws = createMockWs()
    connectAuthed(ws)
    sendMessage(ws, { jsonrpc: '2.0', id: 1, method: 'unknown:method' })

    await vi.waitFor(() => expect(replies(ws).length).toBeGreaterThan(0))
    const response = replies(ws)[0]
    expect(response.error.code).toBe(-32601)
  })

  it('dispatches to registered method handler', async () => {
    registerMethod('config:load' as never, (() => ({ ok: true })) as never)

    const ws = createMockWs()
    connectAuthed(ws)
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
    connectAuthed(ws)
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
    connectAuthed(ws)
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
    connectAuthed(ws)

    const first = sentFrames(ws)[0]
    expect(first.method).toBe('server:hello')
    expect(first.params?.protocolVersion).toBe(RUNTIME_PROTOCOL_VERSION)
  })

  it('advertises the capabilities it actually implements', () => {
    const ws = createMockWs()
    connectAuthed(ws)

    // Declared by the code that enforces it, so the advertisement cannot drift
    // from the behaviour. Pass A shipped this empty because nothing was true yet.
    expect(sentFrames(ws)[0].params?.capabilities).toEqual({ auth: 1 })
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
    connectAuthed(ws)

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
    connectAuthed(ws)
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
    connectAuthed(ws)
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
    connectAuthed(ws)
    sendMessage(ws, { jsonrpc: '2.0', method: 'bridge:identify' })
    ;(ws as unknown as EventEmitter).emit('error', new Error('reset'))

    expect(browserBridge.isConnected).toBe(false)
    expect(clientRegistry.remove).toHaveBeenCalledWith(ws)
  })
})

/**
 * The socket boundary.
 *
 * Before this, any client that reached the port was fully trusted — and browsers
 * let an arbitrary web page open a socket to a server bound on loopback, because
 * WebSocket upgrades are subject to neither CORS nor same-origin policy. So this
 * is not a hardening of remote access; it is what stops a visited website from
 * spawning a terminal.
 */
describe('authentication', () => {
  it('keeps an unauthenticated socket out of the broadcast registry', () => {
    const ws = createMockWs()
    handleConnection(ws)

    // clientRegistry.broadcast fans terminal output, config and session events to
    // every member, so joining before proving anything would hand over the work.
    expect(clientRegistry.add).not.toHaveBeenCalled()
  })

  it('refuses a method sent before authenticating, and closes', async () => {
    registerMethod('config:load' as never, (() => ({ secret: true })) as never)

    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', id: 1, method: 'config:load' })

    await vi.waitFor(() => expect(replies(ws).length).toBeGreaterThan(0))
    expect(replies(ws)[0].error?.code).toBe(-32001)
    expect(ws.close).toHaveBeenCalledWith(CLOSE_UNAUTHENTICATED, expect.any(String))
  })

  it('refuses a notification sent before authenticating', async () => {
    const handler = vi.fn()
    registerNotification('terminal:write', handler)

    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', method: 'terminal:write', params: { data: 'rm -rf' } })

    await vi.waitFor(() => expect(ws.close).toHaveBeenCalled())
    expect(handler).not.toHaveBeenCalled()
  })

  it('admits a socket that authenticates by message', async () => {
    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', method: 'auth:authenticate', params: { token: GOOD_TOKEN } })

    await vi.waitFor(() => expect(clientRegistry.add).toHaveBeenCalledWith(ws))
    expect(replies(ws).some((f) => f.method === 'auth:ok')).toBe(true)
    expect(ws.close).not.toHaveBeenCalled()
  })

  it('admits a socket that authenticates on the upgrade, with no message needed', () => {
    const ws = createMockWs()
    handleConnection(ws, GOOD_TOKEN)

    // MCP opens a fresh connection per RPC call, so a mandatory round-trip here
    // would tax every one of them.
    expect(clientRegistry.add).toHaveBeenCalledWith(ws)
    expect(replies(ws)).toHaveLength(0)
  })

  it('refuses a bad credential rather than leaving the socket open', async () => {
    const ws = createMockWs()
    handleConnection(ws)
    sendMessage(ws, { jsonrpc: '2.0', id: 4, method: 'auth:authenticate', params: { token: 'no' } })

    await vi.waitFor(() => expect(ws.close).toHaveBeenCalled())
    expect(replies(ws)[0].error?.code).toBe(-32001)
    expect(clientRegistry.add).not.toHaveBeenCalled()
    // Distinct from the timeout code, so a client knows to discard the token it
    // presented rather than keep retrying with it.
    expect(ws.close).toHaveBeenCalledWith(CLOSE_CREDENTIAL_REJECTED, expect.any(String))
  })

  it('closes a socket that never authenticates', async () => {
    vi.useFakeTimers()
    try {
      const ws = createMockWs()
      handleConnection(ws)
      expect(ws.close).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(10_000)
      expect(ws.close).toHaveBeenCalledWith(CLOSE_UNAUTHENTICATED, expect.any(String))
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('bridge authorization', () => {
  it('refuses bridge:identify from an unauthenticated socket', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const ws = createMockWs()
    handleConnection(ws)

    sendMessage(ws, { jsonrpc: '2.0', method: 'bridge:identify' })

    await vi.waitFor(() => expect(ws.close).toHaveBeenCalled())
    expect(browserBridge.isBridgeSocket(ws)).toBe(false)
  })

  it('refuses the bridge to an authenticated client that is not the desktop', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const remote = createMockWs()
    handleConnection(remote, DEVICE_TOKEN)

    sendMessage(remote, { jsonrpc: '2.0', id: 21, method: 'bridge:identify' })

    await vi.waitFor(() => expect(replies(remote).length).toBeGreaterThan(0))
    // Every socket here is authenticated, so liveness alone would let a remote
    // device token take the bridge during main's reconnect window.
    expect(replies(remote)[0].result).toEqual({ ok: false })
    expect(browserBridge.isBridgeSocket(remote)).toBe(false)
  })

  it('refuses a second identify while a live socket holds the bridge', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const main = createMockWs()
    const thief = createMockWs()
    connectAuthed(main)
    connectAuthed(thief)

    sendMessage(main, { jsonrpc: '2.0', method: 'bridge:identify' })
    sendMessage(thief, { jsonrpc: '2.0', id: 9, method: 'bridge:identify' })

    await vi.waitFor(() => expect(replies(thief).length).toBeGreaterThan(0))
    // Taking the bridge means receiving every page read, screenshot and
    // app-install request meant for main — and answering them.
    expect(replies(thief)[0].result).toEqual({ ok: false })
    expect(browserBridge.isBridgeSocket(main)).toBe(true)
  })

  it('replaces a dead bridge socket, so main is not locked out after a reconnect', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const stale = createMockWs()
    connectAuthed(stale)
    sendMessage(stale, { jsonrpc: '2.0', method: 'bridge:identify' })
    await vi.waitFor(() => expect(browserBridge.isBridgeSocket(stale)).toBe(true))
    ;(stale as unknown as { readyState: number }).readyState = 3 // CLOSED

    const fresh = createMockWs()
    connectAuthed(fresh)
    sendMessage(fresh, { jsonrpc: '2.0', id: 11, method: 'bridge:identify' })

    await vi.waitFor(() => expect(replies(fresh).length).toBeGreaterThan(0))
    expect(replies(fresh)[0].result).toEqual({ ok: true })
    expect(browserBridge.isBridgeSocket(fresh)).toBe(true)
  })

  it('ignores a bridge response from a socket that does not hold the bridge', async () => {
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const main = createMockWs()
    const thief = createMockWs()
    connectAuthed(main)
    connectAuthed(thief)
    sendMessage(main, { jsonrpc: '2.0', method: 'bridge:identify' })

    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' })
    const asked = replies(main).at(-1)!

    // Bridge ids are negative and sequential, so they are guessable. Answering
    // one from another socket would let it forge a page read's result.
    sendMessage(thief, { jsonrpc: '2.0', id: asked.id, result: { nodes: ['forged'] } })
    sendMessage(main, { jsonrpc: '2.0', id: asked.id, result: { nodes: ['real'] } })

    await expect(inflight).resolves.toEqual({ nodes: ['real'] })
  })
})
