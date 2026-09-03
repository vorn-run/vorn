import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import type { SdkConnectorAuth } from '@vornrun/shared/types'
import { resolveExecutable } from '../resolve-executable'
import { getSafeEnv, isAbsolutelyStrippedEnvName } from '../process-utils'
import log from '../logger'

/**
 * The rung a connector declared, acted on.
 *
 * A `cli` connector borrows a login that already works on the machine, which
 * means two questions the app can answer without asking anyone to paste
 * anything: is that tool signed in, and what does it hand over. Both were
 * previously answered for `gh` alone, in two places; this is the one
 * implementation every connector's declaration drives.
 */

const execFileAsync = promisify(execFile)

/** Long enough for a cold CLI, short enough that a form does not hang on it. */
const PROBE_TIMEOUT_MS = 5_000

/**
 * What asking the borrowed tool answered.
 *
 * `ok: null` is "nothing to ask" — a rung whose readiness this probe cannot
 * speak to — kept distinct from `false` so a key-rung connector is never
 * reported as signed out.
 */
export interface AuthProbeReport {
  ok: boolean | null
  /** Who the tool says you are, when its output names anyone. */
  identity?: string
  message?: string
  /** How to get the tool, when it is the tool that is missing. */
  installHint?: string
}

/** The impure edges, so a test can answer for a CLI that is not installed here. */
export interface AuthProbeDeps {
  resolve?: (name: string) => string | null
  run?: (
    file: string,
    args: string[],
    options: { timeout: number; env: Record<string, string> }
  ) => Promise<{ stdout: string; stderr: string }>
}

function deps(overrides: AuthProbeDeps): Required<AuthProbeDeps> {
  return {
    resolve: overrides.resolve ?? resolveExecutable,
    run:
      overrides.run ??
      ((file, args, options) =>
        execFileAsync(file, args, options) as Promise<{ stdout: string; stderr: string }>)
  }
}

/**
 * Where to get a tool, for the tools we can say something useful about.
 *
 * A generic sentence is the honest answer for anything else: guessing a
 * package name reads as authority and sends people to install the wrong thing.
 */
const KNOWN_HINTS: Record<string, () => string> = {
  gh: () => {
    switch (process.platform) {
      case 'darwin':
        return 'Install with Homebrew: `brew install gh`'
      case 'win32':
        return 'Install with winget: `winget install --id GitHub.cli` (or download from https://cli.github.com)'
      default:
        return 'Install from https://cli.github.com (Debian/Ubuntu: `sudo apt install gh`)'
    }
  },
  glab: () => {
    switch (process.platform) {
      case 'darwin':
        return 'Install with Homebrew: `brew install glab`'
      case 'win32':
        return 'Install with winget: `winget install --id GitLab.glab`'
      default:
        return 'Install from https://gitlab.com/gitlab-org/cli'
    }
  }
}

export function installHintFor(command: string): string {
  const known = KNOWN_HINTS[command]
  if (known) return known()
  return `Install \`${command}\` and make sure it is on your PATH.`
}

/**
 * The variables a connector is allowed to borrow from this machine.
 *
 * Naming a variable in `borrow.env` walks around `getSafeEnv()`, so the ask is
 * held to two things it cannot talk its way past: the connector must declare
 * the variable in its own manifest — a package can only reach for what it
 * openly reads — and no ask reaches a name that is stripped for everyone. A
 * name failing either is dropped and said out loud, because silently handing a
 * connector less than it asked for reads as the service being down.
 */
export function borrowableNames(
  auth: SdkConnectorAuth | undefined,
  declared: readonly string[]
): string[] {
  const asked = auth?.borrow?.env ?? []
  const declaredUpper = new Set(declared.map((name) => name.toUpperCase()))
  const allowed: string[] = []
  for (const name of asked) {
    if (isAbsolutelyStrippedEnvName(name)) {
      log.warn(`[auth] refused to borrow ${name}: it is stripped from every environment`)
      continue
    }
    if (!declaredUpper.has(name.toUpperCase())) {
      log.warn(`[auth] refused to borrow ${name}: the connector does not declare reading it`)
      continue
    }
    allowed.push(name)
  }
  return allowed
}

/**
 * The environment a borrowed tool is invoked with.
 *
 * `getSafeEnv()` strips tokens on purpose, so a connector cannot read
 * credentials meant for something else. Only the names it may borrow are put
 * back, and only when this machine actually has them.
 */
export function borrowedEnv(
  auth: SdkConnectorAuth | undefined,
  declared: readonly string[] = []
): Record<string, string> {
  const env = getSafeEnv()
  for (const name of borrowableNames(auth, declared)) {
    const value = process.env[name]
    if (value) env[name] = value
  }
  return env
}

/** Colour codes are for a terminal; an identity read out of them is not. */
const ANSI = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*m`, 'g')

/** Only a phrase that names somebody counts, never whatever came first. */
const NAMED = [/\baccount\s+(\S+)/i, /(?<!\bnot\s)(?<!could not\s)(?<!failed to\s)\bas\s+(\S+)/i]

/**
 * The account a tool names, when its output names one at all.
 *
 * Deliberately narrow. An earlier version fell back to the first line of
 * output, which is a guess about a stranger's format: `gh auth token` prints a
 * credential, `az account show` prints `{`, and both would have been rendered
 * as who you are. No phrase, no identity — the row still says you are signed
 * in, which is the part the probe actually established.
 */
export function identityFrom(output: string): string | undefined {
  const text = output.replace(ANSI, '')
  for (const pattern of NAMED) {
    const named = text.match(pattern)
    if (named) return named[1].replace(/[.,)]+$/, '')
  }
  return undefined
}

/** What went wrong, never what was printed: these streams can hold a token. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message.split('\n')[0] : String(error)
}

/**
 * What to tell someone to run, derived from what they declared asking.
 *
 * A status command is conventionally one word away from its login, so the
 * probe's own args name the fix; anything else falls back to the tool itself
 * rather than inventing a subcommand it may not have.
 */
export function signInCommand(auth: SdkConnectorAuth): string {
  const command = auth.probe?.command ?? ''
  const args = auth.probe?.args ?? []
  const last = args[args.length - 1]
  if (last === 'status') return [command, ...args.slice(0, -1), 'login'].join(' ')
  return command
}

/**
 * Ask the tool a `cli` connector borrows whether it is signed in.
 *
 * Answers `ok: null` for every other rung rather than guessing: a key is
 * checked by preflight against the service itself, and `none` has nothing to
 * check at all.
 */
export async function probeAuth(
  auth: SdkConnectorAuth | undefined,
  declared: readonly string[] = [],
  overrides: AuthProbeDeps = {}
): Promise<AuthProbeReport> {
  if (auth?.rung === 'none') {
    return { ok: true, message: 'Nothing to sign in to — installing it is the whole setup.' }
  }
  if (auth?.rung !== 'cli' || !auth.probe?.command) return { ok: null }

  const { resolve, run } = deps(overrides)
  const command = auth.probe.command
  const resolved = resolve(command)
  if (!resolved) {
    return {
      ok: false,
      message: `${command} is not installed or not on PATH.`,
      installHint: installHintFor(command)
    }
  }

  try {
    const result = await run(resolved, auth.probe.args ?? [], {
      timeout: PROBE_TIMEOUT_MS,
      env: borrowedEnv(auth, declared)
    })
    // Several CLIs write their status to stderr; both streams are the answer.
    const identity = identityFrom(`${result.stdout}\n${result.stderr}`)
    return { ok: true, ...(identity && { identity }) }
  } catch (error) {
    // A non-zero exit from a status command is the signed-out answer, not a
    // fault to surface: the message names the sign-in a person can go run.
    // Logged without either stream, which a status command can fill with one.
    log.warn(`[auth] ${command} reported no session: ${messageOf(error)}`)
    return {
      ok: false,
      message: `Sign in by running \`${signInCommand(auth)}\` in your terminal.`
    }
  }
}

/**
 * The variables a `cli` connector's child is started with.
 *
 * A name already set in the environment is passed through; anything left is
 * filled from the token command, run fresh at spawn so no credential is ever
 * written to the connection. A token command with nothing to fill is not run.
 */
export async function borrowedSecrets(
  auth: SdkConnectorAuth | undefined,
  declared: readonly string[] = [],
  overrides: AuthProbeDeps = {}
): Promise<Record<string, string>> {
  if (auth?.rung !== 'cli') return {}
  const names = borrowableNames(auth, declared)
  if (names.length === 0) return {}

  const borrowed: Record<string, string> = {}
  for (const name of names) {
    const value = process.env[name]
    if (value) borrowed[name] = value
  }

  // One variable receives the token, named by the connector or defaulting to
  // the first it asked for. The rest of `borrow.env` are pass-throughs — a host
  // or an account id filled with a token would authenticate against nothing.
  const target = auth.borrow?.tokenEnv ?? names[0]
  const tokenArgs = auth.borrow?.tokenArgs
  const command = auth.probe?.command
  const wanted = names.includes(target) && !borrowed[target]
  if (!wanted || !tokenArgs || tokenArgs.length === 0 || !command) return borrowed

  const { resolve, run } = deps(overrides)
  const resolved = resolve(command)
  if (!resolved) return borrowed

  try {
    const result = await run(resolved, tokenArgs, {
      timeout: PROBE_TIMEOUT_MS,
      env: borrowedEnv(auth, declared)
    })
    const token = result.stdout.trim()
    if (token) borrowed[target] = token
  } catch (error) {
    // A tool that will not hand over a token leaves the child to fail with its
    // own message, which names the service rather than this borrow. Said here
    // so it is not silent — the message only, since the streams held a token.
    log.warn(`[auth] ${command} could not hand over a token: ${messageOf(error)}`)
  }
  return borrowed
}
