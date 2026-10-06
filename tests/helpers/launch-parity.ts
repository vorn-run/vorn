import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { vi } from 'vitest'
import {
  buildAgentLaunchLine,
  buildHeadlessSpawnArgs
} from '../../packages/server/src/agent-launch'
import type { AgentCommandConfig, AiAgentType, CreateTerminalPayload } from '@vornrun/shared/types'

/**
 * The shared launch corpus (`fixtures/launch-lines.json`) and how one case
 * runs through the TypeScript and through `vorn_core.node`.
 *
 * What the TypeScript reads from its process -- the platform, the default
 * shell that decides quoting on Windows -- each case names, and the
 * TypeScript is made to see it: `process.platform` spied, and the
 * environment its default-shell lookup reads set so that lookup lands on the
 * case's shell. The native side is handed the same as arguments.
 */

export type Platform = 'posix' | 'win32'
export type Outcome<T> = T | { error: string }

export interface Headless {
  command: string
  args: string[]
  stdin?: string
}

export interface LaunchCase {
  name: string
  platform: Platform
  /** The default shell on Windows. */
  shell?: string
  payload: Partial<CreateTerminalPayload> & { agentType: string }
  commands?: Partial<Record<AiAgentType, AgentCommandConfig>>
  /** Files made executable in `{bin}`, which is PATH unless `path` or `env` says otherwise. */
  bin?: string[]
  path?: string
  env?: Record<string, string>
  line?: Outcome<string>
  headless?: Outcome<Headless>
}

export interface ShellCase {
  shell: string
  minimalPrompt: boolean
  env: Record<string, string>
  home: string
  setup?: { env: Record<string, string>; args: string[] | null }
}

export interface Corpus {
  about: string
  launch: LaunchCase[]
  tokens: { line: string; tokens?: { raw: string; value: string }[] | null }[]
  strip: { line: string; agent: AiAgentType; command: string; expected?: string }[]
  displayNames: { prompt: string; maxLen?: number; name?: string | null }[]
  resumeCwd: {
    session: { projectPath: string; worktreePath?: string; shellCwd?: string }
    dirs: string[]
    result?: { cwd: string; fellBackFrom?: string } | null
  }[]
  env: {
    source: [string, string][]
    passthrough: string[]
    dataDir: string | null
    safe?: [string, string][]
    launch?: [string, string][]
  }[]
  shell: ShellCase[]
}

export const CORPUS_PATH = path.resolve(__dirname, '../fixtures/launch-lines.json')

export function readCorpus(): Corpus {
  return JSON.parse(fs.readFileSync(CORPUS_PATH, 'utf-8')) as Corpus
}

/** The native exports these tests use; test-only, so not in `NativeCore`. */
export interface LaunchHost {
  platform: string
  shell: string
}

export interface NativeLaunch {
  launchLine(
    payload: object,
    commands: object,
    env: Record<string, string>,
    host: LaunchHost
  ): string
  headlessArgs(
    payload: object,
    commands: object,
    env: Record<string, string>,
    host: LaunchHost
  ): Headless
  launchTokens(line: string): { raw: string; value: string }[] | null
  shellSetup(
    shell: string,
    minimalPrompt: boolean,
    env: Record<string, string>,
    home: string,
    shimRoot: string
  ): { env: Record<string, string>; args?: string[] | null }
}

/** Every string in `value` with `{name}` replaced by `vars[name]`. */
export function fill<T>(value: T, vars: Record<string, string>): T {
  if (typeof value === 'string') {
    return value.replace(/\{(\w+)\}/g, (whole, name: string) => vars[name] ?? whole) as T
  }
  if (Array.isArray(value)) return value.map((v) => fill(v, vars)) as T
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, fill(v, vars)])) as T
  }
  return value
}

/** `{bin}`, with the case's executables in it. */
export function makeBin(names: string[]): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-launch-bin-'))
  for (const name of names) {
    const file = path.join(dir, name)
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, '#!/bin/sh\n', { mode: 0o755 })
  }
  return dir
}

/** The environment the case's launch reads its PATH from. */
export function launchEnv(c: LaunchCase): Record<string, string> {
  if (c.env) return c.env
  return { PATH: c.path ?? (c.bin ? '{bin}' : '/nonexistent-vorn-bin') }
}

export function hostOf(c: LaunchCase): LaunchHost {
  return { platform: c.platform === 'win32' ? 'win32' : 'linux', shell: c.shell ?? '/bin/zsh' }
}

/**
 * Runs `fn` with the TypeScript seeing the case's machine. On Windows its
 * default shell is COMSPEC once PATH holds no `pwsh.exe` and SystemRoot no
 * Windows PowerShell, which an empty directory and a missing one guarantee.
 */
export function asMachine<T>(host: LaunchHost, empty: string, fn: () => T): T {
  if (host.platform !== 'win32') return fn()
  const saved = {
    PATH: process.env.PATH,
    SystemRoot: process.env.SystemRoot,
    COMSPEC: process.env.COMSPEC
  }
  const platform = vi.spyOn(process, 'platform', 'get').mockReturnValue('win32')
  process.env.PATH = empty
  process.env.SystemRoot = path.join(empty, 'no-windows')
  process.env.COMSPEC = host.shell
  try {
    return fn()
  } finally {
    platform.mockRestore()
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }
}

export function outcome<T>(fn: () => T): Outcome<T> {
  try {
    return fn()
  } catch (err) {
    return { error: (err as Error).message }
  }
}

/** A headless spawn as JSON keeps it: no `stdin` key when there is none. */
export function plainHeadless(h: Outcome<Headless>): Outcome<Headless> {
  if ('error' in h) return h
  return h.stdin === undefined || h.stdin === null
    ? { command: h.command, args: h.args }
    : { command: h.command, args: h.args, stdin: h.stdin }
}

/**
 * Only the case's own commands: both sides fall back to their own defaults
 * for the rest, as the server does for an agent with no configuration.
 */
const commandsOf = (c: LaunchCase) => (c.commands ?? {}) as Record<AiAgentType, AgentCommandConfig>

/** The TypeScript's answers for a filled case. */
export function viaTypeScript(
  c: LaunchCase,
  empty: string
): { line: Outcome<string>; headless: Outcome<Headless> } {
  const host = hostOf(c)
  const payload = c.payload as CreateTerminalPayload
  const env = launchEnv(c)
  return asMachine(host, empty, () => ({
    line: outcome(() => buildAgentLaunchLine(payload, commandsOf(c), env)),
    headless: plainHeadless(outcome(() => buildHeadlessSpawnArgs(payload, commandsOf(c), env)))
  }))
}

/** The native answers for a filled case. */
export function viaNative(
  core: NativeLaunch,
  c: LaunchCase
): { line: Outcome<string>; headless: Outcome<Headless> } {
  const host = hostOf(c)
  const env = launchEnv(c)
  return {
    line: outcome(() => core.launchLine(c.payload, commandsOf(c), env, host)),
    headless: plainHeadless(outcome(() => core.headlessArgs(c.payload, commandsOf(c), env, host)))
  }
}

/** A small seeded generator (mulberry32), so a failing case can be replayed. */
export function seeded(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
