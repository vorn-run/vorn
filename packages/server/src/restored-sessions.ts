import type { RestoredSession, TerminalSession } from '@vornrun/shared/types'
import log from './logger'

/**
 * The sessions a previous run left behind that no pane has claimed yet.
 *
 * ## The bug this exists to close
 *
 * `saveSessions` is a whole-table replace fed by `ptyManager.getActiveSessions()`,
 * and after a restart that map is empty. So opening a single pane replaced the
 * entire `sessions` table with that one row, and every record from the previous
 * run was gone half a second later. On the *next* start `recoverHistory` then
 * found history for ids the database no longer knew, judged it unreachable --
 * correctly, by its own rule -- and deleted it.
 *
 * Terminal history therefore survived exactly one restart, which is one fewer
 * than the whole point of writing it down.
 *
 * ## Why hold records rather than soften the sweep
 *
 * The sweep's rule is sound and worth keeping intact: history is keyed by
 * session id, `getPreviousSessions` is the only way a pane can ever name one, so
 * an id the database does not know is unreachable. A grace period would stop the
 * deletion without restoring the reachability -- the files would sit there with
 * nothing able to ask for them, which is a leak wearing a fix's clothes.
 *
 * Holding the records keeps one deletion point and one rule. What ages out here
 * is simply absent from the list handed to recovery, so the sweep removes its
 * history in the same boot, for the same reason as everything else.
 *
 * ## What this deliberately does not do
 *
 * It does not seed `ptyManager`. `getActiveSessions()` feeds `terminal:listActive`,
 * which feeds MCP's `list_sessions` and the web client's hydration; a dead
 * session reported there would be a session an agent believes it can write to.
 * And it does not touch `livePtyCount()`, which is what decides whether an idle
 * server may leave -- holding records must never be the reason a machine stays
 * awake.
 */

export type { RestoredSession }

/**
 * How long an unclaimed record is kept.
 *
 * Long enough that a machine left off over a long weekend still comes back to
 * its terminals, short enough that a session abandoned a month ago is not still
 * holding a pane's worth of disk. Measured from `saved_at`, which is the last
 * time the previous run wrote the record down.
 */
export const MAX_RESTORED_AGE_MS = 7 * 24 * 60 * 60 * 1000

const held = new Map<string, RestoredSession>()

/**
 * Take the previous run's records, and answer with the ones worth keeping.
 *
 * The return value is what `recoverHistory` should be given: anything dropped
 * here is absent from that list, so its history goes in the same sweep that
 * removes every other unreachable tree.
 *
 * `null` in means `null` out. `readPreviousSessions` answers null when the
 * database could not be read, and the difference between "there are none" and
 * "I could not tell" is the difference between removing nothing and removing
 * every terminal's history on one transient error.
 */
export function seedRestored(
  previous: TerminalSession[] | null,
  now: number = Date.now()
): TerminalSession[] | null {
  held.clear()
  if (previous === null) return null

  const keep: TerminalSession[] = []
  let aged = 0
  for (const session of previous) {
    const endedAt = session.savedAt ?? session.createdAt ?? now
    if (now - endedAt > MAX_RESTORED_AGE_MS) {
      aged += 1
      continue
    }
    held.set(session.id, { session, endedAt, replayable: false, partial: false })
    keep.push(session)
  }

  if (held.size || aged) {
    log.info({ holding: held.size, aged }, '[restored] sessions carried over from the last run')
  }
  return keep
}

/** Note which of them the server actually rebuilt a screen for. */
export function markRecovered(recovered: Array<{ id: string; stopped: string }>): void {
  for (const one of recovered) {
    const entry = held.get(one.id)
    if (!entry) continue
    entry.replayable = true
    entry.partial = one.stopped !== 'end'
  }
}

/** What a client is offered. */
export function listRestored(): RestoredSession[] {
  return [...held.values()]
}

/**
 * The records to persist beside the live ones.
 *
 * Without this the next save erases them, which is the whole bug.
 */
export function restoredRecords(): TerminalSession[] {
  return [...held.values()].map((entry) => entry.session)
}

/**
 * Claim one, exactly once.
 *
 * Atomic because two clients can be looking at the same cold pane: the second
 * one to press resume must be told it is gone rather than starting a second
 * agent against one transcript.
 */
export function consumeRestored(id: string): RestoredSession | null {
  const entry = held.get(id)
  if (!entry) return null
  held.delete(id)
  return entry
}

/**
 * Put one back, because the thing it was claimed for did not happen.
 *
 * Claiming is destructive on purpose -- it is what stops two clients starting
 * two agents against one transcript. But a claim that then fails to spawn would
 * otherwise leave the session in neither place: gone from here, never in the pty
 * manager, and erased from the database by the next save. There would be nothing
 * left to try again from.
 */
export function restoreHeld(entry: RestoredSession): void {
  held.set(entry.session.id, entry)
}

export function consumeAllRestored(): RestoredSession[] {
  const all = [...held.values()]
  held.clear()
  return all
}

/** Test-only, mirroring the other module-level stores in this package. */
export function resetRestored(): void {
  held.clear()
}
