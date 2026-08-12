// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'
import { useVisibleTerminals } from '../src/renderer/hooks/useVisibleTerminals'
import {
  sessionPositionForGridIndex,
  visiblePanesWhileMaximized,
  maximizedCellSpan,
  maximizedBoundingRect,
  paneLayoutKey,
  boundingBoxPaneFor
} from '../src/renderer/lib/pane-order'

const session = (id: string) =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    createdAt: 0,
    displayName: id
  }) as never

/**
 * A session's file panes are derived inside `useVisibleTerminals`'s memo. The
 * memo must depend on `filesPanes`/`editorPanes` or opening a pane updates the
 * store while the grid keeps rendering a stale id list — the pane silently never
 * appears. These assertions fail if those deps are dropped again.
 */
describe('useVisibleTerminals — session-owned panes', () => {
  beforeEach(() => {
    // removeTerminal pings the widget; stub it so the store call works headless.
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api?: Record<string, unknown> }).api,
      notifyWidgetStatus: vi.fn()
    }
    const terminals = new Map()
    terminals.set('t1', {
      id: 't1',
      session: session('t1'),
      status: 'idle',
      lastOutputTimestamp: 1
    })
    act(() => {
      useAppStore.setState({
        terminals,
        activeProject: null,
        activeWorktreePath: null,
        filesPanes: new Set(),
        editorPanes: new Map(),
        minimizedTerminals: new Set(),
        terminalOrder: ['t1'],
        statusFilter: 'all'
      })
    })
  })

  it('adds and removes child pane ids as panes open and close', () => {
    const { result } = renderHook(() => useVisibleTerminals())
    expect(result.current.orderedIds).toEqual(['t1'])

    act(() => useAppStore.getState().openFilesPane('t1'))
    expect(result.current.orderedIds).toEqual(['t1', 'files:t1'])

    act(() => useAppStore.getState().openEditorPane('t1', '/p/a.ts'))
    expect(result.current.orderedIds).toEqual(['t1', 'files:t1', 'editor:t1'])

    // Panes are independent: closing the tree leaves the open file alone.
    act(() => useAppStore.getState().closeFilesPane('t1'))
    expect(result.current.orderedIds).toEqual(['t1', 'editor:t1'])

    act(() => useAppStore.getState().closeEditorPane('t1'))
    expect(result.current.orderedIds).toEqual(['t1'])
  })

  it('keeps a session and its panes adjacent, so maximize can span them', () => {
    const terminals = useAppStore.getState().terminals
    const next = new Map(terminals)
    next.set('t2', {
      id: 't2',
      session: session('t2'),
      status: 'idle',
      lastOutputTimestamp: 1
    })
    act(() => useAppStore.setState({ terminals: next, terminalOrder: ['t1', 't2'] }))

    const { result } = renderHook(() => useVisibleTerminals())
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openFilesPane('t2')
    })

    expect(result.current.orderedIds).toEqual(['t1', 'files:t1', 't2', 'files:t2'])
  })

  it('routes a minimized child pane to the dock, not the grid', () => {
    const { result } = renderHook(() => useVisibleTerminals())
    act(() => useAppStore.getState().openFilesPane('t1'))
    act(() => useAppStore.getState().toggleMinimized('files:t1'))

    expect(result.current.orderedIds).toEqual(['t1'])
    expect(result.current.minimizedIds).toEqual(['files:t1'])
  })

  it('keeps visibleTerminalIds sessions-only, so Cmd+N navigation stays valid', () => {
    renderHook(() => useVisibleTerminals())
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openEditorPane('t1', '/p/a.ts')
    })

    // Cmd+], Cmd+[ and Cmd+1-9 index into this list and hand the result to
    // setActiveTabId / setSelectedTerminal — a pane id there lands on a tab no
    // terminals.get() can resolve.
    expect(useAppStore.getState().visibleTerminalIds).toEqual(['t1'])
  })

  it('drops a session-owned pane when the session closes', () => {
    const { result } = renderHook(() => useVisibleTerminals())
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openEditorPane('t1', '/p/a.ts')
    })
    expect(result.current.orderedIds).toHaveLength(3)

    act(() => useAppStore.getState().removeTerminal('t1'))
    expect(result.current.orderedIds).toEqual([])
    expect(useAppStore.getState().filesPanes.has('t1')).toBe(false)
    expect(useAppStore.getState().editorPanes.has('t1')).toBe(false)
  })
})

describe('sessionPositionForGridIndex', () => {
  // The grid interleaves each session's panes; terminalOrder holds sessions
  // only. Reordering with a raw grid index splices the wrong element — or an
  // undefined one — and corrupts the persisted session order.
  const grid = ['t1', 'files:t1', 'editor:t1', 't2', 't3', 'files:t3']

  it('maps a grid drop position to a session position', () => {
    expect(sessionPositionForGridIndex(grid, 0)).toBe(0) // before t1
    expect(sessionPositionForGridIndex(grid, 3)).toBe(1) // before t2
    expect(sessionPositionForGridIndex(grid, 4)).toBe(2) // before t3
    expect(sessionPositionForGridIndex(grid, 6)).toBe(3) // past the end
  })

  it('never exceeds the session count, whatever the grid index', () => {
    const sessions = grid.filter((id) => !id.includes(':')).length
    for (let i = 0; i <= grid.length + 2; i++) {
      expect(sessionPositionForGridIndex(grid, i)).toBeLessThanOrEqual(sessions)
    }
  })

  it('is an identity when no panes are open', () => {
    const plain = ['t1', 't2', 't3']
    expect(sessionPositionForGridIndex(plain, 0)).toBe(0)
    expect(sessionPositionForGridIndex(plain, 2)).toBe(2)
  })
})

describe('session-scoped maximize', () => {
  // t1 owns a tree and a file; t2 is a plain session. Maximizing must collapse
  // only t1's group — t2 keeps rendering, which is the entire point.
  const grid = ['t1', 'files:t1', 'editor:t1', 't2']

  it("hides the maximized pane's siblings and nothing else", () => {
    expect(visiblePanesWhileMaximized(grid, 'editor:t1')).toEqual(['editor:t1', 't2'])
    expect(visiblePanesWhileMaximized(grid, 't1')).toEqual(['t1', 't2'])
  })

  it('is a no-op with nothing maximized, or a pane not on screen', () => {
    expect(visiblePanesWhileMaximized(grid, null)).toEqual(grid)
    expect(visiblePanesWhileMaximized(grid, 'files:ghost')).toEqual(grid)
  })

  it('spans exactly the cells its owner gave up', () => {
    expect(maximizedCellSpan(grid, 'editor:t1')).toBe(3)
    expect(maximizedCellSpan(grid, 't2')).toBe(1)
    expect(maximizedCellSpan(grid, null)).toBe(1)
    expect(maximizedCellSpan(grid, 'files:ghost')).toBe(1)
  })

  it("unions the owner group's rects in flexible mode", () => {
    const rects: Record<string, { x: number; y: number; w: number; h: number }> = {
      t1: { x: 0, y: 0, w: 4, h: 3 },
      'files:t1': { x: 4, y: 0, w: 2, h: 3 },
      'editor:t1': { x: 0, y: 3, w: 6, h: 2 },
      t2: { x: 8, y: 0, w: 4, h: 3 }
    }
    expect(maximizedBoundingRect(grid, 'editor:t1', (id) => rects[id])).toEqual({
      x: 0,
      y: 0,
      w: 6,
      h: 5
    })
    // t2's rect sits outside the group and must not widen the box.
    expect(maximizedBoundingRect(grid, 't2', (id) => rects[id])).toBeNull()
  })

  it('returns null when there is nothing to span', () => {
    // A lone session has no siblings to absorb, and a group with no saved rects
    // has no box — in both cases the pane keeps its own rect.
    expect(maximizedBoundingRect(['t2'], 't2', () => undefined)).toBeNull()
    expect(maximizedBoundingRect(grid, 'editor:t1', () => undefined)).toBeNull()
  })
})

describe('flexible-layout keys and the persist guard', () => {
  const grid = ['t1', 'files:t1', 'editor:t1', 't2']

  it('namespaces child pane keys so siblings keep independent rects', () => {
    // All three share one owner stable key; without the kind prefix they would
    // overwrite each other's saved position.
    expect(paneLayoutKey('t1', 'stable-1')).toBe('stable-1')
    expect(paneLayoutKey('files:t1', 'stable-1')).toBe('files:stable-1')
    expect(paneLayoutKey('editor:t1', 'stable-1')).toBe('editor:stable-1')
  })

  it('flags a pane only while it actually renders as a bounding box', () => {
    expect(boundingBoxPaneFor(['editor:t1', 't2'], grid, 'editor:t1')).toBe('editor:t1')
    // Nothing maximized.
    expect(boundingBoxPaneFor(grid, grid, null)).toBeNull()
    // A lone session spans nothing, so its own rect is still the real one.
    expect(boundingBoxPaneFor(grid, grid, 't2')).toBeNull()
  })

  it('never flags a stale id, so a pane cannot be frozen out of persistence', () => {
    // If a stale `maximizedPaneId` suppressed persistence forever, that pane's
    // position would silently stop being saved.
    expect(boundingBoxPaneFor(grid, grid, 'files:ghost')).toBeNull()
    expect(boundingBoxPaneFor(['t2'], grid, 'editor:t1')).toBeNull()
  })
})
