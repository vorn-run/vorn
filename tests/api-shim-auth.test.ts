import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import {
  CLOSE_CREDENTIAL_REJECTED,
  CLOSE_UNAUTHENTICATED,
  RUNTIME_PROTOCOL_VERSION
} from '@vornrun/shared/protocol'
import { encodeTerminalFrame } from '../packages/shared/src/terminal-frame'

/**
 * The web client's half of the auth boundary.
 *
 * A browser cannot set headers on the upgrade, so it presents its credential as
 * the first message and `__ready()` settles on the server's `auth:ok` rather
 * than on the socket opening. The cases worth pinning are the failures: before
 * this, a rejected credential looked exactly like a network problem — the client
 * retried every two seconds forever while the page stayed blank.
 */

const sockets: FakeSocket[] = []

class FakeSocket {
  static OPEN = 1
  binaryType = 'blob'
  onopen: (() => void) | null = null
  onmessage: ((e: { data: string | ArrayBuffer }) => void) | null = null
  onclose: ((e: { code: number }) => void) | null = null
  onerror: (() => void) | null = null
  readyState = 1
  sent: string[] = []

  constructor(public url: string) {
    sockets.push(this)
  }

  send(data: string): void {
    this.sent.push(data)
  }
  close(): void {}

  /** Drive the handshake the way the server does. */
  open(): void {
    this.onopen?.()
  }
  authOk(): void {
    this.onmessage?.({ data: JSON.stringify({ jsonrpc: '2.0', method: 'auth:ok', params: {} }) })
  }
  closeWith(code: number): void {
    this.onclose?.({ code })
  }
  /** What the socket hands over for a binary message once `binaryType` is set. */
  binary(bytes: Uint8Array): void {
    this.onmessage?.({
      data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
    })
  }
}

let createApiShim: typeof import('../packages/web/src/api-shim').createApiShim

beforeEach(async () => {
  sockets.length = 0
  vi.useFakeTimers()
  vi.stubGlobal('WebSocket', FakeSocket)
  // Held in a closure rather than on the object: `this` inside an object literal
  // method has no contextual type, so every access to it was untyped.
  const store = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k)
  })
  vi.resetModules()
  createApiShim = (await import('../packages/web/src/api-shim')).createApiShim
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('the credential handshake', () => {
  it('presents a credential as its first message', () => {
    localStorage.setItem('vorn.deviceToken', 'vorn_abc_def')
    createApiShim('ws://x/ws')
    sockets[0].open()

    const first = JSON.parse(sockets[0].sent[0])
    expect(first.method).toBe('auth:authenticate')
    expect(first.params.token).toBe('vorn_abc_def')
  })

  it('is not ready until the server confirms, not merely when the socket opens', async () => {
    const api = createApiShim('ws://x/ws')
    let ready = false
    void api.__ready().then(() => {
      ready = true
    })

    sockets[0].open()
    await vi.advanceTimersByTimeAsync(0)
    // Opening proves nothing: the server refuses every method until it has the
    // credential, so resolving here would let callers send requests that fail.
    expect(ready).toBe(false)

    sockets[0].authOk()
    await vi.advanceTimersByTimeAsync(0)
    expect(ready).toBe(true)
  })
})

describe('a rejected credential', () => {
  it('asks for a token and stops retrying', async () => {
    const api = createApiShim('ws://x/ws')
    const asked = vi.fn()
    api.__onAuthRequired(asked)

    sockets[0].open()
    sockets[0].closeWith(CLOSE_CREDENTIAL_REJECTED)
    await vi.advanceTimersByTimeAsync(5000)

    expect(asked).toHaveBeenCalled()
    expect(sockets).toHaveLength(1) // no reconnect — retrying cannot help
  })

  it('discards the token that was rejected', async () => {
    localStorage.setItem('vorn.deviceToken', 'stale-token')
    createApiShim('ws://x/ws')
    sockets[0].open()
    sockets[0].closeWith(CLOSE_CREDENTIAL_REJECTED)

    expect(localStorage.getItem('vorn.deviceToken')).toBeNull()
  })

  it('asks again when the rejection arrives on a reconnect, not just the first load', async () => {
    localStorage.setItem('vorn.deviceToken', 'revoked-later')
    const api = createApiShim('ws://x/ws')
    const asked = vi.fn()
    api.__onAuthRequired(asked)

    // Connect and authenticate normally: the readiness promise the bootstrap
    // awaits resolves here and is never replaced in its view.
    sockets[0].open()
    sockets[0].authOk()
    await vi.advanceTimersByTimeAsync(0)

    // The socket drops for an ordinary reason and reconnects.
    sockets[0].closeWith(1006)
    await vi.advanceTimersByTimeAsync(2000)
    expect(sockets).toHaveLength(2)

    // Meanwhile the token was revoked, so the server rejects the reconnect. The
    // rejection lands on a promise nobody is awaiting, which is why this has to
    // reach the caller some other way — otherwise the page just goes quiet.
    sockets[1].open()
    sockets[1].closeWith(CLOSE_CREDENTIAL_REJECTED)
    await vi.advanceTimersByTimeAsync(5000)

    expect(asked).toHaveBeenCalled()
    expect(sockets).toHaveLength(2)
  })
})

describe('a timeout or a dropped socket', () => {
  it('keeps the token and keeps retrying', async () => {
    localStorage.setItem('vorn.deviceToken', 'good-token')
    const api = createApiShim('ws://x/ws')
    const asked = vi.fn()
    api.__onAuthRequired(asked)

    sockets[0].open()
    // A backgrounded phone whose socket stalled past the server's auth window.
    // Conflating this with a rejection would throw away a working token and send
    // the user back to the machine running Vorn.
    sockets[0].closeWith(CLOSE_UNAUTHENTICATED)
    await vi.advanceTimersByTimeAsync(2000)

    expect(asked).not.toHaveBeenCalled()
    expect(localStorage.getItem('vorn.deviceToken')).toBe('good-token')
    expect(sockets).toHaveLength(2)
  })
})

describe('a first connection that fails before authenticating', () => {
  it('still settles the readiness the app is holding', async () => {
    // main.tsx awaits __ready() exactly once and renders when it resolves. The
    // reconnect used to replace that promise, so a retry could connect and
    // authenticate perfectly while the promise the app held stayed pending — the
    // loading screen never lifted. Easy to hit: the page opens a moment before the
    // server is listening and the first attempt loses.
    const api = createApiShim('ws://localhost:1234/ws')
    const settled = vi.fn()
    api.__ready().then(settled)

    sockets[0].open()
    sockets[0].closeWith(CLOSE_UNAUTHENTICATED)
    await vi.advanceTimersByTimeAsync(2100)

    expect(sockets).toHaveLength(2)
    sockets[1].open()
    sockets[1].authOk()
    await Promise.resolve()

    expect(settled).toHaveBeenCalled()
  })

  it('gives a later call a promise tied to the live socket', async () => {
    // The reason a settled promise is still replaced: a call made after a drop must
    // wait for the next connection rather than resolve against the closed one.
    const api = createApiShim('ws://localhost:1234/ws')
    api.__ready()

    sockets[0].open()
    sockets[0].authOk()
    await Promise.resolve()
    sockets[0].closeWith(1006)
    await vi.advanceTimersByTimeAsync(2100)

    const afterDrop = vi.fn()
    api.__ready().then(afterDrop)
    await Promise.resolve()
    expect(afterDrop).not.toHaveBeenCalled()

    sockets[1].open()
    sockets[1].authOk()
    await Promise.resolve()
    expect(afterDrop).toHaveBeenCalled()
  })
})

describe('a bundle and a server that disagree', () => {
  function hello(ws: FakeSocket, protocolVersion: number): void {
    ws.onmessage?.({
      data: JSON.stringify({
        jsonrpc: '2.0',
        method: 'server:hello',
        params: { protocolVersion, capabilities: {} }
      })
    })
  }

  it('reports a server newer than this page', async () => {
    // The stale-cache case: a service worker serving a build from before the
    // server was updated. Left undetected it fails later, in ways that read as
    // the app being broken rather than merely out of date.
    const api = createApiShim('ws://localhost:1234/ws')
    const seen = vi.fn()
    api.__onVersionMismatch(seen)

    sockets[0].open()
    hello(sockets[0], RUNTIME_PROTOCOL_VERSION + 1)

    expect(seen).toHaveBeenCalledWith(RUNTIME_PROTOCOL_VERSION + 1, RUNTIME_PROTOCOL_VERSION)
  })

  it('reports a server older than this page', async () => {
    const api = createApiShim('ws://localhost:1234/ws')
    const seen = vi.fn()
    api.__onVersionMismatch(seen)

    sockets[0].open()
    hello(sockets[0], RUNTIME_PROTOCOL_VERSION - 1)

    expect(seen).toHaveBeenCalledWith(RUNTIME_PROTOCOL_VERSION - 1, RUNTIME_PROTOCOL_VERSION)
  })

  it('says nothing when they agree', async () => {
    const api = createApiShim('ws://localhost:1234/ws')
    const seen = vi.fn()
    api.__onVersionMismatch(seen)

    sockets[0].open()
    hello(sockets[0], RUNTIME_PROTOCOL_VERSION)

    expect(seen).not.toHaveBeenCalled()
  })

  it('does not mistake the handshake for a response', async () => {
    // `server:hello` carries no id and must not settle readiness — that is what
    // `auth:ok` is for.
    const api = createApiShim('ws://localhost:1234/ws')
    const ready = vi.fn()
    api.__ready().then(ready)

    sockets[0].open()
    hello(sockets[0], RUNTIME_PROTOCOL_VERSION)
    await Promise.resolve()

    expect(ready).not.toHaveBeenCalled()
  })
})

describe('the filter a phone asked for', () => {
  const sentMethods = (s: FakeSocket): string[] =>
    s.sent.map((m) => (JSON.parse(m) as { method: string }).method)

  it('is sent again after a reconnect, which opens with only the base topics', async () => {
    localStorage.setItem('vorn.deviceToken', 'ok')
    const api = createApiShim('ws://x/ws')
    sockets[0].open()
    sockets[0].authOk()
    await vi.advanceTimersByTimeAsync(0)

    void api.setTopics(['session:*', 'terminal:data#abc']).catch(() => {})
    expect(sentMethods(sockets[0])).toContain('subscribe:set')

    sockets[0].closeWith(1006)
    await vi.advanceTimersByTimeAsync(2000)
    sockets[1].open()
    sockets[1].authOk()
    await vi.advanceTimersByTimeAsync(0)

    const resent = sockets[1].sent
      .map((m) => JSON.parse(m) as { method: string; params?: { topics?: string[] } })
      .find((m) => m.method === 'subscribe:set')
    expect(resent?.params?.topics).toEqual(['session:*', 'terminal:data#abc'])
  })

  it('is not sent on a reconnect when nothing was ever asked for', async () => {
    createApiShim('ws://x/ws')
    sockets[0].open()
    sockets[0].authOk()
    sockets[0].closeWith(1006)
    await vi.advanceTimersByTimeAsync(2000)
    sockets[1].open()
    sockets[1].authOk()
    expect(sentMethods(sockets[1])).not.toContain('subscribe:set')
  })
})

// Bytes are asked for only of a server whose hello says it sends frame layout 1, and a frame reaches the listeners a JSON notification does.
describe('terminal output as bytes', () => {
  function helloWith(ws: FakeSocket, capabilities: Record<string, number>): void {
    ws.onmessage?.({
      data: JSON.stringify({
        jsonrpc: '2.0',
        method: 'server:hello',
        params: { protocolVersion: RUNTIME_PROTOCOL_VERSION, capabilities }
      })
    })
  }
  const asks = (ws: FakeSocket): unknown[] =>
    ws.sent
      .map((m) => JSON.parse(m) as { method: string; params?: unknown })
      .filter((m) => m.method === 'subscribe:set')
      .map((m) => m.params)

  it('opens the socket for ArrayBuffers', () => {
    createApiShim('ws://x/ws')

    expect(sockets[0].binaryType).toBe('arraybuffer')
  })

  it('asks once let in, and again with every filter', async () => {
    const api = createApiShim('ws://x/ws')
    sockets[0].open()
    helloWith(sockets[0], { auth: 1, subscribe: 1, terminalBytes: 1 })
    sockets[0].authOk()
    void api.setTopics?.(['session:*'])

    expect(asks(sockets[0])).toEqual([
      { terminalBytes: true },
      { topics: ['session:*'], terminalBytes: true }
    ])
  })

  it('asks for nothing of a server that sends text, or a layout it cannot read', async () => {
    const api = createApiShim('ws://x/ws')
    sockets[0].open()
    helloWith(sockets[0], { auth: 1, subscribe: 1, terminalBytes: 2 })
    sockets[0].authOk()
    void api.setTopics?.(['session:*'])

    expect(asks(sockets[0])).toEqual([{ topics: ['session:*'] }])
  })

  it('gives a frame to whoever listens for terminal data', () => {
    const api = createApiShim('ws://x/ws')
    const seen: unknown[] = []
    api.onTerminalData((event) => seen.push(event))
    const data = new TextEncoder().encode('\u001b[32m$\u001b[0m ')

    sockets[0].binary(encodeTerminalFrame({ id: 'term-9', seq: 2, data }))

    expect(seen).toEqual([{ id: 'term-9', seq: 2, data }])
  })

  it('ignores binary that is not a frame', () => {
    const api = createApiShim('ws://x/ws')
    const seen: unknown[] = []
    api.onTerminalData((event) => seen.push(event))

    sockets[0].binary(Uint8Array.from([0, 1, 2]))

    expect(seen).toEqual([])
  })
})
