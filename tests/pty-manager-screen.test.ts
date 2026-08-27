import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import path from 'node:path'
import type { CreateTerminalPayload, TerminalSession } from '@vornrun/shared/types'

/**
 * The screen model, where there is a real PTY to watch.
 *
 * `terminal-screen.test.ts` proves what a screen becomes; it cannot prove that
 * feeding one never writes back. That needs something on the other end of the
 * PTY recording what arrives, which is what `FakePty` is -- every `write` lands
 * in an array this can read.
 *
 * The failure being guarded is quiet and expensive: an emulator asked who it is
 * answers, and an answer sent to a PTY arrives in the shell's *input* as
 * characters the user did not type.
 */

// `FakePty` is here for its type only -- `lastPty` needs the instance shape.
// eslint-disable-next-line @typescript-eslint/no-unused-vars
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
import { isGitRepo } from '../packages/server/src/git-utils'

vi.mocked(isGitRepo).mockReturnValue(false)

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

const ESC = '\x1b'

/**
 * Wait for the output coalescer.
 *
 * `emitData` only buffers; `flushBuffer` runs on an 8 ms timer and that is where
 * the screen is fed. An assertion made straight after `emitData` is made before
 * anything under test has happened -- which is how the first version of these
 * passed against a `pty-manager` deliberately rigged to write replies back.
 */
const afterFlush = (): Promise<void> => new Promise((r) => setTimeout(r, 40))

beforeEach(() => {
  // Every session this file starts is torn down, so a count taken in one test
  // is not a count of what an earlier one left running.
  for (const s of ptyManager.getActiveSessions()) ptyManager.killPty(s.id)
  spawnMock.mockClear()
})

afterEach(() => {
  for (const s of ptyManager.getActiveSessions()) ptyManager.killPty(s.id)
})

describe('feeding the screen model never writes to the PTY', () => {
  it('stays silent when a program asks the terminal who it is', async () => {
    const { fake } = createAgent()
    const before = fake.written.length

    // A device-attributes query, a cursor-position report and a mode query --
    // three things a terminal answers, arriving as ordinary program output.
    fake.emitData(`${ESC}[c${ESC}[6n${ESC}[?1049$p`)
    await afterFlush()

    expect(fake.written.slice(before)).toEqual([])
  })

  it('stays silent across a resize, which also makes a terminal talkative', async () => {
    const { session, fake } = createAgent()
    fake.emitData('some output')
    await afterFlush()
    const before = fake.written.length

    ptyManager.resizePty(session.id, 120, 40)
    fake.emitData(`${ESC}[18t`)
    await afterFlush()

    expect(fake.written.slice(before)).toEqual([])
  })

  it('still delivers what the user actually types', () => {
    // The counterpart: silence would be worthless if it meant the PTY heard
    // nothing at all.
    const { session, fake } = createAgent()

    ptyManager.writeToPty(session.id, 'ls -la\r')

    expect(fake.written).toContain('ls -la\r')
  })
})

describe('the model follows the session it belongs to', () => {
  it('records the geometry a resize asked for', () => {
    const { session } = createAgent()

    ptyManager.resizePty(session.id, 132, 43)

    const live = ptyManager.getActiveSessions().find((s) => s.id === session.id)
    expect(live?.cols).toBe(132)
    expect(live?.rows).toBe(43)
  })

  it('ignores a resize that would throw inside node-pty', () => {
    // Arrives as a fire-and-forget notification, so a throw here has no caller.
    const { session, fake } = createAgent()

    expect(() => ptyManager.resizePty(session.id, 0, 0)).not.toThrow()

    expect(fake.resize).not.toHaveBeenCalled()
    const live = ptyManager.getActiveSessions().find((s) => s.id === session.id)
    expect(live?.cols).toBe(80)
  })

  it('lets go of the model when the terminal is killed', async () => {
    // A `Terminal` holds buffers, so a map delete is not enough -- miss the
    // dispose and every session ever closed stays resident. Counted as a delta
    // rather than an absolute, because other sessions in this file are alive.
    const { screenCount, serializeScreen } = await import('../packages/server/src/terminal-screen')
    const { session, fake } = createAgent()
    fake.emitData('output')
    await afterFlush()
    const held = screenCount()
    expect(await serializeScreen(session.id)).toContain('output')

    ptyManager.killPty(session.id)

    expect(screenCount()).toBe(held - 1)
    expect(await serializeScreen(session.id)).toBe('')
  })
})
