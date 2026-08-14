import { useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { displayHost } from '../lib/browser-url'
import { isPromotedCardId } from '../lib/pane-id'
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

function collect(state: PaneMaps, owner?: string): PromotedCard[] {
  const cards: PromotedCard[] = []
  for (const [id, pane] of state.editorPanes) {
    if (!isPromotedPane(id, pane)) continue
    if (owner !== undefined && pane.sessionId !== owner) continue
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
    if (owner !== undefined && pane.sessionId !== owner) continue
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

/** The cards `sessionId` popped out. */
export function usePromotedCardsFor(sessionId: string): PromotedCard[] {
  const { editorPanes, browserPanes } = usePaneMaps()
  return useMemo(
    () => collect({ editorPanes, browserPanes }, sessionId),
    [editorPanes, browserPanes, sessionId]
  )
}

/**
 * Card ids grouped by the session they came from.
 *
 * The one place that answers "which cards sit beside which session" — the
 * ordering contract the grid and the tab strip are both built on. Written twice,
 * they would drift the moment either grew a sort or a filter, and the two strips
 * would disagree about where a card belongs.
 *
 * Grouped on `pane.sessionId`, the authoritative owner, rather than by parsing
 * it back out of the card's id.
 */
export function promotedCardsByOwner(state: PaneMaps): Map<string, string[]> {
  const cards = collect(state)
  if (cards.length === 0) return NO_OWNERS
  const byOwner = new Map<string, string[]>()
  for (const card of cards) {
    const owned = byOwner.get(card.sessionId)
    if (owned) owned.push(card.id)
    else byOwner.set(card.sessionId, [card.id])
  }
  return byOwner
}

export function usePromotedCardsByOwner(): Map<string, string[]> {
  const { editorPanes, browserPanes } = usePaneMaps()
  return useMemo(
    () => promotedCardsByOwner({ editorPanes, browserPanes }),
    [editorPanes, browserPanes]
  )
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
  // No memo needed: `cards` is itself memoized, so `find` hands back the very
  // object it holds, and that reference is stable for as long as the list is.
  // A session id can never name a card, so the scan is skipped outright for the
  // far more common case of a plain terminal pill or tab.
  return isPromotedCardId(cardId) ? (cards.find((c) => c.id === cardId) ?? null) : null
}
