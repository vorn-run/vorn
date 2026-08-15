import { useMemo } from 'react'
import { useAppStore } from '../stores'
import type { AppStore } from '../stores/types'

/** Stable empty, so "nothing claimed" does not invalidate every memo downstream. */
const NONE = new Set<string>()

/**
 * Every terminal currently claimed by some session's panel.
 *
 * A claimed terminal is drawn in that panel and nowhere else — it is kept out
 * of the grid, the tab strip, the sidebar, the dock and keyboard nav until it
 * is extracted. That is not only tidiness: the terminal registry is
 * last-writer-wins on a slot, so one terminal rendered in two places would have
 * the two fighting over a single wrapper, and whichever lost would go blank.
 *
 * Derived rather than stored, the way a card is derived from a pane whose key
 * is not its owner — so there is no second list to fall out of step with the
 * panels themselves.
 */
export function claimedTerminalIds(state: Pick<AppStore, 'terminalsPanes'>): Set<string> {
  if (state.terminalsPanes.size === 0) return NONE
  const claimed = new Set<string>()
  for (const pane of state.terminalsPanes.values()) {
    for (const id of pane.terminals) claimed.add(id)
  }
  return claimed
}

export function useClaimedTerminalIds(): Set<string> {
  const terminalsPanes = useAppStore((s) => s.terminalsPanes)
  // Memoized on the map, which is replaced only when a panel actually changes —
  // adding, extracting, closing, or switching which shell is in front.
  return useMemo(() => claimedTerminalIds({ terminalsPanes }), [terminalsPanes])
}
