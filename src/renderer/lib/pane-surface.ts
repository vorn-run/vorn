/**
 * The background every session-owned pane shares.
 *
 * Panes are frames around someone else's content — a file tree, a file, a web
 * page. Holding them all to one ground stops the chrome competing with what it
 * holds, and stops a column of panes reading as a patchwork of slightly
 * different greys.
 *
 * One step below the terminal beside it, and that step is the only thing
 * dividing them — no border, no gutter. A border a few pixels inside the card's
 * own reads as a doubled edge, and a gutter exposes a lighter band of card that
 * pulls harder than either surface.
 */
export const PANE_SURFACE = '#101012'
