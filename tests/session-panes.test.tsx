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
