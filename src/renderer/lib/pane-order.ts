import { isTerminalPane, parsePaneId } from './pane-id'

export interface Rect {
  x: number
  y: number
  w: number
  h: number
}

/**
 * Translate a drop position in the rendered grid into an index in
 * `terminalOrder`, which holds sessions only.
 *
 * The grid interleaves each session's child panes (`t1`, `files:t1`, `t2`), so
 * the two index spaces diverge as soon as any pane is open. Splicing a raw grid
 * index into `terminalOrder` targets the wrong element — or, past the end, an
 * undefined one that then gets persisted.
 *
 * The session position is the number of session panes appearing before the drop
 * point; dropping past the last session yields the end of the list.
 */
export function sessionPositionForGridIndex(orderedIds: string[], gridIndex: number): number {
  let sessions = 0
  for (let i = 0; i < orderedIds.length && i < gridIndex; i++) {
    if (isTerminalPane(orderedIds[i])) sessions++
  }
  return sessions
}

/**
 * Panes still rendered while `maximizedPaneId` is maximized.
 *
 * Maximize is session-scoped: the pane takes over its *owner's* footprint, so
 * only that session's siblings drop out. Every other session keeps rendering,
 * which is what makes maximize usable for comparing two worktrees side by side.
 */
export function visiblePanesWhileMaximized(
  orderedIds: string[],
  maximizedPaneId: string | null
): string[] {
  if (!maximizedPaneId || !orderedIds.includes(maximizedPaneId)) return orderedIds
  const owner = parsePaneId(maximizedPaneId).sessionId
  return orderedIds.filter((id) => id === maximizedPaneId || parsePaneId(id).sessionId !== owner)
}

/** How many grid cells a maximized pane absorbs — its owner's pane count. */
export function maximizedCellSpan(orderedIds: string[], maximizedPaneId: string | null): number {
  if (!maximizedPaneId || !orderedIds.includes(maximizedPaneId)) return 1
  const owner = parsePaneId(maximizedPaneId).sessionId
  return orderedIds.filter((id) => parsePaneId(id).sessionId === owner).length
}

/**
 * Persisted-layout key for a pane.
 *
 * Rects are keyed by the owner session's *stable* key (which survives restarts)
 * rather than its id, and child panes namespace that key by kind so a session's
 * tree and file keep independent saved positions alongside its terminal.
 */
export function paneLayoutKey(paneId: string, ownerStableKey: string): string {
  const { kind } = parsePaneId(paneId)
  return kind === 'terminal' ? ownerStableKey : `${kind}:${ownerStableKey}`
}

/**
 * The pane whose rect is currently a computed bounding box, if any.
 *
 * Only a maximized pane that is on screen *and* has siblings to span renders as
 * a box. Deriving the persist-skip from that condition — rather than from
 * `maximizedPaneId` alone — means a stale id can never permanently stop a
 * pane's rect from being saved.
 */
export function boundingBoxPaneFor(
  visibleIds: string[],
  allIds: string[],
  maximizedPaneId: string | null
): string | null {
  if (!maximizedPaneId || !visibleIds.includes(maximizedPaneId)) return null
  const owner = parsePaneId(maximizedPaneId).sessionId
  const siblings = allIds.filter((id) => parsePaneId(id).sessionId === owner)
  return siblings.length > 1 ? maximizedPaneId : null
}

/**
 * Bounding box of an owner session's saved rects, for a maximized pane in the
 * flexible layout.
 *
 * Returns null when there is nothing to span — one pane, or no saved rects. The
 * caller must never persist this box: it is a render-time union, and writing it
 * back would overwrite each sibling's own saved position.
 */
export function maximizedBoundingRect(
  orderedIds: string[],
  maximizedPaneId: string,
  rectFor: (paneId: string) => Rect | undefined
): Rect | null {
  const owner = parsePaneId(maximizedPaneId).sessionId
  const group = orderedIds.filter((id) => parsePaneId(id).sessionId === owner)
  if (group.length <= 1) return null

  const rects = group.map(rectFor).filter((r): r is Rect => Boolean(r))
  if (rects.length === 0) return null

  const x = Math.min(...rects.map((r) => r.x))
  const y = Math.min(...rects.map((r) => r.y))
  const right = Math.max(...rects.map((r) => r.x + r.w))
  const bottom = Math.max(...rects.map((r) => r.y + r.h))
  return { x, y, w: right - x, h: bottom - y }
}
