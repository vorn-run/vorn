import { useMemo, useEffect } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { MAIN_WORKTREE_SENTINEL, type SortMode, type TerminalState } from '../stores/types'
import { usePromotedCardsByOwner } from './usePromotedCards'
import { useClaimedTerminalIds } from './usePanelTerminals'

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

  // The grouping, not the two pane Maps it comes from. Those are replaced on
  // any pane write — a browser tab switch, a keystroke in the address bar —
  // which would re-run this whole memo, re-sorting every session twice and
  // triggering a reconcile pass, for a list that had not changed. The grouping
  // is a stable empty when nothing is popped out, which is the common case.
  const cardsByOwner = usePromotedCardsByOwner()
  // Terminals a session holds in its panel. They are drawn there and nowhere
  // else, so they are not layout units, not focusable, and not dock entries —
  // and the registry's one-slot-per-terminal rule depends on that staying true.
  const claimedTerminals = useClaimedTerminalIds()

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
    const all = Array.from(terminals.entries()).filter(([id]) => !claimedTerminals.has(id))
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

    // Sessions, plus every file and tab popped out of one.
    //
    // A session's panes render inside its own card and are not layout units at
    // all. A popped-out card is: it is a cell like a session, placed directly
    // after the session it came from so the two stay together as the grid
    // reflows, rather than drifting a row apart.
    const ordered: string[] = []
    const minimized: string[] = []
    const place = (id: string): void => {
      if (minimizedTerminals.has(id)) minimized.push(id)
      else ordered.push(id)
    }
    for (const [id] of filtered) {
      place(id)
      for (const cardId of cardsByOwner.get(id) ?? []) place(cardId)
    }

    // Focused-mode nav spans the active project (or workspace) regardless of
    // worktree filter or status filter, so cycling reaches all of them.
    //
    // Cards are in this list too, beside their owner. Cmd+] and Cmd+1-9 both
    // index straight into it, so a card being focusable at all is decided here
    // and nowhere else — the shortcut handlers never learn what an id is.
    const focusable: string[] = []
    for (const [id] of all.filter(([, t]) => inActiveScope(t)).sort(sortFn)) {
      focusable.push(id)
      focusable.push(...(cardsByOwner.get(id) ?? []))
    }

    return { orderedIds: ordered, minimizedIds: minimized, focusableIds: focusable }
  }, [
    terminals,
    activeProject,
    activeWorktreePath,
    workspaceProjects,
    statusFilter,
    sortMode,
    terminalOrder,
    minimizedTerminals,
    cardsByOwner,
    claimedTerminals
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
