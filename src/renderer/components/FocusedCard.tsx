import { Globe, Minimize2 } from 'lucide-react'
import { motion } from 'framer-motion'
import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { isPromotedPane } from '../stores/types'
import { displayHost } from '../lib/browser-url'
import { isMac } from '../lib/platform'
import { useIsMobile } from '../hooks/useIsMobile'
import { FileTypeIcon } from './file-icons'
import { PaneOwnerLabel } from './PaneCard'
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
  // Flat values only. A nested object built inside the selector is a new
  // reference every call, which `useShallow` compares as changed — the render
  // then re-selects, and the two chase each other until React gives up.
  const { kind, name, sessionId, setFocused, setPreviewTerminal, isPreview } = useAppStore(
    useShallow((s) => {
      const editor = s.editorPanes.get(cardId)
      const browser = s.browserPanes.get(cardId)
      const promotedEditor = editor && isPromotedPane(cardId, editor) ? editor : null
      const promotedBrowser = browser && isPromotedPane(cardId, browser) ? browser : null
      return {
        kind: promotedEditor ? ('editor' as const) : promotedBrowser ? ('browser' as const) : null,
        name: promotedEditor
          ? (promotedEditor.filePath.split(/[/\\]/).pop() ?? '')
          : promotedBrowser
            ? displayHost(
                promotedBrowser.tabs[promotedBrowser.activeTab] ?? promotedBrowser.tabs[0] ?? ''
              )
            : '',
        sessionId: promotedEditor?.sessionId ?? promotedBrowser?.sessionId ?? null,
        setFocused: s.setFocusedTerminal,
        setPreviewTerminal: s.setPreviewTerminal,
        isPreview: s.previewTerminalId === cardId && s.focusedTerminalId !== cardId
      }
    })
  )
  const isMobile = useIsMobile()

  // Closed while focused. The stage empties on the next pass; bailing here
  // keeps it from drawing a header for a card that is gone.
  if (!kind || !sessionId) return null

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
        <span className="shrink-0 flex items-center justify-center w-4 h-4">
          {kind === 'browser' ? (
            <Globe size={16} strokeWidth={1.5} className="text-ink-faint" />
          ) : (
            <FileTypeIcon name={name} size={16} />
          )}
        </span>
        <span className="text-[13px] font-medium text-gray-200 truncate">{name}</span>
        <PaneOwnerLabel sessionId={sessionId} />
        <span className="flex-1" />
        <Tooltip label="Back to the grid">
          <button
            type="button"
            onClick={contract}
            className={ICON_BUTTON}
            aria-label={`Collapse ${name}`}
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
