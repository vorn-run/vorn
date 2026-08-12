import { FolderOpen, Globe, Maximize2, Minimize2, Minus, MoreHorizontal, X } from 'lucide-react'
import { useState, useRef } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { ConfirmPopover } from '../ConfirmPopover'
import { CardContextMenu } from '../CardContextMenu'
import { closeTerminalSession } from '../../lib/terminal-close'
import { toast } from '../Toast'
import { getDisplayName } from '../../lib/terminal-display'
import { MOD } from '../../lib/platform'

export type CardVariant = 'mini' | 'focused'

interface Props {
  terminalId: string
  variant: CardVariant
}

export function CardActionCluster({ terminalId, variant }: Props) {
  const { terminal, setFocused, toggleMinimized, toggleFilesPane, toggleBrowserPane } = useAppStore(
    useShallow((s) => ({
      terminal: s.terminals.get(terminalId),
      setFocused: s.setFocusedTerminal,
      toggleMinimized: s.toggleMinimized,
      toggleFilesPane: s.toggleFilesPane,
      toggleBrowserPane: s.toggleBrowserPane
    }))
  )
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null)
  const moreRef = useRef<HTMLButtonElement>(null)

  if (!terminal) return null

  const handleBrowseFiles = (e: React.MouseEvent): void => {
    e.stopPropagation()
    toggleFilesPane(terminalId)
  }

  const handleOpenBrowser = (e: React.MouseEvent): void => {
    e.stopPropagation()
    toggleBrowserPane(terminalId)
  }

  const handleMinimize = (e: React.MouseEvent): void => {
    e.stopPropagation()
    toggleMinimized(terminalId)
  }

  const handleExpand = (e: React.MouseEvent): void => {
    e.stopPropagation()
    setFocused(terminalId)
  }

  const handleCollapse = (e: React.MouseEvent): void => {
    e.stopPropagation()
    setFocused(null)
  }

  const handleClose = async (): Promise<void> => {
    const name = getDisplayName(terminal.session)
    await closeTerminalSession(terminalId)
    toast.success(`Session "${name}" closed`)
  }

  const handleMore = (e: React.MouseEvent): void => {
    e.stopPropagation()
    if (contextMenu) {
      setContextMenu(null)
      return
    }
    const rect = moreRef.current?.getBoundingClientRect()
    if (rect) {
      setContextMenu({ x: rect.left, y: rect.bottom + 4 })
    }
  }

  const showMinimize = variant === 'mini'
  const isFocused = variant === 'focused'
  // In focused mode the header sits at the top of the window, so top-positioned
  // tooltips get clipped off-screen. Drop them below the buttons instead.
  const tooltipPos = isFocused ? 'bottom' : 'top'

  const btn = 'p-1 rounded text-gray-500 hover:text-white hover:bg-white/[0.08] transition-colors'

  return (
    <div className="flex items-center gap-0.5 shrink-0">
      <Tooltip label="More actions" position={tooltipPos}>
        <button
          ref={moreRef}
          type="button"
          onClick={handleMore}
          onPointerDown={(e) => e.stopPropagation()}
          className={btn}
          aria-label="More actions"
        >
          <MoreHorizontal size={14} strokeWidth={2} />
        </button>
      </Tooltip>

      <Tooltip label="Browse files" position={tooltipPos}>
        <button
          type="button"
          onClick={handleBrowseFiles}
          onPointerDown={(e) => e.stopPropagation()}
          className={btn}
          aria-label="Browse files"
        >
          <FolderOpen size={14} strokeWidth={2} />
        </button>
      </Tooltip>

      <Tooltip label="Open browser" position={tooltipPos}>
        <button
          type="button"
          onClick={handleOpenBrowser}
          onPointerDown={(e) => e.stopPropagation()}
          className={btn}
          aria-label="Open browser"
        >
          <Globe size={14} strokeWidth={2} />
        </button>
      </Tooltip>

      {showMinimize && (
        <Tooltip label="Minimize" position={tooltipPos}>
          <button
            type="button"
            onClick={handleMinimize}
            onPointerDown={(e) => e.stopPropagation()}
            className={btn}
            aria-label="Minimize session"
          >
            <Minus size={14} strokeWidth={2} />
          </button>
        </Tooltip>
      )}

      <Tooltip
        label={isFocused ? 'Collapse to grid' : 'Expand'}
        shortcut={isFocused ? `${MOD}W` : `${MOD}O`}
        position={tooltipPos}
      >
        <button
          type="button"
          onClick={isFocused ? handleCollapse : handleExpand}
          onPointerDown={(e) => e.stopPropagation()}
          className={btn}
          aria-label={isFocused ? 'Collapse session' : 'Expand session'}
        >
          {isFocused ? (
            <Minimize2 size={14} strokeWidth={2} />
          ) : (
            <Maximize2 size={14} strokeWidth={2} />
          )}
        </button>
      </Tooltip>

      <ConfirmPopover message="Close this session?" confirmLabel="Close" onConfirm={handleClose}>
        <Tooltip label="Close session" position={tooltipPos}>
          <button
            type="button"
            onPointerDown={(e) => e.stopPropagation()}
            className="p-1 rounded text-gray-500 hover:text-red-400 hover:bg-white/[0.08] transition-colors"
            aria-label="Close session"
          >
            <X size={14} strokeWidth={2} />
          </button>
        </Tooltip>
      </ConfirmPopover>

      {contextMenu && (
        <CardContextMenu
          terminalId={terminalId}
          position={contextMenu}
          onClose={() => setContextMenu(null)}
        />
      )}
    </div>
  )
}
