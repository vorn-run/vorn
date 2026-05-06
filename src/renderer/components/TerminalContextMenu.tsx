import { useEffect, useRef, useState, useCallback } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { Copy, ClipboardPaste, Zap, ChevronRight } from 'lucide-react'
import {
  getTerminalSelection,
  clearTerminalSelection,
  pasteToTerminal,
  focusTerminal
} from '../lib/terminal-registry'
import { useAppStore } from '../stores'
import { useWorkspaceWorkflows } from '../hooks/useWorkspaceWorkflows'
import { buildWorkflowMenuItems } from '../lib/workflow-menu-items'

interface Props {
  terminalId: string
  position: { x: number; y: number }
  onClose: () => void
}

export function TerminalContextMenu({ terminalId, position, onClose }: Props) {
  const menuRef = useRef<HTMLDivElement>(null)
  const submenuRef = useRef<HTMLDivElement>(null)
  const hideTimeout = useRef<ReturnType<typeof setTimeout> | null>(null)
  const selection = getTerminalSelection(terminalId)
  const workspaceWorkflows = useWorkspaceWorkflows()
  const sourceSession = useAppStore((s) => s.terminals.get(terminalId)?.session)

  const [showWorkflowSubmenu, setShowWorkflowSubmenu] = useState(false)
  const [workflowBtnTop, setWorkflowBtnTop] = useState(0)

  const clearHideTimeout = useCallback(() => {
    if (hideTimeout.current) {
      clearTimeout(hideTimeout.current)
      hideTimeout.current = null
    }
  }, [])

  const scheduleHide = useCallback(() => {
    clearHideTimeout()
    hideTimeout.current = setTimeout(() => setShowWorkflowSubmenu(false), 150)
  }, [clearHideTimeout])

  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      const target = e.target as Node
      if (menuRef.current?.contains(target) || submenuRef.current?.contains(target)) return
      onClose()
    }
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('pointerdown', handleClick)
    document.addEventListener('keydown', handleKey)
    return () => {
      document.removeEventListener('pointerdown', handleClick)
      document.removeEventListener('keydown', handleKey)
      clearHideTimeout()
    }
  }, [onClose, clearHideTimeout])

  const close = () => {
    onClose()
    focusTerminal(terminalId)
  }

  const handleCopy = () => {
    if (selection) {
      navigator.clipboard.writeText(selection)
      clearTerminalSelection(terminalId)
    }
    close()
  }

  const handlePaste = () => {
    navigator.clipboard.readText().then((text) => {
      if (text) pasteToTerminal(terminalId, text)
    })
    close()
  }

  const workflowSubmenuItems = buildWorkflowMenuItems(
    workspaceWorkflows,
    close,
    sourceSession ? { source: sourceSession } : undefined
  )

  const hasWorkflows = workflowSubmenuItems.length > 0
  const itemCount = 2 + (hasWorkflows ? 1 : 0)
  const menuWidth = 180
  const menuHeight = itemCount * 32 + (hasWorkflows ? 9 : 0) + 16
  const left = Math.max(8, Math.min(position.x, window.innerWidth - menuWidth - 8))
  const top = Math.max(8, Math.min(position.y, window.innerHeight - menuHeight - 8))

  const submenuWidth = 200
  let submenuLeft = left + menuWidth + 4
  let submenuTop = workflowBtnTop || top
  if (showWorkflowSubmenu) {
    if (submenuLeft + submenuWidth > window.innerWidth - 8) {
      submenuLeft = left - submenuWidth - 4
    }
    const subHeight = workflowSubmenuItems.length * 32 + 16
    submenuTop = Math.max(8, Math.min(submenuTop, window.innerHeight - subHeight - 8))
  }

  return createPortal(
    <AnimatePresence>
      <motion.div
        ref={menuRef}
        role="menu"
        initial={{ opacity: 0, y: -4, scale: 0.96 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ type: 'spring', stiffness: 500, damping: 30 }}
        className="fixed z-[150] rounded-lg border border-white/[0.1] py-1 shadow-2xl"
        style={{ top, left, background: '#1e1e22', minWidth: menuWidth }}
      >
        <button
          role="menuitem"
          onClick={handleCopy}
          disabled={!selection}
          className="w-full flex items-center gap-2.5 px-3 py-1.5 text-xs text-gray-300
                     hover:bg-white/[0.06] active:bg-white/[0.1] transition-colors
                     disabled:opacity-40 disabled:pointer-events-none"
        >
          <Copy size={14} className="text-gray-500" />
          <span>Copy</span>
        </button>
        <button
          role="menuitem"
          onClick={handlePaste}
          className="w-full flex items-center gap-2.5 px-3 py-1.5 text-xs text-gray-300
                     hover:bg-white/[0.06] active:bg-white/[0.1] transition-colors"
        >
          <ClipboardPaste size={14} className="text-gray-500" />
          <span>Paste</span>
        </button>

        {hasWorkflows && (
          <>
            <div className="border-t border-white/[0.06] my-1" />
            <button
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={showWorkflowSubmenu}
              onClick={(e) => {
                e.stopPropagation()
                clearHideTimeout()
                setWorkflowBtnTop(e.currentTarget.getBoundingClientRect().top)
                setShowWorkflowSubmenu((v) => !v)
              }}
              onMouseEnter={(e) => {
                clearHideTimeout()
                setWorkflowBtnTop(e.currentTarget.getBoundingClientRect().top)
                setShowWorkflowSubmenu(true)
              }}
              onMouseLeave={scheduleHide}
              className="w-full flex items-center gap-2.5 px-3 py-1.5 text-xs text-gray-300
                         hover:bg-white/[0.06] active:bg-white/[0.1] transition-colors"
            >
              <Zap size={14} className="text-gray-500" />
              <span className="flex-1 text-left">Run workflow</span>
              <ChevronRight size={11} className="text-gray-600 ml-auto shrink-0" />
            </button>
          </>
        )}
      </motion.div>

      {showWorkflowSubmenu && workflowSubmenuItems.length > 0 && (
        <motion.div
          ref={submenuRef}
          initial={{ opacity: 0, x: -4, scale: 0.96 }}
          animate={{ opacity: 1, x: 0, scale: 1 }}
          exit={{ opacity: 0, x: -4, scale: 0.96 }}
          transition={{ type: 'spring', stiffness: 500, damping: 30 }}
          role="menu"
          className="fixed z-[151] rounded-lg border border-white/[0.1] shadow-2xl py-1 max-h-[320px] overflow-y-auto"
          style={{
            top: submenuTop,
            left: submenuLeft,
            background: '#1e1e22',
            minWidth: submenuWidth
          }}
          onMouseEnter={clearHideTimeout}
          onMouseLeave={scheduleHide}
        >
          {workflowSubmenuItems.map((sub) => (
            <button
              key={sub.id}
              role="menuitem"
              onClick={(e) => {
                e.stopPropagation()
                sub.onClick()
              }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-xs text-gray-300
                         hover:bg-white/[0.06] transition-colors"
            >
              {sub.iconElement}
              <span className="flex-1 text-left truncate">{sub.label}</span>
            </button>
          ))}
        </motion.div>
      )}
    </AnimatePresence>,
    document.body
  )
}
