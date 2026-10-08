import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import { BOOTSTRAP_ENV_VAR, WS_PORT_FILENAME } from '../../packages/shared/src/protocol'
import type { RunDirs } from './sessions-parity'

/**
 * A real server started as a child, with the vornd it keeps in front of it.
 * The server runs on directories of its own under the temporary root: its
 * home, its data directory and a work directory for projects; a second
 * server started on the same directories is what a restart is.
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

  static open(port: number, credential = TEST_CREDENTIAL): Promise<Watcher> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { authorization: `Bearer ${credential}` }
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
      const onMessage = (raw: WebSocket.RawData, isBinary: boolean): void => {
        // An attached session's bytes: a throw here would stop the socket reading replies.
        if (isBinary) return
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

/**
 * @param dirs An earlier server's directories, to start again on what it left.
 * @param options.early Returns once vornd says where it is, as the app connects, without waiting for the holder.
 */
export async function startRealServer(
  dirs?: RunDirs,
  options: { early?: boolean } = {}
): Promise<RealServer> {
  const made = (name: string): string =>
    fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), `vorn-parity-${name}-`)))
  // Its own home: the agents' hook settings and the hook endpoint are there.
  dirs ??= { home: made('home'), data: made('data'), work: made('work') }
  // A killed server leaves its port behind.
  fs.rmSync(path.join(dirs.data, WS_PORT_FILENAME), { force: true })
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
        VORND_GROUPS: '',
        // Secrets go to a private file in the data directory, never this machine's keychain.
        VORND_KEYCHAIN: '0',
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
    const s = await direct.result<{ state: string; port?: number }>('server:vornd')
    if (s.state !== 'on' || !s.port) return false
    server.vornd = s.port
    return true
  })
  if (options.early) {
    direct.close()
    return server
  }
  await until('the session holder, and the copy deciding', async () => {
    const res = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    const health = (await res.json()) as {
      sessiond?: { current?: { pid?: number } }
      registry?: { fed?: boolean; decides?: boolean }
    }
    if (!health.sessiond?.current?.pid) return false
    return health.registry?.fed === true && health.registry.decides === true
  })
  direct.close()
  return server
}

/**
 * Waits for the vornd on `port` to finish stopping: it writes its session
 * records and last checkpoints after its server has exited.
 */
export async function vorndStopped(port: number): Promise<void> {
  if (!port) return
  await until('vornd to stop', () =>
    fetch(`http://127.0.0.1:${port}/vornd/health`).then(
      () => false,
      () => true
    )
  )
}

/** The pid vornd announced in `dataDir`, read before it stops and withdraws it. */
export function announcedVornd(dataDir: string): number | undefined {
  try {
    const said = JSON.parse(fs.readFileSync(path.join(dataDir, 'run', 'vornd-app'), 'utf8')) as {
      pid?: unknown
    }
    return typeof said.pid === 'number' ? said.pid : undefined
  } catch {
    return undefined
  }
}

/** The session holder's pid, as the vornd on `port` reports it. */
export function holderPid(port: number): Promise<number | undefined> {
  return fetch(`http://127.0.0.1:${port}/vornd/health`)
    .then((r) => r.json() as Promise<{ sessiond?: { current?: { pid?: number } } }>)
    .then((h) => h.sessiond?.current?.pid)
    .catch(() => undefined)
}

/** Waits for `pid` to exit: a process still exiting may still write to its directories. */
export async function processGone(pid: number | undefined): Promise<void> {
  if (!pid) return
  await until(`process ${pid} to exit`, () => {
    try {
      process.kill(pid, 0)
      return false
    } catch {
      return true
    }
  })
}

/** Ends the session holder `pid` and waits for it to exit. */
export async function stopHolder(pid: number | undefined): Promise<void> {
  if (!pid) return
  try {
    process.kill(pid, 'SIGTERM')
  } catch {
    return
  }
  await processGone(pid)
}

/** @param keepHolder Leaves the session holder and its sessions running, for a server to start again on. */
/** Stops it, and fails if its vornd handed the server a call it should have answered. */
export async function stopRealServer(server: RealServer, keepHolder = false): Promise<void> {
  const unexpected = await unexpectedForwards(server.vornd)
  await stopServerChild(server.child, server.vornd, server.dirs.data, keepHolder)
  if (Object.keys(unexpected).length > 0) {
    throw new Error(`vornd handed the server calls it should answer: ${JSON.stringify(unexpected)}`)
  }
}

/** The calls vornd forwarded that its list of still-forwarded calls does not allow. */
export async function unexpectedForwards(port: number): Promise<Record<string, number>> {
  if (!port) return {}
  try {
    const res = await fetch(`http://127.0.0.1:${port}/vornd/health`)
    const health = (await res.json()) as { unexpectedForwards?: Record<string, number> }
    return health.unexpectedForwards ?? {}
  } catch {
    // A vornd the test already stopped has nothing left to report.
    return {}
  }
}

/**
 * Stops a server started as a child on `dataDir`, its vornd on `vorndPort`
 * and, unless kept, its session holder, and waits for each to exit.
 */
export async function stopServerChild(
  child: ChildProcess,
  vorndPort: number,
  dataDir: string,
  keepHolder = false
): Promise<void> {
  const holder = await holderPid(vorndPort)
  const vorndPid = announcedVornd(dataDir)
  if (child.exitCode === null) {
    const exited = new Promise((r) => child.once('exit', r))
    child.kill()
    await exited
  }
  await vorndStopped(vorndPort)
  await processGone(vorndPid)
  // The session holder outlives the server, by design, and its sessions with it.
  if (!keepHolder) await stopHolder(holder)
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
