import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../../stores'
import { AgentStatusIcon } from '../AgentStatusIcon'
import { InlineRename } from '../InlineRename'
import { CardActionCluster, type CardVariant } from './CardActionCluster'
import { FocusedNavHint } from './FocusedNavHint'
import { AppNavCluster } from '../AppNavCluster'
import { useFocusedTitlebar } from '../../hooks/useFocusedTitlebar'
import { getDisplayName } from '../../lib/terminal-display'
import { Pencil } from 'lucide-react'
import { MOD, TRAFFIC_LIGHT_PAD_PX } from '../../lib/platform'
import { toast } from '../Toast'

interface Props {
  terminalId: string
  variant: CardVariant
  index?: number
  draggable?: boolean
  onDragStart?: (terminalId: string, e: React.PointerEvent) => void
  onDoubleClick?: () => void
  /** Force the action cluster to be visible (e.g. on touch devices where hover is unreliable). */
  revealActions?: boolean
  /** Visually de-emphasise this header when another card is the active selection. */
  dimmed?: boolean
}

export function CardHeader({
  terminalId,
  variant,
  index,
  draggable,
  onDragStart,
  onDoubleClick,
  revealActions,
  dimmed
}: Props) {
  const { terminal, isRenaming, setRenamingTerminalId, renameTerminal } = useAppStore(
    useShallow((s) => ({
      terminal: s.terminals.get(terminalId),
      isRenaming: s.renamingTerminalId === terminalId,
      setRenamingTerminalId: s.setRenamingTerminalId,
      renameTerminal: s.renameTerminal
    }))
  )
  // Only the focused variant is ever the window's titlebar; the grid's mini
  // headers sit inside a window that still has one of its own.
  const { needsTrafficLightPad, showsAppNav } = useFocusedTitlebar()
  const isTitlebar = variant === 'focused'

  if (!terminal) return null

  const displayName = getDisplayName(terminal.session)

  const alwaysVisible = variant === 'focused' || revealActions === true
  const revealClass = alwaysVisible
    ? 'opacity-100'
    : 'opacity-0 group-hover/card:opacity-100 focus-within:opacity-100 transition-opacity'

  const dragHandleClass = draggable ? `drag-handle${onDragStart ? ' cursor-grab' : ''}` : ''

  return (
    // The card is one object, so its header, body and status bar share a ground
    // and the hairline below does the dividing. Two fills made the header read
    // as a separate band stuck to the top of the card rather than part of it.
    <div
      className={`flex items-center gap-2 px-3 py-2.5 border-b border-white/[0.04] shrink-0
                  transition-opacity duration-200 ease-out
                  ${dimmed ? 'opacity-60 group-hover/card:opacity-100' : 'opacity-100'}`}
      data-testid={isTitlebar ? 'focused-session-header' : undefined}
      style={{
        background: 'var(--color-surface-raised)',
        // Standing in for the titlebar means clearing the traffic lights, which
        // otherwise sit straight on top of the session's name.
        ...(isTitlebar && needsTrafficLightPad ? { paddingLeft: `${TRAFFIC_LIGHT_PAD_PX}px` } : {})
      }}
    >
      {isTitlebar && showsAppNav && (
        <>
          <AppNavCluster />
          <div className="w-px h-4 bg-white/[0.06]" />
        </>
      )}

      <div
        className={`flex-1 min-w-0 flex items-center gap-2 ${dragHandleClass}`}
        onDoubleClick={onDoubleClick}
        onPointerDown={onDragStart ? (e) => onDragStart(terminalId, e) : undefined}
      >
        <AgentStatusIcon
          agentType={terminal.session.agentType}
          status={terminal.status}
          size={18}
        />
        <div className="titlebar-no-drag min-w-0 flex items-center gap-1 group/rename">
          {isRenaming ? (
            <InlineRename
              value={displayName}
              onCommit={(name) => {
                renameTerminal(terminalId, name)
                setRenamingTerminalId(null)
                toast.success(`Renamed to "${name}"`)
              }}
              onCancel={() => setRenamingTerminalId(null)}
              className="text-[13px] font-medium w-full"
            />
          ) : (
            <>
              <span
                className="text-[13px] font-medium text-gray-300 truncate cursor-text"
                title={displayName}
              >
                {displayName}
              </span>
              <button
                type="button"
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation()
                  setRenamingTerminalId(terminalId)
                }}
                className="titlebar-no-drag opacity-0 group-hover/rename:opacity-100 text-gray-500 hover:text-gray-300 transition-opacity shrink-0"
                aria-label="Rename session"
              >
                <Pencil size={10} />
              </button>
            </>
          )}
        </div>
      </div>

      {variant === 'focused' && (
        <div className="titlebar-no-drag">
          <FocusedNavHint terminalId={terminalId} />
        </div>
      )}

      {/* ⌘N badge and action cluster share this slot so the badge sits flush right when idle. */}
      <div className="relative flex items-center shrink-0">
        {variant === 'mini' && typeof index === 'number' && index < 9 && !alwaysVisible && (
          <span
            className="absolute right-0 top-1/2 -translate-y-1/2 pointer-events-none
                       px-1.5 py-0.5 text-[10px] font-mono text-gray-600
                       bg-white/[0.04] border border-white/[0.06] rounded
                       leading-none opacity-100 group-hover/card:opacity-0 transition-opacity"
          >
            {MOD}
            {index + 1}
          </span>
        )}
        <div className={`titlebar-no-drag ${revealClass}`}>
          <CardActionCluster terminalId={terminalId} variant={variant} />
        </div>
      </div>
    </div>
  )
}
