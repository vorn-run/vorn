//! Motion: values that move to a new target over time instead of jumping,
//! with a CSS cubic-bezier or a spring.
//!
//! A [`Motion`] holds where a value is heading and how it got there, and
//! answers what it is at any instant in closed form, so a frame costs one
//! evaluation per moving value and nothing is ticked in between. Retargeting
//! mid-flight starts from the current value (and, for a spring, its
//! velocity), so interrupted motion never jumps. The [`Ui`](crate::Ui)
//! keeps one per key (see `Ui::animate`) and asks for frames only while one
//! is unsettled; once everything settles the window sleeps.

use std::time::{Duration, Instant};

use crate::scene::Rgba;
use crate::theme;

/// How a value travels to its target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transition {
    /// CSS `transition`: `duration` along a cubic-bezier timing function
    /// `(x1, y1, x2, y2)`; `(0, 0, 1, 1)` is linear.
    Tween { duration: Duration, curve: [f32; 4] },
    /// A damped spring that takes about `duration` to arrive, overshooting
    /// by `bounce` (0: none, as critically damped; towards 1: springier).
    /// It carries its velocity through a retarget.
    Spring { duration: Duration, bounce: f32 },
}

impl Transition {
    /// No motion: the value is at its target at once.
    pub const INSTANT: Transition = Transition::Tween {
        duration: Duration::ZERO,
        curve: [0.0, 0.0, 1.0, 1.0],
    };

    /// Tailwind's `transition-colors`: 150 ms, `ease-in-out`.
    pub const COLORS: Transition = Transition::Tween {
        duration: theme::TRANSITION,
        curve: [0.4, 0.0, 0.2, 1.0],
    };

    /// The app's menu and popover spring: 0.2 s, bounce 0.1.
    pub const POPOVER: Transition = Transition::Spring {
        duration: Duration::from_millis(200),
        bounce: 0.1,
    };

    pub const fn linear(duration: Duration) -> Transition {
        Transition::Tween {
            duration,
            curve: [0.0, 0.0, 1.0, 1.0],
        }
    }

    pub const fn cubic_bezier(
        duration: Duration,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> Transition {
        Transition::Tween {
            duration,
            curve: [x1, y1, x2, y2],
        }
    }

    pub const fn spring(duration: Duration, bounce: f32) -> Transition {
        Transition::Spring { duration, bounce }
    }
}

/// A value [`Motion`] can move: up to four numbers, each moved alone.
pub trait Animate: Copy + PartialEq {
    /// Whether it moves something on screen (position, size, scale), so
    /// reduced motion makes it jump; colours and opacity still fade.
    const MOVES: bool;
    fn to_lanes(self) -> [f32; 4];
    fn from_lanes(l: [f32; 4]) -> Self;
}

/// Opacity, a progress fraction, an angle: fades under reduced motion.
impl Animate for f32 {
    const MOVES: bool = false;
    fn to_lanes(self) -> [f32; 4] {
        [self, 0.0, 0.0, 0.0]
    }
    fn from_lanes(l: [f32; 4]) -> f32 {
        l[0]
    }
}

/// A size or an offset, logical pixels.
impl Animate for (f32, f32) {
    const MOVES: bool = true;
    fn to_lanes(self) -> [f32; 4] {
        [self.0, self.1, 0.0, 0.0]
    }
    fn from_lanes(l: [f32; 4]) -> (f32, f32) {
        (l[0], l[1])
    }
}

/// Interpolated per channel in sRGB, as CSS transitions colours.
impl Animate for Rgba {
    const MOVES: bool = false;
    fn to_lanes(self) -> [f32; 4] {
        self.0
    }
    fn from_lanes(l: [f32; 4]) -> Rgba {
        Rgba(l)
    }
}

/// An element's transform as [`El::transform`](crate::El::transform)
/// takes it: a scale about the centre, then a shift.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub scale: f32,
    pub offset: (f32, f32),
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        scale: 1.0,
        offset: (0.0, 0.0),
    };
}

impl Animate for Transform {
    const MOVES: bool = true;
    fn to_lanes(self) -> [f32; 4] {
        [self.scale, self.offset.0, self.offset.1, 0.0]
    }
    fn from_lanes(l: [f32; 4]) -> Transform {
        Transform {
            scale: l[0],
            offset: (l[1], l[2]),
        }
    }
}

/// Lanes nearer their target than this, and as slow, have arrived: a
/// 2000th of a colour channel, a 500th of a pixel.
const SETTLE: f32 = 0.002;
/// Springs are given up on after this long whatever their state.
const MAX_SPRING: f32 = 10.0;

/// One value in flight: where it set off from, how fast it was going,
/// where it is heading, and when the leg began.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    from: [f32; 4],
    velocity: [f32; 4],
    target: [f32; 4],
    at: Instant,
    transition: Transition,
    /// Seconds after `at` when the value has arrived.
    settles: f32,
}

impl Motion {
    /// A value at rest at `v`.
    pub fn at_rest<T: Animate>(v: T, now: Instant) -> Motion {
        let l = v.to_lanes();
        Motion {
            from: l,
            velocity: [0.0; 4],
            target: l,
            at: now,
            transition: Transition::INSTANT,
            settles: 0.0,
        }
    }

    /// Heads for `target` from wherever the value is at `now`, keeping its
    /// velocity under a spring. A no-op when it is already heading there.
    pub fn retarget<T: Animate>(&mut self, target: T, transition: Transition, now: Instant) {
        let target = target.to_lanes();
        if target == self.target && transition == self.transition {
            return;
        }
        let (from, velocity) = self.state(now);
        self.from = from;
        self.velocity = velocity;
        self.target = target;
        self.at = now;
        self.transition = transition;
        self.settles = settle_time(transition, from, velocity, target);
    }

    /// Jumps to `v` and rests there.
    pub fn snap<T: Animate>(&mut self, v: T, now: Instant) {
        *self = Motion::at_rest(v, now);
    }

    /// The value at `now`.
    pub fn value<T: Animate>(&self, now: Instant) -> T {
        T::from_lanes(self.state(now).0)
    }

    /// Whether the value has arrived by `now`.
    pub fn settled(&self, now: Instant) -> bool {
        self.elapsed(now) >= self.settles
    }

    /// When it arrives.
    pub fn settles_at(&self) -> Instant {
        self.at + Duration::from_secs_f32(self.settles)
    }

    fn elapsed(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.at).as_secs_f32()
    }

    /// Value and velocity (per second) of each lane at `now`.
    fn state(&self, now: Instant) -> ([f32; 4], [f32; 4]) {
        let t = self.elapsed(now);
        if t >= self.settles {
            return (self.target, [0.0; 4]);
        }
        let mut v = [0.0; 4];
        let mut dv = [0.0; 4];
        for i in 0..4 {
            (v[i], dv[i]) = lane(
                self.transition,
                self.from[i],
                self.velocity[i],
                self.target[i],
                t,
            );
        }
        (v, dv)
    }
}

/// One lane's value and velocity `t` seconds into a leg.
fn lane(tr: Transition, from: f32, v0: f32, to: f32, t: f32) -> (f32, f32) {
    match tr {
        Transition::Tween { duration, curve } => {
            let d = duration.as_secs_f32();
            if d <= 0.0 || t >= d {
                return (to, 0.0);
            }
            let ease = |x: f32| theme::cubic_bezier(curve[0], curve[1], curve[2], curve[3], x);
            let x = t / d;
            let h = 1e-3;
            let slope = (ease((x + h).min(1.0)) - ease((x - h).max(0.0))) / (2.0 * h);
            (from + (to - from) * ease(x), (to - from) * slope / d)
        }
        Transition::Spring { duration, bounce } => {
            let (x, v) = spring(duration, bounce, from - to, v0, t);
            (to + x, v)
        }
    }
}

/// A unit-mass spring's displacement and velocity `t` seconds after it
/// was at `x0` moving at `v0`. Stiffness gives a period of `duration`
/// and the damping ratio is `1 - bounce`, so `duration` reads as how long
/// the motion takes and `bounce` as how much it overshoots.
pub fn spring(duration: Duration, bounce: f32, x0: f32, v0: f32, t: f32) -> (f32, f32) {
    let w0 = std::f32::consts::TAU / duration.as_secs_f32().max(1e-3);
    let zeta = (1.0 - bounce).clamp(0.05, 2.0);
    if (zeta - 1.0).abs() < 1e-3 {
        let e = (-w0 * t).exp();
        let b = v0 + w0 * x0;
        let x = e * (x0 + b * t);
        let v = e * (b - w0 * (x0 + b * t));
        (x, v)
    } else if zeta < 1.0 {
        let wd = w0 * (1.0 - zeta * zeta).sqrt();
        let e = (-zeta * w0 * t).exp();
        let b = (v0 + zeta * w0 * x0) / wd;
        let (s, c) = (wd * t).sin_cos();
        let x = e * (x0 * c + b * s);
        let v = e * ((b * wd - zeta * w0 * x0) * c - (x0 * wd + zeta * w0 * b) * s);
        (x, v)
    } else {
        let r = w0 * (zeta * zeta - 1.0).sqrt();
        let (r1, r2) = (-zeta * w0 + r, -zeta * w0 - r);
        let c2 = (v0 - r1 * x0) / (r2 - r1);
        let c1 = x0 - c2;
        let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
        (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    }
}

/// Seconds until every lane rests within [`SETTLE`] of its target.
fn settle_time(tr: Transition, from: [f32; 4], v0: [f32; 4], to: [f32; 4]) -> f32 {
    match tr {
        Transition::Tween { duration, .. } => {
            if from == to {
                0.0
            } else {
                duration.as_secs_f32()
            }
        }
        Transition::Spring { duration, bounce } => {
            // Stepping finds when the spring last leaves the band; it runs
            // once per retarget, not per frame.
            let step = 1.0 / 240.0;
            let mut last_out = 0.0;
            let mut t = 0.0;
            while t < MAX_SPRING {
                let out = (0..4).any(|i| {
                    let (x, v) = spring(duration, bounce, from[i] - to[i], v0[i], t);
                    x.abs() > SETTLE || v.abs() > SETTLE * 10.0
                });
                if out {
                    last_out = t + step;
                } else if t > last_out + duration.as_secs_f32() {
                    break;
                }
                t += step;
            }
            last_out.min(MAX_SPRING)
        }
    }
}

/// Whether the platform asks for less motion: `VORNUI_REDUCE_MOTION=1`, or
/// Windows' "Show animations in Windows" turned off.
pub fn system_reduced_motion() -> bool {
    if let Some(v) = std::env::var_os("VORNUI_REDUCE_MOTION") {
        return v != "0" && !v.is_empty();
    }
    platform_reduced_motion()
}

#[cfg(windows)]
fn platform_reduced_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };
    let mut on: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the
    // pointer, which points at a live i32.
    let ok =
        unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut on).cast(), 0) };
    ok != 0 && on == 0
}

#[cfg(not(windows))]
fn platform_reduced_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, s: f32) -> Instant {
        t0 + Duration::from_secs_f32(s)
    }

    #[test]
    fn tween_follows_its_curve_and_arrives() {
        let t0 = Instant::now();
        let mut m = Motion::at_rest(0.0f32, t0);
        m.retarget(10.0f32, Transition::linear(Duration::from_millis(100)), t0);
        assert!((m.value::<f32>(at(t0, 0.05)) - 5.0).abs() < 1e-3);
        assert!(!m.settled(at(t0, 0.05)));
        assert_eq!(m.value::<f32>(at(t0, 0.1)), 10.0);
        assert!(m.settled(at(t0, 0.1)));
    }

    #[test]
    fn retarget_starts_from_the_current_value() {
        let t0 = Instant::now();
        let mut m = Motion::at_rest(0.0f32, t0);
        m.retarget(10.0f32, Transition::linear(Duration::from_millis(100)), t0);
        let mid = at(t0, 0.05);
        m.retarget(0.0f32, Transition::linear(Duration::from_millis(100)), mid);
        assert!((m.value::<f32>(mid) - 5.0).abs() < 1e-3);
        assert!((m.value::<f32>(at(t0, 0.1)) - 2.5).abs() < 1e-2);
    }

    #[test]
    fn spring_keeps_velocity_through_a_retarget() {
        let t0 = Instant::now();
        let tr = Transition::spring(Duration::from_millis(300), 0.0);
        let mut m = Motion::at_rest(0.0f32, t0);
        m.retarget(100.0f32, tr, t0);
        let mid = at(t0, 0.05);
        let before = m.value::<f32>(mid);
        // Retargeting further along the same line must not stall it.
        m.retarget(200.0f32, tr, mid);
        let after = m.value::<f32>(at(t0, 0.06));
        assert!(after > before);
    }

    #[test]
    fn springs_settle_on_target() {
        for bounce in [0.0, 0.1, 0.5, -0.5] {
            let t0 = Instant::now();
            let mut m = Motion::at_rest(Transform::IDENTITY, t0);
            let target = Transform {
                scale: 0.95,
                offset: (0.0, -8.0),
            };
            m.retarget(
                target,
                Transition::spring(Duration::from_millis(200), bounce),
                t0,
            );
            let end = m.settles_at();
            assert!(end > t0 && end < at(t0, 3.0), "bounce {bounce}");
            assert_eq!(m.value::<Transform>(end), target);
            let near: Transform = m.value(end - Duration::from_millis(1));
            assert!((near.offset.1 + 8.0).abs() < 0.01, "bounce {bounce}");
        }
    }

    #[test]
    fn a_bouncy_spring_overshoots_and_a_damped_one_does_not() {
        let d = Duration::from_millis(300);
        let peak = |bounce| {
            (0..200)
                .map(|i| 1.0 + spring(d, bounce, -1.0, 0.0, i as f32 / 200.0).0)
                .fold(f32::MIN, f32::max)
        };
        // ζ = 0.7 overshoots by e^(-πζ/√(1-ζ²)) ≈ 4.6%.
        assert!((peak(0.3) - 1.046).abs() < 0.005);
        assert!(peak(0.0) <= 1.0 + 1e-4);
    }

    #[test]
    fn spring_velocity_is_the_derivative() {
        let d = Duration::from_millis(250);
        for bounce in [0.0, 0.2, -0.4] {
            let t = 0.07;
            let h = 1e-3;
            let (x1, _) = spring(d, bounce, 3.0, -5.0, t - h);
            let (x2, _) = spring(d, bounce, 3.0, -5.0, t + h);
            let (_, v) = spring(d, bounce, 3.0, -5.0, t);
            assert!(((x2 - x1) / (2.0 * h) - v).abs() < 0.05 * v.abs().max(1.0));
        }
    }
}
