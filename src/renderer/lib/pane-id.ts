/**
 * Pane identity.
 *
 * The grid, tab strip, dock and layout persistence all address panes by opaque
 * string id. Historically that id was always a terminal id, and every consumer
 * did `terminals.get(id)` and bailed on a miss. To let a session own additional
 * panes (its file tree, its open file, its browser) without rewriting that
 * machinery, child
 * panes get a prefixed id derived from their owner session's id.
 *
 * Ids stay opaque strings, so ordering, drag/resize and minimize keep working
 * untouched — only the components that *render* a pane need to branch on kind.
 */

export type PaneKind = 'terminal' | 'files' | 'editor' | 'browser'

const FILES_PREFIX = 'files:'
const EDITOR_PREFIX = 'editor:'
const BROWSER_PREFIX = 'browser:'

/** Id of the file-tree pane owned by `sessionId`. */
export function filesPaneId(sessionId: string): string {
  return `${FILES_PREFIX}${sessionId}`
}

/** Id of the file-editor pane owned by `sessionId`. */
export function editorPaneId(sessionId: string): string {
  return `${EDITOR_PREFIX}${sessionId}`
}

/** Id of the browser pane owned by `sessionId`. */
export function browserPaneId(sessionId: string): string {
  return `${BROWSER_PREFIX}${sessionId}`
}

/**
 * Resolve a pane id to its kind and owner session.
 *
 * Terminal panes are their own owner, so `parsePaneId(termId)` yields
 * `{ kind: 'terminal', sessionId: termId }`. That makes "which session does this
 * pane belong to" a single call regardless of kind.
 */
export function parsePaneId(paneId: string): { kind: PaneKind; sessionId: string } {
  if (paneId.startsWith(FILES_PREFIX)) {
    return { kind: 'files', sessionId: paneId.slice(FILES_PREFIX.length) }
  }
  if (paneId.startsWith(EDITOR_PREFIX)) {
    return { kind: 'editor', sessionId: paneId.slice(EDITOR_PREFIX.length) }
  }
  if (paneId.startsWith(BROWSER_PREFIX)) {
    return { kind: 'browser', sessionId: paneId.slice(BROWSER_PREFIX.length) }
  }
  return { kind: 'terminal', sessionId: paneId }
}

/** Kind of a pane id, without allocating the owner string. */
export function paneKind(paneId: string): PaneKind {
  if (paneId.startsWith(FILES_PREFIX)) return 'files'
  if (paneId.startsWith(EDITOR_PREFIX)) return 'editor'
  if (paneId.startsWith(BROWSER_PREFIX)) return 'browser'
  return 'terminal'
}

/** The session a pane belongs to. For a terminal pane, the pane id itself. */
export function paneOwnerId(paneId: string): string {
  return parsePaneId(paneId).sessionId
}

/** True when the pane is a session's terminal rather than one of its children. */
export function isTerminalPane(paneId: string): boolean {
  return paneKind(paneId) === 'terminal'
}
