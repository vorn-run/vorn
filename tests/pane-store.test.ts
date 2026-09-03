// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'
import { activeBrowserUrl, isPromotedPane } from '../src/renderer/stores/types'
import { parsePersistedBrowsers } from '../src/renderer/stores/ui-slice'
import type { DeviceClaimFailure } from '../packages/shared/src/types'
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
      visibleTerminalIds: [],
      // What the server has. The reconcile prunes against this, not against the
      // board, so a fixture that seeds one without the other prunes everything.
      knownSessionIds: new Set(ids)
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

  it('forgets a page closed by its tab, rather than the pane', () => {
    // Closing the last tab routes through closeBrowserPane, so without this the
    // discard gesture files the page into memory and reopening hands back
    // exactly the page just thrown away.
    const s = () => useAppStore.getState()
    act(() => s().openBrowserPane('t1', 'example.com'))
    act(() => s().closeBrowserTab('t1', 0))
    expect(s().browserPanes.has('t1')).toBe(false)
    expect(s().browserMemory.has('t1')).toBe(false)

    act(() => s().openBrowserPane('t1'))
    expect(browserUrl('t1')).toBe('about:blank')
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
      t1: { tabs: [{ url: 'https://example.com/' }], activeTab: 0, sessionId: 't1' }
    })

    act(() => s().removeTerminal('t1'))
    expect(s().browserPanes.has('t1')).toBe(false)
  })

  it('reads a pane persisted by an older build that stored one url', () => {
    // Shipping the tab strip must not silently drop the page people already
    // had open when they upgrade.
    const panes = parsePersistedBrowsers({ t1: 'https://old.example/' })
    // Owned by its key, which is what a session-keyed entry always meant — so
    // an upgraded entry reads back as the session's own browser, not as a card.
    expect(panes.get('t1')).toEqual({
      tabs: [{ url: 'https://old.example/' }],
      activeTab: 0,
      sessionId: 't1'
    })
  })

  it("carries a card's owner through a persistence round trip", async () => {
    const { parsePersistedEditors } = await import('../src/renderer/stores/ui-slice')
    // The owner is the only thing that makes an entry a card. Lose it and the
    // record reads back as self-owned: it stops being a card, vanishes from the
    // grid, and the next reconcile deletes it outright because `card:t1:0` is
    // not a live session.
    const panes = parsePersistedEditors({
      t1: { filePath: '/p/own.ts', sessionId: 't1' },
      'card:t1:0': { filePath: '/p/popped.ts', sessionId: 't1' }
    })

    expect(panes.get('card:t1:0')).toEqual({ filePath: '/p/popped.ts', sessionId: 't1' })
    expect(isPromotedPane('card:t1:0', panes.get('card:t1:0')!)).toBe(true)
    expect(isPromotedPane('t1', panes.get('t1')!)).toBe(false)
  })

  it('reads an editor persisted by an older build that stored a bare path', async () => {
    const { parsePersistedEditors } = await import('../src/renderer/stores/ui-slice')
    // Owned by its key, which is what a session-keyed entry always meant — so
    // upgrading neither loses the open file nor mistakes it for a card.
    const panes = parsePersistedEditors({ t1: '/p/legacy.ts' })
    expect(panes.get('t1')).toEqual({ filePath: '/p/legacy.ts', sessionId: 't1' })
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
      { url: 'https://example.com/' },
      { url: 'https://example.org/' }
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
      editors: { t1: { filePath: '/p/a.ts', sessionId: 't1' } },
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
    // What the server has, which is what the reconcile prunes against.
    act(() => useAppStore.setState({ terminals, knownSessionIds: new Set(['t1']) }))
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

describe('urls main has already vetted', () => {
  const s = () => useAppStore.getState()

  beforeEach(() => {
    localStorage.clear()
    seed(['t1'])
  })

  // Main allows `file:` inside the session's own root, and it is the only side
  // that can decide that: containment is a filesystem question and the renderer
  // has no filesystem. So a vetted url has to be taken as given here. Re-running
  // `normalizeUrl` on arrival would refuse every one of them, and the refusal is
  // silent — the pane simply never appears.
  const FILE_URL = 'file:///repo/index.html'

  it('opens a pane on a file url main approved', () => {
    act(() => s().openBrowserPane('t1', FILE_URL, { trusted: true }))
    expect(browserUrl('t1')).toBe(FILE_URL)
  })

  it('still refuses a file url nobody vetted', () => {
    // Anything reaching the store on its own — the address bar, a restored
    // session — goes through normalizeUrl exactly as before.
    act(() => s().openBrowserPane('t1', FILE_URL))
    expect(s().browserPanes.has('t1')).toBe(false)
  })

  it('adds a tab on a vetted file url', () => {
    act(() => {
      s().openBrowserPane('t1', 'localhost:5173')
      s().addBrowserTab('t1', FILE_URL, { trusted: true })
    })
    expect(
      s()
        .browserPanes.get('t1')
        ?.tabs.map((t) => t.url)
    ).toEqual(['http://localhost:5173/', FILE_URL])
  })

  it('returns a card sitting on a file page instead of destroying it', () => {
    // The card holds the only copy of that page. Before the landing check
    // counted tabs, an add that no-opped still looked like a success — the
    // card was closed and the page went with it.
    act(() => s().openBrowserPane('t1', 'localhost:5173'))
    let cardId = ''
    act(() => {
      s().addBrowserTab('t1', FILE_URL, { trusted: true })
      cardId = s().promoteBrowserTab('t1', 1) as string
    })
    expect(s().browserPanes.get(cardId)?.tabs[0]?.url).toBe(FILE_URL)

    act(() => s().returnCardToSession(cardId))

    expect(s().browserPanes.has(cardId)).toBe(false)
    expect(
      s()
        .browserPanes.get('t1')
        ?.tabs.map((t) => t.url)
    ).toEqual(['http://localhost:5173/', FILE_URL])
  })
})

describe('what a tab reports about itself', () => {
  const s = () => useAppStore.getState()

  beforeEach(() => {
    localStorage.clear()
    seed(['t1'])
    act(() => s().openBrowserPane('t1', 'example.com'))
  })

  it('drops the old page’s title when the guest moves to a new one', () => {
    act(() => s().syncBrowserTab('t1', 0, { title: 'Old page' }))
    expect(s().browserPanes.get('t1')?.tabs[0]?.title).toBe('Old page')

    // A page with no <title> reports its url with explicitSet false, which the
    // card drops — so nothing would ever overwrite the old title, and the strip
    // would advertise the previous page's name against the new url forever.
    act(() => s().syncBrowserTab('t1', 0, { url: 'https://elsewhere.example/' }))

    const tab = s().browserPanes.get('t1')?.tabs[0]
    expect(tab?.liveUrl).toBe('https://elsewhere.example/')
    expect(tab?.title).toBeUndefined()
  })

  it('keeps a title the page set before its first navigation report', () => {
    // `page-title-updated` can land before `did-navigate`. That first url
    // report names the page the tab was already on, so it is not a move — and
    // treating it as one threw away the name the page had just given.
    act(() => s().syncBrowserTab('t1', 0, { title: 'Dev server' }))
    act(() => s().syncBrowserTab('t1', 0, { url: 'https://example.com/' }))
    expect(s().browserPanes.get('t1')?.tabs[0]?.title).toBe('Dev server')

    // A real move still drops it.
    act(() => s().syncBrowserTab('t1', 0, { url: 'https://elsewhere.example/' }))
    expect(s().browserPanes.get('t1')?.tabs[0]?.title).toBeUndefined()
  })

  it('lets a page clear its own name', () => {
    // An explicitly empty title is a page saying it has none, not a missing
    // report. Treated as absent it would leave the previous name in place.
    act(() => s().syncBrowserTab('t1', 0, { title: 'Named' }))
    act(() => s().syncBrowserTab('t1', 0, { title: '' }))
    expect(s().browserPanes.get('t1')?.tabs[0]?.title).toBeUndefined()
  })

  it('keeps the title when the guest re-reports the same page', () => {
    // A reload or a repeated load event is not a new page, and clearing the
    // title on every one would make the strip flicker between name and host.
    act(() => s().syncBrowserTab('t1', 0, { url: 'https://example.com/', title: 'Example' }))
    act(() => s().syncBrowserTab('t1', 0, { url: 'https://example.com/' }))
    expect(s().browserPanes.get('t1')?.tabs[0]?.title).toBe('Example')
  })

  it('leaves intent alone when the guest wanders', () => {
    // `src` is bound to intent. If an observation could rewrite it, every
    // navigation would re-set `src` and reload the page it just reached.
    act(() => s().syncBrowserTab('t1', 0, { url: 'https://redirected.example/' }))
    expect(s().browserPanes.get('t1')?.tabs[0]?.url).toBe('https://example.com/')
  })

  it('gives a promoted card the page it will actually load', () => {
    // The card mounts a fresh guest on intent. Carrying the old observation
    // across would have its address bar assert the redirected location while
    // the guest is still at the original — permanently, for a page that always
    // redirects.
    act(() =>
      s().syncBrowserTab('t1', 0, { url: 'https://redirected.example/', title: 'Redirected' })
    )
    let cardId = ''
    act(() => {
      cardId = s().promoteBrowserTab('t1', 0) as string
    })

    const tab = s().browserPanes.get(cardId)?.tabs[0]
    expect(tab?.url).toBe('https://example.com/')
    expect(tab?.liveUrl).toBeUndefined()
    expect(tab?.title).toBeUndefined()
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
    const deviceClaim = vi
      .fn()
      .mockResolvedValue({ ok: true, udid: 'u1', name: 'iPhone 17', booted: true })
    stubApi({ deviceClaim })

    let err: DeviceClaimFailure | null = { reason: 'gone', message: 'unset' }
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
      deviceClaim: vi
        .fn()
        .mockResolvedValue({ ok: true, udid: 'u1', name: 'iPhone 17 Pro', booted: true })
    })
    await act(async () => {
      await useAppStore.getState().claimAndOpenDevicePane('s1', device)
    })
    expect(useAppStore.getState().devicePanes.get('s1')?.name).toBe('iPhone 17 Pro')
  })

  it('leaves the pane shut when the claim is refused, and says why', async () => {
    stubApi({
      deviceClaim: vi.fn().mockResolvedValue({
        ok: false,
        reason: 'held-by-session',
        holder: 'other-1',
        message: 'iPhone 17 is in use by session other-1'
      })
    })

    let err: DeviceClaimFailure | null = { reason: 'gone', message: 'unset' }
    await act(async () => {
      err = await useAppStore.getState().claimAndOpenDevicePane('s1', device)
    })

    // A pane opened over a refused claim is worse than no pane: it shows a
    // frame of nothing and buries the one message naming the holder.
    expect(useAppStore.getState().devicePanes.has('s1')).toBe(false)
    expect(err?.reason).toBe('held-by-session')
    expect(err?.message).toContain('other-1')
  })
})

/**
 * Popping a file or a tab out gives it a card of its own in the grid.
 *
 * A card is not a flag beside the pane — it *is* a pane entry whose key is not
 * its owner's id. These pin down that the two collections stay a faithful
 * account of what is on screen: nothing left behind, nothing owned twice.
 */
describe('popping an item out to its own card', () => {
  const s = () => useAppStore.getState()

  beforeEach(() => {
    localStorage.clear()
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api?: Record<string, unknown> }).api,
      notifyWidgetStatus: vi.fn(),
      reorderSessions: vi.fn()
    }
    seed(['t1', 't2'])
  })

  it('opens a file as a card without disturbing the session editor', () => {
    act(() => s().openEditorPane('t1', '/p/open.ts'))
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/popped.ts')
    })

    // The whole point: the session's editor holds one file, so a second file
    // has to land somewhere else or it would displace the first.
    expect(s().editorPanes.get('t1')?.filePath).toBe('/p/open.ts')
    expect(s().editorPanes.get(cardId)).toEqual({ filePath: '/p/popped.ts', sessionId: 't1' })
  })

  it('gives every card a distinct id, so two never collapse into one', () => {
    let a = ''
    let b = ''
    act(() => {
      a = s().promoteFile('t1', '/p/a.ts')
      b = s().promoteFile('t1', '/p/b.ts')
    })
    expect(a).not.toBe(b)
    expect(s().editorPanes.size).toBe(2)
  })

  it('takes a tab out of the strip rather than copying it', () => {
    act(() => {
      s().openBrowserPane('t1', 'example.com')
      s().addBrowserTab('t1', 'vorn.dev')
    })
    let cardId: string | null = null
    act(() => {
      cardId = s().promoteBrowserTab('t1', 1)
    })

    // Left in both places it would be two guests on one url, each with its own
    // scroll position — and closing either would look like a refusal to go.
    expect(s().browserPanes.get('t1')?.tabs).toEqual([{ url: 'https://example.com/' }])
    expect(s().browserPanes.get(cardId as unknown as string)?.tabs).toEqual([
      { url: 'https://vorn.dev/' }
    ])
  })

  it('closes the strip when its last tab is popped out', () => {
    act(() => s().openBrowserPane('t1', 'example.com'))
    act(() => {
      s().promoteBrowserTab('t1', 0)
    })
    // A browser with no pages is a box taking up a cell, the same as closing
    // its last tab any other way.
    expect(s().browserPanes.has('t1')).toBe(false)
  })

  it('refuses an index that names no tab', () => {
    act(() => s().openBrowserPane('t1', 'example.com'))
    let cardId: string | null = 'unset'
    act(() => {
      cardId = s().promoteBrowserTab('t1', 4)
    })
    expect(cardId).toBeNull()
    expect(s().browserPanes.get('t1')?.tabs).toHaveLength(1)
  })

  it('returns a file to the session editor', () => {
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/popped.ts')
    })
    act(() => s().returnCardToSession(cardId))

    expect(s().editorPanes.has(cardId)).toBe(false)
    expect(s().editorPanes.get('t1')?.filePath).toBe('/p/popped.ts')
  })

  it('returns a tab to the end of the strip it came from', () => {
    act(() => {
      s().openBrowserPane('t1', 'example.com')
      s().addBrowserTab('t1', 'vorn.dev')
    })
    let cardId = ''
    act(() => {
      cardId = s().promoteBrowserTab('t1', 1) as string
    })
    act(() => s().returnCardToSession(cardId))

    expect(s().browserPanes.has(cardId)).toBe(false)
    expect(s().browserPanes.get('t1')?.tabs).toEqual([
      { url: 'https://example.com/' },
      { url: 'https://vorn.dev/' }
    ])
  })

  it('opens a browser to receive a tab whose strip has since closed', () => {
    act(() => s().openBrowserPane('t1', 'example.com'))
    let cardId = ''
    act(() => {
      cardId = s().promoteBrowserTab('t1', 0) as string
    })
    expect(s().browserPanes.has('t1')).toBe(false)

    act(() => s().returnCardToSession(cardId))
    // Refusing would strand the page: the card is closing either way, so with
    // nowhere to land the tab would simply be gone.
    expect(s().browserPanes.get('t1')?.tabs).toEqual([{ url: 'https://example.com/' }])
  })

  it('un-minimizes nothing but forgets the card it closed', () => {
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/popped.ts')
    })
    act(() => s().toggleMinimized(cardId))
    expect(s().minimizedTerminals.has(cardId)).toBe(true)

    act(() => s().closeEditorPane(cardId))
    // An id left in the dock is an entry that restores nothing.
    expect(s().minimizedTerminals.has(cardId)).toBe(false)
  })

  it('does not file a popped-out page into the reopen memory', () => {
    act(() => {
      s().openBrowserPane('t1', 'example.com')
      s().addBrowserTab('t1', 'vorn.dev')
    })
    let cardId = ''
    act(() => {
      cardId = s().promoteBrowserTab('t1', 1) as string
    })
    act(() => s().closeBrowserPane(cardId))

    // Memory is for reopening a session's own browser. A discarded card filed
    // there would resurface its page on the session's next open.
    expect(s().browserMemory.has(cardId)).toBe(false)
  })

  it("takes a session's cards down with it", () => {
    act(() => {
      s().openBrowserPane('t1', 'example.com')
      s().addBrowserTab('t1', 'vorn.dev')
    })
    let file = ''
    let tab = ''
    act(() => {
      file = s().promoteFile('t1', '/p/popped.ts')
      tab = s().promoteBrowserTab('t1', 1) as string
      s().toggleMinimized(file)
    })

    act(() => s().removeTerminal('t1'))

    // Only the record names the owner. Dropping by key alone would leave both
    // cards on the grid, drawn against a session the store no longer has.
    expect(s().editorPanes.has(file)).toBe(false)
    expect(s().browserPanes.has(tab)).toBe(false)
    expect(s().minimizedTerminals.has(file)).toBe(false)
  })

  it('drops a selection pointing at a card that has been closed', () => {
    // Cmd+O focuses whatever is selected, so a dead id here reaches the same
    // empty stage a dead focus id does. A view does sweep a stale selection,
    // but only while a view deriving the visible list is mounted — which is not
    // true on the focus stage, so the store cannot rely on being tidied up.
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/a.ts')
    })
    act(() => s().setSelectedTerminal(cardId))

    act(() => s().closeCard(cardId))
    expect(s().selectedTerminalId).toBeNull()
  })

  it("drops a selection pointing at a closed session's card", () => {
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/a.ts')
    })
    act(() => s().setSelectedTerminal(cardId))

    act(() => s().removeTerminal('t1'))
    expect(s().selectedTerminalId).toBeNull()
  })

  it("releases focus, preview and maximize when a card's session is closed", async () => {
    const { dirtyRefFor, isEditorDirty } = await import('../src/renderer/lib/editor-dirty')
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/popped.ts')
    })
    dirtyRefFor(cardId).current = true
    act(() =>
      useAppStore.setState({
        focusedTerminalId: cardId,
        previewTerminalId: cardId,
        maximizedPaneId: cardId
      } as never)
    )

    act(() => s().removeTerminal('t1'))

    // The focus pair is the dangerous one: the stage is chosen by "is anything
    // focused" and the titlebar is dropped while something is, so a card
    // outliving its session leaves an empty window with no chrome.
    expect(s().focusedTerminalId).toBeNull()
    expect(s().previewTerminalId).toBeNull()
    expect(s().maximizedPaneId).toBeNull()
    // And the buffer flag, or a later card inherits its prompt.
    expect(isEditorDirty(cardId)).toBe(false)
  })

  it("leaves another session's cards alone", () => {
    let mine = ''
    let theirs = ''
    act(() => {
      mine = s().promoteFile('t1', '/p/a.ts')
      theirs = s().promoteFile('t2', '/p/b.ts')
    })

    act(() => s().removeTerminal('t1'))
    expect(s().editorPanes.has(mine)).toBe(false)
    expect(s().editorPanes.get(theirs)?.sessionId).toBe('t2')
  })

  it("asks before a close discards a card's unsaved edits", async () => {
    const { dirtyRefFor, isEditorDirty } = await import('../src/renderer/lib/editor-dirty')
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/draft.ts')
    })
    dirtyRefFor(cardId).current = true
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false)

    act(() => s().closeCard(cardId))
    expect(confirm).toHaveBeenCalledOnce()
    // Cancelled: the card and its buffer both survive.
    expect(s().editorPanes.has(cardId)).toBe(true)
    expect(isEditorDirty(cardId)).toBe(true)

    confirm.mockReturnValue(true)
    act(() => s().closeCard(cardId))
    expect(s().editorPanes.has(cardId)).toBe(false)
    // The flag must not outlive the card, or a later card inherits the prompt.
    expect(isEditorDirty(cardId)).toBe(false)
    confirm.mockRestore()
  })

  it("asks before a return discards the session editor's buffer", async () => {
    const { dirtyRefFor } = await import('../src/renderer/lib/editor-dirty')
    act(() => s().openEditorPane('t1', '/p/open.ts'))
    dirtyRefFor('t1').current = true
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/popped.ts')
    })
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false)

    act(() => s().returnCardToSession(cardId))
    // Returning displaces the session's editor exactly as picking a file in the
    // tree does — and that path has always asked first.
    expect(confirm).toHaveBeenCalled()
    expect(s().editorPanes.get('t1')?.filePath).toBe('/p/open.ts')
    expect(s().editorPanes.has(cardId)).toBe(true)
    confirm.mockRestore()
  })

  it("does not clear one buffer's flag when the other answer is no", async () => {
    const { dirtyRefFor, isEditorDirty } = await import('../src/renderer/lib/editor-dirty')
    act(() => s().openEditorPane('t1', '/p/open.ts'))
    dirtyRefFor('t1').current = true
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/popped.ts')
    })
    dirtyRefFor(cardId).current = true
    // Yes, then no. Asked as two questions the yes cleared the session editor's
    // flag and the no then bailed, leaving those edits on screen with nothing
    // left to prompt about them — so the next pane switch discarded them in
    // silence. One question cannot produce that state at all.
    const confirm = vi.spyOn(window, 'confirm').mockReturnValueOnce(true).mockReturnValueOnce(false)

    try {
      act(() => s().returnCardToSession(cardId))

      expect(confirm).toHaveBeenCalledOnce()
      // The single yes covered both buffers, so the return actually happened —
      // rather than half-applying and leaving the card where it was.
      expect(s().editorPanes.has(cardId)).toBe(false)
      expect(s().editorPanes.get('t1')?.filePath).toBe('/p/popped.ts')
      expect(isEditorDirty('t1')).toBe(false)
      expect(isEditorDirty(cardId)).toBe(false)
    } finally {
      confirm.mockRestore()
    }
  })

  it('lands a returned card on the tab that was in front', async () => {
    act(() => s().openBrowserPane('t1', 'one.example'))
    let cardId = ''
    act(() => {
      cardId = s().promoteBrowserTab('t1', 0) as string
    })
    act(() => {
      s().addBrowserTab(cardId, 'two.example')
      s().addBrowserTab(cardId, 'three.example')
      s().setActiveBrowserTab(cardId, 1)
    })

    act(() => s().returnCardToSession(cardId))
    const pane = s().browserPanes.get('t1')!
    // `addBrowserTab` activates what it adds, so the strip ended up on the
    // card's *last* page rather than the one being looked at.
    expect(pane.tabs[pane.activeTab]?.url).toBe('https://two.example/')
  })

  it('surfaces the existing card when a file is popped out twice', () => {
    let first = ''
    act(() => {
      first = s().promoteFile('t1', '/p/same.ts')
    })
    act(() => s().toggleMinimized(first))
    expect(s().minimizedTerminals.has(first)).toBe(true)

    let second = ''
    act(() => {
      second = s().promoteFile('t1', '/p/same.ts')
    })

    // Returning the id and doing nothing else made the control look dead: the
    // card existed, minimized, and nothing brought it back.
    expect(second).toBe(first)
    expect(s().minimizedTerminals.has(first)).toBe(false)
  })

  it('prunes a dead session even when the visible list never changes', () => {
    // `setVisibleTerminalIds` is reconcile's only trigger. Gating the whole
    // write on the list changing meant a launch where it never moved — every
    // session filtered out, or every restored one minimized — never pruned at
    // all, and the dead panes stayed in localStorage for the whole run.
    act(() => s().openFilesPane('t1'))
    // t1 is gone but t2 is live, so reconcile has a non-empty live set to prune
    // against — and the visible list is unchanged, which is the whole point.
    act(() =>
      useAppStore.setState({
        terminals: new Map([
          ['t2', { id: 't2', session: session('t2'), status: 'idle', lastOutputTimestamp: 1 }]
        ]),
        visibleTerminalIds: ['t2'],
        knownSessionIds: new Set(['t2'])
      } as never)
    )
    expect(s().filesPanes.has('t1')).toBe(true)

    act(() => s().setVisibleTerminalIds(['t2']))
    expect(s().filesPanes.has('t1')).toBe(false)
  })

  it('closes a page card without prompting — there is no buffer to lose', async () => {
    const { dirtyRefFor } = await import('../src/renderer/lib/editor-dirty')
    act(() => s().openBrowserPane('t1', 'example.com'))
    let cardId = ''
    act(() => {
      cardId = s().promoteBrowserTab('t1', 0) as string
    })
    // A dirty flag under this id, so "no prompt" is a decision rather than an
    // accident of there being nothing to prompt about. A page has no buffer;
    // the flag is stale registry state and must not resurrect a dialog.
    dirtyRefFor(cardId).current = true
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)

    act(() => s().closeCard(cardId))
    expect(confirm).not.toHaveBeenCalled()
    expect(s().browserPanes.has(cardId)).toBe(false)
    confirm.mockRestore()
  })

  it('returns every tab a card gathered, not just the one in front', () => {
    // A card's strip keeps its `+`, so it can collect pages of its own. Carrying
    // back only the active one dropped the rest without a word, and the pop-out
    // control is hidden on a card, so there was no way to rescue them first.
    act(() => s().openBrowserPane('t1', 'example.com'))
    let cardId = ''
    act(() => {
      cardId = s().promoteBrowserTab('t1', 0) as string
    })
    act(() => {
      s().addBrowserTab(cardId, 'second.example')
      s().addBrowserTab(cardId, 'third.example')
    })

    act(() => s().returnCardToSession(cardId))
    expect(s().browserPanes.get('t1')?.tabs).toEqual([
      { url: 'https://example.com/' },
      { url: 'https://second.example/' },
      { url: 'https://third.example/' }
    ])
  })

  it('never opens the same file twice for one session', () => {
    // Two cards on one path is two editors over one file, each with its own
    // buffer: save in one, save in the other, and the second writes its stale
    // copy over the first with nothing to report it.
    let first = ''
    let second = ''
    act(() => {
      first = s().promoteFile('t1', '/p/same.ts')
      second = s().promoteFile('t1', '/p/same.ts')
    })

    expect(second).toBe(first)
    expect([...s().editorPanes].filter(([id]) => id !== 't1')).toHaveLength(1)
  })

  it('still gives two sessions their own card for the same path', () => {
    // Different worktrees, different files on disk under the same relative name.
    let a = ''
    let b = ''
    act(() => {
      a = s().promoteFile('t1', '/p/same.ts')
      b = s().promoteFile('t2', '/p/same.ts')
    })
    expect(a).not.toBe(b)
  })

  it('never lets a card corrupt the session order', () => {
    // The grid and the tab strip drag within lists that interleave cards, while
    // terminalOrder holds sessions only. Passing an index from one into the
    // other moved the wrong session, and an index past the end spliced nothing
    // and wrote `undefined` into the order — which is then persisted and sent
    // to the server.
    seed(['t1', 't2', 't3'])
    const reorderSessions = vi.fn()
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api?: Record<string, unknown> }).api,
      reorderSessions
    }
    act(() => {
      s().promoteFile('t1', '/p/a.ts')
    })

    // Dropped by id, so the card sitting between t1 and t2 in the *visible*
    // order cannot shift the target. Under the old index-based call this same
    // gesture moved a different session, and a visible index past the end of
    // `terminalOrder` spliced nothing and wrote `undefined` into it.
    act(() => s().reorderTerminals('t3', 't1'))
    expect(s().terminalOrder).toEqual(['t3', 't1', 't2'])
    // What actually reaches the server is the thing that was being corrupted.
    expect(reorderSessions).toHaveBeenLastCalledWith(['t3', 't1', 't2'])
  })

  it('treats dragging a card as a no-op rather than moving its owner', () => {
    // A card has no position of its own — it is drawn beside the session it came
    // from — so there is nothing for a drag to reorder.
    //
    // Three sessions and the middle one as the target, deliberately: with two
    // sessions and the last as target, a missing guard splices at -1 and lands
    // on an order identical to the one it started with, so the bug hides.
    seed(['t1', 't2', 't3'])
    const reorderSessions = vi.fn()
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api?: Record<string, unknown> }).api,
      reorderSessions
    }
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/a.ts')
    })
    const before = s().terminalOrder

    act(() => s().reorderTerminals(cardId, 't2'))
    // Reference identity: the reducer must return nothing at all, not an equal
    // array. And nothing may reach the server.
    expect(s().terminalOrder).toBe(before)
    expect(reorderSessions).not.toHaveBeenCalled()
  })

  it("resolves a drop onto a card to its owner's slot", () => {
    seed(['t1', 't2', 't3'])
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t3', '/p/a.ts')
    })

    act(() => s().reorderTerminals('t1', cardId))
    expect(s().terminalOrder).toEqual(['t2', 't3', 't1'])
  })

  it('keeps cards through the reconcile that prunes dead sessions', () => {
    // Reconcile prunes on the owner, not the key. Pruning by key would delete
    // every card on the first pass, silently discarding the files and pages
    // someone had put on the grid.
    let cardId = ''
    act(() => {
      cardId = s().promoteFile('t1', '/p/popped.ts')
    })
    act(() => s().setVisibleTerminalIds(['t1', 't2']))

    expect(s().editorPanes.has(cardId)).toBe(true)
  })
})
