import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'
import { RUNTIME_PROTOCOL_VERSION, type ServerIdentity } from '@vornrun/shared/protocol'

/** The launcher and vornd, which is the server: what Settings is told of it. */

const published = { port: 50091 as number | null }

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
      this.isConnected = true
      this.emit('connected')
      this.emit('identity', identity)
    })
  }
  async request(method: string): Promise<unknown> {
    this.requests.push(method)
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
  holders.read.mockReset()
  holders.read.mockResolvedValue({ current: null, older: [], error: null })
})

async function launch() {
  const launcher = await import('../src/main/server/server-launcher')
  const bridge = (await launcher.launchServer()) as unknown as FakeBridge
  return { ...launcher, bridge }
}

describe('vornd as the server', () => {
  it('talks to vornd where it published its port, asking nothing more', async () => {
    const { bridge, getVorndStatus } = await launch()
    expect(bridge.requests).not.toContain('server:vornd')
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({ state: 'on', port: 50091 })
  })

  it('reads the session holders from it', async () => {
    const { getSessionHolders } = await launch()
    expect(await getSessionHolders()).toEqual({ current: null, older: [], error: null })
    expect(holders.read).toHaveBeenCalledWith(50091)
  })

  it('reads no holders once the app lets go of the server', async () => {
    const { detachFromServer, getVorndStatus, getSessionHolders, endOlderSessionHolder } =
      await launch()
    detachFromServer()
    expect(getVorndStatus()).toEqual({ state: 'off' })
    expect(await getSessionHolders()).toBeNull()
    expect(await endOlderSessionHolder('1a2b')).toEqual({
      ok: false,
      detail: 'vornd is not in use'
    })
  })
})
