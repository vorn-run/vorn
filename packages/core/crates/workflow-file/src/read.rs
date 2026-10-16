//! A workflow file read into the stored workflow, every error at its line
//! and column.

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use serde_json::{Map, Number, Value};

use crate::write::default_id;
use crate::{Bindings, FileError};

/// A file read: the workflow, and what it names that this machine lacks.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub workflow: Value,
    /// A connection no connection here is called, kept by its name: the
    /// step that uses it fails until one is.
    pub warnings: Vec<FileError>,
}

/// Reads workflow file `text`, its connections and paths as `bindings` name them.
pub fn read(text: &str, bindings: &Bindings<'_>) -> Result<Read, Vec<FileError>> {
    let doc = KdlDocument::parse_v2(text).map_err(|e| {
        e.diagnostics
            .iter()
            .map(|d| FileError {
                at: line_col(text, d.span.offset()),
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
        bindings,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let workflow = r.document(&doc);
    if r.errors.is_empty() {
        Ok(Read {
            workflow,
            warnings: r.warnings,
        })
    } else {
        r.errors.sort_by_key(|e| e.at);
        Err(r.errors)
    }
}

/// Line and column, from 1, of byte `offset` of `text`.
pub(crate) fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// The node properties a step or trigger takes.
const NODE_PROPS: [&str; 7] = ["id", "label", "slug", "onError", "x", "y", "needs"];

/// The closest of `known` to `word`, when one is near.
fn near<'a>(word: &str, known: &[&'a str]) -> Option<&'a str> {
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
    known
        .iter()
        .map(|k| (distance(word, k), *k))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k)
}

/// A key as the file wrote it, back as the stored key: one `@` less.
fn unescape(name: &str) -> &str {
    name.strip_prefix('@').unwrap_or(name)
}

struct Reader<'a> {
    text: &'a str,
    bindings: &'a Bindings<'a>,
    errors: Vec<FileError>,
    warnings: Vec<FileError>,
}

/// One word of `needs=`: the step needed, the branch of it, and the edge's own id.
type Need = (String, Option<String>, Option<String>);

/// A step's edges as its `needs=` gives them.
enum Needs {
    /// None said: the step before.
    Before,
    /// `needs=#null`: none.
    Nothing,
    List(Vec<Need>),
}

impl Reader<'_> {
    fn err(&mut self, offset: usize, message: impl Into<String>) {
        self.errors.push(FileError {
            at: line_col(self.text, offset),
            message: message.into(),
        });
    }

    fn document(&mut self, doc: &KdlDocument) -> Value {
        let nodes = doc.nodes();
        let Some(first) = nodes.first() else {
            self.err(0, "a workflow file holds one workflow node");
            return Value::Null;
        };
        for extra in &nodes[1..] {
            self.err(
                extra.span().offset(),
                format!(
                    "`{}`: a workflow file holds one workflow node, and this is a second",
                    extra.name().value()
                ),
            );
        }
        if first.name().value() != "workflow" {
            self.err(
                first.name().span().offset(),
                format!(
                    "`{}`: a workflow file starts with `workflow \"Its name\" {{`",
                    first.name().value()
                ),
            );
            return Value::Null;
        }
        self.workflow(first)
    }

    fn workflow(&mut self, wf: &KdlNode) -> Value {
        let mut out = Map::new();
        for entry in wf.entries() {
            match (entry.name(), entry.value()) {
                (None, KdlValue::String(name)) if !out.contains_key("name") => {
                    out.insert("name".into(), Value::String(name.clone()));
                }
                _ => self.err(
                    entry.span().offset(),
                    "a workflow takes its name, and nothing else on its line",
                ),
            }
        }
        let mut nodes: Vec<Value> = Vec::new();
        let mut needs: Vec<(usize, Option<String>, Needs)> = Vec::new();
        let mut whole: Vec<Value> = Vec::new();
        let mut order: Option<(usize, Vec<usize>)> = None;
        let mut raw_nodes: Option<Value> = None;
        let mut raw_edges: Option<Value> = None;
        for child in wf.children().map(KdlDocument::nodes).unwrap_or_default() {
            let name = child.name().value();
            match name {
                "trigger" | "step" => {
                    let (node, said) = self.node(child, name == "trigger");
                    let id = node.get("id").and_then(Value::as_str).map(str::to_owned);
                    needs.push((child.span().offset(), id, said));
                    nodes.push(node);
                }
                "edge" => whole.push(self.edge(child)),
                "edge-order" => {
                    let places = child
                        .entries()
                        .iter()
                        .filter_map(|e| match e.value() {
                            KdlValue::Integer(i) => usize::try_from(*i).ok(),
                            _ => None,
                        })
                        .collect();
                    order = Some((child.span().offset(), places));
                }
                "@nodes" => raw_nodes = Some(self.value(child)),
                "@edges" => raw_edges = Some(self.value(child)),
                _ => {
                    let value = self.value(child);
                    out.insert(unescape(name).to_owned(), value);
                }
            }
        }
        let mut edges: Vec<Value> = Vec::new();
        let mut previous: Option<String> = None;
        let ids: Vec<Option<String>> = needs.iter().map(|(_, id, _)| id.clone()).collect();
        for (at, id, said) in needs {
            let target = id.clone();
            let mut push = |source: &str, branch: Option<String>, edge_id: Option<String>| {
                let Some(target) = &target else { return };
                let mut e = Map::new();
                e.insert(
                    "id".into(),
                    Value::String(edge_id.unwrap_or_else(|| default_id(source, target))),
                );
                e.insert("source".into(), Value::String(source.to_owned()));
                e.insert("target".into(), Value::String(target.clone()));
                if let Some(b) = branch {
                    e.insert("conditionBranch".into(), Value::String(b));
                }
                edges.push(Value::Object(e));
            };
            match said {
                Needs::Before => {
                    if let (Some(p), Some(_)) = (&previous, &id) {
                        push(p, None, None);
                    }
                }
                Needs::Nothing => {}
                Needs::List(list) => {
                    if id.is_none() {
                        self.err(at, "a step that needs another has an id=");
                    }
                    for (source, branch, edge_id) in list {
                        if !ids.iter().any(|i| i.as_deref() == Some(source.as_str())) {
                            self.err(at, format!("needs `{source}`, which no step here is"));
                        }
                        push(&source, branch, edge_id);
                    }
                }
            }
            previous = id;
        }
        edges.extend(whole);
        if let Some((at, places)) = order {
            let mut sorted = places.clone();
            sorted.sort_unstable();
            if sorted != (0..edges.len()).collect::<Vec<_>>() {
                self.err(
                    at,
                    format!("edge-order places each of the {} edges once", edges.len()),
                );
            } else {
                edges = places.iter().map(|&p| edges[p].clone()).collect();
            }
        }
        out.insert(
            "nodes".into(),
            raw_nodes.unwrap_or_else(|| Value::Array(nodes)),
        );
        out.insert(
            "edges".into(),
            raw_edges.unwrap_or_else(|| Value::Array(edges)),
        );
        Value::Object(out)
    }

    fn edge(&mut self, node: &KdlNode) -> Value {
        let mut out = Map::new();
        for entry in node.entries() {
            let key = match entry.name().map(|n| n.value()) {
                Some("id") => "id",
                Some("from") => "source",
                Some("to") => "target",
                Some("branch") => "conditionBranch",
                other => {
                    self.err(
                        entry.span().offset(),
                        format!(
                            "an edge takes from=, to=, id= and branch=, not {}",
                            other.unwrap_or("a value")
                        ),
                    );
                    continue;
                }
            };
            match entry.value().as_string() {
                Some(s) => {
                    out.insert(key.into(), Value::String(s.to_owned()));
                }
                None => self.err(entry.span().offset(), format!("{key} is a string")),
            }
        }
        for child in node.children().map(KdlDocument::nodes).unwrap_or_default() {
            let value = self.value(child);
            out.insert(unescape(child.name().value()).to_owned(), value);
        }
        Value::Object(out)
    }

    /// A step or the trigger, and what it needs.
    fn node(&mut self, node: &KdlNode, trigger: bool) -> (Value, Needs) {
        let mut out = Map::new();
        let mut config = Map::new();
        let mut typed = false;
        let mut position: (Option<Value>, Option<Value>) = (None, None);
        let mut needs = Needs::Before;
        for entry in node.entries() {
            let at = entry.span().offset();
            let Some(name) = entry.name().map(|n| n.value()) else {
                match (entry.value(), trigger, typed) {
                    (KdlValue::String(t), true, false) => {
                        config.insert("triggerType".into(), Value::String(t.clone()));
                        typed = true;
                    }
                    (KdlValue::String(t), false, false) => {
                        out.insert("type".into(), Value::String(t.clone()));
                        typed = true;
                    }
                    _ => self.err(
                        at,
                        if trigger {
                            "a trigger takes its type, as `trigger \"manual\"`, and properties"
                        } else {
                            "a step takes its type, as `step \"script\"`, and properties"
                        },
                    ),
                }
                continue;
            };
            match name {
                "id" | "label" | "slug" | "onError" => match entry.value().as_string() {
                    Some(s) => {
                        out.insert(name.into(), Value::String(s.to_owned()));
                    }
                    None => self.err(at, format!("{name}= is a string")),
                },
                "x" | "y" => match number(entry.value()) {
                    Some(n) => {
                        let slot = if name == "x" {
                            &mut position.0
                        } else {
                            &mut position.1
                        };
                        *slot = Some(Value::Number(n));
                    }
                    None => self.err(at, format!("{name}= is a number")),
                },
                "needs" => {
                    needs = match entry.value() {
                        KdlValue::Null => Needs::Nothing,
                        KdlValue::String(list) => match tokens(list) {
                            Ok(t) => Needs::List(t),
                            Err(bad) => {
                                self.err(at, format!("needs= names steps, as \"fetch check:true\"; `{bad}` is not one"));
                                Needs::Nothing
                            }
                        },
                        _ => {
                            self.err(
                                at,
                                "needs= names the steps it needs, as \"fetch check:true\"",
                            );
                            Needs::Nothing
                        }
                    }
                }
                other => {
                    let hint = near(other, &NODE_PROPS)
                        .map(|k| format!("; did you mean {k}=?"))
                        .unwrap_or_default();
                    self.err(at, format!("unknown property {other}={hint}"));
                }
            }
        }
        if trigger {
            out.insert("type".into(), Value::String("trigger".into()));
        }
        match position {
            (Some(x), Some(y)) => {
                let mut p = Map::new();
                p.insert("x".into(), x);
                p.insert("y".into(), y);
                out.insert("position".into(), Value::Object(p));
            }
            (None, None) => {}
            _ => self.err(
                node.span().offset(),
                "a step has both x= and y=, or neither",
            ),
        }
        let children = node.children();
        let mut has_config = typed && trigger;
        let mut explicit: Option<Value> = None;
        if let Some(block) = children {
            let plain = block.nodes().iter().any(|c| !is_field(c.name().value()));
            if block.nodes().is_empty() || plain {
                has_config = true;
            }
            for child in block.nodes() {
                let name = child.name().value();
                let value = self.value_in(child, true);
                if name == "@config" {
                    explicit = Some(value);
                } else if is_field(name) {
                    out.insert(name[1..].to_owned(), value);
                } else {
                    config.insert(unescape(name).to_owned(), value);
                }
            }
        }
        if let Some(c) = explicit {
            out.insert("config".into(), c);
        } else if has_config {
            out.insert("config".into(), Value::Object(config));
        }
        (Value::Object(out), needs)
    }

    fn value(&mut self, node: &KdlNode) -> Value {
        self.value_in(node, false)
    }

    /// A key's value: its one argument, a block of keys, or a block of `-` items.
    fn value_in(&mut self, node: &KdlNode, config: bool) -> Value {
        let at = node.span().offset();
        let ty = node.ty().map(|t| t.value());
        let args: Vec<&KdlEntry> = node.entries().iter().collect();
        if let Some(e) = args.iter().find(|e| e.name().is_some()) {
            self.err(
                e.span().offset(),
                format!("`{}` holds a value, not properties", node.name().value()),
            );
            return Value::Null;
        }
        match (ty, args.as_slice(), node.children()) {
            (Some("list"), [], None) => Value::Array(Vec::new()),
            (Some(other), _, _) if other != "list" => {
                self.err(at, format!("({other}) is not a kind of value; (list) is"));
                Value::Null
            }
            (_, [one], None) => self.scalar(one, config),
            (_, [], Some(block)) => {
                let items = block.nodes();
                let list = !items.is_empty() && items.iter().all(|c| c.name().value() == "-");
                if list {
                    Value::Array(items.iter().map(|c| self.value_in(c, config)).collect())
                } else {
                    let mut map = Map::new();
                    for c in items {
                        let v = self.value_in(c, config);
                        map.insert(unescape(c.name().value()).to_owned(), v);
                    }
                    Value::Object(map)
                }
            }
            (_, [], None) => {
                self.err(
                    at,
                    format!(
                        "`{}` has no value: give it one, #null, or {{}} for an empty block",
                        node.name().value()
                    ),
                );
                Value::Null
            }
            _ => {
                self.err(
                    at,
                    format!(
                        "`{}` holds one value, or a block, not both or several",
                        node.name().value()
                    ),
                );
                Value::Null
            }
        }
    }

    fn scalar(&mut self, entry: &KdlEntry, config: bool) -> Value {
        let at = entry.span().offset();
        let ty = entry.ty().map(|t| t.value());
        let value = entry.value();
        match (ty, value) {
            (None, v) => match plain(v) {
                Some(v) => v,
                None => {
                    self.err(at, "a number JSON cannot hold");
                    Value::Null
                }
            },
            (Some("connection"), KdlValue::String(name)) if config => {
                let found: Vec<&(String, String)> = self
                    .bindings
                    .connections
                    .iter()
                    .filter(|(_, n)| n == name)
                    .collect();
                match found.as_slice() {
                    [(id, _)] => Value::String(id.clone()),
                    [] => {
                        self.warnings.push(FileError {
                            at: line_col(self.text, at),
                            message: format!("no connection here is called {name}"),
                        });
                        Value::String(name.clone())
                    }
                    _ => {
                        self.warnings.push(FileError {
                            at: line_col(self.text, at),
                            message: format!("more than one connection here is called {name}"),
                        });
                        Value::String(name.clone())
                    }
                }
            }
            (Some("project"), KdlValue::String(rest)) if config => match self.bindings.project {
                Some((_, path)) if rest == "." => Value::String(path.to_owned()),
                Some((_, path)) => Value::String(format!("{path}/{rest}")),
                None => Value::String(rest.clone()),
            },
            (Some("project-name"), KdlValue::String(written)) if config => {
                match self.bindings.project {
                    Some((name, _)) => Value::String(name.to_owned()),
                    None => Value::String(written.clone()),
                }
            }
            (Some(other), _) => {
                self.err(
                    at,
                    format!("({other}) is not a kind of value a step's config takes; (connection) and (project) are"),
                );
                Value::Null
            }
        }
    }
}

/// `@key`: a field of the node, not its config; `@@key` is a config key starting with `@`.
fn is_field(name: &str) -> bool {
    name.starts_with('@') && !name.starts_with("@@") && name != "@-"
}

/// `source[:branch][#id]` words, space between.
fn tokens(list: &str) -> Result<Vec<Need>, String> {
    list.split_whitespace()
        .map(|word| {
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
            if source.is_empty() {
                return Err(word.to_owned());
            }
            Ok((source.to_owned(), branch, id))
        })
        .collect()
}

fn number(v: &KdlValue) -> Option<Number> {
    match plain(v)? {
        Value::Number(n) => Some(n),
        _ => None,
    }
}

/// A KDL value as JSON: `None` for a number JSON cannot hold.
fn plain(v: &KdlValue) -> Option<Value> {
    Some(match v {
        KdlValue::String(s) => Value::String(s.clone()),
        KdlValue::Integer(i) => {
            if let Ok(n) = i64::try_from(*i) {
                Value::from(n)
            } else {
                Value::from(u64::try_from(*i).ok()?)
            }
        }
        KdlValue::Float(f) => Value::Number(Number::from_f64(*f)?),
        KdlValue::Bool(b) => Value::Bool(*b),
        KdlValue::Null => Value::Null,
    })
}

/// One step or trigger of a file, where it is: what a check of its type reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outlined {
    pub trigger: bool,
    /// The step's type, or the trigger's `triggerType`.
    pub kind: Option<String>,
    pub at: (usize, usize),
}

/// The steps and trigger of a file that parses, in order, each where it is.
pub fn outline(text: &str) -> Vec<Outlined> {
    let Ok(doc) = KdlDocument::parse_v2(text) else {
        return Vec::new();
    };
    let Some(wf) = doc.nodes().first().and_then(KdlNode::children) else {
        return Vec::new();
    };
    wf.nodes()
        .iter()
        .filter(|n| matches!(n.name().value(), "trigger" | "step"))
        .map(|n| Outlined {
            trigger: n.name().value() == "trigger",
            kind: n
                .entries()
                .iter()
                .find(|e| e.name().is_none())
                .and_then(|e| e.value().as_string())
                .map(str::to_owned),
            at: line_col(text, n.span().offset()),
        })
        .collect()
}
