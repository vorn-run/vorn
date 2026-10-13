# Capturing an animation

vornui animations are a function of the frame clock, so a capture is
deterministic: fix the clock, step it 1/60 s, render offscreen, save a PNG.
Put this in a throwaway test (or a `#[ignore]` one) next to
`vornui/tests/widgets.rs`; it runs on the CPU renderer, needs no window and
writes only where you point it.

```rust
use std::time::{Duration, Instant};
use vornui::theme::color;
use vornui::widgets::{icon_button, icons};
use vornui::{div, El, Id, RenderMode, Ui, UiConfig};

const SIZE: (u32, u32) = (240, 120);
const STEP: Duration = Duration::from_micros(16_667);

fn view(ui: &Ui) -> El {
    div().w(SIZE.0 as f32).h(SIZE.1 as f32).p(16.0).child(
        icon_button(ui, Id::new("b"), icons::PLUS, 16.0, "New task"),
    )
}

#[test]
#[ignore = "writes a frame strip; run by hand"]
fn capture_hover() {
    let out = std::env::var("VORNUI_CAPTURE").expect("VORNUI_CAPTURE=dir");
    let mut ui = Ui::headless(RenderMode::Cpu, &UiConfig::system(2.0, 13.0));
    let target = ui.offscreen((SIZE.0 * 2, SIZE.1 * 2));
    let mut now = Instant::now();
    for frame in 0..24 {
        if frame == 2 {
            ui.pointer_move(28.0, 28.0); // the change under test
        }
        loop {
            ui.begin_at(color::SURFACE_BASE, now);
            ui.layout(view(&ui), (SIZE.0 as f32, SIZE.1 as f32));
            if ui.render_offscreen(&target) {
                break;
            }
        }
        ui.save_png(&target, &format!("{out}/f{frame:03}.png")).unwrap();
        now += STEP;
    }
}
```

Swap in the widget you changed. Rules:

- Lay out at least one frame before sending pointer input, so hit targets
  exist. Send input between frames, as the window does.
- Capture 2 frames before the change and enough after it to see it settle
  (24 frames = 400 ms covers every Vorn animation).
- For interruption, send the reverse input mid-way (e.g. `pointer_move` off
  the button at frame 6) and check the strip turns smoothly.
- Write to a temp directory, never into the repo or `~/.vorn`.

Run it with the ci profile, as the rest of the build:

```sh
VORNUI_CAPTURE=$TMPDIR/hover nice -n 10 cargo test --profile ci -j 3 \
  -p vornui --test capture -- --ignored --nocapture
```

Assemble for the PR (optional):

```sh
ffmpeg -framerate 60 -i $TMPDIR/hover/f%03d.png -vf "scale=iw/2:-1" hover.gif
# or a contact strip, every 2nd frame:
ffmpeg -i $TMPDIR/hover/f%03d.png -vf "select=not(mod(n\,2)),tile=12x1" strip.png
```

Read the strip, not just the GIF: the first changed frame must be the one
right after the input, consecutive frames must differ by a small, smooth
amount, and the last frames must be identical (settled).
