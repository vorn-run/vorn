import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'
import { RUNTIME_PROTOCOL_VERSION, type ServerHello } from '@vornrun/shared/protocol'

/**
 * Whether a launch starts a server or joins the one already running.
 *
 * The failure this guards is quiet and expensive: two servers on one data
 * directory, the app talking to the empty one while the agents run in the other.
 * The user sees terminals that accept keystrokes and never run them, so every
 * path that could spawn a second server is asserted here.
 */

const hostSettings = {
  value: { mode: 'local', url: '', token: undefined } as Record<string, unknown>
}

/** What the fake data directory currently publishes. */
const published = {
  port: null as number | null,
  token: 'local-secret' as string | null,
  hello: null as ServerHello | null,
  /** Whether the adopted server's process is still alive, when asked. */
  pidAlive: true
}

const spawned: { cmd: string; opts: Record<string, unknown> }[] = []
const bridges: FakeBridge[] = []

class FakeBridge extends EventEmitter {
  requests: string[] = []
  closed = false
  isConnected = false
  constructor(
    public url: string,
    public credential?: string
  ) {
    super()
    bridges.push(this)
  }
  connect(): void {
    setImmediate(() => {
      // The real server sends its greeting as the first frame on the socket and
      // before authentication, which is what lets a launcher judge it without
      // first proving who it is.
      if (published.hello) this.emit('hello', published.hello)
      this.isConnected = true
      this.emit('connected')
    })
  }
  async request(method: string): Promise<unknown> {
    this.requests.push(method)
    return undefined
  }
  close(): void {
    this.closed = true
  }
}

vi.mock('../src/main/server/host-store', () => ({
  readHostSettings: () => hostSettings.value
}))
vi.mock('../src/main/server/server-bridge', () => ({ ServerBridge: FakeBridge }))
vi.mock('../src/main/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('electron', () => ({
  app: {
    getPath: () => '/userData',
    getAppPath: () => '/app',
    getVersion: () => '0.7.0-beta.4',
    isPackaged: true
  }
}))

// The pure judgement stays real; only what the filesystem says is faked, so a
// wiring change cannot pass by disagreeing with the rules tested next door.
vi.mock('../src/main/server/server-adoption', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/main/server/server-adoption')>()
  return {
    ...actual,
    resolveDataDir: () => '/Users/x/.vorn',
    readPortFile: () => (published.port === null ? null : { port: published.port, pid: 999 }),
    readLocalToken: () => published.token,
    isPidAlive: () => published.pidAlive
  }
})

vi.mock('node:child_process', () => ({
  spawn: (cmd: string, _args: string[], opts: Record<string, unknown>) => {
    spawned.push({ cmd, opts })
    const child = new EventEmitter() as EventEmitter & Record<string, unknown>
    const stdout = new EventEmitter() as EventEmitter & Record<string, unknown>
    stdout.destroy = (): void => {}
    // readline reads this; emitting the port line is what the launcher waits on.
    setImmediate(() => stdout.emit('data', JSON.stringify({ port: 51000 }) + '\n'))
    child.stdout = stdout
    child.stderr = null
    child.unref = (): void => {}
    child.killed = false
    child.kill = (): void => {}
    return child
  }
}))

vi.mock('node:readline', () => ({
  createInterface: ({ input }: { input: EventEmitter }) => {
    const rl = new EventEmitter() as EventEmitter & Record<string, unknown>
    rl.close = (): void => {}
    input.on('data', (chunk: string) => {
      for (const line of String(chunk).split('\n')) if (line) rl.emit('line', line)
    })
    return rl
  }
}))

function helloFrom(over: Partial<ServerHello> = {}): ServerHello {
  return {
    protocolVersion: RUNTIME_PROTOCOL_VERSION,
    capabilities: { auth: 1 },
    dataDir: '/Users/x/.vorn',
    buildChannel: 'packaged',
    pid: 999,
    appVersion: '0.7.0-beta.3',
    ...over
  }
}

beforeEach(() => {
  vi.resetModules()
  // Only Electron sets this, and the packaged entry point is resolved from it.
  ;(process as NodeJS.Process & { resourcesPath?: string }).resourcesPath = '/app/Resources'
  spawned.length = 0
  bridges.length = 0
  published.port = null
  published.token = 'local-secret'
  published.hello = null
  published.pidAlive = true
  hostSettings.value = { mode: 'local', url: '', token: undefined }
})

describe('finding a server that is already running', () => {
  it('adopts it instead of spawning a second', async () => {
    published.port = 50091
    published.hello = helloFrom()
    const { launchServer } = await import('../src/main/server/server-launcher')

    const bridge = await launchServer()

    expect(spawned).toEqual([])
    expect((bridge as unknown as FakeBridge).url).toBe('ws://127.0.0.1:50091/ws')
  })

  it('authenticates with the published credential, not a fresh secret', async () => {
    // A detached server outlives the app that generated its bootstrap token, so
    // the next launch must read the credential the server published rather than
    // inventing one the server has never seen.
    published.port = 50091
    published.hello = helloFrom()
    const { launchServer } = await import('../src/main/server/server-launcher')

    const bridge = await launchServer()

    expect((bridge as unknown as FakeBridge).credential).toBe('local-secret')
  })

  it('adopts one running an older app version', async () => {
    published.port = 50091
    published.hello = helloFrom({ appVersion: '0.6.0' })
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    expect(spawned).toEqual([])
  })
})

describe('declining a server that is running', () => {
  it('spawns its own on a protocol mismatch, and never kills the incumbent', async () => {
    // The running server holds the PTYs, so it holds the user's work. A client
    // that cannot speak to it says so and steps aside; it does not resolve the
    // disagreement by ending sessions it cannot even read.
    published.port = 50091
    published.hello = helloFrom({ protocolVersion: RUNTIME_PROTOCOL_VERSION + 1 })
    const { launchServer, getLastAdoptionRefusal } =
      await import('../src/main/server/server-launcher')

    await launchServer()

    expect(spawned).toHaveLength(1)
    expect(getLastAdoptionRefusal()).toMatchObject({ reason: 'protocol-mismatch' })
    expect(bridges[0].requests).not.toContain('server:shutdown')
  })

  it('spawns its own when the running server is the other build', async () => {
    published.port = 50091
    published.hello = helloFrom({ buildChannel: 'dev' })
    const { launchServer, getLastAdoptionRefusal } =
      await import('../src/main/server/server-launcher')

    await launchServer()

    expect(spawned).toHaveLength(1)
    expect(getLastAdoptionRefusal()).toMatchObject({ reason: 'different-build' })
  })

  it('spawns its own when no credential was published', async () => {
    published.port = 50091
    published.token = null
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    expect(spawned).toHaveLength(1)
  })

  it('spawns when nothing is published at all', async () => {
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    expect(spawned).toHaveLength(1)
  })
})

describe('the server it spawns', () => {
  it('is detached, so quitting the app does not end it', async () => {
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    expect(spawned[0].opts.detached).toBe(true)
  })

  it('runs the Electron binary as Node rather than as another app', async () => {
    // process.execPath is the Electron binary. Spawning it without this variable
    // launches a second full Vorn — an infinite spawn loop this code has hit.
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    const env = spawned[0].opts.env as Record<string, string>
    expect(env.ELECTRON_RUN_AS_NODE).toBe('1')
  })

  it('tells the server which build it is, so the next launch can judge it', async () => {
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    const env = spawned[0].opts.env as Record<string, string>
    expect(env.VORN_BUILD_CHANNEL).toBe('packaged')
    expect(env.VORN_APP_VERSION).toBe('0.7.0-beta.4')
  })

  it('runs from a directory that cannot be deleted underneath it', async () => {
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    expect(spawned[0].opts.cwd).toBe('/Users/x/.vorn')
  })
})

describe('quitting', () => {
  it('lets go of an adopted server without stopping it', async () => {
    published.port = 50091
    published.hello = helloFrom()
    const { launchServer, detachFromServer, getServerBridge } =
      await import('../src/main/server/server-launcher')
    await launchServer()

    detachFromServer()

    expect(bridges[0].requests).not.toContain('server:shutdown')
    expect(bridges[0].closed).toBe(true)
    expect(getServerBridge()).toBeNull()
  })

  it('still shuts down a server it spawned when asked to stop', async () => {
    const { launchServer, stopServer } = await import('../src/main/server/server-launcher')
    await launchServer()

    await stopServer()

    expect(bridges.at(-1)?.requests).toContain('server:shutdown')
  })
})

describe('an adopted server that dies', () => {
  it('is replaced, even though there is no child process to notice it', async () => {
    // Crash recovery hangs off `child.on('exit')`, and an adopted server has no
    // child to emit one. Without this the case adoption exists for would be the
    // one case that never recovers: the bridge retrying forever against a port
    // with nothing behind it, which is the symptom #492 was written to end.
    published.port = 50091
    published.hello = helloFrom()
    const { launchServer } = await import('../src/main/server/server-launcher')
    await launchServer()

    published.pidAlive = false
    bridges[0].emit('disconnected')
    await vi.waitFor(() => expect(spawned).toHaveLength(1))
  })

  it('is left alone while its process is still running', async () => {
    // A socket dropping is not proof the server died — it is usually the bridge
    // reconnecting. Spawning a replacement on that signal alone would put a
    // second server on the database every time the connection blipped.
    published.port = 50091
    published.hello = helloFrom()
    const { launchServer } = await import('../src/main/server/server-launcher')
    await launchServer()

    bridges[0].emit('disconnected')
    await new Promise((r) => setTimeout(r, 20))
    expect(spawned).toEqual([])
  })

  it('is not replaced once the app has decided to quit', async () => {
    published.port = 50091
    published.hello = helloFrom()
    const { launchServer, detachFromServer } = await import('../src/main/server/server-launcher')
    await launchServer()

    detachFromServer()
    published.pidAlive = false
    bridges[0].emit('disconnected')
    await new Promise((r) => setTimeout(r, 20))
    expect(spawned).toEqual([])
  })
})
