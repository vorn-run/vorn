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
let stillHoldsEndpoint: (() => boolean) | null = null

/**
 * Teach this module how to check, rather than waiting to be told.
 *
 * Losing the endpoint happens *to* this process: another server that found it
 * unreachable takes the name and has no way to say so. An earlier version noticed
 * only inside the idle watch's snapshot, which meant up to a minute of creating
 * sessions on a name that already pointed elsewhere -- and no noticing at all for
 * `vorn-server serve`, where that watch is switched off entirely.
 *
 * One `lstat` at session creation, which is a rare, user-initiated moment.
 */
export function watchEndpoint(holds: () => boolean): void {
  stillHoldsEndpoint = holds
}

/** Irreversible on purpose. Nothing gives an endpoint back once it is lost. */
export function beginDraining(): void {
  draining = true
}

export function isDraining(): boolean {
  if (draining) return true
  // A server that never held an endpoint is not draining: it is running
  // TCP-only, which is a downgrade rather than a loss, and refusing its sessions
  // would leave the machine with nothing that works.
  if (stillHoldsEndpoint && !stillHoldsEndpoint()) beginDraining()
  return draining
}

/** Test-only. Production has no path back. */
export function resetDrainingForTests(): void {
  draining = false
  stillHoldsEndpoint = null
}

/** The message a person sees when a session is refused. */
export const DRAINING_MESSAGE =
  'This server no longer holds the local endpoint and is finishing its remaining ' +
  'sessions. Reopen Vorn to start new ones.'
