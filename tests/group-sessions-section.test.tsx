// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockStore = {
  terminals: new Map(),
  terminalsPanes: new Map(),
  filesPanes: new Set(),
  editorPanes: new Map(),
  browserPanes: new Map(),
  devicePanes: new Map(),
  mobileProjectCache: new Map(),
  loadMobileProject: vi.fn(),
  toggleFilesPane: vi.fn(),
  toggleBrowserPane: vi.fn(),
  claimAndOpenDevicePane: vi.fn(),
  closeDevicePane: vi.fn(),
  setActiveTabId: vi.fn(),
  setPreviewTerminal: vi.fn(),
  focusedTerminalId: null as string | null,
  previewTerminalId: null as string | null,
  activeTabId: null as string | null,
  activeProject: null as string | null,
  setActiveProject: vi.fn(),
  activeGroupId: null as string | null,
  setActiveGroup: vi.fn(),
  setFocusedTerminal: vi.fn(),
  addSessionGroup: vi.fn(),
  updateSessionGroup: vi.fn(),
  removeSessionGroup: vi.fn(),
  moveSessionToGroup: vi.fn(),
  activeWorkspace: 'personal',
  sidebarProjectSort: 'manual',
  sidebarWorktreeSort: 'name',
  sidebarWorktreeFilter: 'all',
  sidebarViewMode: 'groups-sessions',
  setSidebarProjectSort: vi.fn(),
  setSidebarWorktreeSort: vi.fn(),
  setSidebarWorktreeFilter: vi.fn(),
  setSidebarViewMode: vi.fn(),
  config: null as { projects?: unknown[]; sessionGroups?: unknown[] } | null
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn() }
}))

const { GroupSessionsSection } =
  await import('../src/renderer/components/project-sidebar/GroupSessionsSection')

const session = (id: string, name: string, project: string, groupId?: string) => [
  id,
  {
    session: {
      id,
      projectName: project,
      agentType: 'claude',
      displayName: name,
      ...(groupId && { groupId })
    },
    status: 'idle',
    lastOutputTimestamp: 1
  }
]

beforeEach(() => {
  vi.clearAllMocks()
  mockStore.activeGroupId = null
  mockStore.activeProject = null
  mockStore.config = {
    projects: [
      { name: 'vorn', path: '/tmp/vorn', preferredAgents: [] },
      { name: 'ode', path: '/tmp/ode', preferredAgents: [] }
    ],
    sessionGroups: [{ id: 'g1', name: 'Sidebar work', order: 0, workspaceId: 'personal' }]
  }
  mockStore.terminals = new Map([
    session('s1', 'in-group', 'vorn', 'g1'),
    session('s2', 'also-in-group', 'ode', 'g1'),
    session('s3', 'loose', 'vorn')
  ] as never)
})

const draw = () =>
  render(
    <GroupSessionsSection
      isCollapsed={false}
      workspaceProjectNames={new Set(['vorn', 'ode'])}
      workspaceTerminalCount={3}
    />
  )

describe('the Groups > Sessions mode', () => {
  it('draws a group holding sessions from more than one repo', () => {
    draw()
    expect(screen.getByRole('button', { name: 'Sidebar work' })).toBeInTheDocument()
    expect(screen.getByText('in-group')).toBeInTheDocument()
    expect(screen.getByText('also-in-group')).toBeInTheDocument()
  })

  it('draws ungrouped sessions below, with no header of their own', () => {
    draw()
    expect(screen.getByText('loose')).toBeInTheDocument()
    expect(screen.queryByText('Ungrouped')).not.toBeInTheDocument()
  })

  it('makes a group in the active workspace, after the last one', () => {
    draw()
    fireEvent.click(screen.getByRole('button', { name: 'New group' }))
    expect(mockStore.addSessionGroup).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'New group', order: 1, workspaceId: 'personal' })
    )
  })

  it('scopes to a group when its row is clicked', () => {
    draw()
    fireEvent.click(screen.getByRole('button', { name: 'Sidebar work' }))
    expect(mockStore.setActiveGroup).toHaveBeenCalledWith('g1')
  })

  it('keeps the group populated whichever group is selected', () => {
    mockStore.activeGroupId = 'g1'
    draw()
    expect(screen.getByText('in-group')).toBeInTheDocument()
    expect(screen.getByText('loose')).toBeInTheDocument()
  })

  it('says so only when there is genuinely nothing', () => {
    mockStore.terminals = new Map()
    draw()
    expect(screen.getByText('No active sessions')).toBeInTheDocument()
  })

  /** A deleted group must not take its sessions off the screen with it. */
  it('shows a session whose group no longer exists as ungrouped', () => {
    mockStore.config = { projects: mockStore.config!.projects, sessionGroups: [] }
    draw()
    expect(screen.getByText('in-group')).toBeInTheDocument()
    expect(screen.getByText('also-in-group')).toBeInTheDocument()
  })
})

describe('filling and folding a group', () => {
  it('collapses and expands, hiding its sessions', () => {
    draw()
    expect(screen.getByText('in-group')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Toggle Sidebar work' }))
    expect(screen.queryByText('in-group')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Toggle Sidebar work' }))
    expect(screen.getByText('in-group')).toBeInTheDocument()
  })

  it('says what an empty group is for', () => {
    mockStore.terminals = new Map([session('s3', 'loose', 'vorn')] as never)
    draw()
    expect(screen.getByText('Drop a session here')).toBeInTheDocument()
  })

  it('files a session dropped onto the group', () => {
    draw()
    const row = screen.getByRole('button', { name: 'Sidebar work' }).closest('div')!.parentElement!
    const dataTransfer = {
      types: ['application/vorn-session'],
      getData: () => 's3',
      dropEffect: ''
    }
    fireEvent.dragOver(row, { dataTransfer })
    fireEvent.drop(row, { dataTransfer })
    expect(mockStore.moveSessionToGroup).toHaveBeenCalledWith('s3', 'g1')
  })

  it('ignores a drag that is not carrying a session', () => {
    draw()
    const row = screen.getByRole('button', { name: 'Sidebar work' }).closest('div')!.parentElement!
    const dataTransfer = { types: ['text/plain'], getData: () => '', dropEffect: '' }
    fireEvent.dragOver(row, { dataTransfer })
    fireEvent.drop(row, { dataTransfer })
    expect(mockStore.moveSessionToGroup).not.toHaveBeenCalled()
  })

  it('opens a freshly made group straight into its name field', () => {
    draw()
    fireEvent.click(screen.getByRole('button', { name: 'New group' }))
    const id = mockStore.addSessionGroup.mock.calls[0][0].id
    expect(id).toBeTruthy()
  })

  it('clears the selection through All Sessions', () => {
    mockStore.activeGroupId = 'g1'
    draw()
    fireEvent.click(screen.getByRole('button', { name: /All Sessions/ }))
    expect(mockStore.setActiveProject).toHaveBeenCalledWith(null)
    expect(mockStore.setActiveGroup).toHaveBeenCalledWith(null)
  })

  it('folds the whole section away', () => {
    draw()
    fireEvent.click(screen.getByText('Groups'))
    expect(screen.queryByRole('button', { name: 'Sidebar work' })).not.toBeInTheDocument()
  })
})
