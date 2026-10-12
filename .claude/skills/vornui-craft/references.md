# vornui craft: references

Two kinds of source, kept apart on purpose:

- **Design reference (what it should look and feel like): Apple only.** HIG,
  WWDC sessions, SwiftUI and Core Animation defaults.
- **Implementation notes (how to draw it, not what it should look like):**
  shader write-ups and open-source renderers. Take algorithms from them, never
  looks, values or styles.

Local clones live in `~/dev/references` (shallow, sparse). Line numbers are
for the commits listed with each clone; re-run `git -C <dir> log -1` and
re-check lines if a clone was refreshed.

---

## Part 1. Design reference: Apple

### HIG pages (local copy)

The `apple-hig` skill mirrors the HIG under
`~/.claude/skills/apple-hig/references/`. Read these before any visual change.

**Motion** (`motion.md`; developer.apple.com/design/human-interface-guidelines/motion)

> "Add motion purposefully, supporting the experience without overshadowing
> it. Don't add motion for the sake of adding motion."

> "Aim for brevity and precision in feedback animations. When animated
> feedback is brief and precise, it tends to feel lightweight and
> unobtrusive."

> "In apps, generally avoid adding motion to UI interactions that occur
> frequently."

> "Let people cancel motion. As much as possible, don't make people wait for
> an animation to complete before they can do anything."

> "Make motion optional. … avoid using it as the only way to communicate
> important information."

Take: the 300 ms cap, no motion on hover beyond a colour fade, interruptible
everything, motion never the only signal.

**Materials** (`materials.md`)

> "A material is a visual effect that creates a sense of depth, layering, and
> hierarchy between foreground and background elements."

> "Choose materials and effects based on semantic meaning and recommended
> usage. Avoid selecting a material or effect based on the apparent color it
> imparts to your interface."

> "Thicker materials, which are more opaque, can provide better contrast for
> text and other elements with fine features."

Take: the *layering model* (functional surfaces float over content, each
layer chosen by meaning). Leave: blur, translucency, Liquid Glass. Vorn's
brand forbids them, so each layer is an opaque surface step plus a hairline
(SKILL.md, Materials).

**Typography** (`typography.md`)

> "In general, avoid light font weights. … prefer Regular, Medium, Semibold,
> or Bold font weights."

> "Minimize the number of typefaces you use, even in a highly customized
> interface."

> "Adjust font weight, size, and color as needed to emphasize important
> information and help people visualize hierarchy."

Take: one face, two sizes and two weights per surface, hierarchy by ink.

**Accessibility** (`accessibility.md`), **Dark Mode** (`dark-mode.md`),
**Buttons** (`buttons.md`), **Focus and selection** (`focus-and-selection.md`)

> "When this setting is active, ensure your app or game responds by reducing
> automatic and repetitive animations, including zooming, scaling, and
> peripheral motion. … Tightening animation springs to reduce bounce effects.
> Tracking animations directly with people's gestures." (accessibility)

> "At a minimum, make sure the contrast ratio between colors is no lower than
> 4.5:1. For custom foreground and background colors, strive for a contrast
> ratio of 7:1, especially in small text." (dark mode)

> "In general, it works well to add about 12 points of padding around
> elements that include a bezel. For elements without a bezel, about 24
> points of padding works well around the element's visible edges."
> (accessibility)

> "In general, use a focus ring for a text or search field, but use a
> highlight in a list or collection." (focus and selection)

Take: the reduce-motion recipe, the contrast floor, spacing around controls,
ring versus row highlight.

### WWDC sessions

**Designing Fluid Interfaces**, WWDC18, session 803
(developer.apple.com/videos/play/wwdc2018/803/)

- Response first: "people are really, really sensitive to latency."
- "Allow for constant redirection and interruption." Never make the user
  wait for an animation.
- Springs are described by **damping** ("from 100% damping, where there will
  be no overshoot") and **response** ("how quickly the value will try and get
  to the target"); avoid the word duration for elastic behaviour.
- "We recommend starting with 100% damping, or no overshoot."
- Bounce only to reward momentum: "if the gesture that's driving the motion
  itself has momentum, then you should reward that momentum with a little bit
  of overshoot."
- Projection: throw-to-target uses the release velocity and the scroll
  deceleration rate to pick the destination, then retargets a spring there.
- Hysteresis: a drag starts after a small distance (vornui's `MOVE_SLOP`,
  4 px, `interact.rs:70`).

**Animate with springs**, WWDC23, session 10158
(developer.apple.com/videos/play/wwdc2023/10158/)

- A spring is set by **duration** (perceptual, predictable) and **bounce**.
- Settling duration "can be a bit unpredictable … you shouldn't wait for the
  settling duration for user-facing changes."
- Continuous position *and* velocity: a jump in velocity "doesn't feel
  right"; a retargeted spring keeps its velocity.
- Bounce scale: 0 "a smooth, gradual change"; "around 15%" brisk without
  feeling bouncy; "30%" noticeably bouncy; "be cautious about using values
  higher than around 0.4."
- Conversion, mass 1: `stiffness = (2π ÷ duration)²`; damping
  `(1 − bounce) · 4π ÷ duration` for bounce ≥ 0, `4π ÷ (duration · (1 + bounce))`
  for bounce < 0.

**Explore SwiftUI animation**, WWDC23, session 10156
(developer.apple.com/videos/play/wwdc2023/10156/): animations are functions
of time evaluated per frame, and springs are the default because they retarget
smoothly. That is how vornui works: state reads `ui.now`.

**Enhance your UI animations and transitions**, WWDC24, session 10145
(developer.apple.com/videos/play/wwdc2024/10145/): follow-up on interruptible
transitions and gesture-driven springs.

### SwiftUI spring defaults (Apple's documentation, declarations)

| API | Default |
|---|---|
| `Animation.spring(response:dampingFraction:blendDuration:)` | 0.5, 0.825, 0 |
| `Animation.spring(duration:bounce:blendDuration:)` | 0.5, 0.0, 0 |
| `Animation.interactiveSpring(response:dampingFraction:blendDuration:)` | 0.15, 0.86, 0.25 |
| `Animation.smooth(duration:extraBounce:)` | 0.5, 0 (base bounce 0) |
| `Animation.snappy(duration:extraBounce:)` | 0.5, 0 ("small amount of bounce"; WWDC23 uses 0.15) |
| `Animation.bouncy(duration:extraBounce:)` | 0.5, 0 ("higher amount of bounce"; WWDC23 uses 0.3) |

`dampingFraction = 1 − bounce`; `response ≈ duration`.

### Core Animation timing defaults

| Name | Control points |
|---|---|
| `kCAMediaTimingFunctionDefault` | (0.25, 0.1, 0.25, 1.0) |
| `easeIn` | (0.42, 0, 1, 1) |
| `easeOut` | (0, 0, 0.58, 1) |
| `easeInEaseOut` | (0.42, 0, 0.58, 1) |
| `linear` | (0, 0, 1, 1) |

Implicit `CATransaction` and `NSAnimationContext` durations default to
0.25 s. `CASpringAnimation`'s raw defaults (mass 1, stiffness 100, damping 10)
give a damping ratio of 0.5, far too bouncy; always set it from response and
ratio.

### Vorn's own sources

- Brand: `~/.claude/projects/-Users-javiercanizalez-dev-vorn/memory/`
  `feedback_codex_visual.md`, `feedback_codex_voice.md`,
  `feedback_neutral_buttons.md`, `feedback_keep_recognisable_icons.md`,
  `feedback_native_keeps_current_ux.md`.
- Tokens: `packages/core/crates/vornui/src/theme.rs` (colours :12, type :51,
  radii :57, `TRANSITION` :66, `ease_in_out` :69, focus :98, scrollbar :107);
  `packages/core/crates/vorn-app/src/look.rs`.
- Motion today: `vornui/src/interact.rs` (`TOOLTIP_DELAY` :92, `MENU_IN` :94,
  `Fade` :120, `fade` :308, `hover` :353, `menu_progress` :394, `animating`
  :499, `next_wake` :505, menu `ease_out` :1091);
  menu entrance (opacity, 0.95→1 scale, 4 px rise) `vornui/src/widgets.rs:280`.
- Drawing: `vornui/src/shader.wgsl` (`rounded_box` :62, `quad_fs` :68),
  its CPU twin `vornui/src/cpu.rs` (`quad_pixel` :488), opacity and transform
  stacks `vornui/src/scene.rs:289` and `:308`, focus ring `vornui/src/ui.rs:749`.
- Renders: `vorn-app/tests/support/mod.rs:308` (`Shot::save`),
  `vornui/tests/widgets.rs:42` (`Rig::wait`), `vornui/tests/parity.rs`.

---

## Part 2. Implementation notes (how to draw it, not what it should look like)

Nothing here sets Vorn's look. Use these for algorithms only.

### A closed-form spring for vornui

Evaluate from `ui.now`; on retarget, restart with `x0 = current − new_target`
and `v0 = current velocity`.

```rust
/// Displacement from target and velocity after `t` seconds.
fn spring(response: f32, ratio: f32, x0: f32, v0: f32, t: f32) -> (f32, f32) {
    let w = std::f32::consts::TAU / response;
    if ratio >= 1.0 {
        let b = v0 + w * x0;
        let e = (-w * t).exp();
        ((x0 + b * t) * e, (v0 - w * b * t) * e)
    } else {
        let wd = w * (1.0 - ratio * ratio).sqrt();
        let e = (-ratio * w * t).exp();
        let (s, c) = (wd * t).sin_cos();
        let x = e * (x0 * c + (v0 + ratio * w * x0) / wd * s);
        let v = e * (v0 * c - (w * w * x0 + ratio * w * v0) / wd * s);
        (x, v)
    }
}
```

Report `animating()` until `|x| < 0.5 px` and `|v| < 5 px/s`, then snap.

### Evan Wallace: Fast Rounded Rectangle Shadows

madebyevan.com/shaders/fast-rounded-rectangle-shadows/ (code CC0)

- A box shadow is the product of two 1-D blurred boxes; the 1-D integral is
  closed form with `erf`, so a rectangle shadow costs one fragment step.
- For rounded corners, integrate along y with a few samples and solve x in
  closed form, bending the box's x extent by the corner circle.
- Take: the shadow primitive for floating surfaces, drawn in the quad pass
  over a rect expanded by 3σ plus offset. SKILL.md's blur values are
  CSS-style (σ = blur ÷ 2); the shader takes σ.

### Inigo Quilez: 2D distance functions

iquilezles.org/articles/distfunctions2d/

- `sdRoundedBox` (per-corner radii), `sdBox`, `sdCircle`, `sdSegment`,
  `sdArc`, `sdPie`, plus `opRound` and `opOnion` for strokes.
- Take: vornui's `rounded_box` (`shader.wgsl:62`) is the uniform-radius form.
  Use the per-corner form for tab shapes; `sdSegment` for 1 px dividers and
  edges; `opOnion` for rings. Antialias with `clamp(0.5 − d, 0, 1)` in device
  pixels, as `quad_fs` does.

### Raph Levien: blur and text

- **Blurred rounded rectangles** (raphlinus.github.io/graphics/2020/04/21/blurred-rounded-rects.html):
  an approximate blurred rounded rect from a distance field and `erf`, with no
  sampling loop; cheaper than Evan's at a small accuracy cost.
- **A Few of My Favorite Sigmoids** (raphlinus.github.io/audio/2018/09/05/sigmoid.html):
  the cheap `erf` approximations used by both shadow methods.
- **Fast 2D rendering on GPU** (raphlinus.github.io/rust/graphics/gpu/2020/06/13/fast-2d-rendering.html):
  why antialiasing is computed analytically per pixel, not by MSAA.
- **Text layout is a loose hierarchy of segmentation**
  (raphlinus.github.io/text/2020/10/26/text-layout.html): shaping, bidi and
  fallback; read before touching `vornui/src/text.rs`.
- Font gamma: there is no single Levien essay on it. The practical problem is
  that linear-coverage glyphs look thin on dark backgrounds. Any correction
  must be checked against the Electron app at the same scale before adopting.

### iced shaders (`~/dev/references/iced`, commit a8231fd, 2026-10-07)

- `wgpu/src/shader/quad.wgsl:8` `rounded_box_sdf` with four radii.
- `wgpu/src/shader/quad/shadow.wgsl:5` `shadow_expanded_bounds`, `:22`
  `mix_shadow`: a `smoothstep` falloff, cheaper but not gaussian; it reads
  harder-edged than Evan's. Use it only as the cheap fallback for the CPU
  renderer if needed.

### vello (`~/dev/references/vello`, commit 235739c, 2026-10-09)

- `vello_cpu/src/fine/common/rounded_blurred_rect.rs:6` (adapted from Raph's
  `blurrr`), `:175` the `erf` evaluation, `:234` sigmoid note: Raph's method
  in SIMD on the CPU. The model for `vornui/src/cpu.rs` if a shadow lands.
- `vello_cpu/src/filter/gaussian_blur.rs:62` `apply_blur`: separable,
  decimated gaussian. Reference only; Vorn draws no blurred content.
- `vello_cpu/src/filter/drop_shadow.rs:57` `apply_drop_shadow`.

### wgpu examples (`~/dev/references/wgpu`, commit 5422372, 2026-10-12)

- `examples/features/src/hello_triangle/shader.wgsl`: the smallest pipeline.
- `examples/features/src/msaa_line/mod.rs:34` and `shader.wgsl`: MSAA set-up,
  for comparison with vornui's analytic antialiasing (keep analytic).
- `examples/features/src/framework.rs:676` sample count, `:701`
  `PresentMode::Fifo`: surface configuration and frame pacing.

### Refreshing the clones

```sh
cd ~/dev/references
git -C iced pull --depth 1 && git -C vello pull --depth 1 && git -C wgpu pull --depth 1
```

Then re-check the line numbers above with `grep -n`.
