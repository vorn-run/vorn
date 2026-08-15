// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockStore = {
  terminals: new Map(),
  terminalsPanes: new Map(),
  // A session row reads these; the rows are what this file renders.
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
  config: null,
  activeProject: null as string | null,
  setActiveProject: vi.fn(),
  setFocusedTerminal: vi.fn(),
  sidebarProjectSort: 'manual',
  sidebarWorktreeSort: 'name',
  sidebarWorktreeFilter: 'all',
  sidebarViewMode: 'sessions-flat',
  setSidebarProjectSort: vi.fn(),
  setSidebarWorktreeSort: vi.fn(),
  setSidebarWorktreeFilter: vi.fn(),
  setSidebarViewMode: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    return selector ? selector(mockStore) : mockStore
  }
}))

const { FlatSessionsSection } =
  await import('../src/renderer/components/project-sidebar/FlatSessionsSection')

beforeEach(() => {
  mockStore.terminals.clear()
  mockStore.terminalsPanes.clear()
  mockStore.activeProject = null
  mockStore.setActiveProject.mockReset()
  mockStore.setFocusedTerminal.mockReset()
})

describe('FlatSessionsSection', () => {
  it('renders the Sessions header and All Projects button', () => {
    render(
      <FlatSessionsSection
        isCollapsed={false}
        workspaceProjectNames={new Set(['p1'])}
        workspaceTerminalCount={3}
      />
    )
    expect(screen.getByText('Sessions')).toBeInTheDocument()
    expect(screen.getByText('All Projects')).toBeInTheDocument()
    expect(screen.getByText('3')).toBeInTheDocument()
  })

  it('collapses when the Sessions header is clicked', () => {
    render(
      <FlatSessionsSection
        isCollapsed={false}
        workspaceProjectNames={new Set()}
        workspaceTerminalCount={0}
      />
    )
    fireEvent.click(screen.getByText('Sessions'))
    expect(screen.queryByText('All Projects')).not.toBeInTheDocument()
  })

  it('clears active project and focused terminal when All Projects is clicked', () => {
    render(
      <FlatSessionsSection
        isCollapsed={false}
        workspaceProjectNames={new Set()}
        workspaceTerminalCount={0}
      />
    )
    fireEvent.click(screen.getByText('All Projects'))
    expect(mockStore.setActiveProject).toHaveBeenCalledWith(null)
    expect(mockStore.setFocusedTerminal).toHaveBeenCalledWith(null)
  })

  it("leaves out a shell held by a session's terminals panel", () => {
    const term = (id: string, name: string) => ({
      id,
      session: {
        id,
        projectName: 'p1',
        projectPath: '/p1',
        agentType: 'shell',
        displayName: name
      },
      status: 'idle',
      lastOutputTimestamp: 1
    })
    mockStore.terminals.set('agent', term('agent', 'Agent'))
    mockStore.terminals.set('sh1', term('sh1', 'Held shell'))
    mockStore.terminalsPanes.set('agent', { terminals: ['sh1'], activeTab: 0 })

    render(
      <FlatSessionsSection
        isCollapsed={false}
        workspaceProjectNames={new Set(['p1'])}
        workspaceTerminalCount={2}
      />
    )

    // A claimed shell is drawn in its panel and nowhere else. A row here would
    // focus a terminal that has no cell in the grid and no tab in the strip —
    // and the sidebar builds its list straight off the terminals map, so it has
    // to drop them itself.
    expect(screen.getByText('Agent')).toBeInTheDocument()
    expect(screen.queryByText('Held shell')).not.toBeInTheDocument()
  })

  it('shows empty state when there are no sessions', () => {
    render(
      <FlatSessionsSection
        isCollapsed={false}
        workspaceProjectNames={new Set()}
        workspaceTerminalCount={0}
      />
    )
    expect(screen.getByText('No active sessions')).toBeInTheDocument()
  })
})
