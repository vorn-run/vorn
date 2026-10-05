import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { createInterface } from 'node:readline'
import type { ExperimentalConfig, VorndStatus } from '@vornrun/shared/types'
import log from './logger'

/**
 * vornd, the native daemon, which this server starts and keeps running.
 *
 * vornd stands in front of the server for its clients and starts every
 * terminal and headless agent in its session holder, vorn-sessiond, so they
 * outlive this server and the app. It ends with this server; the holder, and
 * the sessions in it, do not.
 */

/** The `protocol` vornd reports that this server knows how to use. */
export const VORND_PROTOCOL = 1

/**
 * Where vornd reads the desktop's launch token. A WebSocket that opens with it
 * is the desktop's, which vornd's size rule favours over a phone or a browser.
 * vornd takes it out of its environment before it starts anything.
 */
export const VORND_DESKTOP_TOKEN_ENV = 'VORND_DESKTOP_TOKEN'

/** Names a vornd binary to use instead of looking for one; vorn-sessiond is beside it. */
export const VORND_PATH_ENV = 'VORN_VORND_PATH'

/** How long vornd has to say where it listens. It binds before it says anything. */
export const VORND_START_TIMEOUT_MS = 5_000

/**
 * Settings › Experimental › Native server: vornd answers the groups of calls it
 * has taken over from the server itself. `VORN_NATIVE_SERVER` overrides it (1
 * or 0), so a test run can put every call on either side.
 */
export function nativeServerSwitch(experimental: ExperimentalConfig | undefined): boolean {
  const forced = process.env.VORN_NATIVE_SERVER
  if (forced === '1') return true
  if (forced === '0') return false
  return experimental?.nativeServer === true
}

const STDERR_KEPT = 5
const STOP_GRACE_MS = 2_000

/** vornd and the session holder it keeps running, as this machine has them. */
export interface VorndBinaries {
  vornd: string
  /** Null when this build has no holder: vornd then forwards but starts no sessions. */
  sessiond: string | null
}

/**
 * Where vornd and vorn-sessiond may be, in the order they are tried.
 *
 * `dir` is the directory of the running server code: `packages/server/src`
 * under tsx, `packages/server/dist` for a built checkout, and
 * `resources/server` in a packaged app, where electron-builder puts both at
 * `resources/vornd`. In a checkout they are wherever `yarn build:core` copied
 * them, or straight out of cargo's target directory.
 */
export function vorndCandidates(
  dir: string,
  override?: string,
  platform: NodeJS.Platform = process.platform
): string[] {
  const name = platform === 'win32' ? 'vornd.exe' : 'vornd'
  const core = path.join(dir, '..', '..', 'core')
  const candidates = [
    path.join(dir, '..', 'vornd', name),
    path.join(core, name),
    path.join(core, 'target', 'release', name),
    path.join(core, 'target', 'debug', name)
  ]
  return override ? [path.resolve(override), ...candidates] : candidates
}

/** vornd and its holder, or null when this build has no vornd. The holder sits beside vornd. */
export function findVornd(
  dir: string = serverDir(),
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  exists: (file: string) => boolean = existsSync
): VorndBinaries | null {
  const vornd = vorndCandidates(dir, env[VORND_PATH_ENV], platform).find((file) => exists(file))
  if (!vornd) return null
  const sessiond = path.join(
    path.dirname(vornd),
    platform === 'win32' ? 'vorn-sessiond.exe' : 'vorn-sessiond'
  )
  return { vornd, sessiond: exists(sessiond) ? sessiond : null }
}

// Same resolution as index.ts: __dirname in the CJS bundle, the entry script's
// directory under tsx.
function serverDir(): string {
  return typeof __dirname !== 'undefined' ? __dirname : path.dirname(process.argv[1])
}

/** A running vornd. */
export interface Vornd {
  /** Where it listens for clients, on 127.0.0.1. */
  port: number
  /** The server port it forwards to. */
  upstream: number
  /** Its channel for this server, when it holds sessions: terminals start there. */
  app?: string
  /** Called once if it exits, with why, unless `stop` was called first. */
  onExit(listener: (detail: string) => void): void
  stop(): void
}

/**
 * Start vornd in front of the server on `upstream` and wait until it says where
 * it listens.
 *
 * It is started with `--exit-with-stdin` and its stdin is a pipe only this
 * process holds, so it ends with the server however the server ends, even
 * killed. The session holder it keeps running outlives it, and exits on its
 * own once it holds no sessions and no vornd comes back for it.
 */
export function startVornd(
  binary: string,
  upstream: number,
  options: {
    timeoutMs?: number
    spawnImpl?: typeof spawn
    /** The session holder for vornd to keep running, and the data directory it lives in. */
    sessiond?: { binary: string; home: string }
    /** The credential the desktop's connection presents, so vornd can tell it is the desktop. */
    desktopToken?: string
    /** Answer the groups vornd has taken over itself, reading the database at `db`. */
    nativeServer?: { db: string }
  } = {}
): Promise<Vornd> {
  const run = options.spawnImpl ?? spawn
  const timeoutMs = options.timeoutMs ?? VORND_START_TIMEOUT_MS
  return new Promise((resolve, reject) => {
    let child: ChildProcess
    try {
      const args = ['--upstream', `127.0.0.1:${upstream}`, '--exit-with-stdin']
      if (options.sessiond) {
        args.push('--sessiond', options.sessiond.binary, '--home', options.sessiond.home)
      }
      if (options.nativeServer) args.push('--native-server', '--db', options.nativeServer.db)
      child = run(binary, args, {
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
        // In the environment rather than the arguments, which anyone on the
        // machine can list.
        ...(options.desktopToken
          ? { env: { ...process.env, [VORND_DESKTOP_TOKEN_ENV]: options.desktopToken } }
          : {})
      })
    } catch (err) {
      reject(new Error(`could not start vornd: ${(err as Error).message}`))
      return
    }

    // Its log goes into the server's, and the last few lines say why when it
    // fails to start.
    const stderr: string[] = []
    if (child.stderr) {
      createInterface({ input: child.stderr }).on('line', (line) => {
        stderr.push(line)
        if (stderr.length > STDERR_KEPT) stderr.shift()
        log.info(`[vornd] ${line}`)
      })
    }
    const why = (detail: string): Error =>
      new Error(stderr.length ? `${detail}: ${stderr.join(' | ')}` : detail)

    let settled = false
    let stopped = false
    let exitListener: ((detail: string) => void) | null = null
    let exitDetail: string | null = null

    const fail = (err: Error): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      stopped = true
      child.stdin?.end()
      child.kill()
      reject(err)
    }
    const timer = setTimeout(
      () => fail(why(`vornd did not say where it listens within ${timeoutMs}ms`)),
      timeoutMs
    )

    child.once('error', (err) => fail(new Error(`could not start vornd: ${err.message}`)))
    child.once('exit', (code, signal) => {
      exitDetail = `code=${code}, signal=${signal}`
      if (!settled) {
        fail(why(`vornd exited before it was listening (${exitDetail})`))
        return
      }
      if (!stopped) exitListener?.(exitDetail)
    })
    // A broken pipe on the way out is how a stopped vornd says goodbye.
    child.stdin?.on('error', () => {})

    if (!child.stdout) {
      fail(new Error('vornd has no stdout to read its port from'))
      return
    }
    const lines = createInterface({ input: child.stdout })
    lines.once('line', (line) => {
      lines.close()
      let reported: { port?: unknown; protocol?: unknown; app?: unknown }
      try {
        reported = JSON.parse(line) ?? {}
      } catch {
        fail(new Error(`vornd said something other than where it listens: ${line}`))
        return
      }
      const { port, protocol, app } = reported
      if (typeof port !== 'number' || !Number.isInteger(port) || port <= 0) {
        fail(new Error(`vornd reported no usable port: ${line}`))
        return
      }
      if (protocol !== VORND_PROTOCOL) {
        fail(
          new Error(`vornd speaks protocol ${String(protocol)}, and this server ${VORND_PROTOCOL}`)
        )
        return
      }
      settled = true
      clearTimeout(timer)
      resolve({
        port,
        upstream,
        ...(typeof app === 'string' && app ? { app } : {}),
        onExit(listener) {
          if (stopped) return
          if (exitDetail !== null) listener(exitDetail)
          else exitListener = listener
        },
        stop() {
          if (stopped) return
          stopped = true
          // Closing stdin is enough. The signal is for a vornd too busy to see
          // it, and unref'd: a server on its way out closes the pipe by exiting.
          child.stdin?.end()
          setTimeout(() => {
            if (exitDetail === null) child.kill()
          }, STOP_GRACE_MS).unref()
        }
      })
    })
  })
}

/** How long after an unexpected exit vornd is started again, doubling to the cap. */
const RESTART_FIRST_MS = 500
const RESTART_MAX_MS = 30_000
/** An exit after this long up starts the backoff over. */
const RESTART_RESET_MS = 60_000

/** What the keeper needs from around it; injected so its decisions can be tested. */
export interface KeeperDeps {
  find?: () => VorndBinaries | null
  start?: typeof startVornd
  /** Called with vornd's channel for this server each time one starts. */
  connect: (endpoint: string) => Promise<boolean>
  desktopToken?: () => string | null
  /** Whether vornd should answer the groups it has taken over; read at each start. */
  nativeServer?: () => boolean
  now?: () => number
}

/**
 * Keeps one vornd running in front of this server for as long as the server
 * runs, starting it again when it exits on its own.
 */
export class VorndKeeper {
  private running: Vornd | null = null
  private status: VorndStatus = { state: 'off' }
  private stopped = false
  private restartTimer: ReturnType<typeof setTimeout> | undefined
  private restartDelay = RESTART_FIRST_MS
  private upstream = 0
  private home = ''
  private inFlight: Promise<void> | null = null
  private readonly find: () => VorndBinaries | null
  private readonly start: typeof startVornd
  private readonly now: () => number

  constructor(private readonly deps: KeeperDeps) {
    this.find = deps.find ?? (() => findVornd())
    this.start = deps.start ?? startVornd
    this.now = deps.now ?? Date.now
  }

  /** Whether vornd is up, its port, or why it is not. */
  get state(): VorndStatus {
    return this.status
  }

  /** Where clients reach the server through vornd, or null while it is not up. */
  get port(): number | null {
    return this.running?.port ?? null
  }

  /** Start vornd in front of the server on `upstream`, keeping its holder in `home`. */
  launch(upstream: number, home: string): Promise<void> {
    this.upstream = upstream
    this.home = home
    this.stopped = false
    return this.run()
  }

  /** Whether a start is in flight. */
  get starting(): boolean {
    return this.inFlight !== null
  }

  /** A start in flight, or a resolved promise when none is. */
  ready(): Promise<void> {
    return this.inFlight ?? Promise.resolve()
  }

  private run(): Promise<void> {
    if (this.inFlight) return this.inFlight
    const attempt = this.startOnce().finally(() => {
      if (this.inFlight === attempt) this.inFlight = null
    })
    this.inFlight = attempt
    return attempt
  }

  private async startOnce(): Promise<void> {
    const binaries = this.find()
    if (!binaries) {
      this.fail('vornd is not in this build; build it with `yarn build:core`')
      return
    }
    // Without a holder vornd still forwards, and says why there are no sessions.
    if (!binaries.sessiond) log.error('[vornd] vorn-sessiond is not in this build')
    let started: Vornd
    const upAt = this.now()
    const nativeServer = this.deps.nativeServer?.() ?? false
    try {
      started = await this.start(binaries.vornd, this.upstream, {
        sessiond: binaries.sessiond ? { binary: binaries.sessiond, home: this.home } : undefined,
        desktopToken: this.deps.desktopToken?.() ?? undefined,
        // The server's database, in the same data directory as the holder.
        nativeServer: nativeServer ? { db: path.join(this.home, 'vorn.db') } : undefined
      })
    } catch (err) {
      this.fail((err as Error).message)
      this.scheduleRestart()
      return
    }
    if (this.stopped) {
      started.stop()
      return
    }
    this.running = started
    this.status = { state: 'on', port: started.port, nativeServer }
    log.info(`[vornd] forwarding to the server on ${this.upstream} from ${started.port}`)
    started.onExit((detail) => {
      if (this.running !== started) return
      this.running = null
      this.fail(`vornd exited (${detail})`)
      if (this.now() - upAt > RESTART_RESET_MS) this.restartDelay = RESTART_FIRST_MS
      this.scheduleRestart()
    })
    if (started.app) {
      const connected = await this.deps.connect(started.app)
      if (!connected) log.error('[vornd] could not reach the channel vornd opened for this server')
    } else {
      log.error('[vornd] vornd opened no channel for this server; no terminal can start')
    }
  }

  private fail(detail: string): void {
    this.status = { state: 'failed', detail }
    log.error(`[vornd] ${detail}`)
  }

  private scheduleRestart(): void {
    if (this.stopped || this.restartTimer) return
    const delay = this.restartDelay
    this.restartDelay = Math.min(this.restartDelay * 2, RESTART_MAX_MS)
    this.restartTimer = setTimeout(() => {
      this.restartTimer = undefined
      if (!this.stopped) void this.run()
    }, delay)
    this.restartTimer.unref?.()
  }

  /** Stop vornd, for good: the holder and its sessions carry on. */
  stop(): void {
    this.stopped = true
    clearTimeout(this.restartTimer)
    this.restartTimer = undefined
    const running = this.running
    this.running = null
    running?.stop()
    this.status = { state: 'off' }
  }
}
