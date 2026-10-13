// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { TerminalSession, AgentType } from '../packages/shared/src/types'

// Mock window.api (needed by addTerminal)
Object.defineProperty(window, 'api', {
  value: { notifyWidgetStatus: vi.fn() },
  writable: true
})

import { create } from 'zustand'
import { createTerminalsSlice } from '../packages/web/src/renderer/stores/terminals-slice'
import type { TerminalsSlice } from '../packages/web/src/renderer/stores/types'

function makeSession(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'term-1',
    agentType: 'claude' as AgentType,
    projectName: 'my-app',
    projectPath: '/home/user/my-app',
    status: 'running',
    createdAt: Date.now(),
    pid: 1234,
    branch: 'main',
    ...overrides
  }
}

// Create a minimal store with just the terminals slice.
// The slice creator expects AppStore but only accesses TerminalsSlice fields,
// so casting is safe for unit tests.
function createStore() {
  return create<TerminalsSlice>()((...a) => ({
    ...createTerminalsSlice(...(a as Parameters<typeof createTerminalsSlice>)),
    // stub fields from other slices that the cast expects
    terminalOrder: [] as string[],
    minimizedTerminals: new Set<string>(),
    gitDiffStats: new Map()
  }))
}

describe('updateSessionBranch', () => {
  let store: ReturnType<typeof createStore>

  beforeEach(() => {
    store = createStore()
    store.getState().addTerminal(makeSession({ id: 'term-1', branch: 'main' }))
  })

  it('updates branch when it differs from current', () => {
    store.getState().updateSessionBranch('term-1', 'feature/new')
    const term = store.getState().terminals.get('term-1')
    expect(term?.session.branch).toBe('feature/new')
  })

  it('returns same state when branch is unchanged', () => {
    const before = store.getState().terminals
    store.getState().updateSessionBranch('term-1', 'main')
    const after = store.getState().terminals
    // Same reference means no unnecessary re-render
    expect(after).toBe(before)
  })

  it('is a no-op for unknown terminal id', () => {
    const before = store.getState().terminals
    store.getState().updateSessionBranch('nonexistent', 'feature/x')
    const after = store.getState().terminals
    expect(after).toBe(before)
  })

  it('does not affect other terminals', () => {
    store.getState().addTerminal(makeSession({ id: 'term-2', branch: 'dev' }))
    store.getState().updateSessionBranch('term-1', 'feature/new')

    expect(store.getState().terminals.get('term-1')?.session.branch).toBe('feature/new')
    expect(store.getState().terminals.get('term-2')?.session.branch).toBe('dev')
  })
})

describe('setBranchForCwd', () => {
  let store: ReturnType<typeof createStore>

  beforeEach(() => {
    store = createStore()
  })

  it('updates every session whose worktreePath matches cwd', () => {
    store.getState().addTerminal(
      makeSession({
        id: 'a',
        branch: 'main',
        worktreePath: '/wt/emerald',
        projectPath: '/proj'
      })
    )
    store.getState().addTerminal(
      makeSession({
        id: 'b',
        branch: 'main',
        worktreePath: '/wt/emerald',
        projectPath: '/proj'
      })
    )

    store.getState().setBranchForCwd('/wt/emerald', 'feature/x')

    expect(store.getState().terminals.get('a')?.session.branch).toBe('feature/x')
    expect(store.getState().terminals.get('b')?.session.branch).toBe('feature/x')
  })

  it('falls back to projectPath when session has no worktreePath', () => {
    store
      .getState()
      .addTerminal(
        makeSession({ id: 'a', branch: 'main', projectPath: '/proj', worktreePath: undefined })
      )

    store.getState().setBranchForCwd('/proj', 'feature/x')

    expect(store.getState().terminals.get('a')?.session.branch).toBe('feature/x')
  })

  it('leaves non-matching sessions untouched', () => {
    store
      .getState()
      .addTerminal(
        makeSession({ id: 'match', branch: 'main', worktreePath: '/wt/a', projectPath: '/proj' })
      )
    store
      .getState()
      .addTerminal(
        makeSession({ id: 'other', branch: 'dev', worktreePath: '/wt/b', projectPath: '/proj' })
      )

    store.getState().setBranchForCwd('/wt/a', 'feature/x')

    expect(store.getState().terminals.get('match')?.session.branch).toBe('feature/x')
    expect(store.getState().terminals.get('other')?.session.branch).toBe('dev')
  })

  it('preserves terminals Map reference when nothing changes', () => {
    store
      .getState()
      .addTerminal(
        makeSession({ id: 'a', branch: 'main', worktreePath: '/wt/a', projectPath: '/proj' })
      )
    const before = store.getState().terminals

    store.getState().setBranchForCwd('/wt/unrelated', 'feature/x')
    expect(store.getState().terminals).toBe(before)

    store.getState().setBranchForCwd('/wt/a', 'main')
    expect(store.getState().terminals).toBe(before)
  })
})
