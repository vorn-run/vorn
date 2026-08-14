// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'
import { useVisibleTerminals } from '../src/renderer/hooks/useVisibleTerminals'

const session = (id: string) =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    createdAt: 0,
    displayName: id
  }) as never

function seed(ids: string[]): void {
  const terminals = new Map()
  for (const id of ids) {
    terminals.set(id, { id, session: session(id), status: 'idle', lastOutputTimestamp: 1 })
  }
  act(() => {
    useAppStore.setState({
      terminals: terminals as never,
      terminalOrder: ids,
      sortMode: 'manual',
      statusFilter: 'all',
      activeProject: null,
      activeWorktreePath: null,
      minimizedTerminals: new Set(),
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      visibleTerminalIds: []
    })
  })
}

/**
 * A popped-out file or tab is a layout unit — a grid cell of its own. This is
 * where it becomes one, and where it gets its place in the order.
 */
describe('useVisibleTerminals with popped-out cards', () => {
  beforeEach(() => {
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      notifyWidgetStatus: vi.fn(),
      reorderSessions: vi.fn()
    }
    seed(['t1', 't2'])
  })

  it('places a card directly after the session it came from', () => {
    const { result, rerender } = renderHook(() => useVisibleTerminals())
    expect(result.current.orderedIds).toEqual(['t1', 't2'])

    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/p/a.ts')
    })
    rerender()

    // Beside its owner, not appended at the end: the two are read together, and
    // a grid that reflows would otherwise separate them by a whole row.
    expect(result.current.orderedIds).toEqual(['t1', cardId, 't2'])
  })

  it('sends a minimized card to the dock, not the grid', () => {
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/p/a.ts')
      useAppStore.getState().toggleMinimized(cardId)
    })
    const { result } = renderHook(() => useVisibleTerminals())

    expect(result.current.orderedIds).toEqual(['t1', 't2'])
    expect(result.current.minimizedIds).toEqual([cardId])
  })

  it("leaves a session's own panes out of the layout", () => {
    // Opening a file in the session's editor must not add a cell. Only popping
    // one out does — otherwise every file anyone opened would rearrange the grid.
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openEditorPane('t1', '/p/a.ts')
    })
    const { result } = renderHook(() => useVisibleTerminals())

    expect(result.current.orderedIds).toEqual(['t1', 't2'])
  })

  it("keeps each session's cards with that session", () => {
    let mine = ''
    let theirs = ''
    act(() => {
      mine = useAppStore.getState().promoteFile('t1', '/p/a.ts')
      theirs = useAppStore.getState().promoteFile('t2', '/p/b.ts')
    })
    const { result } = renderHook(() => useVisibleTerminals())

    expect(result.current.orderedIds).toEqual(['t1', mine, 't2', theirs])
  })

  it('does not disturb the layout when a pane changes but the cells do not', () => {
    // Both pane Maps are replaced on any pane write — switching a browser tab,
    // typing in the address bar. Depending on them directly re-ran the whole
    // layout memo, re-sorting every session twice and triggering a reconcile
    // pass that copies six collections, for a list that had not changed.
    const { result, rerender } = renderHook(() => useVisibleTerminals())
    const before = result.current.orderedIds

    act(() => {
      useAppStore.getState().openBrowserPane('t1', 'example.com')
      useAppStore.getState().addBrowserTab('t1', 'second.example')
      useAppStore.getState().setActiveBrowserTab('t1', 0)
    })
    rerender()

    expect(result.current.orderedIds).toBe(before)
  })

  it('makes a card reachable by keyboard nav', () => {
    // Cmd+] and Cmd+1-9 both index straight into focusableIds, so whether a
    // card can be focused at all is decided here — the shortcut handlers never
    // learn what an id is, and would skip cards silently if this list did.
    let cardId = ''
    act(() => {
      cardId = useAppStore.getState().promoteFile('t1', '/p/a.ts')
    })
    renderHook(() => useVisibleTerminals())

    expect(useAppStore.getState().focusableTerminalIds).toEqual(['t1', cardId, 't2'])
  })

  it("orders a session's cards straight after it, however many", () => {
    // What the tab strip and the grid both consume. Cards cannot be sorted
    // among the sessions — compareTerminalIds reads the terminals map, which
    // holds no entry for a card — so placement beside the owner is the ordering.
    let a = ''
    let b = ''
    act(() => {
      a = useAppStore.getState().promoteFile('t1', '/p/a.ts')
      b = useAppStore.getState().promoteFile('t1', '/p/b.ts')
    })
    const { result } = renderHook(() => useVisibleTerminals())

    expect(result.current.orderedIds).toEqual(['t1', a, b, 't2'])
  })
})
