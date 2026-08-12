import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { AgentIcon } from './AgentIcon'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import { STATUS_DOT } from '../lib/status-colors'
import { GitBranch, FolderGit2, FolderTree, FileCode, Globe } from 'lucide-react'
import { parsePaneId } from '../lib/pane-id'
import { displayHost } from '../lib/browser-url'

/**
 * Dock pill for a minimized file-tree, editor or browser pane. Labelled with
 * its owner session so several sessions' panes stay tellable apart in the dock.
 */
function ChildPanePill({ paneId }: { paneId: string }) {
  const { kind, sessionId } = parsePaneId(paneId)
  const { terminal, filePath, browserUrl, toggleMinimized } = useAppStore(
    useShallow((s) => ({
      terminal: s.terminals.get(sessionId),
      filePath: s.editorPanes.get(sessionId)?.filePath ?? null,
      browserUrl: s.browserPanes.get(sessionId)?.url ?? null,
      toggleMinimized: s.toggleMinimized
    }))
  )

  if (!terminal) return null

  const owner = getDisplayName(terminal.session)
  // Each fallback matters: an empty basename or url would render a pill with no
  // label at all, leaving a minimized pane you cannot identify in the dock.
  const label =
    kind === 'editor'
      ? filePath?.split(/[/\\]/).pop() || 'File'
      : kind === 'browser'
        ? browserUrl
          ? displayHost(browserUrl)
          : 'Browser'
        : 'Files'

  return (
    <button
      type="button"
      className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.06] bg-[#1a1a1e]
                 px-2.5 py-1 cursor-pointer transition-[border-color] select-none
                 hover:border-white/[0.12]"
      onClick={() => toggleMinimized(paneId)}
      title={`Click to restore — ${owner}`}
    >
      {kind === 'editor' ? (
        <FileCode size={13} strokeWidth={1.5} className="text-gray-400 shrink-0" />
      ) : kind === 'browser' ? (
        <Globe size={13} strokeWidth={1.5} className="text-gray-400 shrink-0" />
      ) : (
        <FolderTree size={13} strokeWidth={1.5} className="text-gray-400 shrink-0" />
      )}
      <span className="text-[11px] font-medium text-gray-200 truncate max-w-[120px]">{label}</span>
      <span className="text-[10px] text-gray-600 shrink-0">&middot;</span>
      <span className="text-[10px] text-gray-500 truncate max-w-[90px]">{owner}</span>
    </button>
  )
}

export function MinimizedPill({ terminalId }: { terminalId: string }) {
  const isChildPane = parsePaneId(terminalId).kind !== 'terminal'

  const { terminal, toggleMinimized, setActiveTabId } = useAppStore(
    useShallow((s) => ({
      terminal: s.terminals.get(terminalId),
      toggleMinimized: s.toggleMinimized,
      setActiveTabId: s.setActiveTabId
    }))
  )

  if (isChildPane) return <ChildPanePill paneId={terminalId} />

  if (!terminal) return null

  const { session, status } = terminal

  return (
    <button
      type="button"
      className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.06] bg-[#1a1a1e]
                 px-2.5 py-1 cursor-pointer transition-[border-color] select-none
                 hover:border-white/[0.12]"
      onClick={() => {
        toggleMinimized(terminalId)
        setActiveTabId(terminalId)
      }}
      title="Click to restore"
    >
      <span
        className={`w-1.5 h-1.5 rounded-full shrink-0 ${STATUS_DOT[status]} ${
          status === 'running' ? 'animate-pulse' : ''
        }`}
      />

      <AgentIcon agentType={session.agentType} size={14} />

      <span className="text-[11px] font-medium text-gray-200 truncate max-w-[120px]">
        {getDisplayName(session)}
      </span>

      {session.branch && (
        <>
          <span className="text-[10px] text-gray-600 shrink-0">&middot;</span>
          <span className="flex items-center gap-0.5 text-[10px] font-mono text-gray-500 truncate max-w-[90px]">
            {session.isWorktree ? (
              <FolderGit2 size={9} strokeWidth={1.5} className="text-amber-400/70 shrink-0" />
            ) : (
              <GitBranch size={9} strokeWidth={1.5} className="shrink-0" />
            )}
            {getBranchLabel(session)}
          </span>
        </>
      )}
    </button>
  )
}
