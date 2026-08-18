import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * Listing what the tab strip holds.
 *
 * `close` and `select` take a zero-based index, and until a listing existed an
 * agent could only guess what any index named — so it either acted on a tab it
 * had never seen or switched to one just to find out what it was. Both are the
 * quiet kind of wrong: the call succeeds, on the wrong page.
 *
 * The listing is a mirror of what the renderer reported, never a second copy
 * main maintains. These tests pin that: main must not invent, keep, or edit
 * tab state of its own.
 */

vi.mock('electron', () => ({
  webContents: {
    fromId: () => ({
      isDestroyed: () => false,
      getURL: () => 'https://example.test/',
      getTitle: () => 'Example',
      debugger: {
        isAttached: () => false,
        attach: () => {},
        detach: () => {},
        on: () => {},
        off: () => {},
        removeListener: () => {},
        sendCommand: async () => ({})
      }
    })
  }
}))

import { attach, detach, listTabs, syncTabs, setRendererSend } from '../src/main/browser-registry'

const strip = [
  { index: 0, url: 'http://localhost:5173/', title: 'Dev server', active: false },
  { index: 1, url: 'https://vorn.dev/docs', title: 'Docs', active: true }
]

beforeEach(() => {
  setRendererSend(() => {})
  detach('sess-tabs')
  attach('sess-tabs', 1)
})

describe('listing a pane’s tabs', () => {
  it('answers with what the renderer reported, indices included', () => {
    syncTabs('sess-tabs', strip)

    const { tabs } = listTabs({ sessionId: 'sess-tabs' })
    expect(tabs).toEqual(strip)
    // The index is the whole point: it is what `close` and `select` take, so a
    // listing that renumbered or reordered would be worse than none at all.
    expect(tabs.map((t) => t.index)).toEqual([0, 1])
    expect(tabs.find((t) => t.active)?.url).toBe('https://vorn.dev/docs')
  })

  it('reports a strip that has not arrived yet as empty, not as a missing pane', () => {
    // The renderer's report follows the pane by a frame. Throwing here would
    // tell an agent the session has no browser a moment after one opened.
    expect(listTabs({ sessionId: 'sess-tabs' })).toEqual({ tabs: [] })
  })

  it('says there is no pane rather than listing someone else’s', () => {
    syncTabs('sess-tabs', strip)
    detach('sess-tabs')

    expect(() => listTabs({ sessionId: 'sess-tabs' })).toThrow(/no browser pane open/)
  })

  it('forgets the strip when the pane goes, so a later pane cannot inherit it', () => {
    syncTabs('sess-tabs', strip)
    detach('sess-tabs')
    // A new pane on the same session id. Without the mirror being cleared it
    // would answer with the previous pane's tabs — indices pointing at pages
    // that are no longer there.
    attach('sess-tabs', 1)

    expect(listTabs({ sessionId: 'sess-tabs' })).toEqual({ tabs: [] })
  })

  it('takes the renderer’s word wholesale rather than merging with its own', () => {
    syncTabs('sess-tabs', strip)
    // Someone closed the first tab by hand. Main cannot see that happen, which
    // is exactly why it keeps no copy to reconcile — it is told the result.
    syncTabs('sess-tabs', [{ index: 0, url: 'https://vorn.dev/docs', active: true }])

    expect(listTabs({ sessionId: 'sess-tabs' }).tabs).toEqual([
      { index: 0, url: 'https://vorn.dev/docs', active: true }
    ])
  })
})
