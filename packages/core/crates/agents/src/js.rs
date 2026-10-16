//! Text and number rules the agents' answers share: what counts as
//! whitespace, how a string reads as a number or a date, how a string is cut
//! by UTF-16 length, and how a number is written as JSON.

use std::cmp::Ordering;

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::{Offset, TimeZone};
use serde_json::Value;

/// Whitespace: ASCII and Unicode spaces, line terminators and the byte order mark.
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// Trims [`is_space`] from both ends: unlike `str::trim`, this strips a byte order mark.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// The first `n` UTF-16 code units of `s`. A surrogate pair the cut would
/// split is left out whole.
pub fn slice_utf16(s: &str, n: usize) -> String {
    let mut units = 0;
    for (at, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return s[..at].to_owned();
        }
    }
    s.to_owned()
}

/// A number as JSON: an integer without a fraction, `null` for NaN and the
/// infinities.
pub fn number(n: f64) -> Value {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        // Exact: an integral f64 below 2^53 is an i64.
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

/// `text` as a number: decimal (with `Infinity`) and `0x`/`0o`/`0b` literals,
/// `0` for blank text, NaN for anything else.
pub fn to_number(text: &str) -> f64 {
    let t = trim(text);
    if t.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = t.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix).map_or(f64::NAN, |n| n as f64);
        }
    }
    let unsigned = t.strip_prefix(['+', '-']).unwrap_or(t);
    if unsigned == "Infinity" {
        return if t.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    // Rust's parser also reads `inf` and `NaN`; those are NaN here.
    let decimal = unsigned
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        && unsigned
            .bytes()
            .next()
            .is_some_and(|b| b != b'+' && b != b'-');
    if !decimal {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

/// Orders timestamps newest first, keeping ties in their order. A NaN
/// timestamp sorts last.
pub fn newest_first(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => b.partial_cmp(&a).unwrap_or(Ordering::Equal),
    }
}

/// Milliseconds since the epoch for the forms an agent records a time in:
/// ISO 8601 dates (`2025-01-02`, read as UTC) and date-times with a `T` or a
/// space, with or without fractional seconds and an offset (read as local
/// time without one). NaN for anything else, including looser forms such as
/// `2025-1-2`.
pub fn date_ms(text: &str) -> f64 {
    parse_date(text).unwrap_or(f64::NAN)
}

fn parse_date(text: &str) -> Option<f64> {
    let b = text.as_bytes();
    let mut at = 0;
    let digits = |at: &mut usize, n: usize| -> Option<i64> {
        let part = b.get(*at..*at + n)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        *at += n;
        std::str::from_utf8(part).ok()?.parse().ok()
    };
    let year = digits(&mut at, 4)?;
    let mut month = 1;
    let mut day = 1;
    if b.get(at) == Some(&b'-') {
        at += 1;
        month = digits(&mut at, 2)?;
        if b.get(at) == Some(&b'-') {
            at += 1;
            day = digits(&mut at, 2)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // A day past the month's end runs on into the next.
    let date = Date::new(i16::try_from(year).ok()?, i8::try_from(month).ok()?, 1)
        .ok()?
        .checked_add(jiff::Span::new().days(day - 1))
        .ok()?;
    if at == b.len() {
        return Some(utc_ms(date.to_datetime(Time::midnight())));
    }
    if !matches!(b[at], b'T' | b't' | b' ') {
        return None;
    }
    at += 1;
    let hour = digits(&mut at, 2)?;
    if b.get(at) != Some(&b':') {
        return None;
    }
    at += 1;
    let minute = digits(&mut at, 2)?;
    let mut second = 0;
    // At most 999, so its nanoseconds fit an i32.
    let mut millis: i32 = 0;
    if b.get(at) == Some(&b':') {
        at += 1;
        second = digits(&mut at, 2)?;
        if b.get(at) == Some(&b'.') {
            at += 1;
            let start = at;
            while b.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            if at == start {
                return None;
            }
            // Milliseconds: the first three digits, the rest dropped.
            let frac = &text[start..at.min(start + 3)];
            millis = format!("{frac:0<3}").parse().ok()?;
        }
    }
    let midnight_after = hour == 24 && minute == 0 && second == 0 && millis == 0;
    if !(hour < 24 || midnight_after) || minute > 59 || second > 59 {
        return None;
    }
    let time = Time::new(
        i8::try_from(hour % 24).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        millis * 1_000_000,
    )
    .ok()?;
    let mut civil = date.to_datetime(time);
    if midnight_after {
        civil = civil.checked_add(jiff::Span::new().days(1)).ok()?;
    }
    let offset_secs = match b.get(at) {
        None => {
            let zoned = TimeZone::system()
                .to_ambiguous_zoned(civil)
                .compatible()
                .ok()?;
            return Some(zoned.timestamp().as_millisecond() as f64);
        }
        Some(b'Z' | b'z') if at + 1 == b.len() => 0,
        Some(&sign @ (b'+' | b'-')) => {
            at += 1;
            let h = digits(&mut at, 2)?;
            if b.get(at) == Some(&b':') {
                at += 1;
            }
            let m = digits(&mut at, 2)?;
            if at != b.len() || h > 23 || m > 59 {
                return None;
            }
            let secs = h * 3600 + m * 60;
            if sign == b'-' {
                -secs
            } else {
                secs
            }
        }
        _ => return None,
    };
    let offset = Offset::from_seconds(i32::try_from(offset_secs).ok()?).ok()?;
    let ts = offset.to_timestamp(civil).ok()?;
    Some(ts.as_millisecond() as f64)
}

fn utc_ms(civil: DateTime) -> f64 {
    Offset::UTC
        .to_timestamp(civil)
        .map_or(f64::NAN, |ts| ts.as_millisecond() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_unicode_whitespace_and_the_bom() {
        assert_eq!(trim("\u{FEFF} a b \u{3000}\n"), "a b");
        assert_eq!(trim("\u{85}x"), "\u{85}x");
    }

    #[test]
    fn slices_by_utf16_units_without_splitting_a_pair() {
        assert_eq!(slice_utf16("héllo", 2), "hé");
        assert_eq!(slice_utf16("a😀b", 2), "a");
        assert_eq!(slice_utf16("a😀b", 3), "a😀");
        assert_eq!(slice_utf16("short", 80), "short");
    }

    #[test]
    fn writes_whole_numbers_as_integers() {
        assert_eq!(number(1_735_787_045_000.0).to_string(), "1735787045000");
        assert_eq!(number(1.5).to_string(), "1.5");
        assert_eq!(number(f64::NAN), Value::Null);
        assert_eq!(number(f64::INFINITY), Value::Null);
    }

    #[test]
    fn reads_decimal_and_prefixed_numbers() {
        assert_eq!(to_number(" 12 "), 12.0);
        assert_eq!(to_number(""), 0.0);
        assert_eq!(to_number("1.5e3"), 1500.0);
        assert_eq!(to_number("0x10"), 16.0);
        assert_eq!(to_number("-Infinity"), f64::NEG_INFINITY);
        assert!(to_number("inf").is_nan());
        assert!(to_number("NaN").is_nan());
        assert!(to_number("12px").is_nan());
        assert!(to_number("--1").is_nan());
    }

    #[test]
    fn reads_iso_dates() {
        // Each value is the expected milliseconds since the epoch.
        let utc = [
            ("2025-01-02T03:04:05.678Z", 1_735_787_045_678.0),
            ("2025-01-02T03:04:05Z", 1_735_787_045_000.0),
            ("2025-01-02T03:04Z", 1_735_787_040_000.0),
            ("2025-01-02", 1_735_776_000_000.0),
            ("2025-01", 1_735_689_600_000.0),
            ("2025", 1_735_689_600_000.0),
            ("2023-02-31", 1_677_801_600_000.0),
            ("2025-01-02T03:04:05.1234567Z", 1_735_787_045_123.0),
            ("2025-01-02T03:04:05.1Z", 1_735_787_045_100.0),
            ("2025-01-02T03:04:05+02:00", 1_735_779_845_000.0),
            ("2025-01-02T03:04:05+0200", 1_735_779_845_000.0),
            ("2025-01-02T24:00:00Z", 1_735_862_400_000.0),
            ("2025-01-02t03:04:05.678z", 1_735_787_045_678.0),
            ("2025-01-02 03:04:05Z", 1_735_787_045_000.0),
        ];
        for (text, want) in utc {
            assert_eq!(date_ms(text), want, "{text}");
        }
        for text in [
            "2025-13-01",
            "1700000000000",
            " 2025-01-02T03:04:05Z",
            "2025-01-02T03:04:05.Z",
            "2025-01-02T3:04:05Z",
            "2025-01-02T25:00:00Z",
            "",
        ] {
            assert!(date_ms(text).is_nan(), "{text}");
        }
        // Without an offset it is local time, whatever zone this runs in.
        let local = date_ms("2025-01-02T03:04:05");
        assert!((local - 1_735_787_045_000.0).abs() <= 14.0 * 3_600_000.0);
        assert_eq!(local, date_ms("2025-01-02 03:04:05"));
    }

    #[test]
    fn sorts_newest_first_and_nan_last() {
        let mut v = [1.0, f64::NAN, 3.0, 2.0];
        v.sort_by(|a, b| newest_first(*a, *b));
        assert_eq!(&v[..3], &[3.0, 2.0, 1.0]);
        assert!(v[3].is_nan());
    }
}
