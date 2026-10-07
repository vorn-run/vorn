import { randomUUID } from 'node:crypto'
import log from './logger'
import { vorndSessions, type VorndSessions } from './vornd-sessions'

/**
 * Project scripts run in vornd's session holder, with the Native server
 * switch on: vornd builds the process as `executeScript` does and starts it
 * on pipes under a session id chosen here, which this server follows as it
 * follows a headless agent, reading the output and the exit effect. In
 * shadow mode this server runs each script itself and sends vornd what it
 * started, to compare with what vornd would have.
 */

/** What stands for the script's file in a compared plan: each side writes its own. */
export const SCRIPT_FILE = '<script>'

/** A script as vornd is asked to run it: the cwd resolved, the secrets read. */
export interface VorndScript {
  scriptType: string
  scriptContent: string
  cwd: string
  args: string[]
  secretEnv: Record<string, string>
}

/** What a script started as, compared in shadow mode. */
export interface ScriptPlan {
  argv: string[]
  cwd: string
  envKeys: string[]
}

/** How a script vornd ran ended: everything it printed, both streams in one. */
export interface VorndScriptEnd {
  output: string
  exitCode: number
}

/** The scripts running in vornd, cancelled when this server shuts down. */
const running = new Set<string>()

/**
 * Run `script` in vornd, telling each piece of output to `onData`. Answers
 * null when vornd did not start it, so nothing ran and this server runs it.
 */
export async function runInVornd(
  script: VorndScript,
  onData: (data: string) => void,
  sessions: VorndSessions = vorndSessions
): Promise<VorndScriptEnd | null> {
  const id = `script-${randomUUID()}`
  // Followed before it is asked for, so no output or exit can come first.
  const pty = sessions.follow(id, true)
  let output = ''
  pty.onData((data) => {
    output += data
    onData(data)
  })
  const ended = new Promise<number>((resolve) => pty.onExit((e) => resolve(e.exitCode)))
  let started: { pid: number; epoch: number } | null
  try {
    started = await sessions.ask('vornd:script', { id, ...script })
  } catch (err) {
    log.warn({ err }, '[script-runner] vornd did not run the script; running it here')
    started = null
  }
  if (!started) {
    sessions.release(id)
    return null
  }
  running.add(id)
  pty.started(started.pid, started.epoch)
  try {
    const exitCode = await ended
    return { output, exitCode }
  } finally {
    running.delete(id)
  }
}

/** Send vornd what this server started for a script, to compare with what it would have. */
export function comparePlan(
  script: Omit<VorndScript, 'scriptContent' | 'secretEnv'>,
  secretKeys: string[],
  plan: ScriptPlan,
  sessions: VorndSessions = vorndSessions
): void {
  const { scriptType, cwd, args } = script
  const sorted = [...new Set(plan.envKeys)].sort()
  void sessions.tell('vornd:scriptPlan', {
    scriptType,
    cwd,
    args,
    secretKeys,
    plan: { ...plan, envKeys: sorted }
  })
}

/** Stop every script running in vornd, for a server shutting down. */
export function cancelScripts(sessions: VorndSessions = vorndSessions): void {
  for (const id of running) void sessions.tell('vornd:scriptCancel', { id })
}
