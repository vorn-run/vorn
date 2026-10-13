// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { TerminalSession, AgentType, GitDiffStat } from '../packages/shared/src/types'

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

const mockGetGitDiffStat = vi.fn<(cwd: string) => Promise<GitDiffStat | null>>()
const mockGetGitBranch = vi.fn<(cwd: string) => Promise<string | null>>()
const mockNotifyWidgetStatus = vi.fn()

// Mock window.api before importing the hook
Object.defineProperty(window, 'api', {
  value: {
    getGitDiffStat: (...args: unknown[]) => mockGetGitDiffStat(args[0] as string),
    getGitBranch: (...args: unknown[]) => mockGetGitBranch(args[0] as string),
    notifyWidgetStatus: mockNotifyWidgetStatus
  },
  writable: true
})

// Import the store (must come after window.api mock)
import { useAppStore } from '../packages/web/src/renderer/stores'

beforeEach(() => {
  vi.clearAllMocks()
  vi.useFakeTimers()
  // Reset store
  useAppStore.setState({
    terminals: new Map(),
    gitDiffStats: new Map()
  })
  // Default: not hidden
  Object.defineProperty(document, 'hidden', { value: false, configurable: true })
})

afterEach(() => {
  vi.useRealTimers()
})

// We test the poll logic by importing the hook and simulating the interval.
// Since the hook uses useEffect + setInterval, we exercise the core logic
// by directly calling the same store + API pattern the hook uses.

async function simulatePoll(): Promise<void> {
  const { terminals, updateGitDiffStats, updateSessionBranch } = useAppStore.getState()

  const entries = Array.from(terminals).filter(([, t]) => !t.session.remoteHostId)

  const batchStats = new Map<string, GitDiffStat>()
  await Promise.allSettled(
    entries.map(async ([id, t]) => {
      const cwd = t.session.worktreePath || t.session.projectPath
      const [stat, branch] = await Promise.all([
        window.api.getGitDiffStat(cwd),
        window.api.getGitBranch(cwd)
      ])
      if (stat) batchStats.set(id, stat)
      if (branch && branch !== t.session.branch) {
        updateSessionBranch(id, branch)
      }
    })
  )

  if (batchStats.size > 0) {
    updateGitDiffStats(batchStats)
  }
}

describe('git diff polling — branch update', () => {
  it('updates branch when it changes', async () => {
    const session = makeSession({ id: 'term-1', branch: 'main' })
    useAppStore.getState().addTerminal(session)

    mockGetGitDiffStat.mockResolvedValue({ filesChanged: 0, insertions: 0, deletions: 0 })
    mockGetGitBranch.mockResolvedValue('feature/new')

    await simulatePoll()

    const term = useAppStore.getState().terminals.get('term-1')
    expect(term?.session.branch).toBe('feature/new')
  })

  it('does not update branch when unchanged', async () => {
    const session = makeSession({ id: 'term-1', branch: 'main' })
    useAppStore.getState().addTerminal(session)

    const termsBefore = useAppStore.getState().terminals

    mockGetGitDiffStat.mockResolvedValue(null)
    mockGetGitBranch.mockResolvedValue('main')

    await simulatePoll()

    const termsAfter = useAppStore.getState().terminals
    // Branch didn't change so terminals map reference should be same
    expect(termsAfter).toBe(termsBefore)
  })

  it('skips remote terminals', async () => {
    const session = makeSession({ id: 'term-remote', remoteHostId: 'host-1' })
    useAppStore.getState().addTerminal(session)

    await simulatePoll()

    expect(mockGetGitBranch).not.toHaveBeenCalled()
    expect(mockGetGitDiffStat).not.toHaveBeenCalled()
  })

  it('uses worktreePath when available', async () => {
    const session = makeSession({
      id: 'term-wt',
      projectPath: '/home/user/project',
      worktreePath: '/home/user/.worktrees/feature-abc'
    })
    useAppStore.getState().addTerminal(session)

    mockGetGitDiffStat.mockResolvedValue(null)
    mockGetGitBranch.mockResolvedValue('feature-abc')

    await simulatePoll()

    expect(mockGetGitBranch).toHaveBeenCalledWith('/home/user/.worktrees/feature-abc')
  })

  it('handles getGitBranch returning null gracefully', async () => {
    const session = makeSession({ id: 'term-1', branch: 'main' })
    useAppStore.getState().addTerminal(session)

    mockGetGitDiffStat.mockResolvedValue(null)
    mockGetGitBranch.mockResolvedValue(null)

    await simulatePoll()

    // Branch should remain unchanged
    const term = useAppStore.getState().terminals.get('term-1')
    expect(term?.session.branch).toBe('main')
  })

  it('handles API errors without crashing', async () => {
    const session = makeSession({ id: 'term-1', branch: 'main' })
    useAppStore.getState().addTerminal(session)

    mockGetGitDiffStat.mockRejectedValue(new Error('git error'))
    mockGetGitBranch.mockRejectedValue(new Error('git error'))

    // Should not throw
    await expect(simulatePoll()).resolves.toBeUndefined()

    // Branch unchanged
    const term = useAppStore.getState().terminals.get('term-1')
    expect(term?.session.branch).toBe('main')
  })

  it('updates multiple terminals independently', async () => {
    useAppStore.getState().addTerminal(makeSession({ id: 't1', branch: 'main', projectPath: '/a' }))
    useAppStore.getState().addTerminal(makeSession({ id: 't2', branch: 'dev', projectPath: '/b' }))

    mockGetGitDiffStat.mockResolvedValue(null)
    mockGetGitBranch.mockImplementation(async (cwd: string) => {
      if (cwd === '/a') return 'feature/x'
      if (cwd === '/b') return 'dev' // unchanged
      return null
    })

    await simulatePoll()

    expect(useAppStore.getState().terminals.get('t1')?.session.branch).toBe('feature/x')
    expect(useAppStore.getState().terminals.get('t2')?.session.branch).toBe('dev')
  })
})
