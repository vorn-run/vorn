// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, act, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.hoisted(() => {
  Object.defineProperty(window, 'api', {
    value: {
      isWorktreeDirty: () => Promise.resolve(false),
      getGitDiffStat: () => Promise.resolve(null),
      getGitBranch: () => Promise.resolve(null),
      notifyWidgetStatus: () => {},
      detectIDEs: () => Promise.resolve([]),
      openInIDE: () => {},
      detectInstalledAgents: () => Promise.resolve([]),
      listDir: () => Promise.resolve([])
    },
    writable: true
  })
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
})

vi.mock('../src/renderer/components/TerminalSlot', () => ({
  TerminalSlot: ({ terminalId }: { terminalId: string }) => (
    <div data-testid={`slot-${terminalId}`} />
  )
}))
vi.mock('../src/renderer/hooks/useTerminalScrollButton', () => ({
  useTerminalScrollButton: () => ({ showScrollBtn: false, handleScrollToBottom: () => {} })
}))
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
vi.mock('../src/renderer/components/DeviceCard', () => ({
  DeviceCard: ({ sessionId }: { sessionId: string }) => <div data-testid={`device-${sessionId}`} />
}))

import { useAppStore } from '../src/renderer/stores'
import { AgentCard } from '../src/renderer/components/AgentCard'

function seed(ids: string[]): void {
  const terminals = new Map()
  for (const id of ids) {
    terminals.set(id, {
      id,
      session: {
        id,
        agentType: 'claude',
        projectName: 'Vorn',
        projectPath: '/tmp/vorn',
        displayName: id,
        createdAt: 0
      },
      status: 'idle',
      lastOutputTimestamp: 1
    })
  }
  act(() => {
    useAppStore.setState({
      config: { defaults: { domBlockRendering: false } } as never,
      terminals,
      focusedTerminalId: null,
      selectedTerminalId: null,
      renamingTerminalId: null,
      minimizedTerminals: new Set<string>(),
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      devicePanes: new Map(),
      cardSplits: {},
      maximizedPaneId: null
    })
  })
}

beforeEach(() => {
  localStorage.clear()
  seed(['t1', 't2'])
})
afterEach(() => cleanup())

describe('AgentCard pane column', () => {
  it('renders a session’s panes inside its own card, not beside it', () => {
    act(() => useAppStore.getState().openFilesPane('t1'))
    const { container } = render(<AgentCard terminalId="t1" />)

    const card = container.firstElementChild as HTMLElement
    // The whole point of the change: the tree is a descendant of the card that
    // owns it, so the card's space is divided among its own contents.
    expect(card).toContainElement(screen.getByTestId('files-t1'))
  })

  it('adds no divider or column to a session with no panes open', () => {
    render(<AgentCard terminalId="t1" />)
    expect(screen.queryByRole('separator')).not.toBeInTheDocument()
    expect(screen.getByTestId('slot-t1')).toBeInTheDocument()
  })

  it('mounts the column for a device pane, as for every other kind', () => {
    // The live bug: `hasPanes` listed files, editor and browser but not device,
    // so `device:openPane` put a pane in the store, reported success, and the
    // column that would have rendered it was never mounted. Nothing anywhere
    // said so — the pane simply never appeared.
    act(() => useAppStore.getState().openDevicePane('t1', { udid: 'udid-1', name: 'iPhone 17' }))
    render(<AgentCard terminalId="t1" />)
    expect(screen.getByTestId('device-t1')).toBeInTheDocument()
  })

  it('hides its own terminal when one of its panes is maximized', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().setMaximizedPane('files:t1')
    })
    render(<AgentCard terminalId="t1" />)

    expect(screen.getByTestId('files-t1')).toBeInTheDocument()
    expect(screen.getByTestId('card-terminal-t1')).toHaveClass('hidden')
  })

  it('keeps the card reachable while one of its panes is maximized', () => {
    // The session's chrome rides inside the terminal column so the panes get the
    // card's full height — but that column hides behind a maximized pane, and
    // the chrome must not go with it. Otherwise the card loses its name, its
    // drag handle and the only buttons that close or minimize it, leaving a
    // maximized browser sitting in a card you cannot identify or dismiss.
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().setMaximizedPane('files:t1')
    })
    render(<AgentCard terminalId="t1" />)

    const close = screen.getByLabelText('Close session')
    expect(close).toBeInTheDocument()
    expect(screen.getByTestId('card-terminal-t1')).not.toContainElement(close)
  })

  it('swaps the terminal for a placeholder once the session is expanded', () => {
    // The expanded stage owns the live terminal; a second mount here would
    // fight it for the same session.
    act(() => useAppStore.getState().setFocusedTerminal('t1'))
    render(<AgentCard terminalId="t1" />)

    expect(screen.getByText('Expanded')).toBeInTheDocument()
    expect(screen.queryByTestId('slot-t1')).not.toBeInTheDocument()
  })

  it("leaves another card's terminal alone while one card is maximized", () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().setMaximizedPane('files:t1')
    })
    render(<AgentCard terminalId="t2" />)

    // Maximize covers the owner card only — the other session stays usable,
    // which is what makes it a side-by-side compare rather than a full screen.
    expect(screen.getByTestId('card-terminal-t2')).not.toHaveClass('hidden')
  })

  it('keeps its terminal when one of its own cards is maximized', () => {
    // A popped-out card carries its owner's session id and a kind of its own,
    // so "maximized pane whose kind is not terminal" matched it — and the
    // session that the card came from hid its whole terminal column while the
    // card itself did not maximize. Nothing on screen explained it.
    let cardId = ''
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      cardId = useAppStore.getState().promoteFile('t1', '/repo/a.ts')
      useAppStore.getState().setMaximizedPane(cardId)
    })
    render(<AgentCard terminalId="t1" />)

    expect(screen.getByTestId('card-terminal-t1')).not.toHaveClass('hidden')
  })

  it('sizes the terminal from the stored split', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().setCardSplit('t1', { terminal: 0.7, panes: [] })
    })
    render(<AgentCard terminalId="t1" />)

    // jsdom does not round-trip the `flex` shorthand, so read the attribute.
    expect(screen.getByTestId('card-terminal-t1').getAttribute('style')).toContain('flex: 0.7 1 0')
  })

  it('gives the terminal the whole card when no pane is open', () => {
    // A lone flex child with a grow factor under 1 leaves the rest of the row
    // empty — a dead strip down the right of every card with no panes open.
    act(() => useAppStore.getState().setCardSplit('t1', { terminal: 0.5, panes: [] }))
    render(<AgentCard terminalId="t1" />)

    expect(Number(screen.getByTestId('card-terminal-t1').style.flexGrow)).toBe(1)
  })

  it('resizes the terminal when the card divider is dragged', () => {
    act(() => useAppStore.getState().openFilesPane('t1'))
    const { container } = render(<AgentCard terminalId="t1" />)
    const body = screen.getByTestId('card-terminal-t1').parentElement as HTMLElement
    vi.spyOn(body, 'getBoundingClientRect').mockReturnValue({
      top: 0,
      left: 0,
      right: 1000,
      bottom: 100,
      width: 1000,
      height: 100,
      x: 0,
      y: 0,
      toJSON: () => ({})
    } as DOMRect)

    fireEvent.pointerDown(screen.getByTestId('card-divider-t1'), { clientX: 500 })
    fireEvent(document, new PointerEvent('pointermove', { clientX: 700 }))
    expect(Number(screen.getByTestId('card-terminal-t1').style.flexGrow)).toBeCloseTo(0.7)

    fireEvent(document, new PointerEvent('pointerup'))
    expect(useAppStore.getState().cardSplits.t1.terminal).toBeCloseTo(0.7)
    expect(container).toBeTruthy()
  })

  it('splits the card between terminal and panes with grow factors summing to 1', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().setCardSplit('t1', { terminal: 0.7, panes: [] })
    })
    render(<AgentCard terminalId="t1" />)

    // Two siblings sharing a row only honour a stored ratio if their grow
    // factors sum to 1 — otherwise 0.7 renders as 0.7/(0.7+1) and the divider
    // lags the cursor for the whole drag.
    const terminal = screen.getByTestId('card-terminal-t1')
    const column = terminal.parentElement!.lastElementChild as HTMLElement
    expect(column).toContainElement(screen.getByTestId('files-t1'))
    expect(Number((column as HTMLElement).style.flexGrow)).toBeCloseTo(0.3)
    expect(Number(terminal.style.flexGrow) + Number(column.style.flexGrow)).toBeCloseTo(1)
  })
})
