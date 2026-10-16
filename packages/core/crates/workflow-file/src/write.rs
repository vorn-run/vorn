//! A stored workflow written as a file.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::{Bindings, CONNECTION_KEYS, RUNTIME_FIELDS};

/// Children of a `workflow` node that are not one of its fields.
pub(crate) const RESERVED: [&str; 4] = ["trigger", "step", "edge", "edge-order"];

const INDENT: &str = "    ";

/// The file `definition` (a stored workflow) writes as. Its fields come
/// first, then its nodes in order, each naming the steps it needs when they
/// are not simply the one before it.
pub fn write(definition: &Value, bindings: &Bindings<'_>) -> String {
    let mut out = String::new();
    let empty = Map::new();
    let wf = definition.as_object().unwrap_or(&empty);
    let name = wf.get("name").and_then(Value::as_str);
    out.push_str("workflow");
    if let Some(name) = name {
        out.push(' ');
        out.push_str(&string(name, 0));
    }
    out.push_str(" {\n");
    let w = Writer { bindings };
    for (key, value) in wf {
        let plain_name = key == "name" && value.is_string();
        if plain_name || key == "nodes" || key == "edges" || RUNTIME_FIELDS.contains(&key.as_str())
        {
            continue;
        }
        w.value_node(&mut out, &field_key(key, &RESERVED), value, 1, false);
    }
    let nodes = wf.get("nodes");
    let edges = wf.get("edges");
    match (
        nodes.and_then(Value::as_array),
        edges.and_then(Value::as_array),
    ) {
        (Some(nodes), Some(edges)) if nodes.iter().all(Value::is_object) => {
            w.graph(&mut out, nodes, edges);
        }
        _ => {
            if let Some(n) = nodes {
                w.value_node(&mut out, "@nodes", n, 1, false);
            }
            if let Some(e) = edges {
                w.value_node(&mut out, "@edges", e, 1, false);
            }
        }
    }
    out.push_str("}\n");
    out
}

/// `@key` for a key that would read as something else here.
pub(crate) fn field_key(key: &str, reserved: &[&str]) -> String {
    if reserved.contains(&key) || key.starts_with('@') || key == "-" {
        format!("@{key}")
    } else {
        key.to_owned()
    }
}

/// An edge as the file can say it.
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

/// A word `needs=` can hold: no space, and none of `:#`.
pub(crate) fn is_token(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c == ':' || c == '#')
}

struct Writer<'b> {
    bindings: &'b Bindings<'b>,
}

impl Writer<'_> {
    fn graph(&self, out: &mut String, nodes: &[Value], edges: &[Value]) {
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
                ids.contains(&Some(edge.target))
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
            let needs_attr = match (needs, id) {
                _ if implicit => None,
                (Some(list), _) => Some(Some(
                    list.iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                )),
                (None, _) if n > 0 => Some(None),
                (None, _) => None,
            };
            if let Some(list) = needs {
                rebuilt.extend(list.iter().map(|(i, _)| *i));
            }
            self.node(out, node, needs_attr);
            previous = id;
        }
        for &i in &whole {
            self.edge(out, &edges[i]);
            rebuilt.push(i);
        }
        // Edges stored out of node order keep their order in an `edge-order` line.
        let in_order = rebuilt.iter().enumerate().all(|(at, i)| at == *i);
        if !in_order {
            let mut place = vec![0usize; rebuilt.len()];
            for (at, i) in rebuilt.iter().enumerate() {
                place[*i] = at;
            }
            out.push_str(INDENT);
            out.push_str("edge-order");
            for p in place {
                let _ = write!(out, " {p}");
            }
            out.push('\n');
        }
    }

    /// One node: `trigger` or `step "<type>"`, its id, label, slug, failure
    /// rule, place and needs as properties, its config as its children.
    fn node(&self, out: &mut String, node: &Value, needs: Option<Option<String>>) {
        let empty = Map::new();
        let map = node.as_object().unwrap_or(&empty);
        let kind = map.get("type").and_then(Value::as_str);
        let config = map.get("config");
        let trigger = kind == Some("trigger");
        out.push_str(INDENT);
        let mut config_rest: Option<Map<String, Value>> = None;
        if trigger {
            out.push_str("trigger");
            match config.and_then(Value::as_object) {
                Some(c) if c.get("triggerType").is_some_and(Value::is_string) => {
                    out.push(' ');
                    out.push_str(&string(c["triggerType"].as_str().unwrap_or_default(), 1));
                    let mut rest = c.clone();
                    rest.shift_remove("triggerType");
                    config_rest = Some(rest);
                }
                _ => {}
            }
        } else {
            out.push_str("step");
            if let Some(k) = kind {
                out.push(' ');
                out.push_str(&string(k, 1));
            }
        }
        let mut extra: Vec<(&String, &Value)> = Vec::new();
        for (key, value) in map {
            match (key.as_str(), value) {
                ("type", Value::String(_)) => {}
                ("config", _) => {}
                ("id" | "label" | "slug" | "onError", Value::String(s)) => {
                    let _ = write!(out, " {key}={}", string(s, 1));
                }
                ("position", p) if position(p).is_some() => {
                    let (x, y) = position(p).unwrap_or_default();
                    let _ = write!(out, " x={x} y={y}");
                }
                _ => extra.push((key, value)),
            }
        }
        match needs {
            Some(Some(list)) => {
                let _ = write!(out, " needs={}", string(&list, 1));
            }
            Some(None) => out.push_str(" needs=#null"),
            None => {}
        }
        let children: Option<Vec<(String, &Value)>> = match (config, &config_rest) {
            (_, Some(rest)) => Some(rest.iter().map(|(k, v)| (child_key(k), v)).collect()),
            (Some(Value::Object(c)), None) => {
                Some(c.iter().map(|(k, v)| (child_key(k), v)).collect())
            }
            _ => None,
        };
        let odd_config = match config {
            None => None,
            Some(Value::Object(_)) => None,
            Some(other) => Some(other),
        };
        let mut body = String::new();
        for (k, v) in &extra {
            self.value_node(&mut body, &format!("@{k}"), v, 2, false);
        }
        if let Some(c) = odd_config {
            self.value_node(&mut body, "@config", c, 2, false);
        }
        // An empty config beside other fields would read as none.
        let empty_beside = config_rest.is_none()
            && !extra.is_empty()
            && children.as_ref().is_some_and(Vec::is_empty);
        if empty_beside {
            self.value_node(&mut body, "@config", &Value::Object(Map::new()), 2, false);
        }
        if let Some(children) = &children {
            for (k, v) in children {
                self.value_node(&mut body, k, v, 2, true);
            }
        }
        // A trigger named by its type has a config already; an empty block says nothing more.
        let has_config_block = children.is_some() && config_rest.is_none();
        if body.is_empty() && !has_config_block {
            out.push('\n');
            return;
        }
        if body.is_empty() {
            out.push_str(" {}\n");
            return;
        }
        out.push_str(" {\n");
        out.push_str(&body);
        out.push_str(INDENT);
        out.push_str("}\n");
    }

    /// An edge the `needs=` of its target cannot say.
    fn edge(&self, out: &mut String, edge: &Value) {
        out.push_str(INDENT);
        out.push_str("edge");
        let empty = Map::new();
        let map = edge.as_object().unwrap_or(&empty);
        let mut extra = Vec::new();
        for (k, v) in map {
            let prop = match k.as_str() {
                "id" => "id",
                "source" => "from",
                "target" => "to",
                "conditionBranch" => "branch",
                _ => "",
            };
            match v {
                Value::String(s) if !prop.is_empty() => {
                    let _ = write!(out, " {prop}={}", string(s, 1));
                }
                _ => extra.push((k, v)),
            }
        }
        if extra.is_empty() {
            out.push('\n');
            return;
        }
        out.push_str(" {\n");
        for (k, v) in extra {
            self.value_node(out, &format!("@{k}"), v, 2, false);
        }
        out.push_str(INDENT);
        out.push_str("}\n");
    }

    /// `key value`, a block of keys for an object, a block of `-` items for
    /// a list. `config` marks a step's config, whose connections and paths
    /// are written by name.
    fn value_node(&self, out: &mut String, key: &str, value: &Value, depth: usize, config: bool) {
        for _ in 0..depth {
            out.push_str(INDENT);
        }
        match value {
            Value::Array(items) if items.is_empty() => {
                out.push_str("(list)");
                out.push_str(&ident(key));
                out.push('\n');
            }
            Value::Array(items) => {
                out.push_str(&ident(key));
                out.push_str(" {\n");
                for item in items {
                    self.value_node(out, "-", item, depth + 1, config);
                }
                indent(out, depth);
                out.push_str("}\n");
            }
            Value::Object(map) => {
                out.push_str(&ident(key));
                if map.is_empty() {
                    out.push_str(" {}\n");
                    return;
                }
                out.push_str(" {\n");
                for (k, v) in map {
                    self.value_node(out, &child_key(k), v, depth + 1, config);
                }
                indent(out, depth);
                out.push_str("}\n");
            }
            scalar => {
                out.push_str(&ident(key));
                out.push(' ');
                let bare = key.strip_prefix('@').unwrap_or(key);
                out.push_str(&self.scalar(bare, scalar, depth, config));
                out.push('\n');
            }
        }
    }

    fn scalar(&self, key: &str, value: &Value, depth: usize, config: bool) -> String {
        if let (true, Value::String(s)) = (config, value) {
            if CONNECTION_KEYS.contains(&key) {
                let named: Vec<&(String, String)> = self
                    .bindings
                    .connections
                    .iter()
                    .filter(|(id, _)| id == s)
                    .collect();
                if let [(_, name)] = named.as_slice() {
                    let unique = self
                        .bindings
                        .connections
                        .iter()
                        .filter(|(_, n)| n == name)
                        .count()
                        == 1;
                    if unique {
                        return format!("(connection){}", string(name, depth));
                    }
                }
            }
            if let Some((name, path)) = self.bindings.project {
                if s == path {
                    return "(project)\".\"".to_owned();
                }
                if let Some(rest) = s.strip_prefix(path).and_then(|r| r.strip_prefix('/')) {
                    if !rest.is_empty() {
                        return format!("(project){}", string(rest, depth));
                    }
                }
                if key == "projectName" && s == name {
                    return format!("(project-name){}", string(name, depth));
                }
            }
        }
        match value {
            Value::Null => "#null".into(),
            Value::Bool(b) => format!("#{b}"),
            Value::Number(n) => number(n),
            Value::String(s) => string(s, depth),
            _ => String::new(),
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str(INDENT);
    }
}

/// A key inside an object: `-` and anything starting with `@` take an `@`.
fn child_key(key: &str) -> String {
    field_key(key, &[])
}

/// `{x, y}` of two numbers and nothing else.
fn position(p: &Value) -> Option<(String, String)> {
    let map = p.as_object()?;
    if map.len() != 2 {
        return None;
    }
    let x = map.get("x")?.as_number()?;
    let y = map.get("y")?.as_number()?;
    Some((number(x), number(y)))
}

/// A number as KDL writes it: a float keeps its point.
pub(crate) fn number(n: &serde_json::Number) -> String {
    if n.is_i64() || n.is_u64() {
        return n.to_string();
    }
    let f = n.as_f64().unwrap_or(0.0);
    let text = format!("{f:?}");
    if text.contains('.') || text.contains("inf") || text.contains("NaN") {
        return text;
    }
    match text.split_once('e') {
        Some((m, e)) => format!("{m}.0e{e}"),
        None => format!("{text}.0"),
    }
}

/// A node name: bare when KDL reads it as the same word, else quoted.
pub(crate) fn ident(key: &str) -> String {
    let mut chars = key.chars();
    let bare = match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '@' => {
            chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'))
        }
        _ => false,
    } && !matches!(key, "true" | "false" | "null" | "inf" | "nan");
    if bare || key == "-" {
        key.to_owned()
    } else {
        string(key, 0)
    }
}

/// One character as a KDL escape, when it needs one.
fn escape(c: char, out: &mut String) {
    match c {
        '\\' => out.push_str("\\\\"),
        '"' => out.push_str("\\\""),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        c if (c as u32) < 0x20 || c == '\u{7f}' || is_newline(c) || is_invisible(c) => {
            let _ = write!(out, "\\u{{{:x}}}", c as u32);
        }
        c => out.push(c),
    }
}

/// Characters KDL reads as a line break.
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{85}' | '\u{b}' | '\u{c}' | '\u{2028}' | '\u{2029}'
    )
}

/// Characters KDL refuses to see in a document, and the byte order mark.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{feff}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Whitespace KDL would read away from the edge of a line of a multi-line string.
fn is_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

/// A string: quoted on one line, or across lines when it has several,
/// indented one deeper than its node at `depth`.
pub(crate) fn string(s: &str, depth: usize) -> String {
    if !s.contains('\n') {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            escape(c, &mut out);
        }
        out.push('"');
        return out;
    }
    let pad = INDENT.repeat(depth + 1);
    let mut out = String::from("\"\"\"\n");
    for line in s.split('\n') {
        if line.is_empty() {
            out.push('\n');
            continue;
        }
        out.push_str(&pad);
        let chars: Vec<char> = line.chars().collect();
        let first = chars.iter().position(|c| !is_space(*c));
        let last = chars.iter().rposition(|c| !is_space(*c));
        for (i, c) in chars.iter().enumerate() {
            // A line of nothing but space, and space at a line's end, would be read away.
            let edge = match (first, last) {
                (Some(_), Some(l)) => i > l,
                _ => true,
            };
            match c {
                ' ' if edge => out.push_str("\\s"),
                c if edge && is_space(*c) => {
                    let _ = write!(out, "\\u{{{:x}}}", *c as u32);
                }
                c => escape(*c, &mut out),
            }
        }
        out.push('\n');
    }
    // The closing line's indent is what each line loses as it is read.
    out.push_str(&pad);
    out.push_str("\"\"\"");
    out
}
