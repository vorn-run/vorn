import { useEffect, useRef, useState, useCallback } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { FolderGit2, GitBranch, Plus, ChevronRight, Terminal, Zap } from 'lucide-react'
import { useAppStore } from '../stores'
import { type ProjectConfig, type AiAgentType } from '../../shared/types'
import { ProjectIcon } from './project-sidebar/ProjectIcon'
import { AgentIcon } from './AgentIcon'
import { buildWorkflowMenuItems } from '../lib/workflow-menu-items'
import {
  createSessionFromProject,
  createShellInProject,
  countSessionsByWorktree,
  formatSessionCount
} from '../lib/session-utils'
import { useWorkspaceProjects } from '../hooks/useWorkspaceProjects'
import { useWorkspaceWorkflows } from '../hooks/useWorkspaceWorkflows'

interface Props {
  position: { x: number; y: number }
  onClose: () => void
}

interface SubmenuItem {
  iconElement?: React.ReactNode
  label: string
  detail?: string
  onClick?: () => void
  separator?: boolean
  isHeader?: boolean
}

type SubmenuKey = 'session-in' | 'terminal-in' | 'run-workflow'

interface MenuItem {
  icon?: React.FC<{ size?: number; className?: string }>
  iconElement?: React.ReactNode
  label: string
  onClick?: () => void
  className?: string
  separator?: boolean
  shortcut?: string
  submenuKey?: SubmenuKey
  onSubmenuEnter?: () => void
}

const MENU_WIDTH = 220
const SUBMENU_WIDTH = 240

function estimatePanelHeight(items: { separator?: boolean }[]): number {
  const seps = items.filter((i) => i.separator).length
  return items.length * 32 + seps * 9 + 16
}

export function GridContextMenu({ position, onClose }: Props) {
  const menuRef = useRef<HTMLDivElement>(null)
  const submenuRef = useRef<HTMLDivElement>(null)
  const itemRefs = useRef<Map<number, HTMLButtonElement>>(new Map())
  const hideTimeout = useRef<ReturnType<typeof setTimeout> | null>(null)

  const worktreeCache = useAppStore((s) => s.worktreeCache)
  const loadWorktrees = useAppStore((s) => s.loadWorktrees)
  const workspaceProjects = useWorkspaceProjects()
  const workspaceWorkflows = useWorkspaceWorkflows()

  const didRefreshRef = useRef(false)
  useEffect(() => {
    if (!didRefreshRef.current && workspaceProjects.length > 0) {
      didRefreshRef.current = true
      workspaceProjects.forEach((p) => loadWorktrees(p.path, true))
    }
  }, [workspaceProjects, loadWorktrees])

  const [hoveredSubmenu, setHoveredSubmenu] = useState<number | null>(null)

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

  const terminals = useAppStore.getState().terminals

  const defaultAgent: AiAgentType = useAppStore.getState().config?.defaults.defaultAgent ?? 'claude'

  const createSession = (
    p: ProjectConfig,
    opts: {
      branch?: string
      existingWorktreePath?: string
      useWorktree?: boolean
    } = {}
  ): void => {
    onClose()
    void createSessionFromProject(p, opts)
  }

  const buildScopedSubmenu = (mode: 'session' | 'terminal'): SubmenuItem[] => {
    const subs: SubmenuItem[] = []
    const sessionCountByPath = countSessionsByWorktree(terminals.values())
    const onClickFor = (
      p: ProjectConfig,
      wtPath?: string,
      branch?: string,
      wtName?: string
    ): (() => void) => {
      if (mode === 'session') {
        return () =>
          wtPath ? createSession(p, { branch, existingWorktreePath: wtPath }) : createSession(p)
      }
      return () => {
        onClose()
        void createShellInProject(wtPath ?? p.path, {
          project: p,
          worktreePath: wtPath,
          worktreeName: wtName,
          branch
        })
      }
    }

    for (const p of workspaceProjects) {
      const worktrees = worktreeCache.get(p.path)
      const mainWt = worktrees?.find((wt) => wt.isMain)
      const nonMain = (worktrees ?? []).filter((wt) => !wt.isMain)
      const hasWorktrees = mainWt || nonMain.length > 0

      if (!hasWorktrees) {
        subs.push({
          iconElement: <ProjectIcon icon={p.icon} color={p.iconColor} size={12} />,
          label: p.name,
          onClick: onClickFor(p),
          separator: subs.length > 0
        })
        continue
      }

      subs.push({
        iconElement: <ProjectIcon icon={p.icon} color={p.iconColor} size={12} />,
        label: p.name,
        isHeader: true,
        separator: subs.length > 0
      })

      if (mainWt) {
        subs.push({
          iconElement: <GitBranch size={12} className="text-gray-400" />,
          label: mainWt.branch,
          detail: formatSessionCount(sessionCountByPath.get(mainWt.path) ?? 0),
          onClick: onClickFor(p, mainWt.path, mainWt.branch, mainWt.name)
        })
      }
      for (const wt of nonMain) {
        subs.push({
          iconElement: <FolderGit2 size={12} className="text-amber-400/70" />,
          label: wt.name,
          detail: formatSessionCount(sessionCountByPath.get(wt.path) ?? 0),
          onClick: onClickFor(p, wt.path, wt.branch, wt.name)
        })
      }

      if (mode === 'session') {
        subs.push({
          iconElement: <Plus size={12} className="text-gray-500" />,
          label: 'New worktree',
          onClick: () => {
            onClose()
            const agentType = useAppStore.getState().config?.defaults.defaultAgent ?? 'claude'
            window.api.listBranches(p.path).then((result) => {
              const branch = result.current || 'main'
              window.api
                .createTerminal({
                  agentType,
                  projectName: p.name,
                  projectPath: p.path,
                  branch,
                  useWorktree: true
                })
                .then((session) => useAppStore.getState().addTerminal(session))
            })
          }
        })
      }
    }
    return subs
  }

  const items: MenuItem[] = []

  if (workspaceProjects.length > 0) {
    items.push({
      iconElement: <AgentIcon agentType={defaultAgent} size={14} />,
      label: 'New session in…',
      submenuKey: 'session-in',
      onSubmenuEnter: () => workspaceProjects.forEach((p) => loadWorktrees(p.path))
    })

    items.push({
      iconElement: <Terminal size={14} className="text-gray-400" />,
      label: 'New terminal in…',
      submenuKey: 'terminal-in',
      onSubmenuEnter: () => workspaceProjects.forEach((p) => loadWorktrees(p.path))
    })
  }

  items.push({
    icon: Plus,
    label: 'New session...',
    shortcut: '⌘N',
    onClick: () => {
      onClose()
      useAppStore.getState().setNewAgentDialogOpen(true)
    },
    separator: workspaceProjects.length > 0
  })

  // Empty grid space has no card / session under cursor — show only
  // non-contextual workflows (contextual ones are listed in card and
  // terminal right-click menus). buildWorkflowMenuItems handles the filter
  // when called without a context argument.
  const gridWorkflowItems = buildWorkflowMenuItems(workspaceWorkflows, onClose)
  if (gridWorkflowItems.length > 0) {
    items.push({
      iconElement: <Zap size={14} className="text-gray-500" />,
      label: 'Run workflow',
      submenuKey: 'run-workflow'
    })
  }

  const hasSubmenu = (item: MenuItem): boolean => item.submenuKey !== undefined

  const menuHeight = estimatePanelHeight(items)
  const left = Math.max(8, Math.min(position.x, window.innerWidth - MENU_WIDTH - 8))
  const top = Math.max(8, Math.min(position.y, window.innerHeight - menuHeight - 8))

  const hoveredItem = hoveredSubmenu !== null ? items[hoveredSubmenu] : null
  const activeSubmenu = hoveredItem?.submenuKey
    ? hoveredItem.submenuKey === 'run-workflow'
      ? gridWorkflowItems
      : buildScopedSubmenu(hoveredItem.submenuKey === 'session-in' ? 'session' : 'terminal')
    : null

  let submenuLeft = left + MENU_WIDTH + 4
  let submenuTop = top
  if (hoveredSubmenu !== null) {
    // eslint-disable-next-line react-hooks/refs
    const itemEl = itemRefs.current.get(hoveredSubmenu)
    if (itemEl) submenuTop = itemEl.getBoundingClientRect().top
    if (submenuLeft + SUBMENU_WIDTH > window.innerWidth - 8) {
      submenuLeft = left - SUBMENU_WIDTH - 4
    }
    if (activeSubmenu) {
      const subHeight = estimatePanelHeight(activeSubmenu)
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
        className="fixed z-[150] rounded-lg border border-white/[0.1] shadow-2xl py-1"
        style={{ top, left, background: '#1e1e22', minWidth: MENU_WIDTH }}
      >
        {items.map((item, i) => {
          const itemHasSubmenu = hasSubmenu(item)
          return (
            <div key={i}>
              {item.separator && <div className="border-t border-white/[0.06] my-1" />}
              <button
                ref={(el) => {
                  if (el) itemRefs.current.set(i, el)
                  else itemRefs.current.delete(i)
                }}
                onClick={(e) => {
                  e.stopPropagation()
                  if (itemHasSubmenu) {
                    if (hoveredSubmenu === i) {
                      setHoveredSubmenu(null)
                    } else {
                      clearHideTimeout()
                      setHoveredSubmenu(i)
                      item.onSubmenuEnter?.()
                    }
                    return
                  }
                  item.onClick?.()
                }}
                onMouseEnter={() => {
                  if (itemHasSubmenu) {
                    clearHideTimeout()
                    setHoveredSubmenu(i)
                    item.onSubmenuEnter?.()
                  } else {
                    scheduleHide()
                  }
                }}
                onMouseLeave={() => {
                  if (itemHasSubmenu) scheduleHide()
                }}
                aria-haspopup={itemHasSubmenu ? 'menu' : undefined}
                aria-expanded={itemHasSubmenu ? hoveredSubmenu === i : undefined}
                className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-xs ${item.className ?? 'text-gray-300'} hover:bg-white/[0.06] transition-colors`}
              >
                {item.iconElement ??
                  (item.icon && (
                    <item.icon size={14} className={item.className ?? 'text-gray-500'} />
                  ))}
                <span className="flex-1 text-left truncate">{item.label}</span>
                {item.shortcut && (
                  <span className="text-[10px] text-gray-600 ml-auto shrink-0">
                    {item.shortcut}
                  </span>
                )}
                {itemHasSubmenu && (
                  <ChevronRight size={11} className="text-gray-600 ml-auto shrink-0" />
                )}
              </button>
            </div>
          )
        })}
      </motion.div>

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
            minWidth: SUBMENU_WIDTH
          }}
          onMouseEnter={clearHideTimeout}
          onMouseLeave={scheduleHide}
        >
          {activeSubmenu.map((sub, j) => (
            <div key={j}>
              {sub.separator && <div className="border-t border-white/[0.06] my-1" />}
              {sub.isHeader ? (
                <div className="flex items-center gap-2 px-3 pt-2 pb-1 text-[10px] font-medium text-gray-500 uppercase tracking-wider">
                  {sub.iconElement}
                  {sub.label}
                </div>
              ) : (
                <button
                  role="menuitem"
                  onClick={(e) => {
                    e.stopPropagation()
                    sub.onClick?.()
                  }}
                  className="w-full flex items-center gap-2.5 px-3 pl-7 py-1.5 text-xs text-gray-300
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
              )}
            </div>
          ))}
        </motion.div>
      )}
    </AnimatePresence>,
    document.body
  )
}
