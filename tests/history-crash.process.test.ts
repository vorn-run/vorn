import { describe, it, expect, afterEach, beforeAll } from 'vitest'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { spawn, spawnSync, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

spawnsRealServers()

/**
 * A real server, a real shell, and a kill that runs nothing.
 *
 * Every other test of this writes history by calling the writer and reads it by
 * calling recovery, in one process that politely tears itself down. None of them
 * can fail the way the thing they model fails. `SIGKILL` runs no handler, no
 * `finally`, no `process.on('exit')` -- whatever the kernel had flushed at that
 * instant is the entire input to the next start, and that is the input this
 * whole design exists for.
 *
 * So this one spawns the built server, asks it for a shell over the wire, waits
 * for a checkpoint to appear on disk, kills it dead, and starts another on the
 * same data directory. It also checks the wiring nothing else covers: that
 * `configureHistory` runs at startup, that `pty-manager` records on the flush,
 * and that recovery is reached in `startServer` at all.
 *
 * HOME is redirected as well as `--data-dir` for the reason `endpoint-race`
 * gives: `shutdown()` writes `~/.claude/settings.json` regardless of the data
 * dir, and without this these would uninstall the developer's own agent hooks.
 */

const SERVER = path.join(__dirname, '..', 'packages', 'server')
const ENTRY = path.join(SERVER, 'dist', 'index.cjs')

/** Distinct from the command that produces it, so an echoed prompt cannot pass. */
const MARKER = 'VORN-RESTORED-OK'
const COMMAND = `printf 'VORN-%s-OK\\n' RESTORED\r`

beforeAll(() => {
  // Always, not only when `dist/` is missing. The other process tests build on
  // absence and that is enough for them, because what they assert has been in
  // the bundle for a while. This one asserts a behaviour that is being written
  // right now, and a bundle from before the edit passes every step until the
  // last and then reports that history does not survive a crash -- which is true
  // of the binary and false of the branch. It costs about a second.
  const built = spawnSync('yarn', ['build'], { cwd: SERVER, stdio: 'inherit', shell: false })
  if (built.status !== 0) throw new Error('could not build the server bundle for this test')
  if (!fs.existsSync(ENTRY)) throw new Error(`the build produced no ${ENTRY}`)
}, 180_000)

let dir: string | null = null
const running: ChildProcess[] = []
let said = ''

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

function dataDir(): string {
  return path.join(dir as string, '.vorn')
}

function launch(): ChildProcess {
  fs.mkdirSync(dataDir(), { recursive: true, mode: 0o700 })
  const proc = spawn(process.execPath, [ENTRY, '--data-dir', dataDir(), '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
    env: {
      ...process.env,
      HOME: dir as string,
      USERPROFILE: dir as string,
      VORN_DATA_DIR: dataDir(),
      VORN_IDLE_TIMEOUT_MS: '600000'
    }
  })
  // Kept, not merely drained. An unread pipe fills at 64KB and blocks the
  // writer, and when one of these fails the server's own log is the only thing
  // that says why.
  proc.stdout?.on('data', (d: Buffer) => {
    said += String(d)
  })
  proc.stderr?.on('data', (d: Buffer) => {
    said += String(d)
  })
  running.push(proc)
  return proc
}

/** The tail of what the servers have said, for an assertion that needs to explain itself. */
function saidSoFar(): string {
  return said.split('\n').slice(-25).join('\n')
}

/**
 * An authenticated socket, once the server has published where and how.
 *
 * `notPort` is the whole reason this is more than four lines. A server that was
 * killed leaves its port file behind naming a port with nothing on it, so the
 * replacement's file is indistinguishable from the corpse's until it is
 * rewritten -- and connecting in between is an ECONNREFUSED that looks like a
 * server which failed to start.
 */
async function connect(notPort?: number, timeoutMs = 30_000): Promise<import('ws').WebSocket> {
  const portFile = path.join(dataDir(), 'ws-port')
  const tokenFile = path.join(dataDir(), 'local-token')
  const deadline = Date.now() + timeoutMs
  let port: number | undefined
  while (Date.now() < deadline) {
    if (fs.existsSync(portFile) && fs.existsSync(tokenFile)) {
      try {
        const found: number = JSON.parse(fs.readFileSync(portFile, 'utf-8')).port
        if (found !== notPort) {
          port = found
          break
        }
      } catch {
        /* being rewritten */
      }
    }
    await sleep(200)
  }
  if (port === undefined) throw new Error(`no server published a port. it said:\n${saidSoFar()}`)
  const token = fs.readFileSync(tokenFile, 'utf-8').trim()
  const { default: WebSocket } = await import('ws')
  const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
    headers: { Authorization: `Bearer ${token}` }
  })
  await new Promise((resolve, reject) => {
    ws.once('open', resolve)
    ws.once('error', reject)
  })
  return ws
}

let nextId = 0

function call<T>(ws: import('ws').WebSocket, method: string, params?: unknown): Promise<T> {
  const id = ++nextId
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${method} never answered`)), 20_000)
    const onMessage = (raw: Buffer): void => {
      const msg = JSON.parse(String(raw))
      if (msg.id !== id) return
      clearTimeout(timer)
      ws.off('message', onMessage)
      if (msg.error) reject(new Error(`${method}: ${JSON.stringify(msg.error)}`))
      else resolve(msg.result as T)
    }
    ws.on('message', onMessage)
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
}

function notify(ws: import('ws').WebSocket, method: string, params: unknown): void {
  ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
}

async function until(done: () => boolean | Promise<boolean>, within: number): Promise<boolean> {
  const deadline = Date.now() + within
  while (Date.now() < deadline) {
    if (await done()) return true
    await sleep(250)
  }
  return false
}

afterEach(async () => {
  said = ''
  for (const proc of running.splice(0)) {
    const pid = proc.pid
    if (pid === undefined || !alive(pid)) continue
    try {
      process.kill(pid, 'SIGKILL')
    } catch {
      /* already gone */
    }
  }
  await sleep(200)
  if (dir) fs.rmSync(dir, { recursive: true, force: true })
  dir = null
})

describe('a server that was killed rather than asked to stop', () => {
  it('gives its terminals back to the server that starts next', async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-crash-'))

    const first = launch()
    const pid = first.pid as number
    let ws = await connect()
    const wasOn: number = JSON.parse(fs.readFileSync(path.join(dataDir(), 'ws-port'), 'utf-8')).port

    const session = await call<{ id: string }>(ws, 'shell:create', dir)
    notify(ws, 'terminal:write', { id: session.id, data: COMMAND })

    // The shell has to actually run it before there is anything to checkpoint.
    const ran = await until(async () => {
      const { data } = await call<{ data: string }>(ws, 'terminal:readScrollback', {
        id: session.id
      })
      return data.includes(MARKER)
    }, 20_000)
    expect(ran, 'the shell never produced the output this test is about').toBe(true)

    // And the server has to have written one. Quiescence is two seconds in
    // production, so this is the wait that the interval buys.
    const history = path.join(dataDir(), 'history', encodeURIComponent(session.id))
    const wrote = await until(() => fs.existsSync(path.join(history, 'checkpoint.json')), 20_000)
    expect(wrote, `no checkpoint under ${history}. it said:\n${saidSoFar()}`).toBe(true)
    ws.close()

    // Nothing runs from here. No shutdown, no flush, no unlink.
    process.kill(pid, 'SIGKILL')
    expect(await until(() => !alive(pid), 10_000), 'the server survived a SIGKILL').toBe(true)

    launch()
    ws = await connect(wasOn)

    const previous = await call<Array<{ id: string }>>(ws, 'sessions:getPrevious')
    expect(previous.map((s) => s.id)).toContain(session.id)

    const { data } = await call<{ data: string }>(ws, 'terminal:readScrollback', { id: session.id })
    ws.close()

    // The whole claim, through a real process boundary: what the terminal
    // printed before the crash is what the next server can hand back.
    expect(data, 'the restored scrollback lost the output').toContain(MARKER)
  }, 120_000)

  it('keeps a terminal through a second restart, even after a new one is opened', async () => {
    // The failure this catches is quiet and takes two restarts to show.
    //
    // A save is a whole-table replace fed by the live session map, and after a
    // restart that map is empty. So opening a single new pane replaced the
    // table with that one row, every record from the previous run went, and on
    // the *next* start recovery judged their history unreachable -- correctly,
    // by its own rule -- and deleted it. Terminal history survived exactly one
    // restart, which is one fewer than the point of writing it down.
    //
    // One restart is not enough to see it: the first restore works. It is the
    // second that comes back empty.
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-crash-'))

    const first = launch()
    let pid = first.pid as number
    let ws = await connect()
    let wasOn: number = JSON.parse(fs.readFileSync(path.join(dataDir(), 'ws-port'), 'utf-8')).port

    const original = await call<{ id: string }>(ws, 'shell:create', dir)
    notify(ws, 'terminal:write', { id: original.id, data: COMMAND })
    const history = path.join(dataDir(), 'history', encodeURIComponent(original.id))
    expect(await until(() => fs.existsSync(path.join(history, 'checkpoint.json')), 20_000)).toBe(
      true
    )
    ws.close()
    process.kill(pid, 'SIGKILL')
    await until(() => !alive(pid), 10_000)

    // Second run: open something new, which is what used to erase the record.
    const second = launch()
    pid = second.pid as number
    ws = await connect(wasOn)
    wasOn = JSON.parse(fs.readFileSync(path.join(dataDir(), 'ws-port'), 'utf-8')).port
    expect(
      (await call<Array<{ id: string }>>(ws, 'sessions:getPrevious')).map((s) => s.id)
    ).toContain(original.id)

    await call<{ id: string }>(ws, 'shell:create', dir)
    // Past the 500ms save debounce, so the replace has certainly happened.
    await sleep(2_000)
    ws.close()
    process.kill(pid, 'SIGKILL')
    await until(() => !alive(pid), 10_000)

    // Third run: the original must still be nameable, and still have its history.
    launch()
    ws = await connect(wasOn)
    const previous = await call<Array<{ id: string }>>(ws, 'sessions:getPrevious')
    const { data } = await call<{ data: string }>(ws, 'terminal:readScrollback', {
      id: original.id
    })
    ws.close()

    expect(
      previous.map((s) => s.id),
      'the record was erased by the second run'
    ).toContain(original.id)
    expect(fs.existsSync(history), 'its history was swept as unreachable').toBe(true)
    expect(data, 'the terminal came back empty').toContain(MARKER)
  }, 180_000)

  it('leaves nothing behind for a terminal the next server cannot name', async () => {
    // The sweep, against real residue rather than a fabricated directory. A
    // crash leaves a directory per live terminal, and history keyed by an id no
    // session record mentions is history nothing can ever ask for.
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-crash-'))

    const first = launch()
    const pid = first.pid as number
    let ws = await connect()
    const wasOn: number = JSON.parse(fs.readFileSync(path.join(dataDir(), 'ws-port'), 'utf-8')).port
    const session = await call<{ id: string }>(ws, 'shell:create', dir)
    notify(ws, 'terminal:write', { id: session.id, data: COMMAND })

    const history = path.join(dataDir(), 'history')
    const wrote = await until(
      () => fs.existsSync(path.join(history, encodeURIComponent(session.id), 'checkpoint.json')),
      20_000
    )
    expect(wrote).toBe(true)
    ws.close()
    process.kill(pid, 'SIGKILL')
    await until(() => !alive(pid), 10_000)

    // A terminal from a run whose session record is long gone.
    const orphan = path.join(history, encodeURIComponent('a-session-nothing-remembers'))
    fs.mkdirSync(orphan, { recursive: true })
    fs.copyFileSync(
      path.join(history, encodeURIComponent(session.id), 'checkpoint.json'),
      path.join(orphan, 'checkpoint.json')
    )

    launch()
    ws = await connect(wasOn)
    await call(ws, 'sessions:getPrevious')
    ws.close()

    expect(fs.existsSync(orphan), 'history nothing can name was left on disk').toBe(false)
    expect(fs.existsSync(path.join(history, encodeURIComponent(session.id)))).toBe(true)
  }, 120_000)
})
