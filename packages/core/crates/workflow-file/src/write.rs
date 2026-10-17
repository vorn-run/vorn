//! A stored workflow written as a file.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde_json::{Map, Number, Value};

use crate::{Bindings, CONNECTION_KEYS, RUNTIME_FIELDS};

/// Keys of the file itself that a workflow's own field of that name must not be read as.
pub(crate) const RESERVED: [&str; 4] = ["trigger", "step", "edge", "edgeOrder"];

/// A step's own fields in its table; every other key is one of its settings.
pub(crate) const STEP_FIELDS: [&str; 8] =
    ["id", "type", "label", "slug", "onError", "x", "y", "needs"];

/// An edge's keys in its table, and the stored fields they stand for.
pub(crate) const EDGE_KEYS: [(&str, &str); 4] = [
    ("id", "id"),
    ("from", "source"),
    ("to", "target"),
    ("branch", "conditionBranch"),
];

/// Single-key tables that stand for a value rather than holding one.
pub(crate) const TAGS: [&str; 4] = ["@json", "@connection", "@project", "@project-name"];

/// The file `definition` (a stored workflow) writes as. Its fields come
/// first, then its trigger and steps in order, each naming the steps it
/// needs when they are not simply the one before it.
pub fn write(definition: &Value, bindings: &Bindings<'_>) -> String {
    let empty = Map::new();
    let wf = definition.as_object().unwrap_or(&empty);
    let w = Writer { bindings };
    let mut top = Table::default();
    if let Some(name) = wf.get("name") {
        top.push(&w, "name".to_owned(), name, false);
    }
    for (key, value) in wf {
        let skip = matches!(key.as_str(), "name" | "nodes" | "edges")
            || RUNTIME_FIELDS.contains(&key.as_str());
        if !skip {
            top.push(&w, escaped(key, &RESERVED), value, false);
        }
    }
    let nodes = wf.get("nodes");
    let edges = wf.get("edges");
    let mut graph = String::new();
    match (
        nodes.and_then(Value::as_array),
        edges.and_then(Value::as_array),
    ) {
        (Some(nodes), Some(edges)) if nodes.iter().all(Value::is_object) => {
            if let Some(order) = w.graph(&mut graph, nodes, edges) {
                top.inline.push(format!("edgeOrder = {order}"));
            }
        }
        _ => {
            if let Some(n) = nodes {
                top.push(&w, "@nodes".to_owned(), n, false);
            }
            if let Some(e) = edges {
                top.push(&w, "@edges".to_owned(), e, false);
            }
        }
    }
    let mut out = String::new();
    top.write(&w, &mut out, &[]);
    out.push_str(&graph);
    out
}

/// `@key` for a key that would read as something else here.
pub(crate) fn escaped(key: &str, reserved: &[&str]) -> String {
    if reserved.contains(&key) || key.starts_with('@') {
        format!("@{key}")
    } else {
        key.to_owned()
    }
}

/// An edge as `needs` can say it.
struct EdgeOf<'a> {
    id: Option<&'a str>,
    source: &'a str,
    target: &'a str,
    branch: Option<&'a str>,
}

impl<'a> EdgeOf<'a> {
    /// The edge, when it holds nothing but these four, each a word.
    fn of(edge: &'a Value) -> Option<EdgeOf<'a>> {
        let map = edge.as_object()?;
        if map
            .keys()
            .any(|k| !matches!(k.as_str(), "id" | "source" | "target" | "conditionBranch"))
        {
            return None;
        }
        let text = |k: &str| match map.get(k) {
            None => Some(None),
            Some(Value::String(s)) if is_token(s) => Some(Some(s.as_str())),
            Some(_) => None,
        };
        Some(EdgeOf {
            id: text("id")?,
            source: text("source")??,
            target: text("target")??,
            branch: text("conditionBranch")?,
        })
    }

    /// `source[:branch][#id]`, the id only when it is not the usual one.
    fn token(&self) -> Option<String> {
        let mut out = self.source.to_owned();
        if let Some(b) = self.branch {
            out.push(':');
            out.push_str(b);
        }
        match self.id {
            Some(id) if id == default_id(self.source, self.target) => {}
            Some(id) => {
                out.push('#');
                out.push_str(id);
            }
            // An edge without an id is written out whole.
            None => return None,
        }
        Some(out)
    }
}

/// The id an edge has unless the file says otherwise.
pub(crate) fn default_id(source: &str, target: &str) -> String {
    format!("{source}-{target}")
}

/// A word `needs` can hold: no space, and none of `:#`.
pub(crate) fn is_token(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c == ':' || c == '#')
}

struct Writer<'b> {
    bindings: &'b Bindings<'b>,
}

/// One table being written: its `key = value` lines, then the tables under it.
#[derive(Default)]
struct Table<'v> {
    inline: Vec<String>,
    /// Objects as `[path.key]`, lists of objects as `[[path.key]]`.
    nested: Vec<(String, &'v Value, bool)>,
}

impl<'v> Table<'v> {
    /// `key` and its value: on a line of its own, or as a table under this one.
    fn push(&mut self, w: &Writer<'_>, key: String, value: &'v Value, config: bool) {
        let tables = match value {
            Value::Object(map) => !map.is_empty() && !is_tag_shaped(map),
            Value::Array(items) => !items.is_empty() && items.iter().all(table_shaped),
            _ => false,
        };
        if tables {
            self.nested.push((key, value, config));
        } else {
            let text = w.inline(&key, value, config);
            self.inline.push(format!("{} = {text}", bare(&key)));
        }
    }

    /// The lines, then each table under this one at `path`.
    fn write(&self, w: &Writer<'_>, out: &mut String, path: &[String]) {
        for line in &self.inline {
            out.push_str(line);
            out.push('\n');
        }
        for (key, value, config) in &self.nested {
            let mut here = path.to_vec();
            here.push(key.clone());
            let header = here.iter().map(|k| bare(k)).collect::<Vec<_>>().join(".");
            let (open, close) = if value.is_array() {
                ("[[", "]]")
            } else {
                ("[", "]")
            };
            let maps: Vec<&Map<String, Value>> = match value {
                Value::Object(map) => vec![map],
                Value::Array(items) => items.iter().filter_map(Value::as_object).collect(),
                _ => Vec::new(),
            };
            for map in maps {
                let _ = write!(out, "\n{open}{header}{close}\n");
                let mut t = Table::default();
                for (k, v) in map {
                    t.push(w, escaped(k, &[]), v, *config);
                }
                t.write(w, out, &here);
            }
        }
    }
}

/// An object a reader would take for a tag: one key, a tag's name.
fn is_tag_shaped(map: &Map<String, Value>) -> bool {
    map.len() == 1 && map.keys().all(|k| TAGS.contains(&k.as_str()))
}

/// What can be written as a table of its own.
fn table_shaped(v: &Value) -> bool {
    v.as_object().is_some_and(|m| !is_tag_shaped(m))
}

impl Writer<'_> {
    /// Writes the trigger and steps; returns `edgeOrder` when edges are stored out of step order.
    fn graph(&self, out: &mut String, nodes: &[Value], edges: &[Value]) -> Option<String> {
        let ids: Vec<Option<&str>> = nodes
            .iter()
            .map(|n| n.get("id").and_then(Value::as_str))
            .collect();
        // Edges into each node, in their order; the rest are written whole.
        let mut into: HashMap<&str, Vec<(usize, String)>> = HashMap::new();
        let mut whole: Vec<usize> = Vec::new();
        for (i, e) in edges.iter().enumerate() {
            let token = EdgeOf::of(e).and_then(|edge| {
                let token = edge.token()?;
                (ids.contains(&Some(edge.target)) && ids.contains(&Some(edge.source)))
                    .then_some((edge.target, token))
            });
            match token {
                Some((target, token)) => into.entry(target).or_default().push((i, token)),
                None => whole.push(i),
            }
        }
        // What a reader rebuilds: each node's needs, in node order, then the whole edges.
        let mut rebuilt: Vec<usize> = Vec::new();
        let mut previous: Option<&str> = None;
        for (n, node) in nodes.iter().enumerate() {
            let id = ids[n];
            let needs = id.and_then(|id| into.get(id));
            // A single plain edge from the node before needs no saying.
            let implicit = match (previous, needs) {
                (Some(p), Some(list)) => list.len() == 1 && list[0].1 == p,
                _ => false,
            };
            let needs_list = match needs {
                _ if implicit => None,
                Some(list) => Some(list.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>()),
                None if n > 0 => Some(Vec::new()),
                None => None,
            };
            if let Some(list) = needs {
                rebuilt.extend(list.iter().map(|(i, _)| *i));
            }
            self.node(out, node, n == 0, needs_list);
            previous = id;
        }
        for &i in &whole {
            self.edge(out, &edges[i]);
            rebuilt.push(i);
        }
        if rebuilt.iter().enumerate().all(|(at, i)| at == *i) {
            return None;
        }
        let mut place = vec![0usize; rebuilt.len()];
        for (at, i) in rebuilt.iter().enumerate() {
            place[*i] = at;
        }
        let list: Vec<String> = place.iter().map(ToString::to_string).collect();
        Some(format!("[{}]", list.join(", ")))
    }

    /// One node as `[trigger]` (a typed trigger first) or `[[step]]`: its fields, then its settings.
    fn node(&self, out: &mut String, node: &Value, first: bool, needs: Option<Vec<&str>>) {
        let empty = Map::new();
        let map = node.as_object().unwrap_or(&empty);
        let config = map.get("config");
        let flat = config.and_then(Value::as_object).filter(|c| {
            c.keys()
                .all(|k| !STEP_FIELDS.contains(&k.as_str()) && !k.starts_with('@'))
        });
        let trigger_type = flat
            .and_then(|c| c.get("triggerType"))
            .and_then(Value::as_str)
            .filter(|_| first && map.get("type").and_then(Value::as_str) == Some("trigger"));
        let mut t = Table::default();
        if let Some(Value::String(id)) = map.get("id") {
            t.inline.push(format!("id = {}", string(id)));
        }
        if let Some(kind) = trigger_type.or_else(|| map.get("type").and_then(Value::as_str)) {
            t.inline.push(format!("type = {}", string(kind)));
        }
        for key in ["label", "slug", "onError"] {
            if let Some(Value::String(s)) = map.get(key) {
                t.inline.push(format!("{key} = {}", string(s)));
            }
        }
        let place = map.get("position").and_then(position);
        if let Some((x, y)) = &place {
            t.inline.push(format!("x = {x}"));
            t.inline.push(format!("y = {y}"));
        }
        if let Some(list) = needs {
            let words: Vec<String> = list.iter().map(|w| string(w)).collect();
            t.inline.push(format!("needs = [{}]", words.join(", ")));
        }
        for (key, value) in map {
            let said = match key.as_str() {
                "id" | "label" | "slug" | "onError" | "type" => value.is_string(),
                "position" => place.is_some(),
                "config" => true,
                _ => false,
            };
            if !said {
                t.push(self, format!("@{key}"), value, false);
            }
        }
        match (config, flat) {
            (_, Some(c)) => {
                for (k, v) in c {
                    if trigger_type.is_none() || k != "triggerType" {
                        t.push(self, k.clone(), v, true);
                    }
                }
            }
            (Some(other), None) => t.push(self, "@config".to_owned(), other, true),
            // No settings at all, which a table without any would read as empty ones.
            (None, None) => t.inline.push(format!("\"@config\" = {}", json_tag(""))),
        }
        let path = if trigger_type.is_some() {
            out.push_str("\n[trigger]\n");
            "trigger"
        } else {
            out.push_str("\n[[step]]\n");
            "step"
        };
        t.write(self, out, &[path.to_owned()]);
    }

    /// An edge `needs` cannot say.
    fn edge(&self, out: &mut String, edge: &Value) {
        let empty = Map::new();
        let map = edge.as_object().unwrap_or(&empty);
        let mut t = Table::default();
        for (k, v) in map {
            let named = EDGE_KEYS.iter().find(|(_, stored)| stored == k);
            match (named, v) {
                (Some((key, _)), Value::String(s)) => {
                    t.inline.push(format!("{key} = {}", string(s)));
                }
                _ => {
                    let taken = EDGE_KEYS.iter().any(|(file, _)| file == k);
                    let key = if taken || k.starts_with('@') {
                        format!("@{k}")
                    } else {
                        k.clone()
                    };
                    t.push(self, key, v, false);
                }
            }
        }
        out.push_str("\n[[edge]]\n");
        t.write(self, out, &["edge".to_owned()]);
    }

    /// A value written inline; `config` marks a step's settings, whose connections and paths go by name.
    fn inline(&self, key: &str, value: &Value, config: bool) -> String {
        match value {
            Value::Null => json_tag("null"),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => number(n).unwrap_or_else(|| json_tag(&n.to_string())),
            Value::String(s) => {
                let bare = key.strip_prefix('@').unwrap_or(key);
                config
                    .then(|| self.bound(bare, s))
                    .flatten()
                    .unwrap_or_else(|| string(s))
            }
            Value::Array(items) if items.is_empty() => "[]".to_owned(),
            Value::Array(items) => {
                let parts: Vec<String> = items.iter().map(|v| self.inline("", v, config)).collect();
                let long = parts.iter().map(String::len).sum::<usize>() > 60
                    || items.iter().any(Value::is_object)
                    || parts.iter().any(|p| p.contains('\n'));
                if !long {
                    return format!("[{}]", parts.join(", "));
                }
                let mut out = String::from("[\n");
                for p in parts {
                    let _ = writeln!(out, "    {p},");
                }
                out.push(']');
                out
            }
            Value::Object(map) if map.is_empty() => "{}".to_owned(),
            Value::Object(map) => {
                let parts: Vec<String> = map
                    .iter()
                    .map(|(k, v)| {
                        let k = escaped(k, &[]);
                        format!("{} = {}", bare(&k), self.inline(&k, v, config))
                    })
                    .collect();
                format!("{{ {} }}", parts.join(", "))
            }
        }
    }

    /// A connection's id as its name, a path in the project by its place in it.
    fn bound(&self, key: &str, s: &str) -> Option<String> {
        if CONNECTION_KEYS.contains(&key) {
            let conns = self.bindings.connections;
            let named: Vec<&(String, String)> = conns.iter().filter(|(id, _)| id == s).collect();
            if let [(_, name)] = named.as_slice() {
                if conns.iter().filter(|(_, n)| n == name).count() == 1 {
                    return Some(format!("{{ \"@connection\" = {} }}", string(name)));
                }
            }
        }
        let (name, path) = self.bindings.project?;
        if s == path {
            return Some("{ \"@project\" = \".\" }".to_owned());
        }
        if let Some(rest) = s.strip_prefix(path).and_then(|r| r.strip_prefix('/')) {
            if !rest.is_empty() {
                return Some(format!("{{ \"@project\" = {} }}", string(rest)));
            }
        }
        (key == "projectName" && s == name)
            .then(|| format!("{{ \"@project-name\" = {} }}", string(name)))
    }
}

/// `{ "@json" = "<text>" }`: a value TOML has no word for.
fn json_tag(text: &str) -> String {
    format!("{{ \"@json\" = {} }}", string(text))
}

/// `{x, y}` of two numbers TOML holds, and nothing else.
fn position(p: &Value) -> Option<(String, String)> {
    let map = p.as_object()?;
    if map.len() != 2 {
        return None;
    }
    let x = number(map.get("x")?.as_number()?)?;
    let y = number(map.get("y")?.as_number()?)?;
    Some((x, y))
}

/// A number as TOML writes it; `None` past TOML's 64-bit signed integers.
fn number(n: &Number) -> Option<String> {
    if let Some(i) = n.as_i64() {
        return Some(i.to_string());
    }
    if n.is_u64() {
        return None;
    }
    let text = format!("{:?}", n.as_f64()?);
    Some(if text.contains(['.', 'e', 'E']) {
        text
    } else {
        format!("{text}.0")
    })
}

/// A key: bare when TOML reads it as the same word, else quoted.
pub(crate) fn bare(key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if plain {
        key.to_owned()
    } else {
        basic(key)
    }
}

/// A character no TOML string holds as it is.
fn is_control(c: char) -> bool {
    (c < ' ' && c != '\t') || c == '\u{7f}'
}

/// A string, across lines when it has several, literal when that spares escapes.
pub(crate) fn string(s: &str) -> String {
    if !s.contains('\n') {
        let worth = s.contains('\\') || s.contains('"');
        if worth && !s.contains('\'') && !s.chars().any(is_control) {
            return format!("'{s}'");
        }
        return basic(s);
    }
    let literal = s.contains('\\')
        && !s.contains("'''")
        && !s.ends_with('\'')
        && !s.chars().any(|c| is_control(c) && c != '\n');
    if literal {
        return format!("'''\n{s}'''");
    }
    let mut out = String::with_capacity(s.len() + 8);
    out.push_str("\"\"\"\n");
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            // Never two quotes in a row, so none closes the string early.
            '"' if chars.peek().is_none_or(|n| *n == '"') => out.push_str("\\\""),
            '\n' => out.push('\n'),
            '\r' => out.push_str("\\r"),
            c if is_control(c) => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push_str("\"\"\"");
    out
}

/// A basic string on one line: `"…"` with escapes.
fn basic(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if is_control(c) => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
