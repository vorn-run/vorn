//! `new Date().toISOString()`, without a date library for one format.

use std::time::{SystemTime, UNIX_EPOCH};

/// Now, as `new Date().toISOString()` writes it.
pub fn now_iso() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    iso(ms)
}

/// Milliseconds since the epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let of_day = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3_600_000,
        of_day / 60_000 % 60,
        of_day / 1000 % 60,
        of_day % 1000
    )
}

/// Days since 1970-01-01 as a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_as_javascript_does() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(951_782_400_123), "2000-02-29T00:00:00.123Z");
        assert_eq!(iso(1_790_000_000_999), "2026-09-21T14:13:20.999Z");
    }
}
