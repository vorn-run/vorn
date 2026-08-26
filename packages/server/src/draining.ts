/**
 * Whether this server has lost the endpoint and is winding down.
 *
 * The claim in `endpoint.ts` ends with a probe and a rename, two syscalls that
 * POSIX gives no way to fuse — there is no rename-if-target-is-inode-X. So the
 * window where two servers might each believe they hold the name cannot be
 * closed. What can be closed is the harm, and this is where.
 *
 * The rule: **never create work on an endpoint this process no longer holds.**
 *
 * A new session started here would be reachable only through a name that now
 * points somewhere else. The desktop would never see it, and the person who
 * started it would watch a terminal that accepts keystrokes and never runs them
 * — the exact failure the endpoint work exists to prevent, arrived at from the
 * other direction.
 *
 * Draining is not shutting down. Sessions already running are still served,
 * because the client holding them has a file descriptor rather than a name and
 * is entirely unaffected. What stops is taking on anything new. The idle watch
 * then ends the process when the last session goes, using the window it already
 * has — there is no second timer here, and no `killAll`.
 *
 * A module-level flag rather than a field on either manager: two managers asking
 * one question need one answer, and this one only ever moves in one direction.
 */

let draining = false

/** Irreversible on purpose. Nothing gives an endpoint back once it is lost. */
export function beginDraining(): void {
  draining = true
}

export function isDraining(): boolean {
  return draining
}

/** Test-only. Production has no path back. */
export function resetDrainingForTests(): void {
  draining = false
}

/** The message a person sees when a session is refused. */
export const DRAINING_MESSAGE =
  'This server no longer holds the local endpoint and is finishing its remaining ' +
  'sessions. Reopen Vorn to start new ones.'
