import { useMemo } from 'react'
import { useAppStore } from '../stores'
import type { SessionGroupConfig } from '../../shared/types'

const EMPTY: SessionGroupConfig[] = []

/**
 * Returns the groups belonging to the active workspace, in their own order.
 */
export function useWorkspaceSessionGroups(): SessionGroupConfig[] {
  const groups = useAppStore((s) => s.config?.sessionGroups)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  return useMemo(() => {
    if (!groups?.length) return EMPTY
    return groups.filter((g) => g.workspaceId === activeWorkspace).sort((a, b) => a.order - b.order)
  }, [groups, activeWorkspace])
}
