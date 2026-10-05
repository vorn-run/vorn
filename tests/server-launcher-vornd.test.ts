import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'
import path from 'node:path'
import { RUNTIME_PROTOCOL_VERSION, type ServerIdentity } from '@vornrun/shared/protocol'

/**
 * The launcher putting vornd in front of the server when Settings ›
 * Experimental asks for it.
 *
 * What matters most is the way out: whatever goes wrong with vornd, the app
 * ends up talking to the server directly, and says why.
 */

const published = { port: 50091 as number | null }
const settings = { vornd: false, nativeServer: false }
const daemon = {
  binary: '/app/Resources/vornd/vornd' as string | null,
  failure: null as string | null,
  /** Whether the bridge connects through vornd once pointed at it. */
  reachable: true,
  /** What vornd was told about its session holder. */
  sessiond: null as { binary: string; home: string } | null,
  /** What vornd was told about the Native server switch. */
  nativeServer: null as { db: string } | null,
  holderBinary: '/app/Resources/vornd/vorn-sessiond' as string | null
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
      if (this.url.includes(':47001') && !daemon.reachable) return
      this.isConnected = true
      this.emit('connected')
      this.emit('identity', identity)
    })
  }
  async request(method: string): Promise<unknown> {
    this.requests.push(method)
    if (method === 'config:load')
      return {
        defaults: {
          experimental: { vornd: settings.vornd, nativeServer: settings.nativeServer }
        }
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

interface FakeVornd {
  port: number
  upstream: number
  stopped: boolean
  exit: (detail: string) => void
  onExit(listener: (detail: string) => void): void
  stop(): void
}
const started: FakeVornd[] = []

vi.mock('../src/main/server/vornd', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/main/server/vornd')>()
  return {
    ...actual,
    findVornd: () => daemon.binary,
    startVornd: async (
      _binary: string,
      upstream: number,
      options: {
        sessiond?: { binary: string; home: string }
        nativeServer?: { db: string }
      } = {}
    ) => {
      daemon.sessiond = options.sessiond ?? null
      daemon.nativeServer = options.nativeServer ?? null
      if (daemon.failure) throw new Error(daemon.failure)
      let listener: ((detail: string) => void) | null = null
      const vornd: FakeVornd = {
        port: 47001,
        upstream,
        stopped: false,
        exit: (detail) => listener?.(detail),
        onExit: (l) => void (listener = l),
        stop: () => void (vornd.stopped = true)
      }
      started.push(vornd)
      return vornd
    }
  }
})
vi.mock('../src/main/server/session-holder', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/main/server/session-holder')>()
  return { ...actual, findSessiond: () => daemon.holderBinary }
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
  ;(process as NodeJS.Process & { resourcesPath?: string }).resourcesPath = '/app/Resources'
  bridges.length = 0
  started.length = 0
  published.port = 50091
  settings.vornd = false
  settings.nativeServer = false
  delete process.env.VORN_NATIVE_SERVER
  daemon.nativeServer = null
  daemon.binary = '/app/Resources/vornd/vornd'
  daemon.failure = null
  daemon.reachable = true
  daemon.sessiond = null
  daemon.holderBinary = '/app/Resources/vornd/vorn-sessiond'
})

async function launch() {
  const launcher = await import('../src/main/server/server-launcher')
  const bridge = (await launcher.launchServer()) as unknown as FakeBridge
  return { ...launcher, bridge }
}

describe('vornd in front of the server', () => {
  it('stays out of the way while the switch is off', async () => {
    const { bridge, getVorndStatus } = await launch()
    expect(started).toEqual([])
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({ state: 'off' })
  })

  it('connects through vornd, forwarding to the server, when the switch is on', async () => {
    settings.vornd = true
    const { bridge, getVorndStatus } = await launch()
    expect(started.map((v) => v.upstream)).toEqual([50091])
    expect(bridge.url).toBe('ws://127.0.0.1:47001/ws')
    expect(bridge.isConnected).toBe(true)
    expect(getVorndStatus()).toEqual({ state: 'on', port: 47001, nativeServer: false })
    expect(daemon.nativeServer).toBeNull()
  })

  it('has vornd answer natively, reading the server database, with the native server switch on', async () => {
    settings.vornd = true
    settings.nativeServer = true
    const { getVorndStatus } = await launch()
    expect(daemon.nativeServer).toEqual({ db: path.join('/Users/x/.vorn', 'vorn.db') })
    expect(getVorndStatus()).toEqual({ state: 'on', port: 47001, nativeServer: true })
  })

  it('leaves the native server switch to the daemon switch', async () => {
    settings.nativeServer = true
    const { getVorndStatus } = await launch()
    expect(started).toEqual([])
    expect(getVorndStatus()).toEqual({ state: 'off' })
  })

  it('lets VORN_NATIVE_SERVER decide over the setting, both ways', async () => {
    settings.vornd = true
    process.env.VORN_NATIVE_SERVER = '1'
    await launch()
    expect(daemon.nativeServer).not.toBeNull()
    vi.resetModules()
    settings.nativeServer = true
    process.env.VORN_NATIVE_SERVER = '0'
    await launch()
    expect(daemon.nativeServer).toBeNull()
  })

  it('has vornd keep the session holder in the data directory', async () => {
    settings.vornd = true
    await launch()
    expect(daemon.sessiond).toEqual({
      binary: '/app/Resources/vornd/vorn-sessiond',
      home: '/Users/x/.vorn'
    })
  })

  it('still forwards through vornd when this build has no session holder', async () => {
    settings.vornd = true
    daemon.holderBinary = null
    const { bridge } = await launch()
    expect(daemon.sessiond).toBeNull()
    expect(bridge.url).toBe('ws://127.0.0.1:47001/ws')
  })

  it('reports no session holders, and ends none, while vornd is not in use', async () => {
    const { getSessionHolders, endOlderSessionHolder } = await launch()
    expect(await getSessionHolders()).toBeNull()
    expect(await endOlderSessionHolder('1a2b')).toEqual({
      ok: false,
      detail: 'vornd is not in use'
    })
  })

  it('goes to the server directly when this build has no vornd', async () => {
    settings.vornd = true
    daemon.binary = null
    const { bridge, getVorndStatus } = await launch()
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({ state: 'failed', detail: 'vornd is not in this build' })
  })

  it('goes to the server directly, and says why, when vornd will not start', async () => {
    settings.vornd = true
    daemon.failure = 'vornd exited before it was listening (code=2, signal=null)'
    const { bridge, getVorndStatus } = await launch()
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({ state: 'failed', detail: daemon.failure })
  })

  it('goes back to the server when nothing connects through vornd', async () => {
    settings.vornd = true
    daemon.reachable = false
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    try {
      const launcher = await import('../src/main/server/server-launcher')
      const attempt = launcher.launchServer()
      await vi.advanceTimersByTimeAsync(6_000)
      const bridge = (await attempt) as unknown as FakeBridge
      expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
      expect(started[0]?.stopped).toBe(true)
      expect(launcher.getVorndStatus()).toEqual({
        state: 'failed',
        detail: 'the server could not be reached through vornd'
      })
    } finally {
      vi.useRealTimers()
    }
  })

  it('goes back to the server when vornd exits under it', async () => {
    settings.vornd = true
    const { bridge, getVorndStatus } = await launch()
    started[0]!.exit('code=null, signal=SIGKILL')
    expect(bridge.url).toBe('ws://127.0.0.1:50091/ws')
    expect(getVorndStatus()).toEqual({
      state: 'failed',
      detail: 'vornd exited (code=null, signal=SIGKILL)'
    })
  })

  it('stops vornd when the app lets go of the server', async () => {
    settings.vornd = true
    const { detachFromServer } = await launch()
    detachFromServer()
    expect(started[0]?.stopped).toBe(true)
  })

  it('stops vornd when the app stops the server', async () => {
    settings.vornd = true
    const { stopServer } = await launch()
    await stopServer()
    expect(started[0]?.stopped).toBe(true)
  })
})
