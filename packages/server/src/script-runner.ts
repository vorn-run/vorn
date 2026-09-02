import { spawn } from 'node:child_process'
import { EventEmitter } from 'node:events'
import { ScriptConfig, IPC } from '@vornrun/shared/types'
import { getLaunchEnv } from './process-utils'
import { getDecryptedCreds } from './connectors/decrypted-creds'
import { SECRET_ENV_FIELD } from './connectors/keys'
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
  const env: Record<string, string> = {}
  for (const [key, value] of Object.entries(decrypted)) {
    if (key !== SECRET_ENV_FIELD) {
      env[envNameFor(key)] = value
      continue
    }
    try {
      const parsed: unknown = JSON.parse(value)
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) continue
      for (const [name, v] of Object.entries(parsed as Record<string, unknown>)) {
        if (typeof v === 'string') env[name] = v
      }
    } catch {
      // A blob this build cannot read contributes nothing, and the step runs
      // without it rather than failing on a value nobody can see.
    }
  }
  return env
}

export interface ScriptExecutionResult {
  success: boolean
  output: string
  error?: string
  exitCode?: number
}

export const scriptRunnerEvents = new EventEmitter()

export async function executeScript(config: ScriptConfig): Promise<ScriptExecutionResult> {
  return new Promise((resolve) => {
    let command: string
    let args: string[]

    const isWin = process.platform === 'win32'

    switch (config.scriptType) {
      case 'bash':
        command = isWin ? 'bash.exe' : 'bash'
        // Run from stdin
        args = ['-s']
        break
      case 'powershell':
        command = 'pwsh'
        args = ['-Command', '-']
        break
      case 'python':
        command = isWin ? 'python' : 'python3'
        args = ['-']
        break
      case 'node':
        command = 'node'
        args = ['-']
        break
      default:
        resolve({
          success: false,
          output: '',
          error: `Unsupported script type: ${config.scriptType}`
        })
        return
    }

    if (config.args && config.args.length > 0) {
      // Append user args if the interpreter supports it (bash -s arg1 arg2)
      args.push(...config.args)
    }

    const cwd = config.cwd || config.projectPath || process.cwd()
    const runId = config.runId

    log.info(`[script-runner] executing ${config.scriptType} script in ${cwd}`)

    const child = spawn(command, args, {
      cwd,
      stdio: ['pipe', 'pipe', 'pipe'],
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
      resolve({
        success: false,
        output: stdout,
        error: err.message
      })
    })

    child.on('close', (code) => {
      log.info(`[script-runner] exited with code ${code}`)
      if (runId) scriptRunnerEvents.emit(IPC.SCRIPT_EXIT, { runId, exitCode: code ?? 1 })
      resolve({
        success: code === 0,
        output: stdout,
        error: code !== 0 ? stderr || `Exited with code ${code}` : undefined,
        exitCode: code ?? undefined
      })
    })

    // Write script content to stdin
    child.stdin?.on('error', () => {}) // prevent EPIPE if process exits early
    child.stdin?.write(config.scriptContent)
    child.stdin?.end()
  })
}
