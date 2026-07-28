import { useRef } from 'react'
import { motion } from 'framer-motion'
import { useAppStore } from '../stores'
import { TerminalPane } from './TerminalPane'
import { terminalTextIndentPx } from '../lib/terminal-indent'
import { AgentStatusIcon } from './AgentStatusIcon'
import { InlineRename } from './InlineRename'
import { CardHeader } from './card/CardHeader'
import { CardStatusBar } from './card/CardStatusBar'
import { IntentBar } from './IntentBar'
import { MobileFontSizeControl } from './MobileFontSizeControl'
import { MobileTerminalKeybar } from './MobileTerminalKeybar'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import { useTerminalScrollButton } from '../hooks/useTerminalScrollButton'
import { useTerminalPinchZoom } from '../hooks/useTerminalPinchZoom'
import { useIsMobile } from '../hooks/useIsMobile'
import { isMac } from '../lib/platform'
import { ArrowDown, FolderGit2, GitBranch, Minimize2, Pencil } from 'lucide-react'

export function FocusedTerminal() {
  const focusedId = useAppStore((s) => s.focusedTerminalId)
  const previewId = useAppStore((s) => s.previewTerminalId)
  const effectiveId = previewId ?? focusedId
  const isPreview = previewId !== null && focusedId !== previewId
  const terminal = useAppStore((s) => (effectiveId ? s.terminals.get(effectiveId) : undefined))
  const setFocused = useAppStore((s) => s.setFocusedTerminal)
  const setPreviewTerminal = useAppStore((s) => s.setPreviewTerminal)
  const isRenaming = useAppStore((s) => s.renamingTerminalId === effectiveId)
  const setRenamingTerminalId = useAppStore((s) => s.setRenamingTerminalId)
  const renameTerminal = useAppStore((s) => s.renameTerminal)
  const { showScrollBtn, handleScrollToBottom } = useTerminalScrollButton(effectiveId)
  const terminalContainerRef = useRef<HTMLDivElement>(null)
  const isMobile = useIsMobile()
  const domBlocks = useAppStore((s) => s.config?.defaults.domBlockRendering ?? true)
  useTerminalPinchZoom(terminalContainerRef)

  if (!effectiveId || !terminal) return null

  const handleContract = (): void => {
    if (isPreview) {
      setPreviewTerminal(null)
    } else {
      setFocused(null)
    }
  }

  return (
    <>
      {/* Backdrop — mobile only */}
      {isMobile && (
        <motion.div
          className="fixed inset-0 bg-black/60 backdrop-blur-sm z-30"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          onClick={handleContract}
        />
      )}

      {/* Focused panel */}
      <motion.div
        className={
          isMobile
            ? 'fixed inset-0 z-40 shadow-2xl flex flex-col overflow-hidden'
            : 'flex-1 flex flex-col min-h-0 overflow-hidden'
        }
        style={{
          background: '#1a1a1e',
          ...(isMobile ? { paddingTop: 'var(--safe-top, 0px)' } : {})
        }}
        {...(isMobile
          ? {
              initial: { opacity: 0, scale: 0.97 },
              animate: { opacity: 1, scale: 1 },
              transition: { type: 'spring', stiffness: 400, damping: 30 }
            }
          : {})}
      >
        {isMobile ? (
          <div
            className="flex items-center gap-3 pl-3 pr-4 py-2.5 border-b border-white/[0.06] titlebar-no-drag"
            onDoubleClick={(e) => {
              if ((e.target as HTMLElement).closest('button, input, [role="button"]')) return
              handleContract()
            }}
          >
            <button
              type="button"
              onClick={handleContract}
              className="p-1.5 -ml-1 rounded-md text-gray-400 hover:text-white hover:bg-white/[0.08] transition-colors"
              aria-label="Back to sessions"
            >
              <Minimize2 size={16} strokeWidth={2} />
            </button>
            <AgentStatusIcon
              agentType={terminal.session.agentType}
              status={terminal.status}
              size={16}
            />
            <div className="flex-1 min-w-0">
              {isRenaming ? (
                <InlineRename
                  value={getDisplayName(terminal.session)}
                  onCommit={(name) => {
                    renameTerminal(effectiveId, name)
                    setRenamingTerminalId(null)
                  }}
                  onCancel={() => setRenamingTerminalId(null)}
                  className="text-[13px] font-medium"
                />
              ) : (
                <span className="inline-flex items-center gap-1 group/rename">
                  <span
                    className="text-[13px] font-medium text-gray-200 cursor-default"
                    onDoubleClick={() => setRenamingTerminalId(effectiveId)}
                  >
                    {getDisplayName(terminal.session)}
                  </span>
                  <button
                    type="button"
                    onClick={() => setRenamingTerminalId(effectiveId)}
                    className="opacity-0 group-hover/rename:opacity-100 text-gray-500 hover:text-gray-300 transition-opacity shrink-0"
                    aria-label="Rename session"
                  >
                    <Pencil size={11} />
                  </button>
                </span>
              )}
              {terminal.session.branch && (
                <span className="flex items-center gap-1 mt-0.5">
                  {terminal.session.isWorktree ? (
                    <FolderGit2 size={11} className="text-amber-500 shrink-0" strokeWidth={1.5} />
                  ) : (
                    <GitBranch size={11} className="text-gray-600 shrink-0" strokeWidth={1.5} />
                  )}
                  <span
                    className={`text-[11px] font-mono truncate ${
                      terminal.session.isWorktree ? 'text-amber-400' : 'text-gray-500'
                    }`}
                  >
                    {getBranchLabel(terminal.session)}
                  </span>
                  {terminal.session.isWorktree && (
                    <>
                      <GitBranch size={10} className="text-gray-600 shrink-0" strokeWidth={1.5} />
                      <span className="text-[10px] font-mono text-gray-500 truncate">
                        {terminal.session.branch}
                      </span>
                    </>
                  )}
                </span>
              )}
            </div>
          </div>
        ) : (
          <div
            className={`group/card ${isMac ? 'titlebar-drag' : 'titlebar-no-drag'}`}
            onDoubleClick={(e) => {
              if ((e.target as HTMLElement).closest('button, input, [role="button"]')) return
              handleContract()
            }}
          >
            <CardHeader terminalId={effectiveId} variant="focused" />
          </div>
        )}

        {/* Terminal */}
        <div
          ref={terminalContainerRef}
          className="relative flex-1 p-1 min-h-0"
          style={{ background: 'rgba(0, 0, 0, 0.3)' }}
        >
          <TerminalPane
            terminalId={effectiveId}
            agentType={terminal.session.agentType}
            isFocused={!isRenaming && !isPreview}
            domBlocks={domBlocks}
          />
          {/* Mobile: floating controls (font size + scroll) */}
          <div className="absolute bottom-4 right-4 flex flex-col items-end gap-2 z-50">
            {isMobile && <MobileFontSizeControl />}
            {showScrollBtn && (
              <button
                className="w-8 h-8 flex items-center justify-center
                           rounded bg-white/[0.08] hover:bg-white/[0.15] text-gray-400 hover:text-white
                           transition-colors"
                onClick={handleScrollToBottom}
                title="Scroll to bottom"
              >
                <ArrowDown size={14} />
              </button>
            )}
          </div>
        </div>

        {/* +4 for this pane's own container padding, so the caret lands in
            the terminal's text column. */}
        {!isMobile && (
          <IntentBar
            terminalId={effectiveId}
            indentPx={terminalTextIndentPx(terminal.session.agentType, domBlocks) + 4}
          />
        )}

        {!isMobile && <CardStatusBar terminalId={effectiveId} />}

        {isMobile && <MobileTerminalKeybar terminalId={effectiveId} />}
      </motion.div>
    </>
  )
}
