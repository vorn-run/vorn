//! Timestamps as the server writes them: milliseconds since the epoch, and
//! ISO 8601 in UTC (`Date.prototype.toISOString`).
//!
//! Small enough to write out rather than take a date library for: the command
//! formats one kind of timestamp and reads back the ISO strings the server
//! stores.

use std::time::{SystemTime, UNIX_EPOCH};

/// `Date.now()`.
pub fn now_ms() -> f64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as f64,
        Err(before) => -(before.duration().as_millis() as f64),
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day since 1970-01-01 (`civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// `new Date(ms).toISOString()`, for the years a clock reads (0 to 9999).
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

/// Now, as `new Date().toISOString()`.
pub fn now_iso() -> String {
    iso(now_ms() as i64)
}

fn digits(s: &str, from: usize, len: usize) -> Option<i64> {
    let part = s.get(from..from + len)?;
    if !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// `Date.parse` for the ISO 8601 forms the server stores: a date, or a date
/// and time with optional seconds, fraction and zone. `None` is `NaN`.
///
/// A date-time with no zone is read as UTC here, where JavaScript reads it as
/// local time; the server never writes one.
pub fn parse_iso(s: &str) -> Option<f64> {
    let year = digits(s, 0, 4)?;
    if s.as_bytes().get(4) != Some(&b'-') || s.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let month = digits(s, 5, 2)?;
    let day = digits(s, 8, 2)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let date = days_from_civil(year, month, day) as f64 * 86_400_000.0;
    let rest = &s[10..];
    if rest.is_empty() {
        return Some(date);
    }
    let rest = rest.strip_prefix('T').or_else(|| rest.strip_prefix(' '))?;
    let hour = digits(rest, 0, 2)?;
    if rest.as_bytes().get(2) != Some(&b':') {
        return None;
    }
    let minute = digits(rest, 3, 2)?;
    let mut at = 5;
    let mut second = 0;
    let mut millis = 0.0;
    if rest.as_bytes().get(at) == Some(&b':') {
        second = digits(rest, at + 1, 2)?;
        at += 3;
        if rest.as_bytes().get(at) == Some(&b'.') {
            let frac_len = rest[at + 1..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            if frac_len == 0 {
                return None;
            }
            let frac = &rest[at + 1..at + 1 + frac_len.min(3)];
            millis = frac.parse::<f64>().ok()? * 10f64.powi(3 - frac.len() as i32);
            at += 1 + frac_len;
        }
    }
    if hour > 24 || minute > 59 || second > 59 {
        return None;
    }
    let offset_ms = match &rest[at..] {
        "" | "Z" | "z" => 0.0,
        zone => {
            let sign = match zone.as_bytes()[0] {
                b'+' => 1.0,
                b'-' => -1.0,
                _ => return None,
            };
            let zh = digits(zone, 1, 2)?;
            let zm = match zone.len() {
                3 => 0,
                6 if zone.as_bytes()[3] == b':' => digits(zone, 4, 2)?,
                5 => digits(zone, 3, 2)?,
                _ => return None,
            };
            sign * ((zh * 60 + zm) as f64) * 60_000.0
        }
    };
    let time = ((hour * 60 + minute) * 60 + second) as f64 * 1000.0 + millis;
    Some(date + time - offset_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_as_to_iso_string() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_790_000_000_123), "2026-09-21T14:13:20.123Z");
        assert_eq!(iso(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn reads_back_what_it_writes_and_what_the_server_stores() {
        for ms in [0, 1_790_000_000_123, 951_782_400_000, 4_102_444_799_999] {
            assert_eq!(parse_iso(&iso(ms)), Some(ms as f64));
        }
        assert_eq!(parse_iso("1970-01-02"), Some(86_400_000.0));
        assert_eq!(parse_iso("1970-01-01T01:00:00+01:00"), Some(0.0));
        assert_eq!(parse_iso("1970-01-01T00:00:00.5Z"), Some(500.0));
        assert_eq!(parse_iso("1970-01-01T00:01Z"), Some(60_000.0));
        assert_eq!(parse_iso("yesterday"), None);
        assert_eq!(parse_iso("1970-13-01"), None);
        assert_eq!(parse_iso(""), None);
    }
}
