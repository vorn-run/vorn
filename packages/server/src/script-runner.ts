import { spawn } from 'node:child_process'
import { EventEmitter } from 'node:events'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { ScriptConfig, IPC } from '@vornrun/shared/types'
import { getLaunchDataDir, getLaunchEnv } from './process-utils'
import log from './logger'
import { vorndSessions } from './vornd-sessions'
import { comparePlan, runInVornd, SCRIPT_FILE } from './vornd-scripts'

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

  const cwd = config.cwd || config.projectPath || process.cwd()
  const mode = vorndSessions.scriptMode()
  if (mode === 'native') {
    const { scriptType, scriptContent } = config
    const script = {
      scriptType,
      scriptContent,
      cwd,
      args: config.args ?? [],
      ...(config.secretsFrom && { secretsFrom: config.secretsFrom })
    }
    const ran = await runInVornd(script, (data) => {
      if (runId) scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId, data })
    })
    if (ran) {
      const { output, exitCode } = ran
      if (runId) scriptRunnerEvents.emit(IPC.SCRIPT_EXIT, { runId, exitCode })
      return {
        success: exitCode === 0,
        output,
        // One stream: what it printed is the error, stderr and stdout alike.
        error: exitCode !== 0 ? output || `Exited with code ${exitCode}` : undefined,
        exitCode
      }
    }
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

    log.info(`[script-runner] executing ${config.scriptType} script in ${cwd}`)

    /** The script's own copy goes with it, so nothing is left behind after the answer. */
    const finish = async (result: ScriptExecutionResult): Promise<void> => {
      if (file) await rm(path.dirname(file), { recursive: true, force: true }).catch(() => {})
      resolve(result)
    }

    // Only this child sees them: the secrets are read here rather than held
    // anywhere the definition, a run record or an export could reach.
    const env = getLaunchEnv()
    if (mode === 'shadow') {
      const argv = [command, ...interpreter.args(file && SCRIPT_FILE), ...(config.args ?? [])]
      const script = { scriptType: config.scriptType, cwd, args: config.args ?? [] }
      comparePlan(script, [], { argv, cwd, envKeys: Object.keys(env) })
    }

    let child: ReturnType<typeof spawn>
    try {
      child = spawn(command, args, {
        cwd,
        // A script that came as a file has no use for stdin, and cannot block waiting on it.
        stdio: [file ? 'ignore' : 'pipe', 'pipe', 'pipe'],
        env,
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
