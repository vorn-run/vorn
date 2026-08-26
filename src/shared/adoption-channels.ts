/**
 * Channels for telling the person what a refused adoption means for them.
 *
 * Named here rather than inline because three layers spell them: main sends,
 * preload subscribes, the renderer listens. They are not in the `IPC` map
 * because that describes the desktop's relay of *server* methods, and neither of
 * these ever reaches the server -- the whole point is that this app could not
 * talk to it.
 */
export const ADOPTION_REFUSED_CHANNEL = 'adoption:refused'
export const ADOPTION_STOP_CHANNEL = 'adoption:stop-refused'

/**
 * Deliberately carries no text from the other server.
 *
 * The refusal `detail` reads well and is useful in the log, but it interpolates
 * `dataDir` and `buildChannel` straight out of that server's greeting -- strings
 * this app never verified, from the one party it just decided it could not
 * trust. React would escape them, so this is not an injection; it is simpler
 * than that. A stranger does not get to write the text of Vorn's own warning.
 * The reason is a closed set, and the renderer says the rest in its own words.
 */
export interface AdoptionRefusedNotice {
  reason: 'no-identity' | 'protocol-mismatch' | 'different-data-dir' | 'different-build'
  /** False when the running server's pid was never learned, so nothing can act on it. */
  canStop: boolean
}
