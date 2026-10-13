import { describe, it, expect } from 'vitest'
import { getProjectHostIds } from '../packages/web/src/shared/types'
import { getDisplayName } from '../packages/web/src/renderer/lib/terminal-display'
import type { ProjectConfig, TerminalSession } from '../packages/web/src/shared/types'

describe('getProjectHostIds', () => {
  it('returns ["local"] when hostIds is undefined', () => {
    const project = { name: 'test', path: '/test', preferredAgents: [] } as ProjectConfig
    expect(getProjectHostIds(project)).toEqual(['local'])
  })

  it('returns ["local"] when hostIds is empty array', () => {
    const project = {
      name: 'test',
      path: '/test',
      preferredAgents: [],
      hostIds: []
    } as ProjectConfig
    expect(getProjectHostIds(project)).toEqual(['local'])
  })

  it('returns hostIds when populated', () => {
    const project = {
      name: 'test',
      path: '/test',
      preferredAgents: [],
      hostIds: ['local', 'host-1', 'host-2']
    } as ProjectConfig
    expect(getProjectHostIds(project)).toEqual(['local', 'host-1', 'host-2'])
  })
})

describe('getDisplayName', () => {
  function makeSession(overrides: Partial<TerminalSession> = {}): TerminalSession {
    return {
      id: '1',
      agentType: 'claude',
      projectName: 'my-project',
      projectPath: '/path/to/project',
      status: 'running',
      createdAt: Date.now(),
      pid: 1234,
      ...overrides
    }
  }

  it('returns displayName when set', () => {
    expect(getDisplayName(makeSession({ displayName: 'Custom Name' }))).toBe('Custom Name')
  })

  it('returns projectName as fallback', () => {
    expect(getDisplayName(makeSession())).toBe('my-project')
  })

  it('trims whitespace from displayName', () => {
    expect(getDisplayName(makeSession({ displayName: '  Spaced  ' }))).toBe('Spaced')
  })

  it('returns projectName when displayName is only whitespace', () => {
    expect(getDisplayName(makeSession({ displayName: '   ' }))).toBe('my-project')
  })

  it('returns projectName when displayName is empty string', () => {
    expect(getDisplayName(makeSession({ displayName: '' }))).toBe('my-project')
  })
})
