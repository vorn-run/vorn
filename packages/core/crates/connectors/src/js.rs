//! JavaScript's string rules, where a manifest or a page counts on them.
//!
//! Manifests are written against the SDK, which measures text in UTF-16 code
//! units and trims JavaScript's whitespace; reading one the same way keeps a
//! title cut at the same character in every host.

use serde_json::Value;

/// `value` when it is a string, else `fallback`: the SDK's `str`.
pub fn str_or<'a>(value: Option<&'a Value>, fallback: &'a str) -> &'a str {
    value.and_then(Value::as_str).unwrap_or(fallback)
}

/// The string at `value`, or `""`.
pub fn str_of(value: Option<&Value>) -> &str {
    str_or(value, "")
}

/// Length in UTF-16 code units, as `String.prototype.length` counts.
pub fn len16(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The first `max` UTF-16 code units, as `s.slice(0, max)`; a surrogate pair
/// cut in half keeps neither half, which is the one place this differs.
pub fn slice16(s: &str, max: usize) -> &str {
    let mut units = 0;
    for (at, ch) in s.char_indices() {
        units += ch.len_utf16();
        if units > max {
            return &s[..at];
        }
    }
    s
}

/// The last `max` UTF-16 code units, as `s.slice(-max)`, under the same rule.
pub fn tail16(s: &str, max: usize) -> &str {
    let mut units = 0;
    for (at, ch) in s.char_indices().rev() {
        units += ch.len_utf16();
        if units > max {
            return &s[at + ch.len_utf8()..];
        }
    }
    s
}

/// What `String.prototype.trim` removes: Unicode white space and the BOM.
fn js_space(ch: char) -> bool {
    ch.is_whitespace() || ch == '\u{feff}'
}

/// `s.trim()`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

/// `Number(value)` for what a manifest holds; `None` stands for `NaN`.
pub fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Null => Some(0.0),
        Value::String(s) => {
            let s = trim(s);
            if s.is_empty() {
                return Some(0.0);
            }
            // Rust reads "inf" and "nan", which Number() does not.
            if s.bytes()
                .any(|b| b.is_ascii_alphabetic() && !matches!(b, b'e' | b'E'))
            {
                return match s {
                    "Infinity" | "+Infinity" => Some(f64::INFINITY),
                    "-Infinity" => Some(f64::NEG_INFINITY),
                    _ => None,
                };
            }
            s.parse().ok()
        }
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// A number as `JSON.stringify` writes it: whole numbers without a fraction.
pub fn json_number(n: f64) -> Value {
    // Exact for every integer an f64 holds below 2^53, which is all a manifest names.
    #[allow(clippy::cast_possible_truncation)]
    let whole = n as i64;
    #[allow(clippy::cast_precision_loss)]
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 && whole as f64 == n {
        return Value::from(whole);
    }
    serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
}

/// Serializes a present number as [`json_number`] writes it.
pub fn serialize_some_number<S: serde::Serializer>(
    n: &Option<f64>,
    s: S,
) -> Result<S::Ok, S::Error> {
    use serde::Serialize;
    n.map(json_number).serialize(s)
}

/// A JavaScript regular expression without flags, compiled once.
pub fn regex(pattern: &str) -> regress::Regex {
    regress::Regex::new(pattern).expect("a pattern written in this crate compiles")
}

/// Whether `re` matches somewhere in `text`, as `RegExp.prototype.test`.
pub fn test(re: &regress::Regex, text: &str) -> bool {
    re.find(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn slices_count_utf16_units() {
        assert_eq!(slice16("héllo", 2), "hé");
        assert_eq!(slice16("a😀b", 2), "a");
        assert_eq!(slice16("a😀b", 3), "a😀");
        assert_eq!(slice16("abc", 10), "abc");
        assert_eq!(tail16("a😀b", 2), "b");
        assert_eq!(tail16("a😀b", 3), "😀b");
        assert_eq!(len16("a😀"), 3);
    }

    #[test]
    fn trims_the_bom_too() {
        assert_eq!(trim("\u{feff} x \n"), "x");
    }

    #[test]
    fn reads_numbers_as_number_does() {
        assert_eq!(number(Some(&json!(7))), Some(7.0));
        assert_eq!(number(Some(&json!(" 10 "))), Some(10.0));
        assert_eq!(number(Some(&json!(""))), Some(0.0));
        assert_eq!(number(Some(&json!("1e1"))), Some(10.0));
        assert_eq!(number(Some(&json!("inf"))), None);
        assert_eq!(number(Some(&json!("Infinity"))), Some(f64::INFINITY));
        assert_eq!(number(Some(&json!(true))), Some(1.0));
        assert_eq!(number(Some(&json!(null))), Some(0.0));
        assert_eq!(number(Some(&json!([]))), None);
        assert_eq!(number(None), None);
    }

    #[test]
    fn writes_whole_numbers_without_a_fraction() {
        assert_eq!(json_number(10.0).to_string(), "10");
        assert_eq!(json_number(7.5).to_string(), "7.5");
    }
}

/// `String(value)`.
pub fn to_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            Some(f) => f.to_string(),
            None => n.to_string(),
        },
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

/// JavaScript truthiness.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `a.localeCompare(b)` for the names Vorn sorts: without case first, then as written.
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
}

/// JavaScript's `a < b` on strings: by UTF-16 code unit.
pub fn less(a: &str, b: &str) -> bool {
    a.encode_utf16().lt(b.encode_utf16())
}
