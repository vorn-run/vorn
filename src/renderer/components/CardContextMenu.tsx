import { useEffect, useRef, useState, useCallback } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { ChevronRight, Zap, Terminal } from 'lucide-react'
import { useAppStore } from '../stores'
import { type AiAgentType, getProjectRemoteHostId } from '../../shared/types'
import { AgentIcon } from './AgentIcon'
import { AGENT_LIST } from '../lib/agent-definitions'
import { useAgentInstallStatus } from '../hooks/useAgentInstallStatus'
import { useIsMobile } from '../hooks/useIsMobile'
import { useWorkspaceWorkflows } from '../hooks/useWorkspaceWorkflows'
import { buildWorkflowMenuItems } from '../lib/workflow-menu-items'
import { createShellInProject } from '../lib/session-utils'

interface Props {
  terminalId: string
  position: { x: number; y: number }
  onClose: () => void
}

interface SubmenuItem {
  iconElement?: React.ReactNode
  label: string
  detail?: string
  onClick: () => void
  separator?: boolean
}

interface MenuItem {
  icon?: React.FC<{ size?: number; className?: string }>
  iconElement?: React.ReactNode
  label: string
  onClick?: () => void
  className?: string
  separator?: boolean
  submenu?: SubmenuItem[]
  onSubmenuEnter?: () => void
}

export function CardContextMenu({ terminalId, position, onClose }: Props) {
  const menuRef = useRef<HTMLDivElement>(null)
  const submenuRef = useRef<HTMLDivElement>(null)
  const hideTimeout = useRef<ReturnType<typeof setTimeout> | null>(null)
  const terminal = useAppStore((s) => s.terminals.get(terminalId))
  const config = useAppStore((s) => s.config)
  const isMobile = useIsMobile()
  const workspaceWorkflows = useWorkspaceWorkflows()
  const { status: agentInstallStatus } = useAgentInstallStatus()

  const [hoveredSubmenu, setHoveredSubmenu] = useState<number | null>(null)
  const [submenuItemTop, setSubmenuItemTop] = useState(0)

  const clearHideTimeout = useCallback(() => {
    if (hideTimeout.current) {
      clearTimeout(hideTimeout.current)
      hideTimeout.current = null
    }
  }, [])

  const scheduleHide = useCallback(() => {
    clearHideTimeout()
    hideTimeout.current = setTimeout(() => setHoveredSubmenu(null), 150)
  }, [clearHideTimeout])

  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (
        menuRef.current &&
        !menuRef.current.contains(e.target as Node) &&
        (!submenuRef.current || !submenuRef.current.contains(e.target as Node))
      )
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

  if (!terminal) return null

  const project = config?.projects.find((p) => p.name === terminal.session.projectName)
  const remoteHostId = project ? getProjectRemoteHostId(project) : undefined
  const projectPath = terminal.session.projectPath
  const projectName = terminal.session.projectName
  const isWorktree = terminal.session.isWorktree
  const branch = terminal.session.branch

  const items: MenuItem[] = []

  const defaultAgent = config?.defaults?.defaultAgent || 'claude'

  const createSessionWithAgent = async (agentType: AiAgentType) => {
    onClose()
    const state = useAppStore.getState()
    const session = await window.api.createTerminal({
      agentType,
      projectName,
      projectPath,
      remoteHostId,
      ...(isWorktree && terminal.session.worktreePath
        ? { branch, existingWorktreePath: terminal.session.worktreePath }
        : {})
    })
    state.addTerminal(session)
  }

  items.push({
    iconElement: <AgentIcon agentType={defaultAgent} size={14} />,
    label: 'New session',
    onClick: () => createSessionWithAgent(defaultAgent)
  })

  items.push({
    iconElement: <Terminal size={14} className="text-gray-400" />,
    label: 'New terminal',
    onClick: () => {
      onClose()
      const worktreePath = isWorktree ? terminal.session.worktreePath : undefined
      const cwd = worktreePath ?? projectPath
      void createShellInProject(cwd, {
        project,
        worktreePath,
        worktreeName: worktreePath ? terminal.session.worktreeName : undefined,
        branch: worktreePath ? branch : undefined
      })
    }
  })

  const agentSubmenuItems: SubmenuItem[] = AGENT_LIST.filter((a) => agentInstallStatus[a.type]).map(
    (agent) => ({
      iconElement: <AgentIcon agentType={agent.type} size={12} />,
      label: agent.displayName,
      onClick: () => createSessionWithAgent(agent.type)
    })
  )

  if (agentSubmenuItems.length > 1) {
    items.push({
      iconElement: <AgentIcon agentType={defaultAgent} size={14} />,
      label: 'New session with…',
      submenu: agentSubmenuItems
    })
  }

  const workflowMenuItems = buildWorkflowMenuItems(workspaceWorkflows, onClose, {
    source: terminal.session
  })
  if (workflowMenuItems.length > 0) {
    items.push({
      iconElement: <Zap size={14} className="text-gray-500" />,
      label: 'Run workflow',
      submenu: workflowMenuItems
    })
  }

  const menuWidth = 220
  const separators = items.filter((i) => i.separator).length
  const menuHeight = items.length * 32 + separators * 9 + 16
  const left = Math.max(8, Math.min(position.x, window.innerWidth - menuWidth - 8))
  const top = Math.max(8, Math.min(position.y, window.innerHeight - menuHeight - 8))

  const activeSubmenu = hoveredSubmenu !== null ? items[hoveredSubmenu]?.submenu : null

  let submenuLeft = left + menuWidth + 4
  let submenuTop = top
  const submenuWidth = 220
  if (hoveredSubmenu !== null) {
    submenuTop = submenuItemTop || top
    if (submenuLeft + submenuWidth > window.innerWidth - 8) {
      submenuLeft = left - submenuWidth - 4
    }
    if (activeSubmenu) {
      const subSeps = activeSubmenu.filter((s) => s.separator).length
      const subHeight = activeSubmenu.length * 32 + subSeps * 9 + 16
      submenuTop = Math.max(8, Math.min(submenuTop, window.innerHeight - subHeight - 8))
    }
  }

  return createPortal(
    <AnimatePresence>
      <motion.div
        ref={menuRef}
        initial={{ opacity: 0, y: -4, scale: 0.96 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: -4, scale: 0.96 }}
        transition={{ type: 'spring', stiffness: 500, damping: 30 }}
        className={`fixed z-[150] rounded-lg border border-white/[0.1] py-1 ${isMobile ? '' : 'shadow-2xl'}`}
        style={{
          top,
          left,
          background: isMobile ? 'var(--glass-bg)' : '#1e1e22',
          backdropFilter: isMobile ? 'var(--glass-blur)' : undefined,
          WebkitBackdropFilter: isMobile ? 'var(--glass-blur)' : undefined,
          boxShadow: isMobile ? 'var(--glass-shadow)' : undefined,
          minWidth: menuWidth
        }}
      >
        {items.map((item, i) => (
          <div key={i}>
            {item.separator && <div className="border-t border-white/[0.06] my-1" />}
            <button
              onClick={(e) => {
                e.stopPropagation()
                if (item.submenu) {
                  clearHideTimeout()
                  setSubmenuItemTop(e.currentTarget.getBoundingClientRect().top)
                  setHoveredSubmenu(hoveredSubmenu === i ? null : i)
                  item.onSubmenuEnter?.()
                } else {
                  item.onClick?.()
                }
              }}
              onMouseEnter={(e) => {
                if (item.submenu) {
                  clearHideTimeout()
                  setSubmenuItemTop(e.currentTarget.getBoundingClientRect().top)
                  setHoveredSubmenu(i)
                  item.onSubmenuEnter?.()
                } else {
                  scheduleHide()
                }
              }}
              onMouseLeave={() => {
                if (item.submenu) scheduleHide()
              }}
              aria-haspopup={item.submenu ? 'menu' : undefined}
              aria-expanded={item.submenu ? hoveredSubmenu === i : undefined}
              className="w-full flex items-center gap-2.5 px-3 py-1.5 text-xs text-gray-300
                         hover:bg-white/[0.06] active:bg-white/[0.1] transition-colors"
            >
              {item.iconElement ??
                (item.icon && (
                  <item.icon size={14} className={item.className ?? 'text-gray-500'} />
                ))}
              <span className="flex-1 text-left truncate">{item.label}</span>
              {item.submenu && (
                <ChevronRight size={11} className="text-gray-600 ml-auto shrink-0" />
              )}
            </button>
          </div>
        ))}
      </motion.div>

      {/* Hover submenu */}
      {activeSubmenu && (
        <motion.div
          ref={submenuRef}
          initial={{ opacity: 0, x: -4, scale: 0.96 }}
          animate={{ opacity: 1, x: 0, scale: 1 }}
          exit={{ opacity: 0, x: -4, scale: 0.96 }}
          transition={{ type: 'spring', stiffness: 500, damping: 30 }}
          role="menu"
          className="fixed z-[151] rounded-lg border border-white/[0.1] shadow-2xl py-1"
          style={{
            top: submenuTop,
            left: submenuLeft,
            background: '#1e1e22',
            minWidth: submenuWidth
          }}
          onMouseEnter={clearHideTimeout}
          onMouseLeave={scheduleHide}
        >
          {activeSubmenu.map((sub, j) => (
            <div key={j}>
              {sub.separator && <div className="border-t border-white/[0.06] my-1" />}
              <button
                role="menuitem"
                onClick={(e) => {
                  e.stopPropagation()
                  sub.onClick()
                }}
                className="w-full flex items-center gap-2.5 px-3 py-2 text-xs text-gray-300
                           hover:bg-white/[0.06] transition-colors"
              >
                {sub.iconElement}
                <span className="flex-1 text-left font-mono truncate">{sub.label}</span>
                {sub.detail && (
                  <span
                    className={`text-[10px] ml-auto shrink-0 ${
                      sub.detail !== 'idle' ? 'text-green-400/70' : 'text-gray-600'
                    }`}
                  >
                    {sub.detail}
                  </span>
                )}
              </button>
            </div>
          ))}
        </motion.div>
      )}
    </AnimatePresence>,
    document.body
  )
}
