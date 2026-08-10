import { spawn } from 'node:child_process'
import { EventEmitter } from 'node:events'
import { ScriptConfig, IPC } from '@vornrun/shared/types'
import { getLaunchEnv } from './process-utils'
import log from './logger'

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
      env: getLaunchEnv(),
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
