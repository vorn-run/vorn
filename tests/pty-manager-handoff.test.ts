import { describe, it, expect, vi, beforeEach } from 'vitest'
import path from 'node:path'
import type { CreateTerminalPayload, TerminalSession } from '@vornrun/shared/types'

/**
 * What the pty manager contributes to a handoff.
 *
 * The mock harness mirrors `pty-manager-screen.test.ts`, duplicated for the
 * reason stated there: `vi.mock` is hoisted per file, so a shared module
 * registers too late and the suite spawns real shells.
 *
 * The fake pty carries an `fd`, because that is the whole question here — a pane
 * this manager cannot name a descriptor for is a machine that must not be handed
 * over at all.
 */
// `FakePty` is here for its type only -- `FakePtyInstance` needs the shape.
// eslint-disable-next-line @typescript-eslint/no-unused-vars
const { spawnMock, FakePty } = vi.hoisted(() => {
  type DataHandler = (data: string) => void
  type ExitHandler = (e: { exitCode: number; signal?: number }) => void

  let nextPid = 5000
  let nextFd = 20

  class FakePty {
    pid = nextPid++
    /** node-pty exposes this on the instance; the typings never declared it. */
    fd: number | undefined = nextFd++
    written: string[] = []
    paused = 0
    resumed = 0
    resize = vi.fn()
    private dataHandlers: DataHandler[] = []
    private exitHandlers: ExitHandler[] = []

    write(data: string): void {
      this.written.push(data)
    }
    kill(): void {}
    pause(): void {
      this.paused += 1
    }
    resume(): void {
      this.resumed += 1
    }
    onData(cb: DataHandler): { dispose: () => void } {
      this.dataHandlers.push(cb)
      return { dispose: () => (this.dataHandlers = this.dataHandlers.filter((h) => h !== cb)) }
    }
    onExit(cb: ExitHandler): { dispose: () => void } {
      this.exitHandlers.push(cb)
      return { dispose: () => (this.exitHandlers = this.exitHandlers.filter((h) => h !== cb)) }
    }
    emitData(data: string): void {
      for (const h of [...this.dataHandlers]) h(data)
    }
  }

  return { spawnMock: vi.fn(() => new FakePty()), FakePty }
})

type FakePtyInstance = InstanceType<typeof FakePty>

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
  getGitHead: vi.fn(() => 'cafe0000'),
  checkoutBranch: vi.fn(),
  createWorktree: vi.fn(),
  extractWorktreeName: vi.fn((p: string) => path.basename(p)),
  isGitRepo: vi.fn(() => false)
}))

vi.mock('../packages/server/src/shell-integration', () => ({
  getShellIntegration: vi.fn(() => ({ env: {} }))
}))

vi.mock('../packages/server/src/agent-launch', () => ({
  buildAgentLaunchLine: vi.fn((payload: CreateTerminalPayload) => `${payload.agentType}-launch`)
}))

vi.mock('../packages/server/src/process-utils', async () => {
  const actual = await vi.importActual<typeof import('../packages/server/src/process-utils')>(
    '../packages/server/src/process-utils'
  )
  return {
    ...actual,
    getSafeEnv: () => ({ HOME: '/home/user', PATH: '/usr/bin' }),
    getLaunchEnv: () => ({ HOME: '/home/user', PATH: '/usr/bin' })
  }
})

import { ptyManager } from '../packages/server/src/pty-manager'
import { hasScreen } from '../packages/server/src/terminal-screen'
import type { AdoptedPane } from '../packages/server/src/handoff/heir'
import type { AdoptedPty } from '../packages/server/src/handoff/adopted-pty'

function createAgent(name: string): { session: TerminalSession; fake: FakePtyInstance } {
  const session = ptyManager.createPty({
    agentType: 'claude',
    projectName: name,
    projectPath: `/tmp/${name}`
  })
  const results = spawnMock.mock.results
  return { session, fake: results[results.length - 1]!.value as FakePtyInstance }
}

beforeEach(() => {
  for (const session of ptyManager.getActiveSessions()) ptyManager.killPty(session.id)
  spawnMock.mockClear()
})

describe('describing a machine for a handoff', () => {
  it('names every live pane, in the order they are held', () => {
    const first = createAgent('one')
    const second = createAgent('two')

    const panes = ptyManager.describeForHandoff()
    expect(panes?.map((p) => p.session.id)).toEqual([first.session.id, second.session.id])
    expect(panes?.map((p) => p.fd)).toEqual([first.fake.fd, second.fake.fd])
    expect(panes?.map((p) => p.pid)).toEqual([first.fake.pid, second.fake.pid])
    // The geometry the program is rendering against, not a default.
    expect(panes?.[0]?.cols).toBe(first.session.cols)
    expect(panes?.[0]?.rows).toBe(first.session.rows)
  })

  it('follows a resize, so the replacement rebuilds at the right width', () => {
    const { session } = createAgent('resized')
    ptyManager.resizePty(session.id, 132, 43)

    const pane = ptyManager.describeForHandoff()?.[0]
    expect(pane?.cols).toBe(132)
    expect(pane?.rows).toBe(43)
  })

  it('refuses the whole machine when one pane has no descriptor', () => {
    createAgent('describable')
    const { fake } = createAgent('not-describable')
    // What win32 looks like from here: a pty with nothing to hand over.
    fake.fd = undefined

    // Null, not a shorter list. Half the terminals arriving is, to the person
    // whose terminals they are, the same as losing the other half.
    expect(ptyManager.describeForHandoff()).toBeNull()
  })

  it('stops and starts every reader', () => {
    const first = createAgent('one')
    const second = createAgent('two')

    ptyManager.pauseAllForHandoff()
    expect([first.fake.paused, second.fake.paused]).toEqual([1, 1])

    ptyManager.resumeAllForHandoff()
    expect([first.fake.resumed, second.fake.resumed]).toEqual([1, 1])
  })
})

describe('taking panes from the previous server', () => {
  /** Only the members `adoptPanes` touches; the real class is covered elsewhere. */
  function inherited(id: string): { pane: AdoptedPane; resumed: () => number } {
    let resumed = 0
    const pty = {
      pid: 7777,
      write: vi.fn(),
      resize: vi.fn(),
      kill: vi.fn(),
      onData: vi.fn(() => ({ dispose: vi.fn() })),
      onExit: vi.fn(() => ({ dispose: vi.fn() })),
      pause: vi.fn(),
      resume: () => {
        resumed += 1
      }
    }
    return {
      pane: {
        session: {
          id,
          agentType: 'claude',
          projectName: 'carried',
          projectPath: '/tmp/carried',
          status: 'running',
          createdAt: Date.now(),
          cols: 100,
          rows: 30,
          pid: 7777
        } as TerminalSession,
        pty: pty as unknown as AdoptedPty,
        cols: 100,
        rows: 30
      },
      resumed: () => resumed
    }
  }

  it('makes an inherited pane an ordinary live session', () => {
    const { pane } = inherited('carried-1')
    ptyManager.adoptPanes([pane])

    expect(ptyManager.getActiveSessions().map((s) => s.id)).toContain('carried-1')
    expect(ptyManager.hasLivePty('carried-1')).toBe(true)
    expect(ptyManager.livePtyCount()).toBe(1)
  })

  it('reads only once every pane is wired', () => {
    const one = inherited('carried-1')
    const two = inherited('carried-2')
    ptyManager.adoptPanes([one.pane, two.pane])

    // Resumed after both were attached, so a burst on one cannot arrive while the
    // other is still being wired.
    expect(one.resumed()).toBe(1)
    expect(two.resumed()).toBe(1)
  })

  it('gives an inherited pane a screen when recovery found none', () => {
    const { pane } = inherited('carried-3')
    ptyManager.adoptPanes([pane])
    expect(hasScreen('carried-3')).toBe(true)
  })

  it('writes and resizes through to the inherited pty', () => {
    const { pane } = inherited('carried-4')
    ptyManager.adoptPanes([pane])

    ptyManager.writeToPty('carried-4', 'hello')
    ptyManager.resizePty('carried-4', 90, 20)
    expect(pane.pty.write).toHaveBeenCalledWith('hello')
    expect(pane.pty.resize).toHaveBeenCalledWith(90, 20)
  })

  it('hands on what it was handed, so a second update is survivable', () => {
    const { pane } = inherited('carried-5')
    ptyManager.adoptPanes([pane])
    // An adopted pty exposes `fd` under the same name a forked one does.
    ;(pane.pty as unknown as { fd: number }).fd = 41

    expect(ptyManager.describeForHandoff()).toEqual([
      expect.objectContaining({ fd: 41, pid: 7777, cols: 100, rows: 30 })
    ])
  })
})
