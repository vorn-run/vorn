/**
 * Where the native screen model (libghostty-vt) is allowed to differ from the
 * headless xterm it replaces.
 *
 * Parity is defined per feature (title, cursor, visible rows, cell styles), not
 * as byte equality of the serialized screen: two VT formatters can write the
 * same screen in different escape sequences. Every difference a test accepts is
 * named here, so the list of what changed is one file long and a new difference
 * has to be added on purpose.
 */

/**
 * Ghostty writes a palette foreground or background as its 256-colour index
 * (`38;5;2`) where xterm's serializer writes the 16-colour code (`32`). Both
 * select palette entry 2, so a client draws the same colour.
 */
export const PALETTE_AS_256 = 'palette-colours-written-as-256-colour-indexes'

/**
 * Ghostty fills a gap in a row with spaces where xterm's serializer moves the
 * cursor over it, so a replayed row has written blank cells where xterm's has
 * empty ones. Both look the same; an unstyled blank is compared as empty.
 */
export const BLANKS_AS_SPACES = 'gaps-written-as-spaces'

/** The SGR that selects palette foreground `n` (0-15), in either form. */
export function paletteForeground(n: number): RegExp {
  const short = n < 8 ? `${30 + n}` : `${90 + n - 8}`
  return new RegExp(`\\x1b\\[(?:[0-9;]*;)?(?:${short}|38;5;${n})(?:;[0-9;]*)?m`)
}
