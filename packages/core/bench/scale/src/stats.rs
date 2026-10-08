//! Summaries of what was timed: latency percentiles and rates.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Percentiles of a set of timings, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Latency {
    pub n: usize,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl Latency {
    /// Summarises `samples`, which it sorts in place.
    pub fn of(samples: &mut [Duration]) -> Latency {
        samples.sort_unstable();
        Latency {
            n: samples.len(),
            p50_ms: ms(percentile(samples, 50.0)),
            p99_ms: ms(percentile(samples, 99.0)),
            max_ms: ms(samples.last().copied().unwrap_or_default()),
        }
    }
}

/// How many of something finished per second, and how long each took.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Rate {
    pub count: usize,
    pub secs: f64,
    pub per_sec: f64,
    pub each: Latency,
}

impl Rate {
    pub fn of(elapsed: Duration, samples: &mut [Duration]) -> Rate {
        let secs = elapsed.as_secs_f64();
        Rate {
            count: samples.len(),
            secs,
            per_sec: if secs > 0.0 {
                samples.len() as f64 / secs
            } else {
                0.0
            },
            each: Latency::of(samples),
        }
    }
}

/// Nearest-rank percentile of sorted samples: always one that was measured,
/// never an interpolation between two.
pub fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(range: std::ops::RangeInclusive<u64>) -> Vec<Duration> {
        range.map(Duration::from_millis).collect()
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let mut s = millis(1..=100);
        s.reverse();
        let l = Latency::of(&mut s);
        assert_eq!(
            (l.n, l.p50_ms, l.p99_ms, l.max_ms),
            (100, 50.0, 99.0, 100.0)
        );
        // With fewer samples than percent steps the tail is the slowest one.
        let l = Latency::of(&mut millis(1..=10));
        assert_eq!((l.p50_ms, l.p99_ms), (5.0, 10.0));
        let l = Latency::of(&mut millis(7..=7));
        assert_eq!((l.p50_ms, l.p99_ms), (7.0, 7.0));
    }

    #[test]
    fn nothing_measured_is_all_zero() {
        assert_eq!(Latency::of(&mut []), Latency::default());
        let r = Rate::of(Duration::ZERO, &mut []);
        assert_eq!(r.per_sec, 0.0);
    }

    #[test]
    fn a_rate_counts_per_second_of_wall_time() {
        let r = Rate::of(Duration::from_secs(2), &mut millis(1..=10));
        assert_eq!((r.count, r.per_sec), (10, 5.0));
    }
}
