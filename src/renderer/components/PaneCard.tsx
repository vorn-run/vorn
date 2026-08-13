import { forwardRef, type ReactNode } from 'react'
import { Maximize2, Minimize2, X } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { Tooltip } from './Tooltip'
import { PANE_SURFACE } from '../lib/pane-surface'
import { ICON_BUTTON, ICON_BUTTON_SIZE } from '../lib/icon-button'

export interface PaneCardProps {
  /** This pane's id — `files:<sessionId>` or `editor:<sessionId>`. */
  paneId: string
  title: string
  /** Secondary line under the title, e.g. a relative file path. */
  subtitle?: string
  icon?: ReactNode
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
  /** Card background. Defaults to the shared pane surface. */
  background?: string
}

/**
 * Maximize / close for one pane.
 *
 * Split out of the header so a headerless pane can seat the same controls in a
 * bar of its own making, rather than growing a second one.
 *
 * There is deliberately no minimize here. A minimized pane had nowhere to go —
 * the dock only surfaces sessions, and expanded mode ignores the state
 * entirely — so the button silently discarded the pane.
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
  const { isMaximized, setMaximizedPane } = useAppStore(
    useShallow((s) => ({
      isMaximized: s.maximizedPaneId === paneId,
      setMaximizedPane: s.setMaximizedPane
    }))
  )

  // Always visible, never hover-revealed: hidden until the pointer arrives, a
  // panel reads as having no controls at all.
  return (
    <div className={`flex items-center gap-0.5 ${className}`}>
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
  {
    paneId,
    title,
    subtitle,
    icon,
    onClose,
    children,
    isDragTarget,
    onDragStart,
    flexible,
    headerless,
    background
  },
  ref
) {
  const { isMaximized, setMaximizedPane } = useAppStore(
    useShallow((s) => ({
      isMaximized: s.maximizedPaneId === paneId,
      setMaximizedPane: s.setMaximizedPane
    }))
  )

  const handleDragStart = (e: React.PointerEvent): void => {
    onDragStart?.(paneId, e)
  }

  return (
    <div
      ref={ref}
      // Square and borderless, filling the card. The step down to PANE_SURFACE
      // is the whole separation — see that module for why.
      className={`group/card relative overflow-hidden flex flex-col h-full
                 transition-shadow
                 ${isDragTarget ? 'card-drop-target' : ''}
                 ${flexible ? '' : 'hover:z-10 focus-within:z-10'}`}
      style={{ background: background ?? PANE_SURFACE }}
    >
      {!headerless && (
        <div
          className={`flex items-center gap-1.5 px-2 py-1 shrink-0
                    ${onDragStart || flexible ? 'drag-handle cursor-grab active:cursor-grabbing' : ''}`}
          onPointerDown={onDragStart ? handleDragStart : undefined}
          onDoubleClick={() => setMaximizedPane(isMaximized ? null : paneId)}
        >
          {icon}
          <span className="text-[12px] text-gray-300 font-medium shrink-0">{title}</span>
          {subtitle && (
            <span
              className="text-[11px] text-gray-500 font-mono flex-1 min-w-0 truncate"
              title={subtitle}
              dir="rtl"
            >
              {subtitle}
            </span>
          )}
          {!subtitle && <span className="flex-1" />}

          <PaneControls paneId={paneId} title={title} onClose={onClose} />
        </div>
      )}

      <div
        className="flex-1 min-h-0 flex flex-col"
        style={{ background: background ?? PANE_SURFACE }}
      >
        {children}
      </div>
    </div>
  )
})
