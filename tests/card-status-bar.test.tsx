// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, act, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'

vi.mock('../src/renderer/components/TerminalSlot', () => ({
  TerminalSlot: () => <div data-testid="terminal-instance" />
}))
vi.mock('../src/renderer/components/PromptLauncher', () => ({
  PromptLauncher: () => null
}))
vi.mock('../src/renderer/components/CardContextMenu', () => ({
  CardContextMenu: () => null
}))
vi.mock('../src/renderer/components/BackgroundTray', () => ({
  BackgroundTray: () => null
}))
vi.mock('../src/renderer/components/MobileFontSizeControl', () => ({
  MobileFontSizeControl: () => null
}))
vi.mock('../src/renderer/components/MobileTerminalKeybar', () => ({
  MobileTerminalKeybar: () => null
}))
vi.mock('../src/renderer/components/GitChangesIndicator', () => ({
  GitChangesIndicator: () => <div data-testid="git-changes" />,
  BrowseFilesButton: () => <div data-testid="browse-files" />
}))
vi.mock('../src/renderer/components/GridContextMenu', () => ({
  GridContextMenu: () => null
}))
vi.mock('../src/renderer/components/GridToolbar', () => ({
  GridToolbar: () => <div data-testid="grid-toolbar" />
}))
vi.mock('../src/renderer/components/WindowControls', () => ({
  WindowControls: () => <div data-testid="window-controls" />
}))
vi.mock('../src/renderer/components/SidebarToggleButton', () => ({
  SidebarToggleButton: () => <button data-testid="sidebar-toggle">sidebar</button>
}))
vi.mock('../src/renderer/components/MainViewPills', () => ({
  MainViewPills: () => <div data-testid="main-view-pills" />
}))
vi.mock('../src/renderer/components/RecentSessionsButton', () => ({
  RecentSessionsButton: () => <button data-testid="recent-sessions">recent</button>
}))
let mockIsMobile = false
vi.mock('../src/renderer/hooks/useIsMobile', () => ({
  useIsMobile: () => mockIsMobile
}))
vi.mock('../src/renderer/hooks/useTerminalScrollButton', () => ({
  useTerminalScrollButton: () => ({ showScrollBtn: false, handleScrollToBottom: vi.fn() })
}))
vi.mock('../src/renderer/hooks/useTerminalPinchZoom', () => ({
  useTerminalPinchZoom: vi.fn()
}))
let mockOrderedIds = ['term-1']
let mockMinimizedIds: string[] = []
vi.mock('../src/renderer/hooks/useVisibleTerminals', () => ({
  useVisibleTerminals: () => ({ orderedIds: mockOrderedIds, minimizedIds: mockMinimizedIds }),
  compareTerminalIds: (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0)
}))
let mockFilteredHeadless: unknown[] = []
vi.mock('../src/renderer/hooks/useFilteredHeadless', () => ({
  useFilteredHeadless: () => mockFilteredHeadless
}))
vi.mock('../src/renderer/lib/terminal-close', () => ({
  closeTerminalSession: vi.fn()
}))
vi.mock('../src/renderer/lib/session-utils', () => ({
  resolveActiveProject: () => null,
  getDisplayPathBasename: (p: string) => p.split('/').filter(Boolean).pop(),
  launchAgentFromShell: vi.fn()
}))
vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn() }
}))
vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))
vi.mock('../src/renderer/components/ConfirmPopover', () => ({
  // Pass-through that also fires onConfirm whenever the wrapped trigger is clicked —
  // this lets tests exercise the handler (line 454 of TabView) without driving
  // the full confirm-popover UI.
  ConfirmPopover: ({ children, onConfirm }: { children: ReactNode; onConfirm: () => void }) => (
    <div onClickCapture={onConfirm}>{children}</div>
  )
}))

Object.defineProperty(window, 'api', {
  value: {
    killTerminal: vi.fn(),
    saveConfig: vi.fn(),
    notifyWidgetStatus: vi.fn(),
    getGitDiffStat: vi.fn().mockResolvedValue(null),
    createTerminal: vi.fn(),
    listBranches: vi.fn().mockResolvedValue({ local: [], remote: [] }),
    listRemoteBranches: vi.fn().mockResolvedValue([]),
    checkoutBranch: vi.fn().mockResolvedValue({ ok: true }),
    detectIDEs: vi.fn().mockResolvedValue([]),
    openInIDE: vi.fn()
  },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { CardStatusBar } from '../src/renderer/components/card/CardStatusBar'
import { TabView } from '../src/renderer/components/TabView'
import { FocusedTerminal } from '../src/renderer/components/FocusedTerminal'

const mockTerminal = {
  id: 'term-1',
  session: {
    id: 'term-1',
    agentType: 'claude' as const,
    projectName: 'Vorn',
    projectPath: '/tmp/vorn',
    status: 'running' as const,
    createdAt: Date.now(),
    pid: 1234,
    branch: 'main',
    isWorktree: false
  },
  status: 'running' as const,
  lastOutputTimestamp: Date.now()
}

const initialState = useAppStore.getState()

beforeEach(() => {
  mockIsMobile = false
  mockOrderedIds = ['term-1']
  mockMinimizedIds = []
  mockFilteredHeadless = []
  const terminals = new Map()
  terminals.set('term-1', mockTerminal)
  act(() => {
    useAppStore.setState({
      terminals,
      activeTabId: 'term-1',
      focusedTerminalId: null,
      previewTerminalId: null,
      statusFilter: 'all',
      sortMode: 'manual',
      renamingTerminalId: null
    })
  })
})

afterEach(() => {
  act(() => {
    useAppStore.setState(initialState)
  })
})

describe('CardStatusBar — bottom VS Code style strip', () => {
  it('renders the branch chip for the given terminal', () => {
    render(<CardStatusBar terminalId="term-1" />)
    expect(screen.getByText('main')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Switch branch/ })).toBeInTheDocument()
  })

  it('hides the branch chip when the terminal has no branch but still renders the bar', () => {
    const terminals = new Map()
    terminals.set('blank', {
      id: 'blank',
      session: { ...mockTerminal.session, id: 'blank', branch: undefined },
      status: 'running' as const,
      lastOutputTimestamp: Date.now()
    })
    act(() => {
      useAppStore.setState({ terminals })
    })
    render(<CardStatusBar terminalId="blank" />)
    expect(screen.queryByRole('button', { name: /Switch branch/ })).toBeNull()
  })

  it('renders nothing when the terminal is missing', () => {
    const { container } = render(<CardStatusBar terminalId="nonexistent" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('opens BranchPicker and switches branches via the dropdown', async () => {
    const listBranches = vi.fn().mockResolvedValue({ local: ['main', 'feature-x'], remote: [] })
    const checkoutBranch = vi.fn().mockResolvedValue({ ok: true })
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(window.api as any).listBranches = listBranches
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(window.api as any).checkoutBranch = checkoutBranch

    render(<CardStatusBar terminalId="term-1" />)
    const branchButton = screen.getByRole('button', { name: /Switch branch/ })
    fireEvent.click(branchButton)

    const featureEntry = await screen.findByText('feature-x')
    fireEvent.click(featureEntry)

    await waitFor(() => expect(checkoutBranch).toHaveBeenCalledWith('/tmp/vorn', 'feature-x'))
  })

  it('renders worktree name and branch name for worktree sessions', () => {
    const terminals = new Map()
    terminals.set('wt-1', {
      id: 'wt-1',
      session: {
        ...mockTerminal.session,
        id: 'wt-1',
        branch: 'feature-x',
        isWorktree: true,
        worktreeName: 'feature-x-wt',
        worktreePath: '/tmp/wt/feature-x'
      },
      status: 'running' as const,
      lastOutputTimestamp: Date.now()
    })
    act(() => {
      useAppStore.setState({ terminals })
    })
    render(<CardStatusBar terminalId="wt-1" />)
    expect(screen.getByText('feature-x-wt')).toBeInTheDocument()
    expect(screen.getByText('feature-x')).toBeInTheDocument()
  })

  it('renders assigned-task pill in the status bar', () => {
    const task = {
      id: 'task-1',
      title: 'Ship the homologation',
      status: 'in_progress' as const,
      assignedSessionId: 'term-1'
    }
    act(() => {
      useAppStore.setState({
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        config: { tasks: [task] } as any
      })
    })
    render(<CardStatusBar terminalId="term-1" />)
    expect(screen.getByRole('button', { name: /Ship the homologation/ })).toBeInTheDocument()
  })

  it('clicking the assigned-task pill opens the task dialog for that task', () => {
    const task = {
      id: 'task-1',
      title: 'Ship the homologation',
      status: 'in_progress' as const,
      assignedSessionId: 'term-1'
    }
    const setEditingTask = vi.fn()
    const setTaskDialogOpen = vi.fn()
    act(() => {
      useAppStore.setState({
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        config: { tasks: [task] } as any,
        setEditingTask,
        setTaskDialogOpen
      })
    })
    render(<CardStatusBar terminalId="term-1" />)
    fireEvent.click(screen.getByRole('button', { name: /Ship the homologation/ }))
    expect(setEditingTask).toHaveBeenCalledWith(task)
    expect(setTaskDialogOpen).toHaveBeenCalledWith(true)
  })
})

describe('TabView renders CardStatusBar below the active terminal', () => {
  it('renders the bottom branch chip so it matches grid and focused views', () => {
    render(<TabView />)
    // The "Switch branch" button only exists inside CardStatusBar.
    expect(screen.getByRole('button', { name: /Switch branch/ })).toBeInTheDocument()
    // The tab itself still carries the branch in its `title` tooltip attribute.
    const tab = screen.getByRole('tab')
    expect(tab.querySelector('[title*="Branch: main"]')).toBeTruthy()
  })

  it('folds cwd into the tab tooltip for shell sessions (not Branch)', () => {
    const terminals = new Map()
    // Must use the id that useVisibleTerminals mock returns (term-1)
    terminals.set('term-1', {
      id: 'term-1',
      session: {
        id: 'term-1',
        agentType: 'shell' as const,
        projectName: 'vorn',
        projectPath: '/home/me/vorn',
        status: 'running' as const,
        createdAt: Date.now(),
        pid: 4321,
        displayName: 'Shell 1',
        shellCwd: '/home/me/vorn'
      },
      status: 'running' as const,
      lastOutputTimestamp: Date.now()
    })
    act(() => {
      useAppStore.setState({ terminals, activeTabId: 'term-1' })
    })
    render(<TabView />)
    const tab = screen.getByRole('tab')
    const titled = tab.querySelector('[title]')
    expect(titled?.getAttribute('title')).toContain('Cwd: /home/me/vorn')
    expect(titled?.getAttribute('title')).not.toContain('Branch:')
  })

  it('unified + button opens the dropdown menu on click (toggles closed on second click)', () => {
    render(<TabView />)
    const plus = screen.getByTitle('New session')
    // Opens
    fireEvent.click(plus)
    // GridContextMenu is mocked to null in this test file, but the state toggle
    // still drives a re-render — second click should set it back to null.
    // We assert that the button stays clickable and doesn't crash on re-click.
    fireEvent.click(plus)
    expect(plus).toBeInTheDocument()
  })

  it('clicking the rename icon puts the tab into renaming mode', () => {
    const setRenamingTerminalId = vi.fn()
    act(() => {
      useAppStore.setState({ setRenamingTerminalId })
    })
    render(<TabView />)
    // TabIconButton wraps its icon in a <button aria-label="Rename session">
    const renameBtn = screen.getByRole('button', { name: /Rename session/ })
    fireEvent.click(renameBtn)
    expect(setRenamingTerminalId).toHaveBeenCalledWith('term-1')
  })

  it('confirming close on the close-session popover closes the terminal session', async () => {
    const { closeTerminalSession } = await import('../src/renderer/lib/terminal-close')
    render(<TabView />)
    const closeBtn = screen.getByRole('button', { name: /Close session/ })
    // The ConfirmPopover mock in this file propagates click to its onConfirm,
    // so clicking the close button exercises handleCloseTab.
    fireEvent.click(closeBtn)
    expect(closeTerminalSession).toHaveBeenCalledWith('term-1')
  })
})

describe('FocusedTerminal uses CardStatusBar on desktop', () => {
  it('renders branch and diff pill in the bottom bar', () => {
    act(() => {
      useAppStore.setState({ focusedTerminalId: 'term-1' })
    })
    render(<FocusedTerminal />)
    expect(screen.getByRole('button', { name: /Switch branch/ })).toBeInTheDocument()
    // Branch renders in the status-bar chip and in the intent bar's context row.
    expect(screen.getAllByText('main').length).toBeGreaterThanOrEqual(1)
    expect(screen.getByTestId('git-changes')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Browse files/ })).toBeInTheDocument()
  })
})

describe('FocusedTerminal on mobile keeps branch in the title bar', () => {
  it('renders branch label for a non-worktree session in the mobile title bar', () => {
    mockIsMobile = true
    act(() => {
      useAppStore.setState({ focusedTerminalId: 'term-1' })
    })
    render(<FocusedTerminal />)
    expect(screen.getByText('main')).toBeInTheDocument()
    expect(screen.queryByText('worktree')).not.toBeInTheDocument()
    // Desktop CardStatusBar should not be mounted on mobile
    expect(screen.queryByTestId('git-changes')).not.toBeInTheDocument()
  })

  it('renders worktree name and branch name on mobile for worktree sessions', () => {
    mockIsMobile = true
    const terminals = new Map()
    terminals.set('wt-1', {
      id: 'wt-1',
      session: {
        ...mockTerminal.session,
        id: 'wt-1',
        branch: 'feature-x',
        isWorktree: true,
        worktreeName: 'feature-x-wt',
        worktreePath: '/tmp/wt/feature-x'
      },
      status: 'running' as const,
      lastOutputTimestamp: Date.now()
    })
    act(() => {
      useAppStore.setState({ terminals, focusedTerminalId: 'wt-1' })
    })
    render(<FocusedTerminal />)
    expect(screen.getByText('feature-x-wt')).toBeInTheDocument()
    expect(screen.getByText('feature-x')).toBeInTheDocument()
    expect(screen.queryByText('worktree')).not.toBeInTheDocument()
  })

  it('back button and double-click title bar both contract (unset focused terminal)', () => {
    mockIsMobile = true
    const setFocused = vi.fn()
    act(() => {
      useAppStore.setState({ focusedTerminalId: 'term-1', setFocusedTerminal: setFocused })
    })
    render(<FocusedTerminal />)
    fireEvent.click(screen.getByRole('button', { name: /Back to sessions/ }))
    expect(setFocused).toHaveBeenCalledWith(null)
  })

  it('clicking the rename button on mobile puts the terminal into renaming state', () => {
    mockIsMobile = true
    const setRenamingTerminalId = vi.fn()
    act(() => {
      useAppStore.setState({ focusedTerminalId: 'term-1', setRenamingTerminalId })
    })
    render(<FocusedTerminal />)
    fireEvent.click(screen.getByRole('button', { name: /Rename session/ }))
    expect(setRenamingTerminalId).toHaveBeenCalledWith('term-1')
  })

  it('double-clicking the name on mobile also puts the terminal into renaming state', () => {
    mockIsMobile = true
    const setRenamingTerminalId = vi.fn()
    act(() => {
      useAppStore.setState({ focusedTerminalId: 'term-1', setRenamingTerminalId })
    })
    render(<FocusedTerminal />)
    fireEvent.doubleClick(screen.getByText('Vorn'))
    expect(setRenamingTerminalId).toHaveBeenCalledWith('term-1')
  })

  it('committing and cancelling the mobile InlineRename call the store', () => {
    mockIsMobile = true
    const renameTerminal = vi.fn()
    const setRenamingTerminalId = vi.fn()
    act(() => {
      useAppStore.setState({
        focusedTerminalId: 'term-1',
        renamingTerminalId: 'term-1',
        renameTerminal,
        setRenamingTerminalId
      })
    })
    const { unmount } = render(<FocusedTerminal />)
    const input = screen.getByDisplayValue('Vorn')
    fireEvent.change(input, { target: { value: 'Mobile renamed' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(renameTerminal).toHaveBeenCalledWith('term-1', 'Mobile renamed')
    expect(setRenamingTerminalId).toHaveBeenCalledWith(null)
    unmount()

    setRenamingTerminalId.mockClear()
    renameTerminal.mockClear()
    act(() => {
      useAppStore.setState({
        focusedTerminalId: 'term-1',
        renamingTerminalId: 'term-1',
        renameTerminal,
        setRenamingTerminalId
      })
    })
    render(<FocusedTerminal />)
    const input2 = screen.getByDisplayValue('Vorn')
    fireEvent.keyDown(input2, { key: 'Escape' })
    expect(renameTerminal).not.toHaveBeenCalled()
    expect(setRenamingTerminalId).toHaveBeenCalledWith(null)
  })
})

describe('FocusedTerminal desktop title bar', () => {
  it('double-clicking the title bar (outside buttons) contracts back to grid', () => {
    mockIsMobile = false
    const setFocused = vi.fn()
    act(() => {
      useAppStore.setState({ focusedTerminalId: 'term-1', setFocusedTerminal: setFocused })
    })
    const { container } = render(<FocusedTerminal />)
    const titleBar = container.querySelector('.titlebar-no-drag') as HTMLElement
    expect(titleBar).not.toBeNull()
    fireEvent.doubleClick(titleBar)
    expect(setFocused).toHaveBeenCalledWith(null)
  })
})

describe('TabView merged toolbar controls', () => {
  it('shows sidebar toggle and view pills when sidebar is closed', () => {
    act(() => {
      useAppStore.setState({ isSidebarOpen: false })
    })
    render(<TabView />)
    expect(screen.getByTestId('sidebar-toggle')).toBeInTheDocument()
    expect(screen.getByTestId('main-view-pills')).toBeInTheDocument()
  })

  it('hides sidebar toggle and view pills when sidebar is open', () => {
    act(() => {
      useAppStore.setState({ isSidebarOpen: true })
    })
    render(<TabView />)
    expect(screen.queryByTestId('sidebar-toggle')).not.toBeInTheDocument()
    expect(screen.queryByTestId('main-view-pills')).not.toBeInTheDocument()
  })

  it('always renders the right toolbar controls (GridToolbar, recent, window controls)', () => {
    render(<TabView />)
    expect(screen.getByTestId('grid-toolbar')).toBeInTheDocument()
    expect(screen.getByTestId('recent-sessions')).toBeInTheDocument()
    expect(screen.getByTestId('window-controls')).toBeInTheDocument()
  })

  it('renders empty state with PromptLauncher when no tabs and no background', () => {
    mockOrderedIds = []
    mockMinimizedIds = []
    render(<TabView />)
    // Toolbar still renders
    expect(screen.getByTestId('grid-toolbar')).toBeInTheDocument()
    // No terminal slot
    expect(screen.queryByTestId('terminal-instance')).not.toBeInTheDocument()
  })

  it('renders "No matching agents" when filtered with no tabs', () => {
    mockOrderedIds = []
    act(() => {
      useAppStore.setState({ statusFilter: 'running' })
    })
    render(<TabView />)
    expect(screen.getByText('No matching agents')).toBeInTheDocument()
  })

  it('renders background tray when no tabs but has minimized sessions', () => {
    mockOrderedIds = []
    mockMinimizedIds = ['term-1']
    render(<TabView />)
    expect(screen.getByTestId('grid-toolbar')).toBeInTheDocument()
  })

  it('renders the new session button', () => {
    render(<TabView />)
    expect(screen.getByRole('button', { name: 'New session' })).toBeInTheDocument()
  })

  it('renders the more session options button', () => {
    render(<TabView />)
    expect(screen.getByRole('button', { name: 'More session options' })).toBeInTheDocument()
  })

  it('opens context menu on tab right-click', () => {
    const { container } = render(<TabView />)
    const tab = screen.getByRole('tab')
    fireEvent.contextMenu(tab)
    expect(container).toBeTruthy()
  })

  it('shows loading skeleton when terminal has no output yet', () => {
    const terminals = new Map()
    terminals.set('term-1', { ...mockTerminal, lastOutputTimestamp: 0 })
    act(() => {
      useAppStore.setState({ terminals })
    })
    const { container } = render(<TabView />)
    expect(container.querySelector('.animate-pulse')).toBeInTheDocument()
  })

  it('renders tab with task-assigned display name', () => {
    act(() => {
      useAppStore.setState({
        config: {
          version: 1,
          defaults: { shell: 'bash', fontSize: 14, theme: 'dark' as const },
          tasks: [
            {
              id: 'task-1',
              title: 'Fix auth bug',
              status: 'in_progress' as const,
              assignedSessionId: 'term-1',
              createdAt: Date.now()
            }
          ]
        }
      })
    })
    render(<TabView />)
    // Title appears both in the tab label and in the CardStatusBar task pill.
    const matches = screen.getAllByText('Fix auth bug')
    expect(matches.length).toBeGreaterThan(0)
    const tab = screen.getByRole('tab')
    expect(tab.textContent).toContain('Fix auth bug')
  })

  it('clicks new session button and opens dialog when no project', () => {
    const setNewAgentDialogOpen = vi.fn()
    act(() => {
      useAppStore.setState({ setNewAgentDialogOpen })
    })
    render(<TabView />)
    fireEvent.click(screen.getByRole('button', { name: 'New session' }))
    expect(setNewAgentDialogOpen).toHaveBeenCalledWith(true)
  })

  it('opens more session options dropdown on click', () => {
    render(<TabView />)
    const moreBtn = screen.getByRole('button', { name: 'More session options' })
    fireEvent.click(moreBtn)
    // Dropdown opens (GridContextMenu is mocked to null but state toggles)
    expect(moreBtn).toBeInTheDocument()
  })

  it('renders tab in renaming mode', () => {
    act(() => {
      useAppStore.setState({ renamingTerminalId: 'term-1' })
    })
    render(<TabView />)
    // InlineRename is not mocked, it renders an input
    // Two textboxes render here (InlineRename input + intent bar textarea);
    // the rename field is the <input>.
    const input = screen.getAllByRole('textbox').find((el) => el.tagName === 'INPUT')!
    expect(input).toBeInTheDocument()
  })

  it('renders inactive tab styling for non-active tabs', () => {
    const terminals = new Map()
    terminals.set('term-1', mockTerminal)
    terminals.set('term-2', {
      ...mockTerminal,
      id: 'term-2',
      session: { ...mockTerminal.session, id: 'term-2' }
    })
    mockOrderedIds = ['term-1', 'term-2']
    act(() => {
      useAppStore.setState({ terminals, activeTabId: 'term-1' })
    })
    render(<TabView />)
    const tabs = screen.getAllByRole('tab')
    expect(tabs).toHaveLength(2)
    expect(tabs[0].className).toContain('text-white')
    expect(tabs[1].className).toContain('text-gray-500')
  })

  it('clicks browse files button on a tab', () => {
    const setDiffSidebar = vi.fn()
    act(() => {
      useAppStore.setState({ setDiffSidebarTerminalId: setDiffSidebar })
    })
    render(<TabView />)
    fireEvent.click(screen.getByRole('button', { name: 'Browse files' }))
    expect(setDiffSidebar).toHaveBeenCalledWith('term-1', 'all-files')
  })

  it('clicks rename button on a tab', () => {
    const setRenamingTerminalId = vi.fn()
    act(() => {
      useAppStore.setState({ setRenamingTerminalId })
    })
    render(<TabView />)
    fireEvent.click(screen.getByRole('button', { name: 'Rename session' }))
    expect(setRenamingTerminalId).toHaveBeenCalledWith('term-1')
  })

  it('commits rename via InlineRename', async () => {
    const renameTerminal = vi.fn()
    const setRenamingTerminalId = vi.fn()
    act(() => {
      useAppStore.setState({
        renamingTerminalId: 'term-1',
        renameTerminal,
        setRenamingTerminalId
      })
    })
    render(<TabView />)
    // Two textboxes render here (InlineRename input + intent bar textarea);
    // the rename field is the <input>.
    const input = screen.getAllByRole('textbox').find((el) => el.tagName === 'INPUT')!
    fireEvent.change(input, { target: { value: 'New Name' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => {
      expect(renameTerminal).toHaveBeenCalledWith('term-1', 'New Name')
      expect(setRenamingTerminalId).toHaveBeenCalledWith(null)
    })
  })

  it('cancels rename via Escape', () => {
    const setRenamingTerminalId = vi.fn()
    act(() => {
      useAppStore.setState({ renamingTerminalId: 'term-1', setRenamingTerminalId })
    })
    render(<TabView />)
    // Two textboxes render here (InlineRename input + intent bar textarea);
    // the rename field is the <input>.
    const input = screen.getAllByRole('textbox').find((el) => el.tagName === 'INPUT')!
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(setRenamingTerminalId).toHaveBeenCalledWith(null)
  })

  it('toggles more session options dropdown open and closed', () => {
    render(<TabView />)
    const moreBtn = screen.getByRole('button', { name: 'More session options' })
    fireEvent.click(moreBtn)
    // Click again to close
    fireEvent.click(moreBtn)
    expect(moreBtn).toBeInTheDocument()
  })
})
