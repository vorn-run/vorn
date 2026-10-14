//! Typed view templates and the rows they draw, checked strictly.
//!
//! Templates are KDL files a person or an agent writes (`views/*.kdl`, read
//! by [`crate::view_file`] into the value checked here); vornd sends the
//! checked template and the rows to clients as JSON. Every error names the
//! field's path and, for a file, its line and column.
//!
//! An extension never draws: it hands over a template (a tree of kit nodes)
//! and keyed rows, and each client lays them out on Pixels or Cells. Parsing
//! turns the template's JSON into [`Template`] or into every error found,
//! each naming its path, so a client draws a red box saying what is wrong
//! instead of an empty one. Rows are checked against the fields the template
//! declares the same way ([`RowSchema::check`]).

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value};

/// The template schema this build reads.
pub const SCHEMA: &str = "vorn:view@0.1";

/// Node kinds, for the error that lists them.
const NODES: &[&str] = &["column", "row", "text", "status", "badge", "button", "list"];

/// One thing wrong with a template or its rows, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    /// Line and column in the template file; `None` for rows and for JSON.
    pub at: Option<(usize, usize)>,
    /// Dotted path from the template's root (`view.list.item.row[2].buton`)
    /// or from the rows (`rows[3].state`).
    pub path: String,
    pub message: String,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.at {
            Some((l, c)) => write!(f, "{l}:{c}: {}: {}", self.path, self.message),
            None => write!(f, "{}: {}", self.path, self.message),
        }
    }
}

/// A checked template: the fields each row carries and the view.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub rows: RowSchema,
    pub view: Node,
}

/// The fields every row has beside its `id`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowSchema {
    pub fields: BTreeMap<String, FieldType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldType {
    String,
    /// A string an `open` action may follow; only `https:` is accepted.
    Url,
    Number,
    Bool,
    /// One of these strings.
    Enum(Vec<String>),
}

/// A kit node. Layout, colour and spacing are the client's.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Column(Vec<Node>),
    Row(Vec<Node>),
    Text {
        value: Text,
        role: Role,
        grow: bool,
    },
    /// A status dot whose tone follows a value.
    Status {
        value: Text,
        tones: BTreeMap<String, Tone>,
    },
    Badge {
        value: Text,
        tones: BTreeMap<String, Tone>,
    },
    Button {
        label: Text,
        action: Action,
    },
    /// One `item` per row; `empty` when there are none.
    List {
        item: Box<Node>,
        empty: Option<Text>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Title,
    Body,
    Secondary,
    Mono,
}

/// Status colours, named by meaning: each client picks its own colour for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Bad,
    Warn,
    Busy,
    Muted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Opens a URL in the person's browser; runs in the client.
    Open(Text),
    /// Calls one of the extension's methods with the row's id.
    Call(String),
}

/// Text with bindings: `"{row.name} failed"`, `"{rows.count} checks"`,
/// `"{rows.state.fail} failing"`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text(Vec<Piece>);

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Lit(String),
    Field(String),
    Count,
    CountWhere(String, String),
}

/// A row as checked: its id and its fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub fields: Map<String, Value>,
}

impl Row {
    /// A field as text; an absent or null field is empty.
    pub fn text(&self, field: &str) -> String {
        match self.fields.get(field) {
            Some(Value::String(s)) => s.clone(),
            None | Some(Value::Null) => String::new(),
            Some(other) => other.to_string(),
        }
    }
}

impl Text {
    /// The text with its bindings filled from `row` (inside a list item) and `rows`.
    pub fn fill(&self, row: Option<&Row>, rows: &[Row]) -> String {
        let mut out = String::new();
        for p in &self.0 {
            match p {
                Piece::Lit(s) => out.push_str(s),
                Piece::Field(f) => out.push_str(&row.map(|r| r.text(f)).unwrap_or_default()),
                Piece::Count => out.push_str(&rows.len().to_string()),
                Piece::CountWhere(f, v) => {
                    let n = rows.iter().filter(|r| r.text(f) == *v).count();
                    out.push_str(&n.to_string());
                }
            }
        }
        out
    }
}

impl Tone {
    /// The tone `value` maps to; muted when the map does not name it.
    pub fn of(tones: &BTreeMap<String, Tone>, value: &str) -> Tone {
        tones.get(value).copied().unwrap_or(Tone::Muted)
    }
}

/// Parses and checks a template sent as JSON, as clients receive it.
pub fn from_json(text: &str) -> Result<Template, Vec<TemplateError>> {
    let v: Value = serde_json::from_str(text).map_err(|e| {
        vec![TemplateError {
            at: None,
            path: "$".into(),
            message: format!("not JSON: {e}"),
        }]
    })?;
    parse_value(&v)
}

/// Checks a template already parsed into a value (as a builder in Rust or
/// TypeScript emits it); errors carry no line.
pub fn parse_value(root: &Value) -> Result<Template, Vec<TemplateError>> {
    let mut p = Parser::default();
    let t = p.template(root);
    match t {
        Some(t) if p.errors.is_empty() => Ok(t),
        _ => Err(p.errors),
    }
}

#[derive(Default)]
struct Parser {
    errors: Vec<TemplateError>,
    schema: RowSchema,
}

/// Where a node sits: whether `{row.*}` means anything there.
#[derive(Clone, Copy)]
struct Scope {
    in_item: bool,
}

impl Parser {
    fn err(&mut self, path: &str, message: impl Into<String>) {
        self.errors.push(TemplateError {
            at: None,
            path: path.to_owned(),
            message: message.into(),
        });
    }

    /// An object with exactly the keys in `required` and some of `optional`.
    fn object<'v>(
        &mut self,
        v: &'v Value,
        path: &str,
        required: &[&str],
        optional: &[&str],
    ) -> Option<&'v Map<String, Value>> {
        let Some(o) = v.as_object() else {
            self.err(path, format!("expected an object, found {}", kind(v)));
            return None;
        };
        for k in o.keys() {
            if !required.contains(&k.as_str()) && !optional.contains(&k.as_str()) {
                let known: Vec<&str> = required.iter().chain(optional).copied().collect();
                let hint = suggest(k, &known)
                    .map(|s| format!("did you mean `{s}`? "))
                    .unwrap_or_default();
                self.err(
                    &format!("{path}.{k}"),
                    format!(
                        "unknown attribute `{k}`; {hint}this node takes {}",
                        known.join(", ")
                    ),
                );
            }
        }
        for k in required {
            if !o.contains_key(*k) {
                self.err(path, format!("missing `{k}`"));
            }
        }
        Some(o)
    }

    fn template(&mut self, root: &Value) -> Option<Template> {
        let o = self.object(root, "$", &["schema", "rows", "view"], &[])?;
        match o.get("schema").and_then(Value::as_str) {
            Some(SCHEMA) => {}
            Some(other) => self.err("schema", format!("this Vorn reads {SCHEMA}, not {other}")),
            None => self.err("schema", "expected a string"),
        }
        if let Some(rows) = o.get("rows") {
            self.schema = self.row_schema(rows);
        }
        let view = self.node(o.get("view")?, "view", Scope { in_item: false })?;
        Some(Template {
            rows: self.schema.clone(),
            view,
        })
    }

    fn row_schema(&mut self, v: &Value) -> RowSchema {
        let mut fields = BTreeMap::new();
        let Some(o) = v.as_object() else {
            self.err("rows", "expected an object of field types");
            return RowSchema::default();
        };
        for (k, t) in o {
            let path = format!("rows.{k}");
            if k == "id" {
                self.err(&path, "`id` is every row's key and is not declared");
                continue;
            }
            let ty = match t {
                Value::String(s) => match s.as_str() {
                    "string" => FieldType::String,
                    "url" => FieldType::Url,
                    "number" => FieldType::Number,
                    "bool" => FieldType::Bool,
                    other => {
                        self.err(&path, format!("unknown type `{other}`; use string, url, number, bool or a list of values"));
                        continue;
                    }
                },
                Value::Array(values) if !values.is_empty() => {
                    let strings: Vec<String> = values
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                    if strings.len() != values.len() {
                        self.err(&path, "an enum lists strings only");
                        continue;
                    }
                    FieldType::Enum(strings)
                }
                other => {
                    self.err(
                        &path,
                        format!(
                            "expected a type name or a list of values, found {}",
                            kind(other)
                        ),
                    );
                    continue;
                }
            };
            fields.insert(k.clone(), ty);
        }
        RowSchema { fields }
    }

    fn node(&mut self, v: &Value, path: &str, scope: Scope) -> Option<Node> {
        let Some(o) = v.as_object() else {
            self.err(
                path,
                format!(
                    "expected a node like {{\"text\": {{...}}}}, found {}",
                    kind(v)
                ),
            );
            return None;
        };
        let mut keys = o.keys();
        let (Some(k), None) = (keys.next(), keys.next()) else {
            self.err(path, "a node is an object with exactly one key, its kind");
            return None;
        };
        let body = &o[k];
        let path = format!("{path}.{k}");
        match k.as_str() {
            "column" | "row" => {
                let Some(items) = body.as_array() else {
                    self.err(&path, "expected a list of nodes");
                    return None;
                };
                let kids: Vec<Node> = items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| self.node(c, &format!("{path}[{i}]"), scope))
                    .collect();
                Some(if k == "row" {
                    Node::Row(kids)
                } else {
                    Node::Column(kids)
                })
            }
            "text" => {
                let o = self.object(body, &path, &["value"], &["role", "grow"])?;
                let value = self.text(o.get("value")?, &format!("{path}.value"), scope)?;
                let role = match o.get("role").map(|r| r.as_str()) {
                    None | Some(Some("body")) => Role::Body,
                    Some(Some("title")) => Role::Title,
                    Some(Some("secondary")) => Role::Secondary,
                    Some(Some("mono")) => Role::Mono,
                    Some(_) => {
                        self.err(
                            &format!("{path}.role"),
                            "expected title, body, secondary or mono",
                        );
                        Role::Body
                    }
                };
                let grow = self.boolean(o.get("grow"), &format!("{path}.grow"));
                Some(Node::Text { value, role, grow })
            }
            "status" | "badge" => {
                let required: &[&str] = if k == "status" {
                    &["value", "tones"]
                } else {
                    &["value"]
                };
                let o = self.object(body, &path, required, &["tones"])?;
                let value = self.text(o.get("value")?, &format!("{path}.value"), scope)?;
                let tones = match o.get("tones") {
                    Some(t) => self.tones(t, &format!("{path}.tones"), &value),
                    None => BTreeMap::new(),
                };
                Some(if k == "status" {
                    Node::Status { value, tones }
                } else {
                    Node::Badge { value, tones }
                })
            }
            "button" => {
                let o = self.object(body, &path, &["label", "action"], &[])?;
                let label = self.text(o.get("label")?, &format!("{path}.label"), scope)?;
                let action = self.action(o.get("action")?, &format!("{path}.action"), scope)?;
                Some(Node::Button { label, action })
            }
            "list" => {
                let o = self.object(body, &path, &["item"], &["empty"])?;
                if scope.in_item {
                    self.err(&path, "a list cannot sit inside a list's item");
                }
                let item = self.node(
                    o.get("item")?,
                    &format!("{path}.item"),
                    Scope { in_item: true },
                )?;
                let empty = match o.get("empty") {
                    Some(e) => Some(self.text(e, &format!("{path}.empty"), scope)?),
                    None => None,
                };
                Some(Node::List {
                    item: Box::new(item),
                    empty,
                })
            }
            other => {
                let hint = suggest(other, NODES)
                    .map(|s| format!("did you mean `{s}`? "))
                    .unwrap_or_default();
                self.err(
                    &path,
                    format!(
                        "unknown node `{other}`; {hint}the kit has {}",
                        NODES.join(", ")
                    ),
                );
                None
            }
        }
    }

    fn boolean(&mut self, v: Option<&Value>, path: &str) -> bool {
        match v {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(other) => {
                self.err(
                    path,
                    format!("expected true or false, found {}", kind(other)),
                );
                false
            }
        }
    }

    fn tones(&mut self, v: &Value, path: &str, value: &Text) -> BTreeMap<String, Tone> {
        let mut out = BTreeMap::new();
        let Some(o) = v.as_object() else {
            self.err(path, "expected an object from value to tone");
            return out;
        };
        // A tone keyed by a value the bound enum cannot take is a typo.
        let allowed = match value.0.as_slice() {
            [Piece::Field(f)] => match self.schema.fields.get(f) {
                Some(FieldType::Enum(vs)) => Some(vs.clone()),
                _ => None,
            },
            _ => None,
        };
        for (k, t) in o {
            let p = format!("{path}.{k}");
            if let Some(vs) = &allowed {
                if !vs.contains(k) {
                    let names: Vec<&str> = vs.iter().map(String::as_str).collect();
                    let hint = suggest(k, &names)
                        .map(|s| format!("did you mean `{s}`? "))
                        .unwrap_or_default();
                    self.err(
                        &p,
                        format!(
                            "`{k}` is not a value of the bound field; {hint}it takes {}",
                            vs.join(", ")
                        ),
                    );
                }
            }
            let tone = match t.as_str() {
                Some("good") => Tone::Good,
                Some("bad") => Tone::Bad,
                Some("warn") => Tone::Warn,
                Some("busy") => Tone::Busy,
                Some("muted") => Tone::Muted,
                _ => {
                    self.err(&p, "expected good, bad, warn, busy or muted");
                    continue;
                }
            };
            out.insert(k.clone(), tone);
        }
        out
    }

    fn action(&mut self, v: &Value, path: &str, scope: Scope) -> Option<Action> {
        let o = self.object(v, path, &[], &["open", "call"])?;
        match (o.get("open"), o.get("call")) {
            (Some(url), None) => {
                let p = format!("{path}.open");
                let t = self.text(url, &p, scope)?;
                // Only a field declared as a url, or a literal https link, may be opened.
                let ok = match t.0.as_slice() {
                    [Piece::Field(f)] => matches!(self.schema.fields.get(f), Some(FieldType::Url)),
                    [Piece::Lit(s)] => s.starts_with("https://"),
                    _ => false,
                };
                if !ok {
                    self.err(&p, "open takes one field declared as url, or an https link");
                }
                Some(Action::Open(t))
            }
            (None, Some(Value::String(m))) => Some(Action::Call(m.clone())),
            (None, Some(_)) => {
                self.err(&format!("{path}.call"), "expected a method name");
                None
            }
            _ => {
                self.err(
                    path,
                    "an action is either {\"open\": ...} or {\"call\": ...}",
                );
                None
            }
        }
    }

    fn text(&mut self, v: &Value, path: &str, scope: Scope) -> Option<Text> {
        let Some(s) = v.as_str() else {
            self.err(path, format!("expected text, found {}", kind(v)));
            return None;
        };
        let mut pieces = Vec::new();
        let mut rest = s;
        while let Some(a) = rest.find(['{', '}']) {
            if rest[a..].starts_with('}') {
                self.err(path, "a `}` with no `{`");
                return None;
            }
            if a > 0 {
                pieces.push(Piece::Lit(rest[..a].to_owned()));
            }
            let Some(b) = rest[a..].find('}') else {
                self.err(path, "a `{` with no `}`");
                return None;
            };
            let binding = &rest[a + 1..a + b];
            pieces.push(self.binding(binding, path, scope)?);
            rest = &rest[a + b + 1..];
        }
        if !rest.is_empty() {
            pieces.push(Piece::Lit(rest.to_owned()));
        }
        Some(Text(pieces))
    }

    fn binding(&mut self, b: &str, path: &str, scope: Scope) -> Option<Piece> {
        let parts: Vec<&str> = b.split('.').collect();
        let field_known = |s: &Self, f: &str| f == "id" || s.schema.fields.contains_key(f);
        match parts.as_slice() {
            ["row", f] if scope.in_item => {
                if field_known(self, f) {
                    Some(Piece::Field((*f).to_owned()))
                } else {
                    let known: Vec<&str> = std::iter::once("id")
                        .chain(self.schema.fields.keys().map(String::as_str))
                        .collect();
                    let hint = suggest(f, &known)
                        .map(|s| format!("did you mean `{s}`? "))
                        .unwrap_or_default();
                    self.err(
                        path,
                        format!(
                            "binding `{{{b}}}`: rows have no field `{f}`; {hint}they have {}",
                            known.join(", ")
                        ),
                    );
                    None
                }
            }
            ["row", _] => {
                self.err(
                    path,
                    format!("binding `{{{b}}}` is only meaningful inside a list's item"),
                );
                None
            }
            ["rows", "count"] => Some(Piece::Count),
            ["rows", f, v] => {
                match self.schema.fields.get(*f) {
                    Some(FieldType::Enum(vs)) if vs.iter().any(|x| x == v) => {
                        Some(Piece::CountWhere((*f).to_owned(), (*v).to_owned()))
                    }
                    _ => {
                        self.err(path, format!("binding `{{{b}}}`: `{f}` is not an enum field with the value `{v}`"));
                        None
                    }
                }
            }
            _ => {
                self.err(path, format!("unknown binding `{{{b}}}`; use row.<field>, rows.count or rows.<field>.<value>"));
                None
            }
        }
    }
}

/// The candidate within a small edit distance of `word`, for "did you mean".
pub fn suggest<'a>(word: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let limit = if word.len() <= 3 {
        1
    } else {
        (word.len() / 3).max(2)
    };
    candidates
        .iter()
        .map(|c| (distance(word, c), *c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// Levenshtein distance over chars.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != *cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

impl RowSchema {
    /// Checks an extension's answer: a list of objects with a unique string
    /// `id` and exactly the declared fields, each of its type.
    pub fn check(&self, rows: &Value) -> Result<Vec<Row>, Vec<TemplateError>> {
        let mut errors = Vec::new();
        let mut err = |path: String, message: String| {
            errors.push(TemplateError {
                at: None,
                path,
                message,
            })
        };
        let Some(list) = rows.as_array() else {
            err(
                "rows".into(),
                format!("expected a list of rows, found {}", kind(rows)),
            );
            return Err(errors);
        };
        let mut out = Vec::with_capacity(list.len());
        let mut seen = std::collections::HashSet::new();
        for (i, r) in list.iter().enumerate() {
            let path = format!("rows[{i}]");
            let Some(o) = r.as_object() else {
                err(path, format!("expected an object, found {}", kind(r)));
                continue;
            };
            let Some(id) = o.get("id").and_then(Value::as_str) else {
                err(format!("{path}.id"), "every row needs a string id".into());
                continue;
            };
            if !seen.insert(id.to_owned()) {
                err(format!("{path}.id"), format!("id `{id}` is used twice"));
            }
            let mut fields = Map::new();
            for (k, v) in o {
                if k == "id" {
                    continue;
                }
                let Some(ty) = self.fields.get(k) else {
                    err(
                        format!("{path}.{k}"),
                        format!("the template declares no field `{k}`"),
                    );
                    continue;
                };
                let ok = match (ty, v) {
                    (_, Value::Null) => true,
                    (FieldType::String, Value::String(_)) => true,
                    (FieldType::Url, Value::String(s)) => s.is_empty() || s.starts_with("https://"),
                    (FieldType::Number, Value::Number(_)) => true,
                    (FieldType::Bool, Value::Bool(_)) => true,
                    (FieldType::Enum(vs), Value::String(s)) => vs.contains(s),
                    _ => false,
                };
                if !ok {
                    let want = match ty {
                        FieldType::Enum(vs) => format!("one of {}", vs.join(", ")),
                        FieldType::Url => "an https link".into(),
                        other => format!("{other:?}").to_lowercase(),
                    };
                    err(format!("{path}.{k}"), format!("{v} is not {want}"));
                    continue;
                }
                fields.insert(k.clone(), v.clone());
            }
            out.push(Row {
                id: id.to_owned(),
                fields,
            });
        }
        if errors.is_empty() {
            Ok(out)
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn checks_template() -> Value {
        json!({
            "schema": SCHEMA,
            "rows": { "name": "string", "state": ["pass", "fail", "pending"], "link": "url" },
            "view": { "column": [
                { "text": { "value": "{rows.state.fail} failing of {rows.count}", "role": "secondary" } },
                { "list": {
                    "empty": "No pull request",
                    "item": { "row": [
                        { "status": { "value": "{row.state}", "tones": { "pass": "good", "fail": "bad", "pending": "busy" } } },
                        { "text": { "value": "{row.name}", "grow": true } },
                        { "button": { "label": "Open", "action": { "open": "{row.link}" } } }
                    ] }
                } }
            ] }
        })
    }

    fn errors_of(v: Value) -> Vec<String> {
        parse_value(&v)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn reads_a_valid_template_and_fills_its_bindings() {
        let t = parse_value(&checks_template()).unwrap();
        let rows = t
            .rows
            .check(&json!([
                { "id": "a", "name": "build", "state": "fail", "link": "https://x/1" },
                { "id": "b", "name": "lint", "state": "pass", "link": "https://x/2" }
            ]))
            .unwrap();
        let Node::Column(kids) = &t.view else {
            panic!("a column")
        };
        let Node::Text { value, .. } = &kids[0] else {
            panic!("a text")
        };
        assert_eq!(value.fill(None, &rows), "1 failing of 2");
    }

    #[test]
    fn names_the_path_of_a_misspelt_node() {
        let mut v = checks_template();
        let row = &mut v["view"]["column"][1]["list"]["item"]["row"][2];
        *row = json!({ "buton": { "label": "Open", "action": { "open": "{row.link}" } } });
        assert_eq!(
            errors_of(v),
            ["view.column[1].list.item.row[2].buton: unknown node `buton`; did you mean `button`? the kit has column, row, text, status, badge, button, list"]
        );
    }

    #[test]
    fn names_an_unknown_prop_and_a_binding_to_a_missing_field() {
        let mut v = checks_template();
        v["view"]["column"][1]["list"]["item"]["row"][1] =
            json!({ "text": { "value": "{row.nmae}", "size": 12 } });
        let errs = errors_of(v);
        assert!(
            errs.iter().any(|e| e.starts_with(
                "view.column[1].list.item.row[1].text.size: unknown attribute `size`"
            )),
            "{errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.contains("rows have no field `nmae`")),
            "{errs:?}"
        );
    }

    #[test]
    fn refuses_a_row_binding_outside_a_list_and_a_tone_for_a_value_the_enum_lacks() {
        let mut v = checks_template();
        v["view"]["column"][0] = json!({ "text": { "value": "{row.name}" } });
        v["view"]["column"][1]["list"]["item"]["row"][0]["status"]["tones"]["passed"] =
            json!("good");
        let errs = errors_of(v);
        assert!(
            errs.iter()
                .any(|e| e.contains("only meaningful inside a list's item")),
            "{errs:?}"
        );
        assert!(
            errs.iter()
                .any(|e| e.contains("tones.passed: `passed` is not a value")),
            "{errs:?}"
        );
    }

    #[test]
    fn only_opens_a_url_field() {
        let mut v = checks_template();
        v["view"]["column"][1]["list"]["item"]["row"][2]["button"]["action"]["open"] =
            json!("{row.name}");
        assert!(errors_of(v)[0].contains("open takes one field declared as url"));
    }

    #[test]
    fn checks_rows_against_the_declared_fields() {
        let t = parse_value(&checks_template()).unwrap();
        let errs: Vec<String> = t
            .rows
            .check(&json!([
                { "id": "a", "name": "build", "state": "succes" },
                { "id": "a", "name": "x", "colour": "red", "link": "http://insecure" },
                { "name": "no id" }
            ]))
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            errs,
            [
                "rows[0].state: \"succes\" is not one of pass, fail, pending",
                "rows[1].id: id `a` is used twice",
                "rows[1].colour: the template declares no field `colour`",
                "rows[1].link: \"http://insecure\" is not an https link",
                "rows[2].id: every row needs a string id",
            ]
        );
    }

    #[test]
    fn suggests_a_near_name() {
        assert_eq!(suggest("buton", NODES), Some("button"));
        assert_eq!(suggest("rol", &["value", "role", "grow"]), Some("role"));
        assert_eq!(suggest("colour", &["value", "role"]), None);
    }

    #[test]
    fn reports_braces_that_do_not_pair() {
        let mut v = checks_template();
        v["view"]["column"][0]["text"]["value"] = json!("{rows.count failing");
        assert!(errors_of(v)[0].ends_with("a `{` with no `}`"));
    }
}
