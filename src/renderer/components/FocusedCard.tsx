import { Minimize2 } from 'lucide-react'
import { motion } from 'framer-motion'
import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { isMac } from '../lib/platform'
import { useIsMobile } from '../hooks/useIsMobile'
import { CardSubjectIcon } from './CardSubjectIcon'
import { usePromotedCardSubject } from '../hooks/usePromotedCards'
import { PaneOwnerLabel } from './PaneCard'
import { FocusedNavHint } from './card/FocusedNavHint'
import { PromotedPaneCard } from './PromotedPaneCard'
import { Tooltip } from './Tooltip'
import { ICON_BUTTON, ICON_BUTTON_SIZE } from '../lib/icon-button'

/**
 * A popped-out file or tab, filling the stage.
 *
 * The point of the whole feature: asking to look at one file gets you that
 * file. Before this the focus stage only knew how to render a session, so
 * focusing a card focused its *owner* instead — the terminal, its panes, and
 * the card wedged in beside them. You asked for a file and got a workspace.
 *
 * It carries no session chrome. There is no agent, no status, no assigned task
 * and nothing to rename: a card's name is its filename or its host. What it
 * does carry is what a session carries here — who it belongs to, and the way
 * back out.
 */
export function FocusedCard({ cardId }: { cardId: string }) {
  const subject = usePromotedCardSubject(cardId)
  const { setFocused, setPreviewTerminal, isPreview } = useAppStore(
    useShallow((s) => ({
      setFocused: s.setFocusedTerminal,
      setPreviewTerminal: s.setPreviewTerminal,
      isPreview: s.previewTerminalId === cardId && s.focusedTerminalId !== cardId
    }))
  )
  const isMobile = useIsMobile()

  // Closed while focused. The stage empties on the next pass; bailing here
  // keeps it from drawing a header for a card that is gone.
  if (!subject) return null

  const contract = (): void => {
    if (isPreview) setPreviewTerminal(null)
    else setFocused(null)
  }

  return (
    <motion.div
      className={
        isMobile
          ? 'fixed inset-0 z-40 shadow-2xl flex flex-col overflow-hidden'
          : 'flex-1 flex flex-col min-h-0 overflow-hidden'
      }
      style={{
        background: 'var(--color-surface-raised)',
        ...(isMobile ? { paddingTop: 'var(--safe-top, 0px)' } : {})
      }}
    >
      {/* On macOS this header is the window's only drag region while the stage
          is filled — App drops the app titlebar then — so losing it would leave
          the window undraggable with no way back to the grid. */}
      <div
        className={`shrink-0 flex items-center gap-2 px-3 py-2 border-b border-white/[0.06]
                   ${isMac && !isMobile ? 'titlebar-drag' : 'titlebar-no-drag'}`}
        onDoubleClick={(e) => {
          if ((e.target as HTMLElement).closest('button, input, [role="button"]')) return
          contract()
        }}
        data-testid={`focused-card-${cardId}`}
      >
        <span className="titlebar-no-drag flex items-center">
          <CardSubjectIcon card={subject} size={16} />
        </span>
        <span className="text-[13px] font-medium text-gray-200 truncate">{subject.name}</span>
        {/* Interactive children of a drag region have to opt out of it. On
            macOS `-webkit-app-region: drag` swallows every click inside, so
            without this the collapse button and the branch switcher are simply
            dead — visibly present, and nothing happens when you press them. */}
        <span className="titlebar-no-drag flex items-center gap-1.5 min-w-0">
          <PaneOwnerLabel sessionId={subject.sessionId} />
        </span>
        <span className="flex-1" />
        {/* Cards are in the focus ring, so the counter has to be here too —
            without it the "3 / 7" simply vanished while a card was on the stage
            and reappeared on the next session, so the total looked wrong. */}
        <span className="titlebar-no-drag flex items-center shrink-0">
          <FocusedNavHint terminalId={cardId} />
        </span>
        <Tooltip label="Back to the grid">
          <button
            type="button"
            onClick={contract}
            className={`${ICON_BUTTON} titlebar-no-drag`}
            aria-label={`Collapse ${subject.name}`}
          >
            <Minimize2 size={ICON_BUTTON_SIZE} />
          </button>
        </Tooltip>
      </div>

      <div className="flex-1 min-h-0 flex flex-col">
        <PromotedPaneCard cardId={cardId} />
      </div>
    </motion.div>
  )
}
