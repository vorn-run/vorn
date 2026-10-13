//! RunningGlyph and the pulsing status dot: the two pieces of motion a
//! session's status makes, timed as the renderer's CSS times them.

use vornui::accesskit::Role;
use vornui::theme::motion::FRAME;
use vornui::{div, El, Rgba, Ui};

/// A cell of the glyph's 4x4 grid: the inner four pulse brighter than the
/// edge, and the corners are never drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    In,
    Out,
    Hidden,
}

const GRID: [Tier; 16] = {
    use Tier::{Hidden as H, In as I, Out as O};
    [H, O, O, H, O, I, I, O, O, I, I, O, H, O, O, H]
};

/// The renderer's `seeded`: a fixed pseudo-random fraction per seed, so
/// every glyph twinkles in the same pattern.
fn seeded(seed: f64) -> f64 {
    let s = (seed * 12.9898 + 78.233).sin() * 43758.5453;
    s - s.floor()
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at progress `t`.
fn bezier(t: f64, (x1, y1, x2, y2): (f64, f64, f64, f64)) -> f64 {
    let at = |s: f64, a: f64, b: f64| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    // x(s) rises monotonically for these curves, so bisection finds s.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if at(mid, x1, x2) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at((lo + hi) / 2.0, y1, y2)
}

/// A keyframe envelope that goes `low` → `high` → `low`, peaking halfway,
/// eased per half with `curve`, `elapsed` seconds after a `delay`.
fn envelope(
    elapsed: f64,
    delay: f64,
    duration: f64,
    (low, high): (f64, f64),
    curve: (f64, f64, f64, f64),
) -> f64 {
    // `fill: both` holds the first keyframe through the delay.
    let t = (elapsed - delay).max(0.0) % duration / duration;
    let (from, to, p) = if t < 0.5 {
        (low, high, t * 2.0)
    } else {
        (high, low, t * 2.0 - 1.0)
    };
    from + (to - from) * bezier(p, curve)
}

const EASE_IN_OUT: (f64, f64, f64, f64) = (0.42, 0.0, 0.58, 1.0);

/// Tailwind's `animate-pulse` curve.
const PULSE: (f64, f64, f64, f64) = (0.4, 0.0, 0.6, 1.0);

/// The opacity of cell `i` `elapsed` seconds into the animation:
/// `rg-pulse-in` or `rg-pulse-out` with the cell's seeded delay and length.
fn cell_opacity(i: usize, elapsed: f64) -> f64 {
    let range = match GRID[i] {
        Tier::Hidden => return 0.0,
        Tier::In => (0.35, 0.95),
        Tier::Out => (0.12, 0.38),
    };
    let delay = seeded(i as f64 + 1.0) * 1.3;
    let duration = 0.95 + seeded(i as f64 + 101.0) * 0.8;
    envelope(elapsed, delay, duration, range, EASE_IN_OUT)
}

/// RunningGlyph: a 15-unit grid of twelve 3x3 cells in `color`, each
/// pulsing on its own beat, drawn `size` square.
pub fn running(ui: &Ui, size: f32, color: Rgba) -> El {
    ui.wake_at(ui.now() + FRAME);
    let t = ui.anim_time();
    let unit = size / 15.0;
    let mut g = div()
        .size(size, size)
        .shrink0()
        .role(Role::Image, "Running");
    for (i, tier) in GRID.iter().enumerate() {
        if *tier == Tier::Hidden {
            continue;
        }
        let (x, y) = ((i % 4) as f32 * 4.0, (i / 4) as f32 * 4.0);
        g = g.child(
            div()
                .absolute(x * unit, y * unit)
                .size(3.0 * unit, 3.0 * unit)
                .rounded(unit)
                .bg(color.alpha(cell_opacity(i, t) as f32)),
        );
    }
    g
}

/// A status dot's opacity under `animate-pulse`: 1 → 0.5 → 1 every 2 s.
pub fn pulse(ui: &Ui) -> f32 {
    ui.wake_at(ui.now() + FRAME);
    envelope(ui.anim_time(), 0.0, 2.0, (1.0, 0.5), PULSE) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_matches_the_renderer() {
        // Math.sin(1 * 12.9898 + 78.233) * 43758.5453, fractional part.
        assert!((seeded(1.0) - 0.740_084_82).abs() < 1e-6);
        for i in 0..200 {
            assert!((0.0..1.0).contains(&seeded(i as f64)));
        }
    }

    #[test]
    fn cells_hold_the_first_keyframe_through_their_delay() {
        for (i, tier) in GRID.iter().enumerate() {
            let want = match tier {
                Tier::Hidden => 0.0,
                Tier::In => 0.35,
                Tier::Out => 0.12,
            };
            assert!((cell_opacity(i, 0.0) - want).abs() < 1e-9, "cell {i}");
        }
    }

    #[test]
    fn a_cell_peaks_halfway_through_its_cycle_and_repeats() {
        let i = 5;
        let delay = seeded(6.0) * 1.3;
        let duration = 0.95 + seeded(106.0) * 0.8;
        assert!((cell_opacity(i, delay + duration / 2.0) - 0.95).abs() < 1e-6);
        let a = cell_opacity(i, delay + 0.3);
        let b = cell_opacity(i, delay + 0.3 + duration * 3.0);
        assert!((a - b).abs() < 1e-6);
    }

    #[test]
    fn ease_in_out_is_symmetric_and_bounded() {
        assert!(bezier(0.0, EASE_IN_OUT).abs() < 1e-6);
        assert!((bezier(1.0, EASE_IN_OUT) - 1.0).abs() < 1e-6);
        assert!((bezier(0.5, EASE_IN_OUT) - 0.5).abs() < 1e-6);
        let (a, b) = (bezier(0.2, EASE_IN_OUT), bezier(0.8, EASE_IN_OUT));
        assert!((a + b - 1.0).abs() < 1e-6);
        // Slow at the ends.
        assert!(a < 0.2);
    }

    #[test]
    fn the_status_dot_pulses_to_half_and_back() {
        let at = |t| envelope(t, 0.0, 2.0, (1.0, 0.5), PULSE);
        assert!((at(0.0) - 1.0).abs() < 1e-9);
        assert!((at(1.0) - 0.5).abs() < 1e-6);
        assert!((at(2.0) - 1.0).abs() < 1e-6);
    }
}
