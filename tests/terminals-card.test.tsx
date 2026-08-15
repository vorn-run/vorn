// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'

Object.defineProperty(window, 'matchMedia', {
  value: () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
  writable: true,
  configurable: true
})

const createShellTerminal = vi.fn()
const killTerminal = vi.fn().mockResolvedValue(undefined)
Object.defineProperty(window, 'api', {
  value: {
    notifyWidgetStatus: vi.fn(),
    reorderSessions: vi.fn(),
    createShellTerminal: (...a: unknown[]) => createShellTerminal(...a),
    killTerminal: (...a: unknown[]) => killTerminal(...a)
  },
  writable: true,
  configurable: true
})

// The terminal body is the registry's business and has its own tests; this file
// is about which shell the panel decides to draw and what its tabs do.
vi.mock('../src/renderer/components/TerminalPane', () => ({
  TerminalPane: ({ terminalId }: { terminalId: string }) => (
    <div data-testid={`terminal-body-${terminalId}`} />
  )
}))

const { useAppStore } = await import('../src/renderer/stores')
const { TerminalsCard } = await import('../src/renderer/components/TerminalsCard')

const session = (id: string, displayName = id) =>
  ({
    id,
    projectName: 'vorn',
    projectPath: '/repo',
    worktreePath: '/repo',
    agentType: 'shell',
    createdAt: 0,
    displayName
  }) as never

beforeEach(() => {
  localStorage.clear()
  createShellTerminal.mockReset()
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        [
          'owner',
          {
            id: 'owner',
            session: session('owner', 'agent'),
            status: 'idle',
            lastOutputTimestamp: 1
          }
        ],
        [
          'sh1',
          { id: 'sh1', session: session('sh1', 'zsh'), status: 'idle', lastOutputTimestamp: 1 }
        ],
        [
          'sh2',
          { id: 'sh2', session: session('sh2', 'build'), status: 'idle', lastOutputTimestamp: 1 }
        ]
      ]) as never,
      terminalOrder: ['owner', 'sh1', 'sh2'],
      terminalsPanes: new Map(),
      focusedTerminalId: null,
      maximizedPaneId: null,
      config: { defaults: { domBlockRendering: false } } as never
    })
  })
})

describe('TerminalsCard', () => {
  const open = (): void => {
    act(() => {
      useAppStore.getState().openTerminalsPane('owner', 'sh1')
      useAppStore.getState().openTerminalsPane('owner', 'sh2')
    })
  }

  it('draws only the shell in front, and names the rest on tabs', () => {
    open()
    render(<TerminalsCard sessionId="owner" />)

    // Both are tabs; one is drawn. The others keep running — the registry owns
    // the xterm, and a slot is only where a terminal is currently shown. Two
    // slots for one id would fight over a single wrapper.
    expect(screen.getByRole('tab', { name: /zsh/ })).toBeInTheDocument()
    expect(screen.getByTestId('terminal-body-sh2')).toBeInTheDocument()
    expect(screen.queryByTestId('terminal-body-sh1')).not.toBeInTheDocument()
  })

  it('switches which shell is drawn', () => {
    open()
    render(<TerminalsCard sessionId="owner" />)

    fireEvent.click(screen.getByRole('tab', { name: /zsh/ }))
    expect(screen.getByTestId('terminal-body-sh1')).toBeInTheDocument()
    expect(screen.queryByTestId('terminal-body-sh2')).not.toBeInTheDocument()
  })

  it('switches from the keyboard too', () => {
    open()
    render(<TerminalsCard sessionId="owner" />)

    fireEvent.keyDown(screen.getByRole('tab', { name: /zsh/ }), { key: 'Enter' })
    expect(screen.getByTestId('terminal-body-sh1')).toBeInTheDocument()
  })

  it('extracts a shell from its tab', () => {
    open()
    render(<TerminalsCard sessionId="owner" />)

    fireEvent.click(screen.getByRole('button', { name: /Open zsh as its own terminal/ }))

    // Letting go of the claim is the whole action — the terminal was a session
    // all along and is now simply nobody's.
    expect(useAppStore.getState().terminalsPanes.get('owner')?.terminals).toEqual(['sh2'])
    expect(useAppStore.getState().terminals.has('sh1')).toBe(true)
  })

  it('starts another shell in the session it belongs to', async () => {
    createShellTerminal.mockResolvedValue({
      id: 'sh3',
      agentType: 'shell',
      projectName: '',
      projectPath: ''
    })
    open()
    render(<TerminalsCard sessionId="owner" />)

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'New terminal in this session' }))
    })

    // In the session's own directory, not wherever the app happens to be.
    expect(createShellTerminal).toHaveBeenCalledWith('/repo')
    expect(useAppStore.getState().terminalsPanes.get('owner')?.terminals).toEqual([
      'sh1',
      'sh2',
      'sh3'
    ])
  })

  it('closes the panel from its controls', () => {
    open()
    render(<TerminalsCard sessionId="owner" />)

    fireEvent.click(screen.getByRole('button', { name: 'Close Terminals' }))
    expect(useAppStore.getState().terminalsPanes.has('owner')).toBe(false)
  })

  it('closes a shell from its tab', async () => {
    open()
    render(<TerminalsCard sessionId="owner" />)

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Close zsh' }))
    })

    // Closing kills the pty; extraction does not. The two live side by side on
    // the tab and must not be the same action.
    expect(killTerminal).toHaveBeenCalledWith('sh1')
  })

  it('reports a shell that failed to start instead of opening an empty tab', async () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    createShellTerminal.mockRejectedValue(new Error('no pty'))
    open()
    render(<TerminalsCard sessionId="owner" />)

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'New terminal in this session' }))
    })

    expect(useAppStore.getState().terminalsPanes.get('owner')?.terminals).toEqual(['sh1', 'sh2'])
    err.mockRestore()
  })

  it('hands the whole strip to the drag handler, not one tab', () => {
    const onDragStart = vi.fn()
    open()
    render(<TerminalsCard sessionId="owner" onDragStart={onDragStart} />)

    fireEvent.pointerDown(screen.getByTestId('terminals-pane-header-owner'))
    // The pane moves as a unit, so what the grid is told to drag is the pane
    // id — dragging a single tab out is the ↗ button's job.
    expect(onDragStart).toHaveBeenCalledWith('terminals:owner', expect.anything())
  })

  it('renders nothing when the session holds no shells', () => {
    const { container } = render(<TerminalsCard sessionId="owner" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('draws the first shell when the stored active index is stale', () => {
    // Persisted state can name an index past the end after shells are closed
    // elsewhere; an out-of-range read would draw nothing at all.
    act(() => {
      useAppStore.setState({
        terminalsPanes: new Map([['owner', { terminals: ['sh1'], activeTab: 5 }]])
      } as never)
    })
    render(<TerminalsCard sessionId="owner" />)

    expect(screen.getByTestId('terminal-body-sh1')).toBeInTheDocument()
  })
})
