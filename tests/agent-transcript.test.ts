import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { HeadlessSession, RecentSession, TerminalSession } from '../packages/shared/src/types'

const getRecentSessionsFor = vi.fn()
vi.mock('../packages/server/src/agent-history', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../packages/server/src/agent-history')>()),
  getRecentSessionsFor: (...args: unknown[]) => getRecentSessionsFor(...args)
}))

import {
  resolveTranscriptId,
  heldTranscripts,
  transcriptHolder,
  sessionToBindOnCreate,
  claimTranscriptFor
} from '../packages/server/src/agent-transcript'
import {
  resetTranscriptClaims,
  releaseSpawningTranscript
} from '../packages/server/src/transcript-claims'

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
  resetTranscriptClaims()
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
    getRecentSessionsFor.mockReturnValue([recent({ sessionId: 'sess-2' })])
    const resumed = session({ agentSessionId: 'sess-1' })
    expect(resolveTranscriptId(resumed, new Set(['sess-1']))).toBe('sess-2')
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

describe('two cold panes resumed one after the other', () => {
  it('take different conversations, because the first claims while it starts', () => {
    getRecentSessionsFor.mockReturnValue([
      recent({ sessionId: 'transcript-a', agentType: 'codex' }),
      recent({ sessionId: 'transcript-b', agentType: 'codex' })
    ])
    const first = session({ id: 'one', agentType: 'codex' })
    const second = session({ id: 'two', agentType: 'codex' })

    const firstTranscript = claimTranscriptFor(first, [], 'one')
    // Started, but has not yet reported what it took.
    const live = [{ ...first, agentSessionId: undefined }]
    const secondTranscript = claimTranscriptFor(second, live, 'two')

    expect(firstTranscript).toBe('transcript-a')
    expect(secondTranscript).toBe('transcript-b')
  })

  it('skip a conversation a live session has already reported', () => {
    getRecentSessionsFor.mockReturnValue([
      recent({ sessionId: 'transcript-a' }),
      recent({ sessionId: 'transcript-b' })
    ])
    const live = [session({ id: 'one', agentSessionId: 'transcript-a' })]
    expect(claimTranscriptFor(session({ id: 'two' }), live, 'two')).toBe('transcript-b')
  })

  it('leave the agent to choose when every conversation is taken', () => {
    getRecentSessionsFor.mockReturnValue([recent({ sessionId: 'transcript-a' })])
    const live = [session({ id: 'one', agentSessionId: 'transcript-a' })]
    expect(claimTranscriptFor(session({ id: 'two' }), live, 'two')).toBeUndefined()
  })
})

describe('a launch that names no conversation', () => {
  it('claims nothing, so a workflow step and a resume never contend', () => {
    getRecentSessionsFor.mockReturnValue([recent({ sessionId: 'transcript-a' })])
    const live = [session({ id: 'one', agentSessionId: 'transcript-a' })]

    expect(transcriptHolder('', live)).toBeUndefined()
    // The resume beside it still resolves normally.
    expect(claimTranscriptFor(session({ id: 'two' }), live, 'two')).toBeUndefined()
  })

  it('does not treat a session with no reported conversation as holding one', () => {
    const live = [session({ id: 'one' }), session({ id: 'two' })]
    expect(heldTranscripts(live).size).toBe(0)
  })
})

describe('a workflow step running beside the panes', () => {
  const headless = (overrides: Partial<HeadlessSession> = {}): HeadlessSession =>
    ({
      id: 'headless-1',
      pid: 99,
      agentType: 'claude',
      projectName: 'my-app',
      projectPath: '/home/user/my-app',
      status: 'running',
      startedAt: Date.now(),
      ...overrides
    }) as HeadlessSession

  it('holds the conversation it was launched on, though no pane is drawing it', () => {
    expect(heldTranscripts([], [headless({ agentSessionId: 'transcript-a' })])).toEqual(
      new Set(['transcript-a'])
    )
  })

  it('is let go of once it has exited', () => {
    const done = headless({ agentSessionId: 'transcript-a', status: 'exited' })
    expect(heldTranscripts([], [done]).size).toBe(0)
  })

  it('is not resumed onto, even by the pane that pinned that conversation', () => {
    getRecentSessionsFor.mockReturnValue([recent({ sessionId: 'transcript-b' })])
    const resumed = session({ id: 'pane', agentSessionId: 'transcript-a' })
    const running = [headless({ agentSessionId: 'transcript-a' })]
    expect(claimTranscriptFor(resumed, [], 'pane', running)).toBe('transcript-b')
  })

  it('leaves the agent to choose when its conversation is the only one', () => {
    getRecentSessionsFor.mockReturnValue([recent({ sessionId: 'transcript-a' })])
    const running = [headless({ agentSessionId: 'transcript-a' })]
    expect(claimTranscriptFor(session({ id: 'pane' }), [], 'pane', running)).toBeUndefined()
  })
})

describe('a launch that names a conversation already running', () => {
  it('is handed the session writing it rather than starting beside it', () => {
    const live = [session({ id: 'one', agentSessionId: 'transcript-a' })]
    expect(sessionToBindOnCreate('transcript-a', live)?.id).toBe('one')
  })

  it('starts normally when nothing is writing that conversation', () => {
    const live = [session({ id: 'one', agentSessionId: 'transcript-a' })]
    expect(sessionToBindOnCreate('transcript-b', live)).toBeUndefined()
  })

  it('starts normally when it names no conversation at all', () => {
    const live = [session({ id: 'one', agentSessionId: 'transcript-a' })]
    expect(sessionToBindOnCreate(undefined, live)).toBeUndefined()
  })
})

describe('a spawn that fails', () => {
  it('can release exactly what it claimed, so a retry gets the same conversation', () => {
    getRecentSessionsFor.mockReturnValue([
      recent({ sessionId: 'transcript-a', agentType: 'codex' })
    ])
    const cold = session({ id: 'one', agentType: 'codex' })

    const claimed = claimTranscriptFor(cold, [], 'one')
    expect(claimed).toBe('transcript-a')

    releaseSpawningTranscript(claimed!, 'one')

    expect(claimTranscriptFor(cold, [], 'one')).toBe('transcript-a')
  })

  it('leaves the conversation unreachable when the claim is released under another key', () => {
    getRecentSessionsFor.mockReturnValue([
      recent({ sessionId: 'transcript-a', agentType: 'codex' }),
      recent({ sessionId: 'transcript-b', agentType: 'codex' })
    ])
    const cold = session({ id: 'one', agentType: 'codex' })
    claimTranscriptFor(cold, [], 'one')

    releaseSpawningTranscript('', 'one')

    expect(claimTranscriptFor(cold, [], 'one')).toBe('transcript-b')
  })
})
