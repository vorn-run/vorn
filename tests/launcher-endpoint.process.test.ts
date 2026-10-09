import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { builtSessiond, builtVornd } from './helpers/served'

spawnsRealServers()

/**
 * The real launcher, against a real vornd.
 *
 * `server-launcher-adoption.test.ts` fakes the bridge and the filesystem, which
 * is the right way to pin what the *decision* is. It cannot pin whether the
 * decision meets reality: what a quit or a crash leaves behind is the ordinary
 * state of a machine between launches, and a launcher that misreads it leaves
 * Vorn unable to start again.
 *
 * So here nothing is faked but Electron itself: a real `launchServer` spawns a
 * real detached vornd from the app's resources, and a real `ServerBridge`
 * connects to it. HOME is a directory of the test's own, so the data directory
 * the launcher resolves is too; nothing here can touch the developer's own.
 */

let home: string
let resources: string

vi.mock('electron', () => ({
  app: {
    getPath: () => path.join(os.tmpdir(), 'vorn-launcher-userdata'),
    getAppPath: () => path.join(home, 'app'),
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

const tokenPath = (): string => path.join(home, '.vorn', 'local-token')

/**
 * The server this sandbox is running, by the pid it published.
 *
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
  home = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-launcher-')))
  fs.mkdirSync(path.join(home, '.vorn'), { recursive: true, mode: 0o700 })
  // The packaged app's shape: vornd and its session holder in Resources/vornd.
  resources = path.join(home, 'Resources')
  fs.mkdirSync(path.join(resources, 'vornd'), { recursive: true })
  fs.symlinkSync(builtVornd!, path.join(resources, 'vornd', 'vornd'))
  fs.symlinkSync(builtSessiond!, path.join(resources, 'vornd', 'vorn-sessiond'))
  ;(process as NodeJS.Process & { resourcesPath?: string }).resourcesPath = resources
  process.env.HOME = home
  process.env.USERPROFILE = home
  process.env.VORN_BUILD_CHANNEL = 'packaged'
  process.env.VORN_IDLE_TIMEOUT_MS = '600000'
  // A debug vornd refuses the default data directory, which here is under this test's own HOME.
  process.env.VORN_ALLOW_DEFAULT_DATA_DIR = os.homedir() === home ? '1' : '0'
  delete process.env.VORN_DATA_DIR
  delete process.env.VORN_VORND_PATH
})

/**
 * Every server this sandbox started, published or not.
 *
 * `serverPid()` reads the one named in `ws-port`, and killing only that one
 * leaked. These tests deliberately kill servers and let the launcher relaunch,
 * and a spawn that dies before it publishes is never named there at all -- so
 * each run left detached servers behind that nothing would ever reap. They
 * orphan to init and sit for hours: seventeen of them, holding 117 MB, in
 * directories that had already been deleted out from under them.
 *
 * Found by this sandbox's home on the command line: vornd runs from its
 * Resources, and the session holder from its data directory.
 */
function sandboxServers(): number[] {
  try {
    const ps = spawnSync('ps', ['-eo', 'pid=,command='], { encoding: 'utf-8' })
    return ps.stdout
      .split('\n')
      .filter((line) => line.includes(path.basename(home)))
      .map((line) => Number(line.trim().split(/\s+/)[0]))
      .filter((pid) => Number.isInteger(pid) && pid > 0 && pid !== process.pid)
  } catch {
    return []
  }
}

afterEach(async () => {
  for (const pid of sandboxServers()) {
    try {
      process.kill(pid, 'SIGKILL')
    } catch {
      /* already gone */
    }
  }
  await sleep(300)
  fs.rmSync(home, { recursive: true, force: true })
})

describe.skipIf(!builtVornd || !builtSessiond || process.platform === 'win32')(
  'launching against a real machine',
  () => {
    it('starts a server, publishes where it is, and connects to it', async () => {
      const { launchServer, detachFromServer } = await import('../src/main/server/server-launcher')

      const bridge = await launchServer()

      expect(serverPid(), 'no server published its port').not.toBeNull()
      expect(fs.existsSync(tokenPath()), 'no credential was published').toBe(true)
      expect(bridge.isConnected, 'the bridge never connected').toBe(true)
      detachFromServer()
    }, 90_000)

    it('adopts the running server instead of starting a second', async () => {
      const first = await import('../src/main/server/server-launcher')
      await first.launchServer()
      const owner = serverPid()
      first.detachFromServer()

      // A fresh launcher, exactly as a second app launch would be.
      vi.resetModules()
      const second = await import('../src/main/server/server-launcher')
      const bridge = await second.launchServer()

      expect(serverPid(), 'a second server took over').toBe(owner)
      expect(bridge.isConnected, 'the second launch never connected').toBe(true)
      second.detachFromServer()
    }, 90_000)

    it('starts again after a quit', async () => {
      const first = await import('../src/main/server/server-launcher')
      await first.launchServer()
      const stopped = serverPid()
      await first.stopServer()
      for (let i = 0; i < 40 && alive(stopped as number); i++) await sleep(100)

      expect(alive(stopped as number), 'the server outlived the quit').toBe(false)
      expect(fs.existsSync(tokenPath()), 'the credential outlived its server').toBe(false)

      vi.resetModules()
      const second = await import('../src/main/server/server-launcher')
      const bridge = await second.launchServer()

      expect(bridge.isConnected, 'the app could not start again after a quit').toBe(true)
      expect(serverPid(), 'no new server').not.toBe(stopped)
      second.detachFromServer()
    }, 90_000)

    it('starts again after a crash, past the port and the credential left behind', async () => {
      // SIGKILL leaves the port record and credential beside a dead pid, which is no server.
      const first = await import('../src/main/server/server-launcher')
      await first.launchServer()
      const doomed = serverPid()
      expect(doomed, 'no server to kill').not.toBeNull()

      first.detachFromServer()
      process.kill(doomed as number, 'SIGKILL')
      for (let i = 0; i < 40 && alive(doomed as number); i++) await sleep(100)
      expect(alive(doomed as number), 'the server survived SIGKILL').toBe(false)
      expect(fs.existsSync(tokenPath()), 'a crash somehow cleaned up after itself').toBe(true)

      vi.resetModules()
      const second = await import('../src/main/server/server-launcher')
      const bridge = await second.launchServer()

      expect(bridge.isConnected, 'the app could not start again after a crash').toBe(true)
      expect(serverPid(), 'the corpse was never replaced').not.toBe(doomed)
      second.detachFromServer()
    }, 90_000)

    it('replaces its own server when that server crashes under it', async () => {
      const app = await import('../src/main/server/server-launcher')
      await app.launchServer()
      const doomed = serverPid()

      process.kill(doomed as number, 'SIGKILL')
      const deadline = Date.now() + 30_000
      while (Date.now() < deadline && (serverPid() === doomed || serverPid() === null)) {
        await sleep(250)
      }

      expect(serverPid(), 'no replacement was started').not.toBe(doomed)
      expect(serverPid(), 'no replacement was started').not.toBeNull()
      app.detachFromServer()
    }, 90_000)

    it('leaves the running server and its sessions alone when it adopts', async () => {
      const first = await import('../src/main/server/server-launcher')
      const bridge = await first.launchServer()
      let created: { id?: string } = {}
      for (let i = 0; i < 100 && !created.id; i++) {
        created = (await bridge.request('shell:create', os.tmpdir()).catch(() => ({}))) as {
          id?: string
        }
        if (!created.id) await sleep(100)
      }
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
  }
)
