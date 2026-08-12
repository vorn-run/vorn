// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, act, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))
vi.mock('../src/renderer/components/ConfirmPopover', () => ({
  ConfirmPopover: ({ children, onConfirm }: { children: ReactNode; onConfirm: () => void }) => (
    <span
      data-testid="confirm-wrap"
      onClick={() => {
        onConfirm()
      }}
    >
      {children}
    </span>
  )
}))
vi.mock('../src/renderer/lib/terminal-close', () => ({
  closeTerminalSession: vi.fn().mockResolvedValue(undefined)
}))
vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn() }
}))

Object.defineProperty(window, 'api', {
  value: { killTerminal: vi.fn() },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { CardActionCluster } from '../src/renderer/components/card/CardActionCluster'
import { closeTerminalSession } from '../src/renderer/lib/terminal-close'
import { toast } from '../src/renderer/components/Toast'

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

let toggleFilesPane: ReturnType<typeof vi.fn>
let setFocused: ReturnType<typeof vi.fn>
let toggleMinimized: ReturnType<typeof vi.fn>

beforeEach(() => {
  vi.clearAllMocks()
  toggleFilesPane = vi.fn()
  setFocused = vi.fn()
  toggleMinimized = vi.fn()
  const terminals = new Map()
  terminals.set('term-1', mockTerminal)
  act(() => {
    useAppStore.setState({
      terminals,
      focusedTerminalId: null,
      toggleFilesPane,
      setFocusedTerminal: setFocused,
      toggleMinimized
    })
  })
})

afterEach(() => {
  act(() => {
    useAppStore.setState(initialState)
  })
})

describe('CardActionCluster — mini variant (grid, hover-revealed)', () => {
  it('renders folder, minimize, expand, and close buttons', () => {
    render(<CardActionCluster terminalId="term-1" variant="mini" />)
    expect(screen.getByRole('button', { name: /Browse files/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Minimize/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Expand/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Close/ })).toBeInTheDocument()
  })

  it("Browse files toggles the session's own Files pane", () => {
    render(<CardActionCluster terminalId="term-1" variant="mini" />)
    fireEvent.click(screen.getByRole('button', { name: /Browse files/ }))
    expect(toggleFilesPane).toHaveBeenCalledWith('term-1')
  })

  it('Minimize toggles the minimized state for this terminal', () => {
    render(<CardActionCluster terminalId="term-1" variant="mini" />)
    fireEvent.click(screen.getByRole('button', { name: /Minimize/ }))
    expect(toggleMinimized).toHaveBeenCalledWith('term-1')
  })

  it('Expand focuses this terminal', () => {
    render(<CardActionCluster terminalId="term-1" variant="mini" />)
    fireEvent.click(screen.getByRole('button', { name: /Expand/ }))
    expect(setFocused).toHaveBeenCalledWith('term-1')
  })

  it('Close triggers closeTerminalSession and a success toast', async () => {
    render(<CardActionCluster terminalId="term-1" variant="mini" />)
    await act(async () => {
      fireEvent.click(screen.getByTestId('confirm-wrap'))
    })
    expect(closeTerminalSession).toHaveBeenCalledWith('term-1')
    expect(toast.success).toHaveBeenCalledWith(expect.stringContaining('Card redesign'))
  })
})

describe('CardActionCluster — focused variant (full overlay, always visible)', () => {
  it('renders folder + collapse + close, without minimize', () => {
    render(<CardActionCluster terminalId="term-1" variant="focused" />)
    expect(screen.getByRole('button', { name: /Browse files/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Minimize/ })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Collapse/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Close/ })).toBeInTheDocument()
  })

  it('Collapse clears the focused terminal (back to grid)', () => {
    render(<CardActionCluster terminalId="term-1" variant="focused" />)
    fireEvent.click(screen.getByRole('button', { name: /Collapse/ }))
    expect(setFocused).toHaveBeenCalledWith(null)
  })
})

describe('CardActionCluster guards', () => {
  it('renders nothing when the terminal is missing', () => {
    const { container } = render(<CardActionCluster terminalId="nope" variant="mini" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('pointer-down on action buttons does not trigger their click handlers', () => {
    render(<CardActionCluster terminalId="term-1" variant="mini" />)
    fireEvent.pointerDown(screen.getByRole('button', { name: /Browse files/ }))
    fireEvent.pointerDown(screen.getByRole('button', { name: /Minimize/ }))
    fireEvent.pointerDown(screen.getByRole('button', { name: /Expand/ }))
    fireEvent.pointerDown(screen.getByRole('button', { name: /Close/ }))
    // onPointerDown calls stopPropagation only — none of the click-triggered store actions fire.
    expect(toggleFilesPane).not.toHaveBeenCalled()
    expect(toggleMinimized).not.toHaveBeenCalled()
    expect(setFocused).not.toHaveBeenCalled()
  })
})
