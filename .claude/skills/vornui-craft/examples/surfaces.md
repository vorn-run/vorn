# Surface pairs

## Buttons and colour

A session header with Rename, Duplicate, Delete.

- **Do.** Three `icon_button`s, 24 × 24, `INK_FAINT` icons turning `INK` on a
  `white/0.06` hover fill, each with a tooltip "Rename ⌘R". Delete lives in
  the `…` menu: red icon and red text, neutral row, no red fill.
- **Don't.** A Bronzo "Rename" pill, a red-filled "Delete" button, a text
  label beside every icon, or a border on each button.

## Bronzo budget

- **Do.** Bronzo marks the one thing that is live: the running step's edge
  pulse, the selected node's 1 px outline. Count it: under 5% of the pixels on
  screen.
- **Don't.** Bronzo headings, Bronzo icons in the toolbar, Bronzo focus
  rings, a Bronzo tint behind the active tab.

## Layering and shadows

A context menu over the canvas.

- **Do.** `SURFACE_OVERLAY` fill, 1 px `white/0.08` border, one soft shadow
  straight down (y 4, blur 16, black 0.35) once vornui has a shadow
  primitive. The rows inside: no fill at rest, `white/0.06` on hover, no
  shadow, no border.
- **Don't.** A shadow on a card that sits in the layout, on a button, on an
  input, or on the active tab. Two stacked shadows. A coloured glow. A
  translucent or blurred menu that lets the canvas show through.

## Radius

- **Do.** New cards, inputs and buttons at 4 px (`radius::SM`) or square.
  Nested shapes: inner radius = outer radius − padding, floored at 0 (a 4 px
  card with 4 px padding holds square-cornered content).
- **Don't.** 12 px cards, pill-shaped (`FULL`) buttons, or an inner element
  rounder than its container. Status dots and the scrollbar thumb are the
  only round things.

## Focus ring

- **Do.** Tab to a button: a 1 px `white/0.45` ring appears 1 px outside it
  at 4 px radius on the next frame. Clicking with the mouse shows no ring.
- **Don't.** Show the ring on mouse click; draw it inside the element (it
  shifts content); use Bronzo; clip it under the neighbouring panel; remove
  it because it looks busy.

## Layout shift

A tab bar where the selected tab is bold.

- **Do.** All tabs the same weight; the selected one is `GRAY_200` on
  `white/0.06`, the others `GRAY_500` turning `GRAY_300` on hover
  (`widgets::tab_strip`). A tab's close button keeps its 14 px slot while
  hidden (opacity 0), so titles do not move when it appears.
- **Don't.** Bold the selected tab (its width grows ~5% and every tab to its
  right jumps), add a 1 px border on hover without reserving it, or insert
  the close button into the flow on hover.

## Alignment

A row: 16 px icon, 14/20 label, 12/16 shortcut on the right.

- **Do.** Icon centred on the label's x-height (it usually sits ~0.5 px
  above the line box centre), label and shortcut on one baseline, 8 px
  between icon and label, 8 px row padding, row height 28 or 32.
- **Don't.** Centre the shortcut's line box against the label's (12 px text
  then sits ~1 px high), mix 6, 10 and 12 px gaps in one new row, or let a
  long label push the shortcut off the row (truncate with an ellipsis
  instead).

## Empty and loading

- **Do.** A loading list keeps its row height; rows show a spinner or
  `INK_GHOST` placeholders of the final line heights. Empty: one
  `INK_SECONDARY` sentence centred in the same box, one neutral action.
- **Don't.** Collapse to zero height then jump open, an illustration, or a
  Bronzo call to action.
