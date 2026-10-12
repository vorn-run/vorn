# Motion pairs

## Hover on an icon button

`widgets::icon_button`: 16 px icon, 4 px padding (24 × 24), radius 4,
icon `INK_FAINT` at rest.

- **Do.** On the first frame after the pointer enters, the fill starts moving
  from transparent towards `white/0.06` and the icon from `INK_FAINT` towards
  `INK`, over 150 ms (`theme::TRANSITION`), eased with `theme::ease_in_out`.
  Leaving reverses from the current value. Frame strip at 60 fps:
  0, 3, 18, 46, 70, 84, 92, 97, 99, 100%. Size and position never change.
- **Don't.** Wait for the tooltip delay before showing hover; scale the button
  to 1.05; add a shadow; tint the fill Bronzo; fade over 300 ms (it feels
  sticky as the pointer sweeps across a toolbar).

## Press

- **Do.** On pointer down (`Ui::pressed`) a slightly stronger fill
  (`white/0.10`) appears on the next frame, no fade. On release it fades back to the hover fill over 100–150 ms.
  The action fires on release inside the target.
- **Don't.** Animate the press in; shrink the button by more than 2%; move the
  label; fire the action on pointer down for anything destructive.

## Menu or popover entrance

Today's picker menu (`widgets.rs:280`): opacity 0 → 1, scale 0.95 → 1 about
the centre, and a 4 px rise, over `MENU_IN` (200 ms) with a cubic ease out.

- **Do.** Keep the three properties together and short; place the panel 4 px
  from its trigger; the ease out means the first frame already shows ~23%. Exit in
  ~140 ms with ease in (or instantly on choice, which is what a picked item
  should feel like).
- **Don't.** Slide in from 24 px away; scale from 0; run opacity and scale one
  after the other; bounce past `snap`'s 0.1. Any of these makes a menu read
  as decoration.

## Interrupting

Pointer enters a button, leaves at 60 ms, re-enters at 90 ms.

- **Do.** The fade reverses from wherever it was at 60 ms and turns again at
  90 ms; the strip shows a continuous curve with no frame at 0 or 1 that was
  not reached smoothly. A spring retargeted mid-flight carries its velocity
  into the new target.
- **Don't.** Restart from 0 on each change (visible flash), queue the second
  animation behind the first, or ignore input until the first one ends.

## Panel or sidebar

A 256 px sidebar collapsing.

- **Do.** Animate its offset (transform) with the `panel` spring
  (response 0.24, damping 1.0) while the content area keeps its old width,
  then commit the new layout once. Total under 240 ms. Content inside the
  panel does not reflow while it moves.
- **Don't.** Animate the width (every frame relays the terminal grid and
  text); overshoot; fade the content separately from the panel.

## Thrown card (momentum)

- **Do.** Project the release velocity to choose the destination slot, then
  retarget a `move` spring (0.30) with bounce ≤ 0.15 so the momentum lands.
- **Don't.** Add bounce to a card that was clicked into place; there was no
  momentum to reward.

## Reduce motion on

- **Do.** The menu appears with a 120 ms opacity fade only, no scale or rise;
  the sidebar collapses with a 150 ms crossfade of the two layouts; hover
  fades stay.
- **Don't.** Keep the movement but make it faster; or remove all transitions
  so states snap with no cue at all.
