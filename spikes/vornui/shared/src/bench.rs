//! The bench driver both prototypes run, so they are measured the same way.
//!
//! A prototype draws offscreen; the driver plays the display. It waits for
//! the grid to change, paces frames to a 120 Hz display (never two frame
//! starts closer than 8.33 ms), and times each frame from its start to the
//! GPU submit. Meanwhile it types into pane 0 through the prototype's own
//! input path: each key arms a probe on the grid client, and the key's
//! latency ends at the submit of the first frame that draws its glyph.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::grid::Grid;
use crate::metrics;

/// What one call to [`Proto::frame`] did.
#[derive(Debug, Clone, Copy, Default)]
pub struct Frame {
    /// Something was drawn and submitted.
    pub drawn: bool,
    /// A pane in this frame showed the armed probe's glyph.
    pub probe_hit: bool,
    /// Every pane has its screen in this frame.
    pub complete: bool,
}

/// A prototype under the driver.
pub trait Proto {
    /// Types `ch` into pane 0 the way a key press reaches the UI.
    fn key(&mut self, ch: char);
    /// Presses Enter in pane 0.
    fn enter(&mut self);
    /// Pulls the changed panes, rebuilds what they show and submits a frame.
    fn frame(&mut self) -> Frame;
}

pub struct Config {
    pub settle: Duration,
    pub measure: Duration,
}

const PERIOD: Duration = Duration::from_nanos(8_333_333);

/// Runs the bench and answers its numbers.
pub fn run(grid: &Grid, p: &mut dyn Proto, cfg: &Config) -> Value {
    let started = Instant::now();
    let mut seen = 0u64;
    let mut drawn_changes = u64::MAX;
    let mut last_frame: Option<Instant> = None;
    let mut window: Option<(Instant, metrics::Usage)> = None;
    let mut frames_ms = Vec::new();
    let mut latency_ms = Vec::new();
    let mut lost = 0u32;
    let mut typed_at: Option<Instant> = None;
    let mut next_key = started;
    let mut on_line = 0u32;
    let mut seq = 0u32;
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    loop {
        let now = Instant::now();
        if window.is_none() && now.duration_since(started) >= cfg.settle {
            window = Some((now, metrics::usage()));
            next_key = now;
        }
        if let Some((w, _)) = window {
            if now.duration_since(w) >= cfg.measure {
                break;
            }
            if let Some(t) = typed_at {
                if !grid.probe_pending(0) {
                    typed_at = None;
                } else if now.duration_since(t) > Duration::from_secs(1) {
                    grid.probe_cancel(0);
                    typed_at = None;
                    lost += 1;
                }
            }
            if typed_at.is_none() && now >= next_key {
                if on_line >= 30 {
                    on_line = 0;
                    p.enter();
                    next_key = now + Duration::from_millis(300);
                } else {
                    let ch = (b'a' + (seq % 26) as u8) as char;
                    seq += 1;
                    on_line += 1;
                    grid.probe_arm(0, ch);
                    typed_at = Some(Instant::now());
                    p.key(ch);
                    // Jittered so typing does not lock to the frame phase.
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    next_key = now + Duration::from_millis(90 + rng % 80);
                }
            }
        }
        if seen == drawn_changes {
            let wake = next_key.min(now + Duration::from_millis(20));
            seen = grid.wait(seen, wake);
            if seen == drawn_changes {
                continue;
            }
        }
        if let Some(slot) = last_frame.map(|l| l + PERIOD) {
            if let Some(d) = slot.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
        drawn_changes = grid.wait(seen, Instant::now());
        seen = drawn_changes;
        let t0 = Instant::now();
        let f = p.frame();
        let t1 = Instant::now();
        if !f.drawn {
            continue;
        }
        last_frame = Some(t0);
        if window.is_some() {
            frames_ms.push(ms(t1 - t0));
            if f.probe_hit {
                if let Some(t) = typed_at.take() {
                    latency_ms.push(ms(t1 - t));
                }
            }
        }
    }
    let (w, u0) = window.unwrap_or((started, metrics::usage()));
    let u1 = metrics::usage();
    let secs = w.elapsed().as_secs_f64();
    json!({
        "panes": grid.panes(),
        "seconds": secs,
        "frames": frames_ms.len(),
        "fps": frames_ms.len() as f64 / secs,
        "frame_ms": summary(&mut frames_ms),
        "latency_ms": summary(&mut latency_ms),
        "lost_probes": lost,
        "cpu_pct": 100.0 * (u1.cpu_s - u0.cpu_s) / secs,
        "peak_rss_mb": u1.peak_rss as f64 / 1048576.0,
        "footprint_mb": u1.footprint as f64 / 1048576.0,
        "errors": grid.errors(),
    })
}

/// Draws until every pane has its screen; answers how long that took.
pub fn first_frame(grid: &Grid, p: &mut dyn Proto, timeout: Duration) -> Option<Duration> {
    let t = Instant::now();
    let mut seen = 0;
    while t.elapsed() < timeout {
        seen = grid.wait(seen, Instant::now() + Duration::from_millis(20));
        if grid.all_snapshotted() && p.frame().complete {
            return Some(t.elapsed());
        }
    }
    None
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// p50/p95/p99/max of a sample, in its own unit.
pub fn summary(v: &mut [f64]) -> Value {
    if v.is_empty() {
        return json!({ "n": 0 });
    }
    v.sort_by(f64::total_cmp);
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    json!({
        "n": v.len(),
        "p50": r(q(0.5)),
        "p95": r(q(0.95)),
        "p99": r(q(0.99)),
        "max": r(v[v.len() - 1]),
        "mean": r(v.iter().sum::<f64>() / v.len() as f64),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn summary_quantiles() {
        let mut v: Vec<f64> = (1..=101).rev().map(f64::from).collect();
        let s = super::summary(&mut v);
        assert_eq!(s["p50"], 51.0);
        assert_eq!(s["p99"], 100.0);
        assert_eq!(s["max"], 101.0);
    }
}
