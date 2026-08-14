import { useState } from 'react'
import { useAppStore } from '../stores'
import { AgentStatusIcon } from './AgentStatusIcon'
import { GitChangesIndicator } from './GitChangesIndicator'
import { CardContextMenu } from './CardContextMenu'
import { PromptLauncher } from './PromptLauncher'
import { useVisibleTerminals } from '../hooks/useVisibleTerminals'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import { GitBranch, FolderGit2, Globe } from 'lucide-react'
import { isPromotedCardId } from '../lib/pane-id'
import { usePromotedCardSubject } from '../hooks/usePromotedCards'
import { FileTypeIcon } from './file-icons'

/**
 * A popped-out file or page in the mobile list.
 *
 * Cards are cells like sessions, so they arrive in `orderedIds` here too — and
 * `MobileSessionCard` returns null for one, which left them occupying a slot in
 * the keyboard ring and the visible set with no row to tap. Mobile shows one
 * thing at a time, and tapping this opens the same focus stage a session does.
 */
function MobileCardRow({
  cardId,
  isSelected,
  onTap
}: {
  cardId: string
  isSelected: boolean
  onTap: () => void
}) {
  const card = usePromotedCardSubject(cardId)
  const owner = useAppStore((s) => s.terminals.get(card?.sessionId ?? '')?.session.projectName)

  if (!card) return null

  return (
    <button
      onClick={onTap}
      className={`w-full text-left rounded-lg border px-3 py-2.5 flex items-center gap-2.5 transition-colors ${
        isSelected ? 'border-white/40' : 'border-white/[0.06]'
      }`}
      style={{ background: 'var(--color-surface-raised)' }}
    >
      <span className="shrink-0 flex items-center justify-center w-4 h-4">
        {card.kind === 'browser' ? (
          <Globe size={16} strokeWidth={1.5} className="text-ink-faint" />
        ) : (
          <FileTypeIcon name={card.name} size={16} />
        )}
      </span>
      <span className="text-[13px] text-gray-200 truncate flex-1">{card.name}</span>
      {owner && <span className="text-[11px] text-ink-secondary truncate shrink-0">{owner}</span>}
    </button>
  )
}

/**
 * Compact session card for the mobile card list.
 * Shows session summary — tap to open FocusedTerminal overlay.
 */
function MobileSessionCard({
  terminalId,
  isSelected,
  onTap
}: {
  terminalId: string
  isSelected: boolean
  onTap: () => void
}) {
  const terminal = useAppStore((s) => s.terminals.get(terminalId))
  const assignedTask = useAppStore((s) =>
    s.config?.tasks?.find((t) => t.assignedSessionId === terminalId && t.status === 'in_progress')
  )
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null)

  if (!terminal) return null

  const name = terminal.session.displayName?.trim()
    ? getDisplayName(terminal.session)
    : assignedTask
      ? assignedTask.title
      : getDisplayName(terminal.session)

  return (
    <>
      <button
        onClick={onTap}
        onContextMenu={(e) => {
          e.preventDefault()
          e.stopPropagation()
          setContextMenu({ x: e.clientX, y: e.clientY })
        }}
        className={`w-full rounded-xl border text-left transition-all ${
          isSelected ? 'border-white/20 ring-1 ring-white/10' : 'border-white/[0.06]'
        }`}
        style={{
          background: 'var(--glass-bg, var(--color-surface-overlay))',
          backdropFilter: 'var(--glass-blur, none)',
          WebkitBackdropFilter: 'var(--glass-blur, none)',
          boxShadow: isSelected ? 'var(--glass-shadow-thumb, none)' : 'var(--glass-shadow, none)',
          padding: '10px 12px'
        }}
      >
        {/* Row 1: icon + name + status */}
        <div className="flex items-center gap-2.5">
          <AgentStatusIcon
            agentType={terminal.session.agentType}
            status={terminal.status}
            size={16}
          />
          <span className="flex-1 min-w-0 text-[13px] font-medium text-gray-200 truncate">
            {name}
          </span>
        </div>

        {/* Row 2: branch + git diff */}
        <div className="flex items-center gap-2 mt-1 ml-[26px]">
          {terminal.session.branch && (
            <span className="flex items-center gap-1 text-[10px] font-mono text-gray-500 truncate min-w-0">
              {terminal.session.isWorktree ? (
                <FolderGit2 size={10} className="text-amber-500 shrink-0" strokeWidth={1.5} />
              ) : (
                <GitBranch size={10} className="text-gray-600 shrink-0" strokeWidth={1.5} />
              )}
              <span className={terminal.session.isWorktree ? 'text-amber-400' : ''}>
                {getBranchLabel(terminal.session)}
              </span>
            </span>
          )}
          <GitChangesIndicator terminalId={terminalId} />
        </div>
      </button>

      {contextMenu && (
        <CardContextMenu
          terminalId={terminalId}
          position={contextMenu}
          onClose={() => setContextMenu(null)}
        />
      )}
    </>
  )
}

/**
 * Mobile sessions layout: scrollable list of compact session cards.
 * Tap a card → opens FocusedTerminal fullscreen overlay.
 * Replaces the old single-pane swipe navigation.
 */
export function MobileSinglePane() {
  const { orderedIds } = useVisibleTerminals()
  const selectedId = useAppStore((s) => s.selectedTerminalId)
  const setSelected = useAppStore((s) => s.setSelectedTerminal)
  const setFocused = useAppStore((s) => s.setFocusedTerminal)

  // No terminals — show launcher
  if (orderedIds.length === 0) {
    return (
      <div className="h-full overflow-auto p-4">
        <PromptLauncher mode="inline" />
      </div>
    )
  }

  return (
    <div className="h-full overflow-y-auto px-3 py-3 space-y-2">
      {orderedIds.map((id) => {
        const onTap = (): void => {
          setSelected(id)
          setFocused(id)
        }
        return isPromotedCardId(id) ? (
          <MobileCardRow key={id} cardId={id} isSelected={id === selectedId} onTap={onTap} />
        ) : (
          <MobileSessionCard
            key={id}
            terminalId={id}
            isSelected={id === selectedId}
            onTap={onTap}
          />
        )
      })}
    </div>
  )
}
