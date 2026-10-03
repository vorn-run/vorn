import { execFileSync } from 'node:child_process'
import type { RemoteHost } from '@vornrun/shared/types'
import { experimentalFlag } from './experimental'
import log from './logger'
import { nativeBinary, type NativeCore } from './native-core'
import { getSafeEnv, sshExec, sshExecSync } from './process-utils'
import { resolveExecutable } from './resolve-executable'

/**
 * How a git command runs. Every function in `git-utils` goes through one of
 * these, so the switch between them is a single place.
 *
 * `js` is what shipped before: `execFileSync`, which holds the event loop for
 * as long as git takes, and every terminal, RPC and client with it. `native`
 * hands the command to the Rust core, which answers it on a thread of its own
 * and resolves the promise when it is done. It is opt-in through Settings ›
 * Experimental (`nativeGit`), or `VORN_GIT=native|js`, which wins over the
 * setting so a benchmark or a test can pin either.
 */
export type GitMode = 'js' | 'native'

export interface GitRunOptions {
  timeout: number
  /** Stdout past this is an error, as for `execFileSync`; its default when absent. */
  maxBuffer?: number
}

export interface GitRunner {
  mode: GitMode
  /** `git <args>` in `cwd`; resolves with stdout as git printed it, untrimmed. */
  local(args: string[], cwd: string, opts: GitRunOptions): Promise<string>
  /** A shell command on a remote host over SSH. */
  remote(host: RemoteHost, command: string, opts: { timeout: number }): Promise<string>
}

/** `execFileSync`'s own default, kept so the native path fails on the same diffs. */
export const DEFAULT_MAX_BUFFER = 1024 * 1024

// Resolve `git` from the login-shell PATH so packaged Electron finds the
// same binary the user would from their terminal (e.g. a newer Homebrew git
// rather than the Xcode stub). Falls back to the bare name so callers still
// work if resolution fails.
export function gitBin(): string {
  return resolveExecutable('git') ?? 'git'
}

const EXEC_OPTS = {
  encoding: 'utf-8' as const,
  stdio: ['pipe', 'pipe', 'pipe'] as ['pipe', 'pipe', 'pipe']
}

/**
 * The blocking path, unchanged. Wrapped in a promise so callers are the same
 * for both modes, but the work is done before the promise is returned, so the
 * loop is held exactly as long as it was.
 */
export const jsRunner: GitRunner = {
  mode: 'js',
  local(args, cwd, opts) {
    try {
      return Promise.resolve(
        execFileSync(gitBin(), args, {
          cwd,
          ...EXEC_OPTS,
          env: getSafeEnv(),
          timeout: opts.timeout,
          maxBuffer: opts.maxBuffer
        })
      )
    } catch (err) {
      return Promise.reject(err)
    }
  },
  remote(host, command, opts) {
    try {
      return Promise.resolve(sshExecSync(host, command, opts))
    } catch (err) {
      return Promise.reject(err)
    }
  }
}

type GitRun = NonNullable<NativeCore['gitRun']>

export function nativeRunner(gitRun: GitRun): GitRunner {
  return {
    mode: 'native',
    local: (args, cwd, opts) =>
      gitRun({
        bin: gitBin(),
        args,
        cwd,
        env: getSafeEnv(),
        timeoutMs: opts.timeout,
        maxBuffer: opts.maxBuffer ?? DEFAULT_MAX_BUFFER
      }),
    // SSH is a network wait rather than work, so it only has to leave the loop,
    // which an async child process already does.
    remote: (host, command, opts) => sshExec(host, command, opts)
  }
}

export function requestedGitMode(env: NodeJS.ProcessEnv = process.env): GitMode {
  const pinned = env.VORN_GIT?.trim().toLowerCase()
  if (pinned === 'native' || pinned === 'js') return pinned
  return experimentalFlag('nativeGit') ? 'native' : 'js'
}

let native: GitRunner | null | undefined
let warned = false

/**
 * The runner for this call. Read per call, so turning the switch takes effect on
 * the next git command without a restart. A native path that cannot load keeps
 * git on the JS path and says so once: the switch is an experiment, and a
 * missing or stale binary must not cost anyone their diff panel.
 */
export function gitRunner(): GitRunner {
  if (requestedGitMode() === 'js') return jsRunner
  if (native === undefined) {
    const loaded = nativeBinary()
    const gitRun = loaded.native?.gitRun
    native = typeof gitRun === 'function' ? nativeRunner(gitRun.bind(loaded.native)) : null
    if (!native && !warned) {
      warned = true
      log.warn(
        `[git] staying on js: ${loaded.error ?? 'the loaded vorn core has no gitRun(); rebuild it with `yarn build:core`'}`
      )
    }
  }
  return native ?? jsRunner
}

/** For tests: forget the loaded runner so the next call resolves it again. */
export function resetGitRunner(next?: GitRunner | null): void {
  native = next === null ? null : (next ?? undefined)
  warned = false
}
