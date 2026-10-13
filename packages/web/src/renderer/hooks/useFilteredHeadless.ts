import { useMemo } from 'react'
import { useAppStore } from '../stores'
import type { HeadlessSession } from '../../shared/types'

const EMPTY: HeadlessSession[] = []

/**
 * Filters headless sessions by visibility setting, active project/workspace, and status filter.
 */
export function useFilteredHeadless(): HeadlessSession[] {
  const headlessSessions = useAppStore((s) => s.headlessSessions)
  const showHeadless = useAppStore((s) => s.config?.defaults?.showHeadlessAgents !== false)
  const activeProject = useAppStore((s) => s.activeProject)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  const projects = useAppStore((s) => s.config?.projects)
  const statusFilter = useAppStore((s) => s.statusFilter)

  const workspaceProjects = useMemo(() => {
    if (!projects) return null
    return new Set(
      projects.filter((p) => (p.workspaceId ?? 'personal') === activeWorkspace).map((p) => p.name)
    )
  }, [projects, activeWorkspace])

  return useMemo(() => {
    if (!showHeadless || headlessSessions.length === 0) return EMPTY
    return headlessSessions.filter((s) => {
      if (activeProject && s.projectName !== activeProject) return false
      if (!activeProject && workspaceProjects && !workspaceProjects.has(s.projectName)) return false
      if (statusFilter !== 'all') {
        const mapped = s.status === 'running' ? 'running' : s.exitCode !== 0 ? 'error' : 'idle'
        if (mapped !== statusFilter) return false
      }
      return true
    })
  }, [headlessSessions, showHeadless, activeProject, workspaceProjects, statusFilter])
}
