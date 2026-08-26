import { describe, it, expect, beforeAll, beforeEach, afterEach, vi } from 'vitest'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'

spawnsRealServers()

/**
 * The real launcher, against real servers.
 *
 * `server-launcher-adoption.test.ts` fakes the bridge and the filesystem, which
 * is the right way to pin what the *decision* is. It cannot pin whether the
 * decision meets reality, and that is where the expensive bug was: a leftover
 * endpoint is the ordinary state of a machine between launches, the launcher
 * treated it as proof a server was running, and quitting Vorn once made Vorn
 * unable to start again. Every mock in that file agreed with the code.
 *
 * So here nothing is faked but Electron itself: a real `launchServer` spawns a
 * real detached server, claims a real socket, and a real `ServerBridge` connects
 * over `ws+unix://`.
 *
 * HOME is redirected, which is what isolates it -- `resolveDataDir` is
 * `os.homedir()/.vorn`, and `shutdown()` writes `~/.claude/settings.json`
 * regardless of any data dir. Nothing here can touch the developer's own server.
 */

const REPO = path.join(__dirname, '..')
const BUNDLE = path.join(REPO, 'packages', 'server', 'dist', 'index.cjs')

let home: string
let resources: string
let appPath: string

beforeAll(() => {
  if (!fs.existsSync(BUNDLE)) {
    const built = spawnSync('yarn', ['build'], {
      cwd: path.join(REPO, 'packages', 'server'),
      stdio: 'inherit',
      shell: false
    })
    if (built.status !== 0) throw new Error('could not build the server bundle')
  }
})

vi.mock('electron', () => ({
  app: {
    getPath: () => path.join(os.tmpdir(), 'vorn-launcher-userdata'),
    // A packaged app's shape, built in the sandbox: the spawn derives the native
    // module paths from this (`<appPath>/node_modules` and
    // `<appPath>.unpacked/node_modules`) so a plain Node child can require what
    // Electron would otherwise resolve through its ASAR patching. Pointing it at
    // a fiction leaves the child unable to load libsql, which fails exactly like
    // a server that will not start.
    getAppPath: () => appPath,
    getVersion: () => '0.7.0-test',
    isPackaged: true
  }
}))
vi.mock('../src/main/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

/** Whether anything is serving this name right now. */
function served(at: string): Promise<boolean> {
  return new Promise((resolve) => {
    const c = net.connect(at)
    const done = (answer: boolean): void => {
      c.destroy()
      resolve(answer)
    }
    c.setTimeout(1_000, () => done(false))
    c.once('connect', () => done(true))
    c.once('error', () => done(false))
  })
}

const socketPath = (): string => path.join(home, '.vorn', 'vorn.sock')
const tokenPath = (): string => path.join(home, '.vorn', 'local-token')

/**
 * The server this sandbox is running, by the pid it published.
 *
 * Not `pgrep` on the data directory: the packaged spawn deliberately passes no
 * `--data-dir` argument, so the path appears nowhere on the command line and a
 * pattern match silently finds nothing -- which reads as "no server" and makes
 * every assertion about killing one vacuously true.
 */
function serverPid(): number | null {
  try {
    const raw = JSON.parse(fs.readFileSync(path.join(home, '.vorn', 'ws-port'), 'utf-8'))
    return typeof raw.pid === 'number' && alive(raw.pid) ? raw.pid : null
  } catch {
    return null
  }
}

beforeEach(() => {
  vi.resetModules()
  home = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-launcher-'))
  fs.mkdirSync(path.join(home, '.vorn'), { recursive: true, mode: 0o700 })
  // `resolveServerEntry` reads this for the packaged branch, which is the one
  // that spawns detached — the behaviour under test.
  resources = path.join(home, 'Resources')
  fs.mkdirSync(resources, { recursive: true })
  // Symlinked, not copied. The bundle leaves native modules external -- libsql
  // and node-pty resolve from node_modules relative to the entry -- and Node
  // resolves through a symlink by its realpath, so this finds them where a copy
  // in a temp directory could not.
  fs.symlinkSync(path.dirname(BUNDLE), path.join(resources, 'server'))
  appPath = path.join(home, 'app')
  fs.mkdirSync(appPath, { recursive: true })
  fs.mkdirSync(`${appPath}.unpacked`, { recursive: true })
  const realModules = path.join(REPO, 'node_modules')
  fs.symlinkSync(realModules, path.join(appPath, 'node_modules'))
  fs.symlinkSync(realModules, path.join(`${appPath}.unpacked`, 'node_modules'))
  ;(process as NodeJS.Process & { resourcesPath?: string }).resourcesPath = resources
  process.env.HOME = home
  process.env.USERPROFILE = home
  process.env.VORN_BUILD_CHANNEL = 'packaged'
  process.env.VORN_IDLE_TIMEOUT_MS = '600000'
  delete process.env.VORN_DATA_DIR
})

afterEach(async () => {
  const pid = serverPid()
  if (pid) {
    try {
      process.kill(pid, 'SIGKILL')
    } catch {
      /* already gone */
    }
  }
  await sleep(300)
  fs.rmSync(home, { recursive: true, force: true })
})

describe('launching against a real machine', () => {
  it('starts a server, claims the endpoint, and connects over it', async () => {
    const { launchServer, detachFromServer } = await import('../src/main/server/server-launcher')

    const bridge = await launchServer()

    expect(fs.lstatSync(socketPath()).isSocket(), 'no endpoint was claimed').toBe(true)
    expect(await served(socketPath()), 'the endpoint is not being served').toBe(true)
    expect(bridge.isConnected, 'the bridge never connected').toBe(true)
    detachFromServer()
  }, 90_000)

  it('adopts the running server instead of starting a second', async () => {
    const first = await import('../src/main/server/server-launcher')
    await first.launchServer()
    const held = fs.lstatSync(socketPath()).ino
    const owner = serverPid()
    first.detachFromServer()

    // A fresh launcher, exactly as a second app launch would be.
    vi.resetModules()
    const second = await import('../src/main/server/server-launcher')
    const bridge = await second.launchServer()

    expect(fs.lstatSync(socketPath()).ino, 'the endpoint changed hands').toBe(held)
    expect(serverPid(), 'a second server took over').toBe(owner)
    expect(bridge.isConnected, 'the second launch never connected').toBe(true)
    second.detachFromServer()
  }, 90_000)

  it('starts again after a quit, past the endpoint the old server left behind', async () => {
    // The bug this test exists for. Nothing removes the endpoint on shutdown, so
    // this is what every quit leaves: a socket file with nothing behind it and no
    // credential beside it. A launcher that reads that as a running server
    // refuses to start, and the app cannot be opened again.
    const first = await import('../src/main/server/server-launcher')
    await first.launchServer()
    const abandoned = fs.lstatSync(socketPath()).ino
    await first.stopServer()
    await sleep(1_500)

    expect(fs.lstatSync(socketPath()).isSocket(), 'the endpoint was removed on quit').toBe(true)
    expect(await served(socketPath()), 'something still serves a stopped server').toBe(false)
    expect(fs.existsSync(tokenPath()), 'the credential outlived its server').toBe(false)

    vi.resetModules()
    const second = await import('../src/main/server/server-launcher')
    const bridge = await second.launchServer()

    expect(bridge.isConnected, 'the app could not start again after a quit').toBe(true)
    expect(fs.lstatSync(socketPath()).ino, 'the leftover was never replaced').not.toBe(abandoned)
    expect(await served(socketPath()), 'the new server is not serving').toBe(true)
    second.detachFromServer()
  }, 90_000)

  it('starts again after a crash, past the endpoint and the credential left behind', async () => {
    // SIGKILL runs nothing on the way out, so this leftover keeps its credential
    // too -- a stale secret beside a dead socket, which must not read as a
    // server. The app is let go of first, because this is the case where Vorn was
    // closed and the server died afterwards: a running app would relaunch its own
    // server rather than leave the corpse for the next launch, and does.
    const first = await import('../src/main/server/server-launcher')
    await first.launchServer()
    const abandoned = fs.lstatSync(socketPath()).ino
    const doomed = serverPid()
    expect(doomed, 'no server to kill').not.toBeNull()

    first.detachFromServer()
    process.kill(doomed as number, 'SIGKILL')
    for (let i = 0; i < 40 && alive(doomed as number); i++) await sleep(100)
    expect(alive(doomed as number), 'the server survived SIGKILL').toBe(false)

    expect(fs.existsSync(tokenPath()), 'a crash somehow cleaned up after itself').toBe(true)
    expect(await served(socketPath()), 'something is still serving a killed server').toBe(false)

    vi.resetModules()
    const second = await import('../src/main/server/server-launcher')
    const bridge = await second.launchServer()

    expect(bridge.isConnected, 'the app could not start again after a crash').toBe(true)
    expect(fs.lstatSync(socketPath()).ino, 'the corpse was never replaced').not.toBe(abandoned)
    second.detachFromServer()
  }, 90_000)

  it('replaces its own server when that server crashes under it', async () => {
    // The other half, and the reason the case above has to let go first: an app
    // that is still running owns the server it started, so a crash is answered
    // with a replacement rather than left for the next launch. Found by a test
    // that killed a server the launcher was still watching and was surprised the
    // endpoint kept answering.
    const app = await import('../src/main/server/server-launcher')
    await app.launchServer()
    const doomed = serverPid()

    process.kill(doomed as number, 'SIGKILL')
    const deadline = Date.now() + 30_000
    while (Date.now() < deadline && (serverPid() === doomed || serverPid() === null)) {
      await sleep(250)
    }

    expect(serverPid(), 'no replacement was started').not.toBe(doomed)
    expect(await served(socketPath()), 'the replacement never took the endpoint').toBe(true)
    app.detachFromServer()
  }, 90_000)

  it('leaves the running server and its sessions alone when it adopts', async () => {
    const first = await import('../src/main/server/server-launcher')
    const bridge = await first.launchServer()
    const created = (await bridge.request('shell:create', os.tmpdir())) as { id: string }
    expect(created.id, 'no session was created to protect').toBeTruthy()
    const owner = serverPid()
    first.detachFromServer()

    vi.resetModules()
    const second = await import('../src/main/server/server-launcher')
    const rejoined = await second.launchServer()
    const sessions = (await rejoined.request('terminal:listActive')) as Array<{ id: string }>

    expect(serverPid(), 'the server was replaced rather than adopted').toBe(owner)
    expect(
      sessions.map((s) => s.id),
      'the adopted server lost its session'
    ).toContain(created.id)
    second.detachFromServer()
  }, 90_000)
})
