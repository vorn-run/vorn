import { isTerminalPane } from './pane-id'

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
