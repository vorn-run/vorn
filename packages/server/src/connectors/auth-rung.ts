import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import type { AuthProbeReport, SdkConnectorAuth } from '@vornrun/shared/types'
import { declaredBorrows } from '@vornrun/shared/types'
import { resolveExecutable } from '../resolve-executable'
import { getSafeEnv, isAbsolutelyStrippedEnvName, isSensitiveEnvName } from '../process-utils'
import { stripAnsi } from '../ansi-strip'
import log from '../logger'

// The rung a connector declared, acted on: is the borrowed tool signed in, and what does it hand over.

const execFileAsync = promisify(execFile)

/** Long enough for a cold CLI, short enough that a form does not hang on it. */
const PROBE_TIMEOUT_MS = 5_000

export type { AuthProbeReport }

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

// Only the tools we can say something useful about; a generic sentence is the honest answer otherwise.
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

/** The borrow a connection may act on: which auth block, what it declares reading, and whether the app wrote it. */
export interface BorrowSource {
  auth: SdkConnectorAuth | undefined
  declared: readonly string[]
  trusted?: boolean
}

// Held to three things a manifest cannot talk past: declared, not stripped for everyone, not a credential name.
export function borrowableNames(source: BorrowSource): string[] {
  const allowed: string[] = []
  const declared = source.declared.map((name) => ({ name }))
  const honoured = declaredBorrows(source.auth, declared)
  const passthrough: ReadonlySet<string> = new Set()
  for (const asked of source.auth?.borrow?.env ?? []) {
    const name = honoured.find((entry) => entry.toUpperCase() === asked.toUpperCase())
    if (name === undefined) {
      log.warn(`[auth] refused to borrow ${asked}: the connector does not declare reading it`)
      continue
    }
    if (isAbsolutelyStrippedEnvName(name)) {
      log.warn(`[auth] refused to borrow ${name}: it is stripped from every environment`)
      continue
    }
    if (!source.trusted && isSensitiveEnvName(name, passthrough)) {
      log.warn(`[auth] refused to borrow ${name}: a connector may not ask for a credential by name`)
      continue
    }
    allowed.push(name)
  }
  return allowed
}

// The safe env plus only the names this source may borrow, and only when the machine has them.
export function borrowedEnv(
  source: BorrowSource,
  names = borrowableNames(source)
): Record<string, string> {
  const env = getSafeEnv()
  for (const name of names) {
    const value = process.env[name]
    if (value) env[name] = value
    else delete env[name]
  }
  return env
}

/** Only a phrase that names somebody counts, never whatever came first. */
const NAMED = [/\baccount\s+(\S+)/i, /\bas\s+(\S+)/i]

// The whole run-up to the phrase on its line says whether it is a claim or a refusal.
const NEGATION = /\b(not|cannot|can't|couldn't|failed|unable|denied|expired|invalid)\b/i

// Deliberately narrow: no named phrase, no identity, since a first line can be a token or a brace.
export function identityFrom(output: string): string | undefined {
  const text = stripAnsi(output)
  for (const pattern of NAMED) {
    const named = pattern.exec(text)
    if (!named) continue
    const lineStart = text.lastIndexOf('\n', named.index) + 1
    if (NEGATION.test(text.slice(lineStart, named.index))) continue
    return named[1].replace(/[.,)]+$/, '')
  }
  return undefined
}

/** What went wrong, never what was printed: these streams can hold a token. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message.split('\n')[0] : String(error)
}

// A status command is one word from its login; anything else names the tool rather than inventing a subcommand.
export function signInCommand(auth: SdkConnectorAuth): string {
  const command = auth.probe?.command ?? ''
  const args = auth.probe?.args ?? []
  const last = args[args.length - 1]
  if (last === 'status') return [command, ...args.slice(0, -1), 'login'].join(' ')
  return command
}

// Answers ok:null for every rung but cli: a key is checked by preflight, and none has nothing to check.
export async function probeAuth(
  source: BorrowSource,
  overrides: AuthProbeDeps = {}
): Promise<AuthProbeReport> {
  const { auth } = source
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
      env: borrowedEnv(source)
    })
    // Several CLIs write their status to stderr; both streams are the answer.
    const identity = identityFrom(`${result.stdout}\n${result.stderr}`)
    return { ok: true, ...(identity && { identity }) }
  } catch (error) {
    // A non-zero exit is the signed-out answer; logged without the streams, which can hold a token.
    log.warn(`[auth] ${command} reported no session: ${messageOf(error)}`)
    return {
      ok: false,
      message: `Sign in by running \`${signInCommand(auth)}\` in your terminal.`
    }
  }
}

// What a cli connector's child is started with: set names pass through, the token is fetched fresh at spawn.
export async function borrowedSecrets(
  source: BorrowSource,
  overrides: AuthProbeDeps = {}
): Promise<Record<string, string>> {
  const { auth } = source
  if (auth?.rung !== 'cli') return {}
  const names = borrowableNames(source)
  if (names.length === 0) return {}

  const borrowed: Record<string, string> = {}
  for (const name of names) {
    const value = process.env[name]
    if (value) borrowed[name] = value
  }

  // One variable receives the token; the rest of borrow.env are pass-throughs.
  const asked = (auth.borrow?.tokenEnv ?? names[0]).toUpperCase()
  const target = names.find((name) => name.toUpperCase() === asked)
  const tokenArgs = auth.borrow?.tokenArgs
  const command = auth.probe?.command
  const wanted = target !== undefined && !borrowed[target]
  if (!wanted || !tokenArgs || tokenArgs.length === 0 || !command) return borrowed

  const { resolve, run } = deps(overrides)
  const resolved = resolve(command)
  if (!resolved) return borrowed

  try {
    const result = await run(resolved, tokenArgs, {
      timeout: PROBE_TIMEOUT_MS,
      env: borrowedEnv(source, names)
    })
    const token = result.stdout.trim()
    if (token && target) borrowed[target] = token
  } catch (error) {
    // Said so it is not silent; the message only, since the streams held a token.
    log.warn(`[auth] ${command} could not hand over a token: ${messageOf(error)}`)
  }
  return borrowed
}
