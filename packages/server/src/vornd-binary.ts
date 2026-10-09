import { spawn } from 'node:child_process'
import { existsSync, realpathSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

/**
 * Where vornd, the Vorn server, is, and starting it in the foreground.
 *
 * `vorn server serve` runs it as the server for a data directory, from the
 * install this command belongs to: in the packaged app beside
 * `resources/server`, in a checkout where `yarn build:core` put it.
 */

/** Names a vornd elsewhere, for a build that is not beside this one. */
export const VORND_PATH_ENV = 'VORN_VORND_PATH'

export interface VorndBinaries {
  vornd: string
  sessiond: string | null
}

/** Every place vornd may be, best first. */
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

/** vornd and the session holder beside it, or null when this install has none. */
export function findVornd(
  dir: string = here(),
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

/** The web client's build, which the server serves under /app, when there is one. */
export function findWebClient(dir: string = here(), exists = existsSync): string | null {
  const candidates = [path.resolve(dir, '../web/dist'), path.resolve(dir, '../../web/dist')]
  return candidates.find((d) => exists(d)) ?? null
}

function here(): string {
  return typeof __dirname !== 'undefined' ? __dirname : path.dirname(process.argv[1])
}

/** How to run vornd as the server for `options`. */
export function serveArgs(
  binaries: VorndBinaries,
  options: { dataDir: string; port?: number; host?: string; web?: string | null }
): string[] {
  return [
    '--data-dir',
    options.dataDir,
    ...(binaries.sessiond ? ['--sessiond', binaries.sessiond] : []),
    ...(options.web ? ['--web', options.web] : []),
    ...(options.port !== undefined ? ['--port', String(options.port)] : []),
    ...(options.host ? ['--host', options.host] : [])
  ]
}

/**
 * Runs vornd as the server until it stops, with this process's terminal; a
 * stop asked of this process is passed on. Answers its exit code.
 */
export function runVornd(binaries: VorndBinaries, args: string[]): Promise<number> {
  return new Promise((resolve) => {
    const child = spawn(binaries.vornd, args, { stdio: 'inherit' })
    const pass = (signal: NodeJS.Signals): void => {
      child.kill(signal)
    }
    process.on('SIGINT', pass)
    process.on('SIGTERM', pass)
    const done = (code: number): void => {
      process.off('SIGINT', pass)
      process.off('SIGTERM', pass)
      resolve(code)
    }
    child.once('error', () => done(1))
    child.once('exit', (code, signal) => done(code ?? (signal ? 1 : 0)))
  })
}

/** Lets a run from source use the default data directory. */
export const ALLOW_DEFAULT_ENV = 'VORN_ALLOW_DEFAULT_DATA_DIR'

/**
 * Why `dir` may not be used, when it is the default data directory, `~/.vorn`,
 * and this command runs from source or under a test, not told otherwise: a
 * test that forgets its own directory must never reach a person's data.
 */
export function refusesDefault(
  dir: string,
  options: { debug?: boolean; home?: string; env?: NodeJS.ProcessEnv } = {}
): string | null {
  const env = options.env ?? process.env
  const debug =
    options.debug ?? (env.VITEST !== undefined || (process.argv[1] ?? '').endsWith('.ts'))
  if (!debug || env[ALLOW_DEFAULT_ENV] === '1') return null
  const real = (p: string): string => {
    try {
      return realpathSync(p)
    } catch {
      return path.resolve(p)
    }
  }
  const home = options.home ?? os.homedir()
  if (real(dir) !== real(path.join(home, '.vorn'))) return null
  return `a build run from source will not use the default data directory ${dir}; set ${ALLOW_DEFAULT_ENV}=1 to allow it`
}
