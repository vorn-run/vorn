import { useAppStore } from '../stores'
import { isPromotedCardId } from '../lib/pane-id'
import { FocusedTerminal } from './FocusedTerminal'
import { FocusedCard } from './FocusedCard'

/**
 * Whatever is focused, filling the stage.
 *
 * The focus stage takes the same ids the grid and the tab strip take, and like
 * them it learns what an id is in exactly one place — here. `GridCell` in
 * `GridView` is the same shape for the same reason.
 *
 * Without this the stage could only render a session, so focusing a card fell
 * back to focusing its owner and the card came along as a passenger. Which is
 * the whole difference between a card that belongs to a session and a card that
 * is a thing in its own right.
 */
export function FocusedStage() {
  const focusedId = useAppStore((s) => s.focusedTerminalId)
  const previewId = useAppStore((s) => s.previewTerminalId)
  const effectiveId = previewId ?? focusedId

  if (effectiveId && isPromotedCardId(effectiveId)) return <FocusedCard cardId={effectiveId} />
  return <FocusedTerminal />
}
