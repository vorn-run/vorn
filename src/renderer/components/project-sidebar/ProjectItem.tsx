import { useState, useMemo, useCallback, useEffect, useRef } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { withProgressToast } from '../../lib/progress-toast'
import { createSessionFromProject, createShellInProject } from '../../lib/session-utils'
import { ProjectIcon } from './ProjectIcon'
import { ProjectContextMenu } from './ProjectContextMenu'
import { WorktreeItem } from './WorktreeItem'
import { generateWorktreeName } from '../../lib/worktree-names'
import { SessionItem } from './SessionItem'
import {
  ChevronRight,
  Plus,
  MoreHorizontal,
  GitBranch,
  FolderGit2,
  Server,
  Terminal
} from 'lucide-react'
import type { ProjectConfig } from '../../../shared/types'
import { getProjectHostIds, getProjectRemoteHostId } from '../../../shared/types'
import { MAIN_WORKTREE_SENTINEL } from '../../stores/types'
import type { SidebarViewMode, WorktreeInfo } from '../../stores/types'
import type { SidebarSessionInfo } from './types'

const EMPTY_WORKTREES: WorktreeInfo[] = []
const EMPTY_SESSIONS: SidebarSessionInfo[] = []

export function ProjectItem({
  project,
  sessionCount,
  defaultExpanded,
  isActive,
  isCollapsed,
  worktreeSessionCounts,
  mainRepoSessionCount,
  viewMode,
  worktreeSessions,
  mainRepoSessions,
  projectSessions
}: {
  project: ProjectConfig
  sessionCount: number
  defaultExpanded: boolean
  isActive: boolean
  isCollapsed: boolean
  worktreeSessionCounts: Map<string, number>
  mainRepoSessionCount: number
  viewMode: SidebarViewMode
  worktreeSessions: Map<string, SidebarSessionInfo[]>
  mainRepoSessions: SidebarSessionInfo[]
  projectSessions: SidebarSessionInfo[]
}) {
  const setActiveProject = useAppStore((s) => s.setActiveProject)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const activeWorktreePath = useAppStore((s) => s.activeWorktreePath)
  const setActiveWorktreePath = useAppStore((s) => s.setActiveWorktreePath)
  const worktreeCache = useAppStore((s) => s.worktreeCache)
  const loadWorktrees = useAppStore((s) => s.loadWorktrees)
  const config = useAppStore((s) => s.config)
  const setEditingProject = useAppStore((s) => s.setEditingProject)
  const setAddProjectDialogOpen = useAppStore((s) => s.setAddProjectDialogOpen)
  const removeProject = useAppStore((s) => s.removeProject)
  const worktreeFilter = useAppStore((s) => s.sidebarWorktreeFilter)
  const worktreeSort = useAppStore((s) => s.sidebarWorktreeSort)

  const [isExpanded, setIsExpanded] = useState(defaultExpanded)
  const [openMenu, setOpenMenu] = useState(false)
  const [creatingSession, setCreatingSession] = useState(false)
  const [creatingTerminal, setCreatingTerminal] = useState(false)
  const [creatingWorktree, setCreatingWorktree] = useState(false)
  const [creatingMainSession, setCreatingMainSession] = useState(false)
  const [creatingMainTerminal, setCreatingMainTerminal] = useState(false)
  const creatingSessionLock = useRef(false)
  const creatingTerminalLock = useRef(false)
  const creatingWorktreeLock = useRef(false)
  const creatingMainSessionLock = useRef(false)
  const creatingMainTerminalLock = useRef(false)
  const [collapsedBranches, setCollapsedBranches] = useState<Set<string>>(new Set())
  const isRemoteProject = !!getProjectRemoteHostId(project)
  const [isGitRepo, setIsGitRepo] = useState(isRemoteProject)

  useEffect(() => {
    if (isRemoteProject) return
    window.api
      .isGitRepo(project.path)
      .then(setIsGitRepo)
      .catch(() => setIsGitRepo(false))
  }, [project.path, isRemoteProject])

  const showSessions = viewMode === 'worktrees-sessions'
  const sessionsOnly = viewMode === 'sessions'

  // Remote host labels for badge
  const remoteHosts = config?.remoteHosts
  const remoteHostLabels = useMemo(() => {
    const hostIds = getProjectHostIds(project)
    const remoteIds = hostIds.filter((id) => id !== 'local')
    if (remoteIds.length === 0 || !remoteHosts) return null
    return remoteIds
      .map((id) => remoteHosts.find((h) => h.id === id)?.label)
      .filter(Boolean) as string[]
  }, [project, remoteHosts])

  const toggleBranchCollapsed = useCallback((key: string) => {
    setCollapsedBranches((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }, [])

  const allWorktrees = worktreeCache.get(project.path) ?? EMPTY_WORKTREES
  const mainWt = allWorktrees.find((wt) => wt.isMain)
  const isMainActive = activeWorktreePath === MAIN_WORKTREE_SENTINEL && isActive

  // Narrow selector: only re-render when linked session timestamps change
  const linkedIds = useMemo(
    () =>
      sessionsOnly
        ? []
        : allWorktrees
            .filter((wt) => !wt.isMain && wt.linkedSessionId)
            .map((wt) => wt.linkedSessionId!),
    [allWorktrees, sessionsOnly]
  )
  const linkedTimestamps = useAppStore(
    useShallow(
      useCallback(
        (s) => {
          if (worktreeSort !== 'recent') return null
          const map: Record<string, number> = {}
          for (const id of linkedIds) {
            map[id] = s.terminals.get(id)?.lastOutputTimestamp ?? 0
          }
          return map
        },
        [worktreeSort, linkedIds]
      )
    )
  )

  const sortedWorktrees = useMemo(() => {
    if (sessionsOnly) return []
    let wts = allWorktrees.filter((wt) => !wt.isMain)
    if (worktreeFilter === 'active') {
      wts = wts.filter((wt) => (worktreeSessionCounts.get(wt.path) || 0) > 0)
    }
    if (worktreeSort === 'recent' && linkedTimestamps) {
      wts = [...wts].sort((a, b) => {
        const aTime = a.linkedSessionId ? (linkedTimestamps[a.linkedSessionId] ?? 0) : 0
        const bTime = b.linkedSessionId ? (linkedTimestamps[b.linkedSessionId] ?? 0) : 0
        if (bTime !== aTime) return bTime - aTime
        return a.name.localeCompare(b.name)
      })
    } else {
      wts = [...wts].sort((a, b) => a.name.localeCompare(b.name))
    }
    return wts
  }, [
    allWorktrees,
    worktreeFilter,
    worktreeSort,
    worktreeSessionCounts,
    linkedTimestamps,
    sessionsOnly
  ])

  // Load worktrees when isGitRepo resolves true while already expanded
  useEffect(() => {
    if (isExpanded && !sessionsOnly && isGitRepo) loadWorktrees(project.path)
  }, [isGitRepo]) // eslint-disable-line react-hooks/exhaustive-deps

  const toggleExpanded = () => {
    const expanding = !isExpanded
    if (expanding && !sessionsOnly && isGitRepo) loadWorktrees(project.path)
    setIsExpanded(!isExpanded)
  }

  const handleEdit = () => {
    setEditingProject(project)
    setAddProjectDialogOpen(true)
  }

  return (
    <div>
      <div className="group relative flex items-center">
        <button
          onClick={() => {
            setActiveProject(project.name)
            setFocusedTerminal(null)
          }}
          className={`flex-1 text-left px-2 py-1.5 rounded-md text-[13px] transition-colors flex items-center gap-2 ${
            isActive
              ? 'bg-white/[0.08] text-white'
              : 'text-gray-300 hover:text-white hover:bg-white/[0.04]'
          } ${isCollapsed ? 'justify-center px-0' : ''}`}
          title={isCollapsed ? project.name : undefined}
        >
          {isCollapsed ? (
            <ProjectIcon icon={project.icon} color={project.iconColor} size={22} />
          ) : (
            <div
              className="relative w-[14px] h-[14px] shrink-0"
              onClick={(e) => {
                e.stopPropagation()
                toggleExpanded()
              }}
            >
              <span className="group-hover:hidden flex items-center justify-center w-full h-full">
                <ProjectIcon icon={project.icon} color={project.iconColor} size={14} />
              </span>
              <ChevronRight
                size={12}
                strokeWidth={2.5}
                className={`hidden group-hover:block text-gray-500 transition-transform absolute top-[1px] left-[1px] ${isExpanded ? 'rotate-90' : ''}`}
              />
            </div>
          )}
          {!isCollapsed && (
            <>
              <span className="truncate">{project.name}</span>
              {remoteHostLabels && remoteHostLabels.length > 0 && (
                <Tooltip label={remoteHostLabels.join(', ')} position="right">
                  <Server
                    size={11}
                    strokeWidth={1.5}
                    className="text-blue-400/60 shrink-0 group-hover:hidden"
                  />
                </Tooltip>
              )}
              {sessionCount > 0 && !sessionsOnly && !showSessions && (
                <span className="text-gray-600 text-xs ml-auto group-hover:hidden">
                  {sessionCount}
                </span>
              )}
              <div className="hidden group-hover:flex items-center gap-0.5 ml-auto">
                <Tooltip label="New terminal" position="right">
                  <button
                    type="button"
                    aria-label="New terminal"
                    disabled={creatingTerminal}
                    onClick={(e) => {
                      e.stopPropagation()
                      if (creatingTerminalLock.current) return
                      creatingTerminalLock.current = true
                      setCreatingTerminal(true)
                      void createShellInProject(project.path, { project }).finally(() => {
                        creatingTerminalLock.current = false
                        setCreatingTerminal(false)
                      })
                    }}
                    className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors disabled:opacity-50"
                  >
                    <Terminal size={14} strokeWidth={1.5} />
                  </button>
                </Tooltip>
                <Tooltip label="New session" position="right">
                  <button
                    type="button"
                    aria-label="New session"
                    disabled={creatingSession}
                    onClick={(e) => {
                      e.stopPropagation()
                      if (creatingSessionLock.current) return
                      creatingSessionLock.current = true
                      setCreatingSession(true)
                      void withProgressToast(
                        { loading: 'Starting session…', success: 'Session started' },
                        () => createSessionFromProject(project)
                      ).finally(() => {
                        creatingSessionLock.current = false
                        setCreatingSession(false)
                      })
                    }}
                    className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors disabled:opacity-50"
                  >
                    <Plus size={14} strokeWidth={2} />
                  </button>
                </Tooltip>
                {isGitRepo && (
                  <Tooltip label="New worktree" position="right">
                    <button
                      type="button"
                      aria-label="New worktree"
                      disabled={creatingWorktree}
                      onClick={(e) => {
                        e.stopPropagation()
                        if (creatingWorktreeLock.current) return
                        creatingWorktreeLock.current = true
                        const name = generateWorktreeName()
                        setCreatingWorktree(true)
                        void withProgressToast(
                          {
                            loading: 'Creating worktree…',
                            success: `Worktree "${name}" created`
                          },
                          async () => {
                            await window.api.createWorktree(project.path, 'main', name)
                            loadWorktrees(project.path, true)
                            if (!isExpanded) setIsExpanded(true)
                          }
                        ).finally(() => {
                          creatingWorktreeLock.current = false
                          setCreatingWorktree(false)
                        })
                      }}
                      className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors disabled:opacity-50"
                    >
                      <FolderGit2 size={14} strokeWidth={1.5} className="text-ink-secondary" />
                    </button>
                  </Tooltip>
                )}
                <Tooltip label="More" position="right">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation()
                      setOpenMenu(!openMenu)
                    }}
                    className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors"
                  >
                    <MoreHorizontal size={14} strokeWidth={2} />
                  </button>
                </Tooltip>
              </div>
            </>
          )}
        </button>
        {!isCollapsed && openMenu && (
          <div className="relative">
            <ProjectContextMenu
              project={project}
              onEdit={handleEdit}
              onDelete={() => removeProject(project.name)}
              onClose={() => setOpenMenu(false)}
            />
          </div>
        )}
      </div>

      {!isCollapsed && isExpanded && (
        <div className="ml-2 space-y-0">
          {sessionsOnly ? (
            projectSessions.length > 0 ? (
              projectSessions.map((s) => <SessionItem key={s.id} session={s} />)
            ) : (
              <p className="text-[11px] text-gray-600 px-2 py-1">No active sessions</p>
            )
          ) : !isGitRepo ? (
            showSessions || sessionsOnly ? (
              projectSessions.length > 0 ? (
                projectSessions.map((s) => <SessionItem key={s.id} session={s} />)
              ) : (
                <p className="text-[11px] text-gray-600 px-2 py-1">No active sessions</p>
              )
            ) : null
          ) : (
            <>
              {mainWt && (
                <>
                  <div className="group/main flex items-center">
                    <button
                      onClick={() => {
                        setActiveProject(project.name)
                        setActiveWorktreePath(isMainActive ? null : MAIN_WORKTREE_SENTINEL)
                        setFocusedTerminal(null)
                      }}
                      className={`flex-1 text-left px-2 py-1.5 rounded-md text-[13px] flex items-center gap-2 min-w-0 transition-colors ${
                        isMainActive
                          ? 'bg-white/[0.08] text-white'
                          : 'text-gray-400 hover:text-white hover:bg-white/[0.04]'
                      }`}
                    >
                      {showSessions ? (
                        <div
                          role="button"
                          tabIndex={0}
                          aria-expanded={!collapsedBranches.has('__main__')}
                          aria-label="Toggle sessions"
                          className="relative w-[14px] h-[14px] shrink-0"
                          onClick={(e) => {
                            e.stopPropagation()
                            toggleBranchCollapsed('__main__')
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter' || e.key === ' ') {
                              e.stopPropagation()
                              e.preventDefault()
                              toggleBranchCollapsed('__main__')
                            }
                          }}
                        >
                          <span className="group-hover/main:hidden flex items-center justify-center w-full h-full">
                            <GitBranch size={14} className="text-gray-500" strokeWidth={1.5} />
                          </span>
                          <ChevronRight
                            size={12}
                            strokeWidth={2.5}
                            className={`hidden group-hover/main:block text-gray-500 transition-transform absolute top-[1px] left-[1px] ${!collapsedBranches.has('__main__') ? 'rotate-90' : ''}`}
                          />
                        </div>
                      ) : (
                        <GitBranch size={14} className="text-gray-500 shrink-0" strokeWidth={1.5} />
                      )}
                      <span className="truncate">{mainWt.branch}</span>
                      {mainRepoSessionCount > 0 && !showSessions && (
                        <span className="text-gray-600 text-xs ml-auto group-hover/main:hidden shrink-0">
                          {mainRepoSessionCount}
                        </span>
                      )}
                      <div className="hidden group-hover/main:flex items-center gap-0.5 ml-auto">
                        <Tooltip label="New terminal" position="right">
                          <button
                            type="button"
                            aria-label="New terminal"
                            disabled={creatingMainTerminal}
                            onClick={(e) => {
                              e.stopPropagation()
                              if (creatingMainTerminalLock.current) return
                              creatingMainTerminalLock.current = true
                              setCreatingMainTerminal(true)
                              void createShellInProject(mainWt.path, {
                                project,
                                worktreePath: mainWt.path,
                                worktreeName: mainWt.name,
                                branch: mainWt.branch
                              }).finally(() => {
                                creatingMainTerminalLock.current = false
                                setCreatingMainTerminal(false)
                              })
                            }}
                            className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors disabled:opacity-50"
                          >
                            <Terminal size={14} strokeWidth={1.5} />
                          </button>
                        </Tooltip>
                        <Tooltip label="New session" position="right">
                          <button
                            type="button"
                            aria-label="New session"
                            disabled={creatingMainSession}
                            onClick={(e) => {
                              e.stopPropagation()
                              if (creatingMainSessionLock.current) return
                              creatingMainSessionLock.current = true
                              setCreatingMainSession(true)
                              void withProgressToast(
                                { loading: 'Starting session…', success: 'Session started' },
                                () => createSessionFromProject(project, { branch: mainWt.branch })
                              ).finally(() => {
                                creatingMainSessionLock.current = false
                                setCreatingMainSession(false)
                              })
                            }}
                            className="text-gray-500 hover:text-white p-0.5 rounded hover:bg-white/[0.08] transition-colors disabled:opacity-50"
                          >
                            <Plus size={14} strokeWidth={2} />
                          </button>
                        </Tooltip>
                      </div>
                    </button>
                  </div>
                  {showSessions &&
                    !collapsedBranches.has('__main__') &&
                    mainRepoSessions.map((s) => (
                      <div key={s.id} className="ml-4">
                        <SessionItem session={s} showBranch={false} />
                      </div>
                    ))}
                </>
              )}
              {sortedWorktrees.map((wt) => (
                <div key={wt.path}>
                  <WorktreeItem
                    worktree={wt}
                    projectPath={project.path}
                    projectName={project.name}
                    isActiveWorktree={activeWorktreePath === wt.path}
                    sessionCount={showSessions ? 0 : worktreeSessionCounts.get(wt.path) || 0}
                    onSelect={() => {
                      setActiveProject(project.name)
                      setActiveWorktreePath(activeWorktreePath === wt.path ? null : wt.path)
                      setFocusedTerminal(null)
                    }}
                    onWorktreesChanged={() => loadWorktrees(project.path, true)}
                    sessionsExpanded={showSessions ? !collapsedBranches.has(wt.path) : undefined}
                    onToggleSessionsExpanded={
                      showSessions ? () => toggleBranchCollapsed(wt.path) : undefined
                    }
                  />
                  {showSessions &&
                    !collapsedBranches.has(wt.path) &&
                    (worktreeSessions.get(wt.path) ?? EMPTY_SESSIONS).map((s) => (
                      <div key={s.id} className="ml-4">
                        <SessionItem session={s} showBranch={false} />
                      </div>
                    ))}
                </div>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  )
}
