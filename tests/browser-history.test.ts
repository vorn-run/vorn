import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * Stepping through the pane's own history.
 *
 * The failure worth testing is the quiet one. An agent that asks to go back at
 * the start of a tab's history and is told "ok" carries on believing it moved,
 * then reads the same page again thinking it is the previous one — a wrong
 * answer built on a call that reported success.
 */

let cdpCalls: { method: string; params?: Record<string, unknown> }[] = []
/** What `Page.getNavigationHistory` reports. Swapped per test. */
const history = {
  currentIndex: 1,
  entries: [
    { id: 10, url: 'https://first.example/' },
    { id: 11, url: 'https://second.example/' },
    { id: 12, url: 'https://third.example/' }
  ]
}

vi.mock('electron', () => ({
  webContents: {
    fromId: () => ({
      isDestroyed: () => false,
      getURL: () => 'https://second.example/',
      getTitle: () => 'Second',
      debugger: {
        isAttached: () => false,
        attach: () => {},
        detach: () => {},
        on: () => {},
        off: () => {},
        removeListener: () => {},
        sendCommand: async (method: string, params?: Record<string, unknown>) => {
          cdpCalls.push({ method, params })
          if (method === 'Page.getNavigationHistory') return history
          return {}
        }
      }
    })
  }
}))

import { attach, detach, goHistory, setRendererSend } from '../src/main/browser-registry'

const jumped = (): Record<string, unknown> | undefined =>
  cdpCalls.find((c) => c.method === 'Page.navigateToHistoryEntry')?.params

beforeEach(() => {
  cdpCalls = []
  history.currentIndex = 1
  setRendererSend(() => {})
  detach('sess-history')
  attach('sess-history', 1)
  cdpCalls = []
})

describe('walking the pane back and forward', () => {
  it('goes to the entry before this one and says where it landed', async () => {
    const result = await goHistory({ sessionId: 'sess-history', direction: 'back' })

    expect(jumped()).toMatchObject({ entryId: 10 })
    // The url comes from the history entry, not from a re-read of the guest:
    // the navigation has been asked for, not finished, and reading the guest
    // now would report the page being left.
    expect(result.url).toBe('https://first.example/')
  })

  it('goes to the entry after this one', async () => {
    const result = await goHistory({ sessionId: 'sess-history', direction: 'forward' })

    expect(jumped()).toMatchObject({ entryId: 12 })
    expect(result.url).toBe('https://third.example/')
  })

  it('refuses to go back from the first page rather than reporting success', async () => {
    history.currentIndex = 0

    await expect(goHistory({ sessionId: 'sess-history', direction: 'back' })).rejects.toThrow(
      /first page/
    )
    // Nothing was navigated. A jump to a bogus entry id would be the worst
    // outcome: an agent told it moved, looking at a page it did not choose.
    expect(jumped()).toBeUndefined()
  })

  it('refuses to go forward from the newest page', async () => {
    history.currentIndex = 2

    await expect(goHistory({ sessionId: 'sess-history', direction: 'forward' })).rejects.toThrow(
      /newest page/
    )
    expect(jumped()).toBeUndefined()
  })

  it('says there is no pane rather than acting on someone else’s', async () => {
    detach('sess-history')

    await expect(goHistory({ sessionId: 'sess-history', direction: 'back' })).rejects.toThrow(
      /no browser pane open/
    )
  })
})
