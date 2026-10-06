//! What every module needs to read rows and bind values the way the
//! TypeScript store it replaced did through libsql.

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::Row;
use serde_json::Value;

use crate::{Error, Result};

/// A JS number as SQLite stores it: an integer when it is one, else a real.
pub fn num(n: f64) -> SqlValue {
    if n.fract() == 0.0 && n.is_finite() && n.abs() <= 9_007_199_254_740_992.0 {
        SqlValue::Integer(n as i64)
    } else {
        SqlValue::Real(n)
    }
}

/// An optional JS number, `null` when absent.
pub fn opt_num(n: Option<f64>) -> SqlValue {
    n.map_or(SqlValue::Null, num)
}

/// An optional string, `null` when absent.
pub fn opt_text(s: Option<&str>) -> SqlValue {
    s.map_or(SqlValue::Null, |s| SqlValue::Text(s.to_owned()))
}

/// What JS `JSON.stringify(value)` writes.
pub fn json_text(value: &Value) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// `value ? JSON.stringify(value) : null`.
pub fn json_if_truthy(value: Option<&Value>) -> Result<SqlValue> {
    match value {
        Some(v) if truthy(v) => Ok(SqlValue::Text(json_text(v)?)),
        _ => Ok(SqlValue::Null),
    }
}

/// JavaScript truthiness of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// What `JSON.parse` of a column gives back. Throws as JSON.parse would.
pub fn parse_json(text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(Error::Json)
}

/// A column read as a JSON number, string or null, the way libsql hands a
/// column to JavaScript: integers and reals as numbers, text as strings.
pub fn column_value(row: &Row<'_>, name: &str) -> Result<Value> {
    Ok(match row.get_ref(name)? {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Array(b.iter().map(|byte| Value::from(*byte)).collect()),
    })
}

/// A number column (`order`, `pid`), as JavaScript reads it. A non-number
/// reads as NaN would, which no caller relies on; it is 0 here.
pub fn get_f64(row: &Row<'_>, name: &str) -> Result<f64> {
    Ok(match row.get_ref(name)? {
        ValueRef::Integer(i) => i as f64,
        ValueRef::Real(f) => f,
        ValueRef::Text(t) => std::str::from_utf8(t)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0),
        ValueRef::Null | ValueRef::Blob(_) => 0.0,
    })
}

/// A nullable number column.
pub fn get_opt_f64(row: &Row<'_>, name: &str) -> Result<Option<f64>> {
    if matches!(row.get_ref(name)?, ValueRef::Null) {
        return Ok(None);
    }
    get_f64(row, name).map(Some)
}

/// A text column. A number stored where text belongs reads as its digits.
pub fn get_text(row: &Row<'_>, name: &str) -> Result<String> {
    Ok(match row.get_ref(name)? {
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => format_js_number(f),
        ValueRef::Null => String::new(),
        ValueRef::Blob(b) => String::from_utf8_lossy(b).into_owned(),
    })
}

/// A nullable text column: `None` for SQL NULL.
pub fn get_opt_text(row: &Row<'_>, name: &str) -> Result<Option<String>> {
    if matches!(row.get_ref(name)?, ValueRef::Null) {
        return Ok(None);
    }
    get_text(row, name).map(Some)
}

/// How JavaScript prints a number with `String(n)`, for the cases a store
/// meets: integers without a fraction, anything else as Rust prints it.
pub fn format_js_number(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e21 {
        format!("{f:.0}")
    } else {
        f.to_string()
    }
}

/// `new Date().toISOString()`.
pub fn now_iso() -> String {
    iso_from_millis(now_millis())
}

/// `Date.now()`.
pub fn now_millis() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

/// `new Date(ms).toISOString()`: `2026-10-05T17:16:44.123Z`.
pub fn iso_from_millis(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let (h, rem) = (rem / 3_600_000, rem % 3_600_000);
    let (min, rem) = (rem / 60_000, rem % 60_000);
    let (s, milli) = (rem / 1000, rem % 1000);
    if (0..=9999).contains(&y) {
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{milli:03}Z")
    } else {
        let sign = if y < 0 { '-' } else { '+' };
        format!(
            "{sign}{:06}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{milli:03}Z",
            y.abs()
        )
    }
}

/// `Date.parse` of an ISO string as `toISOString` writes it (and the date-only
/// and offset forms around it). `None` where JavaScript gives NaN.
pub fn parse_iso_millis(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, time) = match text.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (text, None),
    };
    let mut parts = date.splitn(3, '-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next().map_or(Some(1), |p| p.parse().ok())?;
    let day: i64 = parts.next().map_or(Some(1), |p| p.parse().ok())?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut ms = days_from_civil(year, month, day) * 86_400_000;
    if let Some(time) = time {
        let (clock, offset_ms) = if let Some(clock) = time.strip_suffix('Z') {
            (clock, 0)
        } else if let Some(at) = time.rfind(['+', '-']) {
            let (clock, offset) = time.split_at(at);
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let offset = &offset[1..];
            let (oh, om) = offset.split_once(':').unwrap_or((offset, "0"));
            let minutes = oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?;
            (clock, sign * minutes * 60_000)
        } else {
            // No zone on a date-time is local time in JavaScript; every
            // timestamp the store is given carries one.
            (time, 0)
        };
        let mut fields = clock.splitn(3, ':');
        let h: i64 = fields.next()?.parse().ok()?;
        let m: i64 = fields.next()?.parse().ok()?;
        let (s, frac) = match fields.next() {
            Some(sec) => match sec.split_once('.') {
                Some((s, f)) => (s.parse::<i64>().ok()?, f),
                None => (sec.parse::<i64>().ok()?, ""),
            },
            None => (0, ""),
        };
        let millis = if frac.is_empty() {
            0
        } else {
            let digits: String = frac.chars().chain("000".chars()).take(3).collect();
            digits.parse::<i64>().ok()?
        };
        ms += h * 3_600_000 + m * 60_000 + s * 1000 + millis - offset_ms;
    }
    Some(ms)
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `crypto.randomUUID()`.
pub fn random_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_strings_round_trip_as_javascript_writes_them() {
        for ms in [
            0,
            1_728_148_604_123,
            951_782_400_000,
            -1,
            253_402_300_799_999,
        ] {
            let iso = iso_from_millis(ms);
            assert_eq!(parse_iso_millis(&iso), Some(ms), "{iso}");
        }
        assert_eq!(
            iso_from_millis(1_728_148_604_123),
            "2024-10-05T17:16:44.123Z"
        );
        assert_eq!(iso_from_millis(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn parses_the_forms_date_parse_accepts_from_the_server() {
        assert_eq!(parse_iso_millis("1970-01-01"), Some(0));
        assert_eq!(parse_iso_millis("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(parse_iso_millis("1970-01-01T00:00:01.5Z"), Some(1500));
        assert_eq!(parse_iso_millis("not a date"), None);
    }

    #[test]
    fn numbers_bind_as_sqlite_would_store_them() {
        assert_eq!(num(3.0), SqlValue::Integer(3));
        assert_eq!(num(1.5), SqlValue::Real(1.5));
        assert_eq!(format_js_number(42.0), "42");
    }

    #[test]
    fn truthiness_is_javascripts() {
        for (value, expected) in [
            (serde_json::json!(null), false),
            (serde_json::json!(""), false),
            (serde_json::json!(0), false),
            (serde_json::json!(false), false),
            (serde_json::json!([]), true),
            (serde_json::json!({}), true),
            (serde_json::json!("x"), true),
        ] {
            assert_eq!(truthy(&value), expected, "{value}");
        }
    }
}
