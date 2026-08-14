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
      promotedPanes: new Set(),
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      visibleTerminalIds: []
    })
  })
}

/**
 * A promoted pane is a layout unit — a grid cell of its own. This is where it
 * becomes one, and where it gets its place in the order.
 */
describe('useVisibleTerminals with promoted panes', () => {
  beforeEach(() => {
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      notifyWidgetStatus: vi.fn(),
      reorderSessions: vi.fn()
    }
    seed(['t1', 't2'])
  })

  it('places a promoted pane directly after the session it came from', () => {
    const { result, rerender } = renderHook(() => useVisibleTerminals())
    expect(result.current.orderedIds).toEqual(['t1', 't2'])

    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().promotePane('files:t1')
    })
    rerender()

    // Beside its owner, not appended at the end: the two are read together, and
    // a grid that reflows would otherwise separate them by a whole row.
    expect(result.current.orderedIds).toEqual(['t1', 'files:t1', 't2'])
  })

  it('sends a minimized promoted pane to the dock, not the grid', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().promotePane('files:t1')
      useAppStore.getState().toggleMinimized('files:t1')
    })
    const { result } = renderHook(() => useVisibleTerminals())

    expect(result.current.orderedIds).toEqual(['t1', 't2'])
    expect(result.current.minimizedIds).toEqual(['files:t1'])
  })

  it('leaves a pane that is merely open inside its card out of the layout', () => {
    // Opening a pane must not add a cell. Only promotion does — otherwise every
    // file tree anyone opened would rearrange the whole grid.
    act(() => useAppStore.getState().openFilesPane('t1'))
    const { result } = renderHook(() => useVisibleTerminals())

    expect(result.current.orderedIds).toEqual(['t1', 't2'])
  })
})
