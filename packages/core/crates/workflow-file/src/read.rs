//! A workflow file read into the stored workflow, every error at its line
//! and column.

use serde_json::{Map, Number, Value};
use toml_edit::{Document, Item, Table};

use crate::write::{default_id, EDGE_KEYS, TAGS};
use crate::{Bindings, FileError};

/// A file read: the workflow, and what it names that this machine lacks.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub workflow: Value,
    /// A connection no connection here is called, kept by its name: the
    /// step that uses it fails until one is.
    pub warnings: Vec<FileError>,
    /// A setting whose name is a step field's but for a letter: most likely
    /// a typo, read as a setting all the same.
    pub hints: Vec<FileError>,
}

/// Reads workflow file `text`, its connections and paths as `bindings` name them.
pub fn read(text: &str, bindings: &Bindings<'_>) -> Result<Read, Vec<FileError>> {
    let doc = Document::parse(text).map_err(|e| {
        vec![FileError {
            at: line_col(text, e.span().map_or(0, |s| s.start)),
            message: e.message().trim().to_owned(),
        }]
    })?;
    let mut r = Reader {
        text,
        bindings,
        errors: Vec::new(),
        warnings: Vec::new(),
        hints: Vec::new(),
    };
    let workflow = r.document(doc.as_table());
    if r.errors.is_empty() {
        Ok(Read {
            workflow,
            warnings: r.warnings,
            hints: r.hints,
        })
    } else {
        r.errors.sort_by_key(|e| e.at);
        Err(r.errors)
    }
}

/// Line and column, from 1, of byte `offset` of `text`.
pub(crate) fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// The fields a typo of which is worth a hint: long enough to tell apart.
const HINTED: [&str; 4] = ["label", "slug", "onError", "needs"];

/// The step field `word` is one letter away from, when there is one.
fn near(word: &str) -> Option<&'static str> {
    fn distance(a: &str, b: &str) -> usize {
        let b: Vec<char> = b.chars().collect();
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.chars().enumerate() {
            let mut prev = row[0];
            row[0] = i + 1;
            for j in 0..b.len() {
                let cur = row[j + 1];
                row[j + 1] = (prev + usize::from(ca != b[j]))
                    .min(row[j] + 1)
                    .min(cur + 1);
                prev = cur;
            }
        }
        row[b.len()]
    }
    // A swap of two letters is one slip too.
    let swapped = |a: &str, b: &str| {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let diff: Vec<usize> = (0..a.len()).filter(|&i| a.get(i) != b.get(i)).collect();
        a.len() == b.len() && diff.len() == 2 && a[diff[0]] == b[diff[1]] && a[diff[1]] == b[diff[0]]
    };
    HINTED
        .into_iter()
        .find(|k| *k != word && (distance(word, k) == 1 || swapped(word, k)))
}

/// A key as the file wrote it, back as the stored key: one `@` less.
fn unescape(name: &str) -> &str {
    name.strip_prefix('@').unwrap_or(name)
}

/// Where an item starts in the file, when it came from one.
fn start(item: &Item) -> Option<usize> {
    match item {
        Item::Value(v) => v.span(),
        Item::Table(t) => t.span(),
        Item::ArrayOfTables(a) => a.span(),
        Item::None => None,
    }
    .map(|s| s.start)
}

/// An item as a table: a `[header]` one or an inline `{ … }` one.
fn table_of(item: &Item) -> Option<Table> {
    match item {
        Item::Table(t) => Some(t.clone()),
        Item::Value(toml_edit::Value::InlineTable(t)) => Some(t.clone().into_table()),
        _ => None,
    }
}

/// An item as a list of tables: `[[header]]` ones or a list of inline ones.
fn tables_of(item: &Item) -> Option<Vec<Table>> {
    match item {
        Item::ArrayOfTables(a) => Some(a.iter().cloned().collect()),
        Item::Value(toml_edit::Value::Array(a)) => a
            .iter()
            .map(|v| match v {
                toml_edit::Value::InlineTable(t) => Some(t.clone().into_table()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

struct Reader<'a> {
    text: &'a str,
    bindings: &'a Bindings<'a>,
    errors: Vec<FileError>,
    warnings: Vec<FileError>,
    hints: Vec<FileError>,
}

/// One word of `needs`: the step needed, the branch of it, and the edge's own id.
type Need = (String, Option<String>, Option<String>);

/// A step's edges as its `needs` gives them.
enum Needs {
    /// None said: the step before.
    Before,
    List(Vec<Need>),
}

impl Reader<'_> {
    fn err(&mut self, offset: Option<usize>, message: impl Into<String>) {
        self.errors.push(FileError {
            at: line_col(self.text, offset.unwrap_or(0)),
            message: message.into(),
        });
    }

    /// Where key `key` of `table` is, else where its value is.
    fn key_at(table: &Table, key: &str) -> Option<usize> {
        table
            .key(key)
            .and_then(toml_edit::Key::span)
            .map(|s| s.start)
            .or_else(|| table.get(key).and_then(start))
    }

    fn document(&mut self, root: &Table) -> Value {
        let mut out = Map::new();
        let mut nodes: Vec<(Option<usize>, Value, Needs)> = Vec::new();
        let mut whole: Vec<Value> = Vec::new();
        let mut order: Option<(Option<usize>, Vec<usize>)> = None;
        let mut raw_nodes: Option<Value> = None;
        let mut raw_edges: Option<Value> = None;
        for (key, item) in root.iter() {
            let at = Self::key_at(root, key);
            match key {
                "trigger" => match table_of(item) {
                    Some(t) => {
                        let (node, needs) = self.node(&t, true);
                        nodes.insert(0, (t.span().map(|s| s.start).or(at), node, needs));
                    }
                    None => self.err(at, "trigger is a table: [trigger], then its type and settings"),
                },
                "step" => match tables_of(item) {
                    Some(steps) => {
                        for t in steps {
                            let (node, needs) = self.node(&t, false);
                            nodes.push((t.span().map(|s| s.start).or(at), node, needs));
                        }
                    }
                    None => self.err(at, "step is a list of tables: one [[step]] for each"),
                },
                "edge" => match tables_of(item) {
                    Some(edges) => {
                        for t in edges {
                            let edge = self.edge(&t);
                            whole.push(edge);
                        }
                    }
                    None => self.err(at, "edge is a list of tables: one [[edge]] for each"),
                },
                "edgeOrder" => {
                    let places = item.as_array().map(|a| {
                        a.iter()
                            .map(|v| v.as_integer().and_then(|i| usize::try_from(i).ok()))
                            .collect::<Option<Vec<usize>>>()
                    });
                    match places.flatten() {
                        Some(p) => order = Some((at, p)),
                        None => self.err(at, "edgeOrder is a list of the edges' places, from 0"),
                    }
                }
                "@nodes" => raw_nodes = Some(self.value(item, false)),
                "@edges" => raw_edges = Some(self.value(item, false)),
                other => {
                    let value = self.value(item, false);
                    out.insert(unescape(other).to_owned(), value);
                }
            }
        }
        let mut edges = self.edges(&nodes);
        edges.extend(whole);
        if let Some((at, places)) = order {
            let mut sorted = places.clone();
            sorted.sort_unstable();
            if sorted == (0..edges.len()).collect::<Vec<_>>() {
                edges = places.iter().map(|&p| edges[p].clone()).collect();
            } else {
                self.err(
                    at,
                    format!("edgeOrder places each of the {} edges once", edges.len()),
                );
            }
        }
        let nodes = nodes.into_iter().map(|(_, node, _)| node).collect();
        out.insert("nodes".into(), raw_nodes.unwrap_or(Value::Array(nodes)));
        out.insert("edges".into(), raw_edges.unwrap_or(Value::Array(edges)));
        Value::Object(out)
    }

    /// The edges the steps' `needs` say, in step order.
    fn edges(&mut self, nodes: &[(Option<usize>, Value, Needs)]) -> Vec<Value> {
        let ids: Vec<Option<&str>> = nodes
            .iter()
            .map(|(_, n, _)| n.get("id").and_then(Value::as_str))
            .collect();
        let mut edges = Vec::new();
        let mut previous: Option<&str> = None;
        for (i, (at, _, needs)) in nodes.iter().enumerate() {
            let target = ids[i];
            let mut push = |source: &str, branch: Option<&String>, edge_id: Option<&String>| {
                let Some(target) = target else { return };
                let mut e = Map::new();
                let id = edge_id.cloned().unwrap_or_else(|| default_id(source, target));
                e.insert("id".into(), Value::String(id));
                e.insert("source".into(), Value::String(source.to_owned()));
                e.insert("target".into(), Value::String(target.to_owned()));
                if let Some(b) = branch {
                    e.insert("conditionBranch".into(), Value::String(b.clone()));
                }
                edges.push(Value::Object(e));
            };
            match needs {
                Needs::Before => {
                    if let Some(p) = previous {
                        push(p, None, None);
                    }
                }
                Needs::List(list) => {
                    for (source, branch, edge_id) in list {
                        push(source, branch.as_ref(), edge_id.as_ref());
                    }
                    if target.is_none() && !list.is_empty() {
                        self.err(*at, "a step that needs another has an id");
                    }
                    for (source, _, _) in list {
                        if !ids.contains(&Some(source.as_str())) {
                            self.err(*at, format!("needs `{source}`, which no step here is"));
                        }
                    }
                }
            }
            previous = target;
        }
        edges
    }

    fn edge(&mut self, table: &Table) -> Value {
        let mut out = Map::new();
        for (key, item) in table.iter() {
            let at = Self::key_at(table, key);
            if let Some((_, stored)) = EDGE_KEYS.iter().find(|(file, _)| *file == key) {
                match item.as_str() {
                    Some(s) => {
                        out.insert((*stored).to_owned(), Value::String(s.to_owned()));
                    }
                    None => self.err(at, format!("an edge's {key} is a string")),
                }
                continue;
            }
            let value = self.value(item, false);
            out.insert(unescape(key).to_owned(), value);
        }
        Value::Object(out)
    }

    /// A step or the trigger, and what it needs.
    fn node(&mut self, table: &Table, trigger: bool) -> (Value, Needs) {
        let mut out = Map::new();
        let mut config = Map::new();
        let mut trigger_type: Option<Value> = None;
        let mut explicit: Option<(Option<usize>, Option<Value>)> = None;
        let mut position: (Option<Value>, Option<Value>) = (None, None);
        let mut needs = Needs::Before;
        for (key, item) in table.iter() {
            let at = Self::key_at(table, key);
            match key {
                "id" | "label" | "slug" | "onError" | "type" => match item.as_str() {
                    Some(s) if trigger && key == "type" => {
                        trigger_type = Some(Value::String(s.to_owned()));
                    }
                    Some(s) => {
                        out.insert(key.into(), Value::String(s.to_owned()));
                    }
                    None => self.err(at, format!("{key} is a string")),
                },
                "x" | "y" => match self.number(item) {
                    Some(n) => {
                        let slot = if key == "x" {
                            &mut position.0
                        } else {
                            &mut position.1
                        };
                        *slot = Some(Value::Number(n));
                    }
                    None => self.err(at, format!("{key} is a number")),
                },
                "needs" => needs = Needs::List(self.needs(item, at)),
                "@config" => {
                    let absent = table_of(item).is_some_and(|t| {
                        t.len() == 1 && t.get("@json").and_then(Item::as_str) == Some("")
                    });
                    let value = (!absent).then(|| self.value(item, true));
                    explicit = Some((at, value));
                }
                field if field.starts_with('@') => {
                    let value = self.value(item, false);
                    out.insert(unescape(field).to_owned(), value);
                }
                setting => {
                    if let Some(field) = near(setting) {
                        self.hints.push(FileError {
                            at: line_col(self.text, at.unwrap_or(0)),
                            message: format!(
                                "`{setting}` is read as a setting of the step; did you mean {field}?"
                            ),
                        });
                    }
                    let value = self.value(item, true);
                    config.insert(setting.to_owned(), value);
                }
            }
        }
        let header = table.span().map(|s| s.start);
        if trigger {
            out.insert("type".into(), Value::String("trigger".into()));
            match trigger_type {
                Some(t) => {
                    let mut typed = Map::new();
                    typed.insert("triggerType".into(), t);
                    typed.extend(config);
                    config = typed;
                }
                None => self.err(header, "a trigger says its type, as type = \"manual\""),
            }
        }
        match position {
            (Some(x), Some(y)) => {
                let mut p = Map::new();
                p.insert("x".into(), x);
                p.insert("y".into(), y);
                out.insert("position".into(), Value::Object(p));
            }
            (None, None) => {}
            _ => self.err(header, "a step has both x and y, or neither"),
        }
        match explicit {
            Some((at, _)) if !config.is_empty() => self.err(
                at,
                "a step with \"@config\" keeps all its settings there, and none beside it",
            ),
            Some((_, Some(c))) => {
                out.insert("config".into(), c);
            }
            Some((_, None)) => {}
            None => {
                out.insert("config".into(), Value::Object(config));
            }
        }
        (Value::Object(in_step_order(out)), needs)
    }

    /// `needs = ["fetch", "check:true"]`: each word, `source[:branch][#id]`.
    fn needs(&mut self, item: &Item, at: Option<usize>) -> Vec<Need> {
        let Some(list) = item.as_array() else {
            self.err(
                at,
                "needs is a list of the steps it needs, as [\"fetch\", \"check:true\"]",
            );
            return Vec::new();
        };
        let mut out = Vec::new();
        for v in list {
            let here = v.span().map(|s| s.start).or(at);
            match v.as_str().map(token) {
                Some(Ok(need)) => out.push(need),
                Some(Err(bad)) => self.err(
                    here,
                    format!("needs names steps, as \"fetch\" or \"check:true\"; `{bad}` is not one"),
                ),
                None => self.err(here, "needs names steps by their id, as strings"),
            }
        }
        out
    }

    fn number(&mut self, item: &Item) -> Option<Number> {
        match item.as_value()? {
            toml_edit::Value::Integer(i) => Some(Number::from(*i.value())),
            toml_edit::Value::Float(f) => Number::from_f64(*f.value()),
            _ => None,
        }
    }

    /// An item as JSON. `config` marks a step's settings, where tags name
    /// a connection or a path in the project.
    fn value(&mut self, item: &Item, config: bool) -> Value {
        match item {
            Item::None => Value::Null,
            Item::Value(v) => self.plain(v, config),
            Item::Table(t) => self.object(t, config),
            Item::ArrayOfTables(a) => {
                Value::Array(a.iter().map(|t| self.object(t, config)).collect())
            }
        }
    }

    fn plain(&mut self, v: &toml_edit::Value, config: bool) -> Value {
        let at = v.span().map(|s| s.start);
        match v {
            toml_edit::Value::String(s) => Value::String(s.value().clone()),
            toml_edit::Value::Integer(i) => Value::from(*i.value()),
            toml_edit::Value::Float(f) => match Number::from_f64(*f.value()) {
                Some(n) => Value::Number(n),
                None => {
                    self.err(at, "a number JSON cannot hold");
                    Value::Null
                }
            },
            toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
            toml_edit::Value::Datetime(d) => Value::String(d.value().to_string()),
            toml_edit::Value::Array(a) => {
                Value::Array(a.iter().map(|v| self.plain(v, config)).collect())
            }
            toml_edit::Value::InlineTable(t) => self.object(&t.clone().into_table(), config),
        }
    }

    /// A table as an object, or the value its one tag stands for.
    fn object(&mut self, table: &Table, config: bool) -> Value {
        let mut iter = table.iter();
        if let (Some((tag, item)), None) = (iter.next(), iter.next()) {
            if TAGS.contains(&tag) {
                return self.tag(tag, item, Self::key_at(table, tag), config);
            }
        }
        let mut map = Map::new();
        for (key, item) in table.iter() {
            let value = self.value(item, config);
            map.insert(unescape(key).to_owned(), value);
        }
        Value::Object(map)
    }

    fn tag(&mut self, tag: &str, item: &Item, at: Option<usize>, config: bool) -> Value {
        let Some(text) = item.as_str() else {
            self.err(at, format!("{tag} holds a string"));
            return Value::Null;
        };
        if tag == "@json" {
            return match serde_json::from_str(text) {
                Ok(v) => v,
                Err(e) => {
                    self.err(at, format!("@json holds JSON: {e}"));
                    Value::Null
                }
            };
        }
        if !config {
            self.err(
                at,
                format!("{tag} names something only in a step's settings"),
            );
            return Value::Null;
        }
        let name = text.to_owned();
        match tag {
            "@connection" => {
                let found: Vec<&(String, String)> = self
                    .bindings
                    .connections
                    .iter()
                    .filter(|(_, n)| *n == name)
                    .collect();
                match found.as_slice() {
                    [(id, _)] => Value::String(id.clone()),
                    [] => {
                        self.warn(at, format!("no connection here is called {name}"));
                        Value::String(name)
                    }
                    _ => {
                        self.warn(at, format!("more than one connection here is called {name}"));
                        Value::String(name)
                    }
                }
            }
            "@project" => match self.bindings.project {
                Some((_, path)) if name == "." => Value::String(path.to_owned()),
                Some((_, path)) => Value::String(format!("{path}/{name}")),
                None => Value::String(name),
            },
            _ => match self.bindings.project {
                Some((project, _)) => Value::String(project.to_owned()),
                None => Value::String(name),
            },
        }
    }

    fn warn(&mut self, at: Option<usize>, message: String) {
        self.warnings.push(FileError {
            at: line_col(self.text, at.unwrap_or(0)),
            message,
        });
    }
}

/// A step's keys in the order the editor and the seeds always stored them, the rest after.
fn in_step_order(mut step: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for k in [
        "id", "type", "label", "slug", "onError", "position", "config",
    ] {
        if let Some(v) = step.shift_remove(k) {
            out.insert(k.to_owned(), v);
        }
    }
    out.extend(step);
    out
}

/// `source[:branch][#id]`.
fn token(word: &str) -> Result<Need, String> {
    let (head, id) = match word.split_once('#') {
        Some((h, id)) if !id.is_empty() => (h, Some(id.to_owned())),
        Some(_) => return Err(word.to_owned()),
        None => (word, None),
    };
    let (source, branch) = match head.split_once(':') {
        Some((s, b)) if !b.is_empty() => (s, Some(b.to_owned())),
        Some(_) => return Err(word.to_owned()),
        None => (head, None),
    };
    if source.is_empty() || source.contains(char::is_whitespace) {
        return Err(word.to_owned());
    }
    Ok((source.to_owned(), branch, id))
}

/// One step or trigger of a file, where it is: what a check of its type reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outlined {
    pub trigger: bool,
    /// The step's type, or the trigger's.
    pub kind: Option<String>,
    pub at: (usize, usize),
}

/// The steps and trigger of a file that parses, in order, each where it is.
pub fn outline(text: &str) -> Vec<Outlined> {
    let Ok(doc) = Document::parse(text) else {
        return Vec::new();
    };
    let root = doc.as_table();
    let at = |t: &Table| line_col(text, t.span().map_or(0, |s| s.start));
    let text_of = |t: &Table, k: &str| t.get(k).and_then(Item::as_str).map(str::to_owned);
    let mut out = Vec::new();
    if let Some(t) = root.get("trigger").and_then(table_of) {
        out.push(Outlined {
            trigger: true,
            kind: text_of(&t, "type"),
            at: at(&t),
        });
    }
    for t in root.get("step").and_then(tables_of).unwrap_or_default() {
        let kind = text_of(&t, "type");
        let trigger = kind.as_deref() == Some("trigger");
        out.push(Outlined {
            trigger,
            kind: if trigger {
                text_of(&t, "triggerType")
            } else {
                kind
            },
            at: at(&t),
        });
    }
    out
}
