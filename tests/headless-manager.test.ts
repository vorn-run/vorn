import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

/** Each agent vornd was asked to start, with what it was written, in order. */
const { spawnMock } = vi.hoisted(() => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { EventEmitter } = require('node:events') as typeof import('node:events')
  const spawnMock = vi.fn(() => {
    const agent = Object.assign(new EventEmitter(), {
      written: [] as string[],
      isEnded: false,
      write: vi.fn((data: string) => void agent.written.push(data)),
      closeStdin: vi.fn(),
      kill: vi.fn(),
      onData: vi.fn(),
      onExit: vi.fn()
    })
    return agent
  })
  return { spawnMock }
})

vi.mock('../packages/server/src/vornd-sessions', () => ({
  vorndSessions: {
    spawn: spawnMock,
    release: vi.fn(),
    on: vi.fn(),
    createsHeadless: () => false,
    mirror: { headlessRecord: () => undefined }
  }
}))
vi.mock('../packages/server/src/resolve-executable', () => ({
  findOnPath: (name: string) => (name === 'claude' ? '/opt/agents/bin/claude' : null)
}))
vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(async () => 'main'),
  checkoutBranch: vi.fn(async () => {}),
  createWorktree: vi.fn(),
  extractWorktreeName: vi.fn(),
  isGitRepo: vi.fn(async () => false)
}))

import { headlessManager } from '../packages/server/src/headless-manager'

/** The `n`th start: the command, its arguments, and the agent that came back. */
function started(n = 0): {
  command: string
  args: string[]
  spec: { argv: string[]; cwd: string; piped?: boolean }
  agent: { written: string[]; closeStdin: ReturnType<typeof vi.fn> }
} {
  const call = spawnMock.mock.calls[n] as unknown as [string, { argv: string[]; cwd: string }]
  const spec = call[1]
  return {
    command: spec.argv[0],
    args: spec.argv.slice(1),
    spec,
    agent: spawnMock.mock.results[n].value
  }
}

describe('headlessManager.createHeadless', () => {
  it('preserves the requested Codex UUID and emits exec resume', async () => {
    const session = await headlessManager.createHeadless({
      agentType: 'codex',
      projectName: 'p',
      projectPath: '/p',
      resumeSessionId: 'known-id',
      initialPrompt: 'continue'
    })
    expect(session.agentSessionId).toBe('known-id')
    expect(started(spawnMock.mock.calls.length - 1).args).toEqual([
      '-a',
      'never',
      'exec',
      'resume',
      'known-id',
      '-'
    ])
    headlessManager.killHeadless(session.id)
  })
  beforeEach(() => {
    spawnMock.mockClear()
  })

  it('pins a fresh agentSessionId for claude and injects --session-id', async () => {
    const session = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true
    })

    expect(session.agentSessionId).toMatch(/^[0-9a-f-]{36}$/)

    const { args, spec } = started()
    expect(spec.piped).toBe(true)
    const idx = args.indexOf('--session-id')
    expect(idx).toBeGreaterThanOrEqual(0)
    expect(args[idx + 1]).toBe(session.agentSessionId)

    headlessManager.killHeadless(session.id)
  })

  it('spawns the agent by its absolute path, so a poor PATH cannot lose it', async () => {
    const session = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true
    })
    expect(started().command).toBe('/opt/agents/bin/claude')
    headlessManager.killHeadless(session.id)
  })

  it('keeps a bare name when nothing on PATH answers to it', async () => {
    const session = await headlessManager.createHeadless({
      agentType: 'codex',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true
    })
    expect(started().command).toBe('codex')
    headlessManager.killHeadless(session.id)
  })

  it('reuses resumeSessionId and uses --resume for claude', async () => {
    const session = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      resumeSessionId: 'existing-session',
      headless: true
    })

    expect(session.agentSessionId).toBe('existing-session')

    const { args } = started()
    expect(args).toContain('--resume')
    expect(args).toContain('existing-session')

    headlessManager.killHeadless(session.id)
  })

  it('writes a multi-line claude prompt to stdin instead of argv', async () => {
    const prompt = '# Workflow: Demo\n\n**Step:** one\n\nDo the thing.'
    const session = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: prompt,
      headless: true
    })

    const { agent } = started()
    expect(agent.written).toEqual([prompt])
    expect(agent.closeStdin).toHaveBeenCalled()

    // The prompt must not leak onto argv, where the Windows shell would split it.
    const { args } = started()
    expect(args).not.toContain(prompt)

    headlessManager.killHeadless(session.id)
  })

  it('does not populate agentSessionId for non-pinning agents', async () => {
    const session = await headlessManager.createHeadless({
      agentType: 'codex',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true
    })

    expect(session.agentSessionId).toBeUndefined()

    headlessManager.killHeadless(session.id)
  })

  it('propagates workflowId / workflowName onto the session', async () => {
    const session = await headlessManager.createHeadless({
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
    it('spawns through the shell with argv quoted so it is not word-split', async () => {
      setPlatform('win32')
      const session = await headlessManager.createHeadless({
        agentType: 'gemini',
        projectName: 'p',
        projectPath: '/p',
        initialPrompt: 'hello',
        args: ['--model', 'some model name'],
        headless: true
      })

      // What `shell: true` would run, spelled out: cmd.exe with one command line.
      const { command, args } = started()
      expect(command).toMatch(/cmd/i)
      expect(args.slice(0, 3)).toEqual(['/d', '/s', '/c'])
      const line = args[3]
      expect(line).toBe(`"${session.launchCommand}"`)
      // The value must NOT appear bare (that word-splits under cmd.exe); it
      // must be a single quoted token that still contains the text. cmd.exe
      // quoting (double quotes) is used regardless of the machine's default
      // shell, not PowerShell single-quotes, which cmd.exe wouldn't treat as quoting.
      expect(line).toContain('"some model name"')
      expect(line).not.toContain("'some model name'")

      headlessManager.killHeadless(session.id)
    })

    it('does not quote args on POSIX (no shell wrapper)', async () => {
      setPlatform('linux')
      const session = await headlessManager.createHeadless({
        agentType: 'gemini',
        projectName: 'p',
        projectPath: '/p',
        initialPrompt: 'hello',
        args: ['--model', 'some model name'],
        headless: true
      })

      const { command, args } = started()
      expect(command).toBe('gemini')
      // Passed to execve verbatim — one unquoted element.
      expect(args).toContain('some model name')

      headlessManager.killHeadless(session.id)
    })

    // The hang this guards against: on Windows a multi-line prompt on the
    // cmd.exe command line is truncated, and copilot, codex and opencode all
    // then block on stdin producing no output at all — the step never ends.
    it.each(['claude', 'copilot', 'codex', 'opencode', 'gemini'] as const)(
      'keeps the %s prompt off the Windows command line entirely',
      async (agentType) => {
        setPlatform('win32')
        const prompt = '# Workflow: Demo\n\n**Step:** one\n\nDo the thing with spaces.'
        const session = await headlessManager.createHeadless({
          agentType,
          projectName: 'p',
          projectPath: '/p',
          initialPrompt: prompt,
          headless: true
        })

        const { args } = started()
        expect(args.some((a) => a.includes('Do the thing with spaces.'))).toBe(false)
        expect(args.some((a) => a.includes('\n'))).toBe(false)

        headlessManager.killHeadless(session.id)
      }
    )
  })
})

describe('a worktree made for a headless run', () => {
  it('is held while the run is prepared, and let go once it is a session', async () => {
    const git = await import('../packages/server/src/git-utils')
    const { isWorkspaceHeld } = await import('../packages/server/src/workspace-holds')
    const made = '/p-worktrees/run-0000cccc'
    const heldDuring: boolean[] = []
    vi.mocked(git.isGitRepo).mockResolvedValueOnce(true)
    vi.mocked(git.createWorktree).mockImplementationOnce(
      async (_project, branch, _name, _remote, onPath) => {
        onPath?.(made)
        heldDuring.push(isWorkspaceHeld(made))
        return { worktreePath: made, branch, name: 'run' }
      }
    )
    const session = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      useWorktree: true,
      branch: 'feature/x',
      initialPrompt: 'go'
    })
    expect(heldDuring).toEqual([true])
    expect(isWorkspaceHeld(made)).toBe(false)
    expect(headlessManager.getActiveSessionsForWorktree(made)).toMatchObject({ count: 1 })
    headlessManager.killHeadless(session.id)
  })
})
