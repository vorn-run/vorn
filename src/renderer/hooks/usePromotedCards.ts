import { useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { displayHost } from '../lib/browser-url'
import { activeBrowserUrl, isPromotedPane, type AppStore } from '../stores/types'

/**
 * What a card is and what it is showing.
 *
 * There is no list of promoted cards to keep in step with anything: a card *is*
 * a pane entry whose key is not its owner's id. Reading them back out of the two
 * pane collections means a card cannot outlive the pane it draws, and closing
 * the pane is what closes the card.
 */
export interface PromotedCard {
  /** The card's pane id — `card:<sessionId>:<n>`. */
  id: string
  kind: 'editor' | 'browser'
  sessionId: string
  /** Absolute file path, or the page's url. */
  subject: string
  /** What to call it: the filename, or the page's host. */
  name: string
}

type PaneMaps = Pick<AppStore, 'editorPanes' | 'browserPanes'>

/** Stable empty results, so "no cards" does not invalidate every memo downstream. */
const NO_CARDS: PromotedCard[] = []
const NO_OWNERS = new Map<string, string[]>()

function fileName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path
}

function collect(state: PaneMaps): PromotedCard[] {
  const cards: PromotedCard[] = []
  for (const [id, pane] of state.editorPanes) {
    if (!isPromotedPane(id, pane)) continue
    cards.push({
      id,
      kind: 'editor',
      sessionId: pane.sessionId,
      subject: pane.filePath,
      name: fileName(pane.filePath)
    })
  }
  for (const [id, pane] of state.browserPanes) {
    if (!isPromotedPane(id, pane)) continue
    const url = activeBrowserUrl(pane) ?? ''
    cards.push({
      id,
      kind: 'browser',
      sessionId: pane.sessionId,
      subject: url,
      name: displayHost(url)
    })
  }
  return cards.length === 0 ? NO_CARDS : cards
}

function usePaneMaps(): PaneMaps {
  return useAppStore(
    useShallow((s) => ({ editorPanes: s.editorPanes, browserPanes: s.browserPanes }))
  )
}

/** Every promoted card. */
export function usePromotedCards(): PromotedCard[] {
  const { editorPanes, browserPanes } = usePaneMaps()
  return useMemo(() => collect({ editorPanes, browserPanes }), [editorPanes, browserPanes])
}

/**
 * The cards `sessionId` popped out.
 *
 * Filtered from the shared list rather than scanning the pane maps itself: this
 * is called once per session row in the sidebar, so scanning per call turned one
 * walk of both maps into one walk per session.
 */
export function usePromotedCardsFor(sessionId: string): PromotedCard[] {
  const all = usePromotedCards()
  return useMemo(() => {
    const mine = all.filter((c) => c.sessionId === sessionId)
    return mine.length === 0 ? NO_CARDS : mine
  }, [all, sessionId])
}

/**
 * Card ids grouped by the session they came from.
 *
 * The one place that answers "which cards sit beside which session" — the
 * ordering contract the grid and the tab strip are both built on. Written twice,
 * they drifted the moment either grew a filter. Grouped on `pane.sessionId`, the
 * authoritative owner, rather than by parsing it back out of the card's id.
 *
 * Keyed on which cards exist and whose they are, deliberately — not on the pane
 * Maps those come from. Those Maps are replaced on *every* pane write: switching
 * a browser tab, a keystroke in the address bar. Memoizing on them meant this
 * Map changed identity on each one, which re-ran the grid's layout memo — two
 * full sorts of every session — for an arrangement that had not moved. What a
 * card *shows* changes often; where it sits almost never.
 */
export function usePromotedCardsByOwner(): Map<string, string[]> {
  const { editorPanes, browserPanes } = usePaneMaps()
  const cards = useMemo(() => collect({ editorPanes, browserPanes }), [editorPanes, browserPanes])
  // Ids and owners only, flattened to a string. Neither can contain a NUL or a
  // SOH, so no two distinct groupings collapse to one signature — and the Map is
  // rebuilt from the signature rather than from `cards`, so this memo depends on
  // nothing else and needs no lint exception to say so.
  const signature = cards.map((c) => `${c.id}\u0000${c.sessionId}`).join('\u0001')

  return useMemo(() => {
    if (!signature) return NO_OWNERS
    const byOwner = new Map<string, string[]>()
    for (const entry of signature.split('\u0001')) {
      const [id, sessionId] = entry.split('\u0000')
      const owned = byOwner.get(sessionId)
      if (owned) owned.push(id)
      else byOwner.set(sessionId, [id])
    }
    return byOwner
  }, [signature])
}

/**
 * What one card is showing — for its tab, its dock pill and the focus stage.
 *
 * All three ask the same question, so all three ask it here, and the answer is
 * derived from the same `collect` everything else uses rather than a second
 * hand-rolled walk of the two maps.
 *
 * Note what is *not* here: no work in the selector body. Zustand runs selectors
 * on every state change, not every render, so a `new URL()` or a path split in
 * one runs on every terminal output tick for every mounted card. The derivation
 * happens once per pane change, inside `collect`'s memo.
 */
export function usePromotedCardSubject(cardId: string): PromotedCard | null {
  const cards = usePromotedCards()
  // No memo: `cards` is memoized, so `find` hands back the very object it holds
  // and that reference is stable for as long as the list is — which is what
  // keeps a caller's own memos and effects from re-running every render.
  return cards.find((c) => c.id === cardId) ?? null
}
