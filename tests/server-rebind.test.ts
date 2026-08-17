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

import {
  initRebind,
  checkAndRebind,
  getCurrentHost,
  isAllowedOrigin
} from '../packages/server/src/server-rebind'

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

  emitter.close = vi.fn(function (this: typeof emitter, cb?: (err?: Error) => void) {
    emitter.closeCalls++
    if (cb) cb()
    return this
  }) as unknown as Server['close']

  emitter.listen = vi.fn(function (
    this: typeof emitter,
    port: number,
    host: string,
    cb?: () => void
  ) {
    emitter.listenCalls.push({ port, host })
    if (cb) cb()
    return this
  }) as unknown as Server['listen']

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

  it('rebinds to 0.0.0.0 when networkAccessEnabled and Tailscale running', async () => {
    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsRunning())

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

  it('stays on localhost when Tailscale is not running', async () => {
    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsStopped())

    await checkAndRebind()

    expect(server.closeCalls).toBe(0)
    expect(getCurrentHost()).toBe('127.0.0.1')
  })

  it('stays on localhost when Tailscale check throws', async () => {
    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockRejectedValue(new Error('tailscale not found'))

    await checkAndRebind()

    expect(server.closeCalls).toBe(0)
    expect(getCurrentHost()).toBe('127.0.0.1')
  })

  it('handles listen error gracefully without crashing', async () => {
    const errorServer = makeFakeServer()
    errorServer.listen = vi.fn(function (
      this: typeof errorServer,
      _port: number,
      _host: string,
      _cb?: () => void
    ) {
      // Simulate EADDRINUSE by emitting error instead of calling cb
      process.nextTick(() => errorServer.emit('error', new Error('EADDRINUSE')))
      return errorServer
    }) as unknown as Server['listen']
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
    server.close = vi.fn(function (this: typeof server, cb?: () => void) {
      server.closeCalls++
      slowClose.then(() => cb?.())
      return server
    }) as unknown as Server['close']

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

/**
 * The Origin allowlist lives here rather than beside the socket route because
 * reachability and origin policy are the same decision — this module is what
 * flips the bind at runtime. Computed anywhere else it goes stale the moment
 * someone enables remote access without restarting, and a stale list refuses the
 * tailnet web client with a 403 on the upgrade.
 */
describe('isAllowedOrigin', () => {
  beforeEach(() => {
    initRebind(makeFakeServer(), '127.0.0.1', 4400, [])
  })

  it('allows an absent origin, which is how non-browser clients arrive', () => {
    // The desktop bridge and MCP send none. They are still held to the
    // credential check, which is the control that applies to them.
    expect(isAllowedOrigin(undefined)).toBe(true)
  })

  it('allows the loopback origins the web client is served from', () => {
    expect(isAllowedOrigin('http://127.0.0.1:4400')).toBe(true)
    expect(isAllowedOrigin('http://localhost:4400')).toBe(true)
  })

  it.each([
    ['a foreign site', 'https://evil.example'],
    ['a lookalike prefix', 'http://127.0.0.1:4400.evil.example'],
    ['the right host on another port', 'http://127.0.0.1:9999'],
    ['empty', '']
  ])('refuses %s', (_label, origin) => {
    // Exact comparison only — substring or wildcard matching is the usual way an
    // allowlist gets defeated, which is why the lookalike is asserted.
    expect(isAllowedOrigin(origin)).toBe(false)
  })

  it('starts allowing the tailnet host as soon as the bind changes', async () => {
    expect(isAllowedOrigin('http://100.1.2.3:4400')).toBe(false)

    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsRunning())
    await checkAndRebind()

    // The regression this placement exists to prevent: enabling remote access at
    // runtime used to rebind to 0.0.0.0 while the origin list stayed empty, so
    // the tailnet web client was refused until the app was restarted.
    expect(isAllowedOrigin('http://100.1.2.3:4400')).toBe(true)
  })

  it('stops allowing it again when remote access is turned off', async () => {
    mockLoadConfig.mockReturnValue(makeConfig(true))
    mockGetTailscaleStatus.mockResolvedValue(tsRunning())
    await checkAndRebind()
    expect(isAllowedOrigin('http://100.1.2.3:4400')).toBe(true)

    mockLoadConfig.mockReturnValue(makeConfig(false))
    await checkAndRebind()
    expect(isAllowedOrigin('http://100.1.2.3:4400')).toBe(false)
  })
})
