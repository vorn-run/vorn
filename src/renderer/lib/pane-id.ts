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

export type PaneKind = 'terminal' | 'files' | 'editor' | 'browser' | 'device'

/**
 * What to call each kind where the pane's own title won't do.
 *
 * A pane titles itself by its content — the open file's name, the page's host,
 * the device's name — which is the right thing inside the pane and the wrong
 * thing in a dock, where the question is which of a session's panes this is.
 */
export const PANE_LABEL: Record<PaneKind, string> = {
  terminal: 'Terminal',
  files: 'Files',
  editor: 'Editor',
  browser: 'Browser',
  device: 'Device'
}

const FILES_PREFIX = 'files:'
const EDITOR_PREFIX = 'editor:'
const BROWSER_PREFIX = 'browser:'
const DEVICE_PREFIX = 'device:'

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
 * Id of the device pane owned by `sessionId`.
 *
 * A session holds at most one simulator, so — like its browser — it has at most
 * one device pane, and the owner id is enough to name it.
 */
export function devicePaneId(sessionId: string): string {
  return `${DEVICE_PREFIX}${sessionId}`
}

/**
 * Id of the pane of `kind` owned by `sessionId` — the inverse of `parsePaneId`.
 *
 * For code that already holds a kind as data (the pane column, which builds its
 * stack from a list of kinds) rather than calling a named builder per branch.
 */
export function paneIdFor(kind: Exclude<PaneKind, 'terminal'>, sessionId: string): string {
  switch (kind) {
    case 'files':
      return filesPaneId(sessionId)
    case 'editor':
      return editorPaneId(sessionId)
    case 'browser':
      return browserPaneId(sessionId)
    case 'device':
      return devicePaneId(sessionId)
  }
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
  if (paneId.startsWith(DEVICE_PREFIX)) {
    return { kind: 'device', sessionId: paneId.slice(DEVICE_PREFIX.length) }
  }
  return { kind: 'terminal', sessionId: paneId }
}

/** Kind of a pane id, without allocating the owner string. */
export function paneKind(paneId: string): PaneKind {
  if (paneId.startsWith(FILES_PREFIX)) return 'files'
  if (paneId.startsWith(EDITOR_PREFIX)) return 'editor'
  if (paneId.startsWith(BROWSER_PREFIX)) return 'browser'
  if (paneId.startsWith(DEVICE_PREFIX)) return 'device'
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

/**
 * The panes `sessionId` owns, out of a set of pane ids.
 *
 * Ownership is encoded in the id, so this needs no store — which is what lets a
 * promoted pane be placed next to the session it came from without either side
 * holding a reference to the other.
 */
export function panesOwnedBy(paneIds: Iterable<string>, sessionId: string): string[] {
  const owned: string[] = []
  for (const paneId of paneIds) {
    const parsed = parsePaneId(paneId)
    if (parsed.kind !== 'terminal' && parsed.sessionId === sessionId) owned.push(paneId)
  }
  return owned
}
