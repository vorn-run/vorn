import { useMemo, useEffect } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { MAIN_WORKTREE_SENTINEL, type SortMode, type TerminalState } from '../stores/types'

/**
 * Stable comparator for terminal ids under the active sortMode. Manual mode
 * pushes ids missing from `terminalOrder` to the end (rather than producing
 * `Infinity - Infinity = NaN`, which is undefined behavior for Array#sort).
 */
export function compareTerminalIds(
  aId: string,
  bId: string,
  terminals: Map<string, TerminalState>,
  sortMode: SortMode,
  terminalOrder: string[]
): number {
  const aState = terminals.get(aId)
  const bState = terminals.get(bId)
  if (!aState || !bState) {
    if (!aState && !bState) return 0
    return aState ? -1 : 1
  }
  switch (sortMode) {
    case 'created':
      return bState.session.createdAt - aState.session.createdAt
    case 'recent':
      return bState.lastOutputTimestamp - aState.lastOutputTimestamp
    case 'manual':
    default: {
      const ia = terminalOrder.indexOf(aId)
      const ib = terminalOrder.indexOf(bId)
      if (ia === -1 && ib === -1) return 0
      if (ia === -1) return 1
      if (ib === -1) return -1
      return ia - ib
    }
  }
}

export function useVisibleTerminals(): { orderedIds: string[]; minimizedIds: string[] } {
  const {
    terminals,
    activeProject,
    activeWorktreePath,
    activeWorkspace,
    projects,
    sortMode,
    statusFilter,
    terminalOrder,
    minimizedTerminals,
    setVisibleTerminalIds,
    setFocusableTerminalIds
  } = useAppStore(
    useShallow((s) => ({
      terminals: s.terminals,
      activeProject: s.activeProject,
      activeWorktreePath: s.activeWorktreePath,
      activeWorkspace: s.activeWorkspace,
      projects: s.config?.projects,
      sortMode: s.sortMode,
      statusFilter: s.statusFilter,
      terminalOrder: s.terminalOrder,
      minimizedTerminals: s.minimizedTerminals,
      setVisibleTerminalIds: s.setVisibleTerminalIds,
      setFocusableTerminalIds: s.setFocusableTerminalIds
    }))
  )

  const workspaceProjects = useMemo(() => {
    if (!projects) return null
    return new Set(
      projects.filter((p) => (p.workspaceId ?? 'personal') === activeWorkspace).map((p) => p.name)
    )
  }, [projects, activeWorkspace])

  const { orderedIds, minimizedIds, focusableIds } = useMemo(() => {
    const inActiveScope = (t: TerminalState): boolean => {
      if (activeProject && t.session.projectName !== activeProject) return false
      if (!activeProject && workspaceProjects && !workspaceProjects.has(t.session.projectName))
        return false
      return true
    }
    const sortFn = ([aId]: [string, TerminalState], [bId]: [string, TerminalState]): number =>
      compareTerminalIds(aId, bId, terminals, sortMode, terminalOrder)
    const all = Array.from(terminals.entries())
    const filtered = all
      .filter(([, t]) => {
        if (!inActiveScope(t)) return false
        if (activeWorktreePath) {
          if (activeWorktreePath === MAIN_WORKTREE_SENTINEL) {
            if (t.session.worktreePath) return false
          } else if (t.session.worktreePath !== activeWorktreePath) return false
        }
        if (statusFilter !== 'all' && t.status !== statusFilter) return false
        return true
      })
      .sort(sortFn)

    // Sessions only. A session's child panes render inside its own card, so
    // they are not layout units here — putting them in this list once forced
    // every consumer to translate between grid positions and session positions.
    const ordered: string[] = []
    const minimized: string[] = []
    for (const [id] of filtered) {
      if (minimizedTerminals.has(id)) minimized.push(id)
      else ordered.push(id)
    }

    // Focused-mode nav spans the active project (or workspace) regardless of
    // worktree filter or status filter, so cycling sessions reaches all of them.
    const focusable = all
      .filter(([, t]) => inActiveScope(t))
      .sort(sortFn)
      .map(([id]) => id)

    return { orderedIds: ordered, minimizedIds: minimized, focusableIds: focusable }
  }, [
    terminals,
    activeProject,
    activeWorktreePath,
    workspaceProjects,
    statusFilter,
    sortMode,
    terminalOrder,
    minimizedTerminals
  ])

  useEffect(() => {
    setVisibleTerminalIds(orderedIds)
    const sel = useAppStore.getState().selectedTerminalId
    if (sel && !orderedIds.includes(sel)) {
      useAppStore.getState().setSelectedTerminal(null)
    }
  }, [orderedIds, setVisibleTerminalIds])

  useEffect(() => {
    setFocusableTerminalIds(focusableIds)
  }, [focusableIds, setFocusableTerminalIds])

  return { orderedIds, minimizedIds }
}
