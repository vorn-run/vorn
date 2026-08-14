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

export type PaneKind = 'terminal' | 'files' | 'editor' | 'browser' | 'device' | 'card'

/** Pane kinds a session stacks inside its own card, i.e. all but those two. */
export type PaneChildKind = Exclude<PaneKind, 'terminal' | 'card'>

const FILES_PREFIX = 'files:'
const EDITOR_PREFIX = 'editor:'
const BROWSER_PREFIX = 'browser:'
const DEVICE_PREFIX = 'device:'
const CARD_PREFIX = 'card:'

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
export function paneIdFor(kind: PaneChildKind, sessionId: string): string {
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
 * Id for an item promoted out of a session's card — one file, one browser tab.
 *
 * Unlike the four ids above there can be many per session, so the id carries a
 * sequence number as well as the owner. It is read back from the right, because
 * a session id may itself contain a colon (`host:1234`) while the sequence
 * number never does — parsing from the left would hand back the wrong owner and
 * label the card with somebody else's branch.
 *
 * The sequence is only ever compared for equality; nothing reads it as a count.
 */
export function promotedCardId(sessionId: string, seq: number): string {
  return `${CARD_PREFIX}${sessionId}:${seq}`
}

/** True for an id from `promotedCardId`. */
export function isPromotedCardId(paneId: string): boolean {
  return paneId.startsWith(CARD_PREFIX)
}

/**
 * Resolve a pane id to its kind and owner session.
 *
 * Terminal panes are their own owner, so `parsePaneId(termId)` yields
 * `{ kind: 'terminal', sessionId: termId }`. That makes "which session does this
 * pane belong to" a single call regardless of kind.
 */
export function parsePaneId(paneId: string): { kind: PaneKind; sessionId: string } {
  if (paneId.startsWith(CARD_PREFIX)) {
    const rest = paneId.slice(CARD_PREFIX.length)
    const seqAt = rest.lastIndexOf(':')
    return { kind: 'card', sessionId: seqAt === -1 ? rest : rest.slice(0, seqAt) }
  }
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
  if (paneId.startsWith(CARD_PREFIX)) return 'card'
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

/**
 * True for ids that are cells of the grid in their own right, and so can own a
 * saved rect: sessions, and cards popped out of one.
 *
 * A session's child panes are drawn inside its card and are not cells — an
 * older build gave them their own, and their rects are pruned on read. Cards
 * are, and pruning theirs is why one never held a position in the flexible
 * layout: it was dropped on load, so every card fell back to the origin.
 */
export function isLayoutCellId(paneId: string): boolean {
  return isTerminalPane(paneId) || isPromotedCardId(paneId)
}

/** True when the pane is a session's terminal rather than one of its children. */
export function isTerminalPane(paneId: string): boolean {
  return paneKind(paneId) === 'terminal'
}
