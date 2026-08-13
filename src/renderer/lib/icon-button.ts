/**
 * The one icon-button style, shared by a session card's actions and a pane's.
 *
 * Both sit in the chrome of the same card, so a difference in size or weight
 * between them reads as two unrelated toolbars rather than one.
 *
 * The danger variant is a separate string rather than an override appended to
 * `ICON_BUTTON`: two `hover:text-*` classes on one element are resolved by the
 * order they appear in the stylesheet, not the order they are written, so the
 * override would win or lose depending on how Tailwind happened to emit them.
 */
const BASE = 'p-1 rounded transition-colors hover:bg-white/[0.10]'

export const ICON_BUTTON = `${BASE} text-ink`

/** For an action that discards something — closing a session, deleting. */
export const ICON_BUTTON_DANGER = `${BASE} text-ink hover:text-danger`

/** Icon size to pair with either button style. */
export const ICON_BUTTON_SIZE = 14
