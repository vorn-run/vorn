import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'

/**
 * Pointing the desktop at a server on another machine.
 *
 * Two things must not happen: spawning a local server anyway, which would mean two
 * databases with the local one silently shadowing the host; and asking the host to
 * shut down on quit, which would take it away from everyone else connected.
 */

const hostSettings = {
  value: { mode: 'local', url: '', token: undefined } as Record<string, unknown>
}
const spawned: string[] = []

vi.mock('../src/main/server/host-store', () => ({
  readHostSettings: () => hostSettings.value
}))

const bridges: FakeBridge[] = []
/** Set when the test wants a host that accepts the socket but never identifies. */
const connectHangs = { value: false }

class FakeBridge extends EventEmitter {
  requests: string[] = []
  closed = false
  constructor(
    public url: string,
    public credential?: string
  ) {
    super()
    bridges.push(this)
  }
  connect(): void {
    // The real bridge emits this once the socket opens and identifies. A host that
    // never answers simply never emits, which is the case the timeout exists for.
    if (connectHangs.value) return
    setImmediate(() => this.emit('connected'))
  }
  async request(method: string): Promise<unknown> {
    this.requests.push(method)
    return undefined
  }
  close(): void {
    this.closed = true
  }
}

vi.mock('../src/main/server/server-bridge', () => ({ ServerBridge: FakeBridge }))

/** The bridge the launcher built, if it built one at all. */
const lastBridge = (): FakeBridge | undefined => bridges.at(-1)

vi.mock('node:child_process', () => ({
  spawn: (cmd: string) => {
    spawned.push(cmd)
    throw new Error('a local server must not be spawned in host mode')
  }
}))
vi.mock('electron', () => ({
  app: { getPath: () => '/userData', isPackaged: false },
  utilityProcess: { fork: () => ({ kill: () => {} }) }
}))
vi.mock('../src/main/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

beforeEach(() => {
  vi.resetModules()
  spawned.length = 0
  bridges.length = 0
  connectHangs.value = false
  hostSettings.value = { mode: 'local', url: '', token: undefined }
})

describe('launching in host mode', () => {
  it('connects to the host instead of spawning a server', async () => {
    hostSettings.value = { mode: 'host', url: 'ws://box:61601/ws', token: 'vorn_a_b' }
    const { launchServer } = await import('../src/main/server/server-launcher')

    const bridge = await launchServer()

    expect(spawned).toEqual([])
    expect((bridge as unknown as FakeBridge).url).toBe('ws://box:61601/ws')
  })

  it('authenticates with the device token, not the bootstrap secret', async () => {
    // The per-launch bootstrap secret only authenticates a server this app
    // started; a remote host has never seen it.
    hostSettings.value = { mode: 'host', url: 'ws://box:61601/ws', token: 'vorn_a_b' }
    const { launchServer } = await import('../src/main/server/server-launcher')

    const bridge = await launchServer()

    expect((bridge as unknown as FakeBridge).credential).toBe('vorn_a_b')
  })

  it('does not ask a host to shut down when this desktop quits', async () => {
    // The host is shared. Closing a laptop must not take the server away from
    // every other desktop and phone connected to it.
    hostSettings.value = { mode: 'host', url: 'ws://box:61601/ws', token: 'vorn_a_b' }
    const { launchServer, stopServer } = await import('../src/main/server/server-launcher')
    await launchServer()

    await stopServer()

    expect(lastBridge()?.requests).not.toContain('server:shutdown')
    expect(lastBridge()?.closed).toBe(true)
  })

  it('gives up the socket when the host never answers', async () => {
    // The bridge reconnects every two seconds on its own, and nothing acts on a
    // late success once startup has moved to the connect window. Leaving it running
    // meant a timer and a listener churning for as long as the app was open.
    hostSettings.value = { mode: 'host', url: 'ws://box:61601/ws', token: 'vorn_a_b' }
    connectHangs.value = true
    vi.useFakeTimers()
    const { launchServer, getServerBridge } = await import('../src/main/server/server-launcher')

    const attempt = launchServer()
    const settled = attempt.then(
      () => 'resolved',
      () => 'rejected'
    )
    await vi.advanceTimersByTimeAsync(20_000)

    expect(await settled).toBe('rejected')
    expect(lastBridge()?.closed).toBe(true)
    expect(getServerBridge()).toBeNull()
    vi.useRealTimers()
  })

  it('asks for a token rather than quietly opening a local database', async () => {
    // Configured for a host but holding no credential — safeStorage was
    // unavailable, or the keychain moved. Falling through to a local server would
    // open a different database than the one asked for, and everything would look
    // empty rather than wrong. Failing puts the connect window up.
    hostSettings.value = { mode: 'host', url: 'ws://box:61601/ws', token: undefined }
    const { launchServer } = await import('../src/main/server/server-launcher')

    await expect(launchServer()).rejects.toThrow(/No stored token/)
    expect(spawned).toEqual([])
    expect(lastBridge()).toBeUndefined()
  })
})
