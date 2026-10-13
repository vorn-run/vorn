import { useMemo } from 'react'
import { useAppStore } from '../stores'
import { isScheduledWorkflow } from '../lib/workflow-helpers'

// Excludes scheduled/event-driven workflows: context menus only surface
// workflows a user can run manually.
export function useWorkspaceWorkflows() {
  const workflows = useAppStore((s) => s.config?.workflows)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  return useMemo(
    () =>
      (workflows ?? []).filter(
        (w) => (w.workspaceId ?? 'personal') === activeWorkspace && !isScheduledWorkflow(w)
      ),
    [workflows, activeWorkspace]
  )
}
