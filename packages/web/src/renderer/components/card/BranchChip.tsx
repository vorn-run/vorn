import { useRef } from 'react'
import { useAppStore } from '../../stores'
import { useShallow } from 'zustand/react/shallow'
import { BranchPicker } from '../BranchPicker'
import { useBranchSwitcher } from '../../hooks/useBranchSwitcher'
import { GitBranch, FolderGit2, ChevronDown } from 'lucide-react'

interface Props {
  terminalId: string
  size?: 'sm' | 'md'
}

export function BranchChip({ terminalId, size = 'sm' }: Props) {
  const { terminal } = useAppStore(useShallow((s) => ({ terminal: s.terminals.get(terminalId) })))

  const session = terminal?.session
  const branchCwd = session && (session.worktreePath ?? session.projectPath)

  const buttonRef = useRef<HTMLButtonElement>(null)
  const { showPicker, togglePicker, closePicker, isSwitching, selectBranch } = useBranchSwitcher({
    projectPath: session?.projectPath,
    branchCwd,
    branchName: session?.branch
  })

  if (!terminal || !session || !session.branch) return null

  const { projectPath, branch: branchName, isWorktree, worktreeName } = session

  const iconSize = size === 'md' ? 11 : 10
  const textSize = size === 'md' ? 'text-[11px]' : 'text-[10px]'
  const maxWidth = size === 'md' ? 'max-w-[140px]' : 'max-w-[110px]'

  return (
    <button
      ref={buttonRef}
      type="button"
      onClick={(e) => {
        e.stopPropagation()
        togglePicker()
      }}
      onPointerDown={(e) => e.stopPropagation()}
      disabled={isSwitching}
      className={`relative flex items-center gap-1 rounded px-1 py-0.5 transition-colors shrink-0 ${
        showPicker
          ? 'text-gray-200 bg-white/[0.08]'
          : 'text-gray-400 hover:text-gray-200 hover:bg-white/[0.06]'
      } ${isSwitching ? 'opacity-50' : ''}`}
      aria-label={`Switch branch (current: ${branchName})`}
    >
      {/* Worth noticing — it explains why a command ran somewhere other than
          where you think — but noticing is not being blocked, so it earns
          brightness rather than the accent. */}
      {isWorktree && worktreeName && (
        <>
          <FolderGit2 size={iconSize} className="text-ink-secondary shrink-0" strokeWidth={1.5} />
          <span className={`font-mono text-ink-secondary truncate ${textSize} ${maxWidth}`}>
            {worktreeName}
          </span>
          <span className="text-gray-600 shrink-0">·</span>
        </>
      )}
      <GitBranch size={iconSize} className="text-gray-500 shrink-0" strokeWidth={1.5} />
      <span className={`font-mono truncate ${textSize} ${maxWidth}`}>{branchName}</span>
      <ChevronDown size={iconSize - 1} className="text-gray-500 shrink-0" strokeWidth={2} />
      {showPicker && (
        <BranchPicker
          projectPath={projectPath}
          currentBranch={branchName}
          onSelect={selectBranch}
          onClose={closePicker}
          anchorRef={buttonRef}
        />
      )}
    </button>
  )
}
