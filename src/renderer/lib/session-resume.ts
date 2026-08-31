import { useAppStore } from '../stores'
import { toast } from '../components/Toast'

/**
 * Taking a session from a previous run, or letting it go.
 *
 * Both go through the server, which holds the record and hands it out once. Two
 * panes can be showing the same ended session -- a desktop and a phone, or two
 * windows -- and the second to act is told it is gone rather than starting a
 * second agent against one transcript.
 */

/**
 * Put an ended session on screen without starting anything.
 *
 * The server still holds its record and the screen it rebuilt from disk; this
 * asks for both and adds the pane. Used by the banner, which offers to bring
 * panes back rather than to relaunch agents -- the same rule start-up follows.
 */
export async function showEndedSession(id: string): Promise<void> {
  const carried = await window.api.getRestoredSessions()
  const one = carried.find((r) => r.session.id === id)
  if (!one) return
  useAppStore.getState().addTerminal(one.session, {
    // The record says which ending this was; `board-sync` reads it and this did
    // not. Taking the banner's offer after an ordinary quit therefore produced a
    // pane reporting that the server had stopped unexpectedly -- a fault report
    // for closing an app.
    reason: one.closedCleanly ? 'app-closed' : 'server-stopped',
    at: one.endedAt,
    replayed: one.replayable,
    partial: one.partial,
    ...(one.session.shellCwd !== undefined && { cwd: one.session.shellCwd })
  })
}

/**
 * Start it again, and put the live session where the ended one was sitting.
 *
 * `automatic` marks the resumes nobody clicked -- the ones start-up and a
 * replaced server do on their own when "Reopen Sessions on Startup" is on. They
 * say nothing when they fail: the pane keeps its ended strip, which is already
 * the offer to try by hand, and a crash that ends six panes would otherwise
 * stack six toasts saying so.
 */
export async function resumeEndedSession(
  terminalId: string,
  options: { automatic?: boolean } = {}
): Promise<void> {
  if (!useAppStore.getState().terminals.has(terminalId)) return

  const result = await window.api.resumeSession({ id: terminalId })
  if (!result.ok) {
    if (result.reason === 'gone') {
      // Another pane, window or device took it first. Nothing to resume and
      // nothing to keep showing -- but only when a person asked for this. An
      // automatic resume that loses the claim has almost always lost it to this
      // same app, and deleting the pane then removes the one the winner just
      // brought back, silently, because automatic resumes say nothing.
      if (options.automatic) return
      useAppStore.getState().removeTerminal(terminalId)
      toast('That session was resumed somewhere else')
      return
    }
    if (!options.automatic) toast.error(result.message ?? 'Could not resume that session')
    return
  }

  useAppStore.getState().replaceTerminal(terminalId, result.session)
  // Said even for an automatic resume: a pane quietly becoming a second view of
  // a session open elsewhere is the one surprise worth a line.
  if (result.boundTo) toast('That conversation was already running. This pane shows it.')
}

/**
 * Say what a pane should do when an attach finds nothing behind it.
 *
 * Wired once at start-up. A window opened onto a terminal that died while it was
 * closed has no start-up reconciliation to tell it, so the attach is where it
 * finds out -- and this is the only place that knows what the pane should then
 * look like.
 */
export function markPaneEnded(terminalId: string): void {
  const state = useAppStore.getState()
  const term = state.terminals.get(terminalId)
  if (!term || term.ended) return
  state.markEnded(terminalId, {
    reason: 'server-stopped',
    at: term.lastOutputTimestamp,
    replayed: true,
    ...(term.session.shellCwd !== undefined && { cwd: term.session.shellCwd })
  })
}
