import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { AgentIcon } from './AgentIcon'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import { STATUS_DOT } from '../lib/status-colors'
import { GitBranch, FolderGit2 } from 'lucide-react'

export function MinimizedPill({ terminalId }: { terminalId: string }) {
  const { terminal, toggleMinimized, setActiveTabId } = useAppStore(
    useShallow((s) => ({
      terminal: s.terminals.get(terminalId),
      toggleMinimized: s.toggleMinimized,
      setActiveTabId: s.setActiveTabId
    }))
  )

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
