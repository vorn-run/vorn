import { execFile } from 'node:child_process'
import type { RemoteHost } from '@vornrun/shared/types'
import log from './logger'
import { nativeCore, type NativeCore } from './native-core'
import { getSafeEnv, sshExec } from './process-utils'
import { resolveExecutable } from './resolve-executable'

/**
 * How a git command runs. Every function in `git-utils` goes through one of
 * these, so the choice between them is a single place.
 *
 * `native` hands the command to the Rust core, which answers it on a thread of
 * its own (in-process through gix where it can) and resolves the promise when
 * it is done. `process` is for a server without the core: git as a child
 * process, which keeps the event loop free too, one process per command.
 */
export type GitMode = 'native' | 'process'

export interface GitRunOptions {
  timeout: number
  /** Stdout past this is an error; `DEFAULT_MAX_BUFFER` when absent. */
  maxBuffer?: number
}

export interface GitRunner {
  mode: GitMode
  /** `git <args>` in `cwd`; resolves with stdout as git printed it, untrimmed. */
  local(args: string[], cwd: string, opts: GitRunOptions): Promise<string>
  /** A shell command on a remote host over SSH. */
  remote(host: RemoteHost, command: string, opts: { timeout: number }): Promise<string>
}

/** Stdout past this is an error unless a caller asks for more: 1 MiB, as `execFileSync`'s default was. */
export const DEFAULT_MAX_BUFFER = 1024 * 1024

// Resolve `git` from the login-shell PATH so packaged Electron finds the
// same binary the user would from their terminal (e.g. a newer Homebrew git
// rather than the Xcode stub). Falls back to the bare name so callers still
// work if resolution fails.
export function gitBin(): string {
  return resolveExecutable('git') ?? 'git'
}

/**
 * Git as a child process, for a server without the core. Rejects with the
 * exit status and stderr on the error.
 */
export const processRunner: GitRunner = {
  mode: 'process',
  local(args, cwd, opts) {
    return new Promise((resolve, reject) => {
      execFile(
        gitBin(),
        args,
        {
          cwd,
          encoding: 'utf-8',
          env: getSafeEnv(),
          timeout: opts.timeout,
          maxBuffer: opts.maxBuffer ?? DEFAULT_MAX_BUFFER
        },
        (err, stdout, stderr) => (err ? reject(Object.assign(err, { stderr })) : resolve(stdout))
      )
    })
  },
  remote: (host, command, opts) => sshExec(host, command, opts)
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

let override: GitRunner | null = null
let built: { core: NativeCore; runner: GitRunner } | null = null
let warned = false

/**
 * The runner for this call: the core's, or a child process when the core did
 * not load or was built without git, which is said once in the log.
 */
export function gitRunner(): GitRunner {
  if (override) return override
  const core = nativeCore()
  const gitRun = core?.gitRun
  if (!core || typeof gitRun !== 'function') {
    warnOnce(core ? 'the loaded vorn core has no gitRun()' : 'the vorn core is not loaded')
    return processRunner
  }
  if (built?.core !== core) built = { core, runner: nativeRunner(gitRun.bind(core)) }
  return built.runner
}

function warnOnce(reason: string): void {
  if (warned) return
  warned = true
  log.warn(`[git] running git as a child process: ${reason}`)
}

/** For tests: use `next` for every git command, or go back to resolving. */
export function resetGitRunner(next?: GitRunner | null): void {
  override = next ?? null
  built = null
  warned = false
}
