// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import type { TerminalSession, RecentSession, AgentType } from '../packages/shared/src/types'

import {
  resolveProjectName,
  buildRestorePayload,
  resolveActiveProject
} from '../src/renderer/lib/session-utils'
import { useAppStore } from '../src/renderer/stores'

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

/**
 * A group holds sessions from many projects, so it cannot name one on its own.
 * Launching from a group starts where its most recent session is.
 */
describe('the project a launch starts in', () => {
  const project = (name: string, workspaceId = 'personal') => ({
    name,
    path: `/tmp/${name}`,
    preferredAgents: [],
    workspaceId
  })
  const term = (id: string, projectName: string, at: number, groupId?: string) => [
    id,
    {
      session: { id, projectName, ...(groupId && { groupId }) },
      status: 'idle',
      lastOutputTimestamp: at
    }
  ]

  const seed = (over: Record<string, unknown> = {}) =>
    useAppStore.setState({
      config: {
        projects: [project('vorn'), project('ode'), project('work-thing', 'work')]
      },
      activeProject: null,
      activeGroupId: null,
      activeWorkspace: 'personal',
      terminals: new Map(),
      ...over
    } as never)

  it('is the selected project when there is one', () => {
    seed({ activeProject: 'ode' })
    expect(resolveActiveProject()?.name).toBe('ode')
  })

  it('is the workspace first project when nothing is selected', () => {
    seed()
    expect(resolveActiveProject()?.name).toBe('vorn')
  })

  // Pre-existing: a named project wins by name alone, workspace unchecked.
  it('honours a named project even from another workspace', () => {
    seed({ activeProject: 'work-thing' })
    expect(resolveActiveProject()?.name).toBe('work-thing')
  })

  it('stays inside the workspace when falling back', () => {
    seed({ activeProject: 'deleted-project' })
    expect(resolveActiveProject()?.name).toBe('vorn')
  })

  it('follows the most recent session of a selected group', () => {
    seed({
      activeGroupId: 'g1',
      terminals: new Map([term('s1', 'vorn', 1, 'g1'), term('s2', 'ode', 9, 'g1')] as never)
    })
    expect(resolveActiveProject()?.name).toBe('ode')
  })

  it('ignores sessions that are not in the group', () => {
    seed({
      activeGroupId: 'g1',
      terminals: new Map([term('s1', 'vorn', 1, 'g1'), term('s2', 'ode', 9)] as never)
    })
    expect(resolveActiveProject()?.name).toBe('vorn')
  })

  it('falls back to the workspace when the group has no sessions left', () => {
    seed({ activeGroupId: 'g1' })
    expect(resolveActiveProject()?.name).toBe('vorn')
  })

  it('has nothing to offer when there are no projects', () => {
    useAppStore.setState({ config: { projects: [] } } as never)
    expect(resolveActiveProject()).toBeUndefined()
  })
})
