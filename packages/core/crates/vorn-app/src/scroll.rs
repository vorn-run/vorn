//! A terminal's scroll position over its history, as a Mac scrolls: a
//! trackpad moves it by the pixel, momentum carries it on, it stretches
//! past either end and springs back, and it comes to rest on a whole row.
//! A notched wheel moves [`NOTCH_ROWS`] rows with a short ease.
//!
//! The position is pixels above the live bottom: 0 is the live screen,
//! `max` the oldest line of history. Wheel `dy` is positive towards the
//! live bottom (it shows what is below), as vornui reports it.

use std::time::Duration;
use web_time::Instant;

use vornui::motion::spring;
use vornui::WheelPhase;

/// Rows one wheel notch scrolls.
pub const NOTCH_ROWS: f32 = 3.0;
/// A touch that lifts and a `Began` this soon after is the momentum.
const MOMENTUM_GAP: Duration = Duration::from_millis(60);
/// Phaseless pixel deltas settle once they stop this long.
const IDLE: Duration = Duration::from_millis(150);
/// A notch's ease.
const EASE: Duration = Duration::from_millis(120);
/// The spring that settles on a row, and the softer one back from an edge.
const SNAP: Duration = Duration::from_millis(200);
const BOUNCE: Duration = Duration::from_millis(300);
/// How stiff the stretch past an edge is (the system's rubber band).
const BAND: f32 = 0.55;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Motion {
    Rest,
    /// Fingers on the trackpad; `raw` is where they would put it unstretched.
    Touch {
        raw: f32,
    },
    /// Fingers lifted; momentum may follow.
    Lifted {
        at: Instant,
    },
    Momentum {
        at: Instant,
    },
    /// Pixel deltas without phases, as some platforms send.
    Free {
        at: Instant,
    },
    Spring {
        x0: f32,
        v0: f32,
        to: f32,
        start: Instant,
        dur: Duration,
    },
    Ease {
        from: f32,
        to: f32,
        start: Instant,
    },
}

/// One pane's scroll position and the motion it is in.
#[derive(Debug, Clone, PartialEq)]
pub struct Scroll {
    pos: f32,
    max: f32,
    row: f32,
    /// The stretch's scale: the pane's height.
    span: f32,
    /// Pixels a second, positive into history.
    vel: f32,
    last: Option<Instant>,
    motion: Motion,
    /// A spring took over at an edge: the rest of the momentum is ignored.
    spent: bool,
    /// Reduce motion: a notch jumps instead of easing.
    pub reduced: bool,
    /// Rows a wheel notch moves, as the system is set; `None` for a page.
    pub lines: Option<f32>,
}

impl Default for Scroll {
    fn default() -> Self {
        Scroll {
            pos: 0.0,
            max: 0.0,
            row: 1.0,
            span: 1.0,
            vel: 0.0,
            last: None,
            motion: Motion::Rest,
            spent: false,
            reduced: false,
            lines: Some(NOTCH_ROWS),
        }
    }
}

impl Scroll {
    /// Pixels above the live bottom; past 0 or `max` while stretched.
    pub fn pos(&self) -> f32 {
        self.pos
    }

    /// Whether it rests away from the live screen, or is headed there.
    pub fn scrolled(&self) -> bool {
        self.pos > 0.5 || self.target() > 0.5
    }

    pub fn resting(&self) -> bool {
        self.motion == Motion::Rest
    }

    /// Row height, pane height and history rows, in the pixels it moves in.
    pub fn set_metrics(&mut self, row: f32, span: f32, history_rows: u64) {
        let row = row.max(1.0);
        if row != self.row && self.motion == Motion::Rest {
            // Keep the same rows in view across a font change.
            self.pos = (self.pos / self.row).round() * row;
        }
        self.row = row;
        self.span = span.max(row);
        self.max = history_rows as f32 * row;
        if self.motion == Motion::Rest {
            self.pos = self.pos.clamp(0.0, self.max);
        }
    }

    /// `rows` new lines went into history: a view scrolled back keeps
    /// showing the same lines; one at the live screen follows it.
    pub fn grew(&mut self, rows: u64) {
        if rows == 0 || !self.scrolled() {
            return;
        }
        let px = rows as f32 * self.row;
        self.pos += px;
        match &mut self.motion {
            Motion::Touch { raw } => *raw += px,
            Motion::Spring { to, .. } => *to += px,
            Motion::Ease { from, to, .. } => {
                *from += px;
                *to += px;
            }
            _ => {}
        }
    }

    /// Straight to `pos`, as a scrollbar drag puts it.
    pub fn jump(&mut self, pos: f32) {
        self.rest_at(pos.clamp(0.0, self.max));
    }

    /// Straight back to the live screen, as typing does.
    pub fn reset(&mut self) {
        self.pos = 0.0;
        self.vel = 0.0;
        self.motion = Motion::Rest;
    }

    /// A wheel or trackpad event of `dy` pixels at `now`.
    pub fn wheel(&mut self, dy: f32, phase: WheelPhase, now: Instant) {
        match phase {
            WheelPhase::Line => {
                let notches = dy / vornui::WHEEL_LINE;
                if (notches - notches.round()).abs() < 1e-3 {
                    self.notch(notches, now);
                } else {
                    // A fraction of a notch (a precision touchpad, a free-spinning wheel) moves by the pixel.
                    self.moved(notches * self.notch_rows() * self.row, now);
                }
            }
            WheelPhase::Began => self.began(now),
            WheelPhase::Moved => self.moved(dy, now),
            WheelPhase::Ended => self.ended(now),
        }
    }

    fn notch(&mut self, notches: f32, now: Instant) {
        let base = match self.motion {
            Motion::Ease { to, .. } => to,
            _ => self.pos,
        };
        let to = self.snapped(base - notches * self.notch_rows() * self.row);
        if self.reduced {
            self.rest_at(to);
            return;
        }
        self.vel = 0.0;
        self.motion = Motion::Ease {
            from: self.pos,
            to,
            start: now,
        };
    }

    /// Rows one notch moves: the system's lines, or a page less a row.
    fn notch_rows(&self) -> f32 {
        self.lines
            .unwrap_or_else(|| (self.span / self.row - 1.0).max(1.0))
    }

    fn began(&mut self, now: Instant) {
        if let Motion::Lifted { at } = self.motion {
            if now.saturating_duration_since(at) <= MOMENTUM_GAP {
                if self.inside() {
                    self.motion = Motion::Momentum { at: now };
                } else {
                    // Lifted past an edge: it springs back, the momentum is not wanted.
                    self.spent = true;
                    self.settle(now);
                }
                return;
            }
        }
        if matches!(self.motion, Motion::Touch { .. }) {
            return;
        }
        self.spent = false;
        self.vel = 0.0;
        self.last = Some(now);
        self.motion = Motion::Touch {
            raw: self.unstretch(self.pos),
        };
    }

    fn moved(&mut self, dy: f32, now: Instant) {
        let dt = self
            .last
            .map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        self.last = Some(now);
        let v = if dt > 0.0 {
            -dy / dt.max(0.004)
        } else {
            self.vel
        };
        match self.motion {
            Motion::Touch { raw } => {
                let raw = raw - dy;
                self.pos = self.stretch(raw);
                self.vel = 0.6 * v + 0.4 * self.vel;
                self.motion = Motion::Touch { raw };
            }
            Motion::Momentum { .. } => {
                self.vel = 0.6 * v + 0.4 * self.vel;
                let next = self.pos - dy;
                if (0.0..=self.max).contains(&next) {
                    self.pos = next;
                    self.motion = Motion::Momentum { at: now };
                } else {
                    // Out over the edge with the momentum's speed, then back.
                    self.pos = next.clamp(0.0, self.max);
                    self.spent = true;
                    self.spring_to(self.pos, BOUNCE, now);
                }
            }
            Motion::Spring { .. } | Motion::Lifted { .. } if self.spent => {}
            _ => {
                self.pos = (self.pos - dy).clamp(0.0, self.max);
                self.vel = 0.0;
                self.motion = Motion::Free { at: now };
            }
        }
    }

    fn ended(&mut self, now: Instant) {
        match self.motion {
            Motion::Touch { .. } => {
                // Fingers that stopped before lifting throw nothing.
                if self
                    .last
                    .is_none_or(|l| now.saturating_duration_since(l) > Duration::from_millis(50))
                {
                    self.vel = 0.0;
                }
                self.motion = Motion::Lifted { at: now };
            }
            Motion::Momentum { .. } => {
                self.vel = 0.0;
                self.settle(now);
            }
            _ => {}
        }
        if !matches!(self.motion, Motion::Lifted { .. }) {
            self.spent = false;
        }
    }

    /// Moves the animation to `now`; returns when it next needs a frame,
    /// `now` itself while it is moving.
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        match self.motion {
            Motion::Rest | Motion::Touch { .. } => None,
            Motion::Lifted { at } => {
                if now.saturating_duration_since(at) >= MOMENTUM_GAP {
                    self.settle(now);
                    self.next(now)
                } else {
                    Some(at + MOMENTUM_GAP)
                }
            }
            Motion::Momentum { at } | Motion::Free { at } => {
                // Momentum that never says it ended settles like phaseless deltas.
                if now.saturating_duration_since(at) >= IDLE {
                    self.vel = 0.0;
                    self.settle(now);
                    self.next(now)
                } else {
                    Some(at + IDLE)
                }
            }
            Motion::Spring {
                x0,
                v0,
                to,
                start,
                dur,
            } => {
                let t = now.saturating_duration_since(start).as_secs_f32();
                let (x, v) = spring(dur, 0.0, x0, v0, t);
                if (x.abs() < 0.5 && v.abs() < 20.0) || t > 1.5 {
                    self.rest_at(to);
                    None
                } else {
                    self.pos = to + x;
                    self.vel = v;
                    Some(now)
                }
            }
            Motion::Ease { from, to, start } => {
                let t = now.saturating_duration_since(start).as_secs_f32() / EASE.as_secs_f32();
                if t >= 1.0 {
                    self.rest_at(to);
                    None
                } else {
                    let e = 1.0 - (1.0 - t).powi(3);
                    self.pos = from + (to - from) * e;
                    Some(now)
                }
            }
        }
    }

    fn next(&self, now: Instant) -> Option<Instant> {
        (self.motion != Motion::Rest).then_some(now)
    }

    fn rest_at(&mut self, to: f32) {
        self.pos = to;
        self.vel = 0.0;
        self.motion = Motion::Rest;
    }

    /// Springs to the nearest row within the history.
    fn settle(&mut self, now: Instant) {
        let inside = self.inside();
        let to = self.snapped(self.pos);
        if (self.pos - to).abs() < 0.5 && self.vel.abs() < 20.0 {
            self.rest_at(to);
            return;
        }
        if !inside || self.vel.abs() < 1.0 {
            self.motion = Motion::Spring {
                x0: self.pos - to,
                v0: if inside { 0.0 } else { self.vel },
                to,
                start: now,
                dur: if inside { SNAP } else { BOUNCE },
            };
        } else {
            self.spring_to(to, SNAP, now);
        }
    }

    fn spring_to(&mut self, to: f32, dur: Duration, now: Instant) {
        self.motion = Motion::Spring {
            x0: self.pos - to,
            v0: self.vel,
            to,
            start: now,
            dur,
        };
    }

    fn target(&self) -> f32 {
        match self.motion {
            Motion::Spring { to, .. } | Motion::Ease { to, .. } => to,
            _ => self.pos,
        }
    }

    fn inside(&self) -> bool {
        (0.0..=self.max).contains(&self.pos)
    }

    fn snapped(&self, px: f32) -> f32 {
        ((px / self.row).round() * self.row).clamp(0.0, self.max)
    }

    /// Where fingers at `raw` show it: past an edge, ever less far.
    fn stretch(&self, raw: f32) -> f32 {
        let band = |x: f32| (1.0 - 1.0 / (x * BAND / self.span + 1.0)) * self.span;
        if raw < 0.0 {
            -band(-raw)
        } else if raw > self.max {
            self.max + band(raw - self.max)
        } else {
            raw
        }
    }

    fn unstretch(&self, pos: f32) -> f32 {
        let unband = |y: f32| {
            let y = y.min(self.span * 0.99);
            self.span / BAND * (1.0 / (1.0 - y / self.span) - 1.0)
        };
        if pos < 0.0 {
            -unband(-pos)
        } else if pos > self.max {
            self.max + unband(pos - self.max)
        } else {
            pos
        }
    }

    /// The rows back from the live screen the view starts at, and the
    /// whole-pixel offset to draw it at (positive moves it down).
    pub fn split(&self) -> (u64, f32) {
        let inside = self.pos.clamp(0.0, self.max);
        let back = (inside / self.row - 1e-3).ceil().max(0.0);
        (back as u64, (self.pos - back * self.row).round())
    }
}

/// Where a wheel goes: the program when it tracks the mouse, arrow keys
/// on the alternate screen (as other terminals do), else the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Program,
    Arrows,
    History,
}

pub fn route(mouse: bool, alternate: bool) -> Route {
    if mouse {
        Route::Program
    } else if alternate {
        Route::Arrows
    } else {
        Route::History
    }
}

/// Whole rows out of trackpad travel for a program: the fraction carries.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rows {
    rest: f32,
}

impl Rows {
    /// Rows for `dy` pixels of `row` height, positive down; a notch is
    /// one wheel report to a program, [`NOTCH_ROWS`] arrow presses.
    pub fn take(&mut self, dy: f32, phase: WheelPhase, row: f32, route: Route) -> i32 {
        let rows = match phase {
            // A fraction of a notch carries to the next, so a touchpad's small steps add up.
            WheelPhase::Line => {
                let per = if route == Route::Arrows {
                    NOTCH_ROWS
                } else {
                    1.0
                };
                let notches = dy / vornui::WHEEL_LINE;
                if (notches - notches.round()).abs() < 1e-3 {
                    self.rest = 0.0;
                    return (notches.round() * per) as i32;
                }
                self.rest + notches * per
            }
            WheelPhase::Began | WheelPhase::Ended => {
                self.rest = 0.0;
                return 0;
            }
            WheelPhase::Moved => self.rest + dy / row.max(1.0),
        };
        // Ten tenths of a notch make one, float rounding or not.
        let whole = (rows + rows.signum() * 1e-4).trunc();
        self.rest = rows - whole;
        whole as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 20.0;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// 100 rows of history in a 400 px pane.
    fn scroll() -> Scroll {
        let mut s = Scroll::default();
        s.set_metrics(ROW, 400.0, 100);
        s
    }

    fn run(s: &mut Scroll, t0: Instant, from_ms: u64) -> u64 {
        let mut ms = from_ms;
        while s.tick(at(t0, ms)).is_some() {
            ms += 8;
            assert!(ms < from_ms + 3000, "never settled: {s:?}");
        }
        ms
    }

    #[test]
    fn a_touch_moves_by_the_pixel_and_settles_on_a_row() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.wheel(0.0, WheelPhase::Began, t0);
        s.wheel(-7.0, WheelPhase::Moved, at(t0, 8));
        s.wheel(-6.0, WheelPhase::Moved, at(t0, 16));
        assert_eq!(s.pos(), 13.0);
        s.wheel(0.0, WheelPhase::Ended, at(t0, 24));
        // Still at the pixel while momentum may come.
        assert_eq!(s.tick(at(t0, 30)), Some(at(t0, 24) + MOMENTUM_GAP));
        assert_eq!(s.pos(), 13.0);
        run(&mut s, t0, 90);
        assert_eq!(s.pos(), ROW);
        assert!(s.resting());
    }

    #[test]
    fn momentum_carries_on_after_the_touch() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.wheel(0.0, WheelPhase::Began, t0);
        s.wheel(-30.0, WheelPhase::Moved, at(t0, 8));
        s.wheel(0.0, WheelPhase::Ended, at(t0, 16));
        s.wheel(0.0, WheelPhase::Began, at(t0, 24));
        for i in 0..10 {
            s.wheel(-10.0, WheelPhase::Moved, at(t0, 32 + i * 8));
        }
        assert_eq!(s.pos(), 130.0);
        s.wheel(0.0, WheelPhase::Ended, at(t0, 120));
        run(&mut s, t0, 128);
        assert_eq!(s.pos(), 140.0);
    }

    #[test]
    fn a_late_began_is_a_new_touch_not_momentum() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.wheel(0.0, WheelPhase::Began, t0);
        s.wheel(-40.0, WheelPhase::Moved, at(t0, 8));
        s.wheel(0.0, WheelPhase::Ended, at(t0, 16));
        run(&mut s, t0, 100);
        s.wheel(0.0, WheelPhase::Began, at(t0, 400));
        assert!(matches!(s.motion, Motion::Touch { .. }));
    }

    #[test]
    fn pulling_past_the_live_bottom_stretches_less_and_springs_back() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.wheel(0.0, WheelPhase::Began, t0);
        s.wheel(100.0, WheelPhase::Moved, at(t0, 8));
        let p1 = s.pos();
        assert!(p1 < 0.0 && p1 > -100.0, "{p1}");
        s.wheel(100.0, WheelPhase::Moved, at(t0, 16));
        let p2 = s.pos();
        assert!(p2 - p1 > -(p1.abs()), "each pixel pulls less: {p1} {p2}");
        assert!(p2 > -400.0);
        s.wheel(0.0, WheelPhase::Ended, at(t0, 24));
        run(&mut s, t0, 32);
        assert_eq!(s.pos(), 0.0);
    }

    #[test]
    fn momentum_over_the_top_overshoots_then_springs_back() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.set_metrics(ROW, 400.0, 2);
        s.wheel(0.0, WheelPhase::Began, t0);
        s.wheel(-30.0, WheelPhase::Moved, at(t0, 8));
        s.wheel(0.0, WheelPhase::Ended, at(t0, 16));
        s.wheel(0.0, WheelPhase::Began, at(t0, 20));
        s.wheel(-20.0, WheelPhase::Moved, at(t0, 28));
        assert_eq!(s.pos(), 40.0);
        assert!(matches!(s.motion, Motion::Spring { .. }));
        // Further momentum is ignored while the spring has it.
        s.wheel(-20.0, WheelPhase::Moved, at(t0, 36));
        s.tick(at(t0, 60));
        assert!(s.pos() > 40.0, "overshoots: {}", s.pos());
        run(&mut s, t0, 60);
        assert_eq!(s.pos(), 40.0);
    }

    #[test]
    fn a_notch_eases_three_rows() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.wheel(-vornui::WHEEL_LINE, WheelPhase::Line, t0);
        s.tick(at(t0, 40));
        let mid = s.pos();
        assert!(mid > 0.0 && mid < 3.0 * ROW, "{mid}");
        // A second notch mid-ease aims past the first's end.
        s.wheel(-vornui::WHEEL_LINE, WheelPhase::Line, at(t0, 40));
        run(&mut s, t0, 48);
        assert_eq!(s.pos(), 6.0 * ROW);
        s.wheel(vornui::WHEEL_LINE * 9.0, WheelPhase::Line, at(t0, 500));
        run(&mut s, t0, 500);
        assert_eq!(s.pos(), 0.0);
    }

    #[test]
    fn reduced_motion_jumps_a_notch() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.reduced = true;
        s.wheel(-vornui::WHEEL_LINE, WheelPhase::Line, t0);
        assert_eq!((s.pos(), s.resting()), (3.0 * ROW, true));
    }

    #[test]
    fn phaseless_pixels_clamp_and_settle_when_idle() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.wheel(-25.0, WheelPhase::Moved, t0);
        assert_eq!(s.pos(), 25.0);
        s.wheel(100.0, WheelPhase::Moved, at(t0, 8));
        assert_eq!(s.pos(), 0.0);
        s.wheel(-27.0, WheelPhase::Moved, at(t0, 16));
        assert_eq!(s.tick(at(t0, 20)), Some(at(t0, 16) + IDLE));
        run(&mut s, t0, 200);
        assert_eq!(s.pos(), ROW);
    }

    #[test]
    fn new_output_keeps_a_scrolled_view_on_its_lines() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.wheel(-vornui::WHEEL_LINE, WheelPhase::Line, t0);
        run(&mut s, t0, 8);
        s.set_metrics(ROW, 400.0, 105);
        s.grew(5);
        assert_eq!(s.pos(), 8.0 * ROW);
        let mut live = scroll();
        live.grew(5);
        assert_eq!(live.pos(), 0.0);
    }

    #[test]
    fn split_gives_rows_back_and_a_pixel_offset() {
        let mut s = scroll();
        s.pos = 0.0;
        assert_eq!(s.split(), (0, 0.0));
        s.pos = 45.0;
        assert_eq!(s.split(), (3, -15.0));
        s.pos = 40.0;
        assert_eq!(s.split(), (2, 0.0));
        s.pos = -12.0;
        assert_eq!(s.split(), (0, -12.0));
        s.pos = 2010.0;
        assert_eq!(s.split(), (100, 10.0));
    }

    #[test]
    fn typing_returns_to_the_live_screen() {
        let t0 = Instant::now();
        let mut s = scroll();
        s.wheel(-vornui::WHEEL_LINE, WheelPhase::Line, t0);
        s.reset();
        assert_eq!((s.pos(), s.resting(), s.scrolled()), (0.0, true, false));
    }

    #[test]
    fn wheels_go_to_the_program_then_arrows_then_history() {
        assert_eq!(route(true, true), Route::Program);
        assert_eq!(route(true, false), Route::Program);
        assert_eq!(route(false, true), Route::Arrows);
        assert_eq!(route(false, false), Route::History);
    }

    #[test]
    fn trackpad_travel_reaches_a_program_a_row_at_a_time() {
        let mut r = Rows::default();
        let p = Route::Program;
        assert_eq!(r.take(-12.0, WheelPhase::Moved, ROW, p), 0);
        assert_eq!(r.take(-12.0, WheelPhase::Moved, ROW, p), -1);
        assert_eq!(r.take(-50.0, WheelPhase::Moved, ROW, p), -2);
        assert_eq!(r.take(0.0, WheelPhase::Ended, ROW, p), 0);
        assert_eq!(r.take(-12.0, WheelPhase::Moved, ROW, p), 0);
        let notch = vornui::WHEEL_LINE;
        assert_eq!(r.take(notch, WheelPhase::Line, ROW, p), 1);
        assert_eq!(r.take(-notch, WheelPhase::Line, ROW, Route::Arrows), -3);
    }

    #[test]
    fn a_fraction_of_a_notch_moves_by_the_pixel_and_settles_on_a_row() {
        let t0 = Instant::now();
        let mut s = Scroll::default();
        s.set_metrics(ROW, 400.0, 100);
        // A precision touchpad's tenths of a notch, upward into history.
        for i in 0..5 {
            s.wheel(-0.1 * vornui::WHEEL_LINE, WheelPhase::Line, at(t0, i * 8));
        }
        let moved = 5.0 * 0.1 * NOTCH_ROWS * ROW;
        assert!(
            (s.pos() - moved).abs() < 0.01,
            "{} by the pixel, no ease",
            s.pos()
        );
        let mut now = at(t0, 40);
        while let Some(next) = s.tick(now) {
            now = next.max(now + Duration::from_millis(16));
        }
        assert_eq!(s.pos() % ROW, 0.0, "rests on a row");
    }

    #[test]
    fn a_notch_moves_the_systems_lines_or_a_page() {
        let t0 = Instant::now();
        let mut s = Scroll {
            reduced: true,
            ..Scroll::default()
        };
        s.set_metrics(ROW, 200.0, 100);
        s.lines = Some(5.0);
        s.wheel(-vornui::WHEEL_LINE, WheelPhase::Line, t0);
        assert_eq!(s.pos(), 5.0 * ROW);
        s.lines = None;
        s.wheel(-vornui::WHEEL_LINE, WheelPhase::Line, t0);
        assert_eq!(s.pos(), 5.0 * ROW + 9.0 * ROW, "a page less a row");
    }

    #[test]
    fn tenths_of_a_notch_add_up_to_whole_reports_for_a_program() {
        let mut rows = Rows::default();
        let sent: i32 = (0..10)
            .map(|_| {
                rows.take(
                    0.1 * vornui::WHEEL_LINE,
                    WheelPhase::Line,
                    ROW,
                    Route::Program,
                )
            })
            .sum();
        assert_eq!(sent, 1);
        let mut arrows = Rows::default();
        assert_eq!(
            arrows.take(vornui::WHEEL_LINE, WheelPhase::Line, ROW, Route::Arrows),
            3
        );
        assert_eq!(
            arrows.take(
                -0.5 * vornui::WHEEL_LINE,
                WheelPhase::Line,
                ROW,
                Route::Arrows
            ),
            -1
        );
    }

    #[test]
    fn a_jump_lands_inside_the_history() {
        let mut s = Scroll::default();
        s.set_metrics(ROW, 400.0, 10);
        s.jump(5.0 * ROW);
        assert_eq!(s.pos(), 5.0 * ROW);
        s.jump(1e6);
        assert_eq!(s.pos(), 10.0 * ROW);
        assert!(s.resting());
    }
}
