// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import { splitPaneWeights, resizePaneWeights } from '../src/renderer/lib/split-ratio'

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

// The three pane bodies are heavy (file trees, editors, a <webview>) and are
// covered by their own tests; this file is about how the column arranges them.
vi.mock('../src/renderer/components/FilesCard', () => ({
  FilesCard: ({ sessionId }: { sessionId: string }) => <div data-testid={`files-${sessionId}`} />
}))
vi.mock('../src/renderer/components/EditorCard', () => ({
  EditorCard: ({ sessionId }: { sessionId: string }) => <div data-testid={`editor-${sessionId}`} />
}))
vi.mock('../src/renderer/components/BrowserCard', () => ({
  BrowserCard: ({ sessionId }: { sessionId: string }) => (
    <div data-testid={`browser-${sessionId}`} />
  )
}))

const { useAppStore } = await import('../src/renderer/stores')
const { PaneColumn } = await import('../src/renderer/components/PaneColumn')

const session = (id: string) =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    createdAt: 0,
    displayName: id
  }) as never

beforeEach(() => {
  localStorage.clear()
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        ['t1', { id: 't1', session: session('t1'), status: 'idle', lastOutputTimestamp: 1 }],
        ['t2', { id: 't2', session: session('t2'), status: 'idle', lastOutputTimestamp: 1 }]
      ]) as never,
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      cardSplits: {},
      maximizedPaneId: null,
      promotedPanes: new Set(),
      // Both placement and layout are global, so a test that promotes a pane or
      // switches to tabs would otherwise hand its state to the next one.
      config: { defaults: { layoutMode: 'grid' } } as never,
      focusedTerminalId: null,
      previewTerminalId: null
    })
  })
})

describe('splitPaneWeights', () => {
  it('gives every pane an even share when nothing is stored', () => {
    expect(splitPaneWeights(undefined, 2)).toEqual([0.5, 0.5])
  })

  it('absorbs a stored list of the wrong length', () => {
    // Opening a third pane must not need a migration of what two panes saved.
    const w = splitPaneWeights([0.7, 0.3], 3)
    expect(w).toHaveLength(3)
    expect(w.reduce((a, b) => a + b, 0)).toBeCloseTo(1)
  })

  it('replaces a corrupted entry rather than propagating it into a flex basis', () => {
    const w = splitPaneWeights([NaN, 0.5], 2)
    expect(w.every((n) => Number.isFinite(n) && n > 0)).toBe(true)
  })
})

describe('resizePaneWeights', () => {
  it('moves only the two panes the divider touches', () => {
    // Uneven on purpose: the third pane keeps its own 0.5, not an even share.
    // Dragging one divider must not redistribute the whole stack.
    const next = resizePaneWeights([0.2, 0.3, 0.5], 0, 0.3)
    expect(next[2]).toBeCloseTo(0.5)
    expect(next[0]).toBeCloseTo(0.3)
    expect(next[1]).toBeCloseTo(0.2)
  })

  it('refuses to collapse either side of the pair', () => {
    const next = resizePaneWeights([0.5, 0.5], 0, 0)
    expect(next[0]).toBeGreaterThan(0)
    expect(next[1]).toBeGreaterThan(0)
  })

  it('ignores a divider index with no pane below it', () => {
    const w = [0.5, 0.5]
    expect(resizePaneWeights(w, 1, 0.9)).toBe(w)
  })
})

describe('PaneColumn', () => {
  it('renders nothing until the session opens a pane', () => {
    const { container } = render(<PaneColumn sessionId="t1" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('stacks a session’s panes with a divider between them', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openBrowserPane('t1')
    })
    render(<PaneColumn sessionId="t1" />)

    expect(screen.getByTestId('files-t1')).toBeInTheDocument()
    expect(screen.getByTestId('browser-t1')).toBeInTheDocument()
    // One divider, between the two — not one above the first.
    expect(screen.getAllByRole('separator')).toHaveLength(1)
  })

  it('lets a promoted pane out of the column entirely', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openBrowserPane('t1')
      useAppStore.getState().promotePane('files:t1')
    })
    render(<PaneColumn sessionId="t1" />)

    // Gone, not hidden: the grid draws it as a cell of its own, and a copy left
    // here would mount the pane twice — two <webview>s on one url, two file
    // trees fighting over the same selection.
    expect(screen.queryByTestId('files-t1')).not.toBeInTheDocument()
    expect(screen.getByTestId('browser-t1')).toBeInTheDocument()
    // And the column closes up: one pane left means no divider.
    expect(screen.queryByRole('separator')).not.toBeInTheDocument()
  })

  it('keeps a promoted pane in the column where no grid is drawing it', () => {
    // Promotion is a grid placement. The tab strip shows one session and has no
    // cell to put a promoted pane in, so honouring the flag there would drop the
    // pane out of the UI with the control to bring it back going with it.
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().promotePane('files:t1')
      useAppStore.setState({
        config: { defaults: { layoutMode: 'tabs' } }
      } as never)
    })
    render(<PaneColumn sessionId="t1" />)

    expect(screen.getByTestId('files-t1')).toBeInTheDocument()
  })

  it('gives a maximized pane the whole column and hides its siblings', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openBrowserPane('t1')
      useAppStore.getState().setMaximizedPane('browser:t1')
    })
    render(<PaneColumn sessionId="t1" />)

    // Out of sight, but still mounted — see below.
    expect(screen.getByTestId('files-t1').closest('[aria-hidden]')).not.toBeNull()
    expect(screen.getByTestId('browser-t1')).toBeInTheDocument()
    expect(screen.getByTestId('browser-t1').closest('[aria-hidden]')).toBeNull()
    // Nothing to drag while one pane owns the column.
    expect(screen.queryByRole('separator')).not.toBeInTheDocument()
  })

  it('keeps a hidden pane mounted rather than tearing it down', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openBrowserPane('t1')
      useAppStore.getState().setMaximizedPane('files:t1')
    })
    render(<PaneColumn sessionId="t1" />)

    // Unmounting to hide destroys the browser's <webview> guest: the person
    // loses the page and scroll position, and the session's agent loses its
    // CDP attachment — after which it is told "no pane open" while the store
    // still holds one, so it cannot even reopen its way out.
    const browser = screen.getByTestId('browser-t1')
    expect(browser).toBeInTheDocument()
    // Inert while hidden, so it cannot swallow clicks aimed at the maximized
    // pane sitting on top of it.
    expect(browser.closest('.pointer-events-none')).not.toBeNull()
  })

  it("ignores another session's maximized pane", () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openBrowserPane('t1')
      useAppStore.getState().setMaximizedPane('browser:t2')
    })
    render(<PaneColumn sessionId="t1" />)

    // Maximize is session-scoped: t2 taking over its own card must not blank t1.
    expect(screen.getByTestId('files-t1')).toBeInTheDocument()
    expect(screen.getByTestId('browser-t1')).toBeInTheDocument()
  })

  it('persists a pane divider drag on pointerup only', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openBrowserPane('t1')
    })
    const { container } = render(<PaneColumn sessionId="t1" />)
    const column = container.firstElementChild as HTMLElement
    vi.spyOn(column, 'getBoundingClientRect').mockReturnValue({
      top: 0,
      left: 0,
      right: 100,
      bottom: 1000,
      width: 100,
      height: 1000,
      x: 0,
      y: 0,
      toJSON: () => ({})
    })

    fireEvent.pointerDown(screen.getByRole('separator'), { clientY: 500 })
    fireEvent(document, new PointerEvent('pointermove', { clientY: 300 }))
    expect(useAppStore.getState().cardSplits.t1).toBeUndefined()

    fireEvent(document, new PointerEvent('pointerup'))
    const panes = useAppStore.getState().cardSplits.t1.panes
    expect(panes[0]).toBeLessThan(panes[1])
  })

  it('keeps a squeezed pane below the split floor through a store round trip', () => {
    // Pane weights are column-relative shares, not two-sided ratios: with three
    // panes open, ~0.1 is legitimate. Clamping it to MIN_SPLIT_RATIO would make
    // the stack jump on pointerup and again on reload.
    const squeezed = [0.1, 0.4, 0.5]
    act(() => {
      useAppStore.getState().setCardSplit('t1', { terminal: 0.5, panes: squeezed })
    })
    expect(useAppStore.getState().cardSplits.t1.panes[0]).toBeCloseTo(0.1)

    const reloaded = JSON.parse(localStorage.getItem('vorn:cardSplits')!)
    expect(reloaded.t1.panes[0]).toBeCloseTo(0.1)
  })
})
