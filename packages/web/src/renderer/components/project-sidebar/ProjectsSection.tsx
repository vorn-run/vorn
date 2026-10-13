import { useState, useMemo, useCallback } from 'react'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { ProjectItem } from './ProjectItem'
import { ProjectsSectionToolbar } from './ProjectsSectionToolbar'
import { SidebarNavItem } from './SidebarNavItem'
import { SidebarSectionHeader } from './SidebarSectionHeader'
import { FolderPlus, Layers } from 'lucide-react'
import type { ProjectConfig } from '../../../shared/types'
import type { SidebarSessionInfo } from './types'

const EMPTY_SESSIONS: SidebarSessionInfo[] = []

export function ProjectsSection({
  isCollapsed,
  workspaceProjects,
  projectTerminals,
  worktreeSessionCounts,
  mainRepoSessionCounts,
  workspaceTerminalCount,
  worktreeSessions,
  mainRepoSessions
}: {
  isCollapsed: boolean
  workspaceProjects: ProjectConfig[]
  projectTerminals: Map<string, SidebarSessionInfo[]>
  worktreeSessionCounts: Map<string, number>
  mainRepoSessionCounts: Map<string, number>
  workspaceTerminalCount: number
  worktreeSessions: Map<string, SidebarSessionInfo[]>
  mainRepoSessions: Map<string, SidebarSessionInfo[]>
}) {
  const activeProject = useAppStore((s) => s.activeProject)
  const setActiveProject = useAppStore((s) => s.setActiveProject)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const setAddProjectDialogOpen = useAppStore((s) => s.setAddProjectDialogOpen)
  const sidebarViewMode = useAppStore((s) => s.sidebarViewMode)
  const sidebarProjectSort = useAppStore((s) => s.sidebarProjectSort)
  const worktreeFilter = useAppStore((s) => s.sidebarWorktreeFilter)
  const reorderProjects = useAppStore((s) => s.reorderProjects)
  const terminals = useAppStore((s) => s.terminals)

  const [sectionCollapsed, setSectionCollapsed] = useState(false)
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null)
  const [dragSourceIndex, setDragSourceIndex] = useState<number | null>(null)

  const iconSize = isCollapsed ? 22 : 14

  const projectLastActivity = useMemo(() => {
    if (sidebarProjectSort !== 'recent') return null
    const map = new Map<string, number>()
    for (const t of terminals.values()) {
      const cur = map.get(t.session.projectName) ?? 0
      if (t.lastOutputTimestamp > cur) map.set(t.session.projectName, t.lastOutputTimestamp)
    }
    return map
  }, [sidebarProjectSort, terminals])

  const sortedProjects = useMemo(() => {
    let projects = workspaceProjects
    if (worktreeFilter === 'active') {
      projects = projects.filter((p) => (projectTerminals.get(p.name)?.length ?? 0) > 0)
    }
    if (sidebarProjectSort === 'manual') return projects
    if (sidebarProjectSort === 'name') {
      return [...projects].sort((a, b) => a.name.localeCompare(b.name))
    }
    return [...projects].sort((a, b) => {
      const aMax = projectLastActivity?.get(a.name) ?? 0
      const bMax = projectLastActivity?.get(b.name) ?? 0
      return bMax - aMax
    })
  }, [workspaceProjects, sidebarProjectSort, projectLastActivity, worktreeFilter, projectTerminals])

  const handleDragStart = useCallback(
    (e: React.DragEvent, index: number) => {
      if (sidebarProjectSort !== 'manual') return
      setDragSourceIndex(index)
      e.dataTransfer.effectAllowed = 'move'
      e.dataTransfer.setData('text/plain', String(index))
    },
    [sidebarProjectSort]
  )

  const handleDragOver = useCallback(
    (e: React.DragEvent, index: number) => {
      if (sidebarProjectSort !== 'manual') return
      e.preventDefault()
      e.dataTransfer.dropEffect = 'move'
      setDragOverIndex(index)
    },
    [sidebarProjectSort]
  )

  const handleDrop = useCallback(
    (e: React.DragEvent, toIndex: number) => {
      e.preventDefault()
      if (sidebarProjectSort !== 'manual') return
      const fromIndex = parseInt(e.dataTransfer.getData('text/plain'), 10)
      if (!isNaN(fromIndex) && fromIndex !== toIndex) {
        reorderProjects(fromIndex, toIndex)
      }
      setDragSourceIndex(null)
      setDragOverIndex(null)
    },
    [reorderProjects, sidebarProjectSort]
  )

  const handleDragEnd = useCallback(() => {
    setDragSourceIndex(null)
    setDragOverIndex(null)
  }, [])

  return (
    <>
      <SidebarSectionHeader
        title="Projects"
        isCollapsed={isCollapsed}
        sectionCollapsed={sectionCollapsed}
        onToggle={() => setSectionCollapsed(!sectionCollapsed)}
        actions={
          <>
            <ProjectsSectionToolbar />
            <Tooltip label="Add project" position="bottom">
              <button
                onClick={() => setAddProjectDialogOpen(true)}
                className="p-0.5 rounded text-gray-600 hover:text-white hover:bg-white/[0.08] transition-colors"
              >
                <FolderPlus size={13} strokeWidth={1.5} />
              </button>
            </Tooltip>
          </>
        }
      />

      {!sectionCollapsed && (
        <SidebarNavItem
          isActive={activeProject === null}
          isCollapsed={isCollapsed}
          icon={<Layers size={iconSize} strokeWidth={1.5} />}
          label="All Projects"
          badge={workspaceTerminalCount}
          onClick={() => {
            setActiveProject(null)
            setFocusedTerminal(null)
          }}
        />
      )}
      {!isCollapsed && !sectionCollapsed && workspaceProjects.length === 0 && (
        <p className="text-[13px] text-gray-600 px-2.5 py-1">No projects</p>
      )}

      {!sectionCollapsed &&
        sortedProjects.map((project, index) => {
          const sessionCount = (projectTerminals.get(project.name) ?? EMPTY_SESSIONS).length
          const isManual = sidebarProjectSort === 'manual'
          return (
            <div
              key={project.name}
              draggable={isManual && !isCollapsed}
              onDragStart={(e) => handleDragStart(e, index)}
              onDragOver={(e) => handleDragOver(e, index)}
              onDrop={(e) => handleDrop(e, index)}
              onDragEnd={handleDragEnd}
              className={`${isManual && !isCollapsed ? 'cursor-grab active:cursor-grabbing' : ''} ${
                dragOverIndex === index && dragSourceIndex !== index
                  ? 'border-t-2 border-white/40'
                  : 'border-t-2 border-transparent'
              }`}
            >
              <ProjectItem
                project={project}
                sessionCount={sessionCount}
                defaultExpanded={sessionCount > 0}
                isActive={activeProject === project.name}
                isCollapsed={isCollapsed}
                worktreeSessionCounts={worktreeSessionCounts}
                mainRepoSessionCount={mainRepoSessionCounts.get(project.name) || 0}
                viewMode={sidebarViewMode}
                worktreeSessions={worktreeSessions}
                mainRepoSessions={mainRepoSessions.get(project.name) ?? EMPTY_SESSIONS}
                projectSessions={projectTerminals.get(project.name) ?? EMPTY_SESSIONS}
              />
            </div>
          )
        })}
    </>
  )
}
