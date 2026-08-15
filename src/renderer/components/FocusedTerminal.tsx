import { useRef } from 'react'
import { motion } from 'framer-motion'
import { useAppStore } from '../stores'
import { selectPaneFlags } from '../stores/ui-slice'
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
import { FilesCard } from './FilesCard'
import { EditorCard } from './EditorCard'
import { BrowserCard } from './BrowserCard'
import { DeviceCard } from './DeviceCard'
import { TerminalsCard } from './TerminalsCard'
import { parsePaneId } from '../lib/pane-id'
import { isMac } from '../lib/platform'
import { ArrowDown, FolderGit2, GitBranch, Minimize2, Pencil } from 'lucide-react'

/**
 * One pane's slot on the expanded stage, kept mounted while hidden.
 *
 * Taken out of flow rather than `display: none` so the pane keeps a real size
 * — a webview collapsed to zero does not reliably come back.
 */
function PaneSlot({
  hidden,
  children
}: {
  hidden: boolean
  children: React.ReactNode
}): React.ReactNode {
  if (hidden) {
    return (
      <div aria-hidden className="absolute inset-0 pointer-events-none invisible">
        {children}
      </div>
    )
  }
  return <div className="flex-1 min-h-0">{children}</div>
}

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
  // The expanded session's own panes come with it, so maximizing a card doesn't
  // hide the tree or file you had open next to it.
  const hasFilesPane = useAppStore((s) => (effectiveId ? s.filesPanes.has(effectiveId) : false))
  const hasEditorPane = useAppStore((s) => (effectiveId ? s.editorPanes.has(effectiveId) : false))
  const hasBrowserPane = useAppStore((s) => (effectiveId ? s.browserPanes.has(effectiveId) : false))
  const hasDevicePane = useAppStore((s) => (effectiveId ? s.devicePanes.has(effectiveId) : false))
  const hasTerminalsPane = useAppStore((s) =>
    effectiveId ? s.terminalsPanes.has(effectiveId) : false
  )
  // Shared with the card grid and tab view, so a new pane kind is added once.
  const hasAnyPane = useAppStore((s) => selectPaneFlags(s, effectiveId).any)
  // Maximize is session-scoped in the grid; expanded mode is that same session
  // filling the stage, so a maximized pane has to take the whole body here too.
  // Reading it only for panes this session owns keeps a stale id inert.
  const maximizedPaneId = useAppStore((s) => s.maximizedPaneId)
  useTerminalPinchZoom(terminalContainerRef)

  if (!effectiveId || !terminal) return null

  // Which of this session's panes, if any, is maximized. A pane belonging to
  // another session must not take over this stage, hence the owner check — and
  // the four kinds are named one by one rather than tested as "not a terminal",
  // because a popped-out card carries this session's id and a kind of its own,
  // so the looser test would match one and blank the stage for a card that is
  // drawn somewhere else entirely.
  const maximized = maximizedPaneId ? parsePaneId(maximizedPaneId) : null
  const maximizedKind = maximized && maximized.sessionId === effectiveId ? maximized.kind : null
  const hasMaximizedPane =
    (maximizedKind === 'files' && hasFilesPane) ||
    (maximizedKind === 'editor' && hasEditorPane) ||
    (maximizedKind === 'browser' && hasBrowserPane) ||
    (maximizedKind === 'device' && hasDevicePane) ||
    (maximizedKind === 'terminals' && hasTerminalsPane)

  const handleContract = (): void => {
    if (isPreview) {
      setPreviewTerminal(null)
    } else {
      setFocused(null)
    }
  }

  // Normally this rides inside the terminal column so the panes keep the full
  // height. A maximized pane collapses that column, and on macOS this wrapper is
  // the window's only drag region while a session is expanded (App.tsx drops the
  // app titlebar then) — losing it would leave the window undraggable, with no
  // Collapse button either. So in that one state it spans the stage.
  const desktopHeader = !isMobile && (
    <div
      className={`group/card shrink-0 ${isMac ? 'titlebar-drag' : 'titlebar-no-drag'}`}
      onDoubleClick={(e) => {
        if ((e.target as HTMLElement).closest('button, input, [role="button"]')) return
        handleContract()
      }}
    >
      <CardHeader terminalId={effectiveId} variant="focused" />
    </div>
  )

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
          background: 'var(--color-surface-raised)',
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
                    <FolderGit2
                      size={11}
                      className="text-ink-secondary shrink-0"
                      strokeWidth={1.5}
                    />
                  ) : (
                    <GitBranch size={11} className="text-gray-600 shrink-0" strokeWidth={1.5} />
                  )}
                  <span
                    className={`text-[11px] font-mono truncate ${
                      terminal.session.isWorktree ? 'text-ink-secondary' : 'text-gray-500'
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
        ) : null}

        {hasMaximizedPane && desktopHeader}

        {/* Terminal, plus this session's file panes riding along beside it. */}
        <div className="flex-1 min-h-0 flex">
          <div
            data-testid="focused-terminal-column"
            className={`flex-1 min-w-0 flex-col ${hasMaximizedPane ? 'hidden' : 'flex'}`}
          >
            {!hasMaximizedPane && desktopHeader}

            <div
              ref={terminalContainerRef}
              className="relative flex-1 p-1 min-h-0"
              style={{ background: 'var(--color-surface-sunken)' }}
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
          </div>

          {/* The expanded session keeps its own Files / File / Browser panes.
              While one of them is maximized it takes the whole stage, matching
              the grid's session-scoped maximize.

              A pane the maximize hides is hidden, not unmounted — see the note
              in PaneColumn: unmounting costs the browser its live guest, the
              page, and the agent's CDP handle. */}
          {!isMobile && hasAnyPane && (
            <div
              className={`relative shrink-0 flex flex-col gap-px ${
                hasMaximizedPane ? 'flex-1 min-w-0' : 'w-[420px] border-l border-white/[0.06]'
              }`}
            >
              {hasFilesPane && (
                <PaneSlot hidden={hasMaximizedPane && maximizedKind !== 'files'}>
                  <FilesCard sessionId={effectiveId} />
                </PaneSlot>
              )}
              {hasEditorPane && (
                <PaneSlot hidden={hasMaximizedPane && maximizedKind !== 'editor'}>
                  <EditorCard sessionId={effectiveId} />
                </PaneSlot>
              )}
              {hasBrowserPane && (
                <PaneSlot hidden={hasMaximizedPane && maximizedKind !== 'browser'}>
                  <BrowserCard sessionId={effectiveId} />
                </PaneSlot>
              )}
              {hasDevicePane && (
                <PaneSlot hidden={hasMaximizedPane && maximizedKind !== 'device'}>
                  <DeviceCard sessionId={effectiveId} />
                </PaneSlot>
              )}
              {hasTerminalsPane && (
                <PaneSlot hidden={hasMaximizedPane && maximizedKind !== 'terminals'}>
                  <TerminalsCard sessionId={effectiveId} />
                </PaneSlot>
              )}
            </div>
          )}
        </div>

        {isMobile && <MobileTerminalKeybar terminalId={effectiveId} />}
      </motion.div>
    </>
  )
}
