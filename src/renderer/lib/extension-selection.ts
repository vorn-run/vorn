import { getTerminalSelection, hasTerminal } from './terminal-registry'

/**
 * Answer what is selected in a terminal, which only this window knows.
 *
 * The server holds a session's bytes, not its screen, so a footer or a pane
 * asking what a person highlighted has to ask the window drawing it. An
 * unanswered request reads as no selection, so a window without that terminal
 * mounted stays silent rather than answering for one it cannot see.
 */
export function listenForSelectionRequests(): () => void {
  const stop = window.api.onExtensionSelectionRequest?.(({ requestId, sessionId }) => {
    // Answering for a terminal this window never drew would win the race with an
    // empty string and silence the window that has the selection.
    if (!hasTerminal(sessionId)) return
    window.api.sendExtensionSelection?.(requestId, getTerminalSelection(sessionId))
  })
  return stop ?? ((): void => {})
}
