import { useState } from 'react'
import { useAppStore } from '../stores'
import { AgentStatusIcon } from './AgentStatusIcon'
import { GitChangesIndicator } from './GitChangesIndicator'
import { CardContextMenu } from './CardContextMenu'
import { PromptLauncher } from './PromptLauncher'
import { useVisibleTerminals } from '../hooks/useVisibleTerminals'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import { GitBranch, FolderGit2 } from 'lucide-react'

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
      {orderedIds.map((id) => (
        <MobileSessionCard
          key={id}
          terminalId={id}
          isSelected={id === selectedId}
          onTap={() => {
            setSelected(id)
            setFocused(id)
          }}
        />
      ))}
    </div>
  )
}
