import { useSyncExternalStore } from 'react'

/** Unsaved-edit flags, one per open file, for the close buttons and store actions that cannot see an editor's buffer. */

const dirtyBySession = new Map<string, { current: boolean }>()
const listeners = new Set<() => void>()
let version = 0

function changed(): void {
  version++
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

function observableRef(): { current: boolean } {
  let value = false
  return {
    get current() {
      return value
    },
    set current(next: boolean) {
      if (next === value) return
      value = next
      changed()
    }
  }
}

/** Re-renders when any editor's unsaved state changes; read flags with `isEditorDirty`. */
export function useDirtyVersion(): number {
  return useSyncExternalStore(subscribe, () => version)
}

/** Whether the editor under `key` holds unsaved changes, kept current. */
export function useIsDirty(key: string): boolean {
  return useSyncExternalStore(subscribe, () => isEditorDirty(key))
}

/** Ref an editor keeps in sync with its unsaved-changes state. */
export function dirtyRefFor(sessionId: string): { current: boolean } {
  let ref = dirtyBySession.get(sessionId)
  if (!ref) {
    ref = observableRef()
    dirtyBySession.set(sessionId, ref)
  }
  return ref
}

export function isEditorDirty(sessionId: string): boolean {
  return dirtyBySession.get(sessionId)?.current === true
}

export function clearDirty(sessionId: string): void {
  if (dirtyBySession.delete(sessionId)) changed()
}

/**
 * Ask once before discarding several editors' unsaved changes.
 *
 * Not `confirmDiscard` called twice. That clears each flag as it is answered,
 * so a yes-then-no left the first buffer still on screen with its dirty flag
 * already deleted — after which nothing would ever prompt for it again, and the
 * next pane switch threw those edits away in silence. One action, one question,
 * and nothing cleared until the answer covers all of it.
 */
export function confirmDiscardAll(ids: string[]): boolean {
  const dirty = ids.filter(isEditorDirty)
  if (dirty.length === 0) return true
  if (!window.confirm('Discard unsaved changes?')) return false
  for (const id of dirty) clearDirty(id)
  return true
}

/**
 * Ask before discarding one editor's unsaved changes. Returns true when the
 * caller should proceed — either the buffer was clean or the user confirmed.
 */
export function confirmDiscard(sessionId: string): boolean {
  if (!isEditorDirty(sessionId)) return true
  const ok = window.confirm('Discard unsaved changes?')
  if (ok) clearDirty(sessionId)
  return ok
}
