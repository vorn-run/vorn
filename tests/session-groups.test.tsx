// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { renderHook } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'
import { useSessionScope } from '../src/renderer/hooks/useScopedSessionIds'
import type { AppConfig, TerminalSession } from '../src/shared/types'

const setSessionGroup = vi.fn(() => Promise.resolve())
Object.defineProperty(window, 'api', {
  value: { saveConfig: () => Promise.resolve(), setSessionGroup },
  writable: true
})

const config = {
  version: 1,
  defaults: {},
  projects: [
    { name: 'vorn', path: '/tmp/vorn', preferredAgents: [] },
    { name: 'vorn-connectors', path: '/tmp/vc', preferredAgents: [] },
    { name: 'ode', path: '/tmp/ode', preferredAgents: [] },
    { name: 'work-thing', path: '/tmp/wt', preferredAgents: [], workspaceId: 'work' }
  ],
  sessionGroups: [
    { id: 'g1', name: 'Sidebar work', order: 0, workspaceId: 'personal' },
    { id: 'g9', name: 'Elsewhere', order: 0, workspaceId: 'work' }
  ]
} as unknown as AppConfig

const term = (id: string, projectName: string, groupId?: string) => [
  id,
  {
    session: {
      id,
      projectName,
      agentType: 'claude',
      ...(groupId && { groupId })
    } as TerminalSession,
    status: 'idle',
    lastOutputTimestamp: 0
  }
]

beforeEach(() => {
  vi.clearAllMocks()
  useAppStore.setState({
    config,
    activeProject: null,
    activeGroupId: null,
    activeWorkspace: 'personal',
    // The group's three sessions span two repos — the point of the feature.
    terminals: new Map([
      term('s1', 'vorn', 'g1'),
      term('s2', 'vorn-connectors', 'g1'),
      term('s3', 'vorn'),
      term('s4', 'ode')
    ] as never)
  })
})

const scope = () => renderHook(() => useSessionScope()).result.current

describe('a group scopes by session, because its members span projects', () => {
  it('narrows to exactly the sessions filed under it', () => {
    useAppStore.setState({ activeGroupId: 'g1' })
    expect([...scope().sessionIds!].sort()).toEqual(['s1', 's2'])
  })

  it('picks up sessions from more than one repo', () => {
    useAppStore.setState({ activeGroupId: 'g1' })
    const ids = scope().sessionIds!
    const projects = [...ids].map(
      (id) => useAppStore.getState().terminals.get(id)!.session.projectName
    )
    expect(new Set(projects)).toEqual(new Set(['vorn', 'vorn-connectors']))
  })

  it('leaves a sibling session of the same project out', () => {
    useAppStore.setState({ activeGroupId: 'g1' })
    // s3 is also in `vorn`, but was never filed.
    expect(scope().sessionIds!.has('s3')).toBe(false)
  })

  it('answers by project name when a project is selected instead', () => {
    useAppStore.setState({ activeProject: 'vorn' })
    expect(scope().sessionIds).toBeNull()
    expect([...scope().projectNames!]).toEqual(['vorn'])
  })

  it('restricts nothing when neither is selected', () => {
    expect(scope().sessionIds).toBeNull()
    expect([...scope().projectNames!].sort()).toEqual(['ode', 'vorn', 'vorn-connectors'])
  })
})

/**
 * A selection can outlive what it pointed at. Narrowing to nothing would blank
 * every list with no visible row to clear, so a stale one means no selection.
 */
describe('a selection that no longer matches anything', () => {
  it('falls back rather than emptying the view when the group is another workspace’s', () => {
    useAppStore.setState({ activeGroupId: 'g9' })
    expect(scope().sessionIds).toBeNull()
    expect([...scope().projectNames!].sort()).toEqual(['ode', 'vorn', 'vorn-connectors'])
  })

  /**
   * A stale selection can name a group that still has sessions. Scoping to them
   * would put another workspace's work on the board.
   */
  it('does not surface another workspace’s sessions even when that group has some', () => {
    useAppStore.setState({
      activeGroupId: 'g9',
      terminals: new Map([term('s1', 'vorn', 'g1'), term('w1', 'work-thing', 'g9')] as never)
    })
    expect(scope().sessionIds).toBeNull()
  })

  it('drops a member whose project is outside the workspace', () => {
    useAppStore.setState({
      activeGroupId: 'g1',
      terminals: new Map([term('s1', 'vorn', 'g1'), term('w1', 'work-thing', 'g1')] as never)
    })
    expect([...scope().sessionIds!]).toEqual(['s1'])
  })

  it('does the same once the group’s last session is killed', () => {
    useAppStore.setState({
      activeGroupId: 'g1',
      terminals: new Map([term('s4', 'ode')] as never)
    })
    expect(scope().sessionIds).toBeNull()
  })

  it('and for a project from another workspace', () => {
    useAppStore.setState({ activeProject: 'work-thing' })
    expect([...scope().projectNames!].sort()).toEqual(['ode', 'vorn', 'vorn-connectors'])
  })
})

describe('filing a session', () => {
  it('tells the server, which is what owns membership', () => {
    useAppStore.getState().moveSessionToGroup('s3', 'g1')
    expect(setSessionGroup).toHaveBeenCalledWith('s3', 'g1')
    expect(useAppStore.getState().terminals.get('s3')!.session.groupId).toBe('g1')
  })

  it('takes it out again with null', () => {
    useAppStore.getState().moveSessionToGroup('s1', null)
    expect(setSessionGroup).toHaveBeenCalledWith('s1', null)
    expect(useAppStore.getState().terminals.get('s1')!.session.groupId).toBeUndefined()
  })

  it('lets every member go when the group is deleted, and kills none', () => {
    useAppStore.getState().removeSessionGroup('g1')
    expect(setSessionGroup).toHaveBeenCalledWith('s1', null)
    expect(setSessionGroup).toHaveBeenCalledWith('s2', null)
    expect(useAppStore.getState().config!.sessionGroups).toHaveLength(1)
    expect(useAppStore.getState().terminals.size).toBe(4)
  })
})

/**
 * Resume rebuilds a session from a whitelist and saves it under the same id, so
 * anything the whitelist omits is written back as null and gone for good. The
 * group came back from a restart and then vanished the moment you resumed.
 */
describe('a resumed session keeps its group', () => {
  it('applies an inbound group change from another client', () => {
    useAppStore.setState({
      terminals: new Map([term('s1', 'vorn', 'g1')] as never)
    })
    useAppStore.getState().updateSessionGroupId('s1', 'g2')
    expect(useAppStore.getState().terminals.get('s1')!.session.groupId).toBe('g2')
  })

  it('applies an inbound ungrouping, which arrives as undefined not null', () => {
    useAppStore.setState({
      terminals: new Map([term('s1', 'vorn', 'g1')] as never)
    })
    useAppStore.getState().updateSessionGroupId('s1', undefined)
    expect(useAppStore.getState().terminals.get('s1')!.session).not.toHaveProperty('groupId')
  })

  it('leaves the map alone when nothing changed', () => {
    useAppStore.setState({
      terminals: new Map([term('s1', 'vorn', 'g1')] as never)
    })
    const before = useAppStore.getState().terminals
    useAppStore.getState().updateSessionGroupId('s1', 'g1')
    expect(useAppStore.getState().terminals).toBe(before)
  })
})
