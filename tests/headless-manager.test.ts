import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('node:child_process', async () => {
  const { EventEmitter } = await import('node:events')
  class FakeChild extends EventEmitter {
    pid = 4242
    stdin = { on: vi.fn(), end: vi.fn(), write: vi.fn() }
    stdout = new EventEmitter()
    stderr = new EventEmitter()
    kill = vi.fn()
    unref = vi.fn()
  }
  return {
    spawn: vi.fn(() => new FakeChild()),
    execFileSync: vi.fn(() => '/usr/bin/cmd')
  }
})

vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(() => 'main'),
  checkoutBranch: vi.fn(),
  createWorktree: vi.fn(),
  extractWorktreeName: vi.fn(),
  isGitRepo: vi.fn(() => false)
}))

import { spawn as spawnImport } from 'node:child_process'
import { headlessManager } from '../packages/server/src/headless-manager'

const spawnMock = spawnImport as unknown as ReturnType<typeof vi.fn>

describe('headlessManager.createHeadless', () => {
  beforeEach(() => {
    spawnMock.mockClear()
  })

  it('pins a fresh agentSessionId for claude and injects --session-id', () => {
    const session = headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true
    })

    expect(session.agentSessionId).toMatch(/^[0-9a-f-]{36}$/)

    const spawnCall = spawnMock.mock.calls[0]
    const args = spawnCall[1] as string[]
    const idx = args.indexOf('--session-id')
    expect(idx).toBeGreaterThanOrEqual(0)
    expect(args[idx + 1]).toBe(session.agentSessionId)

    headlessManager.killHeadless(session.id)
  })

  it('reuses resumeSessionId and uses --resume for claude', () => {
    const session = headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      resumeSessionId: 'existing-session',
      headless: true
    })

    expect(session.agentSessionId).toBe('existing-session')

    const args = spawnMock.mock.calls[0][1] as string[]
    expect(args).toContain('--resume')
    expect(args).toContain('existing-session')

    headlessManager.killHeadless(session.id)
  })

  it('writes a multi-line claude prompt to stdin instead of argv', () => {
    const prompt = '# Workflow: Demo\n\n**Step:** one\n\nDo the thing.'
    const session = headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: prompt,
      headless: true
    })

    const child = spawnMock.mock.results[0].value as {
      stdin: { write: ReturnType<typeof vi.fn>; end: ReturnType<typeof vi.fn> }
    }
    expect(child.stdin.write).toHaveBeenCalledWith(prompt)
    expect(child.stdin.end).toHaveBeenCalled()

    // The prompt must not leak onto argv, where the Windows shell would split it.
    const args = spawnMock.mock.calls[0][1] as string[]
    expect(args).not.toContain(prompt)

    headlessManager.killHeadless(session.id)
  })

  it('does not populate agentSessionId for non-pinning agents', () => {
    const session = headlessManager.createHeadless({
      agentType: 'codex',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true
    })

    expect(session.agentSessionId).toBeUndefined()

    headlessManager.killHeadless(session.id)
  })

  it('propagates workflowId / workflowName onto the session', () => {
    const session = headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true,
      workflowId: 'wf-1',
      workflowName: 'wf'
    })

    expect(session.workflowId).toBe('wf-1')
    expect(session.workflowName).toBe('wf')

    headlessManager.killHeadless(session.id)
  })

  describe('Windows shell spawn', () => {
    const realPlatform = process.platform
    const setPlatform = (value: string) =>
      Object.defineProperty(process, 'platform', { value, configurable: true })

    afterEach(() => setPlatform(realPlatform))

    // The prompt no longer rides on argv for any agent, but per-step args still
    // do, and those can contain spaces — so the cmd.exe quoting still matters.
    it('spawns through the shell with argv quoted so it is not word-split', () => {
      setPlatform('win32')
      const session = headlessManager.createHeadless({
        agentType: 'gemini',
        projectName: 'p',
        projectPath: '/p',
        initialPrompt: 'hello',
        args: ['--model', 'some model name'],
        headless: true
      })

      const [, args, options] = spawnMock.mock.calls[0] as [string, string[], { shell?: boolean }]
      expect(options.shell).toBe(true)
      // The value must NOT appear as a bare element (that word-splits under
      // cmd.exe); it must be a single quoted token that still contains the text.
      expect(args).not.toContain('some model name')
      const quoted = args.filter((a) => a.includes('some model name'))
      expect(quoted).toHaveLength(1)
      // cmd.exe quoting (double quotes) is used regardless of the machine's
      // default shell, since Node's shell:true always runs cmd.exe — not
      // PowerShell single-quotes, which cmd.exe wouldn't treat as quoting.
      expect(quoted[0].startsWith('"')).toBe(true)
      expect(quoted[0].endsWith('"')).toBe(true)

      headlessManager.killHeadless(session.id)
    })

    it('does not quote args on POSIX (no shell wrapper)', () => {
      setPlatform('linux')
      const session = headlessManager.createHeadless({
        agentType: 'gemini',
        projectName: 'p',
        projectPath: '/p',
        initialPrompt: 'hello',
        args: ['--model', 'some model name'],
        headless: true
      })

      const [, args, options] = spawnMock.mock.calls[0] as [string, string[], { shell?: boolean }]
      expect(options.shell).toBe(false)
      // Passed to execve verbatim — one unquoted element.
      expect(args).toContain('some model name')

      headlessManager.killHeadless(session.id)
    })

    // The hang this guards against: on Windows a multi-line prompt on the
    // cmd.exe command line is truncated, and copilot, codex and opencode all
    // then block on stdin producing no output at all — the step never ends.
    it.each(['claude', 'copilot', 'codex', 'opencode', 'gemini'] as const)(
      'keeps the %s prompt off the Windows command line entirely',
      (agentType) => {
        setPlatform('win32')
        const prompt = '# Workflow: Demo\n\n**Step:** one\n\nDo the thing with spaces.'
        const session = headlessManager.createHeadless({
          agentType,
          projectName: 'p',
          projectPath: '/p',
          initialPrompt: prompt,
          headless: true
        })

        const [, args] = spawnMock.mock.calls[0] as [string, string[], { shell?: boolean }]
        expect(args.some((a) => a.includes('Do the thing with spaces.'))).toBe(false)
        expect(args.some((a) => a.includes('\n'))).toBe(false)

        headlessManager.killHeadless(session.id)
      }
    )
  })
})
