import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { existsSync, mkdtempSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  executeScript,
  interpreterFor,
  scriptRunnerEvents
} from '../packages/server/src/script-runner'
import { setLaunchDataDir } from '../packages/server/src/process-utils'
import { IPC } from '@vornrun/shared/types'

/** Where the server would keep its files, so a script's own copy lands where the real one does. */
const dataDir = mkdtempSync(path.join(os.tmpdir(), 'vorn-script-test-'))

/** A project with a dependency, to prove a node script still resolves against the directory it runs in. */
const project = path.join(dataDir, 'project')

beforeAll(() => {
  setLaunchDataDir(dataDir)
  const module = path.join(project, 'node_modules', 'vorn-fixture')
  mkdirSync(module, { recursive: true })
  writeFileSync(path.join(module, 'package.json'), '{"name":"vorn-fixture","main":"index.js"}')
  writeFileSync(path.join(module, 'index.js'), 'module.exports = "from the project"')
})

afterAll(() => {
  rmSync(dataDir, { recursive: true, force: true })
})

// bash on Windows is usually the WSL launcher, which is handed its script on stdin instead of as a file.
const onWindows = process.platform === 'win32'

describe('how each script type is run', () => {
  it('gives a file to the interpreters that read their program as they go', () => {
    expect(interpreterFor('bash', false)).toMatchObject({ file: 'script.sh' })
    expect(interpreterFor('bash', false)?.command(false)).toBe('bash')
    expect(interpreterFor('bash', false)?.args('/tmp/s/script.sh')).toEqual(['/tmp/s/script.sh'])

    const pwsh = interpreterFor('powershell', false)
    expect(pwsh?.file).toBe('script.ps1')
    expect(pwsh?.command(false)).toBe('pwsh')
    // No profile and no policy to refuse it: the script is one this machine just wrote.
    expect(pwsh?.args('/tmp/s/script.ps1')).toEqual([
      '-NoProfile',
      '-ExecutionPolicy',
      'Bypass',
      '-File',
      '/tmp/s/script.ps1'
    ])
  })

  it('keeps the interpreters that read their program whole on stdin', () => {
    for (const type of ['python', 'node']) {
      const interpreter = interpreterFor(type, false)
      expect(interpreter?.file).toBeUndefined()
      expect(interpreter?.args('')).toEqual(['-'])
    }
    expect(interpreterFor('python', false)?.command(false)).toBe('python3')
    expect(interpreterFor('python', true)?.command(true)).toBe('python')
    expect(interpreterFor('node', false)?.command(false)).toBe('node')
  })

  it('keeps bash on stdin where bash.exe is the launcher for another filesystem', () => {
    const onWindows = interpreterFor('bash', true)
    expect(onWindows?.file).toBeUndefined()
    expect(onWindows?.args('')).toEqual(['-s'])
    expect(onWindows?.command(true)).toBe('bash.exe')
  })

  it('knows nothing of a type it does not run', () => {
    expect(interpreterFor('ruby', false)).toBeUndefined()
    // An inherited name is not a script type either.
    expect(interpreterFor('toString', false)).toBeUndefined()
  })
})

describe('when a script cannot be run at all', () => {
  it('refuses a type it does not know, and says which', async () => {
    const result = await executeScript({
      scriptType: 'ruby' as never,
      scriptContent: 'puts 1'
    })

    expect(result.success).toBe(false)
    expect(result.error).toContain('Unsupported script type: ruby')
  })

  it('answers, and takes the copy away, when the interpreter cannot even be started', async () => {
    const result = await executeScript({
      scriptType: 'bash',
      scriptContent: 'echo hi',
      // A directory no filesystem can name: spawn refuses it before there is a child to hear from.
      cwd: `${dataDir}/\0`
    })

    expect(result.success).toBe(false)
    expect(existsSync(path.join(dataDir, 'scripts'))).toBe(true)
    expect(readdirSync(path.join(dataDir, 'scripts'))).toEqual([])
  })

  it('says so on the row when the copy cannot be written', async () => {
    const blocked = path.join(dataDir, 'blocked')
    writeFileSync(blocked, 'not a directory')
    setLaunchDataDir(blocked)
    const seen: { data: string }[] = []
    const onData = (payload: { data: string }): void => {
      seen.push(payload)
    }
    scriptRunnerEvents.on(IPC.SCRIPT_DATA, onData)

    try {
      const result = await executeScript({
        scriptType: 'bash',
        scriptContent: 'echo hi',
        runId: 'run-write-fail'
      })

      expect(result.success).toBe(false)
      expect(result.error).toContain('could not write the script')
      expect(seen.map((s) => s.data).join('')).toContain('could not write the script')
    } finally {
      scriptRunnerEvents.off(IPC.SCRIPT_DATA, onData)
      setLaunchDataDir(dataDir)
    }
  })
})

describe('script-runner streaming', () => {
  it('does not emit when runId is absent', async () => {
    const seen: unknown[] = []
    const onData = (payload: unknown): void => void seen.push(payload)
    scriptRunnerEvents.on(IPC.SCRIPT_DATA, onData)
    try {
      const result = await executeScript({
        scriptType: 'node',
        scriptContent: 'console.log("silent")'
      })
      expect(result.success).toBe(true)
      expect(result.output).toContain('silent')
      expect(seen).toHaveLength(0)
    } finally {
      scriptRunnerEvents.off(IPC.SCRIPT_DATA, onData)
    }
  })

  it('emits SCRIPT_DATA chunks and a terminal SCRIPT_EXIT when runId is set', async () => {
    const data: Array<{ runId: string; data: string }> = []
    const exit: Array<{ runId: string; exitCode: number }> = []
    const onData = (p: { runId: string; data: string }): void => void data.push(p)
    const onExit = (p: { runId: string; exitCode: number }): void => void exit.push(p)
    scriptRunnerEvents.on(IPC.SCRIPT_DATA, onData)
    scriptRunnerEvents.on(IPC.SCRIPT_EXIT, onExit)
    try {
      const result = await executeScript({
        scriptType: 'node',
        scriptContent: 'console.log("hello"); console.error("warn")',
        runId: 'run-xyz'
      })
      expect(result.exitCode).toBe(0)
      expect(data.every((d) => d.runId === 'run-xyz')).toBe(true)
      const joined = data.map((d) => d.data).join('')
      expect(joined).toContain('hello')
      expect(joined).toContain('warn')
      expect(exit).toEqual([{ runId: 'run-xyz', exitCode: 0 }])
    } finally {
      scriptRunnerEvents.off(IPC.SCRIPT_DATA, onData)
      scriptRunnerEvents.off(IPC.SCRIPT_EXIT, onExit)
    }
  })

  it.skipIf(onWindows)('runs the rest of a script whose first line reads input', async () => {
    // The script used to arrive on stdin, so `cat` swallowed everything below it.
    const result = await executeScript({
      scriptType: 'bash',
      scriptContent: 'cat\necho "the second line ran"'
    })

    expect(result.success).toBe(true)
    expect(result.output).toContain('the second line ran')
    expect(result.output).not.toContain('echo')
  })

  it.skipIf(onWindows)('hands arguments to the script as its own', async () => {
    const result = await executeScript({
      scriptType: 'bash',
      scriptContent: 'echo "first=$1 second=$2"',
      args: ['alpha', 'beta']
    })

    expect(result.output).toContain('first=alpha second=beta')
  })

  it.skipIf(onWindows)('takes the copy it wrote away with it', async () => {
    const result = await executeScript({
      scriptType: 'bash',
      scriptContent: 'echo "$0"'
    })

    const script = result.output.trim()
    expect(path.basename(script)).toBe('script.sh')
    // Under this machine's own data directory, not a directory every account can write.
    expect(script.startsWith(path.join(dataDir, 'scripts'))).toBe(true)
    expect(existsSync(script)).toBe(false)
    expect(existsSync(path.dirname(script))).toBe(false)
  })

  it('lets a node script require what the directory it runs in provides', async () => {
    // A file would resolve against wherever it was written, so node keeps reading its program from stdin.
    const result = await executeScript({
      scriptType: 'node',
      scriptContent: 'console.log(require("vorn-fixture"))',
      cwd: project
    })

    expect(result.success).toBe(true)
    expect(result.output).toContain('from the project')
  })

  it('runs a node script that never reads the input it was handed', async () => {
    const result = await executeScript({
      scriptType: 'node',
      scriptContent: 'console.log(JSON.stringify(process.argv.slice(2)))',
      args: ['alpha', 'beta']
    })

    expect(result.success).toBe(true)
    expect(result.output).toContain('["alpha","beta"]')
  })

  it('emits SCRIPT_EXIT with non-zero code on script failure', async () => {
    const exit: Array<{ runId: string; exitCode: number }> = []
    const onExit = (p: { runId: string; exitCode: number }): void => void exit.push(p)
    scriptRunnerEvents.on(IPC.SCRIPT_EXIT, onExit)
    try {
      const result = await executeScript({
        scriptType: 'node',
        scriptContent: 'process.exit(7)',
        runId: 'run-fail'
      })
      expect(result.success).toBe(false)
      expect(result.exitCode).toBe(7)
      expect(exit).toEqual([{ runId: 'run-fail', exitCode: 7 }])
    } finally {
      scriptRunnerEvents.off(IPC.SCRIPT_EXIT, onExit)
    }
  })
})
