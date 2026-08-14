import { useCallback, useMemo } from 'react'
import { useAppStore } from '../../stores'
import { useIsMobile } from '../../hooks/useIsMobile'
import { getDisplayName } from '../../lib/terminal-display'
import { useSidebarResize } from './useSidebarResize'
import { SidebarHeader } from './SidebarHeader'
import { ProjectsSection } from './ProjectsSection'
import { FlatSessionsSection } from './FlatSessionsSection'
import { WorkflowsSection } from './WorkflowsSection'
import { TasksProjectsSection } from './TasksProjectsSection'
import { SidebarFooter } from './SidebarFooter'
import type { SidebarSessionInfo } from './types'

export function ProjectSidebar() {
  const config = useAppStore((s) => s.config)
  const terminals = useAppStore((s) => s.terminals)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  const isSidebarOpen = useAppStore((s) => s.isSidebarOpen)
  const toggleSidebar = useAppStore((s) => s.toggleSidebar)
  const sidebarViewMode = useAppStore((s) => s.sidebarViewMode)
  const mainViewMode = useAppStore((s) => s.config?.defaults?.mainViewMode ?? 'sessions')
  const isMobile = useIsMobile()

  const { sidebarWidth, isResizingState, isCollapsed, handleResizeStart, handleResizeDoubleClick } =
    useSidebarResize()

  const closeSidebarOnMobile = useCallback(() => {
    if (isMobile && isSidebarOpen) toggleSidebar()
  }, [isMobile, isSidebarOpen, toggleSidebar])

  const workspaceProjects = useMemo(
    () => (config?.projects ?? []).filter((p) => (p.workspaceId ?? 'personal') === activeWorkspace),
    [config?.projects, activeWorkspace]
  )

  const workspaceProjectNames = useMemo(
    () => new Set(workspaceProjects.map((p) => p.name)),
    [workspaceProjects]
  )

  const workspaceWorkflows = useMemo(
    () =>
      (config?.workflows ?? []).filter((w) => (w.workspaceId ?? 'personal') === activeWorkspace),
    [config?.workflows, activeWorkspace]
  )

  const {
    projectTerminals,
    worktreeSessions,
    mainRepoSessions,
    worktreeSessionCounts,
    mainRepoSessionCounts
  } = useMemo(() => {
    const byProject = new Map<string, SidebarSessionInfo[]>()
    const byWorktree = new Map<string, SidebarSessionInfo[]>()
    const byMainRepo = new Map<string, SidebarSessionInfo[]>()
    const wtCounts = new Map<string, number>()
    const mainCounts = new Map<string, number>()

    for (const [id, t] of terminals) {
      const pName = t.session.projectName
      const info: SidebarSessionInfo = {
        id,
        name: getDisplayName(t.session),
        status: t.status,
        agentType: t.session.agentType,
        branch: t.session.branch,
        isWorktree: t.session.isWorktree,
        worktreePath: t.session.worktreePath
      }

      if (!byProject.has(pName)) byProject.set(pName, [])
      byProject.get(pName)!.push(info)

      if (t.session.worktreePath) {
        if (!byWorktree.has(t.session.worktreePath)) byWorktree.set(t.session.worktreePath, [])
        byWorktree.get(t.session.worktreePath)!.push(info)
        wtCounts.set(t.session.worktreePath, (wtCounts.get(t.session.worktreePath) || 0) + 1)
      } else {
        if (!byMainRepo.has(pName)) byMainRepo.set(pName, [])
        byMainRepo.get(pName)!.push(info)
        mainCounts.set(pName, (mainCounts.get(pName) || 0) + 1)
      }
    }

    return {
      projectTerminals: byProject,
      worktreeSessions: byWorktree,
      mainRepoSessions: byMainRepo,
      worktreeSessionCounts: wtCounts,
      mainRepoSessionCounts: mainCounts
    }
  }, [terminals])

  const workspaceTerminalCount = useMemo(() => {
    let count = 0
    for (const [, t] of terminals) {
      if (workspaceProjectNames.has(t.session.projectName)) count++
    }
    return count
  }, [terminals, workspaceProjectNames])

  if (!isSidebarOpen) {
    return null
  }

  const sidebarContent = (
    <aside
      role="navigation"
      aria-label="Project sidebar"
      className={`border-r border-white/[0.06] flex flex-col h-full shrink-0 relative ${
        isMobile ? 'w-[85vw] max-w-[320px]' : ''
      }`}
      style={{
        ...(!isMobile ? { width: `${sidebarWidth}px` } : {}),
        // The sidebar is chrome beside the work, the same rung as a card header
        // or the status bar. It was painted at the card-body value, so it read
        // as a lit band down the left edge of every view.
        background: 'var(--color-surface-panel)',
        transition: isResizingState ? 'none' : 'width 0.2s cubic-bezier(0.4, 0, 0.2, 1)'
      }}
    >
      <SidebarHeader isCollapsed={isCollapsed} />

      <div className={`flex-1 overflow-auto space-y-0.5 ${isCollapsed ? 'px-1.5' : 'px-3'}`}>
        {mainViewMode === 'workflows' ? (
          <WorkflowsSection isCollapsed={isCollapsed} workspaceWorkflows={workspaceWorkflows} />
        ) : mainViewMode === 'tasks' ? (
          <TasksProjectsSection isCollapsed={isCollapsed} workspaceProjects={workspaceProjects} />
        ) : sidebarViewMode === 'sessions-flat' ? (
          <FlatSessionsSection
            isCollapsed={isCollapsed}
            workspaceProjectNames={workspaceProjectNames}
            workspaceTerminalCount={workspaceTerminalCount}
          />
        ) : (
          <ProjectsSection
            isCollapsed={isCollapsed}
            workspaceProjects={workspaceProjects}
            projectTerminals={projectTerminals}
            worktreeSessionCounts={worktreeSessionCounts}
            mainRepoSessionCounts={mainRepoSessionCounts}
            workspaceTerminalCount={workspaceTerminalCount}
            worktreeSessions={worktreeSessions}
            mainRepoSessions={mainRepoSessions}
          />
        )}
      </div>

      <SidebarFooter isCollapsed={isCollapsed} closeSidebarOnMobile={closeSidebarOnMobile} />

      {!isMobile && (
        <div
          onPointerDown={handleResizeStart}
          onDoubleClick={handleResizeDoubleClick}
          className="absolute top-0 right-0 w-1.5 h-full cursor-col-resize
                     hover:bg-white/[0.08] active:bg-white/[0.12] transition-colors z-10"
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize sidebar"
        />
      )}
    </aside>
  )

  if (isMobile) {
    return (
      <div
        className="fixed inset-0 z-50 flex"
        style={{
          paddingTop: 'var(--safe-top, 0px)',
          paddingBottom: 'var(--safe-bottom, 0px)',
          paddingLeft: 'var(--safe-left, 0px)'
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget) toggleSidebar()
        }}
      >
        {sidebarContent}
        <div className="flex-1 bg-black/60" />
      </div>
    )
  }

  return sidebarContent
}
