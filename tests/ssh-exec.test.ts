import { describe, expect, it, vi } from 'vitest'

const calls = vi.hoisted(() => ({
  execFile: vi.fn()
}))

vi.mock('node:child_process', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  execFile: calls.execFile
}))

import { sshExec } from '../packages/server/src/process-utils'
import type { RemoteHost } from '../packages/shared/src/types'

const host = { id: 'h', hostname: 'box', user: 'dev', port: 2222 } as RemoteHost

type Callback = (err: Error | null, stdout: string) => void

describe('sshExec', () => {
  it("runs sshExecSync's command without blocking, and resolves with its output", async () => {
    calls.execFile.mockImplementationOnce(
      (_bin: string, _args: string[], _opts: unknown, done: Callback) => done(null, 'main\n')
    )
    await expect(sshExec(host, 'git branch', { timeout: 500 })).resolves.toBe('main\n')
    const [bin, args, opts] = calls.execFile.mock.calls[0]
    expect(bin).toBe('ssh')
    expect(args).toEqual(expect.arrayContaining(['-p', '2222', 'dev@box']))
    expect(args.at(-1)).toBe('git branch')
    expect(opts).toMatchObject({ timeout: 500, encoding: 'utf-8' })
  })

  it('rejects with the error ssh failed with, and defaults the timeout', async () => {
    calls.execFile.mockImplementationOnce(
      (_bin: string, _args: string[], _opts: unknown, done: Callback) =>
        done(new Error('Command failed: ssh'), '')
    )
    await expect(sshExec(host, 'git status')).rejects.toThrow('Command failed: ssh')
    expect(calls.execFile.mock.calls.at(-1)?.[2]).toMatchObject({ timeout: 15000 })
  })
})
