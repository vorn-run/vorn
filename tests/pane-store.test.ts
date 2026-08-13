// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'
import { activeBrowserUrl } from '../src/renderer/stores/types'
import { parsePersistedBrowsers } from '../src/renderer/stores/ui-slice'
import { DEVICE_SPLIT_RATIO } from '../src/renderer/lib/split-ratio'

/** The page a session's browser is showing, i.e. its active tab. */
const browserUrl = (id: string): string | null =>
  activeBrowserUrl(useAppStore.getState().browserPanes.get(id))

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
      browserPanes: new Map(),
      browserMemory: new Map(),
      devicePanes: new Map(),
      cardSplits: {},
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

  it('toggling the tree opens then closes it', () => {
    const s = () => useAppStore.getState()
    act(() => s().toggleFilesPane('t1'))
    expect(s().filesPanes.has('t1')).toBe(true)

    act(() => s().toggleFilesPane('t1'))
    expect(s().filesPanes.has('t1')).toBe(false)
  })

  it('clears the editor dirty flag when its session closes', async () => {
    const { dirtyRefFor, isEditorDirty } = await import('../src/renderer/lib/editor-dirty')
    const s = () => useAppStore.getState()
    act(() => s().openEditorPane('t1', '/p/a.ts'))
    dirtyRefFor('t1').current = true

    act(() => s().removeTerminal('t1'))

    // The registry lives outside the store; a leaked flag would make a recycled
    // session id prompt about edits that no longer exist.
    expect(isEditorDirty('t1')).toBe(false)
  })

  it('opens a browser with a default page and normalizes typed urls', () => {
    const s = () => useAppStore.getState()

    act(() => s().openBrowserPane('t1'))
    expect(browserUrl('t1')).toBe('about:blank')

    act(() => s().openBrowserPane('t1', 'localhost:5173'))
    expect(browserUrl('t1')).toBe('http://localhost:5173/')
  })

  it('ignores an unloadable url rather than blanking the pane', () => {
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'example.com'))

    act(() => s().openBrowserPane('t1', 'javascript:alert(1)'))
    // The pane keeps the page it had; a rejected scheme must not clear it.
    expect(browserUrl('t1')).toBe('https://example.com/')
  })

  it('re-opening without a url keeps the current page', () => {
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'example.com'))
    act(() => s().openBrowserPane('t1'))

    expect(browserUrl('t1')).toBe('https://example.com/')
  })

  it('toggling the browser opens then closes it', () => {
    const s = () => useAppStore.getState()
    act(() => s().toggleBrowserPane('t1'))
    expect(s().browserPanes.has('t1')).toBe(true)

    // Once visible, the toggle closes as usual.
    act(() => s().toggleBrowserPane('t1'))
    expect(s().browserPanes.has('t1')).toBe(false)

    expect(s().maximizedPaneId).toBeNull()
  })

  it('reopens a closed browser on the tabs it had', () => {
    // `browserPanes` does double duty: the entry's presence is what makes the
    // pane open, and its value is the tabs. Closing has to delete the entry, so
    // without somewhere to put them the tabs went too, and reopening handed
    // back a blank page — losing however many pages had been opened.
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'example.com'))
    act(() => s().addBrowserTab('t1', 'vorn.dev'))
    const tabs = s().browserPanes.get('t1')?.tabs
    expect(tabs).toHaveLength(2)

    act(() => s().closeBrowserPane('t1'))
    expect(s().browserPanes.has('t1')).toBe(false)

    act(() => s().openBrowserPane('t1'))
    expect(s().browserPanes.get('t1')?.tabs).toEqual(tabs)
  })

  it("does not hand a recycled session id the previous one's pages", () => {
    // Ids are reused, and remembered tabs outliving their session would reopen
    // a browser onto pages that belonged to someone else's work. Deliberately
    // no re-seed here: seeding resets browserMemory, which would clear the very
    // thing this is checking gets dropped and let the test pass on its own.
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'example.com'))
    act(() => s().closeBrowserPane('t1'))
    expect(s().browserMemory.has('t1')).toBe(true)

    act(() => s().removeTerminal('t1'))
    expect(s().browserMemory.has('t1')).toBe(false)

    act(() => s().openBrowserPane('t1'))
    expect(activeBrowserUrl(s().browserPanes.get('t1'))).toBe('about:blank')
  })

  it('keeps each session on its own page', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openBrowserPane('t1', 'localhost:5173')
      s().openBrowserPane('t2', 'localhost:3000')
    })

    expect(browserUrl('t1')).toBe('http://localhost:5173/')
    expect(browserUrl('t2')).toBe('http://localhost:3000/')
  })

  it('persists the open page and drops it with its session', () => {
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'example.com'))
    expect(JSON.parse(localStorage.getItem('vorn:panes') as string).browsers).toEqual({
      t1: { tabs: ['https://example.com/'], activeTab: 0 }
    })

    act(() => s().removeTerminal('t1'))
    expect(s().browserPanes.has('t1')).toBe(false)
  })

  it('reads a pane persisted by an older build that stored one url', () => {
    // Shipping the tab strip must not silently drop the page people already
    // had open when they upgrade.
    const panes = parsePersistedBrowsers({ t1: 'https://old.example/' })
    expect(panes.get('t1')).toEqual({ tabs: ['https://old.example/'], activeTab: 0 })
  })

  it('clamps a persisted active tab that points past the end', () => {
    const panes = parsePersistedBrowsers({ t1: { tabs: ['https://a/'], activeTab: 4 } })
    expect(panes.get('t1')?.activeTab).toBe(0)
  })

  it('opens a tab, switches to it, and navigates only the active tab', () => {
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'example.com'))
    act(() => s().addBrowserTab('t1', 'localhost:5173'))

    // A new tab is the one you are looking at — otherwise the button appears
    // to do nothing.
    expect(s().browserPanes.get('t1')?.activeTab).toBe(1)
    expect(browserUrl('t1')).toBe('http://localhost:5173/')

    act(() => s().openBrowserPane('t1', 'example.org'))
    // Typing an address replaces the current page rather than spawning a tab.
    expect(s().browserPanes.get('t1')?.tabs).toEqual([
      'https://example.com/',
      'https://example.org/'
    ])
  })

  it('lands on a neighbour when the active tab closes', () => {
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'a.com'))
    act(() => {
      s().addBrowserTab('t1', 'b.com')
      s().addBrowserTab('t1', 'c.com')
    })

    act(() => s().closeBrowserTab('t1', 2))
    expect(browserUrl('t1')).toBe('https://b.com/')

    // Closing a tab to the left shifts the active index, or the user would
    // find themselves on a different page than the one they were reading.
    act(() => s().setActiveBrowserTab('t1', 1))
    act(() => s().closeBrowserTab('t1', 0))
    expect(browserUrl('t1')).toBe('https://b.com/')
  })

  it('closing the last tab closes the pane rather than leaving an empty box', () => {
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'a.com'))
    act(() => s().closeBrowserTab('t1', 0))

    expect(s().browserPanes.has('t1')).toBe(false)
  })

  it('closing a browser clears a maximize that pointed at it', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openBrowserPane('t1', 'example.com')
      s().setMaximizedPane('browser:t1')
    })
    act(() => s().closeBrowserPane('t1'))

    expect(s().maximizedPaneId).toBeNull()
  })

  it('closing a pane clears its maximized state', () => {
    const s = () => useAppStore.getState()
    act(() => {
      s().openFilesPane('t1')
      s().setMaximizedPane('files:t1')
    })
    expect(s().maximizedPaneId).toBe('files:t1')

    act(() => s().closeFilesPane('t1'))
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
      editors: { t1: '/p/a.ts' },
      browsers: {}
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

  it('gives the terminal the larger share when a device pane opens', () => {
    const s = () => useAppStore.getState()
    act(() => s().openDevicePane('t1', { udid: 'u1', name: 'iPhone 17' }))

    // A phone fills a pane by height, so an even split spends width on empty
    // background and takes it from the terminal, which uses every column.
    expect(s().cardSplits.t1.terminal).toBeCloseTo(DEVICE_SPLIT_RATIO)
    expect(s().cardSplits.t1.terminal).toBeGreaterThan(0.5)
  })

  it('leaves a split the person already chose alone', () => {
    const s = () => useAppStore.getState()
    act(() => s().setCardSplit('t1', { terminal: 0.3, panes: [] }))
    act(() => s().openDevicePane('t1', { udid: 'u1', name: 'iPhone 17' }))

    // Seeding a default over a ratio someone dragged silently undoes their
    // decision — the pane would snap back every time a device reattached.
    expect(s().cardSplits.t1.terminal).toBeCloseTo(0.3)
  })
})

describe('claiming a device before showing it', () => {
  const device = { udid: 'u1', name: 'iPhone 17' }

  const stubApi = (api: Record<string, unknown>): void => {
    Object.defineProperty(window, 'api', {
      value: { ...(window as unknown as { api?: object }).api, ...api },
      writable: true,
      configurable: true
    })
  }

  beforeEach(() => {
    act(() => {
      useAppStore.setState({ devicePanes: new Map() })
    })
  })

  it('claims the device before opening the pane', async () => {
    const deviceClaim = vi.fn().mockResolvedValue({ udid: 'u1', name: 'iPhone 17', booted: true })
    stubApi({ deviceClaim })

    let err: string | null = 'unset'
    await act(async () => {
      err = await useAppStore.getState().claimAndOpenDevicePane('s1', device)
    })

    // Opening without claiming leaves the pane polling a session main has no
    // device for: every frame fails with "No device is claimed" and the picker
    // looks like it did nothing at all.
    expect(deviceClaim).toHaveBeenCalledWith('s1', 'u1')
    expect(err).toBeNull()
    expect(useAppStore.getState().devicePanes.get('s1')).toEqual(device)
  })

  it('takes the name main reports, not the one the picker guessed', async () => {
    stubApi({
      deviceClaim: vi.fn().mockResolvedValue({ udid: 'u1', name: 'iPhone 17 Pro', booted: true })
    })
    await act(async () => {
      await useAppStore.getState().claimAndOpenDevicePane('s1', device)
    })
    expect(useAppStore.getState().devicePanes.get('s1')?.name).toBe('iPhone 17 Pro')
  })

  it('leaves the pane shut when the claim is refused, and says why', async () => {
    stubApi({
      deviceClaim: vi.fn().mockRejectedValue(new Error('iPhone 17 is in use by session other-1'))
    })

    let err: string | null = null
    await act(async () => {
      err = await useAppStore.getState().claimAndOpenDevicePane('s1', device)
    })

    // A pane opened over a refused claim is worse than no pane: it shows a
    // frame of nothing and buries the one message naming the holder.
    expect(useAppStore.getState().devicePanes.has('s1')).toBe(false)
    expect(err).toContain('other-1')
  })
})
