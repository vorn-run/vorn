import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import path from 'node:path'
import type { CreateTerminalPayload, TerminalSession } from '@vornrun/shared/types'

/**
 * The screen model, where there is a real PTY to watch.
 *
 * The mock harness below is duplicated from `pty-manager-recovery.test.ts`, and
 * it is duplicated on purpose rather than by neglect. `vi.mock` is hoisted into
 * the file that calls it; moved to a shared module it registers after that
 * file's imports have already resolved, so the real `node-pty` is loaded and the
 * suite spawns actual shells. Extracting it needs the factory form
 * (`vi.mock(spec, () => import(helper))`) in both files, which trades a hundred
 * lines of obvious boilerplate for a subtlety that fails silently. If a third
 * file ever needs this, that trade is worth making.
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

import fs from 'node:fs'
import os from 'node:os'
import { ptyManager } from '../packages/server/src/pty-manager'
import { isGitRepo } from '../packages/server/src/git-utils'
import {
  configureHistory,
  settleHistory,
  resetHistory
} from '../packages/server/src/history/writer'
import { historyDir, LOG_FILE } from '../packages/server/src/history/checkpoint'
import { readFrames, type Frame } from '../packages/server/src/history/log'
import { readScrollback, resetScrollback } from '../packages/server/src/terminal-scrollback'

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

  it('ignores a resize larger than a frame can record', () => {
    // A resize frame stores its dimensions in sixteen bits, so seventy thousand
    // columns would be written to disk as four thousand -- a durable
    // disagreement between what the program rendered against and what a replay
    // lays it out at, re-applied on every start. Refused at the source, where
    // the PTY and the model and the frame all still agree.
    const { session, fake } = createAgent()
    ptyManager.resizePty(session.id, 70_000, 40)

    expect(fake.resize).not.toHaveBeenCalled()
    expect(ptyManager.getActiveSessions().find((s) => s.id === session.id)?.cols).toBe(80)
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
    expect((await serializeScreen(session.id))?.screen).toContain('output')

    ptyManager.killPty(session.id)

    expect(screenCount()).toBe(held - 1)
    expect(await serializeScreen(session.id)).toBeNull()
  })
})

describe('the terminal is recorded where it is fed', () => {
  /**
   * The wiring, which every other history test has to take on trust.
   *
   * `history-writer.test.ts` drives the writer directly and proves what it does
   * with what it is given. Nothing there can show that `pty-manager` gives it
   * anything at all -- four call sites that are one line each, and a one-line
   * call site is exactly the kind that gets dropped in a merge and noticed
   * months later as terminals that come back blank.
   */
  let dir: string

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-pty-history-'))
    configureHistory(dir, { tickMs: 5, quiesceMs: 5_000, checkpointMs: 60_000 })
  })

  afterEach(() => {
    resetHistory()
    fs.rmSync(dir, { recursive: true, force: true })
  })

  async function settled(): Promise<void> {
    for (let round = 0; round < 4; round++) {
      await new Promise((r) => setTimeout(r, 20))
      await settleHistory()
    }
  }

  function framesFor(id: string): Frame[] {
    return readFrames(fs.readFileSync(path.join(historyDir(dir, id), LOG_FILE))).frames
  }

  it('does not let the byte buffer run ahead of the screen model', async () => {
    // Both are fed from the flush, and that is what makes a checkpoint coherent.
    // The buffer used to be fed from `onData` instead, so for up to one flush it
    // held bytes the model had not seen -- and a checkpoint takes both at the
    // same instant. Those bytes went into its scrollback, arrived again as log
    // frames written after it, and a restore counted them twice.
    resetScrollback()
    const { session, fake } = createAgent()
    fake.emitData('printed but not yet flushed')

    expect(readScrollback(session.id), 'the buffer saw it before the model did').toBe('')

    await afterFlush()
    expect(readScrollback(session.id)).toContain('printed but not yet flushed')
  })

  it('opens a log when a terminal is spawned', async () => {
    const { session } = createAgent()
    await settled()

    expect(fs.existsSync(path.join(historyDir(dir, session.id), LOG_FILE))).toBe(true)
  })

  it('records what the terminal printed, and the size it printed at', async () => {
    const { session, fake } = createAgent()
    fake.emitData('tests passed, 402 of them\r\n')
    await afterFlush()
    ptyManager.resizePty(session.id, 132, 43)
    await settled()

    expect(framesFor(session.id)).toEqual(
      expect.arrayContaining<Frame>([
        { kind: 'output', data: 'tests passed, 402 of them\r\n' },
        { kind: 'resize', cols: 132, rows: 43 }
      ])
    )
  })

  it('records once per flush rather than once per chunk', async () => {
    // The reason the call sits in `flushBuffer` and not in `onData`. Thirty
    // keystrokes are one frame, not thirty -- and it is what keeps the byte
    // buffer and the screen model seeing the same bytes at the same moment.
    const { session, fake } = createAgent()
    for (let i = 0; i < 30; i++) fake.emitData('x')
    await afterFlush()
    await settled()

    const output = framesFor(session.id).filter((f) => f.kind === 'output')
    expect(output).toEqual([{ kind: 'output', data: 'x'.repeat(30) }])
  })

  it('takes the history with the terminal when it is killed', async () => {
    const { session } = createAgent()
    await settled()
    expect(fs.existsSync(historyDir(dir, session.id))).toBe(true)

    ptyManager.killPty(session.id)
    await settled()

    expect(fs.existsSync(historyDir(dir, session.id))).toBe(false)
  })
})

describe('the seam between a session and the one resuming it', () => {
  it('puts the reset in the output stream, ahead of the new run', async () => {
    // A resumed session keeps its pane, so the new process inherits whatever the
    // last one left in the emulator -- a scroll region, an alternate screen it
    // never came back from. Something has to sit between the two runs saying so,
    // and it has to be in the stream rather than written by a client: a cold
    // pane has not mounted when the resume starts, and the screen it replays is
    // written when it finally does, by which time the new run is already
    // streaming. Ordering these is the server's job because the server is what
    // orders them.
    const { session, fake } = createAgent()
    fake.emitData('what the last run left')
    await afterFlush()

    ptyManager.injectOutput(session.id, `${ESC}[!p`)
    fake.emitData('what the new run draws')
    await afterFlush()

    const seen = readScrollback(session.id)
    expect(seen).toContain(`${ESC}[!p`)
    expect(seen.indexOf(`${ESC}[!p`)).toBeGreaterThan(seen.indexOf('what the last run left'))
    expect(seen.indexOf(`${ESC}[!p`)).toBeLessThan(seen.indexOf('what the new run draws'))
  })

  it('never answers it back down the pty, as any injected escape must not', async () => {
    const { session, fake } = createAgent()
    const before = fake.written.length

    ptyManager.injectOutput(session.id, `${ESC}[!p${ESC}[?1049l`)
    await afterFlush()

    expect(fake.written.slice(before)).toEqual([])
  })
})

describe('letting go of a session that is about to come back', () => {
  it('announces nothing, where killing one announces an exit', () => {
    // Resume used to route through `killPty`, which emits `session-exit` for a
    // session that is returning under the same id and -- when it was the last
    // one in a worktree -- broadcasts WORKTREE_CONFIRM_CLEANUP. That reaches the
    // person as an offer to delete the worktree the agent is at that moment
    // being resumed into, and taking it removes the tree under a running agent.
    const said: string[] = []
    const onExit = (): void => void said.push('session-exit')
    const onMessage = (channel: string): void => void said.push(channel)
    ptyManager.on('session-exit', onExit)
    ptyManager.on('client-message', onMessage)

    try {
      const coming_back = createAgent()
      ptyManager.releaseForResume(coming_back.session.id)
      expect(said).toEqual([])

      // The contrast, so this cannot pass by nothing being emitted at all.
      const going = createAgent()
      ptyManager.killPty(going.session.id)
      expect(said).toContain('session-exit')
    } finally {
      ptyManager.off('session-exit', onExit)
      ptyManager.off('client-message', onMessage)
    }
  })

  it('leaves the history alone, because the run replacing it resets that', () => {
    // `killPty` calls `stopHistory`, which queues a recursive remove of the very
    // directory `startHistory` resets a few lines later -- two queues over one
    // directory, which the writer is built to never have.
    const { session } = createAgent()
    ptyManager.releaseForResume(session.id)

    expect(ptyManager.hasLivePty(session.id)).toBe(false)
    expect(ptyManager.getActiveSessions().some((s) => s.id === session.id)).toBe(false)
  })
})

describe('a release whose spawn then fails', () => {
  it('can be put back, because otherwise the session is gone for good', () => {
    // Releasing is destructive on purpose -- it is what lets the replacement
    // take the same id -- but a spawn that throws must not end the session. The
    // carried-over kind is handed back to `restored-sessions`; this is the other
    // kind, whose record lives in the pty manager, and it was released and never
    // put anywhere. The pane's next attempt was told the session was gone.
    const { session } = createAgent()
    ptyManager.releaseForResume(session.id)
    expect(ptyManager.getActiveSessions().some((s) => s.id === session.id)).toBe(false)

    ptyManager.restoreReleased(session)

    expect(ptyManager.getActiveSessions().some((s) => s.id === session.id)).toBe(true)
    // Still no process behind it, which is what makes it resumable rather than live.
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
  })

  it('does not put the id in the order twice', () => {
    const { session } = createAgent()
    ptyManager.releaseForResume(session.id)
    ptyManager.restoreReleased(session)
    ptyManager.restoreReleased(session)

    expect(ptyManager.getActiveSessions().filter((s) => s.id === session.id)).toHaveLength(1)
  })
})
