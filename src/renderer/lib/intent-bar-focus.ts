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

/** Focus the composer for a terminal. Returns false when none is mounted. */
export function focusIntentBar(terminalId: string): boolean {
  const el = intentBarInputs.get(terminalId)
  if (!el || !el.isConnected) return false
  el.focus()
  return true
}
