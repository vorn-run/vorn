import { spawn } from 'node:child_process'
import { EventEmitter } from 'node:events'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { ScriptConfig, IPC } from '@vornrun/shared/types'
import { getLaunchEnv } from './process-utils'
import { getDecryptedCreds } from './connectors/decrypted-creds'
import { SECRET_ENV_FIELD, isEnvName } from './connectors/keys'
import log from './logger'

/** `apiKey` names the variable `API_KEY`, the way a connector's own env does. */
function envNameFor(key: string): string {
  return key.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toUpperCase()
}

/**
 * The environment a step's named connection contributes.
 *
 * A connector's env blob already speaks in variable names, so it is spread as
 * written; a single-value field is named after itself. Nothing is read unless
 * a step asked for it by connection id.
 */
export function secretEnvFor(
  connectionId: string | undefined,
  lookup: (id: string) => Record<string, string> | undefined = getDecryptedCreds
): Record<string, string> {
  if (!connectionId) return {}
  const decrypted = lookup(connectionId)
  if (!decrypted) return {}
  const env: Record<string, string> = Object.create(null)
  for (const [key, value] of Object.entries(decrypted)) {
    if (key !== SECRET_ENV_FIELD) {
      env[envNameFor(key)] = value
      continue
    }
    try {
      const parsed: unknown = JSON.parse(value)
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) continue
      for (const [name, v] of Object.entries(parsed as Record<string, unknown>)) {
        if (typeof v === 'string' && isEnvName(name)) env[name] = v
      }
    } catch {
      // A blob this build cannot read contributes nothing, and the step runs
      // without it rather than failing on a value nobody can see.
    }
  }
  return { ...env }
}

export interface ScriptExecutionResult {
  success: boolean
  output: string
  error?: string
  exitCode?: number
}

export const scriptRunnerEvents = new EventEmitter()

/** The name a script of each type is written under, so its interpreter reads a file it recognises. */
const SCRIPT_FILE: Record<ScriptConfig['scriptType'], string> = {
  bash: 'script.sh',
  powershell: 'script.ps1',
  python: 'script.py',
  node: 'script.js'
}

/** How each interpreter is asked to run that file; user args follow it as `$1`, `argv` and the like. */
function invocationFor(
  scriptType: ScriptConfig['scriptType'],
  file: string
): { command: string; args: string[] } {
  const isWin = process.platform === 'win32'
  switch (scriptType) {
    case 'bash':
      return { command: isWin ? 'bash.exe' : 'bash', args: [file] }
    case 'powershell':
      return { command: 'pwsh', args: ['-File', file] }
    case 'python':
      return { command: isWin ? 'python' : 'python3', args: [file] }
    case 'node':
      return { command: 'node', args: [file] }
  }
}

export async function executeScript(config: ScriptConfig): Promise<ScriptExecutionResult> {
  const fileName = SCRIPT_FILE[config.scriptType]
  if (!fileName) {
    return {
      success: false,
      output: '',
      error: `Unsupported script type: ${config.scriptType}`
    }
  }

  const runId = config.runId
  let dir: string
  try {
    // A file rather than stdin, so a step that reads input does not swallow the rest of its own script.
    dir = await mkdtemp(path.join(os.tmpdir(), 'vorn-script-'))
    await writeFile(path.join(dir, fileName), config.scriptContent, { mode: 0o600 })
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    log.error(`[script-runner] could not write the script: ${message}`)
    if (runId) {
      scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId, data: `Error: ${message}\n` })
      scriptRunnerEvents.emit(IPC.SCRIPT_EXIT, { runId, exitCode: 1 })
    }
    return { success: false, output: '', error: message }
  }

  return new Promise((resolve) => {
    const { command, args } = invocationFor(config.scriptType, path.join(dir, fileName))
    if (config.args && config.args.length > 0) args.push(...config.args)

    const cwd = config.cwd || config.projectPath || process.cwd()

    log.info(`[script-runner] executing ${config.scriptType} script in ${cwd}`)

    /** The script's own copy goes with it, so nothing is left in the temp directory after the answer. */
    const finish = async (result: ScriptExecutionResult): Promise<void> => {
      await rm(dir, { recursive: true, force: true }).catch(() => {})
      resolve(result)
    }

    const child = spawn(command, args, {
      cwd,
      stdio: ['ignore', 'pipe', 'pipe'],
      // Only this child sees them: the secrets are read here rather than held
      // anywhere the definition, a run record or an export could reach.
      env: { ...getLaunchEnv(), ...secretEnvFor(config.secretsFrom) },
      windowsHide: true
    })

    let stdout = ''
    let stderr = ''

    child.stdout.on('data', (data) => {
      const chunk = data.toString()
      stdout += chunk
      if (runId) scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId, data: chunk })
    })

    child.stderr.on('data', (data) => {
      const chunk = data.toString()
      stderr += chunk
      if (runId) scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId, data: chunk })
    })

    child.on('error', (err) => {
      log.error(`[script-runner] spawn error: ${err.message}`)
      if (runId) {
        scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId, data: `Error: ${err.message}\n` })
        scriptRunnerEvents.emit(IPC.SCRIPT_EXIT, { runId, exitCode: 1 })
      }
      void finish({
        success: false,
        output: stdout,
        error: err.message
      })
    })

    child.on('close', (code) => {
      log.info(`[script-runner] exited with code ${code}`)
      if (runId) scriptRunnerEvents.emit(IPC.SCRIPT_EXIT, { runId, exitCode: code ?? 1 })
      void finish({
        success: code === 0,
        output: stdout,
        error: code !== 0 ? stderr || `Exited with code ${code}` : undefined,
        exitCode: code ?? undefined
      })
    })
  })
}
