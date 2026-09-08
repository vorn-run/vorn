import type { AppStore } from '../stores/types'

/**
 * Ask the host what this session's extensions say, once.
 *
 * The host pushes a footer only when its reading moves and an activation only
 * when it changes, which is right for a long-lived card and wrong for one that
 * has just appeared: a window opened after the last change would show nothing
 * until something happened to alter it, which for a settled branch can be
 * never. So each session is asked once, and pushes carry it from there.
 *
 * Kept out of the card. A component asking on mount would ask again on every
 * remount — a maximize, a tab switch, a drag between cells — and each answer
 * would rewrite a map every card reads.
 *
 * The store is handed in rather than imported: this is called from inside the
 * store, and importing it back would be a cycle that leaves whichever slice
 * loads second undefined.
 */
const hydrated = new Set<string>()

export async function hydrateExtensions(
  sessionId: string,
  getStore: () => AppStore
): Promise<void> {
  if (hydrated.has(sessionId)) return
  hydrated.add(sessionId)
  const [states, readings] = await Promise.all([
    Promise.resolve(window.api.extensionActivation?.(sessionId)).catch(() => undefined),
    Promise.resolve(window.api.extensionFooterItems?.(sessionId)).catch(() => undefined)
  ])
  const store = getStore()
  // A session closed while the host was answering keeps nothing: the store
  // prunes on removal, and writing here afterwards would put it back.
  // A host that did not know the session yet is asked again when it announces it.
  if (!store.terminals.has(sessionId) || (!states && !readings)) {
    hydrated.delete(sessionId)
    return
  }
  if (states) store.setExtensionActivation(sessionId, states)
  if (readings) store.setExtensionFooters(sessionId, readings)
}

/** Forget a session, so a recycled id is asked about again. */
export function forgetExtensionHydration(sessionId: string): void {
  hydrated.delete(sessionId)
}

/** Test seam: forget every session this process has asked about. */
export function __resetExtensionHydrationForTests(): void {
  hydrated.clear()
}
