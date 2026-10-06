//! JSON as JavaScript writes and reads it.
//!
//! The tools answer with text, and most of that text is `JSON.stringify` of
//! something. serde_json writes `1.0` where JavaScript writes `1`, and
//! `1e21` where JavaScript writes `1e+21`, so everything vornd sends is
//! written here instead. The rest of the module is the handful of JavaScript
//! coercions the tools lean on: what `${value}` prints, what `Number(value)`
//! reads, which values are truthy, and the order an object's keys come back in
//! after `JSON.parse`.

use serde_json::{Map, Number, Value};

/// `JSON.stringify(value)`.
pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write(value, None, 0, &mut out);
    out
}

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    let mut out = String::new();
    write(value, Some(2), 0, &mut out);
    out
}

fn write(value: &Value, indent: Option<usize>, depth: usize, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number_json(n)),
        Value::String(s) => quote(s, out),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(indent, depth + 1, out);
                write(item, indent, depth + 1, out);
            }
            newline(indent, depth, out);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(indent, depth + 1, out);
                quote(key, out);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write(item, indent, depth + 1, out);
            }
            newline(indent, depth, out);
            out.push('}');
        }
    }
}

fn newline(indent: Option<usize>, depth: usize, out: &mut String) {
    if let Some(width) = indent {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', width * depth));
    }
}

/// A string as `JSON.stringify` quotes it: the two-letter escapes it has,
/// `\u00xx` for the other control characters, everything else as it is.
pub fn quote(s: &str, out: &mut String) {
    out.reserve(s.len() + 2);
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

fn number_json(n: &Number) -> String {
    match n.as_f64() {
        Some(f) if f.is_finite() => number_to_string(f),
        _ => "null".to_owned(),
    }
}

/// `Number.prototype.toString()`: the shortest digits that read back as the
/// same double, laid out as ECMAScript's Number::toString lays them out.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x == 0.0 {
        return "0".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let mut out = String::new();
    if x < 0.0 {
        out.push('-');
    }
    // `{:e}` writes the shortest round-trip digits, as `d.ddde±x`.
    let scientific = format!("{:e}", x.abs());
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("LowerExp always writes an exponent");
    let exponent: i64 = exponent
        .parse()
        .expect("LowerExp writes a decimal exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i64;
    let n = exponent + 1;
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at(n as usize);
        out.push_str(int);
        out.push('.');
        out.push_str(frac);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(&digits);
    } else {
        let e = n - 1;
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        if !rest.is_empty() {
            out.push('.');
            out.push_str(rest);
        }
        out.push('e');
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&e.abs().to_string());
    }
    out
}

/// A JavaScript number as a JSON value: what `JSON.stringify` makes of it,
/// so `NaN` and the infinities become `null`.
pub fn num(x: f64) -> Value {
    // A whole number is kept as an integer, so any serializer writes it as
    // JavaScript would: `3`, never `3.0`.
    if x.fract() == 0.0 && x.abs() <= 9_007_199_254_740_991.0 {
        return Value::from(x as i64);
    }
    Number::from_f64(x).map_or(Value::Null, Value::Number)
}

/// An object's keys in the order `JSON.parse` leaves them: the ones that are
/// array indexes first, ascending, then the rest as they came. Applied to
/// anything a client sent, since the TypeScript only ever saw it parsed.
pub fn js_order(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(js_order).collect()),
        Value::Object(map) => {
            let mut indexed: Vec<(u32, String, Value)> = Vec::new();
            let mut named = Map::new();
            for (key, item) in map {
                match array_index(&key) {
                    Some(i) => indexed.push((i, key, js_order(item))),
                    None => {
                        named.insert(key, js_order(item));
                    }
                }
            }
            if indexed.is_empty() {
                return Value::Object(named);
            }
            indexed.sort_by_key(|(i, _, _)| *i);
            let mut out = Map::new();
            for (_, key, item) in indexed {
                out.insert(key, item);
            }
            out.extend(named);
            Value::Object(out)
        }
        other => other,
    }
}

/// The array index a property key names, if it names one.
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    if !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u32>().ok().filter(|i| *i < u32::MAX)
}

/// A string's `length`: UTF-16 code units, which is what zod's limits count.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(0, n)` in UTF-16 code units, never splitting a character.
pub fn utf16_prefix(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

/// Whether JavaScript treats a value as true. `None` is `undefined`.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// `${value}`: what a template literal prints for a value.
pub fn display(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::Null) => "null".to_owned(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.as_f64().map_or_else(String::new, number_to_string),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => join(items, ","),
        Some(Value::Object(_)) => "[object Object]".to_owned(),
    }
}

/// `items.join(separator)`: `null` and `undefined` print as nothing.
pub fn join(items: &[Value], separator: &str) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(separator);
        }
        if !item.is_null() {
            out.push_str(&display(Some(item)));
        }
    }
    out
}

/// `Number(value)`.
pub fn to_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => string_to_number(s),
        Some(array @ Value::Array(_)) => string_to_number(&display(Some(array))),
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// ECMAScript's StringToNumber: blank is 0, hex, octal and binary integer
/// literals, `Infinity`, and decimal literals; anything else is NaN.
pub fn string_to_number(s: &str) -> f64 {
    let s = trim(s);
    if s.is_empty() {
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
        if let Some(digits) = s.strip_prefix(prefix) {
            if digits.is_empty() {
                return f64::NAN;
            }
            let mut value = 0.0_f64;
            for c in digits.chars() {
                match c.to_digit(radix) {
                    Some(d) => value = value * f64::from(radix) + f64::from(d),
                    None => return f64::NAN,
                }
            }
            return value;
        }
    }
    let (sign, unsigned) = match s.as_bytes()[0] {
        b'+' => (1.0, &s[1..]),
        b'-' => (-1.0, &s[1..]),
        _ => (1.0, s),
    };
    if unsigned == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !is_decimal_literal(unsigned) {
        return f64::NAN;
    }
    let mut text = String::with_capacity(unsigned.len() + 2);
    if unsigned.starts_with('.') {
        text.push('0');
    }
    text.push_str(unsigned);
    if let Some(at) = text.find(['e', 'E']) {
        if text[..at].ends_with('.') {
            text.insert(at, '0');
        }
    } else if text.ends_with('.') {
        text.push('0');
    }
    text.parse::<f64>().map_or(f64::NAN, |v| sign * v)
}

/// `StrUnsignedDecimalLiteral`: digits with an optional fraction, or a
/// fraction alone, then an optional exponent.
fn is_decimal_literal(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    let int_digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    i += int_digits;
    let mut frac_digits = 0;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        frac_digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
        i += frac_digits;
    }
    if int_digits + frac_digits == 0 {
        return false;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
        if exp_digits == 0 {
            return false;
        }
        i += exp_digits;
    }
    i == bytes.len()
}

/// JavaScript's WhiteSpace and LineTerminator: what `trim()` and `\s` take.
pub fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `s.trim()`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_whitespace)
}

/// `a === b` for values that are not objects; objects are never the same
/// object here, so never equal.
pub fn strict_equals(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// `util.isDeepStrictEqual` on JSON: the same keys in any order, the same
/// items in the same order, the same primitives.
pub fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| deep_equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w)))
        }
        _ => strict_equals(Some(a), Some(b)),
    }
}

/// `value?.key`: the property of an object, or `undefined` for anything else.
pub fn field<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match value {
        Some(Value::Object(map)) => map.get(key),
        _ => None,
    }
}

/// `value.key`: as [`field`], but reading a property of `null` or
/// `undefined` throws, with V8's words for it.
pub fn prop<'a>(value: Option<&'a Value>, key: &str) -> Result<Option<&'a Value>, String> {
    match value {
        None => Err(format!(
            "Cannot read properties of undefined (reading '{key}')"
        )),
        Some(Value::Null) => Err(format!("Cannot read properties of null (reading '{key}')")),
        other => Ok(field(other, key)),
    }
}

/// The string a value holds, if it is one.
pub fn str_of(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn writes_numbers_as_javascript_does() {
        for (x, want) in [
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (1.5e-10, "1.5e-10"),
            (2.5e25, "2.5e+25"),
            (9007199254740991.0, "9007199254740991"),
            (-0.0, "0"),
        ] {
            assert_eq!(number_to_string(x), want, "{x}");
        }
        assert_eq!(stringify(&json!([1.0, 2.5, null])), "[1,2.5,null]");
        assert_eq!(stringify(&num(f64::NAN)), "null");
    }

    #[test]
    fn pretty_prints_as_json_stringify_with_two_spaces() {
        let value = json!({ "a": [], "b": {}, "c": [1, { "d": "e\u{1}\"" }] });
        assert_eq!(
            pretty(&value),
            "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    1,\n    {\n      \"d\": \"e\\u0001\\\"\"\n    }\n  ]\n}"
        );
        assert_eq!(pretty(&json!("x")), "\"x\"");
    }

    #[test]
    fn puts_array_index_keys_first_as_json_parse_does() {
        let parsed: Value =
            serde_json::from_str(r#"{"b":1,"10":2,"2":3,"01":4,"a":{"1":5,"z":6}}"#)
                .expect("valid JSON");
        let ordered = js_order(parsed);
        let keys: Vec<&String> = ordered.as_object().expect("object").keys().collect();
        assert_eq!(keys, ["2", "10", "b", "01", "a"]);
        assert_eq!(stringify(&ordered["a"]), r#"{"1":5,"z":6}"#);
        assert_eq!(array_index("4294967295"), None);
    }

    #[test]
    fn reads_strings_as_number_does() {
        for (s, want) in [
            ("", 0.0),
            ("  12 ", 12.0),
            ("0x10", 16.0),
            ("0b101", 5.0),
            ("1e3", 1000.0),
            ("1.", 1.0),
            (".5", 0.5),
            ("-2.5e-1", -0.25),
            ("+4", 4.0),
        ] {
            assert_eq!(string_to_number(s), want, "{s:?}");
        }
        for s in ["abc", "1_000", "-0x10", "1e", "e5", ".", "inf", "Infinityx"] {
            assert!(string_to_number(s).is_nan(), "{s:?}");
        }
        assert!(string_to_number("Infinity").is_infinite());
        assert!(string_to_number("1e999").is_infinite());
        assert_eq!(to_number(Some(&json!([5]))), 5.0);
        assert!(to_number(Some(&json!({}))).is_nan());
        assert_eq!(to_number(Some(&Value::Null)), 0.0);
    }

    #[test]
    fn prints_values_as_template_literals_do() {
        assert_eq!(display(None), "undefined");
        assert_eq!(display(Some(&json!([1, null, "a", [2, 3]]))), "1,,a,2,3");
        assert_eq!(display(Some(&json!({ "a": 1 }))), "[object Object]");
        assert_eq!(display(Some(&json!(2.0))), "2");
    }

    #[test]
    fn knows_truthiness_and_equality() {
        assert!(!truthy(Some(&json!(0))));
        assert!(!truthy(Some(&json!(""))));
        assert!(truthy(Some(&json!([]))));
        assert!(strict_equals(Some(&json!(1)), Some(&json!(1.0))));
        assert!(!strict_equals(Some(&json!({})), Some(&json!({}))));
        assert!(deep_equal(
            &json!({ "a": 1, "b": [1] }),
            &json!({ "b": [1.0], "a": 1 })
        ));
        assert!(!deep_equal(&json!({ "a": 1 }), &json!({ "a": 1, "b": 2 })));
    }

    #[test]
    fn counts_and_cuts_utf16() {
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(utf16_prefix("a😀b", 2), "a");
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
        assert_eq!(trim("\u{feff} x \u{a0}"), "x");
    }

    #[test]
    fn property_reads_fail_as_v8_words_them() {
        assert_eq!(
            prop(None, "url").unwrap_err(),
            "Cannot read properties of undefined (reading 'url')"
        );
        assert_eq!(prop(Some(&json!(5)), "url"), Ok(None));
    }
}
