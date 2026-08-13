import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import {
  MoreHorizontal,
  Play,
  Pencil,
  Trash2,
  Terminal,
  CheckCircle2,
  XCircle,
  RotateCcw,
  FileCode,
  Archive,
  ArchiveRestore
} from 'lucide-react'
import { TaskStatus, isTerminalTaskStatus } from '../../../shared/types'

interface MenuItem {
  icon: React.FC<{ size?: number; className?: string }>
  label: string
  onClick: () => void
  className?: string
  separator?: boolean
}

export function KanbanCardMenu({
  status,
  isArchived,
  onEdit,
  onDelete,
  onOpenSession,
  onComplete,
  onCancel,
  onReopen,
  onArchive,
  onUnarchive,
  onReviewDiff,
  sessionIsLive
}: {
  status: TaskStatus
  isArchived?: boolean
  onEdit: () => void
  onDelete: () => void
  onOpenSession?: () => void
  onComplete?: () => void
  onCancel?: () => void
  onReopen?: () => void
  onArchive?: () => void
  onUnarchive?: () => void
  onReviewDiff?: () => void
  sessionIsLive?: boolean
}) {
  const [open, setOpen] = useState(false)
  const [position, setPosition] = useState({ top: 0, left: 0 })
  const [confirmingDelete, setConfirmingDelete] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)

  const handleTrigger = (e: React.MouseEvent) => {
    e.stopPropagation()
    e.preventDefault()
    if (open) {
      close()
      return
    }
    const el = (e.target as HTMLElement).closest('button') ?? (e.target as HTMLElement)
    const rect = el.getBoundingClientRect()
    const MENU_WIDTH = 180
    const MARGIN = 8
    // Right-align the menu to the trigger, then clamp to viewport so it
    // doesn't overflow when the trigger sits in the rightmost column.
    const idealLeft = rect.right - MENU_WIDTH
    const clampedLeft = Math.max(
      MARGIN,
      Math.min(window.innerWidth - MENU_WIDTH - MARGIN, idealLeft)
    )
    setPosition({
      top: rect.bottom + 4,
      left: clampedLeft
    })
    setOpen(true)
    setConfirmingDelete(false)
  }

  const close = () => {
    setOpen(false)
    setConfirmingDelete(false)
  }

  const handleAction = (fn: () => void) => {
    close()
    fn()
  }

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      const target = e.target as Node
      if (triggerRef.current?.contains(target)) return
      if (menuRef.current && !menuRef.current.contains(target)) close()
    }
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    document.addEventListener('mousedown', handleClick)
    document.addEventListener('keydown', handleKey)
    return () => {
      document.removeEventListener('mousedown', handleClick)
      document.removeEventListener('keydown', handleKey)
    }
  }, [open])

  const items: MenuItem[] = []

  if (status === 'in_progress' && onOpenSession) {
    items.push({
      icon: sessionIsLive ? Terminal : Play,
      label: sessionIsLive ? 'Focus session' : 'Resume session',
      onClick: () => handleAction(onOpenSession),
      className: sessionIsLive ? 'text-violet-400' : 'text-amber-400'
    })
  }
  if (status === 'in_review' && onReviewDiff) {
    items.push({
      icon: FileCode,
      label: 'Review diff',
      onClick: () => handleAction(onReviewDiff),
      className: 'text-purple-400'
    })
  }
  if (status === 'in_review' && onComplete) {
    items.push({
      icon: CheckCircle2,
      label: 'Mark as done',
      onClick: () => handleAction(onComplete),
      className: 'text-green-400'
    })
  }
  if (status === 'cancelled' && onReopen) {
    items.push({
      icon: RotateCcw,
      label: 'Reopen task',
      onClick: () => handleAction(onReopen),
      className: 'text-amber-400'
    })
  }

  items.push({ icon: Pencil, label: 'Edit', onClick: () => handleAction(onEdit) })

  const isTerminal = isTerminalTaskStatus(status)
  const archiveItemShown = !isArchived && isTerminal && !!onArchive
  const unarchiveItemShown = isArchived && !!onUnarchive
  const cancelItemShown = !isTerminal && !!onCancel

  if (cancelItemShown) {
    items.push({
      icon: XCircle,
      label: 'Cancel task',
      onClick: () => handleAction(onCancel!),
      separator: true,
      className: 'text-red-400'
    })
  }

  if (archiveItemShown) {
    items.push({
      icon: Archive,
      label: 'Archive',
      onClick: () => handleAction(onArchive!),
      separator: true
    })
  }

  if (unarchiveItemShown) {
    items.push({
      icon: ArchiveRestore,
      label: 'Unarchive',
      onClick: () => handleAction(onUnarchive!),
      separator: true
    })
  }

  const deleteHasSiblingSeparator = archiveItemShown || unarchiveItemShown || cancelItemShown
  items.push({
    icon: Trash2,
    label: 'Delete',
    onClick: () => setConfirmingDelete(true),
    separator: !deleteHasSiblingSeparator,
    className: 'text-red-400'
  })

  return (
    <>
      <button
        ref={triggerRef}
        onClick={handleTrigger}
        className="p-1 text-gray-600 hover:text-gray-300 rounded transition-colors opacity-0 group-hover:opacity-100"
        title="More actions"
      >
        <MoreHorizontal size={14} strokeWidth={2} />
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
              {confirmingDelete ? (
                <div className="px-3 py-2">
                  <p className="text-xs text-gray-300 mb-2.5">Delete this task permanently?</p>
                  <div className="flex justify-end gap-2">
                    <button
                      onClick={() => setConfirmingDelete(false)}
                      className="px-2 py-1 text-[11px] text-gray-400 hover:text-gray-200 bg-white/[0.04] hover:bg-white/[0.08] rounded-md transition-colors"
                    >
                      Cancel
                    </button>
                    <button
                      onClick={() => handleAction(onDelete)}
                      className="px-2 py-1 text-[11px] font-medium text-red-400 bg-red-500/10 hover:bg-red-500/20 border border-red-500/20 rounded-md transition-colors"
                    >
                      Delete
                    </button>
                  </div>
                </div>
              ) : (
                items.map((item, i) => (
                  <div key={i}>
                    {item.separator && <div className="border-t border-white/[0.06] my-1" />}
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        item.onClick()
                      }}
                      className="w-full flex items-center gap-2.5 px-3 py-1.5 text-xs text-gray-300 hover:bg-white/[0.06] transition-colors"
                    >
                      <item.icon size={14} className={item.className ?? 'text-gray-500'} />
                      <span>{item.label}</span>
                    </button>
                  </div>
                ))
              )}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body
      )}
    </>
  )
}
