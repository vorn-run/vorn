import { describe, it, expect, afterEach, beforeAll } from 'vitest'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { spawn, spawnSync, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { EXIT_ENDPOINT_TAKEN } from '../packages/shared/src/protocol'

spawnsRealServers()

/**
 * Two real servers, one machine.
 *
 * The unit tests pin the decision against a fabricated incumbent. These pin the
 * thing the decision exists for and cannot be faked: that starting a second
 * server does not take the endpoint from the first, and that a client talking to
 * the first never notices the attempt.
 *
 * HOME is redirected as well as `--data-dir`, and that is not tidiness:
 * `shutdown()` calls `uninstallHooks()`, which writes `~/.claude/settings.json`
 * regardless of the data dir. Without it these would uninstall the developer's
 * own agent hooks on the way past.
 */

const SERVER = path.join(__dirname, '..', 'packages', 'server')
const ENTRY = path.join(SERVER, 'dist', 'index.cjs')

beforeAll(() => {
  // Built rather than skipped when absent: `dist/` is gitignored and `yarn test`
  // builds nothing, so a skip-if-missing gate means this never runs in CI —
  // green on the one claim the change is named for.
  if (fs.existsSync(ENTRY)) return
  const built = spawnSync('yarn', ['build'], { cwd: SERVER, stdio: 'inherit', shell: false })
  if (built.status !== 0) throw new Error('could not build the server bundle for this test')
}, 180_000)

let dir: string | null = null
const running: ChildProcess[] = []

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

function launch(): ChildProcess {
  const data = path.join(dir as string, '.vorn')
  fs.mkdirSync(data, { recursive: true, mode: 0o700 })
  const proc = spawn(process.execPath, [ENTRY, '--data-dir', data, '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
    env: {
      ...process.env,
      HOME: dir as string,
      USERPROFILE: dir as string,
      VORN_DATA_DIR: data,
      VORN_IDLE_TIMEOUT_MS: '600000'
    }
  })
  // Drained, never left. An unread pipe fills at 64KB and blocks the writer, and
  // this server logs a line per request.
  proc.stdout?.resume()
  proc.stderr?.resume()
  running.push(proc)
  return proc
}

const socketPath = (): string => path.join(dir as string, '.vorn', 'vorn.sock')

async function waitForEndpoint(timeoutMs = 20_000): Promise<string> {
  const socket = socketPath()
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      if (fs.lstatSync(socket).isSocket()) return socket
    } catch {
      /* not yet */
    }
    await sleep(200)
  }
  throw new Error('no endpoint appeared')
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

afterEach(() => {
  for (const proc of running.splice(0)) {
    if (!proc.pid) continue
    try {
      process.kill(-proc.pid, 'SIGKILL')
    } catch {
      try {
        process.kill(proc.pid, 'SIGKILL')
      } catch {
        /* already gone */
      }
    }
  }
  if (dir) fs.rmSync(dir, { recursive: true, force: true })
  dir = null
})

describe('two servers reaching for one machine', () => {
  it('leaves the endpoint with the first, and the second stands down', async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-race-'))
    const first = launch()
    const socket = await waitForEndpoint()
    const held = fs.lstatSync(socket).ino

    const second = launch()
    const code = await new Promise<number | null>((resolve) => {
      second.once('exit', (c) => resolve(c))
      setTimeout(() => resolve(null), 20_000)
    })

    // Not a crash: it did nothing wrong, it arrived second. Carrying on would
    // make it a second writer on one database, and `saveSessions` is a
    // whole-table replace -- the two would erase each other's sessions.
    expect(code, 'the second server did not stand down').toBe(EXIT_ENDPOINT_TAKEN)
    expect(fs.lstatSync(socket).ino, 'the endpoint changed hands').toBe(held)
    expect(await served(socket), 'nothing is serving the endpoint').toBe(true)
    expect(alive(first.pid as number), 'the first server died').toBe(true)
  }, 60_000)

  it('never drops a client held open across the attempt', async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-race-'))
    launch()
    const socket = await waitForEndpoint()

    const tokenFile = path.join(dir, '.vorn', 'local-token')
    for (let i = 0; i < 60 && !fs.existsSync(tokenFile); i++) await sleep(200)
    const token = fs.readFileSync(tokenFile, 'utf-8').trim()

    const { default: WebSocket } = await import('ws')
    const ws = new WebSocket(`ws+unix://${socket}:/ws`, {
      headers: { Authorization: `Bearer ${token}` }
    })
    await new Promise((r) => ws.once('open', r))

    const replies: number[] = []
    let failed = false
    ws.on('message', (raw: Buffer) => {
      try {
        const msg = JSON.parse(raw.toString())
        if (typeof msg.id === 'number') replies.push(msg.id)
      } catch {
        /* greeting frames */
      }
    })
    ws.on('close', () => {
      failed = true
    })
    ws.on('error', () => {
      failed = true
    })

    // Talk right through a second server's whole startup and claim attempt.
    launch()
    let sent = 0
    for (let i = 0; i < 16; i++) {
      ws.send(JSON.stringify({ jsonrpc: '2.0', id: ++sent, method: 'config:load' }))
      await sleep(400)
    }
    await sleep(1_000)
    ws.close()

    expect(failed, 'the socket dropped during the handover attempt').toBe(false)
    expect(replies.length, `only ${replies.length} of ${sent} calls answered`).toBe(sent)
  }, 60_000)

  it('lets a later start claim an endpoint its owner left behind on a clean exit', async () => {
    // The ordinary state of a machine between launches, and the one a review
    // found would have bricked the app: nothing removes the endpoint on shutdown,
    // so quitting Vorn leaves a socket file with nothing behind it. A launcher
    // that committed to that name and gave up when it did not answer would mean
    // quitting Vorn once made Vorn unable to start again.
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-race-'))
    const first = launch()
    const socket = await waitForEndpoint()
    const abandoned = fs.lstatSync(socket).ino

    process.kill(first.pid as number, 'SIGTERM')
    for (let i = 0; i < 40 && alive(first.pid as number); i++) await sleep(250)

    expect(fs.lstatSync(socket).isSocket(), 'the endpoint was removed on the way out').toBe(true)
    expect(await served(socket), 'something still serves a stopped server').toBe(false)
    expect(
      fs.existsSync(path.join(dir, '.vorn', 'local-token')),
      'the owner left its credential behind'
    ).toBe(false)

    const second = launch()
    const deadline = Date.now() + 20_000
    while (Date.now() < deadline && fs.lstatSync(socket).ino === abandoned) await sleep(250)

    expect(fs.lstatSync(socket).ino, 'the leftover was never replaced').not.toBe(abandoned)
    expect(await served(socket), 'the new server is not serving').toBe(true)
    expect(alive(second.pid as number), 'the new server stood down instead of starting').toBe(true)
  }, 60_000)

  it('lets the next start claim an endpoint whose owner was killed outright', async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-race-'))
    const first = launch()
    const socket = await waitForEndpoint()
    const abandoned = fs.lstatSync(socket).ino

    // SIGKILL leaves the socket file behind: nothing runs on the way out.
    process.kill(first.pid as number, 'SIGKILL')
    for (let i = 0; i < 40 && alive(first.pid as number); i++) await sleep(250)
    expect(fs.lstatSync(socket).isSocket(), 'the corpse was cleaned up somehow').toBe(true)
    expect(await served(socket), 'something is still serving a killed server').toBe(false)

    launch()
    const deadline = Date.now() + 20_000
    while (Date.now() < deadline && fs.lstatSync(socket).ino === abandoned) await sleep(250)

    expect(fs.lstatSync(socket).ino, 'the corpse was never replaced').not.toBe(abandoned)
    expect(await served(socket), 'the replacement is not serving').toBe(true)
  }, 60_000)
})
