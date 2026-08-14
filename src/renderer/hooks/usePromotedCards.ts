import { useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { displayHost } from '../lib/browser-url'
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

/** What one card is showing, named the way a person would name it. */
export interface PromotedCardSubject {
  kind: 'editor' | 'browser'
  /** Filename, or the page's host. */
  name: string
  sessionId: string
}

/**
 * What a card is showing — for the tab, the dock pill and the focus stage.
 *
 * All three ask the same question, so all three ask it here. That is worth a
 * hook on its own because the obvious way to write it is a trap: a selector
 * that builds `{ kind, name }` inline returns a new object on every call, and
 * zustand compares snapshots by reference — so the component re-renders, the
 * selector runs again, and React aborts with "maximum update depth exceeded"
 * the moment such a component mounts. This selects flat values and memoizes the
 * object once, which is what makes the reference stable.
 */
export function usePromotedCardSubject(cardId: string): PromotedCardSubject | null {
  const { kind, name, sessionId } = useAppStore(
    useShallow((s) => {
      const editor = s.editorPanes.get(cardId)
      if (editor && isPromotedPane(cardId, editor)) {
        return {
          kind: 'editor' as const,
          name: editor.filePath.split(/[/\\]/).pop() ?? '',
          sessionId: editor.sessionId
        }
      }
      const browser = s.browserPanes.get(cardId)
      if (browser && isPromotedPane(cardId, browser)) {
        return {
          kind: 'browser' as const,
          name: displayHost(browser.tabs[browser.activeTab] ?? browser.tabs[0] ?? ''),
          sessionId: browser.sessionId
        }
      }
      return { kind: null, name: '', sessionId: null }
    })
  )

  return useMemo(
    () => (kind && sessionId ? { kind, name, sessionId } : null),
    [kind, name, sessionId]
  )
}
