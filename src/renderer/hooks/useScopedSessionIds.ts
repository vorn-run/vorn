import { useMemo } from 'react'
import { useAppStore } from '../stores'

/**
 * Which sessions the current selection covers, or null for "no restriction".
 *
 * Selection is one value with three states — all, a group, a single project.
 * A group cuts across the project tree, so it can only be answered in session
 * ids; a project selection is still answered by project name, which is why both
 * shapes come back from here rather than one.
 */
export function useSessionScope(): {
  projectNames: Set<string> | null
  sessionIds: Set<string> | null
} {
  const activeProject = useAppStore((s) => s.activeProject)
  const activeGroupId = useAppStore((s) => s.activeGroupId)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  const projects = useAppStore((s) => s.config?.projects)
  const groups = useAppStore((s) => s.config?.sessionGroups)
  const terminals = useAppStore((s) => s.terminals)

  return useMemo(() => {
    const workspaceNames = projects
      ? new Set(
          projects
            .filter((p) => (p.workspaceId ?? 'personal') === activeWorkspace)
            .map((p) => p.name)
        )
      : null

    // A selection left over from another workspace, or a group whose sessions
    // have all ended, narrows to nothing — so it is read as no selection rather
    // than blanking the board.
    if (activeProject && workspaceNames?.has(activeProject)) {
      return { projectNames: new Set([activeProject]), sessionIds: null }
    }
    // A group belongs to one workspace, and so does every session it can show.
    const group = (groups ?? []).find(
      (g) => g.id === activeGroupId && g.workspaceId === activeWorkspace
    )
    if (group) {
      const ids = new Set<string>()
      for (const [id, t] of terminals) {
        if (t.session.groupId !== group.id) continue
        if (workspaceNames && !workspaceNames.has(t.session.projectName)) continue
        ids.add(id)
      }
      if (ids.size > 0) return { projectNames: workspaceNames, sessionIds: ids }
    }
    return { projectNames: workspaceNames, sessionIds: null }
  }, [activeProject, activeGroupId, activeWorkspace, projects, groups, terminals])
}
