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

  it('keeps the layout list sessions-only as panes open and close', () => {
    const { result } = renderHook(() => useVisibleTerminals())
    expect(result.current.orderedIds).toEqual(['t1'])

    // A session's panes render inside its own card, so they are not layout
    // units in the grid — opening one must not add a cell.
    act(() => useAppStore.getState().openFilesPane('t1'))
    act(() => useAppStore.getState().openEditorPane('t1', '/p/a.ts'))
    act(() => useAppStore.getState().openBrowserPane('t1'))
    expect(result.current.orderedIds).toEqual(['t1'])

    act(() => useAppStore.getState().closeFilesPane('t1'))
    expect(result.current.orderedIds).toEqual(['t1'])
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
    expect(result.current.orderedIds).toEqual(['t1'])

    act(() => useAppStore.getState().removeTerminal('t1'))
    expect(result.current.orderedIds).toEqual([])
    expect(useAppStore.getState().filesPanes.has('t1')).toBe(false)
    expect(useAppStore.getState().editorPanes.has('t1')).toBe(false)
  })
})
