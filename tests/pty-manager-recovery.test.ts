import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { IPC } from '@vornrun/shared/types'
import type { CreateTerminalPayload, RemoteHost, TerminalSession } from '@vornrun/shared/types'

/**
 * Failure-path coverage for the PTY manager: a spawn that never happens, an
 * agent that dies mid-session, an SSH connection that never comes up, and the
 * idle timer that decides when a quiet session is no longer working.
 */

const { spawnMock, FakePty } = vi.hoisted(() => {
  type DataHandler = (data: string) => void
  type ExitHandler = (e: { exitCode: number; signal?: number }) => void

  let nextPid = 1000

  class FakePty {
    pid = nextPid++
    written: string[] = []
    killed = false
    /** Set to simulate killing a process that the OS already reaped. */
    killError: Error | null = null
    resize = vi.fn()
    private dataHandlers: DataHandler[] = []
    private exitHandlers: ExitHandler[] = []

    write(data: string): void {
      this.written.push(data)
    }

    kill(): void {
      if (this.killError) throw this.killError
      this.killed = true
    }

    onData(cb: DataHandler): { dispose: () => void } {
      this.dataHandlers.push(cb)
      return {
        dispose: () => {
          this.dataHandlers = this.dataHandlers.filter((h) => h !== cb)
        }
      }
    }

    onExit(cb: ExitHandler): { dispose: () => void } {
      this.exitHandlers.push(cb)
      return {
        dispose: () => {
          this.exitHandlers = this.exitHandlers.filter((h) => h !== cb)
        }
      }
    }

    emitData(data: string): void {
      for (const h of [...this.dataHandlers]) h(data)
    }

    emitExit(exitCode = 0): void {
      for (const h of [...this.exitHandlers]) h({ exitCode })
    }
  }

  return { spawnMock: vi.fn(() => new FakePty()), FakePty }
})

type FakePtyInstance = InstanceType<typeof FakePty>

// The nested copy by name, not the bare specifier. `packages/server` pins its
// own node-pty, so mocking 'node-pty' from here patches the root copy and
// leaves pty-manager's untouched — the spawns would be real. Same reasoning as
// `pty-session-id.test.ts`, which is where this convention comes from.
vi.mock('../packages/server/node_modules/node-pty', () => ({
  default: { spawn: spawnMock },
  spawn: spawnMock
}))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: vi.fn(() => ({ defaults: { shell: '/bin/zsh', minimalShellPrompt: true } }))
  }
}))

vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(() => 'main'),
  checkoutBranch: vi.fn(),
  createWorktree: vi.fn(),
  extractWorktreeName: vi.fn((p: string) => path.basename(p)),
  isGitRepo: vi.fn(() => false)
}))

vi.mock('../packages/server/src/shell-integration', () => ({
  getShellIntegration: vi.fn(() => ({ env: {} }))
}))

// The real launch line resolver shells out to `which` per agent; the recovery
// paths under test only care that *some* line is written.
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

const createWorktreeMock = vi.mocked(createWorktree)
const isGitRepoMock = vi.mocked(isGitRepo)

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

function lastPty(): FakePtyInstance {
  const results = spawnMock.mock.results
  return results[results.length - 1].value as FakePtyInstance
}

function createAgent(overrides: Partial<CreateTerminalPayload> = {}): {
  session: TerminalSession
  fake: FakePtyInstance
} {
  const session = ptyManager.createPty({
    agentType: 'claude',
    projectName: 'proj',
    projectPath: '/tmp/vorn-proj',
    ...overrides
  })
  return { session, fake: lastPty() }
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
  spawnMock.mockReset()
  spawnMock.mockImplementation(() => new FakePty())
  createWorktreeMock.mockReset()
  isGitRepoMock.mockReset()
  isGitRepoMock.mockReturnValue(false)

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

describe('pty spawn failures', () => {
  it('propagates the spawn error and registers no session', () => {
    spawnMock.mockImplementation(() => {
      throw new Error('posix_spawnp failed')
    })

    expect(() => createAgent()).toThrow(/posix_spawnp failed/)
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
    expect(created).toHaveLength(0)
    expect(messages).toHaveLength(0)
  })

  it('recovers so the next session after a failed spawn still works', () => {
    spawnMock.mockImplementationOnce(() => {
      throw new Error('spawn ENOENT')
    })
    expect(() => createAgent()).toThrow(/ENOENT/)

    const { session } = createAgent()
    expect(session.status).toBe('running')
    expect(ptyManager.getActiveSessions()).toEqual([session])
  })

  it('propagates a spawn failure for shell sessions too', () => {
    spawnMock.mockImplementation(() => {
      throw new Error('shell not found')
    })

    expect(() => ptyManager.createShellPty('/tmp')).toThrow(/shell not found/)
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
  })

  it('aborts before spawning when worktree creation fails', () => {
    isGitRepoMock.mockReturnValue(true)
    createWorktreeMock.mockImplementation(() => {
      throw new Error('fatal: could not create worktree')
    })

    expect(() => createAgent({ useWorktree: true, branch: 'feature/x' })).toThrow(
      /could not create worktree/
    )
    expect(spawnMock).not.toHaveBeenCalled()
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
  })
})

describe('agent crashes mid-session', () => {
  it('reports the crash exit code and parks the session as idle', () => {
    const { session, fake } = createAgent()

    fake.emitExit(139)

    expect(exited).toEqual([session])
    expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 139 }])
    expect(session.status).toBe('idle')
  })

  it('flushes buffered output before announcing the exit', () => {
    const { session, fake } = createAgent()

    fake.emitData('segfault imminent\n')
    fake.emitExit(1)
    vi.advanceTimersByTime(50)

    const channels = messages.map((m) => m.channel)
    expect(channels.indexOf(IPC.TERMINAL_DATA)).toBeLessThan(channels.indexOf(IPC.TERMINAL_EXIT))
    // Exactly one flush — the pending timer must not fire a second, empty one.
    // `seq: 1` says the same thing from the other side: the counter a pane uses
    // to tell what it already has moved once, so there was one flush.
    expect(messagesOn(IPC.TERMINAL_DATA)).toEqual([
      { id: session.id, data: 'segfault imminent\n', seq: 1 }
    ])
  })

  it('cancels the idle timer so a crashed session is never re-marked', () => {
    const { session, fake } = createAgent()

    fake.emitData('still working\n')
    fake.emitExit(1)
    const updatesBefore = statusUpdatesFor(session.id).length
    vi.advanceTimersByTime(60_000)

    expect(statusUpdatesFor(session.id)).toHaveLength(updatesBefore)
    expect(session.status).toBe('idle')
  })

  it('drops the captured output of a crashed session', () => {
    const { session, fake } = createAgent()

    fake.emitData('line one\nline two\n')
    expect(ptyManager.getOutput(session.id)).toEqual(['line one', 'line two'])

    fake.emitExit(1)
    // The session is still known (only killPty forgets it) but its scrollback is gone.
    expect(ptyManager.getOutput(session.id)).toEqual([])
  })

  it('asks to clean up the worktree when the last session using it crashes', () => {
    const worktree = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wt-'))
    try {
      const { session, fake } = createAgent({
        existingWorktreePath: worktree,
        branch: 'feature/x'
      })
      expect(session.isWorktree).toBe(true)

      fake.emitExit(1)

      expect(messagesOn(IPC.WORKTREE_CONFIRM_CLEANUP)).toEqual([
        { id: session.id, projectPath: '/tmp/vorn-proj', worktreePath: worktree }
      ])
    } finally {
      fs.rmSync(worktree, { recursive: true, force: true })
    }
  })

  it('keeps the worktree when another pty session still uses it', () => {
    const worktree = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wt-'))
    try {
      const first = createAgent({ existingWorktreePath: worktree, branch: 'feature/x' })
      createAgent({ existingWorktreePath: worktree, branch: 'feature/x' })

      first.fake.emitExit(1)

      expect(messagesOn(IPC.WORKTREE_CONFIRM_CLEANUP)).toEqual([])
    } finally {
      fs.rmSync(worktree, { recursive: true, force: true })
    }
  })

  it('keeps the worktree when a headless session still uses it', () => {
    const worktree = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wt-'))
    try {
      ptyManager.setHeadlessWorktreeCounter(() => ({ count: 1, sessionIds: ['headless-1'] }))
      const { fake } = createAgent({ existingWorktreePath: worktree, branch: 'feature/x' })

      fake.emitExit(1)

      expect(messagesOn(IPC.WORKTREE_CONFIRM_CLEANUP)).toEqual([])
    } finally {
      fs.rmSync(worktree, { recursive: true, force: true })
    }
  })

  it('still reports an exit when killing a session whose pty already died', () => {
    const { session, fake } = createAgent()
    fake.emitExit(139)
    messages = []
    exited = []

    ptyManager.killPty(session.id)

    // The renderer still gets an exit so it can finish its close-intent cleanup.
    expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 0 }])
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
    // A crash leaves the session in the map, so closing the card afterwards
    // repeats session-exit. Its listeners (hook cleanup, save, broadcast) are
    // idempotent, so the repeat is tolerated rather than suppressed.
    expect(exited).toEqual([session])
  })

  it('swallows a kill that fails because the process is already gone', () => {
    const { session, fake } = createAgent()
    fake.killError = new Error('ESRCH')

    expect(() => {
      ptyManager.killPty(session.id)
      vi.advanceTimersByTime(10)
    }).not.toThrow()
    expect(ptyManager.getActiveSessions()).toHaveLength(0)
  })

  it('clears the idle timer when a session is killed', () => {
    const { session, fake } = createAgent()
    fake.emitData('working\n')

    ptyManager.killPty(session.id)
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

  function createRemoteWithStoredKey(): {
    session: TerminalSession
    fake: FakePtyInstance
    keyPath: string
  } {
    const before = new Set(tempKeyFiles())
    ptyManager.setRemoteHosts([{ ...REMOTE_HOST, authMethod: 'key-stored' }])
    const session = ptyManager.createPty(
      remotePayload({ _decryptedKeyContent: 'PRIVATE KEY MATERIAL' })
    )
    const added = tempKeyFiles().filter((f) => !before.has(f))
    expect(added).toHaveLength(1)
    return { session, fake: lastPty(), keyPath: path.join(os.tmpdir(), added[0]) }
  }

  it('stops the remote command and deletes the temp key on an SSH error', () => {
    const { fake, keyPath } = createRemoteWithStoredKey()
    vi.advanceTimersByTime(300)
    expect(fake.written.join(' ')).toContain(`-i ${keyPath}`)

    fake.emitData('ssh: connect to host build.example.com port 22: Connection refused\r\n')

    expect(fs.existsSync(keyPath)).toBe(false)
    // The fallback must be cancelled — a refused connection has no shell to run in.
    vi.advanceTimersByTime(30_000)
    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(false)
  })

  it.each([
    'Permission denied (publickey).',
    'Host key verification failed.',
    'ssh: Could not resolve hostname build.example.com',
    'Connection timed out'
  ])('treats %j as a connection failure', (errorOutput) => {
    ptyManager.setRemoteHosts([REMOTE_HOST])
    ptyManager.createPty(remotePayload())
    const fake = lastPty()
    vi.advanceTimersByTime(300)

    fake.emitData(`${errorOutput}\r\n`)
    vi.advanceTimersByTime(30_000)

    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(false)
  })

  it('runs the remote command and deletes the temp key once the marker arrives', () => {
    const { session, fake, keyPath } = createRemoteWithStoredKey()
    vi.advanceTimersByTime(300)

    fake.emitData(`__VORN_READY_${session.id.slice(0, 8)}__\r\n`)
    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(false)

    vi.advanceTimersByTime(200)
    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(true)
    expect(fs.existsSync(keyPath)).toBe(false)
  })

  it('falls back to sending the remote command when the marker never arrives', () => {
    ptyManager.createPty(remotePayload())
    const fake = lastPty()

    vi.advanceTimersByTime(300)
    fake.emitData('welcome to a very non-standard shell\r\n')
    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(false)

    vi.advanceTimersByTime(8000)
    expect(fake.written.some((w) => w.includes('cd /srv/proj'))).toBe(true)
  })

  it('never writes to a pty that was killed before the fallback fired', () => {
    const session = ptyManager.createPty(remotePayload())
    const fake = lastPty()

    ptyManager.killPty(session.id)
    vi.advanceTimersByTime(30_000)

    expect(fake.written).toEqual([])
  })

  it('falls back to agent auth when stored-key auth has no decrypted key', () => {
    const before = new Set(tempKeyFiles())
    ptyManager.setRemoteHosts([{ ...REMOTE_HOST, authMethod: 'key-stored' }])
    ptyManager.createPty(remotePayload())
    const fake = lastPty()

    vi.advanceTimersByTime(300)
    expect(fake.written[0]).not.toContain('-i ')
    expect(tempKeyFiles().filter((f) => !before.has(f))).toEqual([])
  })

  it('deletes the temp key when the SSH session drops before connecting', () => {
    const { session, fake, keyPath } = createRemoteWithStoredKey()
    vi.advanceTimersByTime(300)

    fake.emitExit(255)

    expect(fs.existsSync(keyPath)).toBe(false)
    expect(messagesOn(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 255 }])
  })

  it('answers a password prompt once, with the real password', () => {
    ptyManager.setRemoteHosts([{ ...REMOTE_HOST, authMethod: 'password' }])
    ptyManager.createPty(remotePayload({ _decryptedPassword: 'hunter2' }))
    const fake = lastPty()
    vi.advanceTimersByTime(300)

    fake.emitData("dev@build.example.com's password: ")
    vi.advanceTimersByTime(50)
    expect(fake.written).toContain('hunter2\r')

    // A second prompt (wrong password re-ask) must not be auto-answered again.
    fake.emitData("dev@build.example.com's password: ")
    vi.advanceTimersByTime(50)
    expect(fake.written.filter((w) => w === 'hunter2\r')).toHaveLength(1)
  })

  it('stops answering password prompts after the listener window closes', () => {
    ptyManager.setRemoteHosts([{ ...REMOTE_HOST, authMethod: 'password' }])
    ptyManager.createPty(remotePayload({ _decryptedPassword: 'hunter2' }))
    const fake = lastPty()

    vi.advanceTimersByTime(15_000)
    fake.emitData("dev@build.example.com's password: ")
    vi.advanceTimersByTime(100)

    expect(fake.written).not.toContain('hunter2\r')
  })

  it('keeps the decrypted credentials off the payload', () => {
    ptyManager.setRemoteHosts([{ ...REMOTE_HOST, authMethod: 'password' }])
    const payload = remotePayload({ _decryptedPassword: 'hunter2' })
    ptyManager.createPty(payload)

    expect(payload._decryptedPassword).toBeUndefined()
    expect(payload._decryptedKeyContent).toBeUndefined()
  })
})

describe('idle timeout', () => {
  it('marks a silent session idle after the pattern timeout', () => {
    const { session, fake } = createAgent()

    fake.emitData('thinking about it\n')
    vi.advanceTimersByTime(4999)
    expect(session.status).toBe('running')

    vi.advanceTimersByTime(1)
    expect(session.status).toBe('idle')
    expect(statusUpdatesFor(session.id)).toEqual(['idle'])
  })

  it('restarts the countdown on every chunk of output', () => {
    const { session, fake } = createAgent()

    fake.emitData('step one\n')
    vi.advanceTimersByTime(4000)
    fake.emitData('step two\n')
    vi.advanceTimersByTime(4000)
    expect(session.status).toBe('running')

    vi.advanceTimersByTime(1000)
    expect(session.status).toBe('idle')
  })

  it('leaves a session that is waiting for input alone', () => {
    const { session, fake } = createAgent()

    fake.emitData('\x1b[?2004hprompt> ')
    expect(session.status).toBe('waiting')

    vi.advanceTimersByTime(60_000)
    expect(session.status).toBe('waiting')
  })

  it('gives hook-backed sessions the longer timeout', () => {
    const { session, fake } = createAgent()

    fake.emitData('running a tool\n')
    ptyManager.promoteToHookStatus(session.id)
    expect(session.statusSource).toBe('hooks')

    vi.advanceTimersByTime(5000)
    expect(session.status).toBe('running')

    vi.advanceTimersByTime(25_000)
    expect(session.status).toBe('idle')
  })

  it('re-arms the hook timeout on every hook event', () => {
    const { session, fake } = createAgent()
    fake.emitData('running a tool\n')

    ptyManager.promoteToHookStatus(session.id)
    vi.advanceTimersByTime(20_000)
    ptyManager.promoteToHookStatus(session.id)
    vi.advanceTimersByTime(20_000)
    expect(session.status).toBe('running')

    vi.advanceTimersByTime(10_000)
    expect(session.status).toBe('idle')
  })

  it('does not arm a timer when promoting a session that has produced no output', () => {
    const { session } = createAgent()

    ptyManager.promoteToHookStatus(session.id)
    vi.advanceTimersByTime(60_000)

    // Nothing to time out yet — the timer is armed by the first output chunk.
    expect(session.status).toBe('running')
  })

  it('ignores promotion of an unknown session', () => {
    expect(() => ptyManager.promoteToHookStatus('no-such-session')).not.toThrow()
  })

  it('revives an idle session when the user types', () => {
    const { session, fake } = createAgent()
    fake.emitData('done\n')
    vi.advanceTimersByTime(5000)
    expect(session.status).toBe('idle')

    ptyManager.writeToPty(session.id, 'next task\r')

    expect(session.status).toBe('running')
    expect(statusUpdatesFor(session.id)).toEqual(['idle', 'running'])
    expect(fake.written).toContain('next task\r')
  })

  it('leaves hook-backed sessions to their hooks when the user types', () => {
    const { session, fake } = createAgent()
    fake.emitData('done\n')
    ptyManager.promoteToHookStatus(session.id)
    vi.advanceTimersByTime(30_000)
    expect(session.status).toBe('idle')

    ptyManager.writeToPty(session.id, 'next task\r')

    expect(session.status).toBe('idle')
  })

  it('never arms an idle timer for plain shell sessions', () => {
    const session = ptyManager.createShellPty('/tmp')
    const fake = lastPty()

    fake.emitData('$ ls\n')
    vi.advanceTimersByTime(60_000)

    expect(session.status).toBe('running')
    expect(statusUpdatesFor(session.id)).toEqual([])
  })

  it('records the exit code when a shell session ends', () => {
    const session = ptyManager.createShellPty('/tmp')
    const fake = lastPty()

    fake.emitExit(130)

    expect(session.status).toBe('idle')
    expect(session.shellExitCode).toBe(130)
  })

  it('ignores writes and resizes for a session that no longer exists', () => {
    expect(() => ptyManager.writeToPty('gone', 'hello')).not.toThrow()
    expect(() => ptyManager.resizePty('gone', 80, 24)).not.toThrow()
    expect(() => ptyManager.killPty('gone')).not.toThrow()
  })
})

describe('the bell', () => {
  it('rings for output that is actually arriving', () => {
    const { session, fake } = createAgent()

    fake.emitData('done \x07')
    vi.advanceTimersByTime(50)

    expect(messagesOn(IPC.TERMINAL_BELL)).toEqual([{ id: session.id }])
  })

  it('rings for a plain shell too', () => {
    // The status analysis above this returns early for a shell. A bell is not
    // status -- a shell that rings wants you just as much as an agent does.
    const session = ptyManager.createShellPty('/tmp/vorn-proj')
    const fake = lastPty()

    fake.emitData('\x07')
    vi.advanceTimersByTime(50)

    expect(messagesOn(IPC.TERMINAL_BELL)).toEqual([{ id: session.id }])
  })

  it('stays quiet for output with no bell in it', () => {
    const { fake } = createAgent()

    fake.emitData('perfectly ordinary output\n')
    vi.advanceTimersByTime(50)

    expect(messagesOn(IPC.TERMINAL_BELL)).toEqual([])
  })

  it('does not ring for a screen that is only being replayed', () => {
    // The property this replaces was enforced in the client, which had to know
    // which bytes were a seed and which were live. Here it holds by
    // construction: a replay is read from the scrollback and never passes
    // through the flush that announces one.
    const { session, fake } = createAgent()
    fake.emitData('ding \x07 ding')
    vi.advanceTimersByTime(50)
    messages.length = 0

    // Whatever a pane does with the scrollback afterwards, it is not output.
    ptyManager.getOutput(session.id, 100)
    vi.advanceTimersByTime(50)

    expect(messagesOn(IPC.TERMINAL_BELL)).toEqual([])
  })
})
