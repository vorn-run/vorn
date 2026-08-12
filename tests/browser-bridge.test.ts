import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { EventEmitter } from 'events'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

let browserBridge: typeof import('../packages/server/src/browser-bridge').browserBridge

/**
 * The reverse-RPC bridge: the server asking Electron main a question it has no
 * way to answer itself, because the `<webview>` guest and its CDP debugger live
 * only in main.
 *
 * The failure modes worth testing here are the quiet ones. A request that hangs
 * instead of failing, a reply matched to the wrong caller, a promise still
 * waiting after the app has quit — each looks like "the agent is thinking"
 * rather than like a bug.
 */

function mockWs(readyState = 1) {
  const emitter = new EventEmitter()
  return Object.assign(emitter, {
    send: vi.fn(),
    readyState,
    OPEN: 1
  }) as unknown as import('ws').WebSocket & { send: ReturnType<typeof vi.fn> }
}

/** The frame main would have received, decoded. */
function sentFrame(ws: ReturnType<typeof mockWs>, call = 0) {
  return JSON.parse(ws.send.mock.calls[call][0] as string)
}

beforeEach(async () => {
  vi.clearAllMocks()
  vi.resetModules()
  browserBridge = (await import('../packages/server/src/browser-bridge')).browserBridge
})

afterEach(() => {
  vi.useRealTimers()
})

describe('reaching main', () => {
  it('says the app is not running rather than hanging', async () => {
    // No socket has ever identified itself. Failing now beats waiting out the
    // full timeout to say the same thing fifteen seconds later.
    await expect(browserBridge.request('browser:readPage', { sessionId: 's1' })).rejects.toThrow(
      /not running/
    )
  })

  it('refuses a socket that is no longer open', async () => {
    browserBridge.setSocket(mockWs(3 /* CLOSED */))
    expect(browserBridge.isConnected).toBe(false)
    await expect(browserBridge.request('browser:readPage', { sessionId: 's1' })).rejects.toThrow(
      /not running/
    )
  })

  it('sends the call to main and resolves with what comes back', async () => {
    const ws = mockWs()
    browserBridge.setSocket(ws)

    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' })
    const frame = sentFrame(ws)
    expect(frame.method).toBe('browser:readPage')
    expect(frame.params).toEqual({ sessionId: 's1' })

    browserBridge.handleResponse({ jsonrpc: '2.0', id: frame.id, result: { nodes: [] } })
    await expect(inflight).resolves.toEqual({ nodes: [] })
  })

  it('surfaces an error from main as a rejection', async () => {
    const ws = mockWs()
    browserBridge.setSocket(ws)
    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' })

    browserBridge.handleResponse({
      jsonrpc: '2.0',
      id: sentFrame(ws).id,
      error: { code: -1, message: 'No browser pane is open for this session.' }
    })
    // The message has to survive the trip: "no pane open" tells the agent what
    // to do next, where a generic transport failure does not.
    await expect(inflight).rejects.toThrow('No browser pane is open for this session.')
  })
})

describe('matching replies to callers', () => {
  it('keeps ids out of the range main uses for its own requests', async () => {
    const ws = mockWs()
    browserBridge.setSocket(ws)
    void browserBridge.request('browser:readPage', { sessionId: 's1' })

    // Both directions share one id space on this socket. Main counts up, so we
    // count down — without the split, main's request #1 and ours could collide
    // and a reply would resolve the wrong pending call.
    expect(sentFrame(ws).id).toBeLessThan(0)
  })

  it('gives each concurrent call its own id and its own answer', async () => {
    const ws = mockWs()
    browserBridge.setSocket(ws)

    const first = browserBridge.request('browser:readPage', { sessionId: 's1' })
    const second = browserBridge.request('browser:getText', { sessionId: 's2' })
    const [a, b] = [sentFrame(ws, 0), sentFrame(ws, 1)]
    expect(a.id).not.toBe(b.id)

    // Answered out of order on purpose: main is under no obligation to reply in
    // the order it was asked, and a bridge that assumes otherwise hands each
    // caller the other's page.
    browserBridge.handleResponse({ jsonrpc: '2.0', id: b.id, result: { text: 'second' } })
    browserBridge.handleResponse({ jsonrpc: '2.0', id: a.id, result: { text: 'first' } })

    await expect(first).resolves.toEqual({ text: 'first' })
    await expect(second).resolves.toEqual({ text: 'second' })
  })

  it('declines a reply it has no pending call for', () => {
    // Says "not mine" so ws-handler can fall through to its other branches,
    // rather than swallowing a frame that belonged to someone else.
    expect(browserBridge.handleResponse({ jsonrpc: '2.0', id: -999, result: {} })).toBe(false)
  })

  it('resolves a reply whose id arrived as a string', async () => {
    const ws = mockWs()
    browserBridge.setSocket(ws)
    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' })

    // JSON-RPC permits a string id, and a peer that echoes ours back as one
    // would otherwise never match — leaving the caller to time out.
    browserBridge.handleResponse({
      jsonrpc: '2.0',
      id: String(sentFrame(ws).id),
      result: { nodes: [] }
    })
    await expect(inflight).resolves.toEqual({ nodes: [] })
  })
})

describe('when the call does not come back', () => {
  it('times out rather than leaving the caller waiting forever', async () => {
    vi.useFakeTimers()
    const ws = mockWs()
    browserBridge.setSocket(ws)

    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' }, 50)
    // Settled by the timer, so the clock has to move before the assertion can
    // resolve — but the rejection must already be handled by then, or Node
    // sees an unhandled one first.
    const settled = inflight.catch((e: Error) => e.message)
    await vi.advanceTimersByTimeAsync(51)
    await expect(settled).resolves.toMatch(/timed out: browser:readPage/)
  })

  it('does not fire a timeout for a call that already answered', async () => {
    vi.useFakeTimers()
    const ws = mockWs()
    browserBridge.setSocket(ws)

    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' }, 50)
    browserBridge.handleResponse({ jsonrpc: '2.0', id: sentFrame(ws).id, result: { nodes: [] } })
    await expect(inflight).resolves.toEqual({ nodes: [] })

    // An uncleared timer would reject an already-settled promise. That is a
    // no-op for the caller but leaves the entry in `pending` forever.
    await vi.advanceTimersByTimeAsync(100)
  })
})

describe('when main goes away', () => {
  it('fails every in-flight call instead of stranding it', async () => {
    const ws = mockWs()
    browserBridge.setSocket(ws)
    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' })

    browserBridge.clearSocket(ws)

    // The app quit or crashed. These calls can never be answered now, and a
    // promise that stays pending reads to the agent as a tool still working.
    await expect(inflight).rejects.toThrow(/disconnected/)
    expect(browserBridge.isConnected).toBe(false)
  })

  it('ignores a disconnect from a socket that is not the bridge', async () => {
    const bridge = mockWs()
    const other = mockWs()
    browserBridge.setSocket(bridge)
    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' })

    // Renderer clients share this WS server and disconnect all the time. One
    // of them closing must not tear down main's bridge or fail its calls.
    browserBridge.clearSocket(other)
    expect(browserBridge.isConnected).toBe(true)

    browserBridge.handleResponse({ jsonrpc: '2.0', id: sentFrame(bridge).id, result: { ok: true } })
    await expect(inflight).resolves.toEqual({ ok: true })
  })

  it('takes over when main reconnects', async () => {
    const first = mockWs()
    browserBridge.setSocket(first)
    const stranded = browserBridge.request('browser:readPage', { sessionId: 's1' })
    browserBridge.clearSocket(first)
    await expect(stranded).rejects.toThrow(/disconnected/)

    // A restarted app identifies itself again; the bridge must route to the
    // new socket rather than the dead one.
    const second = mockWs()
    browserBridge.setSocket(second)
    const inflight = browserBridge.request('browser:readPage', { sessionId: 's1' })
    expect(second.send).toHaveBeenCalled()

    browserBridge.handleResponse({
      jsonrpc: '2.0',
      id: sentFrame(second).id,
      result: { nodes: [] }
    })
    await expect(inflight).resolves.toEqual({ nodes: [] })
  })
})
