import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { createInterface } from 'node:readline'
import log from '../logger'

/**
 * vornd, the native daemon, run in front of the server when its Experimental
 * switch is on.
 *
 * It forwards to the server everything it has not taken over, so the app
 * works the same through it as without it; with the Native server switch on
 * too, it answers the groups of calls that joined that switch itself.
 * Everything here is about starting it, knowing it is there, and getting
 * out of its way when it is not: a vornd that is missing, will not start or
 * goes away leaves the app talking to the server directly, never without one.
 */

/** The `protocol` vornd reports that this app knows how to use. */
export const VORND_PROTOCOL = 1

/**
 * Where vornd reads the desktop's launch token. A WebSocket that opens with it
 * is the desktop's, which vornd's size rule favours over a phone or a browser.
 * vornd takes it out of its environment before it starts anything.
 */
export const VORND_DESKTOP_TOKEN_ENV = 'VORND_DESKTOP_TOKEN'

/** How long vornd has to say where it listens. It binds before it says anything. */
export const VORND_START_TIMEOUT_MS = 5_000

const STDERR_KEPT = 5
const STOP_GRACE_MS = 2_000

/**
 * Where vornd is on this machine, or null when this build has none.
 *
 * Packaged, it ships in the app's resources. In dev it is wherever
 * `yarn build:core` copied it, or straight out of cargo's target directory for
 * someone who only ran `cargo build`.
 */
export function findVornd(
  where: { packaged: boolean; resourcesPath: string; repoRoot: string },
  platform: NodeJS.Platform = process.platform,
  exists: (file: string) => boolean = existsSync
): string | null {
  const name = platform === 'win32' ? 'vornd.exe' : 'vornd'
  const core = path.join(where.repoRoot, 'packages', 'core')
  const candidates = where.packaged
    ? [path.join(where.resourcesPath, 'vornd', name)]
    : [
        path.join(core, name),
        path.join(core, 'target', 'release', name),
        path.join(core, 'target', 'debug', name)
      ]
  return candidates.find((file) => exists(file)) ?? null
}

/**
 * The TCP port vornd should forward to, for a server the bridge reaches at
 * `target`. vornd only speaks TCP, so a server adopted through its socket is
 * reached by the port it also published, when it published one.
 */
export function upstreamPort(target: string, publishedPort: number | null): number | null {
  if (target.startsWith('ws+unix://')) return publishedPort
  try {
    const url = new URL(target)
    const port = Number(url.port)
    return Number.isInteger(port) && port > 0 ? port : null
  } catch {
    return null
  }
}

/** A running vornd. */
export interface Vornd {
  /** Where it listens, on 127.0.0.1. */
  port: number
  /** The server port it forwards to. */
  upstream: number
  /** Its channel for the server, when it holds sessions: the server starts terminals there. */
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
 * process holds, so it ends with the app however the app ends, even killed. It
 * never outlives the window the way the server does: it holds nothing worth
 * keeping. The session holder it keeps running does outlive it, and exits on
 * its own once it holds no sessions and no vornd comes back for it.
 */
export function startVornd(
  binary: string,
  upstream: number,
  options: {
    timeoutMs?: number
    spawnImpl?: typeof spawn
    /** The session holder for vornd to keep running, and the data directory it lives in. */
    sessiond?: { binary: string; home: string }
    /** The credential this app's own connection presents, so vornd can tell it is the desktop. */
    desktopToken?: string
    /**
     * Answer the groups of calls that have joined the Native server switch,
     * reading the server's database at `db` to tell a local project from a
     * remote one.
     */
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

    // Its log goes into the app's, and the last few lines say why when it
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
        fail(new Error(`vornd speaks protocol ${String(protocol)}, and this app ${VORND_PROTOCOL}`))
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
          // it, and unref'd: an app on its way out closes the pipe by exiting.
          child.stdin?.end()
          setTimeout(() => {
            if (exitDetail === null) child.kill()
          }, STOP_GRACE_MS).unref()
        }
      })
    })
  })
}
