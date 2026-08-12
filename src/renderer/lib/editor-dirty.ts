/**
 * Unsaved-edit tracking for session-owned editor panes.
 *
 * The editor lives in one pane while the actions that would discard its buffer
 * live elsewhere — picking another file happens in the tree pane, closing
 * happens in the card header. Neither can reach the editor's internal state
 * through props, so each editor registers a dirty flag here under its session
 * id and the other panes consult it before throwing work away.
 */

const dirtyBySession = new Map<string, { current: boolean }>()

/** Ref the editor pane keeps in sync with its unsaved-changes state. */
export function dirtyRefFor(sessionId: string): { current: boolean } {
  let ref = dirtyBySession.get(sessionId)
  if (!ref) {
    ref = { current: false }
    dirtyBySession.set(sessionId, ref)
  }
  return ref
}

export function isEditorDirty(sessionId: string): boolean {
  return dirtyBySession.get(sessionId)?.current === true
}

export function clearDirty(sessionId: string): void {
  dirtyBySession.delete(sessionId)
}

/**
 * Ask before discarding an editor's unsaved changes. Returns true when the
 * caller should proceed — either the buffer was clean or the user confirmed.
 */
export function confirmDiscard(sessionId: string): boolean {
  if (!isEditorDirty(sessionId)) return true
  const ok = window.confirm('Discard unsaved changes?')
  if (ok) clearDirty(sessionId)
  return ok
}
