import { describe, it, expect } from 'vitest'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { executeScript, scriptRunnerEvents } from '../packages/server/src/script-runner'
import { IPC } from '@vornrun/shared/types'

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

  it('runs the rest of a script whose first line reads input', async () => {
    // The script used to arrive on stdin, so `cat` swallowed everything below it.
    const result = await executeScript({
      scriptType: 'bash',
      scriptContent: 'cat\necho "the second line ran"'
    })

    expect(result.success).toBe(true)
    expect(result.output).toContain('the second line ran')
    expect(result.output).not.toContain('echo')
  })

  it('hands arguments to the script as its own', async () => {
    const result = await executeScript({
      scriptType: 'bash',
      scriptContent: 'echo "first=$1 second=$2"',
      args: ['alpha', 'beta']
    })

    expect(result.output).toContain('first=alpha second=beta')
  })

  it('takes the copy it wrote away with it', async () => {
    const result = await executeScript({
      scriptType: 'bash',
      scriptContent: 'echo "$0"'
    })

    const script = result.output.trim()
    expect(path.basename(script)).toBe('script.sh')
    expect(existsSync(script)).toBe(false)
    expect(existsSync(path.dirname(script))).toBe(false)
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
