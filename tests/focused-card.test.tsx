// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'

Object.defineProperty(window, 'matchMedia', {
  value: () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
  writable: true,
  configurable: true
})
Object.defineProperty(window, 'api', {
  value: { notifyWidgetStatus: vi.fn(), listDir: vi.fn().mockResolvedValue([]) },
  writable: true,
  configurable: true
})

// The card bodies are heavy (an editor, a <webview>) and covered by their own
// tests; this file is about what the focus stage decides to render.
vi.mock('../src/renderer/components/PromotedPaneCard', () => ({
  PromotedPaneCard: ({ cardId }: { cardId: string }) => <div data-testid={`body-${cardId}`} />
}))
vi.mock('../src/renderer/components/FocusedTerminal', () => ({
  FocusedTerminal: () => <div data-testid="focused-terminal" />
}))

const { useAppStore } = await import('../src/renderer/stores')
const { FocusedStage } = await import('../src/renderer/components/FocusedStage')

const session = (id: string) =>
  ({
    id,
    projectName: 'vorn',
    projectPath: '/repo',
    agentType: 'claude',
    createdAt: 0,
    displayName: id
  }) as never

beforeEach(() => {
  localStorage.clear()
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        ['t1', { id: 't1', session: session('t1'), status: 'idle', lastOutputTimestamp: 1 }]
      ]) as never,
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      focusedTerminalId: null,
      previewTerminalId: null
    })
  })
})

/**
 * The focus stage takes the same ids the grid and the tab strip take. Before
 * this it only knew how to render a session, so focusing a card focused its
 * owner instead and the card arrived as a passenger.
 */
describe('FocusedStage', () => {
  it('renders the session stage for a session id', () => {
    act(() => useAppStore.setState({ focusedTerminalId: 't1' } as never))
    render(<FocusedStage />)
    expect(screen.getByTestId('focused-terminal')).toBeInTheDocument()
  })

  it('renders the card alone for a card id', () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/src/server.ts')
    })
    act(() => useAppStore.setState({ focusedTerminalId: cardId } as never))
    render(<FocusedStage />)

    // The card, and no session stage beside or beneath it.
    expect(screen.getByTestId(`body-${cardId}`)).toBeInTheDocument()
    expect(screen.queryByTestId('focused-terminal')).not.toBeInTheDocument()
  })

  it('names the card by its filename and says whose it is', () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/src/server.ts')
    })
    act(() => useAppStore.setState({ focusedTerminalId: cardId } as never))
    render(<FocusedStage />)

    expect(screen.getByText('server.ts')).toBeInTheDocument()
    // Out on a stage of its own, which worktree this came from is the thing
    // that is no longer obvious and stops being recoverable from context.
    expect(screen.getByText('vorn')).toBeInTheDocument()
  })

  it('names a popped-out page by its host', () => {
    let cardId: string | null = null
    act(() => {
      useAppStore.getState().openBrowserPane('t1', 'vorn.dev')
      cardId = useAppStore.getState().promoteBrowserTab('t1', 0)
    })
    act(() => useAppStore.setState({ focusedTerminalId: cardId } as never))
    render(<FocusedStage />)

    expect(screen.getByText('vorn.dev')).toBeInTheDocument()
  })

  it('collapses back to the grid', () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
    })
    act(() => useAppStore.setState({ focusedTerminalId: cardId } as never))
    render(<FocusedStage />)

    fireEvent.click(screen.getByRole('button', { name: /Collapse a\.ts/ }))
    expect(useAppStore.getState().focusedTerminalId).toBeNull()
  })

  it('renders nothing for a card closed while it was focused', () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
    })
    act(() => {
      useAppStore.setState({ focusedTerminalId: cardId } as never)
      useAppStore.getState().closeEditorPane(cardId)
    })

    // The stage empties on the next pass; drawing a header for a card that is
    // gone would leave a name and a branch with nothing behind them.
    const { container } = render(<FocusedStage />)
    expect(container).toBeEmptyDOMElement()
  })
})
