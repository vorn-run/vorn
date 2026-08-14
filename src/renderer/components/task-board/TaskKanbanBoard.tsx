import { useState, useRef } from 'react'
import { TaskConfig, TaskStatus } from '../../../shared/types'
import { TaskCard } from './TaskCard'
import { TASK_STATUS_ICON, TASK_STATUS_TEXT, TASK_STATUS_LABEL } from '../../lib/task-status'
import { Plus } from 'lucide-react'

const KANBAN_ORDER: TaskStatus[] = ['todo', 'in_progress', 'in_review', 'done', 'cancelled']

const KANBAN_COLUMNS: { status: TaskStatus; title: string }[] = KANBAN_ORDER.map((status) => ({
  status,
  title: TASK_STATUS_LABEL[status]
}))

export function TaskKanbanBoard({
  allTasks,
  onEdit,
  onDelete,
  onDrop,
  onOpenSession,
  onComplete,
  onCancel,
  onReopen,
  onArchive,
  onUnarchive,
  onReviewDiff,
  onSelect,
  onAddTask,
  isSessionLive
}: {
  allTasks: TaskConfig[]
  onEdit: (task: TaskConfig) => void
  onDelete: (id: string) => void
  onDrop: (taskId: string, newStatus: TaskStatus) => void
  onOpenSession: (task: TaskConfig) => (() => void) | undefined
  onComplete: (id: string) => void
  onCancel: (id: string) => void
  onReopen: (id: string) => void
  onArchive: (id: string) => void
  onUnarchive: (id: string) => void
  onReviewDiff: (id: string) => void
  onSelect?: (task: TaskConfig) => void
  onAddTask?: (status: TaskStatus) => void
  isSessionLive: (task: TaskConfig) => boolean
}) {
  const [dragOverCol, setDragOverCol] = useState<TaskStatus | null>(null)
  const [draggingId, setDraggingId] = useState<string | null>(null)
  const dragTaskId = useRef<string | null>(null)
  const dragEnterCount = useRef<Map<TaskStatus, number>>(new Map())

  const handleDragStart = (taskId: string) => {
    dragTaskId.current = taskId
    setDraggingId(taskId)
  }

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault()
  }

  const handleDragEnter = (status: TaskStatus) => {
    const count = (dragEnterCount.current.get(status) ?? 0) + 1
    dragEnterCount.current.set(status, count)
    setDragOverCol(status)
  }

  const handleDragLeave = (status: TaskStatus) => {
    const count = (dragEnterCount.current.get(status) ?? 1) - 1
    dragEnterCount.current.set(status, count)
    if (count <= 0) {
      dragEnterCount.current.delete(status)
      if (dragOverCol === status) setDragOverCol(null)
    }
  }

  const handleDrop = (status: TaskStatus) => {
    if (dragTaskId.current) {
      onDrop(dragTaskId.current, status)
      dragTaskId.current = null
    }
    dragEnterCount.current.clear()
    setDraggingId(null)
    setDragOverCol(null)
  }

  const handleDragEnd = () => {
    dragTaskId.current = null
    dragEnterCount.current.clear()
    setDraggingId(null)
    setDragOverCol(null)
  }

  return (
    <div className="flex gap-2 flex-1 min-h-0">
      {KANBAN_COLUMNS.map((col) => {
        const tasks = allTasks
          .filter((t) => t.status === col.status)
          .sort((a, b) => a.order - b.order)
        const ColIcon = TASK_STATUS_ICON[col.status]
        const iconColor = TASK_STATUS_TEXT[col.status]
        const isDragOver = dragOverCol === col.status

        // A column is a region of the board, not a material laid on top of it. The
        // white wash it used to carry lit five tall slabs a step above the field,
        // which is what made this surface read pale beside the workflow canvas —
        // there the field shows through and only the nodes lift off it.
        return (
          <div
            key={col.status}
            className={`group/col flex-1 min-w-0 min-h-0 flex flex-col rounded-lg transition-all duration-200 ${
              isDragOver ? 'bg-white/[0.04] ring-1 ring-inset ring-white/[0.1]' : ''
            }`}
            onDragOver={handleDragOver}
            onDragEnter={() => handleDragEnter(col.status)}
            onDragLeave={() => handleDragLeave(col.status)}
            onDrop={() => handleDrop(col.status)}
          >
            {/* Column header */}
            <div className="px-3 py-3 flex items-center gap-2 shrink-0">
              <ColIcon size={14} className={iconColor} />
              <span className="text-[13px] font-medium text-ink-secondary">{col.title}</span>
              <span className="text-[11px] text-ink-faint ml-0.5">{tasks.length}</span>
              <div className="flex-1" />
              <button
                onClick={() => onAddTask?.(col.status)}
                className="p-1 text-ink-faint hover:text-ink-secondary rounded transition-colors opacity-0 group-hover/col:opacity-100"
                title="Add task"
              >
                <Plus size={14} />
              </button>
            </div>

            {/* Cards */}
            <div className="flex-1 overflow-y-auto px-2 pb-2 pt-0 space-y-2">
              {tasks.length === 0 ? (
                <div className="flex-1 flex items-center justify-center min-h-[80px]">
                  <div className="border border-dashed border-white/[0.08] rounded-lg px-4 py-5 text-center w-full">
                    <p className="text-xs text-ink-faint">Drop tasks here</p>
                  </div>
                </div>
              ) : (
                tasks.map((task) => (
                  <div
                    key={task.id}
                    draggable
                    onDragStart={() => handleDragStart(task.id)}
                    onDragEnd={handleDragEnd}
                    className="cursor-grab active:cursor-grabbing transition-opacity duration-150"
                    style={{ opacity: draggingId === task.id ? 0.4 : 1 }}
                  >
                    <TaskCard
                      task={task}
                      onEdit={() => onEdit(task)}
                      onDelete={() => onDelete(task.id)}
                      onOpenSession={onOpenSession(task)}
                      onComplete={() => onComplete(task.id)}
                      onCancel={() => onCancel(task.id)}
                      onReopen={() => onReopen(task.id)}
                      onArchive={() => onArchive(task.id)}
                      onUnarchive={() => onUnarchive(task.id)}
                      onReviewDiff={() => onReviewDiff(task.id)}
                      onSelect={onSelect ? () => onSelect(task) : undefined}
                      sessionIsLive={isSessionLive(task)}
                      variant="kanban"
                    />
                  </div>
                ))
              )}
            </div>

            {/* Bottom add button */}
            <div className="px-2 pb-2">
              <button
                onClick={() => onAddTask?.(col.status)}
                className="w-full py-2 text-xs text-ink-faint hover:text-ink-secondary
                           hover:bg-white/[0.04] rounded-lg transition-colors
                           flex items-center justify-center gap-1"
              >
                <Plus size={12} />
              </button>
            </div>
          </div>
        )
      })}
    </div>
  )
}
