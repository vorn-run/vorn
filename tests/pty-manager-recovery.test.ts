import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { IPC } from '@vornrun/shared/types'
import type { CreateTerminalPayload, RemoteHost, TerminalSession } from '@vornrun/shared/types'
import type { HeldSession } from '../packages/server/src/vornd-sessions'

/**
 * The PTY manager over sessions held by vornd: what it asks vornd to start,
 * what it writes to them, how it follows what vornd says about them, and the
 * failure paths -- a spawn that never happens, an agent that dies mid-session,
 * an SSH connection that never comes up, and the idle timer that decides when a
 * quiet session is no longer working.
 */

vi.mock('../packages/server/src/vornd-sessions', async () =>
  (await import('./helpers/fake-vornd-pty')).vorndModule()
)

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: vi.fn(() => ({ defaults: { shell: '/bin/zsh', minimalShellPrompt: true } }))
  }
}))

vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(async () => 'main'),
  getGitHead: vi.fn(async () => 'cafe0000'),
  checkoutBranch: vi.fn(async () => {}),
  createWorktree: vi.fn(),
  extractWorktreeName: vi.fn((p: string) => path.basename(p)),
  isGitRepo: vi.fn(async () => false)
}))

vi.mock('../packages/server/src/shell-integration', () => ({
  getShellIntegration: vi.fn(() => ({ env: {} }))
}))

// The real launch line resolver shells out to `which` per agent.
vi.mock('../packages/server/src/agent-launch', () => ({
  buildAgentLaunchLine: vi.fn((payload: CreateTerminalPayload) => `${payload.agentType}-launch`)
}))

vi.mock('../packages/server/src/process-utils', async () => {
  const actual = await vi.importActual<typeof import('../packages/server/src/process-utils')>(
    '../packages/server/src/process-utils'
  )
  return {
    ...actual,
    // The real versions spawn a login shell to resolve the user's environment.
    getSafeEnv: () => ({ HOME: '/home/user', PATH: '/usr/bin' }),
    getLaunchEnv: () => ({ HOME: '/home/user', PATH: '/usr/bin' })
  }
})

import { ptyManager } from '../packages/server/src/pty-manager'
import { createWorktree, isGitRepo } from '../packages/server/src/git-utils'
import { buildAgentLaunchLine } from '../packages/server/src/agent-launch'
import { isWorkspaceHeld } from '../packages/server/src/workspace-holds'
import { fakeVornd, type FakeVorndPty } from './helpers/fake-vornd-pty'

const createWorktreeMock = vi.mocked(createWorktree)
const isGitRepoMock = vi.mocked(isGitRepo)

const WAITING = 2

const REMOTE_HOST: RemoteHost = {
  id: 'host-1',
  label: 'build-box',
  hostname: 'build.example.com',
  user: 'dev',
  port: 22,
  authMethod: 'agent'
}

let messages: { channel: string; payload: Record<string, unknown> }[] = []
let exited: TerminalSession[] = []
let created: TerminalSession[] = []

async function createAgent(overrides: Partial<CreateTerminalPayload> = {}): Promise<{
  session: TerminalSession
  fake: FakeVorndPty
}> {
  const session = await ptyManager.createPty({
    agentType: 'claude',
    projectName: 'proj',
    projectPath: '/tmp/vorn-proj',
    ...overrides
  })
  return { session, fake: fakeVornd.last() }
}

function messagesOn(channel: string): Record<string, unknown>[] {
  return messages.filter((m) => m.channel === channel).map((m) => m.payload)
}

function statusUpdatesFor(id: string): string[] {
  return messagesOn(IPC.SESSION_UPDATED)
    .filter((p) => p.id === id)
    .map((p) => p.status as string)
}

function tempKeyFiles(): string[] {
  return fs.readdirSync(os.tmpdir()).filter((f) => f.startsWith('vorn-key-'))
}

beforeEach(() => {
  vi.useFakeTimers()
  fakeVornd.reset()
  createWorktreeMock.mockReset()
  isGitRepoMock.mockReset()
  isGitRepoMock.mockResolvedValue(false)

  messages = []
  exited = []
  created = []
  ptyManager.removeAllListeners()
  // Snapshot the payload: SESSION_UPDATED carries the live session object, so a
  // stored reference would report whatever status the session ends the test on.
  ptyManager.on('client-message', (channel: string, payload: Record<string, unknown>) =>
    messages.push({ channel, payload: { ...payload } })
  )
  ptyManager.on('session-exit', (s: TerminalSession) => exited.push(s))
  ptyManager.on('session-created', (s: TerminalSession) => created.push(s))

  ptyManager.setRemoteHosts([REMOTE_HOST])
  ptyManager.setAgentCommands()
  ptyManager.setHeadlessWorktreeCounter(() => ({ count: 0, sessionIds: [] }))
})

afterEach(() => {
  ptyManager.killAll()
  vi.clearAllTimers()
  vi.useRealTimers()
  ptyManager.removeAllListeners()
})

describe('starting a session in vornd', () => {
  it('starts the shell in the project with the terminal type and its own id', async () => {
    const { session, fake } = await createAgent()

    expect(fake.id).toBe(session.id)
    expect(fake.watched).toBe(false)
    expect(fake.spec).toMatchObject({ argv: ['/bin/zsh', '-l'], cwd: '/tmp/vorn-proj' })
    expect(fake.spec?.env).toMatchObject({
      PATH: '/usr/bin',
      TERM: 'xterm-256color',
      VORN_SESSION_ID: session.id
    })
    expect(created).toEqual([session])
  })

  it('writes the agent launch line once the shell has had time to start', async () => {
    const { fake } = await createAgent()

    vi.advanceTimersByTime(299)
    expect(fake.written).toEqual([])

    vi.advanceTimersByTime(1)
    expect(fake.written).toEqual(['claude-launch\r'])
  })

  it('takes the pid from vornd once the spawn is answered', async () => {
    const { session, fake } = await createAgent()
    expect(session.pid).toBe(0)

    fake.start(4321)

    expect(session.pid).toBe(4321)
  })

  it('starts a plain shell where it was asked, with no launch line', () => {
    const session = ptyManager.createShellPty('/tmp/somewhere')
    const fake = fakeVornd.last()

    expect(fake.spec?.cwd).toBe('/tmp/somewhere')
    expect(session.shellCwd).toBe('/tmp/somewhere')
    vi.advanceTimersByTime(1000)
    expect(fake.written).toEqual([])
  })

  it('records the exact Codex resume ID immediately', async () => {
    const { session } = await createAgent({ agentType: 'codex', resumeSessionId: 'known-id' })
    expect(session.agentSessionId).toBe('known-id')
  })

  it('records a remote Codex resume ID without local discovery', async () => {
    const { session } = await createAgent({
      agentType: 'codex',
      resumeSessionId: 'remote-id',
      remoteHostId: REMOTE_HOST.id
    })
    expect(session.agentSessionId).toBe('remote-id')
    expect(session.remoteHostId).toBe(REMOTE_HOST.id)
  })

  it('rejects invalid model selection before spawning', async () => {
    vi.mocked(buildAgentLaunchLine).mockImplementationOnce(() => {
      throw new Error('Invalid model')
    })
    await expect(createAgent({ model: '-invalid' })).rejects.toThrow()
    expect(fakeVornd.spawn).not.toHaveBeenCalled()
  })
})

describe('spawn failures', () => {
  it('propagates the spawn error and registers no session', async () => {
    fakeVornd.spawn.mockImplementation(() => {
      throw new Error('Terminals cannot start: vornd is not running')
    })

    await expect(createAgent()).rejects.toThrow(/vornd is not running/)
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
    expect(created).toHaveLength(0)
    expect(messages).toHaveLength(0)
  })

  it('recovers so the next session after a failed spawn still works', async () => {
    fakeVornd.spawn.mockImplementationOnce(() => {
      throw new Error('Terminals cannot start: vornd is not running')
    })
    await expect(createAgent()).rejects.toThrow(/vornd/)

    const { session } = await createAgent()
    expect(session.status).toBe('running')
    expect(ptyManager.getActiveSessions()).toEqual([session])
  })

  it('propagates a spawn failure for shell sessions too', () => {
    fakeVornd.spawn.mockImplementation(() => {
      throw new Error('Terminals cannot start: vornd is not running')
    })

    expect(() => ptyManager.createShellPty('/tmp')).toThrow(/vornd is not running/)
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
  })

  it('aborts before spawning when worktree creation fails', async () => {
    isGitRepoMock.mockResolvedValue(true)
    createWorktreeMock.mockRejectedValue(new Error('fatal: could not create worktree'))

    await expect(createAgent({ useWorktree: true, branch: 'feature/x' })).rejects.toThrow(
      /could not create worktree/
    )
    expect(fakeVornd.spawn).not.toHaveBeenCalled()
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
  })

  it('reports a spawn vornd could not carry out as an exit', async () => {
    const { session, fake } = await createAgent()

    fake.exit(1)

    expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 1 }])
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
  })
})

describe('a worktree made for a new session', () => {
  it('is held from the moment git names it until the session exists', async () => {
    isGitRepoMock.mockResolvedValue(true)
    const made = '/tmp/.vorn-worktrees/vorn-proj/held-0000aaaa'
    const heldDuring: boolean[] = []
    createWorktreeMock.mockImplementation(async (_project, branch, _name, _remote, onPath) => {
      onPath?.(made)
      heldDuring.push(isWorkspaceHeld(made))
      return { worktreePath: made, branch, name: 'held' }
    })
    const payload = {
      agentType: 'claude',
      projectName: 'proj',
      projectPath: '/tmp/vorn-proj',
      useWorktree: true,
      branch: 'feature/x'
    } as CreateTerminalPayload
    const prepared = await ptyManager.prepareSession(payload)
    expect(heldDuring).toEqual([true])
    expect(isWorkspaceHeld(made)).toBe(true)
    const session = ptyManager.spawnPty(payload, prepared)
    expect(isWorkspaceHeld(made)).toBe(false)
    expect(fakeVornd.last().spec?.cwd).toBe(made)
    expect(session).toMatchObject({ worktreePath: made, isWorktree: true, branch: 'feature/x' })
  })

  it('is let go when preparing fails after git made it', async () => {
    isGitRepoMock.mockResolvedValue(true)
    const made = '/tmp/.vorn-worktrees/vorn-proj/failed-0000bbbb'
    createWorktreeMock.mockImplementation(async (_project, _branch, _name, _remote, onPath) => {
      onPath?.(made)
      throw new Error('fatal: could not create worktree')
    })
    await expect(createAgent({ useWorktree: true, branch: 'feature/x' })).rejects.toThrow()
    expect(isWorkspaceHeld(made)).toBe(false)
  })

  it('is let go when the spawn fails', async () => {
    isGitRepoMock.mockResolvedValue(true)
    const made = '/tmp/.vorn-worktrees/vorn-proj/spawn-0000cccc'
    createWorktreeMock.mockImplementation(async (_project, branch, _name, _remote, onPath) => {
      onPath?.(made)
      return { worktreePath: made, branch, name: 'spawn' }
    })
    fakeVornd.spawn.mockImplementation(() => {
      throw new Error('Terminals cannot start: vornd is not running')
    })
    await expect(createAgent({ useWorktree: true, branch: 'feature/x' })).rejects.toThrow()
    expect(isWorkspaceHeld(made)).toBe(false)
  })
})

describe('agent crashes mid-session', () => {
  it('reports the crash exit code and parks the session as idle', async () => {
    const { session, fake } = await createAgent()

    fake.exit(139)

    expect(exited).toEqual([session])
    expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 139 }])
    expect(session.status).toBe('idle')
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
    expect(ptyManager.getActiveSessions()).toEqual([session])
  })

  it('says nothing to clients about an exit vornd tells again', async () => {
    // Acted on by the server before this one, before vornd restarted.
    const worktree = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wt-'))
    try {
      const { session, fake } = await createAgent({
        existingWorktreePath: worktree,
        branch: 'feature/x'
      })

      fake.exit(0, true)

      expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([])
      expect(messagesOn(IPC.WORKTREE_CONFIRM_CLEANUP)).toEqual([])
      expect(session.status).toBe('idle')
      expect(ptyManager.hasLivePty(session.id)).toBe(false)
    } finally {
      fs.rmSync(worktree, { recursive: true, force: true })
    }
  })

  it('cancels the idle timer so a crashed session is never re-marked', async () => {
    const { session, fake } = await createAgent()

    fake.activity()
    fake.exit(1)
    const updatesBefore = statusUpdatesFor(session.id).length
    vi.advanceTimersByTime(60_000)

    expect(statusUpdatesFor(session.id)).toHaveLength(updatesBefore)
    expect(session.status).toBe('idle')
  })

  it('asks to clean up the worktree when the last session using it crashes', async () => {
    const worktree = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wt-'))
    try {
      const { session, fake } = await createAgent({
        existingWorktreePath: worktree,
        branch: 'feature/x'
      })
      expect(session.isWorktree).toBe(true)

      fake.exit(1)

      expect(messagesOn(IPC.WORKTREE_CONFIRM_CLEANUP)).toEqual([
        { id: session.id, projectPath: '/tmp/vorn-proj', worktreePath: worktree }
      ])
    } finally {
      fs.rmSync(worktree, { recursive: true, force: true })
    }
  })

  it('keeps the worktree when another session still uses it', async () => {
    const worktree = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wt-'))
    try {
      const first = await createAgent({ existingWorktreePath: worktree, branch: 'feature/x' })
      await createAgent({ existingWorktreePath: worktree, branch: 'feature/x' })

      first.fake.exit(1)

      expect(messagesOn(IPC.WORKTREE_CONFIRM_CLEANUP)).toEqual([])
    } finally {
      fs.rmSync(worktree, { recursive: true, force: true })
    }
  })

  it('keeps the worktree when a headless session still uses it', async () => {
    const worktree = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wt-'))
    try {
      ptyManager.setHeadlessWorktreeCounter(() => ({ count: 1, sessionIds: ['headless-1'] }))
      const { fake } = await createAgent({ existingWorktreePath: worktree, branch: 'feature/x' })

      fake.exit(1)

      expect(messagesOn(IPC.WORKTREE_CONFIRM_CLEANUP)).toEqual([])
    } finally {
      fs.rmSync(worktree, { recursive: true, force: true })
    }
  })

  it('still reports an exit when killing a session whose program already ended', async () => {
    const { session, fake } = await createAgent()
    fake.exit(139)
    messages = []
    exited = []

    ptyManager.killPty(session.id)

    // The renderer still gets an exit so it can finish its close-intent cleanup.
    expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 0 }])
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
    // An exit leaves the session in the map, so closing the card afterwards
    // repeats session-exit. Its listeners (hook cleanup, save, broadcast) are
    // idempotent, so the repeat is tolerated rather than suppressed.
    expect(exited).toEqual([session])
  })
})

describe('killing a session', () => {
  it('forgets it at once and signals vornd on the next turn', async () => {
    const { session, fake } = await createAgent()

    ptyManager.killPty(session.id)

    expect(exited).toEqual([session])
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
    expect(fake.kills).toEqual([])

    vi.advanceTimersByTime(1)
    expect(fake.kills).toEqual(['SIGHUP'])
  })

  it('does not repeat session-exit when vornd then reports the exit', async () => {
    const { session, fake } = await createAgent()
    ptyManager.killPty(session.id)
    vi.advanceTimersByTime(1)
    exited = []

    fake.exit(129)

    expect(exited).toEqual([])
    expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 129 }])
  })

  it('swallows a kill that fails because the process is already gone', async () => {
    const { session, fake } = await createAgent()
    fake.killError = new Error('ESRCH')

    expect(() => {
      ptyManager.killPty(session.id)
      vi.advanceTimersByTime(10)
    }).not.toThrow()
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
  })

  it('clears the idle timer', async () => {
    const { session, fake } = await createAgent()
    fake.activity()

    ptyManager.killPty(session.id)
    vi.advanceTimersByTime(60_000)

    expect(statusUpdatesFor(session.id)).toEqual([])
  })

  it('asks to clean up a worktree nothing else uses', async () => {
    const worktree = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wt-'))
    try {
      const { session } = await createAgent({
        existingWorktreePath: worktree,
        branch: 'feature/x'
      })

      ptyManager.killPty(session.id)

      expect(messagesOn(IPC.WORKTREE_CONFIRM_CLEANUP)).toEqual([
        { id: session.id, projectPath: '/tmp/vorn-proj', worktreePath: worktree }
      ])
    } finally {
      fs.rmSync(worktree, { recursive: true, force: true })
    }
  })
})

describe('letting go of every session', () => {
  it('releases each one to vornd without ending it', async () => {
    const a = await createAgent()
    const b = ptyManager.createShellPty('/tmp')
    const shell = fakeVornd.last()

    ptyManager.killAll()

    expect(fakeVornd.release.mock.calls.map(([id]) => id).sort()).toEqual(
      [a.session.id, b.id].sort()
    )
    vi.advanceTimersByTime(1000)
    expect(a.fake.kills).toEqual([])
    expect(shell.kills).toEqual([])
    expect(ptyManager.getActiveSessions()).toEqual([])
    expect(ptyManager.livePtyCount()).toBe(0)
  })

  it('stops every idle timer', async () => {
    const { session, fake } = await createAgent()
    fake.activity()

    ptyManager.killAll()
    vi.advanceTimersByTime(60_000)

    expect(statusUpdatesFor(session.id)).toEqual([])
  })
})

describe('SSH connection failures', () => {
  const remotePayload = (
    overrides: Partial<CreateTerminalPayload> = {}
  ): CreateTerminalPayload => ({
    agentType: 'claude',
    projectName: 'proj',
    projectPath: '/srv/proj',
    remoteHostId: REMOTE_HOST.id,
    ...overrides
  })

  it('reads the output of a remote session, from the home directory', async () => {
    const session = await ptyManager.createPty(remotePayload())
    const fake = fakeVornd.last()

    expect(fake.watched).toBe(true)
    expect(fake.spec?.cwd).toBe(os.homedir())
    expect(session).toMatchObject({ remoteHostId: REMOTE_HOST.id, remoteHostLabel: 'build-box' })

    vi.advanceTimersByTime(300)
    expect(fake.written).toEqual([
      `ssh -t dev@build.example.com 'echo __VORN_READY_${session.id.slice(0, 8)}__ && exec $SHELL -l'\r`
    ])
  })

  it('stops the remote command on an SSH error', async () => {
    await ptyManager.createPty(remotePayload())
    const fake = fakeVornd.last()
    vi.advanceTimersByTime(300)

    fake.print('ssh: connect to host build.example.com port 22: Connection refused\r\n')

    // The fallback must be cancelled — a refused connection has no shell to run in.
    vi.advanceTimersByTime(30_000)
    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(false)
  })

  it.each([
    'Permission denied (publickey).',
    'Host key verification failed.',
    'ssh: Could not resolve hostname build.example.com',
    'Connection timed out'
  ])('treats %j as a connection failure', async (errorOutput) => {
    await ptyManager.createPty(remotePayload())
    const fake = fakeVornd.last()
    vi.advanceTimersByTime(300)

    fake.print(`${errorOutput}\r\n`)
    vi.advanceTimersByTime(30_000)

    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(false)
  })

  it('runs the remote command once the marker arrives', async () => {
    const session = await ptyManager.createPty(remotePayload())
    const fake = fakeVornd.last()
    vi.advanceTimersByTime(300)

    fake.print(`__VORN_READY_${session.id.slice(0, 8)}__\r\n`)
    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(false)

    vi.advanceTimersByTime(200)
    expect(fake.written).toContain('cd /srv/proj && claude-launch\r')
  })

  it('falls back to sending the remote command when the marker never arrives', async () => {
    await ptyManager.createPty(remotePayload())
    const fake = fakeVornd.last()

    vi.advanceTimersByTime(300)
    fake.print('welcome to a very non-standard shell\r\n')
    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(false)

    vi.advanceTimersByTime(8000)
    expect(fake.written).toContain('cd /srv/proj && claude-launch\r')
  })

  it('never writes to a session that was killed before the fallback fired', async () => {
    const session = await ptyManager.createPty(remotePayload())
    const fake = fakeVornd.last()

    ptyManager.killPty(session.id)
    vi.advanceTimersByTime(30_000)

    expect(fake.written).toEqual([])
  })

  it('leaves a stored key to vornd and logs in with the agent', async () => {
    const before = new Set(tempKeyFiles())
    ptyManager.setRemoteHosts([{ ...REMOTE_HOST, authMethod: 'key-stored' }])
    await ptyManager.createPty(remotePayload())
    const fake = fakeVornd.last()

    vi.advanceTimersByTime(300)
    expect(fake.written[0]).not.toContain('-i ')
    expect(tempKeyFiles().filter((f) => !before.has(f))).toEqual([])
  })
})

describe('status from vornd', () => {
  it("leaves an agent's status to vornd, and tells it the input that wakes one", async () => {
    const { session, fake } = await createAgent()

    fake.status(WAITING)
    fake.activity()
    vi.advanceTimersByTime(60_000)
    expect(session.status).toBe('running')
    expect(statusUpdatesFor(session.id)).toEqual([])

    ptyManager.writeToPty(session.id, 'y')
    expect(fakeVornd.input).not.toHaveBeenCalled()
    session.status = 'waiting'
    ptyManager.writeToPty(session.id, 'next task\r')
    expect(fakeVornd.input).toHaveBeenCalledWith(session.id)
    expect(fake.written).toContain('next task\r')
  })

  it('follows a shell to the directory vornd says it is in', () => {
    const session = ptyManager.createShellPty('/tmp')
    const fake = fakeVornd.last()
    const moved: [string, string][] = []
    ptyManager.on('session-cwd', (id: string, cwd: string) => moved.push([id, cwd]))

    fake.cwd('/tmp/deeper')
    fake.cwd('/tmp/deeper')

    expect(session.shellCwd).toBe('/tmp/deeper')
    expect(moved).toEqual([[session.id, '/tmp/deeper']])
  })

  it('keeps no directory for an agent session', async () => {
    const { session, fake } = await createAgent()

    fake.cwd('/elsewhere')

    expect(session.shellCwd).toBeUndefined()
  })
})

describe('a session whose program ends', () => {
  it('records the exit code when a shell session ends', () => {
    const session = ptyManager.createShellPty('/tmp')
    const fake = fakeVornd.last()

    fake.exit(130)

    expect(session.status).toBe('idle')
    expect(session.shellExitCode).toBe(130)
  })

  it('ignores writes and resizes for a session that no longer exists', () => {
    expect(() => ptyManager.writeToPty('gone', 'hello')).not.toThrow()
    expect(() => ptyManager.resizePty('gone', 80, 24)).not.toThrow()
    expect(() => ptyManager.killPty('gone')).not.toThrow()
  })
})

describe('the size a client fitted a session to', () => {
  it('is recorded on the session', async () => {
    const { session } = await createAgent()

    ptyManager.resizePty(session.id, 132, 43)

    expect(session).toMatchObject({ cols: 132, rows: 43 })
  })

  it.each([
    [70_000, 40],
    [0, 0],
    [-1, 24],
    [80.5, 24]
  ])('is ignored when it is %d by %d', async (cols, rows) => {
    // Arrives as a fire-and-forget notification, so a throw here has no caller.
    const { session } = await createAgent()

    expect(() => ptyManager.resizePty(session.id, cols, rows)).not.toThrow()

    expect(session).toMatchObject({ cols: 80, rows: 24 })
  })
})

describe('reading what a session printed', () => {
  it('asks vornd for it', async () => {
    const { session } = await createAgent()
    fakeVornd.readOutput.mockResolvedValueOnce(['line one', 'line two'])

    await expect(ptyManager.readOutput(session.id, 2)).resolves.toEqual(['line one', 'line two'])
    expect(fakeVornd.readOutput).toHaveBeenCalledWith(session.id, 2)
  })

  it('refuses a session it does not know', async () => {
    await expect(ptyManager.readOutput('no-such-session')).rejects.toThrow(/not found/)
    expect(fakeVornd.readOutput).not.toHaveBeenCalled()
  })
})

describe('taking on a terminal vornd still holds', () => {
  function saved(overrides: Partial<TerminalSession> = {}): TerminalSession {
    return {
      id: 'carried-1',
      agentType: 'claude',
      projectName: 'carried',
      projectPath: '/tmp/carried',
      status: 'idle',
      createdAt: Date.now(),
      cols: 100,
      rows: 30,
      pid: 0,
      ...overrides
    }
  }

  function held(id: string, status?: number): HeldSession {
    return {
      id,
      kind: 'pty',
      pid: 7777,
      status:
        status === undefined
          ? null
          : { effectId: `${id}/s`, id, epoch: 1, rseq: 0, index: 0, kind: 'status', status },
      cwd: null,
      exit: null
    }
  }

  it('makes it an ordinary live session', () => {
    const session = saved()
    ptyManager.adoptVornd(session, held(session.id))

    expect(ptyManager.getActiveSessions()).toEqual([session])
    expect(ptyManager.hasLivePty(session.id)).toBe(true)
    expect(session).toMatchObject({ pid: 7777, status: 'running' })
    expect(fakeVornd.adopt.mock.calls[0][1]).toBe(false)
  })

  it('leaves its status to vornd, and follows its exit', () => {
    const session = saved()
    ptyManager.adoptVornd(session, held(session.id, WAITING))
    expect(session.status).toBe('running')

    const pty = fakeVornd.adopt.mock.results[0].value as FakeVorndPty
    pty.exit(0)
    expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 0 }])
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
  })

  it('writes through to it', () => {
    const session = saved()
    ptyManager.adoptVornd(session, held(session.id))

    ptyManager.writeToPty(session.id, 'hello')

    const pty = fakeVornd.adopt.mock.results[0].value as FakeVorndPty
    expect(pty.written).toEqual(['hello'])
  })

  it('reads the output of a remote one', () => {
    const session = saved({ remoteHostId: REMOTE_HOST.id })
    ptyManager.adoptVornd(session, held(session.id))

    expect(fakeVornd.adopt.mock.calls[0][1]).toBe(true)
  })

  it('does not put it in the order twice', () => {
    const session = saved()
    ptyManager.adoptVornd(session, held(session.id))
    ptyManager.adoptVornd(session, held(session.id))

    expect(ptyManager.getActiveSessions()).toEqual([session])
  })
})

describe('letting go of a session that is about to come back', () => {
  it('announces nothing, where killing one announces an exit', async () => {
    // Resume used to route through `killPty`, which emits `session-exit` for a
    // session that is returning under the same id and -- when it was the last
    // one in a worktree -- broadcasts WORKTREE_CONFIRM_CLEANUP. That reaches the
    // person as an offer to delete the worktree the agent is at that moment
    // being resumed into.
    const comingBack = await createAgent()
    ptyManager.releaseForResume(comingBack.session.id)
    vi.advanceTimersByTime(1000)
    expect(exited).toEqual([])
    expect(messages).toEqual([])
    expect(comingBack.fake.kills).toEqual([])
    expect(ptyManager.hasLivePty(comingBack.session.id)).toBe(false)
    expect(ptyManager.getActiveSessions()).toEqual([])

    // The contrast, so this cannot pass by nothing being emitted at all.
    const going = await createAgent()
    ptyManager.killPty(going.session.id)
    expect(exited).toEqual([going.session])
  })

  it('can be put back when its spawn then fails', async () => {
    // Releasing is destructive on purpose -- it is what lets the replacement
    // take the same id -- but a spawn that throws must not end the session.
    const { session } = await createAgent()
    ptyManager.releaseForResume(session.id)

    ptyManager.restoreReleased(session)
    ptyManager.restoreReleased(session)

    expect(ptyManager.getActiveSessions()).toEqual([session])
    // Still no process behind it, which is what makes it resumable rather than live.
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
  })
})
