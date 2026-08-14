import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { useIsMobile } from './useIsMobile'
import { ownsPromotedCard, selectPaneFlags } from '../stores/ui-slice'

/**
 * Whether the current layout gives a popped-out card a cell of its own.
 *
 * The grid draws one as a cell beside the sessions; the tab strip draws one as
 * a tab beside them. Focused mode, hover preview and mobile each show a single
 * session and have nowhere to put a card — so there, a session's cards fall
 * back into its pane column.
 *
 * The item never moves in the store; only where it is drawn changes. It must be
 * drawn in exactly one place under either answer, and the false direction is the
 * dangerous one: a browser card rendered as both a tab and a column pane is two
 * guests loading the same url, each with its own scroll position.
 */
export function useCardsDrawnAsCells(): boolean {
  const { focusedId, previewId } = useAppStore(
    useShallow((s) => ({
      focusedId: s.focusedTerminalId,
      previewId: s.previewTerminalId
    }))
  )
  const isMobile = useIsMobile()

  return !focusedId && !previewId && !isMobile
}

/**
 * Whether `sessionId` has anything to put in a pane column right now.
 *
 * Two questions at once, because asking only the first gets it wrong in a way
 * that is easy to miss: a session's own panes always belong in its column, but
 * its popped-out cards belong there only where the layout gives them no cell of
 * their own. Answering "yes" for a card in the grid reserves a column that then
 * renders nothing — the terminal squeezed to its split ratio, a band of dead
 * space beside it, and no sign of what is holding the space open.
 */
export function useSessionHasPaneColumn(sessionId: string | null): boolean {
  const cardsHaveCells = useCardsDrawnAsCells()
  return useAppStore((s) => {
    if (!sessionId) return false
    if (selectPaneFlags(s, sessionId).any) return true
    return !cardsHaveCells && ownsPromotedCard(s, sessionId)
  })
}
