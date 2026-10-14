//! Percentiles of one run, and the median with its spread across runs.

use serde_json::{json, Value};

/// The nearest-rank percentile of `values`, 0 for none.
pub fn pct(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[rank.clamp(1, v.len()) - 1]
}

pub fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// p50, p99 and max of one run's samples, in the unit given.
pub fn summary(values: &[f64]) -> Value {
    json!({
        "n": values.len(),
        "p50": round(pct(values, 50.0)),
        "p99": round(pct(values, 99.0)),
        "max": round(pct(values, 100.0)),
    })
}

/// The median of several runs' values with their lowest and highest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spread {
    pub median: f64,
    pub low: f64,
    pub high: f64,
}

impl Spread {
    pub fn of(values: &[f64]) -> Option<Spread> {
        if values.is_empty() {
            return None;
        }
        let mut v = values.to_vec();
        v.sort_by(f64::total_cmp);
        let mid = v.len() / 2;
        let median = if v.len().is_multiple_of(2) {
            (v[mid - 1] + v[mid]) / 2.0
        } else {
            v[mid]
        };
        Some(Spread {
            median,
            low: v[0],
            high: v[v.len() - 1],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_take_the_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(pct(&v, 50.0), 50.0);
        assert_eq!(pct(&v, 99.0), 99.0);
        assert_eq!(pct(&v, 100.0), 100.0);
        assert_eq!(pct(&[], 50.0), 0.0);
        assert_eq!(pct(&[3.0], 99.0), 3.0);
    }

    #[test]
    fn a_spread_is_the_median_and_its_ends() {
        let s = Spread::of(&[5.0, 1.0, 3.0, 9.0]).unwrap();
        assert_eq!((s.median, s.low, s.high), (4.0, 1.0, 9.0));
        assert_eq!(Spread::of(&[2.0, 7.0, 4.0]).unwrap().median, 4.0);
        assert!(Spread::of(&[]).is_none());
    }
}
