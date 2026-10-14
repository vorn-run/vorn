//! Hand-written files are TOML; the wire is JSON.
//!
//! [`Doc`] reads a TOML file into the JSON-shaped value the validators walk,
//! and keeps the line every key sits on, by the same dotted path the
//! validators report (`view.column[1].list.item.row[2].buton`). An error can
//! then name the file, the line and the field.

use std::collections::HashMap;

use serde_json::{Map, Number, Value};
use toml::de::{DeTable, DeValue};
use toml::Spanned;

/// A parsed TOML file and where each path sits in it.
#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub value: Value,
    lines: HashMap<String, usize>,
}

/// A TOML syntax error: the line and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: usize,
    pub message: String,
}

impl Doc {
    pub fn parse(text: &str) -> Result<Doc, SyntaxError> {
        let table = DeTable::parse(text).map_err(|e| SyntaxError {
            line: e.span().map_or(1, |s| line_of(text, s.start)),
            message: e.message().to_owned(),
        })?;
        let mut doc = Doc::default();
        let span = table.span();
        doc.lines.insert(String::new(), line_of(text, span.start));
        doc.value = doc.table(text, table.get_ref(), "");
        Ok(doc)
    }

    /// The line of `path`, or of the nearest enclosing path that has one.
    pub fn line(&self, path: &str) -> Option<usize> {
        let mut p = path;
        loop {
            if let Some(l) = self.lines.get(p) {
                return Some(*l);
            }
            let cut = p.rfind(['.', '['])?;
            p = &p[..cut];
        }
    }

    fn table(
        &mut self,
        text: &str,
        t: &toml::map::Map<Spanned<std::borrow::Cow<'_, str>>, Spanned<DeValue<'_>>>,
        path: &str,
    ) -> Value {
        let mut out = Map::new();
        for (k, v) in t {
            let p = if path.is_empty() {
                k.get_ref().to_string()
            } else {
                format!("{path}.{}", k.get_ref())
            };
            self.lines.insert(p.clone(), line_of(text, k.span().start));
            let value = self.value(text, v, &p);
            out.insert(k.get_ref().to_string(), value);
        }
        Value::Object(out)
    }

    fn value(&mut self, text: &str, v: &Spanned<DeValue<'_>>, path: &str) -> Value {
        match v.get_ref() {
            DeValue::String(s) => Value::String(s.to_string()),
            DeValue::Integer(i) => i
                .to_string()
                .parse::<i64>()
                .map(Value::from)
                .unwrap_or_else(|_| Value::String(i.to_string())),
            DeValue::Float(f) => f
                .to_string()
                .parse::<f64>()
                .ok()
                .and_then(Number::from_f64)
                .map_or(Value::Null, Value::Number),
            DeValue::Boolean(b) => Value::Bool(*b),
            DeValue::Datetime(d) => Value::String(d.to_string()),
            DeValue::Array(items) => Value::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let p = format!("{path}[{i}]");
                        self.lines
                            .insert(p.clone(), line_of(text, item.span().start));
                        self.value(text, item, &p)
                    })
                    .collect(),
            ),
            DeValue::Table(t) => self.table(text, t, path),
        }
    }
}

/// The 1-based line of a byte offset.
pub fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_line_of_every_path() {
        let text = "a = 1\n[view.list]\nempty = \"x\"\n\n[[view.list.item.row]]\nbuton = { label = \"Open\" }\n";
        let d = Doc::parse(text).unwrap();
        assert_eq!(
            d.value["view"]["list"]["item"]["row"][0]["buton"]["label"],
            "Open"
        );
        assert_eq!(d.line("view.list.empty"), Some(3));
        assert_eq!(d.line("view.list.item.row[0].buton"), Some(6));
        // A path with no key of its own falls back to the nearest one that has.
        assert_eq!(d.line("view.list.item.row[0].buton.action"), Some(6));
    }

    #[test]
    fn a_syntax_error_names_its_line() {
        let e = Doc::parse("a = 1\nb = = 2\n").unwrap_err();
        assert_eq!(e.line, 2);
    }
}
