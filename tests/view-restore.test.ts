// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest'
import { useAppStore } from '../src/renderer/stores'

const KEY = 'vorn:view'

beforeEach(() => {
  localStorage.clear()
  // The store is a module singleton; without this each test inherits the last.
  useAppStore.setState({
    minimizedTerminals: new Set(),
    maximizedPaneId: null,
    activeTabId: null,
    activeProject: null,
    activeWorktreePath: null,
    visibleTerminalIds: [],
    knownSessionIds: new Set<string>(),
    terminals: new Map()
  })
})

describe('the view state a reload used to lose', () => {
  it('keeps a minimised pane minimised', () => {
    useAppStore.getState().toggleMinimized('browser:term-1')
    expect(JSON.parse(localStorage.getItem(KEY)!).minimized).toEqual(['browser:term-1'])
  })

  it('lets go of it again when it is restored', () => {
    useAppStore.getState().toggleMinimized('browser:term-1')
    useAppStore.getState().toggleMinimized('browser:term-1')
    expect(JSON.parse(localStorage.getItem(KEY)!).minimized).toEqual([])
  })

  it('remembers the tab you were on', () => {
    useAppStore.getState().setActiveTabId('term-2')
    expect(JSON.parse(localStorage.getItem(KEY)!).activeTabId).toBe('term-2')
  })

  it('drops a maximised pane that is being minimised', () => {
    useAppStore.setState({ maximizedPaneId: 'term-3' })
    useAppStore.getState().toggleMinimized('term-3')
    const stored = JSON.parse(localStorage.getItem(KEY)!)
    expect(stored.maximizedPaneId).toBeNull()
    expect(stored.minimized).toEqual(['term-3'])
  })

  it('merges rather than replacing, so one setter does not erase another', () => {
    useAppStore.getState().setActiveTabId('term-2')
    useAppStore.getState().toggleMinimized('term-9')
    const stored = JSON.parse(localStorage.getItem(KEY)!)
    expect(stored.activeTabId).toBe('term-2')
    expect(stored.minimized).toEqual(['term-9'])
  })
})

describe('reading a stored view back', () => {
  it('survives a value that is not the shape it expects', async () => {
    for (const raw of ['{', '[]', 'null', '{"minimized":"term-1"}', '{"activeTabId":42}']) {
      localStorage.setItem(KEY, raw)
      const { loadViewForTest } = await import('../src/renderer/stores/ui-slice')
      expect(() => loadViewForTest()).not.toThrow()
      expect(loadViewForTest().minimized).toEqual([])
      expect(loadViewForTest().activeTabId).toBeNull()
    }
  })

  it('drops entries that are not usable ids', async () => {
    localStorage.setItem(KEY, JSON.stringify({ minimized: ['term-1', '', 7, null, 'term-2'] }))
    const { loadViewForTest } = await import('../src/renderer/stores/ui-slice')
    expect(loadViewForTest().minimized).toEqual(['term-1', 'term-2'])
  })
})

describe('the project scope a reload used to revert', () => {
  it('keeps the project you had selected', () => {
    useAppStore.getState().setActiveProject('vorn')
    expect(JSON.parse(localStorage.getItem(KEY)!).activeProject).toBe('vorn')
  })

  it('drops the worktree when the project changes under it', () => {
    useAppStore.getState().setActiveWorktreePath('/w/p6')
    useAppStore.getState().setActiveProject('vorn')
    expect(JSON.parse(localStorage.getItem(KEY)!).activeWorktreePath).toBeNull()
  })

  it('keeps the worktree you had selected', () => {
    useAppStore.getState().setActiveWorktreePath('/w/p6')
    expect(JSON.parse(localStorage.getItem(KEY)!).activeWorktreePath).toBe('/w/p6')
  })
})

/** Seed what the server is known to have, without standing up real terminals. */
function live(...ids: string[]): void {
  useAppStore.setState({
    terminals: new Map(ids.map((id) => [id, { id } as never])),
    knownSessionIds: new Set(ids)
  })
}

describe('pruning view state whose session never came back', () => {
  it('lets go of a card minimised before a session that is gone', () => {
    useAppStore.setState({ minimizedTerminals: new Set(['term-1', 'term-2']) })
    live('term-1')
    useAppStore.getState().setVisibleTerminalIds(['term-1'])
    expect([...useAppStore.getState().minimizedTerminals]).toEqual(['term-1'])
    expect(JSON.parse(localStorage.getItem(KEY)!).minimized).toEqual(['term-1'])
  })

  it('resolves a child pane to the session it hangs off', () => {
    useAppStore.setState({ minimizedTerminals: new Set(['browser:term-1', 'card:term-2:3']) })
    live('term-1')
    useAppStore.getState().setVisibleTerminalIds(['term-1'])
    expect([...useAppStore.getState().minimizedTerminals]).toEqual(['browser:term-1'])
  })

  it('clears a maximised pane and an active tab that are gone', () => {
    useAppStore.setState({ maximizedPaneId: 'device:term-9', activeTabId: 'term-9' })
    live('term-1')
    useAppStore.getState().setVisibleTerminalIds(['term-1'])
    expect(useAppStore.getState().maximizedPaneId).toBeNull()
    expect(useAppStore.getState().activeTabId).toBeNull()
  })

  it('holds everything back until the server has been asked', () => {
    useAppStore.setState({
      knownSessionIds: null,
      minimizedTerminals: new Set(['term-1']),
      activeTabId: 'term-1'
    })
    useAppStore.getState().setVisibleTerminalIds([])
    expect([...useAppStore.getState().minimizedTerminals]).toEqual(['term-1'])
    expect(useAppStore.getState().activeTabId).toBe('term-1')
  })

  it('prunes once the server answers that it has nothing at all', () => {
    useAppStore.setState({
      knownSessionIds: null,
      minimizedTerminals: new Set(['term-1']),
      activeTabId: 'term-1'
    })
    useAppStore.getState().setKnownSessions([])
    useAppStore.getState().setVisibleTerminalIds([])
    expect([...useAppStore.getState().minimizedTerminals]).toEqual([])
    expect(useAppStore.getState().activeTabId).toBeNull()
  })

  it('leaves the project alone -- a workspace is not a session', () => {
    useAppStore.getState().setActiveProject('vorn')
    live('term-1')
    useAppStore.getState().setVisibleTerminalIds(['term-1'])
    expect(useAppStore.getState().activeProject).toBe('vorn')
  })
})

describe('a session the board is not showing yet', () => {
  it('keeps its pane while the banner is still offering to bring it back', () => {
    // Reopen off: the ended session is not on the board, but the server has it.
    useAppStore.setState({
      terminals: new Map([['term-1', { id: 'term-1' } as never]]),
      knownSessionIds: new Set(['term-1', 'term-2']),
      minimizedTerminals: new Set(['browser:term-2'])
    })
    useAppStore.getState().setVisibleTerminalIds(['term-1'])
    expect([...useAppStore.getState().minimizedTerminals]).toEqual(['browser:term-2'])
  })
})
