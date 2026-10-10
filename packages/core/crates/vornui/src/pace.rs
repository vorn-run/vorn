//! When to draw: on the display's beat normally, but right away when a
//! keystroke is waiting to be seen.
//!
//! A frame loop that only ever draws on its tick adds up to a whole period
//! between a key's echo arriving and its pixels: the echo lands just after
//! a tick and waits for the next. The pacer remembers that input is
//! waiting for its echo, and while it is, the first change to arrive is
//! drawn at once. Once a frame shows the echo, or after [`ECHO_WAIT`] in
//! case none comes (a key the program swallowed), it is back on the beat.

use std::time::{Duration, Instant};

/// How long input waits for its echo before the pacer stops hurrying.
pub const ECHO_WAIT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct Pacer {
    period: Duration,
    last: Option<Instant>,
    awaiting: Option<Instant>,
}

impl Pacer {
    /// A pacer for a display refreshing every `period`.
    pub fn new(period: Duration) -> Pacer {
        Pacer {
            period,
            last: None,
            awaiting: None,
        }
    }

    pub fn period(&self) -> Duration {
        self.period
    }

    /// Input arrived at `now`; its echo should be drawn as soon as it lands.
    pub fn input(&mut self, now: Instant) {
        self.awaiting.get_or_insert(now);
    }

    /// Whether a change arriving at `now` should be drawn immediately.
    pub fn urgent(&self, now: Instant) -> bool {
        self.awaiting
            .is_some_and(|since| now.saturating_duration_since(since) < ECHO_WAIT)
    }

    /// When the next frame should start if something changed at `now`.
    pub fn next_frame(&self, now: Instant) -> Instant {
        if self.urgent(now) {
            return now;
        }
        match self.last {
            Some(last) => (last + self.period).max(now),
            None => now,
        }
    }

    /// A frame started at `now`; `echoed` says it shows what the input was
    /// waiting for.
    pub fn frame(&mut self, now: Instant, echoed: bool) {
        self.last = Some(now);
        if echoed || !self.urgent(now) {
            self.awaiting = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Duration = Duration::from_millis(8);

    #[test]
    fn idle_frames_keep_the_beat() {
        let t0 = Instant::now();
        let mut p = Pacer::new(P);
        assert_eq!(p.next_frame(t0), t0);
        p.frame(t0, false);
        assert_eq!(p.next_frame(t0 + Duration::from_millis(1)), t0 + P);
        assert_eq!(p.next_frame(t0 + P * 3), t0 + P * 3);
    }

    #[test]
    fn an_echo_is_drawn_at_once_then_the_beat_resumes() {
        let t0 = Instant::now();
        let mut p = Pacer::new(P);
        p.frame(t0, false);
        let key = t0 + Duration::from_millis(1);
        p.input(key);
        let echo = key + Duration::from_millis(2);
        assert_eq!(p.next_frame(echo), echo, "no wait for the tick");
        // A frame without the echo (other panes changed) keeps hurrying.
        p.frame(echo, false);
        let later = echo + Duration::from_millis(1);
        assert_eq!(p.next_frame(later), later);
        p.frame(later, true);
        let after = later + Duration::from_millis(1);
        assert_eq!(p.next_frame(after), later + P);
    }

    #[test]
    fn a_swallowed_key_stops_hurrying() {
        let t0 = Instant::now();
        let mut p = Pacer::new(P);
        p.input(t0);
        let late = t0 + ECHO_WAIT;
        assert!(!p.urgent(late));
        p.frame(late, false);
        p.input(late + P);
        assert!(p.urgent(late + P), "a new key hurries again");
    }
}
