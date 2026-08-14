import { CornerDownLeft, FileText, Globe, X } from 'lucide-react'
import { useAppStore } from '../../stores'
import { useShallow } from 'zustand/react/shallow'
import { Tooltip } from '../Tooltip'
import { displayHost } from '../../lib/browser-url'
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
  const { isSelected, setSelected, returnCard, closeEditor, closeBrowser } = useAppStore(
    useShallow((s) => ({
      isSelected: s.selectedTerminalId === card.id,
      setSelected: s.setSelectedTerminal,
      returnCard: s.returnCardToSession,
      closeEditor: s.closeEditorPane,
      closeBrowser: s.closeBrowserPane
    }))
  )

  const Icon = card.kind === 'browser' ? Globe : FileText
  const name =
    card.kind === 'browser' ? displayHost(card.subject) : (card.subject.split(/[/\\]/).pop() ?? '')

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => setSelected(card.id)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          setSelected(card.id)
        }
      }}
      title={card.subject}
      className={`group/card relative w-full text-left pl-6 pr-2 py-1 rounded-md text-[12px]
                  flex items-center gap-2 min-w-0 cursor-default select-none transition-colors ${
                    isSelected
                      ? 'text-white'
                      : 'text-gray-500 hover:text-white hover:bg-white/[0.04]'
                  }`}
    >
      {isSelected && <span className="absolute left-0 top-1 bottom-1 w-px bg-white rounded-full" />}
      <Icon size={12} strokeWidth={1.5} className="shrink-0 text-ink-faint" />
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
            if (card.kind === 'browser') closeBrowser(card.id)
            else closeEditor(card.id)
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
