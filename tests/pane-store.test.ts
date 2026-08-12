// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'

const session = (id: string) =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    createdAt: 0,
    displayName: id
  }) as never

function seed(ids: string[] = ['t1']): void {
  const terminals = new Map()
  for (const id of ids) {
    terminals.set(id, { id, session: session(id), status: 'idle', lastOutputTimestamp: 1 })
  }
  act(() => {
    useAppStore.setState({
      terminals,
      filesPanes: new Set(),
      editorPanes: new Map(),
      minimizedTerminals: new Set(),
      maximizedPaneId: null,
      terminalOrder: ids,
      visibleTerminalIds: []
    })
  })
}

describe('pane store actions', () => {
  beforeEach(() => {
    localStorage.clear()
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api?: Record<string, unknown> }).api,
      notifyWidgetStatus: vi.fn(),
      reorderSessions: vi.fn()
    }
    seed(['t1', 't2'])
  })

  it('opens, toggles and closes a session tree', () => {
    const s = () => useAppStore.getState()

    act(() => s().openFilesPane('t1'))
    expect(s().filesPanes.has('t1')).toBe(true)

    // Opening twice is a no-op rather than a duplicate.
    act(() => s().openFilesPane('t1'))
    expect(s().filesPanes.size).toBe(1)

    act(() => s().toggleFilesPane('t1'))
    expect(s().filesPanes.has('t1')).toBe(false)

    act(() => s().toggleFilesPane('t1'))
    expect(s().filesPanes.has('t1')).toBe(true)
  })

  it("keeps each session's panes separate", () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openFilesPane('t1')
      s().openEditorPane('t1', '/p/a.ts')
      s().openEditorPane('t2', '/p/b.ts')
    })

    // Two sessions on the same worktree hold independent state — this is the
    // whole point of session ownership.
    expect(s().editorPanes.get('t1')?.filePath).toBe('/p/a.ts')
    expect(s().editorPanes.get('t2')?.filePath).toBe('/p/b.ts')
    expect(s().filesPanes.has('t2')).toBe(false)
  })

  it('swaps the file inside one editor rather than stacking editors', () => {
    const s = () => useAppStore.getState()
    act(() => s().openEditorPane('t1', '/p/a.ts'))
    act(() => s().openEditorPane('t1', '/p/b.ts'))

    expect(s().editorPanes.size).toBe(1)
    expect(s().editorPanes.get('t1')?.filePath).toBe('/p/b.ts')
  })

  it('re-opening a file un-minimizes the editor instead of updating it unseen', () => {
    const s = () => useAppStore.getState()
    act(() => s().openEditorPane('t1', '/p/a.ts'))
    act(() => s().toggleMinimized('editor:t1'))
    expect(s().minimizedTerminals.has('editor:t1')).toBe(true)

    act(() => s().openEditorPane('t1', '/p/b.ts'))
    expect(s().minimizedTerminals.has('editor:t1')).toBe(false)
  })

  it('closing a pane clears its minimized and maximized state', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openFilesPane('t1')
      s().setMaximizedPane('files:t1')
      s().toggleMinimized('files:t1')
    })
    // Minimizing a maximized pane already drops the maximize.
    expect(s().maximizedPaneId).toBeNull()

    act(() => s().closeFilesPane('t1'))
    expect(s().minimizedTerminals.has('files:t1')).toBe(false)
  })

  it('minimizing a maximized pane clears the maximize', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openFilesPane('t1')
      s().setMaximizedPane('files:t1')
    })
    expect(s().maximizedPaneId).toBe('files:t1')

    act(() => s().toggleMinimized('files:t1'))
    expect(s().maximizedPaneId).toBeNull()

    // Restoring does not silently re-maximize it.
    act(() => s().toggleMinimized('files:t1'))
    expect(s().maximizedPaneId).toBeNull()
  })

  it('holds at most one maximized pane app-wide', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openFilesPane('t1')
      s().openFilesPane('t2')
      s().setMaximizedPane('files:t1')
      s().setMaximizedPane('files:t2')
    })
    expect(s().maximizedPaneId).toBe('files:t2')

    act(() => s().setMaximizedPane(null))
    expect(s().maximizedPaneId).toBeNull()
  })

  it('closing an editor clears a maximize that pointed at it', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openEditorPane('t1', '/p/a.ts')
      s().setMaximizedPane('editor:t1')
    })
    act(() => s().closeEditorPane('t1'))

    // A stale id here would leave the grid maximizing a pane that is gone.
    expect(s().maximizedPaneId).toBeNull()
    expect(s().editorPanes.has('t1')).toBe(false)
  })

  it('persists open panes so a reload restores the workspace', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openFilesPane('t1')
      s().openEditorPane('t1', '/p/a.ts')
    })

    const raw = localStorage.getItem('vorn:panes')
    expect(raw).toBeTruthy()
    expect(JSON.parse(raw as string)).toEqual({
      files: ['t1'],
      editors: { t1: '/p/a.ts' }
    })

    act(() => s().closeFilesPane('t1'))
    expect(JSON.parse(localStorage.getItem('vorn:panes') as string).files).toEqual([])
  })

  it('drops persisted panes for sessions that never came back', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openFilesPane('t1')
      s().openEditorPane('t2', '/p/b.ts')
    })

    // t2 does not return after a restart; its pane entry would otherwise sit in
    // localStorage forever and could attach to a recycled id.
    const terminals = new Map(s().terminals)
    terminals.delete('t2')
    act(() => useAppStore.setState({ terminals }))
    act(() => s().setVisibleTerminalIds(['t1']))

    expect(s().filesPanes.has('t1')).toBe(true)
    expect(s().editorPanes.has('t2')).toBe(false)
    expect(JSON.parse(localStorage.getItem('vorn:panes') as string).editors).toEqual({})
  })

  it('does not prune while the session list is still empty', () => {
    const s = () => useAppStore.getState()
    act(() => s().openFilesPane('t1'))

    // During startup terminals is empty; pruning then would wipe every restored
    // pane before the sessions arrive.
    act(() => useAppStore.setState({ terminals: new Map() }))
    act(() => s().setVisibleTerminalIds([]))

    expect(s().filesPanes.has('t1')).toBe(true)
  })
})
