import { useMemo } from 'react'
import { useAppStore } from '../stores'

/**
 * Returns the projects belonging to the active workspace.
 */
export function useWorkspaceProjects() {
  const projects = useAppStore((s) => s.config?.projects)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  return useMemo(
    () => (projects ?? []).filter((p) => (p.workspaceId ?? 'personal') === activeWorkspace),
    [projects, activeWorkspace]
  )
}
