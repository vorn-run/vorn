import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'
import type { Server } from 'node:http'

// ─── Mocks ───────────────────────────────────────────────────────

const mockLoadConfig = vi.fn()
const mockGetTailscaleStatus = vi.fn()

vi.mock('../packages/server/src/config-manager', () => ({
  configManager: { loadConfig: (...args: unknown[]) => mockLoadConfig(...args) }
}))
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: (...args: unknown[]) => mockGetTailscaleStatus(...args)
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { initRebind, checkAndRebind, getCurrentHost } from '../packages/server/src/server-rebind'

// ─── Helpers ─────────────────────────────────────────────────────

function makeConfig(networkAccessEnabled: boolean) {
  return { defaults: { networkAccessEnabled } }
}

function tsRunning() {
  return { installed: true, running: true, selfIP: '100.1.2.3', peers: [] }
}

function tsStopped() {
  return { installed: true, running: false, selfIP: '', peers: [] }
}

/**
 * Put a stub where a real server method was.
 *
 * `http.Server` types `close` and `listen` more tightly than any fake can
 * satisfy -- overloads, and a return of the server's own type -- so the slot is
 * written through one cast that says so, rather than four stubs each pretending
 * to have the real signature.
 */
function stubMethod(target: object, name: 'close' | 'listen', impl: unknown): void {
  ;(target as unknown as Record<string, unknown>)[name] = impl
}

/** Create a fake http.Server that resolves close/listen synchronously. */
function makeFakeServer(): Server & {
  closeCalls: number
  listenCalls: Array<{ port: number; host: string }>
} {
  const emitter = new EventEmitter() as Server & {
    closeCalls: number
    listenCalls: Array<{ port: number; host: string }>
  }
  emitter.closeCalls = 0
  emitter.listenCalls = []

  stubMethod(
    emitter,
    'close',
    vi.fn(function (this: typeof emitter, cb?: (err?: Error) => void) {
      emitter.closeCalls++
      if (cb) cb()
      return this
    })
  )

  stubMethod(
    emitter,
    'listen',
    vi.fn(function (this: typeof emitter, port: number, host: string, cb?: () => void) {
      emitter.listenCalls.push({ port, host })
      if (cb) cb()
      return this
    })
  )

  emitter.closeAllConnections = vi.fn()

  return emitter
}

// ─── Tests ───────────────────────────────────────────────────────

describe('server-rebind', () => {
  let server: ReturnType<typeof makeFakeServer>

  beforeEach(() => {
    vi.clearAllMocks()
    server = makeFakeServer()
    // Reset module state by re-initializing to 127.0.0.1
    initRebind(server, '127.0.0.1', 59081)
  })

  it('no-op before initRebind is called', async () => {
    // Overwrite with null server
    initRebind(null as unknown as Server, '127.0.0.1', 0)
    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsRunning())

    await checkAndRebind()

    expect(server.closeCalls).toBe(0)
  })

  it('no-op when desired host matches current host', async () => {
    mockLoadConfig.mockReturnValue(makeConfig(false))

    await checkAndRebind()

    // Already on 127.0.0.1, networkAccess off → desired is 127.0.0.1 → no rebind
    expect(server.closeCalls).toBe(0)
    expect(server.listenCalls).toHaveLength(0)
  })

  it('rebinds to 0.0.0.0 when remote access is enabled', async () => {
    mockLoadConfig.mockReturnValue(makeConfig(true))

    await checkAndRebind()

    expect(server.closeAllConnections).toHaveBeenCalled()
    expect(server.closeCalls).toBe(1)
    expect(server.listenCalls).toEqual([{ port: 59081, host: '0.0.0.0' }])
    expect(getCurrentHost()).toBe('0.0.0.0')
  })

  it('rebinds back to 127.0.0.1 when networkAccessEnabled is off', async () => {
    // Start on 0.0.0.0
    initRebind(server, '0.0.0.0', 59081)
    mockLoadConfig.mockReturnValue(makeConfig(false))

    await checkAndRebind()

    expect(server.closeCalls).toBe(1)
    expect(server.listenCalls).toEqual([{ port: 59081, host: '127.0.0.1' }])
    expect(getCurrentHost()).toBe('127.0.0.1')
  })

  it('binds wide with Tailscale stopped, which used to be refused', async () => {
    // The gate was `networkAccessEnabled && tailscaleRunning`, so remote access
    // was impossible without a tailnet. Every connection is authenticated now, so
    // the credential is the boundary and Tailscale is only a convenient address.
    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsStopped())

    await checkAndRebind()

    expect(server.listenCalls).toEqual([{ port: 59081, host: '0.0.0.0' }])
    expect(getCurrentHost()).toBe('0.0.0.0')
  })

  it('does not consult Tailscale at all', async () => {
    // It used to, on a path that serialises every other rebind behind it — so a
    // hung `tailscale status` stalled the flip for its full ten-second timeout.
    mockLoadConfig.mockReturnValue(makeConfig(true))

    await checkAndRebind()

    expect(mockGetTailscaleStatus).not.toHaveBeenCalled()
  })

  it('still binds wide when a Tailscale probe would have thrown', async () => {
    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockRejectedValue(new Error('tailscale not found'))

    await checkAndRebind()

    expect(server.listenCalls).toEqual([{ port: 59081, host: '0.0.0.0' }])
    expect(getCurrentHost()).toBe('0.0.0.0')
  })

  it('handles listen error gracefully without crashing', async () => {
    const errorServer = makeFakeServer()
    stubMethod(
      errorServer,
      'listen',
      vi.fn(function (this: typeof errorServer, _port: number, _host: string, _cb?: () => void) {
        // Simulate EADDRINUSE by emitting error instead of calling cb
        process.nextTick(() => errorServer.emit('error', new Error('EADDRINUSE')))
        return errorServer
      })
    )
    initRebind(errorServer, '127.0.0.1', 59081)

    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsRunning())

    // Should not throw
    await checkAndRebind()

    // Host should remain unchanged since listen failed
    expect(getCurrentHost()).toBe('127.0.0.1')
  })

  it('serializes concurrent calls', async () => {
    let resolveFirst!: () => void
    const slowClose = new Promise<void>((r) => {
      resolveFirst = r
    })

    // Make close() slow so we can test concurrency
    stubMethod(
      server,
      'close',
      vi.fn(function (this: typeof server, cb?: () => void) {
        server.closeCalls++
        slowClose.then(() => cb?.())
        return server
      })
    )

    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsRunning())

    // Fire two concurrent rebinds
    const p1 = checkAndRebind()
    const p2 = checkAndRebind()

    // Unblock the first
    resolveFirst()
    await Promise.all([p1, p2])

    // close should only be called once (second call waited on the first)
    expect(server.closeCalls).toBe(1)
  })

  it('preserves port across rebinds', async () => {
    initRebind(server, '127.0.0.1', 12345)
    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsRunning())

    await checkAndRebind()

    expect(server.listenCalls[0].port).toBe(12345)
  })
})
