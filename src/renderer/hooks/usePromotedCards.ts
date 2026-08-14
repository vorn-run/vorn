import { useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { isPromotedPane, type AppStore } from '../stores/types'

/**
 * What a promoted card is showing, and whose it is.
 *
 * There is no list of promoted cards to keep in step with anything: a card *is*
 * a pane entry whose key is not its owner's id. Reading it back out of the two
 * pane collections means the card cannot outlive the pane it draws, and closing
 * the pane is what closes the card.
 */
export interface PromotedCard {
  /** The card's pane id — `card:<sessionId>:<n>`. */
  id: string
  kind: 'editor' | 'browser'
  sessionId: string
  /** Absolute file path, or the page's url. What the card is showing. */
  subject: string
}

function collect(state: Pick<AppStore, 'editorPanes' | 'browserPanes'>): PromotedCard[] {
  const cards: PromotedCard[] = []
  for (const [id, pane] of state.editorPanes) {
    if (isPromotedPane(id, pane)) {
      cards.push({ id, kind: 'editor', sessionId: pane.sessionId, subject: pane.filePath })
    }
  }
  for (const [id, pane] of state.browserPanes) {
    if (isPromotedPane(id, pane)) {
      const url = pane.tabs[pane.activeTab] ?? pane.tabs[0] ?? ''
      cards.push({ id, kind: 'browser', sessionId: pane.sessionId, subject: url })
    }
  }
  return cards
}

/** Every promoted card, in no particular order. */
export function usePromotedCards(): PromotedCard[] {
  const { editorPanes, browserPanes } = useAppStore(
    useShallow((s) => ({ editorPanes: s.editorPanes, browserPanes: s.browserPanes }))
  )
  return useMemo(() => collect({ editorPanes, browserPanes }), [editorPanes, browserPanes])
}

/** The cards `sessionId` popped out, in the order they were popped. */
export function usePromotedCardsFor(sessionId: string): PromotedCard[] {
  const all = usePromotedCards()
  return useMemo(() => all.filter((c) => c.sessionId === sessionId), [all, sessionId])
}

/**
 * This pane's owner if it is a promoted card, otherwise null.
 *
 * Doubles as the "is this a card" test, because the two questions are always
 * asked together — a card draws differently *and* has to say whose it is.
 */
export function usePromotedOwner(paneId: string): string | null {
  return useAppStore((s) => {
    const editor = s.editorPanes.get(paneId)
    if (editor && isPromotedPane(paneId, editor)) return editor.sessionId
    const browser = s.browserPanes.get(paneId)
    if (browser && isPromotedPane(paneId, browser)) return browser.sessionId
    return null
  })
}

/** Ids of every promoted card, for the store-free paths that only need ids. */
export function promotedCardIds(state: Pick<AppStore, 'editorPanes' | 'browserPanes'>): string[] {
  return collect(state).map((c) => c.id)
}
