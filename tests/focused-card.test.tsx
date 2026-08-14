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
// jsdom reports no platform, so `isMac` is false and the header renders with
// no drag region at all — which made the opt-out assertion below vacuously
// true. Force the branch that actually has the hazard.
vi.mock('../src/renderer/lib/platform', async (orig) => ({
  ...(await orig<typeof import('../src/renderer/lib/platform')>()),
  isMac: true
}))

const { useAppStore } = await import('../src/renderer/stores')
const { FocusedStage } = await import('../src/renderer/components/FocusedStage')
const { usePromotedCardSubject } = await import('../src/renderer/hooks/usePromotedCards')
const { renderHook } = await import('@testing-library/react')

const session = (id: string) =>
  ({
    id,
    projectName: 'vorn',
    projectPath: '/repo',
    agentType: 'claude',
    createdAt: 0,
    displayName: id,
    // With a branch, so the owner label renders its switcher — an interactive
    // control inside the drag region, and the one most likely to go quietly
    // dead. Without it the label draws no button and the sweep below has
    // nothing to catch.
    branch: 'main'
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

  it('gives up focus when the focused card is closed', () => {
    // Otherwise the app is stranded: the stage is chosen by "is anything
    // focused", and the titlebar is dropped while something is — so a focus id
    // pointing at a closed card renders an empty window with no chrome and no
    // way back but Escape. This test previously asserted that empty render as
    // if it were the intended behaviour.
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
    })
    act(() => {
      useAppStore.setState({ focusedTerminalId: cardId } as never)
      useAppStore.getState().closeEditorPane(cardId)
    })

    expect(useAppStore.getState().focusedTerminalId).toBeNull()
  })

  it('gives up focus when the focused card is put back in its session', () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
    })
    act(() => {
      useAppStore.setState({ focusedTerminalId: cardId } as never)
      useAppStore.getState().returnCardToSession(cardId)
    })

    expect(useAppStore.getState().focusedTerminalId).toBeNull()
  })

  it('keeps every interactive header control out of the drag region', () => {
    // macOS `-webkit-app-region: drag` swallows clicks on everything inside it,
    // so a control in this header without the opt-out is visibly present and
    // completely dead. jsdom has no app-region, so the class is the only proxy
    // available — but it has to be checked on *every* control, not just the one
    // that was reported: the branch switcher in the owner label is interactive
    // too, and is the one most likely to go quietly dead.
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
    })
    act(() => useAppStore.setState({ focusedTerminalId: cardId } as never))
    render(<FocusedStage />)

    const header = screen.getByTestId(`focused-card-${cardId}`)
    expect(header.className).toContain('titlebar-drag')

    const controls = header.querySelectorAll('button, input, [role="button"]')
    expect(controls.length).toBeGreaterThan(0)
    for (const control of controls) {
      expect(control.closest('.titlebar-no-drag')).not.toBeNull()
    }
  })
})

/**
 * The read behind a card's tab, its dock pill and its focus stage.
 *
 * Worth its own test because the obvious way to write it is a trap, and the
 * trap is silent until the component mounts and React aborts the render.
 */
describe('usePromotedCardSubject', () => {
  it('returns the same object across unrelated store updates', () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/src/server.ts')
    })
    const { result, rerender } = renderHook(() => usePromotedCardSubject(cardId))
    const first = result.current
    expect(first).toMatchObject({ kind: 'editor', name: 'server.ts', sessionId: 't1' })

    // A selector that built this object inline would hand back a new reference
    // here, zustand would read it as a changed snapshot, and the component
    // would re-render and re-select until React gave up.
    act(() => useAppStore.setState({ selectedTerminalId: 'anything' } as never))
    rerender()
    expect(result.current).toBe(first)
  })

  it('follows the card when what it shows changes', () => {
    // Navigating a popped-out page, the way its address bar does. Note this is
    // the browser case on purpose: `openEditorPane` stamps its first argument
    // as the owner, so calling it with a card id would rewrite the record's
    // `sessionId` to the card's own — quietly turning the card back into a
    // session pane. Nothing in the app does that; it is a trap for anything
    // that later tries to "change the file a card shows".
    let cardId: string | null = null
    act(() => {
      useAppStore.getState().openBrowserPane('t1', 'first.example')
      cardId = useAppStore.getState().promoteBrowserTab('t1', 0)
    })
    const id = cardId as unknown as string
    const { result, rerender } = renderHook(() => usePromotedCardSubject(id))
    expect(result.current?.name).toBe('first.example')

    act(() => useAppStore.getState().openBrowserPane(id, 'second.example'))
    rerender()
    expect(result.current?.name).toBe('second.example')
    // And it is still a card: navigating must not rewrite who owns it.
    expect(result.current?.sessionId).toBe('t1')
  })

  it('answers null for an id that is not a card', () => {
    const { result } = renderHook(() => usePromotedCardSubject('t1'))
    expect(result.current).toBeNull()
  })
})
