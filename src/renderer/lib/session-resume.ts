import { useAppStore } from '../stores'
import { toast } from '../components/Toast'
import { resolveResumeSessionId } from './session-utils'

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
    reason: 'server-stopped',
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
  const state = useAppStore.getState()
  const previous = state.terminals.get(terminalId)?.session
  if (!previous) return

  // Which agent-side conversation to continue. Resolved here rather than on the
  // server because it fans out to the agent history, which is a client concern
  // and already lives here; the server is given the answer, not the search.
  let resumeSessionId: string | undefined
  try {
    resumeSessionId = await resolveResumeSessionId(previous)
  } catch {
    // No exact match found. The agent's own picker is better than refusing.
  }

  const result = await window.api.resumeSession({ id: terminalId, resumeSessionId })
  if (!result.ok) {
    if (result.reason === 'gone') {
      // Another pane, window or device took it first. Nothing to resume and
      // nothing to keep showing.
      useAppStore.getState().removeTerminal(terminalId)
      if (!options.automatic) toast('That session was resumed somewhere else')
      return
    }
    if (!options.automatic) toast.error(result.message ?? 'Could not resume that session')
    return
  }

  useAppStore.getState().replaceTerminal(terminalId, result.session)
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
