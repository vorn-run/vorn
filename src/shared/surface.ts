/**
 * The rungs of the surface ladder that also have to exist outside the
 * stylesheet.
 *
 * `global.css` is authoritative: everything that can read a custom property
 * does, and this file is only for the places that cannot. There are three, and
 * they are all grounds rather than decoration, which is why getting them wrong
 * shows up as a flash rather than a wrong shade:
 *
 *   - the terminal canvas, because xterm paints to a canvas and takes literal
 *     colours (`lib/terminal-registry.ts`)
 *   - the window itself, because the main process picks a background before any
 *     renderer exists (`main/index.ts`)
 *   - the document before the app mounts, in `index.html` and the web client's
 *     offline page, because there is no stylesheet yet
 *
 * Anything painted above the field flashes light on launch and bands light
 * while the window is resized, which is the same defect as a terminal that
 * stays behind when its card moves — a surface left on the old rung.
 *
 * `tests/surface-token.test.ts` reads every value back out of `global.css`, and
 * the two HTML files out of their own markup, so none of this can drift from
 * the stylesheet without a test saying so.
 */
export const SURFACE = {
  /** App field, the window ground, and the document before the app mounts. */
  base: '#0a0a0c',
  /** Card body and the terminal canvas inside it. */
  sunken: '#0f0f11',
  /** Panes, card header, status bar: beside the work. */
  panel: '#0d0d0f',
  /** Menus, popovers, floating chrome. */
  overlay: '#19191d'
} as const

/** The terminal canvas sits on the same rung as the card holding it. */
export const TERMINAL_BACKGROUND = SURFACE.sunken
