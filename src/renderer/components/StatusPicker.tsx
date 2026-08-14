import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { Check } from 'lucide-react'
import { TaskStatus } from '../../shared/types'
import { TASK_STATUS_LABEL, TASK_STATUS_ICON, TASK_STATUS_TEXT } from '../lib/task-status'
import { useAppStore } from '../stores'
import { toast } from './Toast'

const ALL_STATUSES: TaskStatus[] = ['todo', 'in_progress', 'in_review', 'done', 'cancelled']

export function StatusPicker({
  taskId,
  currentStatus,
  disabled,
  onChange
}: {
  taskId?: string
  currentStatus: TaskStatus
  disabled?: boolean
  onChange?: (status: TaskStatus) => void
}) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState({ top: 0, left: 0 })

  const completeTask = useAppStore((s) => s.completeTask)
  const cancelTask = useAppStore((s) => s.cancelTask)
  const reopenTask = useAppStore((s) => s.reopenTask)
  const reviewTask = useAppStore((s) => s.reviewTask)
  const updateTask = useAppStore((s) => s.updateTask)

  const CurrentIcon = TASK_STATUS_ICON[currentStatus]

  const handleTrigger = (e: React.MouseEvent) => {
    if (disabled) return
    e.stopPropagation()
    if (open) {
      setOpen(false)
      return
    }
    const rect = triggerRef.current?.getBoundingClientRect()
    if (rect) {
      setPosition({ top: rect.bottom + 4, left: rect.left })
    }
    setOpen(true)
  }

  const handleSelect = (status: TaskStatus) => {
    if (status === currentStatus) {
      setOpen(false)
      return
    }

    setOpen(false)

    // Controlled mode: just call onChange
    if (onChange) {
      onChange(status)
      return
    }

    // Store mode: call dedicated store methods for existing tasks
    if (!taskId) return

    switch (status) {
      case 'todo':
        reopenTask(taskId)
        toast.success('Task reopened')
        break
      case 'in_progress':
        // Only todo → in_progress is supported: the seeded Default Task
        // Workflow listens for that exact transition. Other sources would
        // leave completedAt/assignedSessionId stale with no agent spawn.
        if (currentStatus !== 'todo') return
        updateTask(taskId, { status: 'in_progress' })
        break
      case 'in_review':
        reviewTask(taskId)
        toast.info('Task moved to review')
        break
      case 'done':
        completeTask(taskId)
        toast.success('Task completed')
        break
      case 'cancelled':
        cancelTask(taskId)
        toast.info('Task cancelled')
        break
    }
  }

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      const target = e.target as Node
      if (triggerRef.current?.contains(target)) return
      if (menuRef.current && !menuRef.current.contains(target)) setOpen(false)
    }
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', handleClick)
    document.addEventListener('keydown', handleKey)
    return () => {
      document.removeEventListener('mousedown', handleClick)
      document.removeEventListener('keydown', handleKey)
    }
  }, [open])

  return (
    <>
      <button
        ref={triggerRef}
        onClick={handleTrigger}
        className={`flex items-center gap-1.5 rounded px-1.5 py-0.5 -mx-1.5 transition-colors ${
          disabled ? 'cursor-default' : 'hover:bg-white/[0.04]'
        }`}
      >
        <CurrentIcon size={13} className={TASK_STATUS_TEXT[currentStatus]} />
        <span className={`text-[12px] ${TASK_STATUS_TEXT[currentStatus]}`}>
          {TASK_STATUS_LABEL[currentStatus]}
        </span>
      </button>

      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={menuRef}
              initial={{ opacity: 0, y: -4, scale: 0.96 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -4, scale: 0.96 }}
              transition={{ type: 'spring', stiffness: 500, damping: 30 }}
              className="fixed z-[150] rounded-lg border border-white/[0.1] shadow-2xl py-1"
              style={{
                top: position.top,
                left: position.left,
                background: 'var(--color-surface-overlay)',
                minWidth: 180
              }}
            >
              {ALL_STATUSES.map((status) => {
                const Icon = TASK_STATUS_ICON[status]
                const isCurrent = status === currentStatus
                // In store mode, in_progress is only reachable from todo — the
                // default workflow only fires on that transition.
                const isItemDisabled =
                  !onChange && status === 'in_progress' && currentStatus !== 'todo'

                return (
                  <button
                    key={status}
                    onClick={(e) => {
                      e.stopPropagation()
                      handleSelect(status)
                    }}
                    disabled={isItemDisabled}
                    className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-xs transition-colors ${
                      isItemDisabled
                        ? 'text-ink-faint cursor-not-allowed'
                        : 'text-ink-secondary hover:bg-white/[0.06] cursor-pointer'
                    }`}
                    title={isItemDisabled ? 'Move to Todo first, then start the task' : undefined}
                  >
                    <Icon size={14} className={TASK_STATUS_TEXT[status]} />
                    <span className="flex-1 text-left">{TASK_STATUS_LABEL[status]}</span>
                    {isCurrent && <Check size={13} className="text-ink-secondary" />}
                  </button>
                )
              })}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body
      )}
    </>
  )
}
