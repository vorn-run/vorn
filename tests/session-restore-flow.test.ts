import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:child_process', () => ({
  execFileSync: vi.fn(() => '/usr/bin/cmd')
}))

import { buildAgentLaunchLine } from '../packages/server/src/agent-launch'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import type { AgentType, CreateTerminalPayload, TerminalSession } from '@vornrun/shared/types'

// Mock the renderer API for resolveResumeSessionId
const mockGetRecentSessions = vi.fn()
vi.stubGlobal('window', {
  api: {
    getRecentSessions: mockGetRecentSessions
  }
})

import {
  resolveResumeSessionId,
  buildRestorePayload,
  coldSessions
} from '../src/renderer/lib/session-utils'

const env = { PATH: '/usr/bin' }
const cmds = DEFAULT_AGENT_COMMANDS

function makeSession(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'term-1',
    agentType: 'claude',
    projectName: 'test',
    projectPath: '/test',
    status: 'running',
    createdAt: Date.now(),
    pid: 1234,
    ...overrides
  }
}

function makePayload(overrides: Partial<CreateTerminalPayload> = {}): CreateTerminalPayload {
  return {
    agentType: 'claude' as AgentType,
    projectName: 'test',
    projectPath: '/test',
    ...overrides
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  mockGetRecentSessions.mockResolvedValue([])
})

describe('session restore flow: Claude with agentSessionId', () => {
  it('agentSessionId is used as resumeSessionId without scanning history', async () => {
    const session = makeSession({ agentSessionId: 'exact-uuid-123' })
    const resumeId = await resolveResumeSessionId(session)
    expect(resumeId).toBe('exact-uuid-123')
    // Should NOT have called getRecentSessions — agentSessionId was sufficient
    expect(mockGetRecentSessions).not.toHaveBeenCalled()
  })

  it('hookSessionId alone is NOT used for resume (VibeGrid-internal UUID)', async () => {
    const session = makeSession({ hookSessionId: 'hook-only-uuid' })
    const resumeId = await resolveResumeSessionId(session)
    // hookSessionId is a VibeGrid routing UUID, not a real agent session ID
    expect(resumeId).toBeUndefined()
  })

  it('buildRestorePayload passes resumeSessionId through', () => {
    const session = makeSession({
      agentSessionId: 'exact-uuid-123',
      displayName: 'my session',
      worktreePath: '/test/wt',
      isWorktree: true,
      branch: 'feat'
    })
    const payload = buildRestorePayload(session, 'exact-uuid-123')
    expect(payload.resumeSessionId).toBe('exact-uuid-123')
    expect(payload.displayName).toBe('my session')
    expect(payload.existingWorktreePath).toBe('/test/wt')
    expect(payload.branch).toBe('feat')
  })

  it('buildAgentLaunchLine produces --resume with agentSessionId', () => {
    const payload = makePayload({ resumeSessionId: 'exact-uuid-123' })
    const line = buildAgentLaunchLine(payload, cmds, env)
    expect(line).toBe('claude --resume exact-uuid-123')
  })

  it('full chain: agentSessionId → restore payload → launch line', async () => {
    const session = makeSession({ agentSessionId: 'chain-uuid' })

    // Step 1: resolve
    const resumeId = await resolveResumeSessionId(session)
    expect(resumeId).toBe('chain-uuid')

    // Step 2: build payload
    const payload = buildRestorePayload(session, resumeId)
    expect(payload.resumeSessionId).toBe('chain-uuid')

    // Step 3: build launch line
    const line = buildAgentLaunchLine(payload as CreateTerminalPayload, cmds, env)
    expect(line).toBe('claude --resume chain-uuid')
  })
})

describe('session restore flow: Gemini (no resume support)', () => {
  it('resolveResumeSessionId returns undefined for gemini', async () => {
    const session = makeSession({ agentType: 'gemini' })
    const resumeId = await resolveResumeSessionId(session)
    expect(resumeId).toBeUndefined()
  })

  it('buildAgentLaunchLine produces no resume flag for gemini', () => {
    const payload = makePayload({ agentType: 'gemini', resumeSessionId: 'any' })
    const line = buildAgentLaunchLine(payload, cmds, env)
    expect(line).toBe('gemini')
    expect(line).not.toContain('--resume')
  })
})

describe('session restore flow: Codex fallback to history', () => {
  it('codex without hookSessionId falls back to getRecentSessions', async () => {
    mockGetRecentSessions.mockResolvedValue([
      {
        sessionId: 'codex-sess-1',
        agentType: 'codex',
        projectPath: '/test',
        canResumeExact: true,
        timestamp: Date.now()
      }
    ])
    const session = makeSession({ agentType: 'codex' })
    const resumeId = await resolveResumeSessionId(session)
    expect(resumeId).toBe('codex-sess-1')
    expect(mockGetRecentSessions).toHaveBeenCalled()
  })

  it('codex resume produces resume subcommand', () => {
    const payload = makePayload({ agentType: 'codex', resumeSessionId: 'codex-sess-1' })
    const line = buildAgentLaunchLine(payload, cmds, env)
    expect(line).toBe('codex resume codex-sess-1')
  })
})

describe('what is left of what is not running', () => {
  const restored = (id: string) => ({
    session: makeSession({ id }),
    endedAt: Date.now() - 3_600_000,
    replayable: true,
    partial: false
  })

  it('is what the server holds, minus anything that has a process', () => {
    expect(
      coldSessions([makeSession({ id: 'still-going' })], [restored('ended')]).map(
        (r) => r.session.id
      )
    ).toEqual(['ended'])
  })

  it('never includes a session that is running', () => {
    // The two questions are separate round trips, so a resume happening
    // elsewhere between them can put one id in both answers. The live one wins:
    // the alternative is two panes for one terminal, one of them a photograph
    // offering to start what is already going.
    const both = 'resumed-elsewhere'
    expect(coldSessions([makeSession({ id: both })], [restored(both)])).toEqual([])
  })

  it('is empty when the server is holding nothing', () => {
    expect(coldSessions([makeSession({ id: 'live' })], [])).toEqual([])
  })
})
