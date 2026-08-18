import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { CLOSE_CREDENTIAL_REJECTED, CLOSE_UNAUTHENTICATED } from '@vornrun/shared/protocol'

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
  onopen: (() => void) | null = null
  onmessage: ((e: { data: string }) => void) | null = null
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
}

let createApiShim: typeof import('../packages/web/src/api-shim').createApiShim

beforeEach(async () => {
  sockets.length = 0
  vi.useFakeTimers()
  vi.stubGlobal('WebSocket', FakeSocket)
  vi.stubGlobal('localStorage', {
    store: new Map<string, string>(),
    getItem(k: string) {
      return this.store.get(k) ?? null
    },
    setItem(k: string, v: string) {
      this.store.set(k, v)
    },
    removeItem(k: string) {
      this.store.delete(k)
    }
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
