import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { AgentIcon } from './AgentIcon'
import { CardSubjectIcon } from './CardSubjectIcon'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import { STATUS_DOT } from '../lib/status-colors'
import { GitBranch, FolderGit2 } from 'lucide-react'
import { paneOwnerId } from '../lib/pane-id'
import { usePromotedCardSubject } from '../hooks/usePromotedCards'

/**
 * A stowed grid cell, and the way back to it.
 *
 * The dock takes pane ids, not session ids: a popped-out file or tab is a cell
 * like any other and minimizes like one. So this resolves the owner session from
 * the id rather than looking the id up directly — which for a card found
 * nothing, and rendered nothing, stranding it with no way to restore it.
 *
 * A card's pill is the card's own: its file icon or a globe, and its filename or
 * host. It borrows none of the session's chrome — no agent icon, no status dot,
 * because a file has no agent and is never "running", and a pill that showed
 * both would be claiming a state the thing does not have. The branch still
 * shows, since that is which copy of the file you are looking at.
 */
export function MinimizedPill({ terminalId }: { terminalId: string }) {
  const sessionId = paneOwnerId(terminalId)
  const card = usePromotedCardSubject(terminalId)
  const { terminal, toggleMinimized, setActiveTabId } = useAppStore(
    useShallow((s) => ({
      terminal: s.terminals.get(sessionId),
      toggleMinimized: s.toggleMinimized,
      setActiveTabId: s.setActiveTabId
    }))
  )

  if (!terminal) return null

  const { session, status } = terminal

  return (
    <button
      type="button"
      className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.06] bg-surface-raised
                 px-2.5 py-1 cursor-pointer transition-[border-color] select-none
                 hover:border-white/[0.12]"
      onClick={() => {
        toggleMinimized(terminalId)
        // The tab strip only holds sessions, so a card hands focus to its
        // owner — that is the tab it came from.
        setActiveTabId(sessionId)
      }}
      title="Click to restore"
    >
      {card ? (
        <CardSubjectIcon card={card} />
      ) : (
        <>
          <span
            className={`w-1.5 h-1.5 rounded-full shrink-0 ${STATUS_DOT[status]} ${
              status === 'running' ? 'animate-pulse' : ''
            }`}
          />
          <AgentIcon agentType={session.agentType} size={14} />
        </>
      )}

      <span className="text-[11px] font-medium text-gray-200 truncate max-w-[120px]">
        {card ? card.name : getDisplayName(session)}
      </span>

      {session.branch && (
        <>
          <span className="text-[10px] text-gray-600 shrink-0">&middot;</span>
          <span className="flex items-center gap-0.5 text-[10px] font-mono text-gray-500 truncate max-w-[90px]">
            {session.isWorktree ? (
              <FolderGit2 size={9} strokeWidth={1.5} className="text-ink-faint shrink-0" />
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
