import { forwardRef, type ReactNode } from 'react'
import { CornerDownLeft, Maximize2, Minimize2, Minus, SquareArrowOutUpRight, X } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { Tooltip } from './Tooltip'
import { PANE_SURFACE } from '../lib/pane-surface'
import { ICON_BUTTON, ICON_BUTTON_SIZE } from '../lib/icon-button'
import { paneOwnerId } from '../lib/pane-id'
import { BranchChip } from './card/BranchChip'

export interface PaneCardProps {
  /** This pane's id — `files:<sessionId>` or `editor:<sessionId>`. */
  paneId: string
  title: string
  onClose: () => void
  children: ReactNode
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
  /**
   * Drop the card's own header row. For panes like the browser, whose tab strip
   * is already a title bar — two stacked bars cost vertical space and read as
   * chrome-on-chrome. Such a pane renders `PaneControls` inside its own bar.
   */
  headerless?: boolean
}

/**
 * Controls for one pane: where it sits, how big it is, and closing it.
 *
 * Split out of the header so a headerless pane can seat the same controls in a
 * bar of its own making, rather than growing a second one.
 *
 * The set changes with placement, because the useful moves do. A pane inside a
 * session card can be promoted out of it or maximized over its owner; a
 * promoted pane is already a cell of its own, so it minimizes to the dock and
 * returns to its card instead. Close means the same thing either way — the pane
 * is gone, not merely stowed.
 *
 * Minimize appears only once promoted, and that is not an oversight left over.
 * A docked pane had nowhere to go when minimized: the dock surfaces grid cells,
 * and a pane inside a card is not one, so the button silently discarded it.
 * Promotion makes the pane a cell, which is exactly what gives minimize
 * somewhere to land.
 */
export function PaneControls({
  paneId,
  title,
  onClose,
  className = ''
}: {
  paneId: string
  title: string
  onClose: () => void
  className?: string
}): ReactNode {
  const { isMaximized, isPromoted, setMaximizedPane, promotePane, returnPane, toggleMinimized } =
    useAppStore(
      useShallow((s) => ({
        isMaximized: s.maximizedPaneId === paneId,
        isPromoted: s.promotedPanes.has(paneId),
        setMaximizedPane: s.setMaximizedPane,
        promotePane: s.promotePane,
        returnPane: s.returnPaneToCard,
        toggleMinimized: s.toggleMinimized
      }))
    )

  // Always visible, never hover-revealed: hidden until the pointer arrives, a
  // panel reads as having no controls at all.
  return (
    <div className={`flex items-center gap-0.5 ${className}`}>
      {/* Three of the four pane kinds are headerless, seating these controls in
          a bar of their own. Hanging ownership off the controls rather than off
          the header is therefore the one placement every kind inherits. */}
      {isPromoted && <PaneOwnerLabel paneId={paneId} />}
      {isPromoted ? (
        <>
          <Tooltip label="Minimize">
            <button
              onClick={() => toggleMinimized(paneId)}
              className={ICON_BUTTON}
              aria-label={`Minimize ${title}`}
            >
              <Minus size={ICON_BUTTON_SIZE} />
            </button>
          </Tooltip>
          <Tooltip label="Put back in its session card">
            <button
              onClick={() => returnPane(paneId)}
              className={ICON_BUTTON}
              aria-label={`Put ${title} back in its session card`}
            >
              <CornerDownLeft size={ICON_BUTTON_SIZE} />
            </button>
          </Tooltip>
        </>
      ) : (
        <>
          <Tooltip label="Open as its own card">
            <button
              onClick={() => promotePane(paneId)}
              className={ICON_BUTTON}
              aria-label={`Open ${title} as its own card`}
            >
              <SquareArrowOutUpRight size={ICON_BUTTON_SIZE} />
            </button>
          </Tooltip>
          <Tooltip label={isMaximized ? 'Restore' : 'Maximize'}>
            <button
              onClick={() => setMaximizedPane(isMaximized ? null : paneId)}
              className={ICON_BUTTON}
              aria-label={isMaximized ? `Restore ${title}` : `Maximize ${title}`}
            >
              {isMaximized ? (
                <Minimize2 size={ICON_BUTTON_SIZE} />
              ) : (
                <Maximize2 size={ICON_BUTTON_SIZE} />
              )}
            </button>
          </Tooltip>
        </>
      )}
      <Tooltip label="Close">
        <button onClick={onClose} className={ICON_BUTTON} aria-label={`Close ${title}`}>
          <X size={ICON_BUTTON_SIZE} strokeWidth={2} />
        </button>
      </Tooltip>
    </div>
  )
}

/**
 * Chrome shared by the non-terminal panes a session owns (its file tree and its
 * open file). Mirrors `AgentCard`'s outer shape so grid drag/resize and the
 * flexible layout treat every pane the same, but carries its own header with
 * maximize / close.
 *
 * Maximize here is session-scoped: `GridView` reads `maximizedPaneId` and gives
 * the pane its owner session's whole footprint, leaving other sessions alone.
 */
export const PaneCard = forwardRef<HTMLDivElement, PaneCardProps>(function PaneCard(
  { paneId, title, onClose, children, isDragTarget, onDragStart, flexible, headerless },
  ref
) {
  const { isMaximized, isPromoted, isSelected, setMaximizedPane, setSelected } = useAppStore(
    useShallow((s) => ({
      isMaximized: s.maximizedPaneId === paneId,
      isPromoted: s.promotedPanes.has(paneId),
      isSelected: s.selectedTerminalId === paneId,
      setMaximizedPane: s.setMaximizedPane,
      setSelected: s.setSelectedTerminal
    }))
  )

  const handleDragStart = (e: React.PointerEvent): void => {
    onDragStart?.(paneId, e)
  }

  return (
    <div
      ref={ref}
      // Inside a card: square and borderless, filling it. The step down to
      // PANE_SURFACE is the whole separation — see that module for why.
      //
      // Promoted, it is a grid cell among session cards, so it wears what they
      // wear — including selection, since the keyboard shortcuts that walk the
      // grid by position now land here too, and a jump that highlights nothing
      // reads as a jump that did nothing.
      className={`relative overflow-hidden flex flex-col h-full
                 transition-shadow
                 ${
                   isPromoted
                     ? `border transition-colors ${
                         isSelected
                           ? 'border-white/40'
                           : 'border-white/[0.06] hover:border-white/[0.12]'
                       }`
                     : ''
                 }
                 ${isDragTarget ? 'card-drop-target' : ''}
                 ${flexible ? '' : 'hover:z-10 focus-within:z-10'}`}
      style={{ background: isPromoted ? 'var(--color-surface-raised)' : PANE_SURFACE }}
      onPointerDown={isPromoted && !isSelected ? () => setSelected(paneId) : undefined}
    >
      {!headerless && (
        <div
          className={`flex items-center gap-1.5 px-2 py-1 shrink-0
                    ${isPromoted ? 'border-b border-white/[0.04]' : ''}
                    ${onDragStart || flexible ? 'drag-handle cursor-grab active:cursor-grabbing' : ''}`}
          onPointerDown={onDragStart ? handleDragStart : undefined}
          onDoubleClick={() => setMaximizedPane(isMaximized ? null : paneId)}
          data-testid={`pane-header-${paneId}`}
        >
          <span className="text-[12px] text-gray-300 font-medium shrink-0">{title}</span>
          <span className="flex-1" />
          <PaneControls paneId={paneId} title={title} onClose={onClose} />
        </div>
      )}

      <div className="flex-1 min-h-0 flex flex-col" style={{ background: PANE_SURFACE }}>
        {children}
      </div>
    </div>
  )
})

/**
 * Which session a promoted pane came out of.
 *
 * Inside a card the question never arises — the pane is visibly part of
 * something. Out in the grid beside a dozen other cards it does, and the failure
 * is quiet: the right file tree from the wrong worktree looks exactly like the
 * right one. So this says project, then hands worktree and branch to the same
 * `BranchChip` a session card wears, which means the switcher works here too.
 *
 * No accent. Bronzo marks work blocked on the person; a worktree name is
 * information, not a request.
 */
function PaneOwnerLabel({ paneId }: { paneId: string }): ReactNode {
  const sessionId = paneOwnerId(paneId)
  const projectName = useAppStore((s) => s.terminals.get(sessionId)?.session.projectName)

  if (!projectName) return null

  return (
    <>
      <span className="text-ink-ghost shrink-0">·</span>
      <span className="text-[11px] text-ink-secondary truncate max-w-[140px]">{projectName}</span>
      <BranchChip terminalId={sessionId} />
    </>
  )
}
