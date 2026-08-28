import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

// Mock logger and filesystem to prevent side effects
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import {
  initTestDatabase,
  saveSessions,
  getPreviousSessions,
  clearSessions
} from '../packages/server/src/database'
import type { TerminalSession } from '@vornrun/shared/types'

let teardown: () => void

function makeSession(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'test-id-1',
    agentType: 'claude',
    projectName: 'test-project',
    projectPath: '/test/project',
    status: 'running',
    createdAt: Date.now(),
    pid: 12345,
    ...overrides
  }
}

beforeEach(() => {
  teardown = initTestDatabase()
})

afterEach(() => {
  teardown()
})

describe('session persistence (real SQLite)', () => {
  it('saves and loads sessions with all fields', () => {
    const session = makeSession({
      displayName: 'my session',
      branch: 'main',
      worktreePath: '/test/worktree',
      worktreeName: 'friendly-name',
      isWorktree: true,
      remoteHostId: 'host-1',
      remoteHostLabel: 'my-server',
      hookSessionId: 'hook-uuid-123',
      statusSource: 'hooks'
    })

    saveSessions([session])
    const loaded = getPreviousSessions()

    expect(loaded).toHaveLength(1)
    expect(loaded[0].id).toBe(session.id)
    expect(loaded[0].agentType).toBe('claude')
    expect(loaded[0].projectName).toBe('test-project')
    expect(loaded[0].projectPath).toBe('/test/project')
    expect(loaded[0].displayName).toBe('my session')
    expect(loaded[0].branch).toBe('main')
    expect(loaded[0].worktreePath).toBe('/test/worktree')
    expect(loaded[0].worktreeName).toBe('friendly-name')
    expect(loaded[0].isWorktree).toBe(true)
    expect(loaded[0].remoteHostId).toBe('host-1')
    expect(loaded[0].remoteHostLabel).toBe('my-server')
    expect(loaded[0].hookSessionId).toBe('hook-uuid-123')
    expect(loaded[0].statusSource).toBe('hooks')
  })

  it('hookSessionId round-trips through save/load', () => {
    const session = makeSession({ hookSessionId: 'abc-def-123' })
    saveSessions([session])
    const loaded = getPreviousSessions()
    expect(loaded[0].hookSessionId).toBe('abc-def-123')
  })

  it('worktreeName round-trips through save/load', () => {
    const session = makeSession({
      worktreeName: 'galactic-eclipse',
      worktreePath: '/test/wt',
      isWorktree: true
    })
    saveSessions([session])
    const loaded = getPreviousSessions()
    expect(loaded[0].worktreeName).toBe('galactic-eclipse')
  })

  it('returns sessions in sort_order', () => {
    const sessions = [
      makeSession({ id: 'a', displayName: 'first' }),
      makeSession({ id: 'b', displayName: 'second' }),
      makeSession({ id: 'c', displayName: 'third' })
    ]
    saveSessions(sessions)
    const loaded = getPreviousSessions()
    expect(loaded.map((s) => s.id)).toEqual(['a', 'b', 'c'])
  })

  it('clearSessions removes all rows', () => {
    saveSessions([makeSession()])
    clearSessions()
    expect(getPreviousSessions()).toHaveLength(0)
  })

  it('saveSessions replaces previous sessions', () => {
    saveSessions([makeSession({ id: 'old' })])
    saveSessions([makeSession({ id: 'new' })])
    const loaded = getPreviousSessions()
    expect(loaded).toHaveLength(1)
    expect(loaded[0].id).toBe('new')
  })

  it('empty sessions array results in empty table', () => {
    saveSessions([makeSession()])
    saveSessions([])
    expect(getPreviousSessions()).toHaveLength(0)
  })

  it('sessions with null optional fields survive round-trip', () => {
    const session = makeSession()
    saveSessions([session])
    const loaded = getPreviousSessions()
    expect(loaded[0].displayName).toBeUndefined()
    expect(loaded[0].branch).toBeUndefined()
    expect(loaded[0].worktreePath).toBeUndefined()
    expect(loaded[0].worktreeName).toBeUndefined()
    expect(loaded[0].isWorktree).toBeUndefined()
    expect(loaded[0].hookSessionId).toBeUndefined()
  })
})

describe('what a shell needs to come back where it was', () => {
  it('round-trips the directory the shell was actually in', () => {
    // The column never existed, so `shellCwd` wrote to the record in memory and
    // then vanished on read -- and restoring a shell always fell back to its
    // project directory, which for anybody who had navigated anywhere was the
    // wrong place.
    saveSessions([makeSession({ agentType: 'shell', shellCwd: '/Users/x/dev/vorn/packages' })])

    expect(getPreviousSessions()[0].shellCwd).toBe('/Users/x/dev/vorn/packages')
  })

  it('leaves it absent when a session never reported one', () => {
    saveSessions([makeSession()])
    expect(getPreviousSessions()[0].shellCwd).toBeUndefined()
  })

  it('hands back when the record was last written down', () => {
    // The only thing on disk that says roughly when a run ended, which is what
    // a pane restored from a previous process has to tell somebody. It has been
    // selected and discarded since this table was written.
    const before = Date.now()
    saveSessions([makeSession()])

    expect(getPreviousSessions()[0].savedAt).toBeGreaterThanOrEqual(before)
  })
})

describe('verifySchema (real SQLite)', () => {
  it('is idempotent — running initTestDatabase twice does not throw', () => {
    teardown()
    expect(() => {
      teardown = initTestDatabase()
    }).not.toThrow()
  })
})
