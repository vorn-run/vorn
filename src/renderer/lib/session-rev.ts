import type { TerminalSession } from '../../shared/types'

/**
 * Which of two copies of one session record is newer, by the registry
 * revision it carries.
 *
 * A session can reach this window twice: as the answer to a call it made and
 * as the broadcast every client gets, in either order. Each copy carries the
 * revision it was made at, and the window keeps the higher one. A record
 * without a revision is taken as it comes, as it always was.
 */

/** Whether `incoming` is a copy older than, or the same as, the one `held`: dropped when it is. */
export function staleRev(held: TerminalSession | undefined, incoming: TerminalSession): boolean {
  return held?.rev !== undefined && incoming.rev !== undefined && incoming.rev <= held.rev
}

/**
 * Whether a `session:created` broadcast is to be added: always for a session
 * this window does not have, and for one it has only when the broadcast is a
 * newer revision than the copy it holds.
 */
export function takesCreated(
  held: TerminalSession | undefined,
  incoming: TerminalSession
): boolean {
  if (!held) return true
  return incoming.rev !== undefined && !staleRev(held, incoming)
}
