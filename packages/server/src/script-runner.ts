import { spawn } from 'node:child_process'
import { EventEmitter } from 'node:events'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { ScriptConfig, IPC } from '@vornrun/shared/types'
import { getLaunchDataDir, getLaunchEnv } from './process-utils'
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

interface Interpreter {
  /** Set only where the program must be a file; the rest read it whole from stdin. */
  file?: string
  command: (isWin: boolean) => string
  args: (file: string) => string[]
}

// bash and pwsh read their program as they go, so they get a file; node and python read it whole from stdin and keep cwd resolution.
const INTERPRETERS: Record<string, Interpreter | undefined> = Object.assign(Object.create(null), {
  bash: {
    file: 'script.sh',
    command: (w: boolean) => (w ? 'bash.exe' : 'bash'),
    args: (f: string) => [f]
  },
  powershell: {
    file: 'script.ps1',
    command: () => 'pwsh',
    args: (f: string) => ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', f]
  },
  python: { command: (w: boolean) => (w ? 'python' : 'python3'), args: () => ['-'] },
  node: { command: () => 'node', args: () => ['-'] }
} satisfies Record<ScriptConfig['scriptType'], Interpreter>)

/** Under the data directory rather than a shared one, so nothing another account writes is in reach. */
async function scriptFileFor(name: string, contents: string): Promise<string> {
  const dataDir = getLaunchDataDir()
  let root = os.tmpdir()
  if (dataDir) {
    root = path.join(dataDir, 'scripts')
    await mkdir(root, { recursive: true, mode: 0o700 })
  }
  const dir = await mkdtemp(path.join(root, 'vorn-script-'))
  const file = path.join(dir, name)
  try {
    await writeFile(file, contents, { mode: 0o600 })
  } catch (err) {
    await rm(dir, { recursive: true, force: true }).catch(() => {})
    throw err
  }
  return file
}

/** How a script type is run here, or nothing when it is not one this host knows. */
export function interpreterFor(
  scriptType: string,
  isWin: boolean = process.platform === 'win32'
): Interpreter | undefined {
  const known = INTERPRETERS[scriptType]
  // On Windows bash.exe is usually the WSL launcher, which cannot open a Windows path, so bash keeps stdin there.
  if (known && scriptType === 'bash' && isWin)
    return { ...known, file: undefined, args: () => ['-s'] }
  return known
}

export async function executeScript(config: ScriptConfig): Promise<ScriptExecutionResult> {
  const interpreter = interpreterFor(config.scriptType)
  if (!interpreter) {
    return {
      success: false,
      output: '',
      error: `Unsupported script type: ${config.scriptType}`
    }
  }

  const runId = config.runId
  const fail = (message: string, output = ''): ScriptExecutionResult => {
    log.error(`[script-runner] ${message}`)
    if (runId) {
      scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId, data: `Error: ${message}\n` })
      scriptRunnerEvents.emit(IPC.SCRIPT_EXIT, { runId, exitCode: 1 })
    }
    return { success: false, output, error: message }
  }

  let file = ''
  if (interpreter.file) {
    try {
      file = await scriptFileFor(interpreter.file, config.scriptContent)
    } catch (err) {
      return fail(`could not write the script: ${err instanceof Error ? err.message : String(err)}`)
    }
  }

  return new Promise((resolve) => {
    const command = interpreter.command(process.platform === 'win32')
    const args = [...interpreter.args(file), ...(config.args ?? [])]

    const cwd = config.cwd || config.projectPath || process.cwd()

    log.info(`[script-runner] executing ${config.scriptType} script in ${cwd}`)

    /** The script's own copy goes with it, so nothing is left behind after the answer. */
    const finish = async (result: ScriptExecutionResult): Promise<void> => {
      if (file) await rm(path.dirname(file), { recursive: true, force: true }).catch(() => {})
      resolve(result)
    }

    let child: ReturnType<typeof spawn>
    try {
      child = spawn(command, args, {
        cwd,
        // A script that came as a file has no use for stdin, and cannot block waiting on it.
        stdio: [file ? 'ignore' : 'pipe', 'pipe', 'pipe'],
        // Only this child sees them: the secrets are read here rather than held
        // anywhere the definition, a run record or an export could reach.
        env: { ...getLaunchEnv(), ...secretEnvFor(config.secretsFrom) },
        windowsHide: true
      })
    } catch (err) {
      // A bad cwd throws here rather than answering on 'error', and the copy still has to go.
      void finish(fail(err instanceof Error ? err.message : String(err)))
      return
    }

    let stdout = ''
    let stderr = ''

    child.stdout?.on('data', (data) => {
      const chunk = String(data)
      stdout += chunk
      if (runId) scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId, data: chunk })
    })

    child.stderr?.on('data', (data) => {
      const chunk = String(data)
      stderr += chunk
      if (runId) scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId, data: chunk })
    })

    child.on('error', (err) => {
      void finish(fail(err.message, stdout))
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

    if (!file) {
      child.stdin?.on('error', () => {}) // prevent EPIPE if process exits early
      child.stdin?.write(config.scriptContent)
      child.stdin?.end()
    }
  })
}
