//! Template files: KDL a person or an agent writes, read into the value
//! [`crate::parse_value`] checks, with the line and column of every node and
//! entry so each error points at its place.
//!
//! ```kdl
//! schema "vorn:view@1"
//! rows "checks" {
//!     name string
//!     state enum pass fail pending
//!     link url
//! }
//! list "checks" empty="No pull request" {
//!     row {
//!         status "{row.state}" pass=good fail=bad pending=busy
//!         text "{row.name}" grow=#true
//!         button Open open="{row.link}"
//!     }
//! }
//! ```
//!
//! A node's first argument is its value, a button's label, a section's title
//! or a list's collection; its properties are its attributes, a status's
//! tones or a button's action; column, row and section hold their children
//! and a list its item. Anything unexpected is passed through, so the checker
//! reports it by path and the path finds its place in the file.

use std::collections::HashMap;

use kdl::{KdlDocument, KdlNode, KdlValue};
use serde_json::{Map, Value};

use crate::{Options, Template, TemplateError};

/// Parses and checks template `file` (its path within the extension, for errors).
pub fn parse(file: &str, text: &str, opts: Options) -> Result<Template, Vec<TemplateError>> {
    let (value, mut r) = read(file, text)?;
    let errors = match crate::parse_value(&value, opts) {
        Ok(t) if r.extra.is_empty() => return Ok(t),
        Ok(_) => Vec::new(),
        Err(e) => e,
    };
    let mut all = std::mem::take(&mut r.extra);
    for mut e in errors {
        e.at = r.find(&e.path);
        e.file = Some(file.to_owned());
        all.push(e);
    }
    all.sort_by_key(|e| e.at);
    Err(all)
}

/// Parses and checks template `file` into the JSON clients receive.
pub fn to_json(file: &str, text: &str, opts: Options) -> Result<Value, Vec<TemplateError>> {
    let (value, r) = read(file, text)?;
    if !r.extra.is_empty() {
        return Err(r.extra);
    }
    parse(file, text, opts)?;
    Ok(value)
}

fn read<'t>(file: &'t str, text: &'t str) -> Result<(Value, Reader<'t>), Vec<TemplateError>> {
    let doc: KdlDocument = text.parse().map_err(|e: kdl::KdlError| {
        e.diagnostics
            .iter()
            .map(|d| TemplateError {
                file: Some(file.to_owned()),
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
        file,
        text,
        spans: HashMap::new(),
        extra: Vec::new(),
    };
    let value = r.document(&doc);
    Ok((value, r))
}

struct Reader<'t> {
    file: &'t str,
    text: &'t str,
    /// Byte offset of each path, as the checker names paths.
    spans: HashMap<String, usize>,
    /// Errors only a file can have: two views, two collections of one name.
    extra: Vec<TemplateError>,
}

impl Reader<'_> {
    fn mark(&mut self, path: &str, offset: usize) {
        self.spans.entry(path.to_owned()).or_insert(offset);
    }

    fn extra(&mut self, offset: usize, path: &str, message: String) {
        self.extra.push(TemplateError {
            file: Some(self.file.to_owned()),
            at: Some(line_col(self.text, offset)),
            path: path.to_owned(),
            message,
        });
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
        let mut rows = Map::new();
        for node in doc.nodes() {
            let name = node.name().value();
            match name {
                "schema" => {
                    self.mark("schema", node.span().offset());
                    root.insert("schema".into(), first_arg(node).unwrap_or(Value::Null));
                }
                "rows" => {
                    let collection = match first_arg(node) {
                        Some(Value::String(s)) => s,
                        _ => crate::FIRST.to_owned(),
                    };
                    let path = format!("rows.{collection}");
                    if rows.contains_key(&collection) {
                        self.extra(
                            node.span().offset(),
                            &path,
                            format!("a second collection named '{collection}'"),
                        );
                        continue;
                    }
                    self.mark(&path, node.span().offset());
                    let fields = self.fields(node, &path);
                    rows.insert(collection, fields);
                }
                _ if root.contains_key("view") => self.extra(
                    node.span().offset(),
                    "view",
                    format!("a template has one view; '{name}' is a second"),
                ),
                _ => {
                    let v = self.node(node, "view");
                    root.insert("view".into(), v);
                }
            }
        }
        if !rows.is_empty() {
            root.insert("rows".into(), Value::Object(rows));
        }
        Value::Object(root)
    }

    /// `name string` is a field of that type; `state enum a b c` an enum.
    fn fields(&mut self, node: &KdlNode, path: &str) -> Value {
        let mut fields = Map::new();
        for f in node.children().map(KdlDocument::nodes).unwrap_or_default() {
            let name = f.name().value().to_owned();
            self.mark(&format!("{path}.{name}"), f.span().offset());
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
        let mut o = Map::new();
        let first = match kind {
            "button" => "label",
            "section" | "entry" => "title",
            "list" | "choice" | "board" => "rows",
            _ => "value",
        };
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
        let mut tones = Map::new();
        let mut action = Map::new();
        for e in node.entries() {
            let Some(name) = e.name() else { continue };
            let k = name.value();
            let (key, into) = match (kind, k) {
                (_, "when") => (format!("{path}.when"), &mut o),
                ("status" | "badge", _) => (format!("{path}.tones.{k}"), &mut tones),
                ("button", "open" | "call" | "focus" | "show" | "close" | "window" | "signin") => {
                    (format!("{path}.action.{k}"), &mut action)
                }
                _ => (format!("{path}.{k}"), &mut o),
            };
            self.mark(&key, name.span().offset());
            into.insert(k.to_owned(), value(e.value()));
        }
        if !tones.is_empty() {
            self.mark(&format!("{path}.tones"), node.span().offset());
            o.insert("tones".into(), Value::Object(tones));
        }
        if kind == "button" {
            self.mark(&format!("{path}.action"), node.span().offset());
            o.insert("action".into(), Value::Object(action));
        }
        match kind {
            "board" => self.board_parts(kids, &path, &mut o),
            "list" => match kids {
                [item] => {
                    let v = self.node(item, &format!("{path}.item"));
                    o.insert("item".into(), v);
                }
                [] => {}
                [first, second, ..] => {
                    let v = self.node(first, &format!("{path}.item"));
                    o.insert("item".into(), v);
                    self.extra(
                        second.span().offset(),
                        &format!("{path}.item"),
                        "a list draws one item per row; wrap these nodes in a row or a column"
                            .into(),
                    );
                }
            },
            _ if !kids.is_empty() => {
                self.mark(&format!("{path}.children"), kids[0].span().offset());
                let children = kids
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let p = format!("{path}[{i}]");
                        self.mark(&p, c.span().offset());
                        self.node(c, &p)
                    })
                    .collect();
                o.insert("children".into(), Value::Array(children));
            }
            _ => {}
        }
        let mut wrapped = Map::new();
        wrapped.insert(kind.to_owned(), Value::Object(o));
        Value::Object(wrapped)
    }
}

impl Reader<'_> {
    /// A board's `column` and `card` children: the columns in order, and the
    /// card's nodes as one column.
    fn board_parts(&mut self, kids: &[KdlNode], path: &str, o: &mut Map<String, Value>) {
        let mut columns = Vec::new();
        for kid in kids {
            match kid.name().value() {
                "column" => {
                    let at = format!("{path}.columns[{}]", columns.len());
                    self.mark(&at, kid.span().offset());
                    let mut c = Map::new();
                    if let Some(v) = first_arg(kid) {
                        c.insert("value".into(), v);
                    }
                    for e in kid.entries() {
                        if let Some(name) = e.name() {
                            self.mark(&format!("{at}.{}", name.value()), name.span().offset());
                            c.insert(name.value().to_owned(), value(e.value()));
                        }
                    }
                    columns.push(Value::Object(c));
                }
                "card" if !o.contains_key("card") => {
                    let at = format!("{path}.card");
                    self.mark(&at, kid.span().offset());
                    let nodes = kid.children().map(KdlDocument::nodes).unwrap_or_default();
                    let children: Vec<Value> = nodes
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let p = format!("{at}.column[{i}]");
                            self.mark(&p, c.span().offset());
                            self.node(c, &p)
                        })
                        .collect();
                    let mut column = Map::new();
                    column.insert("children".into(), Value::Array(children));
                    let mut wrapped = Map::new();
                    wrapped.insert("column".into(), Value::Object(column));
                    o.insert("card".into(), Value::Object(wrapped));
                }
                other => self.extra(
                    kid.span().offset(),
                    path,
                    format!("a board holds columns and one card, not '{other}'"),
                ),
            }
        }
        self.mark(
            &format!("{path}.columns"),
            kids.first().map_or(0, |k| k.span().offset()),
        );
        o.insert("columns".into(), Value::Array(columns));
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
