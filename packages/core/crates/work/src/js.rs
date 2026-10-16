//! The string and value rules the workflow engine's answers depend on:
//! lengths and cuts counted in UTF-16 units, a value as text, and JSON text.
//!
//! A cut inside a surrogate pair is moved back to the character's start,
//! since a Rust string cannot hold half a character.

use serde_json::Value;

use crate::js_numbers;

/// The length of `text` in UTF-16 units.
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The byte offset of UTF-16 unit `units`, moved back to a character start.
fn byte_at_unit(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (at, c) in text.char_indices() {
        if seen + c.len_utf16() > units {
            return at;
        }
        seen += c.len_utf16();
    }
    text.len()
}

/// The first `units` UTF-16 units of `text`.
pub fn head(text: &str, units: usize) -> &str {
    &text[..byte_at_unit(text, units)]
}

/// The last `units` UTF-16 units of a text longer than `units`; the whole
/// text otherwise.
pub fn tail(text: &str, units: usize) -> &str {
    let len = utf16_len(text);
    if len <= units {
        return text;
    }
    let from = byte_at_unit(text, len - units);
    // A cut inside a pair keeps the whole character rather than half of it.
    &text[from..]
}

/// `ms` as an ISO 8601 UTC time with milliseconds.
pub fn iso(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_default()
}

/// Now as an ISO 8601 UTC time with milliseconds.
pub fn iso_now() -> String {
    iso(jiff::Timestamp::now().as_millisecond())
}

/// `value` as compact JSON, whole numbers without a fraction.
pub fn stringify(value: &Value) -> String {
    serde_json::to_string(&js_numbers(value.clone())).unwrap_or_default()
}

/// `value` as JSON indented by two spaces.
pub fn stringify_pretty(value: &Value) -> String {
    serde_json::to_string_pretty(&js_numbers(value.clone())).unwrap_or_default()
}

/// `value` as text: strings bare, lists joined by commas with nulls empty,
/// objects as `[object Object]`.
pub fn to_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(_) => number_text(value),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => to_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// A number as text: whole numbers without a fraction.
fn number_text(value: &Value) -> String {
    match js_numbers(value.clone()) {
        Value::Number(n) => {
            if let Some(f) = n.as_f64().filter(|_| !n.is_i64() && !n.is_u64()) {
                if f.is_nan() {
                    return "NaN".to_owned();
                }
                if f.is_infinite() {
                    return if f > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
                }
            }
            n.to_string()
        }
        other => to_string(&other),
    }
}

/// The number in the strings a model writes where a number belongs:
/// decimal, exponent, hexadecimal, octal and binary, with blanks around.
/// `None` for anything else.
pub fn number(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    let radix = |prefix: [&str; 2], radix: u32| {
        prefix
            .iter()
            .find_map(|p| t.strip_prefix(p))
            .map(|digits| u64::from_str_radix(digits, radix).ok().map(|n| n as f64))
    };
    if let Some(n) = radix(["0x", "0X"], 16)
        .or_else(|| radix(["0o", "0O"], 8))
        .or_else(|| radix(["0b", "0B"], 2))
    {
        return n;
    }
    match t {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    // Rust's parser also reads `inf` and `nan`, which are not numbers here.
    if !t
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return None;
    }
    t.parse::<f64>().ok()
}

/// A JSON number for `n`, whole numbers as integers.
pub fn number_value(n: f64) -> Value {
    serde_json::Number::from_f64(n).map_or(Value::Null, |n| js_numbers(Value::Number(n)))
}

/// Whether `value` is truthy: not null, false, zero, NaN or empty text.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lengths_and_cuts_count_utf16_units() {
        assert_eq!(utf16_len("a😀b"), 4);
        assert_eq!(head("a😀b", 3), "a😀");
        // Half a pair is left out rather than split.
        assert_eq!(head("a😀b", 2), "a");
        assert_eq!(tail("a😀b", 2), "😀b");
        assert_eq!(tail("abc", 5), "abc");
        assert_eq!(tail("abcdef", 2), "ef");
        assert_eq!(tail("😀😀", 3), "😀😀");
    }

    #[test]
    fn values_as_text_and_json() {
        assert_eq!(to_string(&json!(3.0)), "3");
        assert_eq!(to_string(&json!(2.5)), "2.5");
        assert_eq!(to_string(&json!(["a", 1, null, true])), "a,1,,true");
        assert_eq!(to_string(&json!({ "a": 1 })), "[object Object]");
        assert_eq!(to_string(&json!(null)), "null");
        assert_eq!(
            stringify(&json!({ "n": 4.0, "s": "x" })),
            r#"{"n":4,"s":"x"}"#
        );
        assert_eq!(
            stringify_pretty(&json!({ "a": [1] })),
            "{\n  \"a\": [\n    1\n  ]\n}"
        );
    }

    #[test]
    fn prints_times_as_iso_8601() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(iso_now().len(), 24);
    }

    #[test]
    fn reads_number_strings() {
        assert_eq!(number(" 42 "), Some(42.0));
        assert_eq!(number("1e3"), Some(1000.0));
        assert_eq!(number("0x10"), Some(16.0));
        assert_eq!(number(""), Some(0.0));
        assert_eq!(number("12px"), None);
        assert_eq!(number("inf"), None);
        assert_eq!(number("-Infinity"), Some(f64::NEG_INFINITY));
        assert_eq!(number_value(7.0), json!(7));
        assert!(number_value(7.0).is_i64());
    }
}
