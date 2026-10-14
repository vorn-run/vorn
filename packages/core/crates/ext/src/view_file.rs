//! Template files: KDL a person or an agent writes, read into the value
//! [`template`] checks, with the line and column of every node and entry.
//!
//! ```kdl
//! schema "vorn:view@0.1"
//! rows {
//!     name string
//!     state enum pass fail pending
//!     link url
//! }
//! list empty="No pull request" {
//!     row {
//!         status "{row.state}" pass=good fail=bad pending=busy
//!         text "{row.name}" grow=#true
//!         button Open open="{row.link}"
//!     }
//! }
//! ```
//!
//! A node's first argument is its value (`text`, `status`, `badge`) or label
//! (`button`); its properties are its attributes, a status's tones, or a
//! button's action; `row` and `column` hold their children, `list` its item.
//! The reading is mechanical: anything unexpected is passed through, so the
//! checker reports it by path and the path finds its place in the file.

use std::collections::HashMap;

use kdl::{KdlDocument, KdlNode, KdlValue};
use serde_json::{Map, Value};

use crate::template::{self, Template, TemplateError};

/// Parses and checks a template file.
pub fn parse(text: &str) -> Result<Template, Vec<TemplateError>> {
    let doc: KdlDocument = text.parse().map_err(|e: kdl::KdlError| {
        e.diagnostics
            .iter()
            .map(|d| TemplateError {
                at: Some(line_col(text, d.span.offset())),
                path: "$".into(),
                message: [d.message.as_deref(), d.help.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("; "),
            })
            .collect::<Vec<_>>()
    })?;
    let mut r = Reader {
        text,
        spans: HashMap::new(),
        extra: Vec::new(),
    };
    let value = r.document(&doc);
    let result = template::parse_value(&value);
    let mut errors = match result {
        Ok(t) if r.extra.is_empty() => return Ok(t),
        Ok(_) => Vec::new(),
        Err(e) => e,
    };
    errors.append(&mut r.extra);
    for e in &mut errors {
        if e.at.is_none() {
            e.at = r.find(&e.path);
        }
    }
    Err(errors)
}

/// Parses and checks a template file into the JSON clients receive.
pub fn to_json(text: &str) -> Result<Value, Vec<TemplateError>> {
    parse(text)?;
    let doc: KdlDocument = text.parse().map_err(|_| Vec::new())?;
    let mut r = Reader {
        text,
        spans: HashMap::new(),
        extra: Vec::new(),
    };
    Ok(r.document(&doc))
}

struct Reader<'t> {
    text: &'t str,
    /// Byte offset of each path, as the checker names paths.
    spans: HashMap<String, usize>,
    /// Errors only the file can have (two views, a node where a field goes).
    extra: Vec<TemplateError>,
}

impl Reader<'_> {
    fn mark(&mut self, path: &str, offset: usize) {
        self.spans.entry(path.to_owned()).or_insert(offset);
    }

    /// The place of `path`, or of the nearest enclosing path that has one.
    fn find(&self, path: &str) -> Option<(usize, usize)> {
        let mut p = path;
        loop {
            if let Some(o) = self.spans.get(p) {
                return Some(line_col(self.text, *o));
            }
            let cut = p.rfind(['.', '['])?;
            p = &p[..cut];
        }
    }

    fn document(&mut self, doc: &KdlDocument) -> Value {
        let mut root = Map::new();
        for node in doc.nodes() {
            let name = node.name().value();
            match name {
                "schema" => {
                    self.mark("schema", node.span().offset());
                    root.insert("schema".into(), first_arg(node).unwrap_or(Value::Null));
                }
                "rows" => {
                    self.mark("rows", node.span().offset());
                    root.insert("rows".into(), self.rows(node));
                }
                _ if root.contains_key("view") => self.extra.push(TemplateError {
                    at: Some(line_col(self.text, node.span().offset())),
                    path: "view".into(),
                    message: format!("a template has one view; `{name}` is a second"),
                }),
                _ => {
                    let v = self.node(node, "view");
                    root.insert("view".into(), v);
                }
            }
        }
        Value::Object(root)
    }

    /// `name string` is a field of that type; `state enum a b c` an enum.
    fn rows(&mut self, node: &KdlNode) -> Value {
        let mut fields = Map::new();
        for f in node.children().map(KdlDocument::nodes).unwrap_or_default() {
            let name = f.name().value().to_owned();
            self.mark(&format!("rows.{name}"), f.span().offset());
            let args: Vec<Value> = f
                .entries()
                .iter()
                .filter(|e| e.name().is_none())
                .map(|e| value(e.value()))
                .collect();
            let ty = match args.split_first() {
                Some((Value::String(k), rest)) if k == "enum" => Value::Array(rest.to_vec()),
                Some((t, [])) => t.clone(),
                _ => Value::Null,
            };
            fields.insert(name, ty);
        }
        Value::Object(fields)
    }

    fn node(&mut self, node: &KdlNode, parent: &str) -> Value {
        let kind = node.name().value();
        let path = format!("{parent}.{kind}");
        self.mark(&path, node.name().span().offset());
        let kids = node.children().map(KdlDocument::nodes).unwrap_or_default();
        let body = match kind {
            "column" | "row" => Value::Array(
                kids.iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let p = format!("{path}[{i}]");
                        self.mark(&p, c.span().offset());
                        self.node(c, &p)
                    })
                    .collect(),
            ),
            _ => {
                let mut o = Map::new();
                let first = match kind {
                    "button" => "label",
                    _ => "value",
                };
                let mut tones = Map::new();
                let mut action = Map::new();
                for (i, e) in node
                    .entries()
                    .iter()
                    .filter(|e| e.name().is_none())
                    .enumerate()
                {
                    let key = if i == 0 {
                        first.to_owned()
                    } else {
                        format!("argument {}", i + 1)
                    };
                    self.mark(&format!("{path}.{key}"), e.span().offset());
                    o.insert(key, value(e.value()));
                }
                for e in node.entries() {
                    let Some(name) = e.name() else { continue };
                    let k = name.value();
                    let (key, into) = match (kind, k) {
                        ("status" | "badge", _) => (format!("{path}.tones.{k}"), &mut tones),
                        ("button", "open" | "call") => (format!("{path}.action.{k}"), &mut action),
                        _ => (format!("{path}.{k}"), &mut o),
                    };
                    self.mark(&key, name.span().offset());
                    into.insert(k.to_owned(), value(e.value()));
                }
                if !tones.is_empty() {
                    self.mark(&format!("{path}.tones"), node.span().offset());
                    o.insert("tones".into(), Value::Object(tones));
                }
                if !action.is_empty() {
                    o.insert("action".into(), Value::Object(action));
                }
                if kind == "list" {
                    match kids {
                        [item] => {
                            let v = self.node(item, &format!("{path}.item"));
                            o.insert("item".into(), v);
                        }
                        [] => {}
                        [_, second, ..] => self.extra.push(TemplateError {
                            at: Some(line_col(self.text, second.span().offset())),
                            path: format!("{path}.item"),
                            message: "a list draws one item per row; wrap these nodes in a row or a column".into(),
                        }),
                    }
                } else if let Some(c) = kids.first() {
                    self.extra.push(TemplateError {
                        at: Some(line_col(self.text, c.span().offset())),
                        path: path.clone(),
                        message: format!(
                            "`{kind}` holds no children; only row, column and list do"
                        ),
                    });
                }
                Value::Object(o)
            }
        };
        let mut wrapped = Map::new();
        wrapped.insert(kind.to_owned(), body);
        Value::Object(wrapped)
    }
}

fn first_arg(node: &KdlNode) -> Option<Value> {
    node.entries()
        .iter()
        .find(|e| e.name().is_none())
        .map(|e| value(e.value()))
}

fn value(v: &KdlValue) -> Value {
    match v {
        KdlValue::String(s) => Value::String(s.clone()),
        KdlValue::Integer(i) => {
            i64::try_from(*i).map_or_else(|_| Value::String(i.to_string()), Value::from)
        }
        KdlValue::Float(f) => serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number),
        KdlValue::Bool(b) => Value::Bool(*b),
        KdlValue::Null => Value::Null,
    }
}

/// 1-based line and column (in chars) of a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::Node;

    const CHECKS: &str = r#"schema "vorn:view@0.1"

// Every row also has an id.
rows {
    name string
    state enum pass fail pending
    link url
}

column {
    text "{rows.state.fail} failing" role=secondary
    list empty="No pull request" {
        row {
            status "{row.state}" pass=good fail=bad pending=busy
            text "{row.name}" grow=#true
            button Open open="{row.link}"
        }
    }
}
"#;

    fn errors(text: &str) -> Vec<String> {
        parse(text)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn reads_a_template_file() {
        let t = parse(CHECKS).unwrap();
        let Node::Column(kids) = &t.view else {
            panic!("a column")
        };
        assert!(matches!(&kids[1], Node::List { empty: Some(_), .. }));
        assert_eq!(
            to_json(CHECKS).unwrap()["view"]["column"][1]["list"]["item"]["row"][2]["button"]
                ["action"]["open"],
            "{row.link}"
        );
    }

    #[test]
    fn points_at_a_misspelt_node_and_attribute_with_a_suggestion() {
        let broken = CHECKS
            .replace("button Open", "buton Open")
            .replace("role=secondary", "rol=secondary");
        assert_eq!(
            errors(&broken),
            [
                "11:38: view.column[0].text.rol: unknown attribute `rol`; did you mean `role`? this node takes value, role, grow",
                "16:13: view.column[1].list.item.row[2].buton: unknown node `buton`; did you mean `button`? the kit has column, row, text, status, badge, button, list",
            ]
        );
    }

    #[test]
    fn points_at_a_binding_to_a_missing_field_and_a_tone_for_a_missing_value() {
        let broken = CHECKS
            .replace("{row.name}", "{row.nmae}")
            .replace("pass=good", "passed=good");
        assert_eq!(
            errors(&broken),
            [
                "14:34: view.column[1].list.item.row[0].status.tones.passed: `passed` is not a value of the bound field; did you mean `pass`? it takes pass, fail, pending",
                "15:18: view.column[1].list.item.row[1].text.value: binding `{row.nmae}`: rows have no field `nmae`; did you mean `name`? they have id, link, name, state",
            ]
        );
    }

    #[test]
    fn a_syntax_error_has_its_place() {
        // Some of kdl's errors span the whole node; an unclosed string is placed exactly.
        let errs = parse("column {\n  text \"x\n}\n").unwrap_err();
        assert_eq!(errs[0].at.map(|(l, _)| l), Some(2), "{errs:?}");
    }

    #[test]
    fn a_list_takes_one_item_and_a_text_no_children() {
        let broken = CHECKS.replace(
            "text \"{row.name}\" grow=#true",
            "text \"{row.name}\" grow=#true { badge x }",
        );
        assert_eq!(errors(&broken), ["15:44: view.column[1].list.item.row[1].text: `text` holds no children; only row, column and list do"]);
    }
}
