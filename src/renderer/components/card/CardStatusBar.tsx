import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../../stores'
import { GitChangesIndicator } from '../GitChangesIndicator'
import { OpenInButton } from '../OpenInButton'
import { BranchChip } from './BranchChip'
import { LastCommandChip } from './LastCommandChip'
import { ListTodo } from 'lucide-react'

interface Props {
  terminalId: string
  /** Visually de-emphasise this status bar when another card is the active selection. */
  dimmed?: boolean
}

export function CardStatusBar({ terminalId, dimmed }: Props) {
  const { terminal, assignedTask, setEditingTask, setTaskDialogOpen } = useAppStore(
    useShallow((s) => ({
      terminal: s.terminals.get(terminalId),
      assignedTask: s.config?.tasks?.find(
        (t) => t.assignedSessionId === terminalId && t.status === 'in_progress'
      ),
      setEditingTask: s.setEditingTask,
      setTaskDialogOpen: s.setTaskDialogOpen
    }))
  )

  if (!terminal) return null

  const hasBranch = Boolean(terminal.session.branch)

  return (
    <div
      className={`shrink-0 flex items-center gap-2 px-2 h-[22px] border-t border-white/[0.04] text-[11px]
                  transition-opacity duration-200 ease-out
                  ${dimmed ? 'opacity-60 group-hover/card:opacity-100' : 'opacity-100'}`}
      style={{ background: 'var(--color-surface-raised)' }}
    >
      {hasBranch && <BranchChip terminalId={terminalId} />}

      {assignedTask && (
        <button
          type="button"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation()
            setEditingTask(assignedTask)
            setTaskDialogOpen(true)
          }}
          className="flex items-center gap-1 px-1.5 py-0.5 rounded-full border border-white/[0.08]
                     hover:bg-white/[0.06] transition-colors shrink-0"
        >
          <ListTodo size={10} className="text-ink-faint shrink-0" strokeWidth={2} />
          <span className="text-[10px] text-ink-secondary truncate max-w-[140px]">
            {assignedTask.title}
          </span>
        </button>
      )}

      <LastCommandChip terminalId={terminalId} />

      <div className="flex-1" />

      <GitChangesIndicator terminalId={terminalId} />
      <OpenInButton projectPath={terminal.session.projectPath} direction="up" />
    </div>
  )
}
