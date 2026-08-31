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
  clientRegistry: { add: vi.fn(), remove: vi.fn(), setTopics: vi.fn(), touch: vi.fn() }
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
    expect(clientRegistry.add).toHaveBeenCalledWith(ws, undefined)
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
    expect(response.error?.code).toBe(-32601)
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
    expect(response.error?.code).toBe(-32000)
    expect(response.error?.message).toBe('boom')
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
    expect(sentFrames(ws)[0].params?.capabilities).toEqual({ auth: 1, subscribe: 1 })
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

    await vi.waitFor(() => expect(clientRegistry.add).toHaveBeenCalledWith(ws, undefined))
    expect(replies(ws).some((f) => f.method === 'auth:ok')).toBe(true)
    expect(ws.close).not.toHaveBeenCalled()
  })

  it('admits a socket that authenticates on the upgrade, with no message needed', () => {
    const ws = createMockWs()
    handleConnection(ws, GOOD_TOKEN)

    // MCP opens a fresh connection per RPC call, so a mandatory round-trip here
    // would tax every one of them.
    expect(clientRegistry.add).toHaveBeenCalledWith(ws, undefined)
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

  it('lets a desktop holding a device token claim it, which host mode needs', async () => {
    // This was restricted to the bootstrap credential, on the reasoning that only
    // the process holding the per-launch secret can be main. That stopped being
    // true when the desktop learned to connect to a server on another machine —
    // it authenticates there with a device token — and the restriction silently
    // cost host mode its browser and device panes.
    //
    // Allowing it is not an escalation: a device token already reaches
    // `terminal:create`, so anything the bridge exposes its holder could take
    // with a shell anyway.
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const remote = createMockWs()
    handleConnection(remote, DEVICE_TOKEN)

    sendMessage(remote, { jsonrpc: '2.0', id: 21, method: 'bridge:identify' })

    await vi.waitFor(() => expect(replies(remote).length).toBeGreaterThan(0))
    expect(replies(remote)[0].result).toEqual({ ok: true })
    expect(browserBridge.isBridgeSocket(remote)).toBe(true)
  })

  it('still lets only one hold it at a time, whatever it authenticated with', async () => {
    // The rule that actually protects the bridge, and the one left unchanged.
    const { browserBridge } = await import('../packages/server/src/browser-bridge')
    const desktop = createMockWs()
    const other = createMockWs()
    connectAuthed(desktop)
    handleConnection(other, DEVICE_TOKEN)

    sendMessage(desktop, { jsonrpc: '2.0', method: 'bridge:identify' })
    sendMessage(other, { jsonrpc: '2.0', id: 22, method: 'bridge:identify' })

    await vi.waitFor(() => expect(replies(other).length).toBeGreaterThan(0))
    expect(replies(other)[0].result).toEqual({ ok: false })
    expect(browserBridge.isBridgeSocket(desktop)).toBe(true)
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

describe('a credential offered on the upgrade and rejected', () => {
  it('closes at once rather than waiting out the grace window', () => {
    // Both cases used to land in the timeout branch, so a wrong token was answered
    // ten seconds later with 4001 — the code a client retries on, not the one that
    // means the token itself is bad. MCP opens a connection per RPC call, so a stale
    // token parked a socket for ten seconds on every one of them.
    const ws = createMockWs()

    handleConnection(ws, 'vorn_deadbeef_nope')

    expect(ws.close).toHaveBeenCalledWith(CLOSE_CREDENTIAL_REJECTED, 'credential rejected')
  })

  it('still gives a socket that offered nothing its window', () => {
    const ws = createMockWs()

    handleConnection(ws, undefined)

    expect(ws.close).not.toHaveBeenCalled()
  })
})

describe('revoking a token that a socket is holding', () => {
  it('closes it now rather than at its next reconnect', async () => {
    // The point of revoking a lost phone is that it stops working immediately.
    // clientRegistry keeps a bare set of sockets with no identity, so there was no
    // way to ask which of them held a given token.
    const { disconnectToken, resetTokenTracking } =
      await import('../packages/server/src/ws-handler')
    resetTokenTracking()
    const ws = createMockWs()
    handleConnection(ws, DEVICE_TOKEN)

    const closed = disconnectToken('tok-1')

    expect(closed).toBe(1)
    expect(ws.close).toHaveBeenCalledWith(CLOSE_CREDENTIAL_REJECTED, 'token revoked')
  })

  it('uses the code that makes a browser ask for a new token', async () => {
    // Anything else and the web client simply retries — an endless loop against a
    // door that will not open again.
    const { disconnectToken, resetTokenTracking } =
      await import('../packages/server/src/ws-handler')
    resetTokenTracking()
    const ws = createMockWs()
    handleConnection(ws, DEVICE_TOKEN)

    disconnectToken('tok-1')

    expect((ws.close as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe(CLOSE_CREDENTIAL_REJECTED)
  })

  it('does not track the desktop bootstrap credential, which cannot be revoked', async () => {
    const { disconnectToken, resetTokenTracking } =
      await import('../packages/server/src/ws-handler')
    resetTokenTracking()
    const ws = createMockWs()
    handleConnection(ws, GOOD_TOKEN)

    // It carries no tokenId — there is no device-token row behind it.
    expect(disconnectToken('tok-1')).toBe(0)
    expect(ws.close).not.toHaveBeenCalled()
  })

  it('leaves sockets holding a different token alone', async () => {
    const { disconnectToken, resetTokenTracking } =
      await import('../packages/server/src/ws-handler')
    resetTokenTracking()
    const ws = createMockWs()
    handleConnection(ws, DEVICE_TOKEN)

    expect(disconnectToken('some-other-token-id')).toBe(0)
    expect(ws.close).not.toHaveBeenCalled()
  })
})

describe('sockets that connect and say nothing', () => {
  it('stops accepting once too many are waiting', async () => {
    // Each holds a slot for the whole grace window while proving nothing, so
    // without a ceiling a loop that connects and stays silent — no credential
    // needed — holds every slot open.
    const { resetTokenTracking } = await import('../packages/server/src/ws-handler')
    resetTokenTracking()

    const sockets = Array.from({ length: 64 }, () => createMockWs())
    for (const ws of sockets) handleConnection(ws)
    expect(sockets.every((ws) => !(ws.close as ReturnType<typeof vi.fn>).mock.calls.length)).toBe(
      true
    )

    const overflow = createMockWs()
    handleConnection(overflow)

    expect(overflow.close).toHaveBeenCalledWith(
      CLOSE_UNAUTHENTICATED,
      'too many pending connections'
    )
  })

  it('frees the slot when one authenticates', async () => {
    const { resetTokenTracking } = await import('../packages/server/src/ws-handler')
    resetTokenTracking()

    const sockets = Array.from({ length: 64 }, () => createMockWs())
    for (const ws of sockets) handleConnection(ws)

    // A browser cannot set headers, so it takes a slot and then authenticates in
    // band — which is what has to release it.
    sendMessage(sockets[0], {
      jsonrpc: '2.0',
      method: 'auth:authenticate',
      params: { token: GOOD_TOKEN }
    })
    await vi.waitFor(() => expect(clientRegistry.add).toHaveBeenCalled())

    const next = createMockWs()
    handleConnection(next)

    expect(next.close).not.toHaveBeenCalled()
  })

  it('frees the slot when one disconnects mid-window', async () => {
    // A leak here would close the door on everyone, permanently.
    const { resetTokenTracking } = await import('../packages/server/src/ws-handler')
    resetTokenTracking()

    const sockets = Array.from({ length: 64 }, () => createMockWs())
    for (const ws of sockets) handleConnection(ws)
    ;(sockets[0] as unknown as import('events').EventEmitter).emit('close')

    const next = createMockWs()
    handleConnection(next)

    expect(next.close).not.toHaveBeenCalled()
  })
})

/**
 * A phone wants the session and workflow pushes and none of the PTY output. The
 * socket carries that preference, so it is set here rather than in a handler,
 * which only ever sees its own params.
 */
describe('narrowing what a socket receives', () => {
  it('takes the initial list from the upgrade', () => {
    // On the upgrade, not a later frame: `subscribe:set` can only arrive after the
    // socket is already in the broadcast set, and that gap is enough PTY output to
    // matter on a phone — on every reconnect.
    const ws = createMockWs()
    handleConnection(ws, GOOD_TOKEN, ['session:*'])

    expect(clientRegistry.add).toHaveBeenCalledWith(ws, ['session:*'])
  })

  it('changes the list on request', () => {
    const ws = createMockWs()
    connectAuthed(ws)

    sendMessage(ws, { jsonrpc: '2.0', method: 'subscribe:set', params: { topics: ['session:*'] } })

    expect(clientRegistry.setTopics).toHaveBeenCalledWith(ws, ['session:*'])
  })

  it('acknowledges when asked with an id', async () => {
    const ws = createMockWs()
    connectAuthed(ws)

    sendMessage(ws, { jsonrpc: '2.0', id: 7, method: 'subscribe:set', params: { topics: [] } })

    await vi.waitFor(() => expect(replies(ws).length).toBeGreaterThan(0))
    expect(replies(ws)[0]).toMatchObject({ id: 7, result: { ok: true } })
  })

  it('does not answer -32601 for a subscribe it handled', async () => {
    // It is dispatched before the handler map, so it must not fall through to the
    // unknown-method branch.
    const ws = createMockWs()
    connectAuthed(ws)

    sendMessage(ws, { jsonrpc: '2.0', id: 8, method: 'subscribe:set', params: { topics: ['a'] } })

    await vi.waitFor(() => expect(replies(ws).length).toBeGreaterThan(0))
    expect(replies(ws)[0].error).toBeUndefined()
  })

  it('refuses one from a socket that has not authenticated', () => {
    // Everything past the credential check requires a session; a filter is not an
    // exception just because it only narrows.
    const ws = createMockWs()
    handleConnection(ws)

    sendMessage(ws, { jsonrpc: '2.0', method: 'subscribe:set', params: { topics: ['session:*'] } })

    expect(clientRegistry.setTopics).not.toHaveBeenCalled()
  })
})

describe('what counts as somebody being out there', () => {
  it('counts an ordinary call', () => {
    const ws = createMockWs()
    connectAuthed(ws)
    ;(clientRegistry.touch as ReturnType<typeof vi.fn>).mockClear()
    sendMessage(ws, { jsonrpc: '2.0', id: 1, method: 'config:load' })
    expect(clientRegistry.touch).toHaveBeenCalled()
  })

  it('does not count a frame from a socket that has not proved itself', () => {
    // Anything on this machine can open a socket to loopback. If an
    // unauthenticated frame counted, arbitrary local traffic could pin a server
    // nobody is using without ever proving it is a client -- the same hole the
    // hook endpoint closes by advancing its clock only past authentication.
    const ws = createMockWs()
    ;(clientRegistry.touch as ReturnType<typeof vi.fn>).mockClear()
    handleConnection(ws as never, { socket: { remoteAddress: '127.0.0.1' } } as never)
    sendMessage(ws, { jsonrpc: '2.0', id: 1, method: 'config:load' })
    expect(clientRegistry.touch).not.toHaveBeenCalled()
  })

  it('does not count the frame every connection opens with', () => {
    // `ServerBridge` sends `bridge:identify` from its own `open` handler, so it
    // arrives on every socket -- including the one another Vorn opens purely to
    // ask this server whether it may adopt it, and the one behind
    // `probeSessions`. Counting it would let a user blocked by a leftover server
    // reset that server's idle clock on every launch attempt, so the leftover
    // never leaves and the launches never stop being blocked.
    const ws = createMockWs()
    connectAuthed(ws)
    ;(clientRegistry.touch as ReturnType<typeof vi.fn>).mockClear()
    sendMessage(ws, { jsonrpc: '2.0', id: 1, method: 'bridge:identify' })
    expect(clientRegistry.touch).not.toHaveBeenCalled()
  })
})
