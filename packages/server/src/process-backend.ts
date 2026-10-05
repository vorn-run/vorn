import * as pty from 'node-pty'
import { spawn as spawnChild, type SpawnOptions } from 'node:child_process'
import type { ManagedPty } from './handoff/adopted-pty'
import { configManager } from './config-manager'
import log from './logger'
import { vorndLink } from './vornd-link'
import { VorndChild, VorndProcess } from './vornd-process'

/**
 * Where a session's process runs: in this server through node-pty and
 * child_process, or in vornd's session holder.
 *
 * vornd is the backend when the Native daemon switch is on and a vornd is
 * linked to this server. With the switch off nothing here differs from calling
 * node-pty and `spawn` directly. With it on and no vornd linked (it is missing,
 * failed to start, or has not linked yet), the session is started here, and the
 * first such start says why in the log.
 */

let saidWhy: string | null = null

/** Test seam: the switch as a test sets it, or null to read it from the config. */
let switchOverride: boolean | null = null

export function setNativeDaemonOverride(on: boolean | null): void {
  switchOverride = on
}

/** Whether the Native daemon switch is on. */
export function nativeDaemonWanted(): boolean {
  if (switchOverride !== null) return switchOverride
  try {
    return configManager.loadConfig().defaults.experimental?.vornd === true
  } catch {
    return false
  }
}

/** Whether new sessions start in vornd's session holder. */
export function vorndBackend(): boolean {
  if (!nativeDaemonWanted()) return false
  if (vorndLink.linked()) {
    saidWhy = null
    return true
  }
  const why = 'the Native daemon switch is on but no vornd is linked to this server'
  if (saidWhy !== why) {
    saidWhy = why
    log.warn(`[backend] ${why}; starting sessions here instead`)
  }
  return false
}

/** What a terminal is started with, whichever backend starts it. */
export interface TerminalSpawn {
  /** The session id, which names it in the session holder too. */
  id: string
  file: string
  args: string[]
  cwd: string
  env: Record<string, string>
  cols: number
  rows: number
}

/** A terminal: in vornd's session holder when it is the backend, else a node-pty here. */
export function spawnTerminal(spec: TerminalSpawn): ManagedPty {
  if (vorndBackend()) {
    return VorndProcess.spawn({
      id: spec.id,
      argv: [spec.file, ...spec.args],
      cwd: spec.cwd,
      env: definedOnly(spec.env),
      cols: spec.cols,
      rows: spec.rows
    })
  }
  return spawnTerminalHere(spec)
}

/** A terminal in this process, through node-pty: the backend with the switch off. */
export function spawnTerminalHere(spec: Omit<TerminalSpawn, 'id'>): ManagedPty {
  return pty.spawn(spec.file, spec.args, {
    name: 'xterm-256color',
    cols: spec.cols,
    rows: spec.rows,
    cwd: spec.cwd,
    env: spec.env
  })
}

/** A piped agent, with its prompt on stdin when `stdin` is given. */
export interface PipedSpawn {
  id: string
  command: string
  args: string[]
  options: SpawnOptions & { cwd: string; env: NodeJS.ProcessEnv }
  stdin?: string
}

/** The part of a child process `headless-manager` uses, which both backends give. */
export interface AgentProcess {
  readonly pid?: number
  readonly stdin: {
    write(data: string): unknown
    end(): unknown
    on(event: 'error', listener: (err: Error) => void): unknown
  } | null
  readonly stdout: { on(event: 'data', listener: (chunk: Buffer) => void): unknown } | null
  readonly stderr: { on(event: 'data', listener: (chunk: Buffer) => void): unknown } | null
  on(event: 'exit', listener: (code: number | null) => void): unknown
  on(event: 'error', listener: (err: Error) => void): unknown
  kill(signal?: NodeJS.Signals | number): boolean
}

/**
 * A piped agent: in vornd's session holder when it is the backend, else a child
 * here. The vornd one gets its stdin at spawn and needs nothing written to it;
 * its `stdin` is null.
 */
export function spawnPiped(spec: PipedSpawn): AgentProcess {
  if (vorndBackend()) {
    // Where the child here would go through the platform's shell (Windows, for
    // `.cmd` shims, with the arguments already quoted for it), vornd runs the
    // line through it too.
    return new VorndChild({
      id: spec.id,
      argv: [spec.command, ...spec.args],
      shell: spec.options.shell === true,
      cwd: spec.options.cwd,
      env: definedOnly(spec.options.env),
      stdin: spec.stdin ?? ''
    })
  }
  return spawnPipedHere(spec.command, spec.args, spec.options)
}

/** A piped agent in this process: the backend with the switch off. */
export function spawnPipedHere(
  command: string,
  args: string[],
  options: SpawnOptions
): AgentProcess {
  return spawnChild(command, args, options)
}

/** The holder takes a whole environment of strings. */
function definedOnly(env: Record<string, string | undefined>): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [k, v] of Object.entries(env)) if (typeof v === 'string') out[k] = v
  return out
}
