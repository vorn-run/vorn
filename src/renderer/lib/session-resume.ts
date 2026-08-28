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

/** Start it again, and put the live session where the ended one was sitting. */
export async function resumeEndedSession(terminalId: string): Promise<void> {
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
      toast('That session was resumed somewhere else')
      return
    }
    toast(result.message ?? 'Could not resume that session')
    return
  }

  useAppStore.getState().replaceTerminal(terminalId, result.session)
}

/** Decline the offer. The record and what was written for it both go. */
export async function dismissEndedSession(terminalId: string): Promise<void> {
  useAppStore.getState().removeTerminal(terminalId)
  try {
    await window.api.killTerminal(terminalId)
  } catch {
    // The record is the server's to forget; a pane that has already gone from
    // this client is not worth a message about it.
  }
}
