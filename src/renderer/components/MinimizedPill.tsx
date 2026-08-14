import { useAppStore } from '../stores'
import { useShallow } from 'zustand/react/shallow'
import { AgentIcon } from './AgentIcon'
import { FileTypeIcon } from './file-icons'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import { STATUS_DOT } from '../lib/status-colors'
import { GitBranch, FolderGit2, Globe } from 'lucide-react'
import { paneOwnerId, isPromotedCardId } from '../lib/pane-id'
import { displayHost } from '../lib/browser-url'

/** What a minimized card is, enough to name it and draw its icon. */
type CardSubject = { kind: 'editor'; fileName: string } | { kind: 'browser'; host: string }

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
  const { terminal, card, toggleMinimized, setActiveTabId } = useAppStore(
    useShallow((s) => ({
      terminal: s.terminals.get(sessionId),
      card: !isPromotedCardId(terminalId) ? null : readCardSubject(s, terminalId),
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
        <span className="shrink-0 flex items-center justify-center w-[14px] h-[14px]">
          {card.kind === 'browser' ? (
            <Globe size={14} strokeWidth={1.5} className="text-ink-faint" />
          ) : (
            <FileTypeIcon name={card.fileName} size={14} />
          )}
        </span>
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
        {card ? (card.kind === 'browser' ? card.host : card.fileName) : getDisplayName(session)}
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

function readCardSubject(
  state: {
    editorPanes: Map<string, { filePath: string }>
    browserPanes: Map<string, { tabs: string[]; activeTab: number }>
  },
  cardId: string
): CardSubject | null {
  const editor = state.editorPanes.get(cardId)
  if (editor) return { kind: 'editor', fileName: editor.filePath.split(/[/\\]/).pop() ?? '' }
  const browser = state.browserPanes.get(cardId)
  if (browser) {
    return {
      kind: 'browser',
      host: displayHost(browser.tabs[browser.activeTab] ?? browser.tabs[0] ?? '')
    }
  }
  return null
}
