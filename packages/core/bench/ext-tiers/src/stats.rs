//! Percentiles of one run, and the median with its spread across runs.

use serde_json::{json, Value};

/// The nearest-rank percentile of `values`, 0 for none.
pub fn pct(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_take_the_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(pct(&v, 50.0), 50.0);
        assert_eq!(pct(&v, 99.0), 99.0);
        assert_eq!(pct(&[], 50.0), 0.0);
    }
}
