// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, act, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))
vi.mock('../src/renderer/components/ConfirmPopover', () => ({
  ConfirmPopover: ({ children }: { children: ReactNode }) => <>{children}</>
}))
vi.mock('../src/renderer/components/GitChangesIndicator', () => ({
  GitChangesIndicator: () => <div data-testid="git-changes" />,
  BrowseFilesButton: () => <div data-testid="browse-files" />
}))
vi.mock('../src/renderer/lib/terminal-close', () => ({
  closeTerminalSession: vi.fn().mockResolvedValue(undefined)
}))
vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn() }
}))

Object.defineProperty(window, 'api', {
  value: {
    killTerminal: vi.fn(),
    listBranches: vi.fn().mockResolvedValue({ local: [], remote: [] }),
    listRemoteBranches: vi.fn().mockResolvedValue([]),
    checkoutBranch: vi.fn().mockResolvedValue({ ok: true })
  },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { CardHeader } from '../src/renderer/components/card/CardHeader'

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
    displayName: 'Card redesign',
    branch: 'main',
    isWorktree: false
  },
  status: 'running' as const,
  lastOutputTimestamp: Date.now()
}

const initialState = useAppStore.getState()

let setRenamingTerminalId: ReturnType<typeof vi.fn>

beforeEach(() => {
  vi.clearAllMocks()
  setRenamingTerminalId = vi.fn()
  const terminals = new Map()
  terminals.set('term-1', mockTerminal)
  act(() => {
    useAppStore.setState({
      terminals,
      renamingTerminalId: null,
      setRenamingTerminalId
    })
  })
})

afterEach(() => {
  act(() => {
    useAppStore.setState(initialState)
  })
})

describe('CardHeader', () => {
  it('renders the display name and ⌘N index badge in mini variant', () => {
    render(<CardHeader terminalId="term-1" variant="mini" index={0} />)
    expect(screen.getByText('Card redesign')).toBeInTheDocument()
    // ⌘1 or Ctrl+1 badge — use a permissive matcher
    expect(screen.getByText(/1/)).toBeInTheDocument()
  })

  it('renders nothing when the terminal is missing', () => {
    const { container } = render(<CardHeader terminalId="missing" variant="mini" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('double-click on the name area fires onDoubleClick', () => {
    const handleDoubleClick = vi.fn()
    render(<CardHeader terminalId="term-1" variant="mini" onDoubleClick={handleDoubleClick} />)
    fireEvent.doubleClick(screen.getByText('Card redesign'))
    expect(handleDoubleClick).toHaveBeenCalled()
  })

  it('pointer-down on a draggable name area fires onDragStart with the terminal id', () => {
    const handleDragStart = vi.fn()
    render(
      <CardHeader terminalId="term-1" variant="mini" draggable onDragStart={handleDragStart} />
    )
    fireEvent.pointerDown(screen.getByText('Card redesign'))
    expect(handleDragStart).toHaveBeenCalledWith('term-1', expect.anything())
  })

  it('clicking the rename pencil puts the terminal into renaming state', () => {
    render(<CardHeader terminalId="term-1" variant="mini" />)
    fireEvent.click(screen.getByRole('button', { name: /Rename session/ }))
    expect(setRenamingTerminalId).toHaveBeenCalledWith('term-1')
  })

  it('renders an InlineRename input when the terminal is the one being renamed', () => {
    act(() => {
      useAppStore.setState({ renamingTerminalId: 'term-1' })
    })
    render(<CardHeader terminalId="term-1" variant="mini" />)
    // InlineRename renders an input with the current name
    expect(screen.getByDisplayValue('Card redesign')).toBeInTheDocument()
  })

  it('committing the rename input calls renameTerminal and clears renaming state', () => {
    const renameTerminal = vi.fn()
    act(() => {
      useAppStore.setState({ renamingTerminalId: 'term-1', renameTerminal })
    })
    render(<CardHeader terminalId="term-1" variant="mini" />)
    const input = screen.getByDisplayValue('Card redesign')
    fireEvent.change(input, { target: { value: 'New name' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(renameTerminal).toHaveBeenCalledWith('term-1', 'New name')
    expect(setRenamingTerminalId).toHaveBeenCalledWith(null)
  })

  it('pressing Escape in the rename input cancels without calling rename', () => {
    const renameTerminal = vi.fn()
    act(() => {
      useAppStore.setState({ renamingTerminalId: 'term-1', renameTerminal })
    })
    render(<CardHeader terminalId="term-1" variant="mini" />)
    const input = screen.getByDisplayValue('Card redesign')
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(renameTerminal).not.toHaveBeenCalled()
    expect(setRenamingTerminalId).toHaveBeenCalledWith(null)
  })

  it('pointer-down on the rename button does not open rename (only click does)', () => {
    render(<CardHeader terminalId="term-1" variant="mini" />)
    fireEvent.pointerDown(screen.getByRole('button', { name: /Rename session/ }))
    expect(setRenamingTerminalId).not.toHaveBeenCalled()
  })

  it('sits on the same ground as the rest of the card', () => {
    // A card is one object. The header used to take the pane rung while the
    // body took the card rung, so it read as a separate band stuck to the top
    // rather than part of the card — the hairline is what divides them.
    const { container } = render(<CardHeader terminalId="term-1" variant="mini" />)
    const header = container.firstElementChild as HTMLElement
    expect(header.style.background).toBe('var(--color-surface-raised)')
    expect(header.className).toContain('border-b')
  })

  it('focused variant shows the action cluster without needing hover', () => {
    render(<CardHeader terminalId="term-1" variant="focused" />)
    expect(screen.getByRole('button', { name: /Collapse/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Close/ })).toBeInTheDocument()
  })
})
