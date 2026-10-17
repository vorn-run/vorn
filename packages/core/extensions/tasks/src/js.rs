//! The JavaScript readings of values the board has always made: `String(v)`,
//! truthiness, whole numbers as integers, UTF-16 lengths and ISO times.

use serde_json::Value;

/// `String(value)`: what a template literal prints.
pub fn to_string(value: &Value) -> String {
    vorn_mcp::json::display(Some(value))
}

/// Whether JavaScript treats `value` as true.
pub fn truthy(value: &Value) -> bool {
    vorn_mcp::json::truthy(Some(value))
}

/// A JSON number for `n`, a whole number as an integer.
pub fn num(n: f64) -> Value {
    vorn_mcp::json::num(n)
}

/// A string's length in UTF-16 units.
pub fn utf16_len(text: &str) -> usize {
    vorn_mcp::json::utf16_len(text)
}

/// The first `units` UTF-16 units of `text`, never splitting a character.
pub fn head(text: &str, units: usize) -> &str {
    vorn_mcp::json::utf16_prefix(text, units)
}

/// `ms` since the Unix epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ`, as `Date#toISOString` writes it.
pub fn iso(ms: i64) -> String {
    let (days, in_day) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Howard Hinnant's days-to-civil, for the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m, s, milli) = (
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000,
    );
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{milli:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_iso_strings() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso(1_767_225_599_999), "2025-12-31T23:59:59.999Z");
        assert_eq!(iso(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn values_read_as_javascript_reads_them() {
        assert_eq!(to_string(&serde_json::json!(3.0)), "3");
        assert_eq!(to_string(&serde_json::json!(null)), "null");
        assert!(!truthy(&serde_json::json!("")));
        assert_eq!(head("a😀b", 2), "a");
        assert_eq!(num(2.0), serde_json::json!(2));
    }
}
