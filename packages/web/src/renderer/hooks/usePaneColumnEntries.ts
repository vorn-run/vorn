import { useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { isPromotedCardId, paneIdFor, type PaneChildKind } from '../lib/pane-id'

/** One row of the column, keyed by the pane id maximize is matched on. */
export interface ColumnEntry {
  id: string
  kind: PaneChildKind
}

/** Stable empty, so "no panes" does not invalidate the frames' memos. */
const NO_ENTRIES: ColumnEntry[] = []

/**
 * What this session's column will draw, in order.
 *
 * Exported because the frames around the column — the card's split, the tab
 * strip's sidebar — have to decide whether to leave room for it, and deciding
 * that separately is how a column ends up occupying space while drawing
 * nothing: a fixed-width panel of dead air, or a terminal squeezed to its split
 * ratio with a band beside it and nothing to say what is holding it open. One
 * computation, so the frame and the contents cannot disagree.
 */
export function usePaneColumnEntries(sessionId: string | null): ColumnEntry[] {
  // A card id must answer "no panes": a card owns none.
  const owner = sessionId !== null && !isPromotedCardId(sessionId) ? sessionId : null
  const { hasFiles, hasBrowser, hasDevice, hasExtension, hasTerminals } = useAppStore(
    useShallow((s) => ({
      hasFiles: owner ? s.filesPanes.has(owner) : false,
      hasBrowser: owner ? s.browserPanes.has(owner) : false,
      hasDevice: owner ? s.devicePanes.has(owner) : false,
      hasExtension: owner ? s.extensionPanes.has(owner) : false,
      hasTerminals: owner ? s.terminalsPanes.has(owner) : false
    }))
  )

  // Memoized on flat booleans: an array rebuilt every render is a new reference
  // and would re-run every effect and memo downstream of it.
  return useMemo(() => {
    if (!owner) return NO_ENTRIES
    const kinds = (
      [
        hasFiles ? 'files' : null,
        hasBrowser ? 'browser' : null,
        hasDevice ? 'device' : null,
        hasExtension ? 'extension' : null,
        hasTerminals ? 'terminals' : null
      ] satisfies (PaneChildKind | null)[]
    ).filter((k): k is PaneChildKind => k !== null)
    return kinds.map((kind) => ({ id: paneIdFor(kind, owner), kind }))
  }, [owner, hasFiles, hasBrowser, hasDevice, hasExtension, hasTerminals])
}
