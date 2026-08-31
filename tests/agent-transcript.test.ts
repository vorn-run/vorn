import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { RecentSession, TerminalSession } from '../packages/shared/src/types'

const getRecentSessionsFor = vi.fn()
vi.mock('../packages/server/src/agent-history', () => ({
  getRecentSessionsFor: (...args: unknown[]) => getRecentSessionsFor(...args)
}))

import {
  resolveTranscriptId,
  heldTranscripts,
  transcriptHolder
} from '../packages/server/src/agent-transcript'

/**
 * Which conversation a session continues.
 *
 * Moved here from the renderer: every input was already on this side, and the
 * renderer could only dedupe within one window's restore pass, which is why two
 * devices could take one transcript.
 */

function session(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'term-1',
    agentType: 'claude',
    projectName: 'my-app',
    projectPath: '/home/user/my-app',
    status: 'running',
    createdAt: Date.now(),
    pid: 1234,
    ...overrides
  } as TerminalSession
}

function recent(overrides: Partial<RecentSession> = {}): RecentSession {
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
  getRecentSessionsFor.mockReturnValue([])
})

describe('resolving which conversation a session continues', () => {
  it('takes the id the agent was launched with, without reading any history', () => {
    expect(resolveTranscriptId(session({ agentSessionId: 'claude-abc' }))).toBe('claude-abc')
    expect(getRecentSessionsFor).not.toHaveBeenCalled()
  })

  it('never uses hookSessionId, which the agent has never seen', () => {
    expect(resolveTranscriptId(session({ hookSessionId: 'hook-abc' }))).toBeUndefined()
  })

  it('matches a conversation by the directory it ran in', () => {
    getRecentSessionsFor.mockReturnValue([
      recent({ sessionId: 'sess-match', projectPath: '/home/user/my-app' })
    ])
    expect(resolveTranscriptId(session({ projectPath: '/home/user/my-app' }))).toBe('sess-match')
  })

  it('prefers the worktree it was in over the project root', () => {
    getRecentSessionsFor.mockReturnValue([
      recent({ sessionId: 'sess-root', projectPath: '/home/user/my-app' }),
      recent({
        sessionId: 'sess-worktree',
        projectPath: '/home/user/.vorn-worktrees/my-app/feature-a'
      })
    ])
    const resumed = session({
      projectPath: '/home/user/my-app',
      worktreePath: '/home/user/.vorn-worktrees/my-app/feature-a'
    })
    expect(resolveTranscriptId(resumed)).toBe('sess-worktree')
  })

  it('will not match on a basename, which would cross projects', () => {
    getRecentSessionsFor.mockImplementation((_agent: string, projectPath?: string) =>
      projectPath ? [] : [recent({ sessionId: 'sess-fuzzy', projectPath: '/private/var/my-app' })]
    )
    expect(resolveTranscriptId(session({ projectPath: '/var/my-app' }))).toBeUndefined()
  })

  it('asks the project first, then everywhere', () => {
    const resumed = session()
    resolveTranscriptId(resumed)
    expect(getRecentSessionsFor).toHaveBeenCalledTimes(2)
    expect(getRecentSessionsFor).toHaveBeenNthCalledWith(1, 'claude', resumed.projectPath)
    expect(getRecentSessionsFor).toHaveBeenNthCalledWith(2, 'claude')
  })

  it('asks for one agent, not the merged list of five', () => {
    resolveTranscriptId(session({ agentType: 'codex' }))
    expect(getRecentSessionsFor).toHaveBeenCalledWith('codex', '/home/user/my-app')
  })

  it('answers nothing for an agent that cannot resume an exact conversation', () => {
    expect(resolveTranscriptId(session({ agentType: 'gemini' }))).toBeUndefined()
    expect(getRecentSessionsFor).not.toHaveBeenCalled()
  })

  it('leaves a conversation someone else is already writing', () => {
    getRecentSessionsFor.mockReturnValue([
      recent({ sessionId: 'sess-1' }),
      recent({ sessionId: 'sess-2' })
    ])
    expect(resolveTranscriptId(session())).toBe('sess-1')
    expect(resolveTranscriptId(session(), new Set(['sess-1']))).toBe('sess-2')
  })

  it('leaves its own pinned conversation when that is the one being written', () => {
    // Falls through to the scan rather than handing back an id already in use.
    getRecentSessionsFor.mockReturnValue([recent({ sessionId: 'sess-2' })])
    const resumed = session({ agentSessionId: 'sess-1' })
    expect(resolveTranscriptId(resumed, new Set(['sess-1']))).toBe('sess-2')
  })

  it('survives an agent whose history cannot be read', () => {
    getRecentSessionsFor.mockImplementation(() => {
      throw new Error('database is locked')
    })
    expect(resolveTranscriptId(session())).toBeUndefined()
  })
})

describe('who is writing a conversation right now', () => {
  it('is whichever live session carries its id', () => {
    const live = [session({ id: 'a' }), session({ id: 'b', agentSessionId: 'sess-9' })]
    expect(transcriptHolder('sess-9', live)?.id).toBe('b')
    expect(transcriptHolder('sess-1', live)).toBeUndefined()
  })

  it('is nobody once the processes are gone, which is what a restart leaves', () => {
    expect(heldTranscripts([])).toEqual(new Set())
  })

  it('counts only sessions that know which conversation they took', () => {
    const live = [session({ id: 'a' }), session({ id: 'b', agentSessionId: 'sess-9' })]
    expect(heldTranscripts(live)).toEqual(new Set(['sess-9']))
  })
})
