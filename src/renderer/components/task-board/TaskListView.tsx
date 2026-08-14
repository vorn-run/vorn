import { useState } from 'react'
import { TaskConfig, TaskStatus } from '../../../shared/types'
import { TaskCard } from './TaskCard'
import { TASK_STATUS_ICON, TASK_STATUS_TEXT } from '../../lib/task-status'
import { ChevronDown, ChevronRight, Plus } from 'lucide-react'
import { motion, AnimatePresence } from 'framer-motion'

export function TaskListView({
  sections,
  onEdit,
  onDelete,
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
  sections: { status: TaskStatus; title: string; tasks: TaskConfig[]; emptyText: string }[]
  onEdit: (task: TaskConfig) => void
  onDelete: (id: string) => void
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
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())

  const toggleSection = (title: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(title)) {
        next.delete(title)
      } else {
        next.add(title)
      }
      return next
    })
  }

  return (
    <div className="space-y-1">
      {sections.map((section) => {
        const isCollapsed = collapsed.has(section.title)
        const SectionIcon = TASK_STATUS_ICON[section.status]
        const iconColor = TASK_STATUS_TEXT[section.status]

        return (
          <div key={section.title}>
            {/* Section header bar */}
            <div
              className="w-full flex items-center gap-2 px-3 py-2 rounded-lg bg-white/[0.03] transition-colors hover:bg-white/[0.06] cursor-pointer"
              onClick={() => toggleSection(section.title)}
            >
              {isCollapsed ? (
                <ChevronRight size={14} className="text-ink-secondary shrink-0" />
              ) : (
                <ChevronDown size={14} className="text-ink-secondary shrink-0" />
              )}
              <SectionIcon size={14} className={`${iconColor} shrink-0`} />
              <span className="text-[13px] font-medium text-ink-secondary">{section.title}</span>
              <span className="text-[11px] text-ink-faint">{section.tasks.length}</span>
              <div className="flex-1" />
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation()
                  onAddTask?.(section.status)
                }}
                className="p-1 text-ink-faint hover:text-ink-secondary rounded transition-colors"
                aria-label={`Add task to ${section.title}`}
                title="Add task"
              >
                <Plus size={14} />
              </button>
            </div>

            {/* Collapsible task list */}
            <AnimatePresence initial={false}>
              {!isCollapsed && (
                <motion.div
                  initial={{ height: 0, opacity: 0 }}
                  animate={{ height: 'auto', opacity: 1 }}
                  exit={{ height: 0, opacity: 0 }}
                  transition={{ duration: 0.2, ease: 'easeInOut' }}
                  className="overflow-hidden"
                >
                  <div className="py-1">
                    {section.tasks.length === 0 ? (
                      <p className="text-xs text-ink-faint py-2 pl-10">{section.emptyText}</p>
                    ) : (
                      section.tasks.map((task) => (
                        <TaskCard
                          key={task.id}
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
                        />
                      ))
                    )}
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        )
      })}
    </div>
  )
}
