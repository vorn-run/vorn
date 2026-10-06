import { describe, it, expect, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawn, spawnSync, type ChildProcess } from 'node:child_process'
import WebSocket from 'ws'
import {
  BOOTSTRAP_ENV_VAR,
  ENDPOINT_FILENAME,
  HANDOFF_PROTOCOL_VERSION,
  type HandoffResult
} from '@vornrun/shared/protocol'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { isPidAlive } from '../packages/server/src/published-files'

/**
 * The whole thing, with two real servers, their vornds and a real shell.
 *
 * Every part has its own test; none of them touches the join, which is where it
 * actually goes wrong. What matters here is that a shell marked before the handoff
 * answers a server that did not exist when it was marked.
 */
spawnsRealServers()

const repoRoot = path.join(__dirname, '..')
const CREDENTIAL = 'handoff-test-credential'

const started: ChildProcess[] = []
const dirs: string[] = []
const transcript: string[] = []

afterEach((ctx) => {
  // The replacement writes to `server.log`: a handoff has no parent to pipe to.
  if (ctx.task.result?.state === 'fail') {
    for (const dir of dirs) {
      const at = path.join(dir, 'server.log')
      if (fs.existsSync(at)) transcript.push(`[heir]\n${fs.readFileSync(at, 'utf-8')}`)
    }
    process.stderr.write(transcript.join('') + '\n')
  }
  transcript.length = 0
  // The replacement was spawned by the outgoing server, not by this test, so
  // nothing else reaps it -- it is detached and holds a live shell.
  for (const dir of dirs) {
    try {
      spawnSync('pkill', ['-f', dir])
    } catch {
      // Nothing matched, which is the ordinary case for a handoff that declined.
    }
  }
  for (const child of started.splice(0)) {
    try {
      if (child.pid) process.kill(-child.pid, 'SIGKILL')
    } catch {
      // Already gone, which is what a successful handoff arranges.
    }
    try {
      child.kill('SIGKILL')
    } catch {
      // Same.
    }
  }
  // The session holder is detached and may still be writing as it goes, so a
  // directory it keeps filling is left for the OS rather than failing the test.
  for (const dir of dirs.splice(0)) {
    try {
      fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
    } catch (err) {
      process.stderr.write(`left ${dir}: ${(err as Error).message}\n`)
    }
  }
})

/** What a server is started with, and what it is handed for its replacement. */
function serverCommand(dataDir: string): {
  exec: string
  args: string[]
  env: Record<string, string>
} {
  return {
    exec: process.execPath,
    // `--import tsx`, never the binary: that re-executes in a child that inherits nothing.
    args: [
      '--import',
      'tsx',
      path.join(repoRoot, 'packages', 'server', 'src', 'index.ts'),
      '--data-dir',
      dataDir,
      '--port',
      '0'
    ],
    env: {
      [BOOTSTRAP_ENV_VAR]: CREDENTIAL,
      VORN_BUILD_CHANNEL: 'packaged',
      VORN_APP_VERSION: '9.9.9',
      NODE_ENV: 'test',
      // The servers log, for the transcript a failure prints.
      VITEST: ''
    }
  }
}

function startServerProcess(dataDir: string): ChildProcess {
  const command = serverCommand(dataDir)
  const child = spawn(command.exec, command.args, {
    cwd: repoRoot,
    env: { ...process.env, ...command.env },
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true
  })
  // A handoff goes wrong inside a process this test does not own.
  child.stdout?.on('data', (d) => transcript.push(`[donor] ${d}`))
  child.stderr?.on('data', (d) => transcript.push(`[donor] ${d}`))
  started.push(child)
  return child
}

/** The only honest "it is ready". */
async function endpointReady(dataDir: string, timeoutMs = 60_000): Promise<string> {
  const socket = path.join(dataDir, ENDPOINT_FILENAME)
  const until = Date.now() + timeoutMs
  while (Date.now() < until) {
    if (fs.existsSync(socket)) {
      const ws = await open(socket).catch(() => null)
      if (ws) {
        ws.close()
        return socket
      }
    }
    await new Promise((r) => setTimeout(r, 200))
  }
  throw new Error('the server never answered on its endpoint')
}

/** The server's endpoint socket, or vornd in front of it on a loopback port. */
function open(where: string | number): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const url = typeof where === 'number' ? `ws://127.0.0.1:${where}/ws` : `ws+unix://${where}:/ws`
    const ws = new WebSocket(url, {
      headers: { authorization: `Bearer ${CREDENTIAL}` }
    })
    const timer = setTimeout(() => {
      ws.close()
      reject(new Error('endpoint did not open'))
    }, 5_000)
    ws.once('open', () => {
      clearTimeout(timer)
      resolve(ws)
    })
    ws.once('error', (err) => {
      clearTimeout(timer)
      reject(err)
    })
  })
}

let nextId = 1
function call<T>(ws: WebSocket, method: string, params?: unknown, timeoutMs = 90_000): Promise<T> {
  const id = nextId++
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${method} timed out`)), timeoutMs)
    const onMessage = (raw: WebSocket.RawData): void => {
      // Terminal output shares the socket and is not JSON; a throw here would stall it.
      let frame: { id?: number; result?: unknown; error?: { message: string } }
      try {
        frame = JSON.parse(String(raw))
      } catch {
        return
      }
      if (frame.id !== id) return
      clearTimeout(timer)
      ws.off('message', onMessage)
      if (frame.error) reject(new Error(frame.error.message))
      else resolve(frame.result as T)
    }
    ws.on('message', onMessage)
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
}

/** vornd, which the server keeps in front of it and which serves every terminal. */
async function throughVornd(server: WebSocket): Promise<WebSocket> {
  const status = await call<{ state: string; port?: number }>(server, 'server:vornd')
  expect(status.state).toBe('on')
  return open(status.port as number)
}

/** Keystrokes have no reply, so a frame with an id is answered "method not found". */
function notify(ws: WebSocket, method: string, params: unknown): void {
  ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
}

async function waitFor(
  check: () => Promise<boolean>,
  what: string,
  timeoutMs = 30_000
): Promise<void> {
  const until = Date.now() + timeoutMs
  while (!(await check())) {
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 250))
  }
}

/** A fixed pause is a guess about how fast a shell starts, and wrong under load. */
async function waitForScreen(
  ws: WebSocket,
  id: string,
  match: RegExp,
  timeoutMs = 30_000
): Promise<string> {
  const until = Date.now() + timeoutMs
  let last = ''
  while (Date.now() < until) {
    last = (await call<{ data: string }>(ws, 'terminal:attach', { id })).data
    if (match.test(last)) return last
    await new Promise((r) => setTimeout(r, 250))
  }
  throw new Error(`never saw ${match} on screen; last 300: ${JSON.stringify(last.slice(-300))}`)
}

describe('a live handoff between two real servers', () => {
  it('hands the endpoint to a new server, and the shell in vornd carries on', async () => {
    const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-handoff-live-'))
    dirs.push(dataDir)
    fs.chmodSync(dataDir, 0o700)

    const donor = startServerProcess(dataDir)
    const socket = await endpointReady(dataDir)

    const server = await open(socket)
    const session = await call<{ id: string; pid: number }>(server, 'shell:create', repoRoot)
    // Answered before vornd has started it; the pid follows.
    await waitFor(async () => {
      const sessions = await call<Array<{ id: string; pid: number }>>(server, 'terminal:listActive')
      session.pid = sessions.find((s) => s.id === session.id)?.pid ?? 0
      return session.pid > 0
    }, 'the shell to start')
    const first = await throughVornd(server)

    // `$$` is the interactive shell's own pid: a shell session runs behind an
    // integration wrapper.
    // A pty accepts keystrokes before the shell reads them, and the line discipline
    // echoes what it never ran. Bracketed paste going on is the readiness signal,
    // not a `$`: shell integration paints the prompt through escape sequences.
    // eslint-disable-next-line no-control-regex
    await waitForScreen(first, session.id, /\x1b\[\?2004h/)
    notify(first, 'terminal:write', { id: session.id, data: 'MARK=$$; echo READY_$MARK\r' })
    const before = await waitForScreen(first, session.id, /READY_\d+/)
    const mark = /READY_(\d+)/.exec(before)?.[1]
    expect(mark).toBeDefined()

    const command = serverCommand(dataDir)
    const result = await call<HandoffResult>(server, 'server:handoff', {
      handoffVersion: HANDOFF_PROTOCOL_VERSION,
      exec: command.exec,
      args: command.args,
      env: command.env,
      cwd: repoRoot,
      appVersion: '9.9.9'
    })

    expect(result.kind).toBe('handed-over')
    if (result.kind !== 'handed-over') return
    // Nothing carried across: the shell is in vornd's session holder.
    expect(result.sessions).toBe(0)
    expect(result.pid).not.toBe(donor.pid)

    // The replacement claimed the same name, so the same path reaches it.
    await new Promise((r) => setTimeout(r, 1_000))
    const heir = await open(await endpointReady(dataDir))

    // The session is there, under the same id, with the same process behind it.
    await waitFor(async () => {
      const sessions = await call<Array<{ id: string; pid: number }>>(heir, 'terminal:listActive')
      return sessions.find((s) => s.id === session.id)?.pid === session.pid
    }, 'the heir to take the session on')
    const second = await throughVornd(heir)

    // `$MARK` was set before the handoff by a shell this server never started, so the
    // same number can only come from the original process.
    notify(second, 'terminal:write', { id: session.id, data: 'echo ALIVE_$MARK\r' })
    const after = await waitForScreen(second, session.id, new RegExp(`ALIVE_${mark}`))
    const attached = await call<{ live: boolean }>(second, 'terminal:attach', { id: session.id })
    expect(attached.live).toBe(true)
    // vornd kept the screen, so the pane looks continuous rather than cleared.
    expect(after).toContain(`READY_${mark}`)

    // The outgoing server left, and took nothing with it.
    await new Promise((r) => setTimeout(r, 1_000))
    expect(isPidAlive(donor.pid as number)).toBe(false)
    expect(isPidAlive(session.pid)).toBe(true)

    for (const ws of [first, second, server, heir]) ws.close()
  }, 180_000)
})
