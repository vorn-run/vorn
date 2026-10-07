import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'
import { RUNTIME_PROTOCOL_VERSION, type ServerIdentity } from '@vornrun/shared/protocol'
import type { VorndStatus } from '@vornrun/shared/types'

/**
 * The launcher talking to the server through the vornd the server keeps in
 * front of itself.
 *
 * What matters most is the way out: whatever goes wrong with vornd, the app
 * ends up talking to the server directly, and says why.
 */

const published = { port: 50091 as number | null }
/** What the server answers to `server:vornd`, or the error it fails with. */
const server = {
  vornd: { state: 'on', port: 47001 } as VorndStatus | Error,
  /** Whether the bridge connects once pointed at vornd. */
  reachable: true
}

const bridges: FakeBridge[] = []

class FakeBridge extends EventEmitter {
  requests: string[] = []
  closed = false
  isConnected = false
  serverHelloVersion = RUNTIME_PROTOCOL_VERSION
  constructor(
    public url: string,
    public credential?: string
  ) {
    super()
    bridges.push(this)
  }
  target(): string {
    return this.url
  }
  retarget(url: string): void {
    if (url === this.url) return
    this.url = url
    this.isConnected = false
    this.connect()
  }
  connect(): void {
    setImmediate(() => {
      if (!this.url.includes(':50091') && !server.reachable) return
      this.isConnected = true
      this.emit('connected')
      this.emit('identity', identity)
    })
  }
  /** The connection dropping, as it does when vornd ends. */
  drop(): void {
    this.isConnected = false
    this.emit('disconnected')
  }
  async request(method: string): Promise<unknown> {
    this.requests.push(method)
    if (method === 'server:vornd') {
      if (server.vornd instanceof Error) throw server.vornd
      return server.vornd
    }
    return {}
  }
  close(): void {
    this.closed = true
  }
}

const identity: ServerIdentity = {
  dataDir: '/Users/x/.vorn',
  buildChannel: 'packaged',
  pid: 999,
  appVersion: '0.7.0'
}

const holders = vi.hoisted(() => ({ read: vi.fn() }))
vi.mock('../src/main/server/session-holder', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/main/server/session-holder')>()
  return { ...actual, readSessionHolders: holders.read }
})
vi.mock('../src/main/server/host-store', () => ({
  readHostSettings: () => ({ mode: 'local', url: '', token: undefined })
}))
vi.mock('../src/main/server/server-bridge', () => ({ ServerBridge: FakeBridge }))
vi.mock('../src/main/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('electron', () => ({
  app: {
    getPath: () => '/userData',
    getAppPath: () => '/app',
    getVersion: () => '0.7.0',
    isPackaged: true
  }
}))
vi.mock('../src/main/server/server-adoption', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/main/server/server-adoption')>()
  return {
    ...actual,
    resolveDataDir: () => '/Users/x/.vorn',
    readEndpointPath: () => null,
    readPortFile: () => (published.port === null ? null : { port: published.port, pid: 999 }),
    readLocalToken: () => 'local-secret',
    isPidAlive: () => true
  }
})

beforeEach(() => {
  vi.resetModules()
  bridges.length = 0
  published.port = 50091
  server.vornd = { state: 'on', port: 47001 }
  server.reachable = true
  holders.read.mockReset()
  holders.read.mockResolvedValue({ current: null, older: [], error: null })
})

async function launch() {
  const launcher = await import('../src/main/server/server-launcher')
  const bridge = (await launcher.launchServer()) as unknown as FakeBridge
  return { ...launcher, bridge }
}

const settled = (): Promise<void> => new Promise((resolve) => setImmediate(resolve))

describe('vornd in front of the server', () => {
  it('connects through the vornd the server keeps', async () => {
    const { bridge, getVorndStatus } = await launch()
    expect(bridge.requests).toContain('server:vornd')
    expect(bridge.url).toBe('ws://127.0.0.1:47001/ws')
    expect(bridge.isConnected).toBe(true)
    expect(getVorndStatus()).toEqual({ state: 'on', port: 47001 })
  })

  it('reads the session holders from that vornd', async () => {
    const { getSessionHolders } = await launch()
    expect(await getSessionHolders()).toEqual({ current: null, older: [], error: null })
    expect(holders.read).toHaveBeenCalledWith(47001)
  })

  it('stays on the server, and says why, when its vornd is not running', async () => {
    server.vornd = { state: 'failed', detail: 'vornd is not in this build' }
    const { bridge, getVorndStatus, getSessionHolders, endOlderSessionHolder } = await launch()
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({ state: 'failed', detail: 'vornd is not in this build' })
    expect(await getSessionHolders()).toBeNull()
    expect(await endOlderSessionHolder('1a2b')).toEqual({
      ok: false,
      detail: 'vornd is not in use'
    })
  })

  it('stays on the server when it cannot say where its vornd is', async () => {
    server.vornd = new Error('Method not found: server:vornd')
    const { bridge, getVorndStatus } = await launch()
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({
      state: 'failed',
      detail: 'Method not found: server:vornd'
    })
  })

  it('stays on the server when vornd answers nothing it can use', async () => {
    server.vornd = { state: 'off' }
    const { bridge, getVorndStatus } = await launch()
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({ state: 'failed', detail: 'the server reports no vornd' })
  })

  it('goes back to the server when nothing connects through vornd', async () => {
    server.reachable = false
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    try {
      const launcher = await import('../src/main/server/server-launcher')
      const attempt = launcher.launchServer()
      await vi.advanceTimersByTimeAsync(6_000)
      const bridge = (await attempt) as unknown as FakeBridge
      expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
      expect(launcher.getVorndStatus()).toEqual({
        state: 'failed',
        detail: 'the server could not be reached through vornd'
      })
    } finally {
      vi.useRealTimers()
    }
  })

  it('follows vornd to its new port when it starts again', async () => {
    const { bridge, getVorndStatus } = await launch()
    server.vornd = { state: 'on', port: 47002 }
    bridge.drop()
    // Back to the server, which says where vornd is now.
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({ state: 'failed', detail: 'vornd went away' })
    for (let i = 0; i < 5; i++) await settled()
    expect(bridge.url).toBe('ws://127.0.0.1:47002/ws')
    expect(getVorndStatus()).toEqual({ state: 'on', port: 47002 })
  })

  it('forgets vornd when the app lets go of the server', async () => {
    const { detachFromServer, getVorndStatus } = await launch()
    detachFromServer()
    expect(getVorndStatus()).toEqual({ state: 'off' })
  })
})
