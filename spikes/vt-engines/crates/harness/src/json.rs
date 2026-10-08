//! A JSON object writer for result lines; the harness writes flat objects
//! only, so a serializer crate would be the bigger part of the binary.

use std::fmt::{self, Write};

#[derive(Default)]
pub struct Obj(String);

impl Obj {
    pub fn new() -> Obj {
        Obj::default()
    }

    fn key(mut self, k: &str) -> Obj {
        self.0.push(if self.0.is_empty() { '{' } else { ',' });
        push_str(&mut self.0, k);
        self.0.push(':');
        self
    }

    pub fn str(self, k: &str, v: &str) -> Obj {
        let mut o = self.key(k);
        push_str(&mut o.0, v);
        o
    }

    pub fn num(self, k: &str, v: f64) -> Obj {
        let mut o = self.key(k);
        if v.is_finite() {
            let _ = write!(o.0, "{}", (v * 1000.0).round() / 1000.0);
        } else {
            o.0.push_str("null");
        }
        o
    }

    pub fn bool(self, k: &str, v: bool) -> Obj {
        let mut o = self.key(k);
        o.0.push_str(if v { "true" } else { "false" });
        o
    }

    /// `v` must already be JSON.
    pub fn raw(self, k: &str, v: &str) -> Obj {
        let mut o = self.key(k);
        o.0.push_str(v);
        o
    }
}

impl fmt::Display for Obj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str("{}")
        } else {
            write!(f, "{}}}", self.0)
        }
    }
}

pub fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A JSON array of strings.
pub fn str_array<'a>(xs: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::from("[");
    for (i, x) in xs.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_str(&mut out, x);
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_escaped_flat_objects() {
        let o = Obj::new()
            .str("a", "x\"\x1b")
            .num("b", 1.23456)
            .bool("c", true)
            .raw("d", &str_array(["p", "q"]));
        assert_eq!(
            o.to_string(),
            r#"{"a":"x\"\u001b","b":1.235,"c":true,"d":["p","q"]}"#
        );
        assert_eq!(Obj::new().to_string(), "{}");
    }
}
