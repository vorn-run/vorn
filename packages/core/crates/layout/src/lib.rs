//! Saved layouts: sessions and panes arranged as a tree, kept as a TOML file.
//!
//! A layout file is the tree's root with `name`, `project`, `view` and
//! `columns` beside it. A node is a split (`split`, `ratio`,
//! `[[children]]`) or one thing to open:
//!
//! ```toml
//! name = "dev"
//! project = "vorn"
//! view = "flexible"
//! split = "horizontal"
//! ratio = 0.6
//!
//! [[children]]
//! agent = "claude"
//! label = "agent"
//!
//! [[children]]
//! split = "vertical"
//!
//! [[children.children]]
//! command = ["tail", "-f", "app.log"]
//!
//! [[children.children]]
//! pane = "browser"
//! url = "http://localhost:3000"
//! ```
//!
//! `view` is how the grid shows the sessions: `flexible` lays them out as
//! the tree, splits and ratios as written; `grid` puts them in rows of
//! `columns` (automatic when absent); `tabs` one at a time. Without it, a
//! split opens flexible and a single session leaves the view as it is.
//!
//! [`parse`] reads it and says which line is wrong; [`Layout::to_toml`] writes
//! one back so that `parse` reads the same layout.

use std::borrow::Cow;

use toml::de::{DeTable, DeValue};
use toml::map::Map;
use toml::Spanned;

/// How a split lays its children out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Side by side.
    Horizontal,
    /// One above the other.
    Vertical,
}

impl Direction {
    pub fn word(self) -> &'static str {
        match self {
            Direction::Horizontal => "horizontal",
            Direction::Vertical => "vertical",
        }
    }
}

/// How the grid shows a layout's sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The tree itself: its splits and ratios.
    Flexible,
    /// Rows of cards, `columns` wide.
    Grid,
    /// One session at a time under a tab strip.
    Tabs,
}

impl View {
    pub fn word(self) -> &'static str {
        match self {
            View::Flexible => "flexible",
            View::Grid => "grid",
            View::Tabs => "tabs",
        }
    }

    pub fn parse(word: &str) -> Option<View> {
        match word {
            "flexible" => Some(View::Flexible),
            "grid" => Some(View::Grid),
            "tabs" => Some(View::Tabs),
            _ => None,
        }
    }
}

/// The most columns a grid view takes.
pub const MAX_COLUMNS: u32 = 4;

/// What a session runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// An agent by id (`claude`, `codex`, ...).
    Agent(String),
    /// A program and its arguments, typed into a shell.
    Command(Vec<String>),
    /// A plain shell.
    Shell,
}

/// One session to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub launch: Launch,
    /// Where it starts, relative to the project when not absolute.
    pub cwd: Option<String>,
    pub label: Option<String>,
}

/// A pane to open beside the session before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    /// Only `browser` today.
    pub kind: String,
    pub url: Option<String>,
}

/// A leaf of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leaf {
    Session(Session),
    Pane(Pane),
}

/// A node of the tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Split {
        direction: Direction,
        /// The first child's share, 0 to 1 exclusive; the rest share what is left.
        ratio: f64,
        children: Vec<Node>,
    },
    Leaf(Leaf),
}

/// A saved layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub name: Option<String>,
    /// A project's name or path; the one asked for at open time wins.
    pub project: Option<String>,
    /// As written; [`Layout::view`] is what it opens in.
    pub view: Option<View>,
    /// A grid view's columns, 1 to [`MAX_COLUMNS`]; automatic when absent.
    pub columns: Option<u32>,
    pub root: Node,
}

/// One mistake in a layout file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// 1-based.
    pub line: usize,
    pub message: String,
}

impl Problem {
    /// `file:line: message`, the way a compiler words it.
    pub fn at(&self, file: &str) -> String {
        format!("{file}:{}: {}", self.line, self.message)
    }
}

/// Whether `name` can be a layout's file name: letters, digits, `.`, `_` and `-`, not leading with `.`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl Layout {
    /// The view the layout opens in: `view`, else a grid when `columns` is
    /// given, else flexible for a split. None leaves the view as it is.
    pub fn view(&self) -> Option<View> {
        self.view.or_else(|| {
            if self.columns.is_some() {
                Some(View::Grid)
            } else if matches!(self.root, Node::Split { .. }) {
                Some(View::Flexible)
            } else {
                None
            }
        })
    }

    /// The leaves in tree order, which is the order sessions are started in.
    pub fn leaves(&self) -> Vec<&Leaf> {
        fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Leaf>) {
            match node {
                Node::Leaf(leaf) => out.push(leaf),
                Node::Split { children, .. } => children.iter().for_each(|c| walk(c, out)),
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// The layout as a file; [`parse`] reads it back as the same layout.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        if let Some(name) = &self.name {
            out.push_str(&format!("name = {}\n", quote(name)));
        }
        if let Some(project) = &self.project {
            out.push_str(&format!("project = {}\n", quote(project)));
        }
        if let Some(view) = self.view {
            out.push_str(&format!("view = \"{}\"\n", view.word()));
        }
        if let Some(columns) = self.columns {
            out.push_str(&format!("columns = {columns}\n"));
        }
        write_node(&self.root, "", &mut out);
        out
    }
}

fn quote(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

fn write_node(node: &Node, path: &str, out: &mut String) {
    match node {
        Node::Split {
            direction,
            ratio,
            children,
        } => {
            out.push_str(&format!("split = \"{}\"\n", direction.word()));
            if (*ratio - 0.5).abs() > f64::EPSILON {
                out.push_str(&format!("ratio = {ratio}\n"));
            }
            let child = if path.is_empty() {
                "children".to_owned()
            } else {
                format!("{path}.children")
            };
            for c in children {
                out.push_str(&format!("\n[[{child}]]\n"));
                write_node(c, &child, out);
            }
        }
        Node::Leaf(Leaf::Session(s)) => {
            match &s.launch {
                Launch::Agent(a) => out.push_str(&format!("agent = {}\n", quote(a))),
                Launch::Shell => out.push_str("shell = true\n"),
                Launch::Command(c) => {
                    let words: Vec<String> = c.iter().map(|w| quote(w)).collect();
                    out.push_str(&format!("command = [{}]\n", words.join(", ")));
                }
            }
            if let Some(cwd) = &s.cwd {
                out.push_str(&format!("cwd = {}\n", quote(cwd)));
            }
            if let Some(label) = &s.label {
                out.push_str(&format!("label = {}\n", quote(label)));
            }
        }
        Node::Leaf(Leaf::Pane(p)) => {
            out.push_str(&format!("pane = {}\n", quote(&p.kind)));
            if let Some(url) = &p.url {
                out.push_str(&format!("url = {}\n", quote(url)));
            }
        }
    }
}

/// The 1-based line of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// The keys a layout file holds beside its root node.
const TOP_KEYS: [&str; 4] = ["name", "project", "view", "columns"];

struct Reader<'t> {
    text: &'t str,
    agents: &'t [&'t str],
    problems: Vec<Problem>,
}

type Pairs<'a> = &'a Map<Spanned<Cow<'a, str>>, Spanned<DeValue<'a>>>;

impl Reader<'_> {
    fn bad(&mut self, at: usize, message: String) {
        self.problems.push(Problem {
            line: line_of(self.text, at),
            message,
        });
    }

    fn string(&mut self, v: &Spanned<DeValue<'_>>, key: &str) -> Option<String> {
        match v.get_ref() {
            DeValue::String(s) => Some(s.to_string()),
            _ => {
                self.bad(v.span().start, format!("`{key}` must be a string"));
                None
            }
        }
    }

    /// A node, from the table that holds it. `top` holds the layout's own keys as well.
    fn node(&mut self, t: Pairs<'_>, top: bool, start: usize) -> Option<Node> {
        const KINDS: [&str; 5] = ["split", "agent", "command", "shell", "pane"];
        let has = |k: &str| t.iter().any(|(key, _)| key.get_ref() == k);
        let kinds: Vec<&str> = KINDS.iter().copied().filter(|k| has(k)).collect();
        let allowed: &[&str] = match kinds.first().copied() {
            Some("split") => &["split", "ratio", "children"],
            Some("pane") => &["pane", "url"],
            _ => &["agent", "command", "shell", "cwd", "label"],
        };
        for (key, _) in t.iter() {
            let k = key.get_ref().as_ref();
            let top_key = top && TOP_KEYS.contains(&k);
            if !allowed.contains(&k) && !top_key {
                self.bad(key.span().start, format!("unknown key `{k}` here"));
            }
        }
        match kinds.as_slice() {
            [] => {
                self.bad(
                    start,
                    "says nothing to open: give it agent, command, shell, pane or split".into(),
                );
                None
            }
            [one] => self.kind(t, one, start),
            [_, second, ..] => {
                let at = t
                    .iter()
                    .find(|(k, _)| k.get_ref() == second)
                    .map_or(start, |(k, _)| k.span().start);
                self.bad(
                    at,
                    format!("`{second}` cannot be combined with `{}`", kinds[0]),
                );
                None
            }
        }
    }

    fn kind(&mut self, t: Pairs<'_>, kind: &str, start: usize) -> Option<Node> {
        let get = |k: &str| t.iter().find(|(key, _)| key.get_ref() == k).map(|(_, v)| v);
        if kind == "split" {
            let direction = match self.string(get("split")?, "split").as_deref() {
                Some("horizontal") => Some(Direction::Horizontal),
                Some("vertical") => Some(Direction::Vertical),
                Some(_) => {
                    self.bad(
                        get("split")?.span().start,
                        "`split` is \"horizontal\" or \"vertical\"".into(),
                    );
                    None
                }
                None => None,
            };
            let ratio = match get("ratio") {
                None => Some(0.5),
                Some(v) => match v.get_ref() {
                    DeValue::Float(f) => f.as_str().parse::<f64>().ok(),
                    DeValue::Integer(i) => i.as_str().parse::<f64>().ok(),
                    _ => None,
                }
                .filter(|r| *r > 0.0 && *r < 1.0)
                .or_else(|| {
                    self.bad(v.span().start, "`ratio` is a number between 0 and 1".into());
                    None
                }),
            };
            let mut children = Vec::new();
            match get("children") {
                None => self.bad(start, "a split needs `[[children]]`".into()),
                Some(v) => match v.get_ref() {
                    DeValue::Array(items) if !items.is_empty() => {
                        for item in items {
                            match item.get_ref() {
                                DeValue::Table(ct) => {
                                    if let Some(n) = self.node(ct, false, item.span().start) {
                                        children.push(n);
                                    }
                                }
                                _ => self.bad(item.span().start, "a child is a table".into()),
                            }
                        }
                    }
                    _ => self.bad(
                        v.span().start,
                        "`children` needs at least one `[[children]]`".into(),
                    ),
                },
            }
            return Some(Node::Split {
                direction: direction?,
                ratio: ratio?,
                children,
            });
        }
        if kind == "pane" {
            let kind = self.string(get("pane")?, "pane")?;
            if kind != "browser" {
                self.bad(get("pane")?.span().start, "`pane` is \"browser\"".into());
                return None;
            }
            let url = get("url").and_then(|v| self.string(v, "url"));
            return Some(Node::Leaf(Leaf::Pane(Pane { kind, url })));
        }
        let launch = match kind {
            "agent" => {
                let v = get("agent")?;
                let a = self.string(v, "agent")?;
                if !self.agents.contains(&a.as_str()) {
                    self.bad(
                        v.span().start,
                        format!("unknown agent \"{a}\". Try: {}", self.agents.join(", ")),
                    );
                    return None;
                }
                Launch::Agent(a)
            }
            "shell" => {
                let v = get("shell")?;
                if !matches!(v.get_ref(), DeValue::Boolean(true)) {
                    self.bad(v.span().start, "`shell` is `true`".into());
                    return None;
                }
                Launch::Shell
            }
            _ => {
                let v = get("command")?;
                match v.get_ref() {
                    DeValue::Array(words)
                        if !words.is_empty()
                            && words
                                .iter()
                                .all(|w| matches!(w.get_ref(), DeValue::String(_))) =>
                    {
                        Launch::Command(
                            words
                                .iter()
                                .filter_map(|w| match w.get_ref() {
                                    DeValue::String(s) => Some(s.to_string()),
                                    _ => None,
                                })
                                .collect(),
                        )
                    }
                    _ => {
                        self.bad(
                            v.span().start,
                            "`command` is a list of strings, such as [\"tail\", \"-f\", \"app.log\"]"
                                .into(),
                        );
                        return None;
                    }
                }
            }
        };
        let cwd = get("cwd").and_then(|v| self.string(v, "cwd"));
        let label = get("label").and_then(|v| self.string(v, "label"));
        Some(Node::Leaf(Leaf::Session(Session { launch, cwd, label })))
    }
}

/// Reads a layout file. `agents` are the ids `agent = ...` may name.
///
/// Every mistake found is returned, each with its line, rather than the first.
pub fn parse(text: &str, agents: &[&str]) -> Result<Layout, Vec<Problem>> {
    let table = DeTable::parse(text).map_err(|e| {
        vec![Problem {
            line: e.span().map_or(1, |s| line_of(text, s.start)),
            message: e.message().trim_end().to_owned(),
        }]
    })?;
    let mut r = Reader {
        text,
        agents,
        problems: Vec::new(),
    };
    // The reader borrows the table for as long as the text lives.
    let t: Pairs<'_> = table.get_ref();
    let named = |key: &str, r: &mut Reader<'_>| -> Option<String> {
        let (k, v) = t.iter().find(|(k, _)| k.get_ref() == key)?;
        let _ = k;
        r.string(v, key)
    };
    let name = named("name", &mut r);
    if let Some(n) = &name {
        if !valid_name(n) {
            let at = t
                .iter()
                .find(|(k, _)| k.get_ref() == "name")
                .map_or(0, |(_, v)| v.span().start);
            r.bad(at, "`name` is letters, digits, `.`, `_` and `-`".to_owned());
        }
    }
    let project = named("project", &mut r);
    let at = |key: &str| t.iter().find(|(k, _)| k.get_ref() == key).map(|(_, v)| v);
    let view = at("view").and_then(|v| {
        let word = r.string(v, "view")?;
        View::parse(&word).or_else(|| {
            r.bad(
                v.span().start,
                "`view` is \"flexible\", \"grid\" or \"tabs\"".into(),
            );
            None
        })
    });
    let columns = at("columns").and_then(|v| {
        let n = match v.get_ref() {
            DeValue::Integer(i) => i.as_str().parse::<u32>().ok(),
            _ => None,
        }
        .filter(|n| (1..=MAX_COLUMNS).contains(n));
        if n.is_none() {
            r.bad(
                v.span().start,
                format!("`columns` is a whole number from 1 to {MAX_COLUMNS}"),
            );
        } else if view.is_some_and(|v| v != View::Grid) {
            r.bad(
                v.span().start,
                "`columns` goes with `view = \"grid\"`".into(),
            );
        }
        n
    });
    let root = r.node(t, true, 0);
    match (root, r.problems.is_empty()) {
        (Some(root), true) => Ok(Layout {
            name,
            project,
            view,
            columns,
            root,
        }),
        _ => {
            let mut problems = r.problems;
            problems.sort_by_key(|p| p.line);
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENTS: &[&str] = &["claude", "codex"];

    const DEV: &str = r#"name = "dev"
project = "vorn"
split = "horizontal"
ratio = 0.6

[[children]]
agent = "claude"
label = "agent"

[[children]]
split = "vertical"

[[children.children]]
command = ["tail", "-f", "app.log"]
cwd = "logs"

[[children.children]]
pane = "browser"
url = "http://localhost:3000"
"#;

    #[test]
    fn reads_a_tree_and_walks_its_leaves_in_order() {
        let l = parse(DEV, AGENTS).unwrap();
        assert_eq!(l.name.as_deref(), Some("dev"));
        assert_eq!(l.project.as_deref(), Some("vorn"));
        let leaves = l.leaves();
        assert_eq!(leaves.len(), 3);
        assert!(
            matches!(leaves[0], Leaf::Session(s) if s.launch == Launch::Agent("claude".into())
            && s.label.as_deref() == Some("agent"))
        );
        assert!(matches!(leaves[1], Leaf::Session(s) if s.launch
            == Launch::Command(vec!["tail".into(), "-f".into(), "app.log".into()])
            && s.cwd.as_deref() == Some("logs")));
        assert!(
            matches!(leaves[2], Leaf::Pane(p) if p.url.as_deref() == Some("http://localhost:3000"))
        );
        assert!(matches!(&l.root, Node::Split { ratio, .. } if (*ratio - 0.6).abs() < 1e-9));
    }

    #[test]
    fn a_single_session_is_a_layout() {
        let l = parse("name = \"one\"\nshell = true\n", AGENTS).unwrap();
        assert!(matches!(&l.root, Node::Leaf(Leaf::Session(s)) if s.launch == Launch::Shell));
    }

    #[test]
    fn writes_what_it_reads() {
        let l = parse(DEV, AGENTS).unwrap();
        assert_eq!(parse(&l.to_toml(), AGENTS).unwrap(), l);
        let odd = Layout {
            name: Some("odd".into()),
            project: None,
            view: Some(View::Tabs),
            columns: None,
            root: Node::Leaf(Leaf::Session(Session {
                launch: Launch::Command(vec!["echo".into(), "a \"b\"\nc".into()]),
                cwd: Some("C:\\x".into()),
                label: None,
            })),
        };
        assert_eq!(parse(&odd.to_toml(), AGENTS).unwrap(), odd);
    }

    fn lines(text: &str) -> Vec<(usize, String)> {
        parse(text, AGENTS)
            .unwrap_err()
            .into_iter()
            .map(|p| (p.line, p.message))
            .collect()
    }

    #[test]
    fn names_the_line_of_each_mistake() {
        let bad = "name = \"x\"\nsplit = \"diagonal\"\n\n[[children]]\nagent = \"nobody\"\n\n[[children]]\ncolor = 1\nshell = true\n";
        let got = lines(bad);
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0].0, 2);
        assert!(got[0].1.contains("horizontal"));
        assert_eq!(got[1].0, 5);
        assert!(got[1].1.starts_with("unknown agent \"nobody\""));
        assert_eq!(got[2], (8, "unknown key `color` here".to_owned()));
    }

    #[test]
    fn reports_syntax_errors_and_shapes_that_cannot_be_opened() {
        assert_eq!(lines("name = \n")[0].0, 1);
        assert!(lines("name = \"x\"\n")[0]
            .1
            .contains("says nothing to open"));
        assert!(lines("shell = true\nagent = \"claude\"\n")[0]
            .1
            .contains("cannot be combined"));
        assert!(lines("split = \"vertical\"\n")[0].1.contains("children"));
        assert!(
            lines("split = \"vertical\"\nratio = 1.5\n[[children]]\nshell = true\n")[0]
                .1
                .contains("between 0 and 1")
        );
        assert!(lines("command = []\n")[0].1.contains("list of strings"));
        assert!(lines("pane = \"video\"\n")[0].1.contains("browser"));
        assert!(lines("name = \"a/b\"\nshell = true\n")[0]
            .1
            .contains("letters"));
    }

    #[test]
    fn reads_and_writes_the_view() {
        let l = parse(DEV, AGENTS).unwrap();
        assert_eq!(l.view, None);
        assert_eq!(l.view(), Some(View::Flexible));
        let grid = parse("view = \"grid\"\ncolumns = 3\nshell = true\n", AGENTS).unwrap();
        assert_eq!((grid.view(), grid.columns), (Some(View::Grid), Some(3)));
        assert_eq!(parse(&grid.to_toml(), AGENTS).unwrap(), grid);
        let cols = parse("columns = 2\nshell = true\n", AGENTS).unwrap();
        assert_eq!(cols.view(), Some(View::Grid));
        let one = parse("shell = true\n", AGENTS).unwrap();
        assert_eq!(one.view(), None);
        let flex = DEV.replace(
            "project = \"vorn\"\n",
            "project = \"vorn\"\nview = \"flexible\"\n",
        );
        let l = parse(&flex, AGENTS).unwrap();
        assert_eq!(l.view, Some(View::Flexible));
        assert_eq!(parse(&l.to_toml(), AGENTS).unwrap(), l);
    }

    #[test]
    fn says_what_a_view_may_be() {
        assert!(lines("view = \"cards\"\nshell = true\n")[0]
            .1
            .contains("\"flexible\", \"grid\" or \"tabs\""));
        assert!(lines("columns = 9\nshell = true\n")[0].1.contains("1 to 4"));
        assert_eq!(
            lines("view = \"tabs\"\ncolumns = 2\nshell = true\n")[0],
            (2, "`columns` goes with `view = \"grid\"`".to_owned())
        );
    }

    #[test]
    fn a_name_is_a_safe_file_name() {
        assert!(valid_name("dev-2.v1"));
        for bad in ["", ".hidden", "a/b", "a b", "..", &"x".repeat(65)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }
}
