import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'
import { RUNTIME_PROTOCOL_VERSION, type ServerIdentity } from '@vornrun/shared/protocol'

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
  identity: null as ServerIdentity | null,
  protocolVersion: RUNTIME_PROTOCOL_VERSION as number,
  /** Whether the adopted server's process is still alive, when asked. */
  pidAlive: true,
  /** Whether the running server closes the socket after greeting us. */
  rejectsCredential: false,
  /** Set once the fake spawned server has "published" its port file. */
  spawnedPid: null as number | null
}

const spawned: { cmd: string; opts: Record<string, unknown> }[] = []
/** When set, the spawned child never announces a port — a server that will not start. */
const quietSpawn = { value: false }
const spawnedChildren: EventEmitter[] = []
const bridges: FakeBridge[] = []

class FakeBridge extends EventEmitter {
  requests: string[] = []
  closed = false
  isConnected = false
  helloVersion: number | undefined
  get serverHelloVersion(): number | undefined {
    return this.helloVersion
  }
  constructor(
    public url: string,
    public credential?: string
  ) {
    super()
    bridges.push(this)
  }
  connect(): void {
    setImmediate(() => {
      // Mirrors the real ordering: the socket opens and `connected` fires first,
      // then frames arrive. So `isConnected` is already true when the greeting
      // lands, and cannot be used as proof the credential was accepted.
      this.isConnected = true
      this.emit('connected')
      this.helloVersion = published.protocolVersion
      if (published.identity) this.emit('identity', published.identity)
    })
  }
  async request(method: string): Promise<unknown> {
    this.requests.push(method)
    // `config:load` is what the launcher uses to prove the credential was
    // accepted; a server that refuses this app answers it with a rejection.
    if (method === 'config:load' && published.rejectsCredential) {
      throw new Error('not authenticated')
    }
    return {}
  }
  close(): void {
    this.closed = true
    this.isConnected = false
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
    readPortFile: () =>
      published.port === null ? null : { port: published.port, pid: published.spawnedPid ?? 999 },
    readLocalToken: () => published.token,
    isPidAlive: () => published.pidAlive
  }
})

const madeDirs: string[] = []
const opened: string[] = []
vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>()
  return {
    ...actual,
    mkdirSync: (dir: string) => void madeDirs.push(dir),
    openSync: (file: string) => {
      opened.push(String(file))
      return 99
    },
    closeSync: () => {}
  }
})

vi.mock('node:child_process', () => ({
  spawn: (cmd: string, _args: string[], opts: Record<string, unknown>) => {
    spawned.push({ cmd, opts })
    const child = new EventEmitter() as EventEmitter & Record<string, unknown>
    child.pid = 4242
    const stdout = new EventEmitter() as EventEmitter & Record<string, unknown>
    stdout.destroy = (): void => {}
    stdout.unref = (): void => {}
    // Only the dev path reads this; the packaged path waits on the port file.
    if (!quietSpawn.value) {
      setImmediate(() => stdout.emit('data', JSON.stringify({ port: 51000 }) + '\n'))
    }
    child.stdout = stdout
    child.stderr = null
    child.unref = (): void => {}
    child.killed = false
    child.kill = (): void => {}
    spawnedChildren.push(child)
    // What the server does once it is listening: publish {port, pid}.
    if (!quietSpawn.value) {
      setImmediate(() => {
        published.port = 51000
        published.spawnedPid = 4242
      })
    }
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

function identityFrom(over: Partial<ServerIdentity> = {}): ServerIdentity {
  return {
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
  spawnedChildren.length = 0
  quietSpawn.value = false
  madeDirs.length = 0
  opened.length = 0
  bridges.length = 0
  published.port = null
  published.token = 'local-secret'
  published.identity = null
  published.protocolVersion = RUNTIME_PROTOCOL_VERSION
  published.pidAlive = true
  published.rejectsCredential = false
  published.spawnedPid = null
  hostSettings.value = { mode: 'local', url: '', token: undefined }
})

describe('finding a server that is already running', () => {
  it('adopts it instead of spawning a second', async () => {
    published.port = 50091
    published.identity = identityFrom()
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
    published.identity = identityFrom()
    const { launchServer } = await import('../src/main/server/server-launcher')

    const bridge = await launchServer()

    expect((bridge as unknown as FakeBridge).credential).toBe('local-secret')
  })

  it('adopts one running an older app version', async () => {
    published.port = 50091
    published.identity = identityFrom({ appVersion: '0.6.0' })
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    expect(spawned).toEqual([])
  })
})

describe('declining a server that is running', () => {
  /**
   * Every case here ends the same way: no second server.
   *
   * Refusing to adopt is not grounds to start a rival. Both would open the same
   * SQLite file, and `saveSessions` is a DELETE-then-insert of the whole table on
   * a debounce, so the two would erase each other's sessions with the last writer
   * winning. tmux does not kill a server it cannot speak to and does not start one
   * beside it either -- the client prints the mismatch and exits.
   */
  it('refuses on a protocol mismatch, and never kills the incumbent', async () => {
    published.port = 50091
    published.identity = identityFrom()
    published.protocolVersion = RUNTIME_PROTOCOL_VERSION + 1
    const { launchServer, getLastAdoptionRefusal, AdoptionRefusedError } =
      await import('../src/main/server/server-launcher')

    await expect(launchServer()).rejects.toBeInstanceOf(AdoptionRefusedError)

    expect(spawned).toEqual([])
    expect(getLastAdoptionRefusal()).toMatchObject({ reason: 'protocol-mismatch' })
    expect(bridges[0].requests).not.toContain('server:shutdown')
  })

  it('refuses when the running server is the other build', async () => {
    published.port = 50091
    published.identity = identityFrom({ buildChannel: 'dev' })
    const { launchServer, getLastAdoptionRefusal } =
      await import('../src/main/server/server-launcher')

    await expect(launchServer()).rejects.toThrow()

    expect(spawned).toEqual([])
    expect(getLastAdoptionRefusal()).toMatchObject({ reason: 'different-build' })
  })

  it('refuses when no credential was published to reach it with', async () => {
    published.port = 50091
    published.token = null
    const { launchServer, getLastAdoptionRefusal } =
      await import('../src/main/server/server-launcher')

    await expect(launchServer()).rejects.toThrow()

    expect(spawned).toEqual([])
    expect(getLastAdoptionRefusal()).toMatchObject({ reason: 'unusable' })
  })

  it('spawns when nothing is published at all', async () => {
    // The one case that is genuinely a free database: no live server anywhere.
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

  it("is NOT detached in dev, so a restart cannot adopt yesterday's source", async () => {
    // Every adoption check would pass for a leftover dev server -- same data
    // directory, same build channel -- and it would be running the code as it
    // was before the edit that prompted the restart, with nothing to say so.
    process.env.ELECTRON_RENDERER_URL = 'http://localhost:5173'
    try {
      const { launchServer } = await import('../src/main/server/server-launcher')

      await launchServer()

      expect(spawned[0].opts.detached).toBeUndefined()
    } finally {
      delete process.env.ELECTRON_RENDERER_URL
    }
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
    published.identity = identityFrom()
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
    published.pidAlive = false // it honours the signal, so the wait is immediate

    await stopServer()

    expect(bridges.at(-1)?.requests).toContain('server:shutdown')
  }, 30000)
})

describe('an adopted server that dies', () => {
  it('is replaced, even though there is no child process to notice it', async () => {
    // Crash recovery hangs off `child.on('exit')`, and an adopted server has no
    // child to emit one. Without this the case adoption exists for would be the
    // one case that never recovers: the bridge retrying forever against a port
    // with nothing behind it, which is the symptom #492 was written to end.
    published.port = 50091
    published.identity = identityFrom()
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
    published.identity = identityFrom()
    const { launchServer } = await import('../src/main/server/server-launcher')
    await launchServer()

    bridges[0].emit('disconnected')
    await new Promise((r) => setTimeout(r, 20))
    expect(spawned).toEqual([])
  })

  it('is not replaced once the app has decided to quit', async () => {
    published.port = 50091
    published.identity = identityFrom()
    const { launchServer, detachFromServer } = await import('../src/main/server/server-launcher')
    await launchServer()

    detachFromServer()
    published.pidAlive = false
    bridges[0].emit('disconnected')
    await new Promise((r) => setTimeout(r, 20))
    expect(spawned).toEqual([])
  })
})

describe('the three ways adoption used to go quietly wrong', () => {
  it('keeps the adopted credential, so a replacement server accepts the same bridge', async () => {
    // `retarget` moves the URL and nothing else -- the credential is fixed when
    // the bridge is built. Minting a fresh secret for the replacement left the
    // surviving bridge presenting a token no server had heard of, reconnecting
    // forever against a server that was perfectly healthy.
    published.port = 50091
    published.identity = identityFrom()
    const { launchServer } = await import('../src/main/server/server-launcher')
    await launchServer()

    published.pidAlive = false
    bridges[0].emit('disconnected')
    await vi.waitFor(() => expect(spawned).toHaveLength(1))

    const env = spawned[0].opts.env as Record<string, string>
    expect(env.SECRET_VORN_BOOTSTRAP_TOKEN).toBe('local-secret')
  })

  it('declines a server that greets us but refuses our credential', async () => {
    // A rejected credential does not fail the connect: the socket opens and the
    // greeting arrives regardless, so `isConnected` always said yes and the old
    // check could never run. An authenticated round trip is the actual question.
    published.port = 50091
    published.identity = identityFrom()
    published.rejectsCredential = true
    const { launchServer } = await import('../src/main/server/server-launcher')

    await expect(launchServer()).rejects.toThrow()

    expect(spawned).toEqual([])
  })

  it('creates the data directory before spawning into it', async () => {
    // The server creates it on init -- but it is also the cwd the server is
    // spawned into, and a missing cwd is an ENOENT before any of that runs.
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    expect(madeDirs).toContain('/Users/x/.vorn')
  })
})

describe('what the reviews caught', () => {
  it("sends the packaged server's output to a file, never to a pipe", async () => {
    // A pipe held by this process dies with this process, and the server is
    // meant to outlive it. Measured: the next write to stderr after the parent
    // goes raises EPIPE, the server has no uncaughtException handler, and it
    // exits -- on its first log line, which is milliseconds away.
    const { launchServer } = await import('../src/main/server/server-launcher')

    await launchServer()

    expect(opened.some((f) => f.endsWith('server.log'))).toBe(true)
    expect((spawned[0].opts.stdio as unknown[])[1]).toBe(99)
    expect((spawned[0].opts.stdio as unknown[])[2]).toBe(99)
  })

  it('will not accept a port file that names a server it did not spawn', async () => {
    // After refusing an incumbent the file still names it with a live pid. The
    // launcher no longer spawns at all in that case, which is the stronger form
    // of the same guarantee: there is no second child to mis-resolve.
    published.port = 50091
    published.spawnedPid = 999
    published.identity = identityFrom()
    published.protocolVersion = RUNTIME_PROTOCOL_VERSION + 1
    const { launchServer } = await import('../src/main/server/server-launcher')

    await expect(launchServer()).rejects.toThrow()

    expect(spawned).toEqual([])
  })

  it('ends the dev server on detach rather than orphaning it', async () => {
    // A dev child is not detached, and POSIX does not kill a plain child when
    // its parent exits. Walking away would leave it for the next `yarn dev` to
    // adopt -- running the source from before the edit that prompted the restart.
    process.env.ELECTRON_RENDERER_URL = 'http://localhost:5173'
    try {
      const { launchServer, detachFromServer } = await import('../src/main/server/server-launcher')
      await launchServer()
      const killed: string[] = []
      ;(spawnedChildren[0] as Record<string, unknown>).kill = (sig: string): void => {
        killed.push(sig)
      }

      detachFromServer()

      expect(killed).toEqual(['SIGTERM'])
    } finally {
      delete process.env.ELECTRON_RENDERER_URL
    }
  })

  it('signals an adopted server that ignores the shutdown request', async () => {
    // An adopted server has no child handle, so the RPC is the only way to ask.
    // If it throws, nothing had happened yet: the user picked "Stop Sessions and
    // Server", the app quit, and every session carried on saying nothing.
    published.port = 50091
    published.identity = identityFrom({ pid: 999 })
    const { launchServer, stopServer } = await import('../src/main/server/server-launcher')
    await launchServer()
    bridges[0].request = async (m: string) => {
      if (m === 'server:shutdown') throw new Error('no answer')
      return {}
    }
    const signalled: number[] = []
    const spy = vi.spyOn(process, 'kill').mockImplementation((pid) => {
      signalled.push(pid as number)
      published.pidAlive = false // it goes once actually signalled
      return true
    })

    await stopServer()
    spy.mockRestore()

    expect(signalled).toContain(999)
  }, 30000)
})

describe('a server whose greeting does not match its port file', () => {
  it('is refused, because that pid is later handed to process.kill', async () => {
    // Both name the same server when everything is honest. Only one of them was
    // written by a process this app can attribute, and it is the one to believe.
    published.port = 50091
    published.identity = identityFrom({ pid: 31337 })
    const { launchServer } = await import('../src/main/server/server-launcher')

    await expect(launchServer()).rejects.toThrow()

    expect(spawned).toEqual([])
  })
})

describe('the dev readiness path', () => {
  it('will not resolve to a port from a file it cannot attribute', async () => {
    // Dev spawns through npx, so the child is a parent of the process that
    // listens and there is no pid to match the file against. Falling back to it
    // would hand the bridge somebody else's port with a credential that server
    // never issued -- the wrong-server failure this change exists to prevent.
    process.env.ELECTRON_RENDERER_URL = 'http://localhost:5173'
    quietSpawn.value = true // the dev child never prints its port line
    published.port = null // nothing to adopt at launch, so it spawns
    try {
      const { launchServer } = await import('../src/main/server/server-launcher')
      const launching = launchServer()
      // An incumbent publishes itself while we are still waiting. The old
      // fallback would have read this and resolved to its port.
      setTimeout(() => {
        published.port = 50091
        published.spawnedPid = 999
      }, 100)

      await expect(launching).rejects.toThrow(/Timeout waiting for server port/i)
    } finally {
      delete process.env.ELECTRON_RENDERER_URL
      quietSpawn.value = false
    }
  }, 30000)
})

describe('ending a server, and knowing that it ended', () => {
  /**
   * Every path here shares one requirement: the caller cannot continue until the
   * process is actually gone. `before-quit` calls `app.quit()` the moment
   * `stopServer` resolves, and the connect window relaunches the moment
   * `stopLocalServer` says yes -- so anything left on a timer is something that
   * never happens.
   */
  function killRecorder(): { signals: string[]; restore: () => void } {
    const signals: string[] = []
    const spy = vi.spyOn(process, 'kill').mockImplementation((_pid, sig) => {
      signals.push(String(sig))
      // A SIGKILL is the end of the argument; anything else it may ignore.
      if (String(sig) === 'SIGKILL') published.pidAlive = false
      return true
    })
    return { signals, restore: () => spy.mockRestore() }
  }

  it('escalates to SIGKILL when the server ignores SIGTERM', async () => {
    published.pidAlive = true
    const { launchServer, stopServer } = await import('../src/main/server/server-launcher')
    await launchServer()
    ;(spawnedChildren[0] as unknown as Record<string, unknown>).kill = (): void => {}
    const { signals, restore } = killRecorder()

    await stopServer()
    restore()

    expect(signals).toContain('SIGKILL')
  }, 30000)

  it('leaves a server that honoured SIGTERM alone', async () => {
    const { launchServer, stopServer } = await import('../src/main/server/server-launcher')
    await launchServer()
    published.pidAlive = false // it went when asked
    ;(spawnedChildren[0] as unknown as Record<string, unknown>).kill = (): void => {}
    const { signals, restore } = killRecorder()

    await stopServer()
    restore()

    expect(signals).not.toContain('SIGKILL')
  }, 30000)

  it('does not resolve while the process is still there', async () => {
    published.pidAlive = true
    const { launchServer, stopServer } = await import('../src/main/server/server-launcher')
    await launchServer()
    ;(spawnedChildren[0] as unknown as Record<string, unknown>).kill = (): void => {}
    const spy = vi.spyOn(process, 'kill').mockImplementation(() => true) // ignores everything

    let settled = false
    const stopping = stopServer().then(() => {
      settled = true
    })
    await new Promise((r) => setTimeout(r, 250))
    expect(settled).toBe(false)

    published.pidAlive = false // it finally exits
    await stopping
    spy.mockRestore()

    expect(settled).toBe(true)
  }, 30000)

  it('stopLocalServer reports success only once the pid is gone', async () => {
    published.port = 50091
    published.pidAlive = true
    const { stopLocalServer } = await import('../src/main/server/server-launcher')
    const spy = vi.spyOn(process, 'kill').mockImplementation(() => true)

    let settled = false
    const stopping = stopLocalServer().then((r) => {
      settled = true
      return r
    })
    await new Promise((r) => setTimeout(r, 250))
    expect(settled).toBe(false)

    published.pidAlive = false
    await expect(stopping).resolves.toEqual({ ok: true })
    spy.mockRestore()
  }, 30000)

  it('stopLocalServer reports failure when it will not go at all', async () => {
    published.port = 50091
    published.pidAlive = true
    const { stopLocalServer } = await import('../src/main/server/server-launcher')
    const spy = vi.spyOn(process, 'kill').mockImplementation(() => true)

    const result = await stopLocalServer()
    spy.mockRestore()

    expect(result).toMatchObject({ ok: false })
  }, 30000)
})
