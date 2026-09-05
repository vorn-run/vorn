import type { RestoredSession, TerminalSession } from '../../shared/types'
import { endedFromRestored } from './ended-from-restored'
import { fireSessionRestoredTrigger } from './workflow-triggers'
import { useAppStore } from '../stores'
import { coldSessions } from './session-utils'
import { resumeEndedSession } from './session-resume'

/**
 * Make the board agree with what the server actually has.
 *
 * Two questions -- what is running, and what is left of what is not -- asked at
 * the two moments the answer can change out from under a pane: when the app
 * starts, and when the server behind it is replaced.
 *
 * That second moment is the one this exists for. A pane's content lives in the
 * renderer, so when a server dies nothing about the pane changes: the terminal
 * keeps showing what it was showing, the bridge quietly reconnects to a
 * replacement holding none of the old PTYs, and the pane goes on accepting input
 * for a process that is gone. It looks exactly like a terminal that has been
 * quiet for a moment, which is the one thing a pane must never look like when it
 * is a photograph.
 */
/**
 * The pass in flight, so a second caller waits for it rather than racing it.
 *
 * Two of these overlapping is not a hypothetical: start-up runs one, a server
 * replaced moments later runs another, and in development the mount effect runs
 * twice by design. Both passes then read the same cold records, both ask to
 * resume them, and the loser is told the session is gone -- which used to delete
 * the pane the winner had just brought back.
 */
let inFlight: Promise<void> | null = null

export function syncBoard(options: { showCold: boolean; resume: boolean }): Promise<void> {
  const run = (inFlight ?? Promise.resolve()).catch(() => undefined).then(() => reconcile(options))
  // Tracked separately from what is returned. `finally` hands back a new promise
  // rather than the one it was called on, so comparing against `run` here never
  // matched and the reset never happened -- passes stayed chained off a promise
  // that had settled long ago.
  const tracked: Promise<void> = run.finally(() => {
    if (inFlight === tracked) inFlight = null
  })
  inFlight = tracked
  return run
}

async function reconcile(options: { showCold: boolean; resume: boolean }): Promise<void> {
  // Sessions this pass found ended, which is not the same as sessions that are
  // ended. A pane the person already chose to leave sitting there must not be
  // started again by the next reconciliation that happens to run.
  const justEnded: string[] = []
  const answers = await Promise.all([
    window.api.listActiveSessions(),
    window.api.getRestoredSessions()
  ]).catch((err) => {
    console.error('[board] failed to read what the server has:', err)
    return null
  })
  // A board left as it is beats a board rebuilt from an answer nobody gave. The
  // restored view keeps waiting too: pruning it against a list nobody supplied
  // would delete panes on the strength of a failed request.
  if (!answers) return
  const [active, carried]: [TerminalSession[], RestoredSession[]] = answers

  const store = useAppStore.getState()
  const live = new Set(active.map((s) => s.id))
  const heldById = new Map(carried.map((one) => [one.session.id, one]))

  // Anything the server has that this board does not. A session started from a
  // phone, or by a workflow, or before this window existed.
  for (const session of active) {
    if (store.terminals.has(session.id)) continue
    store.addTerminal(session)
    // Came from the server, not from this client. When its attach finds the
    // process running, that is a warm restore and the trigger is told once.
    restoredIds.add(session.id)
  }

  // Panes this board is holding for sessions the server no longer runs. On a
  // first start there are none; after a replacement this is all of them.
  for (const [id, term] of useAppStore.getState().terminals) {
    if (live.has(id) || term.ended) continue
    const held = heldById.get(id)
    justEnded.push(id)
    // Without a record the best available answer is when this pane last saw
    // anything, and there is no rebuilt screen behind it.
    const base = held
      ? endedFromRestored(held)
      : { reason: 'server-stopped' as const, at: term.lastOutputTimestamp, replayed: false }
    useAppStore.getState().markEnded(id, {
      ...base,
      ...(term.session.shellCwd !== undefined && { cwd: term.session.shellCwd })
    })
  }

  // And the ones with no pane at all yet: ended before this window opened.
  if (options.showCold)
    for (const one of coldSessions(active, carried)) {
      if (useAppStore.getState().terminals.has(one.session.id)) continue
      justEnded.push(one.session.id)
      useAppStore.getState().addTerminal(one.session, endedFromRestored(one))
    }

  // Everything the server has, running or ended -- not what the board ended up
  // showing. With reopen off the ended ones are left for the banner to offer,
  // and their panes have to survive until that offer is answered.
  //
  // Before the resumes, not after: each one spawns a process and goes in order,
  // and the view has nothing left to wait for once the answer is in.
  useAppStore.getState().setKnownSessions([...live, ...carried.map((one) => one.session.id)])

  if (options.resume) await resumeAll(justEnded)
}

/**
 * Start again what this pass found stopped -- not everything that is stopped.
 *
 * Only ids this reconciliation marked itself, which is what keeps two kinds of
 * ended session out of it. A pane somebody looked at and chose to leave sitting
 * there is skipped above, because it was already ended when this began. So is an
 * agent that exited on its own -- a finished turn, a `/quit` -- which the exit
 * handler marked at the time; relaunching that would be a surprise the first
 * time and a loop every time after. Both are the same rule: this starts sessions
 * that were stopped, never sessions that stopped.
 *
 * One at a time, because each one spawns a process: six panes resuming together
 * is six agents starting in the same instant, and going in order means they come
 * back in the order they were in.
 */
async function resumeAll(ids: string[]): Promise<void> {
  for (const id of ids) {
    // Read again rather than carried: each resume above it awaited a spawn, and
    // an exit or a close for this pane could have arrived in that gap.
    const term = useAppStore.getState().terminals.get(id)
    if (!term?.ended) continue
    // A failure leaves the pane ended and its strip on screen, which is already
    // the offer to try again by hand. Nothing is said twice.
    await resumeEndedSession(id, { automatic: true })
  }
}

const restoredIds = new Set<string>()

/**
 * Wired to the registry's live reporter at start-up. A session this client
 * created in this lifetime is new, not restored, and is not in the set.
 */
export function reportWarmAttach(terminalId: string): void {
  if (!restoredIds.delete(terminalId)) return
  const session = useAppStore.getState().terminals.get(terminalId)?.session
  if (session) fireSessionRestoredTrigger(session, { restore: 'warm' })
}

/** Test-only. */
export function resetRestoredIds(): void {
  restoredIds.clear()
}
