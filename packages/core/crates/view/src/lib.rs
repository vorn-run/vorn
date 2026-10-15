//! Extension templates: an extension never draws. It ships typed templates
//! (KDL files read by [`file`], sent to clients as the JSON checked here) and
//! publishes keyed rows; each client lays the nodes out with its UI kit, on
//! pixels or on terminal cells alike.
//!
//! Checking is strict: an unknown node, attribute, binding or tone is an
//! error naming its place and, when one is near, the name meant, so a client
//! draws a box saying what is wrong instead of an empty one.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value};

#[cfg(feature = "kdl")]
pub mod file;

/// The template schema this build reads.
pub const SCHEMA: &str = "vorn:view@1";

/// The node kinds, for the error that lists them.
pub const NODES: &[&str] = &[
    "column", "row", "section", "text", "icon", "status", "badge", "button", "toggle", "field",
    "list", "divider",
];

/// The name of a template's first row collection, whatever it is called.
pub const FIRST: &str = "rows";

/// The most rows one list draws; a longer list belongs in a pane.
pub const MAX_LIST: usize = 500;

/// One thing wrong with a template, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    /// The template's file, relative to its extension (`views/checks.kdl`).
    pub file: Option<String>,
    /// Line and column in the file; `None` for JSON.
    pub at: Option<(usize, usize)>,
    /// Dotted path from the template's root, as `view.column[1].list.item.buton`.
    pub path: String,
    pub message: String,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.file, self.at) {
            (Some(file), Some((l, c))) => write!(f, "{file}:{l}:{c}: {}", self.message),
            (None, Some((l, c))) => write!(f, "{l}:{c}: {}", self.message),
            (Some(file), None) => write!(f, "{file}: {}: {}", self.path, self.message),
            (None, None) => write!(f, "{}: {}", self.path, self.message),
        }
    }
}

/// Where a template is drawn, as far as its bindings care.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Drawn once per session (a card's badge or footer, a sidebar row's
    /// decoration): `{row.*}` is that session's row of the first collection.
    pub session_row: bool,
}

/// A checked template: its row collections and its view.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub collections: Vec<(String, RowSchema)>,
    pub view: Node,
}

impl Template {
    pub fn schema(&self, collection: &str) -> Option<&RowSchema> {
        self.collections
            .iter()
            .find(|(n, _)| n == collection)
            .map(|(_, s)| s)
    }

    /// The first collection's name: whose row a session's template draws.
    pub fn first(&self) -> Option<&str> {
        self.collections.first().map(|(n, _)| n.as_str())
    }
}

/// The fields every row of a collection has beside its `id`.
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

/// A kit node, shown only while its `when` holds. Layout, colour and spacing are the client's.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: Kind,
    pub when: Option<Cond>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Column(Vec<Node>),
    Row(Vec<Node>),
    /// A titled group, as a settings page's.
    Section {
        title: Text,
        children: Vec<Node>,
    },
    Text {
        value: Text,
        role: Role,
        grow: bool,
    },
    /// A named icon from the kit's set.
    Icon {
        name: Text,
        tone: Option<Tone>,
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
        /// Asked inline before the action runs.
        confirm: Option<Text>,
        style: Style,
        icon: Option<String>,
    },
    /// A switch showing a bool field; flipping it calls `call` with the new value.
    Toggle {
        value: Text,
        label: Option<Text>,
        call: String,
    },
    /// A one-line input; Return calls `call` with what it holds.
    Field {
        value: Option<Text>,
        placeholder: Option<Text>,
        call: String,
    },
    /// One `item` per row of `rows`, in the order published; `empty` when there are none.
    List {
        rows: String,
        item: Box<Node>,
        empty: Option<Text>,
    },
    Divider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Title,
    Body,
    Secondary,
    Mono,
}

/// Colours named by meaning: each client picks its own colour for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Bad,
    Warn,
    Busy,
    Muted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    #[default]
    Plain,
    Primary,
    Danger,
}

/// What a button does. At most one of `call`, `open` and `focus`; `show`
/// and `close` may join any of them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Action {
    /// One of the extension's methods, with the row's id.
    pub call: Option<String>,
    /// A URL in the person's browser.
    pub open: Option<Text>,
    /// A session, in the main window.
    pub focus: Option<Text>,
    /// Brings the main window forward.
    pub show: bool,
    /// Closes the dialog or window the template is in.
    pub close: bool,
}

/// A node's condition: its text is filled and read as true unless empty, `false` or `0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cond {
    pub text: Text,
    pub negate: bool,
}

/// Text with bindings: `"{row.name} failed"`, `"{rows.count} checks"`,
/// `"{rows.state.fail} failing"`, `"{summary.disk}"`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text(Vec<Piece>);

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Lit(String),
    /// A field of the row in scope.
    Field(String),
    Count(String),
    CountWhere(String, String, String),
    /// A field of a collection's first row.
    First(String, String),
}

/// A row: its key and its fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub fields: Map<String, Value>,
}

impl Row {
    /// A published row, if it has a string `id`.
    pub fn from_value(v: &Value) -> Option<Row> {
        let o = v.as_object()?;
        let id = o.get("id")?.as_str()?.to_owned();
        let fields = o
            .iter()
            .filter(|(k, _)| *k != "id")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Some(Row { id, fields })
    }

    /// A field as text; an absent or null field is empty.
    pub fn text(&self, field: &str) -> String {
        if field == "id" {
            return self.id.clone();
        }
        match self.fields.get(field) {
            Some(Value::String(s)) => s.clone(),
            None | Some(Value::Null) => String::new(),
            Some(other) => other.to_string(),
        }
    }
}

/// Where a template's rows come from, as a client keeps them.
pub trait Source {
    /// The rows of `collection`, or those whose `field` is `value`.
    fn count(&self, collection: &str, matching: Option<(&str, &str)>) -> usize;
    fn first(&self, collection: &str) -> Option<&Row>;
}

impl Source for BTreeMap<String, Vec<Row>> {
    fn count(&self, collection: &str, matching: Option<(&str, &str)>) -> usize {
        let rows = self.get(collection).map_or(&[][..], Vec::as_slice);
        match matching {
            None => rows.len(),
            Some((f, v)) => rows.iter().filter(|r| r.text(f) == v).count(),
        }
    }

    fn first(&self, collection: &str) -> Option<&Row> {
        self.get(collection).and_then(|r| r.first())
    }
}

impl Text {
    /// Literal text, with no bindings.
    pub fn lit(s: impl Into<String>) -> Text {
        Text(vec![Piece::Lit(s.into())])
    }

    /// The text with its bindings filled from `row` (in a list's item or a session's template) and `src`.
    pub fn fill(&self, row: Option<&Row>, src: &dyn Source) -> String {
        let mut out = String::new();
        for p in &self.0 {
            match p {
                Piece::Lit(s) => out.push_str(s),
                Piece::Field(f) => out.push_str(&row.map(|r| r.text(f)).unwrap_or_default()),
                Piece::Count(c) => out.push_str(&src.count(c, None).to_string()),
                Piece::CountWhere(c, f, v) => {
                    out.push_str(&src.count(c, Some((f, v))).to_string());
                }
                Piece::First(c, f) => {
                    out.push_str(&src.first(c).map(|r| r.text(f)).unwrap_or_default());
                }
            }
        }
        out
    }

    /// The row field this text is exactly, as a toggle's value is.
    pub fn field(&self) -> Option<&str> {
        match self.0.as_slice() {
            [Piece::Field(f)] => Some(f),
            _ => None,
        }
    }
}

impl Cond {
    pub fn holds(&self, row: Option<&Row>, src: &dyn Source) -> bool {
        truthy(&self.text.fill(row, src)) != self.negate
    }
}

/// How a filled binding reads as a condition or a toggle.
pub fn truthy(s: &str) -> bool {
    !matches!(s.trim(), "" | "false" | "0" | "null")
}

impl Tone {
    /// The tone `value` maps to; muted when the map does not name it.
    pub fn of(tones: &BTreeMap<String, Tone>, value: &str) -> Tone {
        tones.get(value).copied().unwrap_or(Tone::Muted)
    }

    fn named(s: &str) -> Option<Tone> {
        Some(match s {
            "good" => Tone::Good,
            "bad" => Tone::Bad,
            "warn" => Tone::Warn,
            "busy" => Tone::Busy,
            "muted" => Tone::Muted,
            _ => return None,
        })
    }
}

/// Checks a template as JSON, as clients receive it from vornd.
pub fn from_json(text: &str, opts: Options) -> Result<Template, Vec<TemplateError>> {
    let v: Value = serde_json::from_str(text).map_err(|e| {
        vec![TemplateError {
            file: None,
            at: None,
            path: "$".into(),
            message: format!("not JSON: {e}"),
        }]
    })?;
    parse_value(&v, opts)
}

/// Checks a template already parsed into a value; errors carry no line.
pub fn parse_value(root: &Value, opts: Options) -> Result<Template, Vec<TemplateError>> {
    let mut p = Parser {
        errors: Vec::new(),
        collections: Vec::new(),
    };
    let t = p.template(root, opts);
    match t {
        Some(t) if p.errors.is_empty() => Ok(t),
        _ => {
            if p.errors.is_empty() {
                p.err("$", "not a template");
            }
            Err(p.errors)
        }
    }
}

struct Parser {
    errors: Vec<TemplateError>,
    collections: Vec<(String, RowSchema)>,
}

/// Which collection `{row.*}` reads where a node sits, if any.
#[derive(Clone, Copy)]
struct Scope<'a> {
    row: Option<&'a str>,
    in_item: bool,
}

/// `'a', 'b' and 'c'`, for an error listing names.
fn listed(names: &[&str]) -> String {
    names.join(", ")
}

/// `; did you mean 'x'?`, or `; <alternatives>`.
fn hint(word: &str, known: &[&str], otherwise: impl FnOnce() -> String) -> String {
    match suggest(word, known) {
        Some(s) => format!("; did you mean '{s}'?"),
        None => format!("; {}", otherwise()),
    }
}

fn keys_of(kind: &str) -> &'static [&'static str] {
    match kind {
        "column" | "row" => &["children", "when"],
        "section" => &["title", "children", "when"],
        "text" => &["value", "role", "grow", "when"],
        "icon" => &["value", "tone", "when"],
        "status" | "badge" => &["value", "tones", "when"],
        "button" => &["label", "action", "confirm", "style", "icon", "when"],
        "toggle" => &["value", "label", "call", "when"],
        "field" => &["value", "placeholder", "call", "when"],
        "list" => &["rows", "item", "empty", "when"],
        _ => &["when"],
    }
}

fn required_of(kind: &str) -> &'static [&'static str] {
    match kind {
        "section" => &["title"],
        "text" | "icon" | "status" | "badge" => &["value"],
        "button" => &["label", "action"],
        "toggle" => &["value", "call"],
        "field" => &["call"],
        "list" => &["item"],
        _ => &[],
    }
}

impl Parser {
    fn err(&mut self, path: &str, message: impl Into<String>) {
        self.errors.push(TemplateError {
            file: None,
            at: None,
            path: path.to_owned(),
            message: message.into(),
        });
    }

    fn template(&mut self, root: &Value, opts: Options) -> Option<Template> {
        let Some(o) = root.as_object() else {
            self.err(
                "$",
                format!("expected a template object, found {}", kind(root)),
            );
            return None;
        };
        for k in o.keys() {
            if !["schema", "rows", "view"].contains(&k.as_str()) {
                let h = hint(k, &["schema", "rows", "view"], || {
                    "a template has schema, rows and view".into()
                });
                self.err(k, format!("unknown entry '{k}'{h}"));
            }
        }
        match o.get("schema").and_then(Value::as_str) {
            Some(SCHEMA) => {}
            Some(other) => self.err("schema", format!("this Vorn reads {SCHEMA}, not {other}")),
            None => self.err(
                "schema",
                format!("a template starts with schema \"{SCHEMA}\""),
            ),
        }
        if let Some(rows) = o.get("rows") {
            self.collections = self.row_schemas(rows);
        }
        let Some(view) = o.get("view") else {
            self.err("view", "a template has one view node");
            return None;
        };
        let first = self.collections.first().map(|(n, _)| n.clone());
        let scope = Scope {
            row: opts.session_row.then_some(first.as_deref()).flatten(),
            in_item: false,
        };
        if opts.session_row && first.is_none() {
            self.err(
                "rows",
                "a session's template declares the rows it draws one of",
            );
        }
        let view = self.node(view, "view", scope)?;
        Some(Template {
            collections: self.collections.clone(),
            view,
        })
    }

    fn row_schemas(&mut self, v: &Value) -> Vec<(String, RowSchema)> {
        let Some(o) = v.as_object() else {
            self.err("rows", "expected an object from collection to fields");
            return Vec::new();
        };
        let mut out = Vec::new();
        for (name, fields) in o {
            let path = format!("rows.{name}");
            let good = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if !good || name == "row" {
                self.err(&path, format!("'{name}' is not a collection name: lowercase letters, digits, - and _, and not 'row'"));
                continue;
            }
            out.push((name.clone(), self.row_schema(fields, &path)));
        }
        out
    }

    fn row_schema(&mut self, v: &Value, path: &str) -> RowSchema {
        let mut fields = BTreeMap::new();
        let Some(o) = v.as_object() else {
            self.err(path, "expected an object of field types");
            return RowSchema::default();
        };
        for (k, t) in o {
            let path = format!("{path}.{k}");
            if k == "id" {
                self.err(&path, "'id' is every row's key and is not declared");
                continue;
            }
            let ty = match t {
                Value::String(s) => match s.as_str() {
                    "string" => FieldType::String,
                    "url" => FieldType::Url,
                    "number" => FieldType::Number,
                    "bool" => FieldType::Bool,
                    other => {
                        let h = hint(other, &["string", "url", "number", "bool", "enum"], || {
                            "use string, url, number, bool or enum with its values".into()
                        });
                        self.err(&path, format!("unknown type '{other}'{h}"));
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
                        format!("expected a type name or enum values, found {}", kind(other)),
                    );
                    continue;
                }
            };
            fields.insert(k.clone(), ty);
        }
        RowSchema { fields }
    }

    fn node(&mut self, v: &Value, path: &str, scope: Scope<'_>) -> Option<Node> {
        let Some(o) = v.as_object() else {
            self.err(path, format!("expected a node, found {}", kind(v)));
            return None;
        };
        let mut keys = o.keys();
        let (Some(k), None) = (keys.next(), keys.next()) else {
            self.err(path, "a node is an object with exactly one key, its kind");
            return None;
        };
        let path = format!("{path}.{k}");
        if !NODES.contains(&k.as_str()) {
            let h = hint(k, NODES, || format!("nodes are {}", listed(NODES)));
            self.err(&path, format!("unknown node '{k}'{h}"));
            return None;
        }
        let empty = Map::new();
        let body = match &o[k] {
            Value::Object(b) => b,
            Value::Null => &empty,
            other => {
                self.err(
                    &path,
                    format!("expected the node's attributes, found {}", kind(other)),
                );
                return None;
            }
        };
        let known = keys_of(k);
        for a in body.keys() {
            if !known.contains(&a.as_str()) {
                let message = if a == "children" {
                    format!("'{k}' holds no children; only column, row, section and list do")
                } else {
                    let h = hint(a, known, || format!("{k} takes {}", listed(known)));
                    format!("unknown attribute '{a}' on {k}{h}")
                };
                self.err(&format!("{path}.{a}"), message);
            }
        }
        for r in required_of(k) {
            if !body.contains_key(*r) {
                self.err(&path, format!("{k} needs '{r}'"));
            }
        }
        let when = match body.get("when") {
            Some(w) => self.cond(w, &format!("{path}.when"), scope),
            None => None,
        };
        let text = |p: &mut Self, key: &str| -> Option<Text> {
            body.get(key)
                .and_then(|v| p.text(v, &format!("{path}.{key}"), scope))
        };
        let kind = match k.as_str() {
            "column" | "row" | "section" => {
                let kids = self.children(body.get("children"), &path, scope);
                match k.as_str() {
                    "column" => Kind::Column(kids),
                    "row" => Kind::Row(kids),
                    _ => Kind::Section {
                        title: text(self, "title")?,
                        children: kids,
                    },
                }
            }
            "text" => {
                let value = text(self, "value")?;
                let role = match body.get("role") {
                    None => Role::Body,
                    Some(r) => match r.as_str() {
                        Some("body") => Role::Body,
                        Some("title") => Role::Title,
                        Some("secondary") => Role::Secondary,
                        Some("mono") => Role::Mono,
                        other => {
                            let h = hint(
                                other.unwrap_or(""),
                                &["title", "body", "secondary", "mono"],
                                || "a role is title, body, secondary or mono".into(),
                            );
                            self.err(
                                &format!("{path}.role"),
                                format!("unknown role '{}'{h}", other.unwrap_or("")),
                            );
                            Role::Body
                        }
                    },
                };
                let grow = self.boolean(body.get("grow"), &format!("{path}.grow"));
                Kind::Text { value, role, grow }
            }
            "icon" => {
                let name = text(self, "value")?;
                let tone = body
                    .get("tone")
                    .and_then(|t| self.tone(t, &format!("{path}.tone")));
                Kind::Icon { name, tone }
            }
            "status" | "badge" => {
                let value = text(self, "value")?;
                let tones = match body.get("tones") {
                    Some(t) => self.tones(t, &format!("{path}.tones"), &value, scope),
                    None => BTreeMap::new(),
                };
                if k == "status" {
                    Kind::Status { value, tones }
                } else {
                    Kind::Badge { value, tones }
                }
            }
            "button" => {
                let label = text(self, "label")?;
                let action = self.action(body.get("action")?, &format!("{path}.action"), scope)?;
                let confirm = text(self, "confirm");
                let style = match body.get("style").map(Value::as_str) {
                    None | Some(Some("plain")) => Style::Plain,
                    Some(Some("primary")) => Style::Primary,
                    Some(Some("danger")) => Style::Danger,
                    Some(other) => {
                        let h = hint(other.unwrap_or(""), &["plain", "primary", "danger"], || {
                            "a style is plain, primary or danger".into()
                        });
                        self.err(
                            &format!("{path}.style"),
                            format!("unknown style '{}'{h}", other.unwrap_or("")),
                        );
                        Style::Plain
                    }
                };
                let icon = match body.get("icon") {
                    None => None,
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(other) => {
                        self.err(
                            &format!("{path}.icon"),
                            format!("expected an icon name, found {}", kind(other)),
                        );
                        None
                    }
                };
                Kind::Button {
                    label,
                    action,
                    confirm,
                    style,
                    icon,
                }
            }
            "toggle" => {
                let value = text(self, "value")?;
                if !self.is_bool(&value, scope) {
                    self.err(
                        &format!("{path}.value"),
                        "a toggle shows one bool field, as \"{row.on}\"",
                    );
                }
                let label = text(self, "label");
                let call = self.method(body.get("call")?, &format!("{path}.call"))?;
                Kind::Toggle { value, label, call }
            }
            "field" => {
                let value = text(self, "value");
                let placeholder = text(self, "placeholder");
                let call = self.method(body.get("call")?, &format!("{path}.call"))?;
                Kind::Field {
                    value,
                    placeholder,
                    call,
                }
            }
            "list" => {
                if scope.in_item {
                    self.err(&path, "a list cannot sit inside a list's item");
                }
                let rows = match body.get("rows") {
                    None => match self.collections.first() {
                        Some((n, _)) => n.clone(),
                        None => {
                            self.err(
                                &path,
                                "a list draws a collection the template declares under rows",
                            );
                            return None;
                        }
                    },
                    Some(Value::String(c)) => {
                        let c = self.collection(c, &format!("{path}.rows"))?;
                        c.to_owned()
                    }
                    Some(other) => {
                        self.err(
                            &format!("{path}.rows"),
                            format!("expected a collection's name, found {}", kind(other)),
                        );
                        return None;
                    }
                };
                let item = self.node(
                    body.get("item")?,
                    &format!("{path}.item"),
                    Scope {
                        row: Some(&rows),
                        in_item: true,
                    },
                )?;
                let empty = text(self, "empty");
                Kind::List {
                    rows,
                    item: Box::new(item),
                    empty,
                }
            }
            _ => Kind::Divider,
        };
        Some(Node { kind, when })
    }

    fn children(&mut self, v: Option<&Value>, path: &str, scope: Scope<'_>) -> Vec<Node> {
        match v {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .filter_map(|(i, c)| self.node(c, &format!("{path}[{i}]"), scope))
                .collect(),
            Some(other) => {
                self.err(
                    &format!("{path}.children"),
                    format!("expected a list of nodes, found {}", kind(other)),
                );
                Vec::new()
            }
        }
    }

    /// `c`, or the first collection for `rows`, if the template declares it.
    fn collection(&mut self, c: &str, path: &str) -> Option<String> {
        if c == FIRST {
            if let Some((n, _)) = self.collections.first() {
                return Some(n.clone());
            }
        }
        if self.collections.iter().any(|(n, _)| n == c) {
            return Some(c.to_owned());
        }
        let names: Vec<&str> = self.collections.iter().map(|(n, _)| n.as_str()).collect();
        let h = hint(c, &names, || {
            if names.is_empty() {
                "the template declares none under rows".into()
            } else {
                format!("it declares {}", listed(&names))
            }
        });
        self.err(path, format!("no collection '{c}'{h}"));
        None
    }

    fn boolean(&mut self, v: Option<&Value>, path: &str) -> bool {
        match v {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(other) => {
                self.err(
                    path,
                    format!("expected #true or #false, found {}", kind(other)),
                );
                false
            }
        }
    }

    fn method(&mut self, v: &Value, path: &str) -> Option<String> {
        let ok = v.as_str().filter(|m| {
            !m.is_empty()
                && m.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        });
        if ok.is_none() {
            self.err(path, "expected one of the extension's method names");
        }
        ok.map(str::to_owned)
    }

    fn tone(&mut self, v: &Value, path: &str) -> Option<Tone> {
        let s = v.as_str().unwrap_or("");
        let t = Tone::named(s);
        if t.is_none() {
            let h = hint(s, &["good", "bad", "warn", "busy", "muted"], || {
                "a tone is good, bad, warn, busy or muted".into()
            });
            self.err(path, format!("unknown tone '{s}'{h}"));
        }
        t
    }

    fn field_type(&self, collection: Option<&str>, field: &str) -> Option<&FieldType> {
        let c = collection?;
        self.collections
            .iter()
            .find(|(n, _)| n == c)
            .and_then(|(_, s)| s.fields.get(field))
    }

    fn is_bool(&self, t: &Text, scope: Scope<'_>) -> bool {
        match t.0.as_slice() {
            [Piece::Field(f)] => matches!(self.field_type(scope.row, f), Some(FieldType::Bool)),
            [Piece::First(c, f)] => matches!(self.field_type(Some(c), f), Some(FieldType::Bool)),
            _ => false,
        }
    }

    fn tones(
        &mut self,
        v: &Value,
        path: &str,
        value: &Text,
        scope: Scope<'_>,
    ) -> BTreeMap<String, Tone> {
        let mut out = BTreeMap::new();
        let Some(o) = v.as_object() else {
            self.err(path, "expected value=tone pairs");
            return out;
        };
        // A tone for a value the bound enum cannot take is a typo.
        let allowed = match value.0.as_slice() {
            [Piece::Field(f)] => match self.field_type(scope.row, f) {
                Some(FieldType::Enum(vs)) => Some(vs.clone()),
                _ => None,
            },
            [Piece::First(c, f)] => match self.field_type(Some(c), f) {
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
                    let h = hint(k, &names, || format!("it takes {}", listed(&names)));
                    self.err(&p, format!("'{k}' is not a value of the bound field{h}"));
                }
            }
            if let Some(tone) = self.tone(t, &p) {
                out.insert(k.clone(), tone);
            }
        }
        out
    }

    fn action(&mut self, v: &Value, path: &str, scope: Scope<'_>) -> Option<Action> {
        let Some(o) = v.as_object() else {
            self.err(path, "expected an action: call, open, focus, show or close");
            return None;
        };
        let names = ["call", "open", "focus", "show", "close"];
        let mut a = Action::default();
        for (k, val) in o {
            let p = format!("{path}.{k}");
            match k.as_str() {
                "call" => a.call = self.method(val, &p),
                "open" => {
                    let t = self.text(val, &p, scope)?;
                    // Only a field declared as a url, or a literal https link, may be opened.
                    let ok = match t.0.as_slice() {
                        [Piece::Field(f)] => {
                            matches!(self.field_type(scope.row, f), Some(FieldType::Url))
                        }
                        [Piece::First(c, f)] => {
                            matches!(self.field_type(Some(c), f), Some(FieldType::Url))
                        }
                        [Piece::Lit(s)] => s.starts_with("https://"),
                        _ => false,
                    };
                    if !ok {
                        self.err(&p, "open takes one field declared as url, or an https link");
                    }
                    a.open = Some(t);
                }
                "focus" => a.focus = self.text(val, &p, scope),
                "show" => a.show = self.boolean(Some(val), &p),
                "close" => a.close = self.boolean(Some(val), &p),
                other => {
                    let h = hint(other, &names, || {
                        "an action is call, open, focus, show or close".into()
                    });
                    self.err(&p, format!("unknown action '{other}'{h}"));
                }
            }
        }
        let goes = [a.call.is_some(), a.open.is_some(), a.focus.is_some()]
            .iter()
            .filter(|x| **x)
            .count();
        if goes > 1 {
            self.err(path, "a button does one of call, open and focus");
        }
        if goes == 0 && !a.show && !a.close {
            self.err(
                path,
                "a button needs an action: call, open, focus, show or close",
            );
        }
        Some(a)
    }

    fn cond(&mut self, v: &Value, path: &str, scope: Scope<'_>) -> Option<Cond> {
        let Some(s) = v.as_str() else {
            self.err(
                path,
                format!("expected a binding as \"{{row.on}}\", found {}", kind(v)),
            );
            return None;
        };
        let (negate, rest) = match s.strip_prefix('!') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let text = self.text(&Value::String(rest.to_owned()), path, scope)?;
        if !text.0.iter().any(|p| !matches!(p, Piece::Lit(_))) {
            self.err(
                path,
                "when takes a binding, as \"{row.on}\" or \"!{rows.count}\"",
            );
        }
        Some(Cond { text, negate })
    }

    fn text(&mut self, v: &Value, path: &str, scope: Scope<'_>) -> Option<Text> {
        let s = match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            other => {
                self.err(path, format!("expected text, found {}", kind(other)));
                return None;
            }
        };
        let mut pieces = Vec::new();
        let mut rest = s.as_str();
        while let Some(a) = rest.find(['{', '}']) {
            if rest[a..].starts_with('}') {
                self.err(path, "a '}' with no '{'");
                return None;
            }
            if a > 0 {
                pieces.push(Piece::Lit(rest[..a].to_owned()));
            }
            let Some(b) = rest[a..].find('}') else {
                self.err(path, "a '{' with no '}'");
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

    fn field_of(&mut self, collection: &str, f: &str, b: &str, path: &str) -> bool {
        let Some(schema) = self.collections.iter().find(|(n, _)| n == collection) else {
            return false;
        };
        if f == "id" || schema.1.fields.contains_key(f) {
            return true;
        }
        let known: Vec<&str> = std::iter::once("id")
            .chain(schema.1.fields.keys().map(String::as_str))
            .collect();
        let h = hint(f, &known, || format!("its rows have {}", listed(&known)));
        let message = format!("'{{{b}}}': {collection} has no field '{f}'{h}");
        self.err(path, message);
        false
    }

    fn binding(&mut self, b: &str, path: &str, scope: Scope<'_>) -> Option<Piece> {
        let parts: Vec<&str> = b.split('.').collect();
        match parts.as_slice() {
            ["row", f] => {
                let Some(c) = scope.row else {
                    self.err(
                        path,
                        format!("'{{{b}}}' means something only in a list's item or a session's template"),
                    );
                    return None;
                };
                self.field_of(c, f, b, path)
                    .then(|| Piece::Field((*f).to_owned()))
            }
            [c, "count"] => {
                let c = self.collection(c, path)?;
                Some(Piece::Count(c))
            }
            [c, f, v] => {
                let c = self.collection(c, path)?;
                match self.field_type(Some(&c), f) {
                    Some(FieldType::Enum(vs)) if vs.iter().any(|x| x == v) => {
                        Some(Piece::CountWhere(c, (*f).to_owned(), (*v).to_owned()))
                    }
                    _ => {
                        self.err(
                            path,
                            format!("'{{{b}}}': {c}.{f} is not an enum with the value '{v}'"),
                        );
                        None
                    }
                }
            }
            [c, f] => {
                let c = self.collection(c, path)?;
                self.field_of(&c, f, b, path)
                    .then(|| Piece::First(c.clone(), (*f).to_owned()))
            }
            _ => {
                self.err(path, format!("unknown binding '{{{b}}}'; use row.<field>, <rows>.count, <rows>.<field> or <rows>.<field>.<value>"));
                None
            }
        }
    }
}

/// The candidate within a small edit distance of `word`, for "did you mean".
pub fn suggest<'a>(word: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let limit = if word.chars().count() <= 3 {
        1
    } else {
        (word.chars().count() / 3).max(2)
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
    /// One published row: a string `id` and declared fields of their types.
    pub fn check_row(&self, v: &Value) -> Result<Row, String> {
        let Some(o) = v.as_object() else {
            return Err(format!("a row is an object, not {}", kind(v)));
        };
        let Some(id) = o.get("id").and_then(Value::as_str) else {
            return Err("every row needs a string id".into());
        };
        let mut fields = Map::new();
        for (k, v) in o {
            if k == "id" {
                continue;
            }
            let Some(ty) = self.fields.get(k) else {
                return Err(format!("row {id}: the template declares no field '{k}'"));
            };
            let ok = match (ty, v) {
                (_, Value::Null) => true,
                (FieldType::String, Value::String(_))
                | (FieldType::Number, Value::Number(_))
                | (FieldType::Bool, Value::Bool(_)) => true,
                (FieldType::Url, Value::String(s)) => s.is_empty() || s.starts_with("https://"),
                (FieldType::Enum(vs), Value::String(s)) => vs.contains(s),
                _ => false,
            };
            if !ok {
                let want = match ty {
                    FieldType::Enum(vs) => format!("one of {}", vs.join(", ")),
                    FieldType::Url => "an https link".into(),
                    FieldType::String => "text".into(),
                    FieldType::Number => "a number".into(),
                    FieldType::Bool => "#true or #false".into(),
                };
                return Err(format!("row {id}: {k} is {v}, not {want}"));
            }
            fields.insert(k.clone(), v.clone());
        }
        Ok(Row {
            id: id.to_owned(),
            fields,
        })
    }
}

#[cfg(test)]
mod tests;
