//! The few JavaScript behaviours the command's output depends on.
//!
//! The TypeScript command prints what the server sent through
//! `JSON.stringify(value, null, 2)`, interpolates values into templates with
//! `String(x)`, and branches on truthiness. A script reading either command's
//! output must not be able to tell them apart, so those three are reproduced
//! here rather than approximated with serde's own printer: numbers print as
//! JavaScript prints them (`1e+21`, not `1e21`; `1`, not `1.0`) and objects put
//! their integer-like keys first, as a JavaScript object does.

use serde_json::{Map, Value};

/// `Number.prototype.toString()` for a finite or non-finite double.
pub fn number(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if x == 0.0 {
        return "0".into();
    }
    // Rust's `{:e}` prints the shortest digits that read back as `x`, which is
    // what JavaScript prints too; only the layout around them differs.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exponent) = sci.split_once('e').expect("{:e} always has an exponent");
    let exponent: i32 = exponent.parse().expect("{:e} prints an integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exponent + 1;

    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at(n as usize);
        format!("{int}.{frac}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{first}e{sign}{}", e.abs())
        } else {
            format!("{first}.{rest}e{sign}{}", e.abs())
        }
    };
    if x < 0.0 {
        format!("-{body}")
    } else {
        body
    }
}

/// A JSON number as JavaScript holds it: a double.
fn number_value(n: &serde_json::Number) -> f64 {
    n.as_f64().unwrap_or(f64::NAN)
}

/// How `JSON.stringify` writes a number: `null` when it is not finite.
fn json_number(n: &serde_json::Number) -> String {
    // Integers that a double holds exactly print as themselves; anything larger
    // is rounded to a double first, as `JSON.parse` would have.
    if let Some(i) = n.as_i64() {
        if i.unsigned_abs() <= (1u64 << 53) {
            return i.to_string();
        }
    } else if let Some(u) = n.as_u64() {
        if u <= (1u64 << 53) {
            return u.to_string();
        }
    }
    let x = number_value(n);
    if x.is_finite() {
        number(x)
    } else {
        "null".into()
    }
}

/// A string as `JSON.stringify` quotes it.
pub fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Whether a key is an array index, which a JavaScript object lists first and
/// in numeric order whatever order it was written in.
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    if !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u32>().ok().filter(|&i| i != u32::MAX)
}

/// An object's entries in the order a JavaScript object enumerates them.
fn js_order(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut indexed: Vec<(u32, (&String, &Value))> = Vec::new();
    let mut named = Vec::with_capacity(map.len());
    for entry in map {
        match array_index(entry.0) {
            Some(i) => indexed.push((i, entry)),
            None => named.push(entry),
        }
    }
    if indexed.is_empty() {
        return named;
    }
    indexed.sort_by_key(|(i, _)| *i);
    indexed.into_iter().map(|(_, e)| e).chain(named).collect()
}

fn write_pretty(value: &Value, indent: usize, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&json_number(n)),
        Value::String(s) => quote(s, out),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, indent + 1);
                write_pretty(item, indent + 1, out);
            }
            out.push('\n');
            push_indent(out, indent);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, (key, item)) in js_order(map).into_iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, indent + 1);
                quote(key, out);
                out.push_str(": ");
                write_pretty(item, indent + 1, out);
            }
            out.push('\n');
            push_indent(out, indent);
            out.push('}');
        }
    }
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

/// `JSON.stringify(value, null, 2)` plus the newline `asJson` adds: the one
/// place data becomes JSON, so every command emits the same shape.
pub fn as_json(value: &Value) -> String {
    let mut out = String::new();
    write_pretty(value, 0, &mut out);
    out.push('\n');
    out
}

/// `String(x)` for a value read from the server; a missing one is `undefined`.
pub fn string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => number(number_value(n)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// JavaScript truthiness; a missing value is `undefined`, which is falsy.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => {
            let x = number_value(n);
            x != 0.0 && !x.is_nan()
        }
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

/// `a ?? b` over a field: the field unless it is missing or null.
pub fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| !v.is_null())
}

/// A field of an object, `undefined` (`None`) on anything that is not one.
pub fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|o| o.get(key))
}

/// Length in UTF-16 code units, which is what `String.prototype.length` and
/// `padEnd` count.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.padEnd(width)`.
pub fn pad_end(s: &str, width: usize) -> String {
    let len = utf16_len(s);
    let mut out = String::with_capacity(s.len() + width.saturating_sub(len));
    out.push_str(s);
    for _ in len..width {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prints_numbers_as_javascript_does() {
        for (x, js) in [
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789.125, "123456789.125"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (-0.0, "0"),
            (5e300, "5e+300"),
            (2.5e25, "2.5e+25"),
        ] {
            assert_eq!(number(x), js, "{x}");
        }
    }

    #[test]
    fn stringifies_as_json_stringify_does() {
        let value: Value = serde_json::from_str(
            r#"{"b":1.0,"2":"x","1":[],"a":{},"s":"q\"\u001b\n","n":null,"big":12345678901234567890,"arr":[1,{"k":true}]}"#,
        )
        .unwrap();
        assert_eq!(
            as_json(&value),
            "{\n  \"1\": [],\n  \"2\": \"x\",\n  \"b\": 1,\n  \"a\": {},\n  \"s\": \"q\\\"\\u001b\\n\",\n  \"n\": null,\n  \"big\": 12345678901234567000,\n  \"arr\": [\n    1,\n    {\n      \"k\": true\n    }\n  ]\n}\n"
        );
        assert_eq!(as_json(&json!([])), "[]\n");
    }

    #[test]
    fn coerces_and_tests_as_javascript_does() {
        assert_eq!(string(None), "undefined");
        assert_eq!(string(Some(&json!([1, null, "a"]))), "1,,a");
        assert!(!truthy(Some(&json!(""))));
        assert!(!truthy(Some(&json!(0))));
        assert!(truthy(Some(&json!([]))));
        assert_eq!(pad_end("é", 3), "é  ");
        assert_eq!(utf16_len("😀"), 2);
    }
}
