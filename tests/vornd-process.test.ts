import { describe, it, expect, vi, afterEach } from 'vitest'
import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import {
  findVornd,
  nativeServerSwitch,
  startVornd,
  VorndKeeper,
  type Vornd,
  type VorndBinaries
} from '../packages/server/src/vornd-process'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

/**
 * Starting vornd from the server, against a stand-in that behaves the ways vornd
 * can: says where it listens, says nothing, says something else, or dies. The
 * stand-in is a Node script so the test does not need the Rust build; the real
 * binary gets its own test below when it is built.
 */
const STUB = `
const mode = process.env.STUB_MODE
const upstream = process.argv[process.argv.indexOf('--upstream') + 1]
if (!process.argv.includes('--exit-with-stdin')) process.exit(9)
if (mode === 'ok') {
  process.stdout.write(JSON.stringify({ port: 47001, protocol: 1, upstream }) + '\\n')
  process.stdin.resume()
  process.stdin.on('end', () => process.exit(0))
} else if (mode === 'native') {
  process.stdout.write(JSON.stringify({ port: 47001, protocol: 1, native: ['git', 'pairing', 5] }) + '\\n')
  process.stdin.resume()
  process.stdin.on('end', () => process.exit(0))
} else if (mode === 'garbage') {
  process.stdout.write('listening!\\n')
  setInterval(() => {}, 1000)
} else if (mode === 'protocol') {
  process.stdout.write('{"port":47001,"protocol":2}\\n')
  setInterval(() => {}, 1000)
} else if (mode === 'dies') {
  process.stderr.write('vornd: --upstream: invalid socket address syntax\\n')
  process.exit(2)
} else {
  setInterval(() => {}, 1000)
}
`

const dir = mkdtempSync(path.join(tmpdir(), 'vornd-launch-'))
const stub = path.join(dir, 'vornd-stub.cjs')
writeFileSync(stub, STUB)

const children: ChildProcess[] = []
const started: Vornd[] = []

function stubSpawn(mode: string): typeof spawn {
  return ((_binary: string, args: string[], options: object) => {
    const child = spawn(process.execPath, [stub, ...args], {
      ...options,
      env: { ...process.env, STUB_MODE: mode }
    })
    children.push(child)
    return child
  }) as unknown as typeof spawn
}

function exited(child: ChildProcess): Promise<number | null> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve(child.exitCode)
  return new Promise((resolve) => child.once('exit', (code) => resolve(code)))
}

afterEach(() => {
  for (const v of started.splice(0)) v.stop()
  for (const child of children.splice(0)) child.kill()
})

describe('finding vornd', () => {
  const packaged = path.join('/app', 'Resources', 'server')
  const at = (dir: string, name: string): string => path.join(dir, name)

  it('looks beside the server in a packaged app, with the session holder next to it', () => {
    const seen: string[] = []
    const found = findVornd(packaged, {}, 'darwin', (f) => (seen.push(f), true))
    const dir = path.join('/app', 'Resources', 'vornd')
    expect(found).toEqual({ vornd: at(dir, 'vornd'), sessiond: at(dir, 'vorn-sessiond') })
    expect(seen[0]).toBe(at(dir, 'vornd'))
  })

  it('looks for the .exe names on Windows', () => {
    const dir = path.join('/app', 'Resources', 'vornd')
    expect(findVornd(packaged, {}, 'win32', () => true)).toEqual({
      vornd: at(dir, 'vornd.exe'),
      sessiond: at(dir, 'vorn-sessiond.exe')
    })
  })

  it('looks where build:core copies it in a checkout, then in cargo’s target directory', () => {
    const src = path.join('/repo', 'packages', 'server', 'src')
    const core = path.join('/repo', 'packages', 'core')
    const release = path.join(core, 'target', 'release', 'vornd')
    expect(findVornd(src, {}, 'linux', (f) => f === release)).toEqual({
      vornd: release,
      sessiond: null
    })
    expect(
      findVornd(
        src,
        {},
        'linux',
        (f) => f !== at(path.join('/repo', 'packages', 'server', 'vornd'), 'vornd')
      )?.vornd
    ).toBe(at(core, 'vornd'))
  })

  it('takes the binary VORN_VORND_PATH names first', () => {
    const named = path.resolve('/opt/vornd/vornd')
    expect(findVornd(packaged, { VORN_VORND_PATH: named }, 'linux', () => true)?.vornd).toBe(named)
  })

  it('answers null when the build has none', () => {
    expect(findVornd(packaged, {}, 'linux', () => false)).toBeNull()
  })
})

describe('starting vornd', () => {
  it('resolves with the port it reports, forwarding to the server', async () => {
    const vornd = await startVornd('vornd', 50091, { spawnImpl: stubSpawn('ok') })
    started.push(vornd)
    expect(vornd.port).toBe(47001)
    expect(vornd.upstream).toBe(50091)
  })

  it('reads the groups it answers itself, and none when it names none', async () => {
    const native = await startVornd('vornd', 50091, { spawnImpl: stubSpawn('native') })
    started.push(native)
    expect(native.native).toEqual(['git', 'pairing'])
    const plain = await startVornd('vornd', 50091, { spawnImpl: stubSpawn('ok') })
    started.push(plain)
    expect(plain.native).toBeUndefined()
  })

  it("hands vornd the desktop's token in its environment, never in its arguments", async () => {
    let seen: { args: string[]; env?: Record<string, string | undefined> } = { args: [] }
    const spawnImpl = ((binary: string, args: string[], options: { env?: never }) => {
      seen = { args, env: options.env }
      return stubSpawn('ok')(binary, args, options as never)
    }) as unknown as typeof spawn
    started.push(await startVornd('vornd', 50091, { spawnImpl, desktopToken: 'launch-secret' }))
    expect(seen.env?.VORND_DESKTOP_TOKEN).toBe('launch-secret')
    expect(seen.args.join(' ')).not.toContain('launch-secret')
    started.push(await startVornd('vornd', 50091, { spawnImpl }))
    expect(seen.env).toBeUndefined()
  })

  it('passes the session holder and its data directory when there is one', async () => {
    let seen: string[] = []
    const spawnImpl = ((binary: string, args: string[], options: object) => {
      seen = args
      return stubSpawn('ok')(binary, args, options as never)
    }) as unknown as typeof spawn
    started.push(await startVornd('vornd', 50091, { spawnImpl }))
    expect(seen).not.toContain('--sessiond')
    started.push(
      await startVornd('vornd', 50091, {
        spawnImpl,
        sessiond: { binary: '/app/vornd/vorn-sessiond', home: '/Users/x/.vorn' }
      })
    )
    expect(seen).toEqual([
      '--upstream',
      '127.0.0.1:50091',
      '--exit-with-stdin',
      '--sessiond',
      '/app/vornd/vorn-sessiond',
      '--home',
      '/Users/x/.vorn'
    ])
  })

  it('names the database when it has one, and asks it to answer the calls it has taken over only when told to', async () => {
    let seen: string[] = []
    const spawnImpl = ((binary: string, args: string[], options: object) => {
      seen = args
      return stubSpawn('ok')(binary, args, options as never)
    }) as unknown as typeof spawn
    started.push(await startVornd('vornd', 50091, { spawnImpl }))
    expect(seen).not.toContain('--native-server')
    expect(seen).not.toContain('--db')
    started.push(await startVornd('vornd', 50091, { spawnImpl, db: '/Users/x/.vorn/vorn.db' }))
    expect(seen).toEqual([
      '--upstream',
      '127.0.0.1:50091',
      '--exit-with-stdin',
      '--db',
      '/Users/x/.vorn/vorn.db'
    ])
    started.push(
      await startVornd('vornd', 50091, {
        spawnImpl,
        db: '/Users/x/.vorn/vorn.db',
        nativeServer: true
      })
    )
    expect(seen.slice(3)).toEqual(['--db', '/Users/x/.vorn/vorn.db', '--native-server'])
  })

  it('stops it by closing its stdin, and does not report that as an exit', async () => {
    const vornd = await startVornd('vornd', 50091, { spawnImpl: stubSpawn('ok') })
    const onExit = vi.fn()
    vornd.onExit(onExit)
    vornd.stop()
    await exited(children[0]!)
    expect(onExit).not.toHaveBeenCalled()
  })

  it('reports an exit nobody asked for', async () => {
    const vornd = await startVornd('vornd', 50091, { spawnImpl: stubSpawn('ok') })
    started.push(vornd)
    const why = new Promise<string>((resolve) => vornd.onExit(resolve))
    children[0]!.kill('SIGKILL')
    expect(await why).toMatch(/signal=SIGKILL/)
  })

  it('fails with what it printed when it exits before listening', async () => {
    await expect(startVornd('vornd', 50091, { spawnImpl: stubSpawn('dies') })).rejects.toThrow(
      /exited before it was listening \(code=2.*invalid socket address/
    )
  })

  it('fails, and ends it, when it says something other than a port', async () => {
    await expect(startVornd('vornd', 50091, { spawnImpl: stubSpawn('garbage') })).rejects.toThrow(
      /something other than where it listens/
    )
    await exited(children[0]!)
  })

  it('fails on a protocol this app does not know', async () => {
    await expect(startVornd('vornd', 50091, { spawnImpl: stubSpawn('protocol') })).rejects.toThrow(
      /protocol 2/
    )
    await exited(children[0]!)
  })

  it('gives up, and ends it, when it never says where it listens', async () => {
    await expect(
      startVornd('vornd', 50091, { spawnImpl: stubSpawn('silent'), timeoutMs: 300 })
    ).rejects.toThrow(/within 300ms/)
    await exited(children[0]!)
  })

  it('fails when the binary is not there', async () => {
    await expect(startVornd(path.join(dir, 'no-such-vornd'), 50091)).rejects.toThrow(
      /could not start vornd/
    )
  })
})

const built = findVornd(path.resolve(__dirname, '..', 'packages', 'server', 'src'))?.vornd

describe.runIf(built && existsSync(built))('the real vornd', () => {
  it('starts, listens on loopback, and exits when stopped', async () => {
    const children: ChildProcess[] = []
    const vornd = await startVornd(built!, 9, {
      spawnImpl: ((...a: Parameters<typeof spawn>) => {
        const child = spawn(...a)
        children.push(child)
        return child
      }) as typeof spawn
    })
    expect(vornd.port).toBeGreaterThan(0)
    const health = await fetch(`http://127.0.0.1:${vornd.port}/vornd/health`)
    // Nothing listens on port 9, so vornd is up and its server is not.
    expect(health.status).toBe(503)
    vornd.stop()
    expect(await exited(children[0]!)).toBe(0)
  })
})

describe('keeping vornd running', () => {
  const binaries: VorndBinaries = { vornd: '/b/vornd', sessiond: '/b/vorn-sessiond' }

  /** A started vornd the test can make exit, and see stopped. */
  function fakeVornd(port: number, app: string | null = '/run/app.sock') {
    let exit: ((detail: string) => void) | null = null
    const vornd: Vornd & { exit(detail: string): void; stopped: boolean } = {
      port,
      upstream: 50091,
      ...(app ? { app } : {}),
      stopped: false,
      onExit: (listener) => void (exit = listener),
      stop: () => void (vornd.stopped = true),
      exit: (detail) => exit?.(detail)
    }
    return vornd
  }

  afterEach(() => {
    vi.useRealTimers()
  })

  it('starts vornd with its holder and the desktop token, then connects to its channel', async () => {
    const running = fakeVornd(47001)
    const start = vi.fn(async () => running)
    const connect = vi.fn(async () => true)
    const keeper = new VorndKeeper({
      find: () => binaries,
      start: start as never,
      connect,
      desktopToken: () => 'secret'
    })
    expect(keeper.state).toEqual({ state: 'off' })
    const launched = keeper.launch(50091, '/home/x/.vorn')
    expect(keeper.starting).toBe(true)
    await keeper.ready()
    await launched
    expect(start).toHaveBeenCalledWith('/b/vornd', 50091, {
      sessiond: { binary: '/b/vorn-sessiond', home: '/home/x/.vorn' },
      desktopToken: 'secret',
      db: path.join('/home/x/.vorn', 'vorn.db'),
      nativeServer: false
    })
    expect(connect).toHaveBeenCalledWith('/run/app.sock')
    expect(keeper.state).toEqual({ state: 'on', port: 47001, nativeServer: false })
    expect(keeper.port).toBe(47001)
    expect(keeper.answers('pairing')).toBe(false)
    expect(keeper.starting).toBe(false)
    keeper.stop()
    expect(running.stopped).toBe(true)
    expect(keeper.state).toEqual({ state: 'off' })
    expect(keeper.port).toBeNull()
  })

  it('reads the Native server switch at each start, and says what vornd was started with', async () => {
    vi.useFakeTimers()
    let first: ReturnType<typeof fakeVornd> | null = null
    const start = vi
      .fn()
      .mockImplementationOnce(async () => (first = fakeVornd(47001)))
      .mockImplementationOnce(async () => fakeVornd(47002))
    let on = true
    const keeper = new VorndKeeper({
      find: () => binaries,
      start: start as never,
      connect: async () => true,
      nativeServer: () => on
    })
    await keeper.launch(50091, path.join('/home', 'x', '.vorn'))
    expect(start).toHaveBeenLastCalledWith('/b/vornd', 50091, {
      sessiond: { binary: '/b/vorn-sessiond', home: path.join('/home', 'x', '.vorn') },
      desktopToken: undefined,
      db: path.join('/home', 'x', '.vorn', 'vorn.db'),
      nativeServer: true
    })
    expect(keeper.state).toEqual({ state: 'on', port: 47001, nativeServer: true })
    first!.native = ['pairing']
    expect(keeper.answers('pairing')).toBe(true)
    expect(keeper.answers('git')).toBe(false)

    on = false
    first!.exit('code=1, signal=null')
    await vi.advanceTimersByTimeAsync(500)
    await keeper.ready()
    expect(start.mock.lastCall?.[2]).toMatchObject({ nativeServer: false })
    expect(keeper.state).toEqual({ state: 'on', port: 47002, nativeServer: false })
    keeper.stop()
  })

  it('says how to get vornd when the build has none', async () => {
    const keeper = new VorndKeeper({ find: () => null, connect: async () => true })
    await keeper.launch(50091, '/h')
    expect(keeper.state).toEqual({
      state: 'failed',
      detail: expect.stringMatching(/yarn build:core/)
    })
  })

  it('starts vornd without a holder when the build has none, and connects nowhere', async () => {
    const start = vi.fn(async () => fakeVornd(47001, null))
    const connect = vi.fn(async () => true)
    const keeper = new VorndKeeper({
      find: () => ({ vornd: '/b/vornd', sessiond: null }),
      start: start as never,
      connect
    })
    await keeper.launch(50091, '/h')
    expect(start).toHaveBeenCalledWith('/b/vornd', 50091, {
      sessiond: undefined,
      desktopToken: undefined,
      db: path.join('/h', 'vorn.db'),
      nativeServer: false
    })
    expect(connect).not.toHaveBeenCalled()
    expect(keeper.state.state).toBe('on')
  })

  it('keeps going when it cannot reach the channel', async () => {
    const keeper = new VorndKeeper({
      find: () => binaries,
      start: (async () => fakeVornd(47001)) as never,
      connect: async () => false
    })
    await keeper.launch(50091, '/h')
    expect(keeper.state).toEqual({ state: 'on', port: 47001, nativeServer: false })
  })

  it('tries again with a growing delay when vornd fails to start', async () => {
    vi.useFakeTimers()
    const start = vi
      .fn()
      .mockRejectedValueOnce(new Error('vornd exited before it was listening'))
      .mockRejectedValueOnce(new Error('still not'))
      .mockResolvedValue(fakeVornd(47002))
    const keeper = new VorndKeeper({
      find: () => binaries,
      start: start as never,
      connect: async () => true
    })
    await keeper.launch(50091, '/h')
    expect(keeper.state).toEqual({
      state: 'failed',
      detail: 'vornd exited before it was listening'
    })
    await vi.advanceTimersByTimeAsync(499)
    expect(start).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(start).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(999)
    expect(start).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(1)
    expect(start).toHaveBeenCalledTimes(3)
    expect(keeper.state).toEqual({ state: 'on', port: 47002, nativeServer: false })
  })

  it('starts vornd again when it exits on its own, and starts the delay over after a long run', async () => {
    vi.useFakeTimers()
    let now = 0
    const first = fakeVornd(47001)
    const second = fakeVornd(47002)
    const third = fakeVornd(47003)
    const start = vi
      .fn()
      .mockResolvedValueOnce(first)
      .mockResolvedValueOnce(second)
      .mockResolvedValueOnce(third)
    const connect = vi.fn(async () => true)
    const keeper = new VorndKeeper({
      find: () => binaries,
      start: start as never,
      connect,
      now: () => now
    })
    await keeper.launch(50091, '/h')
    first.exit('code=1, signal=null')
    expect(keeper.state).toEqual({ state: 'failed', detail: 'vornd exited (code=1, signal=null)' })
    expect(keeper.port).toBeNull()
    await vi.advanceTimersByTimeAsync(500)
    expect(keeper.state).toEqual({ state: 'on', port: 47002, nativeServer: false })
    expect(connect).toHaveBeenCalledTimes(2)

    // Up for longer than a minute: the next restart waits the first delay again.
    now = 120_000
    second.exit('code=null, signal=SIGKILL')
    await vi.advanceTimersByTimeAsync(500)
    expect(keeper.state).toEqual({ state: 'on', port: 47003, nativeServer: false })
  })

  it('does not restart vornd once stopped, and stops one that finishes starting after', async () => {
    vi.useFakeTimers()
    let finish: (v: Vornd) => void = () => {}
    const late = fakeVornd(47001)
    const start = vi.fn(() => new Promise<Vornd>((resolve) => (finish = resolve)))
    const connect = vi.fn(async () => true)
    const keeper = new VorndKeeper({ find: () => binaries, start: start as never, connect })
    const launched = keeper.launch(50091, '/h')
    keeper.stop()
    finish(late)
    await launched
    expect(late.stopped).toBe(true)
    expect(connect).not.toHaveBeenCalled()
    expect(keeper.state).toEqual({ state: 'off' })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(start).toHaveBeenCalledTimes(1)
  })

  it('ignores an exit from a vornd it already stopped', async () => {
    vi.useFakeTimers()
    const running = fakeVornd(47001)
    const start = vi.fn(async () => running)
    const keeper = new VorndKeeper({
      find: () => binaries,
      start: start as never,
      connect: async () => true
    })
    await keeper.launch(50091, '/h')
    keeper.stop()
    running.exit('code=0, signal=null')
    await vi.advanceTimersByTimeAsync(60_000)
    expect(keeper.state).toEqual({ state: 'off' })
    expect(start).toHaveBeenCalledTimes(1)
  })

  it('shares one start between callers', async () => {
    const start = vi.fn(async () => fakeVornd(47001))
    const keeper = new VorndKeeper({
      find: () => binaries,
      start: start as never,
      connect: async () => true
    })
    const a = keeper.launch(50091, '/h')
    const b = keeper.launch(50091, '/h')
    await Promise.all([a, b, keeper.ready()])
    expect(start).toHaveBeenCalledTimes(1)
  })
})

describe('the Native server switch', () => {
  afterEach(() => {
    vi.unstubAllEnvs()
  })

  it('follows Settings › Experimental, off by default', () => {
    vi.stubEnv('VORN_NATIVE_SERVER', '')
    expect(nativeServerSwitch(undefined)).toBe(false)
    expect(nativeServerSwitch({})).toBe(false)
    expect(nativeServerSwitch({ nativeServer: true })).toBe(true)
  })

  it('takes VORN_NATIVE_SERVER over the setting, both ways', () => {
    vi.stubEnv('VORN_NATIVE_SERVER', '1')
    expect(nativeServerSwitch({})).toBe(true)
    vi.stubEnv('VORN_NATIVE_SERVER', '0')
    expect(nativeServerSwitch({ nativeServer: true })).toBe(false)
  })
})
