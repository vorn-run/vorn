/**
 * The one rung of the surface ladder that also has to exist as a JavaScript
 * value.
 *
 * xterm paints to a canvas and takes its theme as literal colours, so it cannot
 * read `--color-surface-sunken` the way every other surface does. That leaves
 * the terminal's own ground spelled out twice, which is exactly the drift that
 * put a lighter rectangle inside the card the last time the ladder moved: the
 * token came down and the canvas stayed where it was.
 *
 * `tests/surface-token.test.ts` reads the token out of global.css and compares
 * it to this, so the two cannot separate without a test saying so.
 */
export const TERMINAL_BACKGROUND = '#0f0f11'
