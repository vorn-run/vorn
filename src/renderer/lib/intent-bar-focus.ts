/**
 * Registry of mounted intent-bar inputs by terminal, so keystrokes aimed at
 * the raw terminal can be routed to the composer while the shell sits at its
 * prompt (see the key redirect wired in App).
 */

const intentBarInputs = new Map<string, HTMLTextAreaElement>()

export function registerIntentBarInput(terminalId: string, el: HTMLTextAreaElement): () => void {
  intentBarInputs.set(terminalId, el)
  return () => {
    if (intentBarInputs.get(terminalId) === el) intentBarInputs.delete(terminalId)
  }
}

/** The composer's input, only while it is still in the document. */
function liveInput(terminalId: string): HTMLTextAreaElement | null {
  const el = intentBarInputs.get(terminalId)
  return el && el.isConnected ? el : null
}

/** Focus the composer for a terminal. Returns false when none is mounted. */
export function focusIntentBar(terminalId: string): boolean {
  const el = liveInput(terminalId)
  if (!el) return false
  el.focus()
  return true
}

/** Asked of the browser: a second copy of this answer is the one that goes stale. */
export function isIntentBarFocused(terminalId: string): boolean {
  const el = liveInput(terminalId)
  return el !== null && document.activeElement === el
}
