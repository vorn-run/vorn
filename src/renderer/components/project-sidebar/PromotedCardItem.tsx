import { CornerDownLeft, X } from 'lucide-react'
import { useAppStore } from '../../stores'
import { useShallow } from 'zustand/react/shallow'
import { Tooltip } from '../Tooltip'
import { CardSubjectIcon } from '../CardSubjectIcon'
import type { PromotedCard } from '../../hooks/usePromotedCards'

/**
 * One popped-out file or tab, listed under the session it came from.
 *
 * A card is a grid cell like a session, so the sidebar — which is how you reach
 * a session without hunting the grid for it — has to reach a card the same way.
 * Nested under its owner rather than in a list of its own, because the first
 * question about a popped-out file is always whose it is, and the indent answers
 * it without spending a word.
 */
export function PromotedCardItem({ card }: { card: PromotedCard }) {
  const {
    isSelected,
    layoutMode,
    setSelected,
    setFocusedTerminal,
    setActiveTabId,
    returnCard,
    closeCard
  } = useAppStore(
    useShallow((s) => ({
      // The same reading a session row uses. Off `selectedTerminalId` alone, a
      // card row stayed dark in tab mode while the session rows beside it lit
      // up — two kinds of row in one list answering different questions.
      isSelected:
        (s.config?.defaults?.layoutMode ?? 'grid') === 'tabs'
          ? s.activeTabId === card.id
          : s.focusedTerminalId === card.id || s.selectedTerminalId === card.id,
      layoutMode: s.config?.defaults?.layoutMode ?? 'grid',
      setSelected: s.setSelectedTerminal,
      setFocusedTerminal: s.setFocusedTerminal,
      setActiveTabId: s.setActiveTabId,
      returnCard: s.returnCardToSession,
      closeCard: s.closeCard
    }))
  )

  /**
   * Go to the card. The card itself — not the session it came from.
   *
   * This mirrors `SessionItem` exactly, on the card's own id, because that is
   * the point: a card is the same kind of thing as a session and is reached the
   * same way. An earlier version focused the *owner*, on the reasoning that the
   * card was drawn inside it — so clicking a file handed you the whole session
   * with the file wedged in beside its terminal, and offered to put the file
   * "back" into the session you were already looking at.
   */
  const reveal = (): void => {
    setSelected(card.id)
    if (layoutMode === 'tabs') {
      setActiveTabId(card.id)
      setFocusedTerminal(null)
    } else {
      setFocusedTerminal(card.id)
    }
  }

  const { name } = card

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={reveal}
      onKeyDown={(e) => {
        // Keys bubbling from a nested control are that control's, not the row's.
        if (e.target !== e.currentTarget) return
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          reveal()
        }
      }}
      title={card.subject}
      // Metrics copied from the session row, not approximated: same padding,
      // same text size, same gap, same left edge. A card is a grid cell exactly
      // as a session is, and a row that sits half an indent off reads as a
      // different kind of thing rather than a sibling.
      className={`group/card relative w-full text-left px-2 py-1 rounded-md text-[12px]
                  flex items-center gap-2 min-w-0 cursor-default select-none transition-colors ${
                    isSelected
                      ? 'text-white'
                      : 'text-gray-500 hover:text-white hover:bg-white/[0.04]'
                  }`}
    >
      {isSelected && <span className="absolute left-0 top-1 bottom-1 w-px bg-white rounded-full" />}
      {/* The file's own icon, the one the tree gives it — a generic page glyph
          would make every card look alike in a list whose whole job is telling
          them apart. Sized to the session row's agent icon. */}
      <CardSubjectIcon card={card} />
      <span className="truncate flex-1">{name}</span>

      <Tooltip label="Put back in its session card" position="right">
        <button
          type="button"
          aria-label={`Put ${name} back in its session card`}
          onClick={(e) => {
            e.stopPropagation()
            returnCard(card.id)
          }}
          className="opacity-0 group-hover/card:opacity-100 focus:opacity-100 text-gray-500
                     hover:text-gray-200 p-0.5 rounded hover:bg-white/[0.08] transition-colors shrink-0"
        >
          <CornerDownLeft size={11} strokeWidth={2} />
        </button>
      </Tooltip>
      <Tooltip label="Close" position="right">
        <button
          type="button"
          aria-label={`Close ${name}`}
          onClick={(e) => {
            e.stopPropagation()
            closeCard(card.id)
          }}
          className="opacity-0 group-hover/card:opacity-100 focus:opacity-100 text-gray-500
                     hover:text-red-400 p-0.5 rounded hover:bg-white/[0.08] transition-colors shrink-0"
        >
          <X size={11} strokeWidth={2} />
        </button>
      </Tooltip>
    </div>
  )
}
