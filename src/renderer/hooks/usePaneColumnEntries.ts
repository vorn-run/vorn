import { useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { paneIdFor, type PaneChildKind } from '../lib/pane-id'

/** One row of the column, keyed by the pane id maximize is matched on. */
export interface ColumnEntry {
  id: string
  kind: PaneChildKind
}

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
  const { hasFiles, hasEditor, hasBrowser, hasDevice } = useAppStore(
    useShallow((s) => ({
      hasFiles: sessionId ? s.filesPanes.has(sessionId) : false,
      hasEditor: sessionId ? s.editorPanes.has(sessionId) : false,
      hasBrowser: sessionId ? s.browserPanes.has(sessionId) : false,
      hasDevice: sessionId ? s.devicePanes.has(sessionId) : false
    }))
  )

  // Memoized on flat booleans: an array rebuilt every render is a new reference
  // and would re-run every effect and memo downstream of it.
  return useMemo(() => {
    if (!sessionId) return []
    const kinds = [
      hasFiles ? ('files' as const) : null,
      hasEditor ? ('editor' as const) : null,
      hasBrowser ? ('browser' as const) : null,
      hasDevice ? ('device' as const) : null
    ].filter((k): k is PaneChildKind => k !== null)
    return kinds.map((kind) => ({ id: paneIdFor(kind, sessionId), kind }))
  }, [sessionId, hasFiles, hasEditor, hasBrowser, hasDevice])
}
