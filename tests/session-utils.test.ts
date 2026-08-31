// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { TerminalSession, RecentSession, AgentType } from '../packages/shared/src/types'

const mockGetRecentSessions = vi.fn()

// Mock window.api
Object.defineProperty(window, 'api', {
  value: { getRecentSessions: (...args: unknown[]) => mockGetRecentSessions(...args) },
  writable: true
})

import {
  resolveResumeSessionId,
  resolveProjectName,
  buildRestorePayload
} from '../src/renderer/lib/session-utils'

function makeSession(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'term-1',
    agentType: 'claude',
    projectName: 'my-app',
    projectPath: '/home/user/my-app',
    status: 'running',
    createdAt: Date.now(),
    pid: 1234,
    ...overrides
  }
}

function makeRecent(overrides: Partial<RecentSession> = {}): RecentSession {
  return {
    sessionId: 'sess-1',
    agentType: 'claude',
    display: 'Fix bug',
    projectPath: '/home/user/my-app',
    timestamp: Date.now(),
    activityCount: 5,
    activityLabel: 'message',
    canResumeExact: true,
    ...overrides
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  mockGetRecentSessions.mockResolvedValue([])
})

describe('resolveResumeSessionId', () => {
  it('returns agentSessionId immediately when present', async () => {
    const session = makeSession({ agentSessionId: 'claude-abc' })
    const result = await resolveResumeSessionId(session)
    expect(result).toBe('claude-abc')
    expect(mockGetRecentSessions).not.toHaveBeenCalled()
  })

  it('ignores hookSessionId for resume (VibeGrid-internal, not a real agent session ID)', async () => {
    const session = makeSession({ hookSessionId: 'hook-abc' })
    const result = await resolveResumeSessionId(session)
    // hookSessionId alone should NOT be used — falls through to history scan
    expect(result).toBeUndefined()
  })

  it('returns exact-match sessionId from recent sessions', async () => {
    mockGetRecentSessions.mockResolvedValue([
      makeRecent({ sessionId: 'sess-match', projectPath: '/home/user/my-app' })
    ])
    const session = makeSession({ projectPath: '/home/user/my-app' })
    const result = await resolveResumeSessionId(session)
    expect(result).toBe('sess-match')
  })

  it('prefers worktree path matches over project root matches', async () => {
    mockGetRecentSessions.mockResolvedValue([
      makeRecent({ sessionId: 'sess-root', projectPath: '/home/user/my-app' }),
      makeRecent({
        sessionId: 'sess-worktree',
        projectPath: '/home/user/.vorn-worktrees/my-app/feature-a'
      })
    ])
    const session = makeSession({
      projectPath: '/home/user/my-app',
      worktreePath: '/home/user/.vorn-worktrees/my-app/feature-a'
    })

    const result = await resolveResumeSessionId(session)
    expect(result).toBe('sess-worktree')
  })

  it('does not fall back to basename match (prevents cross-project confusion)', async () => {
    mockGetRecentSessions.mockImplementation((projectPath?: string) => {
      if (projectPath) return Promise.resolve([])
      return Promise.resolve([
        makeRecent({ sessionId: 'sess-fuzzy', projectPath: '/private/var/folders/my-app' })
      ])
    })
    const session = makeSession({ projectPath: '/var/folders/my-app' })
    const result = await resolveResumeSessionId(session)
    expect(result).toBeUndefined()
  })

  it('returns undefined when no match found', async () => {
    // Scoped call returns nothing; unscoped returns non-matching session
    mockGetRecentSessions.mockImplementation((projectPath?: string) => {
      if (projectPath) return Promise.resolve([])
      return Promise.resolve([
        makeRecent({
          sessionId: 'sess-other',
          agentType: 'copilot',
          projectPath: '/completely/different'
        })
      ])
    })
    const session = makeSession({ projectPath: '/home/user/my-app' })
    const result = await resolveResumeSessionId(session)
    expect(result).toBeUndefined()
  })

  it('tries scoped fetch first, then unscoped fallback', async () => {
    mockGetRecentSessions.mockResolvedValue([])
    const session = makeSession()
    await resolveResumeSessionId(session)
    expect(mockGetRecentSessions).toHaveBeenCalledTimes(2)
    expect(mockGetRecentSessions).toHaveBeenNthCalledWith(1, session.projectPath)
    expect(mockGetRecentSessions).toHaveBeenNthCalledWith(2)
  })

  it('skips already-claimed session IDs', async () => {
    mockGetRecentSessions.mockResolvedValue([
      makeRecent({ sessionId: 'sess-1', projectPath: '/home/user/my-app' }),
      makeRecent({ sessionId: 'sess-2', projectPath: '/home/user/my-app' })
    ])
    const session = makeSession({ projectPath: '/home/user/my-app' })

    // First call claims sess-1
    const claimed = new Set<string>()
    const first = await resolveResumeSessionId(session, claimed)
    expect(first).toBe('sess-1')
    claimed.add(first!)

    // Second call with same session skips sess-1, returns sess-2
    const second = await resolveResumeSessionId(session, claimed)
    expect(second).toBe('sess-2')
  })

  it('skips claimed hookSessionId', async () => {
    const claimed = new Set(['hook-abc'])
    const session = makeSession({ hookSessionId: 'hook-abc' })
    mockGetRecentSessions.mockResolvedValue([
      makeRecent({ sessionId: 'sess-fallback', projectPath: '/home/user/my-app' })
    ])
    const result = await resolveResumeSessionId(session, claimed)
    expect(result).toBe('sess-fallback')
  })

  it('does not attempt exact resume for gemini sessions', async () => {
    const session = makeSession({
      agentType: 'gemini',
      hookSessionId: 'gemini-hook'
    })
    const result = await resolveResumeSessionId(session)
    expect(result).toBeUndefined()
    expect(mockGetRecentSessions).not.toHaveBeenCalled()
  })
})

describe('resolveProjectName', () => {
  const projects = [
    { name: 'My App', path: '/home/user/my-app' },
    { name: 'Backend', path: '/home/user/backend' }
  ]

  it('returns project name on exact match', () => {
    const session = makeRecent({ projectPath: '/home/user/my-app' })
    expect(resolveProjectName(session, projects)).toBe('My App')
  })

  it('matches when paths differ only by trailing slash', () => {
    const session = makeRecent({ projectPath: '/home/user/my-app/' })
    expect(resolveProjectName(session, projects)).toBe('My App')
  })

  it('returns basename when no projects provided', () => {
    const session = makeRecent({ projectPath: '/home/user/my-app' })
    expect(resolveProjectName(session, undefined)).toBe('my-app')
  })

  it('preserves basename casing when no projects are configured', () => {
    const session = makeRecent({ projectPath: '/Users/Alice/MyApp' })
    expect(resolveProjectName(session, undefined)).toBe('MyApp')
  })

  it('returns basename when no project matches', () => {
    const session = makeRecent({ projectPath: '/home/user/unknown' })
    expect(resolveProjectName(session, projects)).toBe('unknown')
  })

  it('preserves basename casing when no configured project matches', () => {
    const session = makeRecent({ projectPath: '/Users/Alice/MyApp' })
    expect(resolveProjectName(session, projects)).toBe('MyApp')
  })

  it('returns project name for managed worktree paths', () => {
    const session = makeRecent({
      projectPath: '/home/user/.vorn-worktrees/my-app/feature-a'
    })
    expect(resolveProjectName(session, projects)).toBe('My App')
  })

  it('returns untitled for root path', () => {
    const session = makeRecent({ projectPath: '/' })
    expect(resolveProjectName(session, projects)).toBe('untitled')
  })
})

describe('buildRestorePayload', () => {
  it('builds basic payload for non-worktree session', () => {
    const session = makeSession({
      projectPath: '/home/user/my-app',
      displayName: 'My Session'
    })
    const payload = buildRestorePayload(session, 'resume-123')
    expect(payload).toEqual({
      agentType: 'claude',
      projectName: 'my-app',
      projectPath: '/home/user/my-app',
      displayName: 'My Session',
      branch: undefined,
      existingWorktreePath: undefined,
      useWorktree: undefined,
      remoteHostId: undefined,
      resumeSessionId: 'resume-123'
    })
  })

  it('passes existingWorktreePath for worktree sessions', () => {
    const session = makeSession({
      isWorktree: true,
      worktreePath: '/home/user/.vorn-worktrees/my-app/main-abc123',
      branch: 'main-worktree-abc123'
    })
    const payload = buildRestorePayload(session)
    expect(payload.existingWorktreePath).toBe('/home/user/.vorn-worktrees/my-app/main-abc123')
    expect(payload.branch).toBe('main-worktree-abc123')
    expect(payload.useWorktree).toBeUndefined()
  })

  it('falls back to useWorktree when worktreePath is missing', () => {
    const session = makeSession({
      isWorktree: true,
      branch: 'feature-x'
    })
    const payload = buildRestorePayload(session)
    expect(payload.existingWorktreePath).toBeUndefined()
    expect(payload.useWorktree).toBe(true)
    expect(payload.branch).toBe('feature-x')
  })

  it('preserves displayName (undefined when not set)', () => {
    const session = makeSession()
    const payload = buildRestorePayload(session)
    expect(payload.displayName).toBeUndefined()
  })

  it('passes remoteHostId when present', () => {
    const session = makeSession({ remoteHostId: 'host-1' })
    const payload = buildRestorePayload(session)
    expect(payload.remoteHostId).toBe('host-1')
  })

  it('throws for shell sessions — shells restore via a separate IPC path', () => {
    const shell = makeSession({ agentType: 'shell' as AgentType, shellCwd: '/home/user' })
    expect(() => buildRestorePayload(shell)).toThrow(
      /shell sessions restore via createShellTerminal/
    )
  })
})
