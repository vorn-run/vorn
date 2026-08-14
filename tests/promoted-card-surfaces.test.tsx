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
  value: {
    notifyWidgetStatus: vi.fn(),
    listDir: vi.fn().mockResolvedValue([]),
    readFileContent: vi.fn().mockResolvedValue('hello'),
    attachBrowser: vi.fn(),
    detachBrowser: vi.fn(),
    cancelBrowserPick: vi.fn(),
    startBrowserPick: vi.fn().mockResolvedValue(null),
    writeTerminal: vi.fn(),
    reorderSessions: vi.fn()
  },
  writable: true,
  configurable: true
})

const { useAppStore } = await import('../src/renderer/stores')
const { PromotedPaneCard } = await import('../src/renderer/components/PromotedPaneCard')
const { MobileSinglePane } = await import('../src/renderer/components/MobileSinglePane')

const session = (id: string) =>
  ({
    id,
    projectName: 'vorn',
    projectPath: '/repo',
    agentType: 'claude',
    createdAt: 0,
    displayName: id,
    branch: 'main'
  }) as never

beforeEach(() => {
  localStorage.clear()
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        ['t1', { id: 't1', session: session('t1'), status: 'idle', lastOutputTimestamp: 1 }]
      ]) as never,
      terminalOrder: ['t1'],
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      minimizedTerminals: new Set(),
      selectedTerminalId: null,
      focusedTerminalId: null,
      previewTerminalId: null,
      maximizedPaneId: null
    })
  })
})

/**
 * The dispatcher every surface uses to draw a card. Which collection holds the
 * key is what says whether it is a file or a page — there is no third list.
 */
describe('PromotedPaneCard', () => {
  it('draws a popped-out file as an editor', async () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/server.ts')
    })
    render(<PromotedPaneCard cardId={cardId} />)

    expect(await screen.findByText('server.ts')).toBeInTheDocument()
  })

  it('draws a popped-out tab as a browser', () => {
    let cardId: string | null = null
    act(() => {
      useAppStore.getState().openBrowserPane('t1', 'vorn.dev')
      cardId = useAppStore.getState().promoteBrowserTab('t1', 0)
    })
    render(<PromotedPaneCard cardId={cardId as unknown as string} />)

    expect(screen.getByLabelText('Address')).toHaveValue('https://vorn.dev/')
  })

  it('draws nothing for an id whose pane has gone', () => {
    const { container } = render(<PromotedPaneCard cardId="card:t1:99" />)
    expect(container).toBeEmptyDOMElement()
  })
})

/**
 * A card's own chrome: whose it is, and the three things you can do with it.
 * Distinct from a pane's, because the useful moves differ.
 */
describe('a promoted card wears its own controls', () => {
  it('says whose it is, and offers minimize, return and close', async () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
    })
    render(<PromotedPaneCard cardId={cardId} />)
    await screen.findByText('a.ts')

    // The owner label — the thing that stops the right file from the wrong
    // worktree looking exactly like the right one.
    expect(screen.getByText('vorn')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /Minimize a\.ts/ }))
    expect(useAppStore.getState().minimizedTerminals.has(cardId)).toBe(true)
  })

  it('puts the card back into its session', async () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
    })
    render(<PromotedPaneCard cardId={cardId} />)
    await screen.findByText('a.ts')

    fireEvent.click(screen.getByRole('button', { name: /back in its session card/ }))
    expect(useAppStore.getState().editorPanes.has(cardId)).toBe(false)
    expect(useAppStore.getState().editorPanes.get('t1')?.filePath).toBe('/repo/a.ts')
  })

  it('selects itself when pressed, so keyboard jumps have something to land on', async () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
    })
    const { container } = render(<PromotedPaneCard cardId={cardId} />)
    await screen.findByText('a.ts')

    fireEvent.pointerDown(container.firstElementChild as HTMLElement)
    expect(useAppStore.getState().selectedTerminalId).toBe(cardId)
  })
})

/**
 * Mobile lists the same ids the grid does, so a card arrives here too — and
 * drew nothing, leaving it a slot in the visible set with no row to tap.
 */
describe('MobileSinglePane with a card', () => {
  it('gives a popped-out file a row of its own', () => {
    act(() => {
      useAppStore.getState().promoteFile('t1', '/repo/notes.md')
    })
    render(<MobileSinglePane />)

    expect(screen.getByText('notes.md')).toBeInTheDocument()
    // Named by its owner's project, since the row is otherwise just a filename.
    expect(screen.getAllByText('vorn').length).toBeGreaterThan(0)
  })

  it('focuses the card when its row is tapped', () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/repo/notes.md')
    })
    render(<MobileSinglePane />)

    fireEvent.click(screen.getByText('notes.md'))
    expect(useAppStore.getState().focusedTerminalId).toBe(cardId)
  })

  it('gives a popped-out page a row named by its host', () => {
    act(() => {
      useAppStore.getState().openBrowserPane('t1', 'vorn.dev')
      useAppStore.getState().promoteBrowserTab('t1', 0)
    })
    render(<MobileSinglePane />)

    expect(screen.getByText('vorn.dev')).toBeInTheDocument()
  })
})
