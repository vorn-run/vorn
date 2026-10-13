---
name: vornui-craft
description: How Vorn's GPU UI should look, move and read. Load it for any visual or interaction change in vornui, vorn-components or vorn-app (widgets, screens, motion, colour, spacing, copy).
---

# vornui craft

vornui (`packages/core/crates/vornui`) draws basic components: quads, text,
icons, focus, hover, menus. `vorn-components` composes them (planned; it is not
in the tree yet). `vorn-app` builds the screens. This skill says what _right_
looks and feels like there, in numbers.

Order of authority, highest first:

1. **Vorn's brand rules** (below). They override everything else.
2. **Today's UX.** The native app ports the current look faithfully. Propose
   changes one at a time; never redesign a surface as a side effect.
3. **Apple's design guidance** (HIG motion, materials, typography; the WWDC
   sessions on fluid interfaces and springs). It is the only aesthetic
   reference. Where it and the brand disagree (blur, glass, large radii), the
   brand wins.

Exact sources, quotes and file:line pointers are in `references.md`. Do/don't
pairs are in `examples/`.

## Brand rules (non-negotiable)

- **No gradients** as fills. Sole exception: the Bronzo data-flow pulse on
  workflow node edges.
- **No shadows on interactive controls**: buttons, pills, inputs, cards, tabs,
  rows. Shadows belong only to surfaces that float (see Materials).
- **No blur, glass or frosted surfaces.** No backdrop blur at any radius.
- **Radius ≤ 4 px** for anything new (`theme::radius::SM`, or 0). Ported
  surfaces keep the radius they have today (`MD` 6, `LG` 8) until a brand
  pass changes them; never add `XL` or `FULL` except for status dots and the
  scrollbar thumb.
- **Colour 80/15/5**: 80% surface (base, panel, sunken, overlay) and ink;
  15% secondary ink and grays; at most 5% Bronzo. Every colour comes from
  `theme::color`. No new hex.
- **Colour means status, never chrome.** Buttons are neutral: gray icon,
  white on hover, `white/0.06` fill on hover, a tooltip naming the action and
  its shortcut. One filled primary button per surface at most, neutral fill.
  Destructive actions live in the `…` menu: red icon and text, neutral row.
- Keep the marks that make things recognisable (workflow icons, connector and
  step glyphs) when you calm a surface; strip chips, pills and tinted text.
- **No patterns, textures or noise.** Dark mode is primary.
- **8 px grid**: spacing in multiples of 8, 4 for micro-adjustments. Ported
  Tailwind values (6, 10, 12) stay until the surface is revisited. Golden
  ratio (1 : 1.618) for major layout divisions.

### Copy voice

- No exclamation marks. No emoji. Short declarative sentences.
- Active voice: "Vorn stopped the agent", not "The agent was stopped".
- Never: seamlessly, revolutionary, game-changing, supercharge, unlock,
  empower, delightful, robust, scalable, synergy, "AI-powered".
- "Atelier", not "workspace". "Canvas", not "dashboard".
- Labels are verbs or nouns in sentence case: "Rename", "New task".
- Errors say what happened and what happens next: "Could not reach vornd.
  Retrying."
- When in doubt, remove the last sentence.
- Never name another product in UI text, code, commits or PRs.

## Motion

Apple's two rules (WWDC18 _Designing Fluid Interfaces_): respond immediately,
and let every motion be interrupted and redirected. Everything below serves
them.

### Response

- **Visible feedback within one frame of input**, always < 100 ms. Hover,
  press and focus start changing on the next frame; never wait for a fetch,
  another thread, or the end of another animation.
- Press feedback is instant (no fade in); the release fades out.
- Tooltips wait `TOOLTIP_DELAY` (400 ms); once one is up, a neighbour's
  shows without the delay for `TOOLTIP_WARM` (500 ms) after it is left.

### Durations

| What                           | Duration                                 | Shape              |
| ------------------------------ | ---------------------------------------- | ------------------ |
| Hover / colour fades           | 100–150 ms (`theme::TRANSITION`, 150 ms) | ease in-out        |
| Press release, toggles, checks | 100–150 ms                               | ease out or `snap` |
| Menus, popovers, tooltips in   | 150–200 ms (`MENU_IN`, 200 ms)           | ease out or `snap` |
| Panels, sidebars, dialogs      | 180–240 ms                               | `panel`            |
| Card reflow, large moves       | 240–300 ms                               | `move`             |
| Exits                          | ~70% of the matching entrance            | ease in            |

- **Nothing exceeds ~300 ms.** Vorn is used all day; the HIG asks for brief,
  precise feedback and little motion on frequent interactions.
- One property set per element per change. Do not chain sequential steps.
- No idle or looping motion except progress indicators and the caret.

### Springs

Specify springs as Apple does: **response** (perceptual duration, seconds)
and **damping ratio** (1 = no overshoot; bounce = 1 − damping). Presets,
derived from SwiftUI's documented defaults and scaled to Vorn's 300 ms cap:

| Preset  | Response | Damping | Use                                       | Apple origin                                                 |
| ------- | -------- | ------- | ----------------------------------------- | ------------------------------------------------------------ |
| `track` | 0.15     | 0.86    | follow the pointer: drags, split dividers | `interactiveSpring()` 0.15 / 0.86                            |
| `snap`  | 0.20     | 0.90    | menus, popovers, toggles                  | today's menu spring, 0.2 s / bounce 0.1 (`interact.rs:1090`) |
| `panel` | 0.24     | 1.00    | panels, sidebars, dialogs                 | `.smooth` (bounce 0), Vorn pace                              |
| `move`  | 0.30     | 1.00    | card reflow, large spatial moves          | `.smooth` (bounce 0), Vorn pace                              |

- SwiftUI's defaults for comparison: `spring()` 0.5 / 0.825;
  `spring(duration:bounce:)` 0.5 s, bounce 0; `.smooth`, `.snappy`,
  `.bouncy` 0.5 s with bounce 0, ≈0.15, ≈0.3. Too slow for Vorn's chrome.
- **Start at damping 1.0.** Add overshoot only when the gesture that ended
  had momentum (a fling, a thrown card), and keep bounce ≤ 0.15. `snap`'s
  0.1 is the one exception: it keeps today's menu feel. Apple warns
  against bounce above ~0.4; Vorn never passes 0.15.
- Convert for code (mass 1): `stiffness = (2π / response)²`,
  `damping = 4π · ratio / response`. A closed-form spring is in
  `references.md`.
- Never wait for a spring to _settle_ before acting; settling is longer and
  unpredictable. Act at the perceptual duration.

### Easing (when a curve, not a spring)

Core Animation's named curves are the reference:

- **Enter: ease out** `(0, 0, 0.58, 1)`. Arrive fast, land softly.
- **Exit: ease in** `(0.42, 0, 1, 1)`, shorter than the entrance.
- **Change in place: ease in-out** `(0.42, 0, 0.58, 1)`. vornui's hover fade
  uses `theme::ease_in_out` `(0.4, 0, 0.2, 1)` for parity with the web app;
  keep it for colour fades.
- **Linear** only for progress bars, spinners and pointer tracking.

### Interruptible

- **Animate from the presented value, never from the start.** vornui's hover
  `Fade` reverses from its current progress (`interact.rs:308`); copy that.
  A spring retargeted mid-flight keeps its velocity.
- Never block input during an animation. Clicking a closing menu's trigger
  reopens it from where it is.
- Animations are a function of `ui.now`: `ui.hover(id)`,
  `ui.menu_progress(id)`. Report pending work through `animating()` and
  `next_wake()` so the window paces frames; never sleep or spawn timers in
  widgets.

### No layout shift

- Hover, press, focus and selection never change a box's size or position.
  Reserve borders as transparent; keep the weight of selected text constant
  and change its colour instead.
- Animate `opacity` and `transform` (scale, offset); they do not touch
  layout. A growing panel animates its transform, then commits layout once.
- Loading keeps the box: a spinner replaces content inside the same size;
  placeholders match the final line heights.
- Scrollbars overlay content (`theme::scrollbar`, 6 px) and never push it
  sideways. Changing digits use a monospace face.

### Reduce motion

- When the OS asks for reduced motion (macOS _Reduce motion_, Windows
  _Show animations_ off, GNOME _Animations_ off): replace movement and scale
  with an opacity crossfade ≤ 150 ms, set every spring to damping 1.0, drop
  repeating motion. Colour fades and progress indicators stay.
- vornui has no reduce-motion switch yet. If your change adds movement, read
  the setting through one theme-level flag (add it in your PR) and test both
  paths. Never make motion the only signal of a state change.

## Focus, hit targets and alignment

- **Focus ring** on keyboard focus only (`focus_visible`): 1 px, 1 px outside
  the element, `white/0.45`, 4 px radius (`theme::focus`, drawn at
  `ui.rs:749`). Every interactive element gets `.ring(...)`. Never remove a
  ring to tidy a screenshot. Lists highlight the whole row instead.
- Pointer hit targets ≥ 24 × 24 px, 28 px for primary toolbar icons, even
  when the glyph is 14–16 px. Keep ≥ 8 px between bezel-less icon targets.
- **Optical alignment over geometric**: centre icons on the text's x-height,
  not its line box; nudge play triangles and chevrons ~1 px towards their
  point; align text by baseline across columns; align a row's icon with the
  text above it, not with its own bounding box.
- Icons: one stroke width per surface (lucide, 1.5–2 px at 16 px), gray at
  rest, ink on hover. Status colour on an icon only when it _is_ the status.

## Type and contrast

- UI font per platform (`look::UI_FONT`); sizes `theme::text::XS` 12/16 and
  `SM` 14/20; 10–11 px only for captions. At most two sizes and two weights
  per surface. Avoid light weights (HIG). Build hierarchy with ink, not size:
  `INK` 1.0, `INK_SECONDARY` 0.55, `INK_FAINT` 0.35.
- Contrast on `SURFACE_BASE`: `INK_SECONDARY` ≈ 6.3:1 (fine for body text);
  `INK_FAINT` ≈ 3.2:1 (placeholders, disabled, hints; never content). The
  HIG minimum is 4.5:1; aim for 7:1 on small text.
- vornui blends in sRGB, as a browser does (`window.rs:577`). Compare text
  weight against the Electron app at the same scale before tuning anything.

## Materials and layering

Apple layers functional surfaces over content. Vorn keeps that hierarchy but
builds it from opaque steps, never blur:

| Layer                       | Fill                      | Edge                      |
| --------------------------- | ------------------------- | ------------------------- |
| Window                      | `SURFACE_BASE` #0d0d0f    | none                      |
| Panels, sidebar             | `SURFACE_PANEL` #101012   | 1 px `white/0.06` divider |
| Cards, inputs, sunken wells | `SURFACE_SUNKEN` #141416  | 1 px `white/0.08`         |
| Menus, popovers, dialogs    | `SURFACE_OVERLAY` #1c1c20 | 1 px `white/0.08`         |

- Separate with one step of fill plus a hairline. Never two borders on one
  edge, never a border and a shadow on a control.
- **Shadows only on floating surfaces** (menus, popovers, dialogs, toasts, a
  dragged card): one each, offset straight down, black, soft.
  - menu / popover: y 4 px, blur 16 px (σ 8), black 0.35
  - dialog: y 8 px, blur 32 px (σ 16), black 0.45, over a `black/0.5` scrim
    that fades with the dialog
  - dragged card: y 6 px, blur 24 px (σ 12), black 0.4
- These blur values are shadow falloff only. vornui has no shadow primitive
  yet; adding one changes `shader.wgsl` and `cpu.rs` together (see the
  implementation notes in `references.md`).

## Performance: per-byte, per-frame and per-pane paths

A person runs 16 agents at once, each streaming. Code that runs per byte a
terminal sends, per frame, or once per pane is a hot path; these rules bind
there. The many-pane bench (`yarn bench:panes`, numbers in
`docs/native-core/benchmarks.md`) measures them.

- **No work that grows with panes × bytes.** A pane's bytes are folded into its
  own screen once; nothing loops over every pane for each byte or chunk one
  pane receives. Per frame, touch only the panes marked dirty
  (`grid.take_dirty()`), never rescan or rebuild every pane.
- **Hidden and offscreen panes cost nothing to draw.** A card not laid out
  this frame (paged out of the grid, behind a focused card, in a collapsed
  group) builds no view, shapes no glyph and pushes no quad. Its bytes still
  reach its screen model, off the frame path; build its view when it shows.
- **No allocation per byte, or per cell per frame.** Reuse buffers across
  frames (`clear()`, not a new `Vec`); intern styles and glyphs; build a row's
  runs without a `String` per cell. Allocating per row per changed frame is
  the ceiling.
- **Coalesce redraws to the frame clock.** Output marks a pane dirty and asks
  for a frame; it never draws, lays out or wakes the window per chunk. However
  many chunks arrive between two ticks, the next frame folds them all in once.
  Animations report `next_wake()` rather than spinning.
- **Measure before and after.** A change on these paths runs the bench at 8 and
  16 panes and quotes frame p50/p99 and CPU per pane in the PR; a frame over
  16.7 ms or CPU per pane that grows with N is a regression.

Known gap: attached panes that are not drawn (behind a focused card, paged
out) still have their views rebuilt in `App::pump` when dirty; the `hidden`
case shows the cost. Fixing it is in scope for any change to `pump`.

## Review loop (every visual change)

1. **Stills.** Render the touched surface before and after, in every state:
   rest, hover, pressed, focus-visible, selected, disabled, loading, empty,
   error. vorn-app surfaces:
   `VORN_APP_SHOT=$PWD/shots/x cargo test --profile ci -p vorn-app --test surfaces`
   writes `x-<name>.png` per `shot.save` (the tests run their own vornd and
   HOME). Widgets: a `Rig` in `vornui/tests/widgets.rs` plus `Ui::save_png`.
   Once the `vorn-components` gallery lands, render its page instead.
2. **Motion capture.** Step the clock at 1/60 s through the animation and
   save each frame (`examples/animation-capture.md`). In the strip: first
   change on frame 1, no frame with a jump, settled by ~300 ms.
3. **Compare** with today's Electron screen at the same size and scale.
4. **Attach** before/after PNGs and the strip (or a GIF of it) to the PR.
5. **Run the checklist.** Fix, re-render, repeat.

### Feels-right checklist

- [ ] Hover, press and focus react on the next frame.
- [ ] Every animation ≤ 300 ms; exits shorter than entrances.
- [ ] Reversing mid-way (hover out, Escape while opening) never jumps.
- [ ] Nothing moves, resizes or reflows on hover, press, focus or select.
- [ ] Keyboard alone reaches every control, and the ring shows.
- [ ] No gradient, no blur, no shadow on a control, radius ≤ 4 px on new work.
- [ ] Bronzo ≤ 5% of the surface; colour only where it means status.
- [ ] Spacing on the 4/8 grid; icons optically centred; baselines align.
- [ ] Contrast ≥ 4.5:1 for anything a user must read.
- [ ] Reduced motion leaves a crossfade or nothing, and still makes sense.
- [ ] Copy passes the voice rules.
- [ ] GPU and CPU renderers match (`vornui/tests/parity.rs` passes).
- [ ] On a per-byte, per-frame or per-pane path: no work in panes × bytes,
      nothing built for panes not drawn, no allocation per byte or cell,
      redraws on the frame clock; `yarn bench:panes` before and after.
- [ ] It looks calm. If something draws the eye without a reason, remove it.
