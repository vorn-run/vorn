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

/// Unix milliseconds of a `toISOString` time (`2026-10-14T09:30:00.123Z`).
pub fn iso_ms(t: &str) -> Option<f64> {
    let num = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    if t.len() < 20 || !t.ends_with('Z') {
        return None;
    }
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let ms = match t.get(19..20) {
        Some(".") => num(20..23)?,
        _ => 0,
    };
    // Days since the epoch of a proleptic Gregorian date.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((((days * 24 + hh) * 60 + mm) * 60 + ss) as f64 * 1000.0 + ms as f64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_ms_reads_iso_strings() {
        assert_eq!(super::iso_ms("1970-01-01T00:00:00.000Z"), Some(0.0));
        assert_eq!(
            super::iso_ms("2026-10-14T09:30:00.123Z"),
            Some(1_791_970_200_123.0)
        );
        assert_eq!(super::iso_ms("garbage"), None);
    }
}
