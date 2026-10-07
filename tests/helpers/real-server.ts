import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import { expect } from 'vitest'
import { BOOTSTRAP_ENV_VAR, WS_PORT_FILENAME } from '../../packages/shared/src/protocol'
import type { RunDirs } from './sessions-parity'

/**
 * A real server started as a child, with the vornd it keeps in front of it,
 * for the tests that compare the Native server switch off against on. The
 * server runs on directories of its own under the temporary root: its home,
 * its data directory and a work directory for projects; a second server
 * started on the same directories is what a restart is.
 */

export const TEST_CREDENTIAL = 'native-server-sessions-credential'

/** Where a built vornd is, when there is one. */
export const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, '../../packages/core/vornd'),
  path.resolve(__dirname, '../../packages/core/target/release/vornd')
].find((p): p is string => !!p && fs.existsSync(p))

/** Whether these tests can run: vornd and the session holder are built, on a Unix. */
export const runnable =
  !!vornd &&
  process.platform !== 'win32' &&
  fs.existsSync(path.join(path.dirname(vornd), 'vorn-sessiond'))

export type Frame = Record<string, unknown>

const PATIENCE_MS = 30_000

export async function until(what: string, check: () => boolean | Promise<boolean>): Promise<void> {
  const start = Date.now()
  while (!(await check())) {
    if (Date.now() - start > PATIENCE_MS) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 50))
  }
}

/** A client of one server that keeps every notification it is told. */
export class Watcher {
  private next = 1
  readonly told: Array<{ method: string; params: unknown }> = []

  private constructor(private ws: WebSocket) {
    ws.on('message', (raw, isBinary) => {
      // A session's bytes, once attached, are not frames.
      if (isBinary) return
      const frame = JSON.parse(String(raw)) as { method?: string; params?: unknown }
      if (frame.method) this.told.push({ method: frame.method, params: frame.params })
    })
  }

  static open(port: number): Promise<Watcher> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { authorization: `Bearer ${TEST_CREDENTIAL}` }
    })
    return new Promise((resolve, reject) => {
      ws.once('open', () => resolve(new Watcher(ws)))
      ws.once('error', reject)
    })
  }

  call(method: string, params?: unknown): Promise<Frame> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 30_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(String(raw)) as Frame
        if (frame.id !== id) return
        this.ws.off('message', onMessage)
        clearTimeout(timer)
        resolve(frame)
      }
      this.ws.on('message', onMessage)
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  async result<T = unknown>(method: string, params?: unknown): Promise<T> {
    const frame = await this.call(method, params)
    if (frame.error) throw new Error(`${method}: ${JSON.stringify(frame.error)}`)
    return frame.result as T
  }

  /** Tells the server `method`, expecting no answer. */
  notify(method: string, params?: unknown): void {
    this.ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
  }

  /** What every client was told by `method`. */
  toldBy(method: string): unknown[] {
    return this.told.filter((t) => t.method === method).map((t) => t.params)
  }

  close(): void {
    this.ws.close()
  }
}

export interface RealServer {
  child: ChildProcess
  dirs: RunDirs
  port: number
  vornd: number
  log: string[]
}

export const realServers: RealServer[] = []

/** @param dirs An earlier server's directories, to start again on what it left. */
export async function startRealServer(nativeServer: boolean, dirs?: RunDirs): Promise<RealServer> {
  const made = (name: string): string =>
    fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), `vorn-parity-${name}-`)))
  // Its own home: the agents' hook settings and the hook endpoint are there.
  dirs ??= { home: made('home'), data: made('data'), work: made('work') }
  const log: string[] = []
  const child = spawn(
    process.execPath,
    [
      '--import',
      'tsx',
      path.join(__dirname, '..', '..', 'packages', 'server', 'src', 'index.ts'),
      '--data-dir',
      dirs.data,
      '--port',
      '0'
    ],
    {
      cwd: path.join(__dirname, '..', '..'),
      env: {
        ...process.env,
        HOME: dirs.home,
        [BOOTSTRAP_ENV_VAR]: TEST_CREDENTIAL,
        VORN_VORND_PATH: vornd!,
        VORN_NATIVE_SERVER: nativeServer ? '1' : '0',
        VORND_NATIVE_SERVER: '',
        VORND_GROUPS: '',
        NODE_ENV: 'test',
        VITEST: ''
      },
      stdio: ['ignore', 'pipe', 'pipe']
    }
  )
  child.stdout?.on('data', (d) => log.push(String(d)))
  child.stderr?.on('data', (d) => log.push(String(d)))
  const server: RealServer = { child, dirs, port: 0, vornd: 0, log }
  realServers.push(server)
  await until('the server to listen', () => {
    try {
      const record = JSON.parse(fs.readFileSync(path.join(dirs.data, WS_PORT_FILENAME), 'utf-8'))
      server.port = typeof record.port === 'number' ? (record.port as number) : 0
    } catch {
      server.port = 0
    }
    return server.port > 0
  })
  const direct = await Watcher.open(server.port)
  await until('vornd to start', async () => {
    const s = await direct.result<{ state: string; port?: number; nativeServer?: boolean }>(
      'server:vornd'
    )
    if (s.state !== 'on' || !s.port) return false
    expect(s.nativeServer).toBe(nativeServer)
    server.vornd = s.port
    return true
  })
  await until('the session holder, and with the switch the copy deciding', async () => {
    const res = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    const health = (await res.json()) as {
      sessiond?: { current?: { pid?: number } }
      registry?: { fed?: boolean; decides?: boolean }
    }
    if (!health.sessiond?.current?.pid) return false
    return !nativeServer || (health.registry?.fed === true && health.registry.decides === true)
  })
  direct.close()
  return server
}

/** @param keepHolder Leaves the session holder and its sessions running, for a server to start again on. */
export async function stopRealServer(server: RealServer, keepHolder = false): Promise<void> {
  const holder = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    .then((r) => r.json() as Promise<{ sessiond?: { current?: { pid?: number } } }>)
    .then((h) => h.sessiond?.current?.pid)
    .catch(() => undefined)
  if (server.child.exitCode === null) {
    const exited = new Promise((r) => server.child.once('exit', r))
    server.child.kill()
    await exited
  }
  // vornd finishes its stop after the server: its directories are left until it is gone.
  await until('vornd to stop', () =>
    fetch(`http://127.0.0.1:${server.vornd}/vornd/health`).then(
      () => false,
      () => true
    )
  )
  // The session holder outlives the server, by design, and its sessions with it.
  if (holder && !keepHolder) {
    try {
      process.kill(holder, 'SIGTERM')
    } catch {
      /* already gone */
    }
  }
}

/** A repository with one commit, the same commit on every run. */
export function repository(dir: string): void {
  fs.mkdirSync(dir, { recursive: true })
  fs.writeFileSync(path.join(dir, 'README'), 'parity\n')
  const env = {
    ...process.env,
    GIT_AUTHOR_NAME: 'Vorn',
    GIT_AUTHOR_EMAIL: 'vorn@example.invalid',
    GIT_COMMITTER_NAME: 'Vorn',
    GIT_COMMITTER_EMAIL: 'vorn@example.invalid',
    GIT_AUTHOR_DATE: '2026-01-01T00:00:00Z',
    GIT_COMMITTER_DATE: '2026-01-01T00:00:00Z'
  }
  const git = (...args: string[]): void => {
    execFileSync('git', args, { cwd: dir, env, stdio: 'ignore' })
  }
  git('init', '-q', '-b', 'main')
  git('add', 'README')
  git('commit', '-q', '-m', 'first')
}

/** A call's answer as the transcript keeps it: its result, or its error's message. */
export function answered(frame: Frame): unknown {
  if (frame.error) return { error: (frame.error as { message?: string }).message }
  return 'result' in frame ? { result: frame.result } : { void: true }
}

/** Removes every started server's directories, once all have stopped. */
export function removeRealServerDirs(): void {
  for (const s of realServers) {
    for (const dir of Object.values(s.dirs)) {
      fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
    }
  }
}
