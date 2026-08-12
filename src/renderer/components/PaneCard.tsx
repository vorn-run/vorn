import { forwardRef, type ReactNode } from 'react'
import { Maximize2, Minimize2, Minus, X } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { Tooltip } from './Tooltip'

// On touch devices, always show action buttons (no hover available). Evaluated
// lazily rather than at module load so importing this file stays safe in
// environments without matchMedia (jsdom tests, SSR).
function isTouchDevice(): boolean {
  return typeof window !== 'undefined' && window.matchMedia?.('(hover: none)').matches === true
}

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
}

/**
 * Chrome shared by the non-terminal panes a session owns (its file tree and its
 * open file). Mirrors `AgentCard`'s outer shape so grid drag/resize and the
 * flexible layout treat every pane the same, but carries its own header with
 * maximize / minimize / close.
 *
 * Maximize here is session-scoped: `GridView` reads `maximizedPaneId` and gives
 * the pane its owner session's whole footprint, leaving other sessions alone.
 */
export const PaneCard = forwardRef<HTMLDivElement, PaneCardProps>(function PaneCard(
  { paneId, title, subtitle, icon, onClose, children, isDragTarget, onDragStart, flexible },
  ref
) {
  const { isMaximized, setMaximizedPane, toggleMinimized } = useAppStore(
    useShallow((s) => ({
      isMaximized: s.maximizedPaneId === paneId,
      setMaximizedPane: s.setMaximizedPane,
      toggleMinimized: s.toggleMinimized
    }))
  )

  const handleDragStart = (e: React.PointerEvent): void => {
    onDragStart?.(paneId, e)
  }

  return (
    <div
      ref={ref}
      className={`group/card relative border overflow-hidden flex flex-col h-full transition-colors
                 ${
                   isDragTarget
                     ? 'card-drop-target border-blue-500/30 hover:border-white/[0.12]'
                     : 'border-white/[0.06] hover:border-white/[0.12]'
                 }
                 ${flexible ? '' : 'hover:z-10 focus-within:z-10'}`}
      style={{ background: '#1a1a1e' }}
    >
      <div
        className={`flex items-center gap-1.5 px-2 py-1 shrink-0 border-b border-white/[0.06]
                    ${onDragStart || flexible ? 'drag-handle cursor-grab active:cursor-grabbing' : ''}`}
        style={{ background: '#1e1e22' }}
        onPointerDown={onDragStart ? handleDragStart : undefined}
        onDoubleClick={() => setMaximizedPane(isMaximized ? null : paneId)}
      >
        {icon}
        <span className="text-[11px] text-gray-300 font-medium shrink-0">{title}</span>
        {subtitle && (
          <span
            className="text-[10px] text-gray-500 font-mono flex-1 min-w-0 truncate"
            title={subtitle}
            dir="rtl"
          >
            {subtitle}
          </span>
        )}
        {!subtitle && <span className="flex-1" />}

        <div
          className={`flex items-center gap-0.5 transition-opacity ${
            isTouchDevice() ? 'opacity-100' : 'opacity-0 group-hover/card:opacity-100'
          }`}
        >
          <Tooltip label="Minimize">
            <button
              onClick={() => toggleMinimized(paneId)}
              className="text-gray-500 hover:text-white p-0.5 rounded transition-colors"
              aria-label={`Minimize ${title}`}
            >
              <Minus size={12} strokeWidth={2} />
            </button>
          </Tooltip>
          <Tooltip label={isMaximized ? 'Restore' : 'Maximize'}>
            <button
              onClick={() => setMaximizedPane(isMaximized ? null : paneId)}
              className="text-gray-500 hover:text-white p-0.5 rounded transition-colors"
              aria-label={isMaximized ? `Restore ${title}` : `Maximize ${title}`}
            >
              {isMaximized ? <Minimize2 size={12} /> : <Maximize2 size={12} />}
            </button>
          </Tooltip>
          <Tooltip label="Close">
            <button
              onClick={onClose}
              className="text-gray-500 hover:text-white p-0.5 rounded transition-colors"
              aria-label={`Close ${title}`}
            >
              <X size={12} strokeWidth={2} />
            </button>
          </Tooltip>
        </div>
      </div>

      <div className="flex-1 min-h-0 flex flex-col" style={{ background: '#141416' }}>
        {children}
      </div>
    </div>
  )
})
