import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:fs', () => ({
  default: {
    existsSync: vi.fn(() => false),
    readFileSync: vi.fn(() => ''),
    readdirSync: vi.fn(() => []),
    statSync: vi.fn(() => ({ mtimeMs: 0 })),
    realpathSync: vi.fn((p: string) => p)
  }
}))

const libsqlRowsQueue: Record<string, unknown>[][] = []
const libsqlSqlCalls: string[] = []

vi.mock('libsql', () => {
  class MockDatabase {
    constructor(_path: string, _opts?: unknown) {}
    prepare(sql: string) {
      libsqlSqlCalls.push(sql)
      return {
        all: () => libsqlRowsQueue.shift() ?? []
      }
    }
    close() {}
  }
  return { default: MockDatabase }
})

vi.mock('../packages/server/src/git-utils', () => ({
  listWorktrees: vi.fn(() => [])
}))

import fs from 'node:fs'
import { listWorktrees } from '../packages/server/src/git-utils'
import { getRecentSessions } from '../packages/server/src/agent-history'

beforeEach(() => {
  vi.clearAllMocks()
  libsqlRowsQueue.length = 0
  libsqlSqlCalls.length = 0
})

describe('getRecentSessions', () => {
  it('returns empty when no history files exist', () => {
    expect(getRecentSessions()).toEqual([])
  })
})

describe('Claude provider', () => {
  it('parses history.jsonl and returns sessions', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => {
      return String(p).includes('.claude/history.jsonl')
    })
    vi.mocked(fs.readFileSync).mockReturnValueOnce(
      [
        JSON.stringify({ sessionId: 's1', display: 'Fix bug', project: '/app', timestamp: 1000 }),
        JSON.stringify({ sessionId: 's1', display: 'Fix bug', project: '/app', timestamp: 2000 }),
        JSON.stringify({
          sessionId: 's2',
          display: 'Add feature',
          project: '/app',
          timestamp: 1500
        })
      ].join('\n')
    )

    const sessions = getRecentSessions()
    const claude = sessions.filter((s) => s.agentType === 'claude')

    expect(claude).toHaveLength(2)
    // Deduplicates by sessionId, uses latest timestamp
    expect(claude[0].sessionId).toBe('s1')
    expect(claude[0].timestamp).toBe(2000)
    expect(claude[0].activityCount).toBe(2)
    expect(claude[0].activityLabel).toBe('entry')
    expect(claude[0].canResumeExact).toBe(true)
    // Sorted by timestamp desc
    expect(claude[1].sessionId).toBe('s2')
  })

  it('filters by projectPath', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => {
      return String(p).includes('.claude/history.jsonl')
    })
    vi.mocked(fs.readFileSync).mockReturnValueOnce(
      [
        JSON.stringify({ sessionId: 's1', display: 'Fix', project: '/app', timestamp: 1000 }),
        JSON.stringify({ sessionId: 's2', display: 'Add', project: '/other', timestamp: 2000 })
      ].join('\n')
    )

    const sessions = getRecentSessions('/app')
    const claude = sessions.filter((s) => s.agentType === 'claude')
    expect(claude).toHaveLength(1)
    expect(claude[0].sessionId).toBe('s1')
  })

  it('includes sessions from known worktrees when filtering by project path', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => {
      return String(p).includes('.claude/history.jsonl')
    })
    vi.mocked(listWorktrees).mockReturnValue([
      { name: 'feature-a', path: '/worktrees/my-app/feature-a', branch: 'feature-a', isMain: false }
    ])
    vi.mocked(fs.readFileSync).mockReturnValueOnce(
      JSON.stringify({
        sessionId: 's-worktree',
        display: 'Fix in worktree',
        project: '/worktrees/my-app/feature-a',
        timestamp: 1000
      })
    )

    const sessions = getRecentSessions('/app')
    const claude = sessions.filter((s) => s.agentType === 'claude')
    expect(claude).toHaveLength(1)
    expect(claude[0].sessionId).toBe('s-worktree')
  })
})

describe('Codex provider', () => {
  it('returns sessions from sqlite query', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => {
      return String(p).includes('.codex/state_5.sqlite')
    })
    libsqlRowsQueue.push([
      { id: 'c1', cwd: '/app', title: 'Test', updated_at: 1700000000, first_user_message: '' }
    ])

    const sessions = getRecentSessions()
    const codex = sessions.filter((s) => s.agentType === 'codex')
    expect(codex).toHaveLength(1)
    expect(codex[0].sessionId).toBe('c1')
  })

  it('returns empty when DB does not exist', () => {
    vi.mocked(fs.existsSync).mockReturnValue(false)
    const sessions = getRecentSessions()
    expect(sessions.filter((s) => s.agentType === 'codex')).toEqual([])
  })

  it('matches Windows project paths after normalization', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => String(p).includes('.codex/state_5.sqlite'))
    libsqlRowsQueue.push([
      {
        id: 'c1',
        cwd: 'c:/Users/Javier/App',
        title: 'Matching',
        updated_at: 1700000001,
        first_user_message: ''
      },
      {
        id: 'c2',
        cwd: 'D:/Elsewhere',
        title: 'Other',
        updated_at: 1700000000,
        first_user_message: ''
      }
    ])

    const sessions = getRecentSessions('C:\\Users\\Javier\\App\\')
    const codex = sessions.filter((s) => s.agentType === 'codex')
    expect(codex).toHaveLength(1)
    expect(codex[0].sessionId).toBe('c1')
  })

  it('queries known worktree paths when filtering by project path', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => String(p).includes('.codex/state_5.sqlite'))
    vi.mocked(listWorktrees).mockReturnValue([
      { name: 'feature-a', path: '/worktrees/my-app/feature-a', branch: 'feature-a', isMain: false }
    ])
    libsqlRowsQueue.push([
      {
        id: 'c1',
        cwd: '/worktrees/my-app/feature-a',
        title: 'Worktree session',
        updated_at: 1700000001,
        first_user_message: ''
      }
    ])

    const sessions = getRecentSessions('/app')
    const codex = sessions.filter((s) => s.agentType === 'codex')
    expect(codex).toHaveLength(1)
    expect(codex[0].sessionId).toBe('c1')

    const sql = libsqlSqlCalls[0]
    expect(String(sql)).toContain('/app')
    expect(String(sql)).toContain('/worktrees/my-app/feature-a')
  })

  it('lowercases scoped SQL path literals for uppercase POSIX paths', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => String(p).includes('.codex/state_5.sqlite'))
    libsqlRowsQueue.push([
      {
        id: 'c1',
        cwd: '/Users/Javier/App',
        title: 'Matching',
        updated_at: 1700000001,
        first_user_message: ''
      }
    ])

    getRecentSessions('/Users/Javier/App')

    const sql = libsqlSqlCalls[0]
    expect(String(sql)).toContain('/users/javier/app')
  })
})

describe('Copilot provider', () => {
  it('matches Windows project paths after normalization', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) =>
      String(p).includes('.copilot/session-store.db')
    )
    libsqlRowsQueue.push([
      {
        id: 'p1',
        cwd: 'c:/Users/Javier/App',
        summary: 'Matching',
        updated_at: '2026-03-25T00:00:00.000Z',
        turn_count: 4
      },
      {
        id: 'p2',
        cwd: 'D:/Elsewhere',
        summary: 'Other',
        updated_at: '2026-03-24T00:00:00.000Z',
        turn_count: 2
      }
    ])

    const sessions = getRecentSessions('C:\\Users\\Javier\\App\\')
    const copilot = sessions.filter((s) => s.agentType === 'copilot')
    expect(copilot).toHaveLength(1)
    expect(copilot[0].sessionId).toBe('p1')
  })
})

describe('OpenCode provider', () => {
  it('returns sessions from the OpenCode database', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => String(p).includes('opencode/opencode.db'))
    libsqlRowsQueue.push([
      {
        id: 'o1',
        directory: '/app',
        title: 'OpenCode session',
        time_updated: 1700000001000,
        message_count: 7
      }
    ])

    const sessions = getRecentSessions()
    const opencode = sessions.filter((s) => s.agentType === 'opencode')
    expect(opencode).toHaveLength(1)
    expect(opencode[0]).toMatchObject({
      sessionId: 'o1',
      projectPath: '/app',
      display: 'OpenCode session',
      activityCount: 7,
      activityLabel: 'message',
      canResumeExact: true
    })
  })

  it('filters OpenCode sessions by project path', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => String(p).includes('opencode/opencode.db'))
    libsqlRowsQueue.push([
      {
        id: 'o1',
        directory: '/app',
        title: 'OpenCode session',
        time_updated: 1700000001000,
        message_count: 7
      }
    ])

    const sessions = getRecentSessions('/app')
    const opencode = sessions.filter((s) => s.agentType === 'opencode')
    expect(opencode).toHaveLength(1)

    const sql = libsqlSqlCalls[0]
    expect(String(sql)).toContain('s.directory')
  })
})

describe('aggregate', () => {
  it('merges all providers, sorted by timestamp, limited', () => {
    // Claude
    vi.mocked(fs.existsSync).mockImplementation((p) => {
      return (
        String(p).includes('.claude/history.jsonl') || String(p).includes('.codex/state_5.sqlite')
      )
    })
    vi.mocked(fs.readFileSync).mockReturnValueOnce(
      JSON.stringify({ sessionId: 's1', display: 'Claude', project: '/app', timestamp: 3000 })
    )
    // Codex
    libsqlRowsQueue.push([
      { id: 'c1', cwd: '/app', title: 'Codex', updated_at: 4, first_user_message: '' }
    ])

    const sessions = getRecentSessions(undefined, 2)
    expect(sessions.length).toBeLessThanOrEqual(2)
    // Should be sorted by timestamp desc
    for (let i = 1; i < sessions.length; i++) {
      expect(sessions[i - 1].timestamp).toBeGreaterThanOrEqual(sessions[i].timestamp)
    }
  })
})

describe('Claude provider path normalization', () => {
  it('matches when filter has trailing slash but history does not', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => String(p).includes('.claude/history.jsonl'))
    vi.mocked(fs.readFileSync).mockReturnValueOnce(
      JSON.stringify({ sessionId: 's1', display: 'Fix', project: '/app', timestamp: 1000 })
    )
    // realpathSync returns as-is (no symlinks)
    vi.mocked(fs.realpathSync).mockImplementation((p) => String(p))

    const sessions = getRecentSessions('/app/')
    const claude = sessions.filter((s) => s.agentType === 'claude')
    expect(claude).toHaveLength(1)
    expect(claude[0].sessionId).toBe('s1')
  })

  it('matches when history has trailing slash but filter does not', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => String(p).includes('.claude/history.jsonl'))
    vi.mocked(fs.readFileSync).mockReturnValueOnce(
      JSON.stringify({ sessionId: 's1', display: 'Fix', project: '/app/', timestamp: 1000 })
    )
    vi.mocked(fs.realpathSync).mockImplementation((p) => String(p))

    const sessions = getRecentSessions('/app')
    const claude = sessions.filter((s) => s.agentType === 'claude')
    expect(claude).toHaveLength(1)
  })

  it('matches via symlink resolution', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => String(p).includes('.claude/history.jsonl'))
    vi.mocked(fs.readFileSync).mockReturnValueOnce(
      JSON.stringify({
        sessionId: 's1',
        display: 'Fix',
        project: '/var/data',
        timestamp: 1000
      })
    )
    // Simulate macOS symlink: /var -> /private/var
    vi.mocked(fs.realpathSync).mockImplementation((p) => {
      const s = String(p)
      if (s.startsWith('/var/')) return s.replace('/var/', '/private/var/')
      if (s.startsWith('/private/var/')) return s
      return s
    })

    const sessions = getRecentSessions('/private/var/data')
    const claude = sessions.filter((s) => s.agentType === 'claude')
    expect(claude).toHaveLength(1)
    expect(claude[0].sessionId).toBe('s1')
  })
})

describe('Gemini provider path normalization', () => {
  it('matches Windows project paths after normalization', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => {
      const value = String(p)
      return value.includes('.gemini/projects.json') || value.includes('.gemini/tmp/my-app/chats')
    })
    vi.mocked(fs.readFileSync)
      .mockReturnValueOnce(
        JSON.stringify({
          projects: { 'c:/Users/Javier/App': 'my-app' }
        })
      )
      .mockReturnValueOnce(
        JSON.stringify({
          sessionId: 'g1',
          startTime: '2026-03-25T00:00:00.000Z',
          lastUpdated: '2026-03-25T01:00:00.000Z',
          messages: [
            { id: 'm1', timestamp: '2026-03-25T00:10:00.000Z', type: 'user', content: 'Hi' }
          ]
        })
      )
    vi.mocked(fs.readdirSync).mockReturnValue(['session-1.json'] as unknown as ReturnType<
      typeof fs.readdirSync
    >)
    vi.mocked(fs.statSync).mockReturnValue({ mtimeMs: 1 } as ReturnType<typeof fs.statSync>)

    const sessions = getRecentSessions('C:\\Users\\Javier\\App\\')
    const gemini = sessions.filter((s) => s.agentType === 'gemini')
    expect(gemini).toHaveLength(1)
    expect(gemini[0].sessionId).toBe('g1')
    expect(gemini[0].canResumeExact).toBe(false)
  })

  it('discovers project roots from tmp directories and parses structured content titles', () => {
    vi.mocked(fs.existsSync).mockImplementation((p) => {
      const value = String(p)
      return (
        value.includes('.gemini/tmp') ||
        value.includes('.gemini/tmp/hash-1/.project_root') ||
        value.includes('.gemini/tmp/hash-1/chats')
      )
    })
    vi.mocked(fs.readdirSync).mockImplementation((p) => {
      const value = String(p)
      if (value.endsWith('.gemini/tmp'))
        return ['hash-1'] as unknown as ReturnType<typeof fs.readdirSync>
      if (value.endsWith('hash-1/chats')) {
        return ['session-1.json'] as unknown as ReturnType<typeof fs.readdirSync>
      }
      return [] as unknown as ReturnType<typeof fs.readdirSync>
    })
    vi.mocked(fs.readFileSync).mockImplementation((p) => {
      const value = String(p)
      if (value.endsWith('.project_root')) return '/app'
      if (value.endsWith('session-1.json')) {
        return JSON.stringify({
          sessionId: 'g-structured',
          startTime: '2026-03-25T00:00:00.000Z',
          lastUpdated: '2026-03-25T01:00:00.000Z',
          messages: [
            {
              id: 'm1',
              timestamp: '2026-03-25T00:10:00.000Z',
              type: 'user',
              content: [{ text: 'Hello from Gemini' }]
            }
          ]
        })
      }
      return ''
    })

    const sessions = getRecentSessions('/app')
    const gemini = sessions.filter((s) => s.agentType === 'gemini')
    expect(gemini).toHaveLength(1)
    expect(gemini[0].display).toBe('Hello from Gemini')
    expect(gemini[0].projectPath).toBe('/app')
    expect(gemini[0].activityLabel).toBe('prompt')
    expect(gemini[0].canResumeExact).toBe(false)
  })
})
