//! Extension UI as an element tree, built with [`sdk`], from Wasm or as JSON, and [`check`]ed the same.

use serde::{Deserialize, Serialize};

pub mod parts;
pub mod sdk;
#[cfg(test)]
mod tests;

pub use parts::{
    Code, CodeState, Command, Filter, Line, LineKind, Log, Outline, Tab, Tabs, Tool, MAX_SPLIT,
    MIN_SPLIT,
};

/// The call that asks an extension for its tree: args `{point, surface}`, answer [`Rendered`].
pub const RENDER: &str = "ui.render";
/// Tells an extension what happened: args `{point, event}`, answer an [`Answer`].
pub const EVENT: &str = "ui.event";
/// The colour tokens this build knows; a tree names the set it was made for.
pub const TOKENS: u32 = 1;

/// Past these a tree is a mistake, not a page.
pub const MAX_NODES: usize = 20_000;
pub const MAX_DEPTH: usize = 48;
pub const MAX_TEXT: usize = 8 * 1024;
pub const MAX_SVG: usize = 64 * 1024;

/// One box of the tree: what it is, the id its events carry, how it sits, and what it holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    #[serde(flatten)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Props::is_empty")]
    pub props: Props,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

/// What a node is. Containers hold `children`; the rest carry what they draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    /// A plain box, laid out by its props: a column unless `row`.
    Box,
    /// Words in one of the designed styles.
    Text {
        text: String,
        #[serde(default)]
        style: TextStyle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
        #[serde(default, skip_serializing_if = "is_false")]
        wrap: bool,
    },
    /// A kit icon by name, as `trash-2`.
    Icon {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
    },
    /// An SVG stretched over `w`×`h`, in one paint; `glyph` stands for it on cells.
    Shape {
        svg: String,
        glyph: String,
        w: f32,
        h: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ink: Option<Paint>,
    },
    /// Reports `click`; with `confirm`, only once the question is answered.
    Button {
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        #[serde(default, skip_serializing_if = "ButtonStyle::is_plain")]
        style: ButtonStyle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirm: Option<String>,
    },
    /// A one-line field: reports `submit` with what it holds on Return and, when `live`, `change` as it is typed.
    Field {
        #[serde(default)]
        value: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default, skip_serializing_if = "is_false")]
        secret: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        live: bool,
    },
    /// One of several, as chips: reports `select` with the option's id.
    Choice {
        options: Vec<Choose>,
    },
    Badge(Badge),
    /// A status dot in a tone; its `label` prop names the state it shows.
    Dot {
        tone: Tone,
    },
    /// A switch beside its label: reports `toggle` with the state it switches to, `true` or `false`.
    Toggle {
        #[serde(default)]
        label: String,
        on: bool,
    },
    /// A line that stands out, as an error says what happened.
    Notice {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
    },
    /// A figure under its quiet label, as a page's total; `end` sits it at the right.
    Stat {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
        #[serde(default, skip_serializing_if = "is_false")]
        end: bool,
    },
    /// A stacked bar and, when its segments are labelled, its legend.
    Meter(Meter),
    /// A titled part of a page.
    Section {
        title: String,
    },
    /// Controls for a whole page; a settings page lifts it into its header.
    Toolbar,
    /// Entries or groups; `select` adds check boxes and `bulk` buttons, `lazy` builds rows in view only.
    List {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loading: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        select: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bulk: Vec<Node>,
        #[serde(default, skip_serializing_if = "is_false")]
        lazy: bool,
    },
    /// Entries under a heading, in a list.
    Group {
        heading: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        aside: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        actions: Vec<Node>,
    },
    /// One list row: `actions` show on hover, `more` behind its menu, `children` under its detail.
    Entry(Box<Entry>),
    /// Columns of cards: each column's children, or, with `cards`, cards the client reads and draws itself.
    Board {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cards: Option<Cards>,
        /// What an empty column says.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty: Option<String>,
    },
    /// A board's column; each child is a card. On a board with `cards`, its id is the value its cards hold.
    Column {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    /// The workflow editor's canvas, which the client draws whole.
    Graph,
    Divider,
    /// A strip of tabs and tools, as a pane's title bar.
    Tabs(Box<Tabs>),
    /// Rows in a tree of folders, as a file tree.
    Outline(Box<Outline>),
    /// Its one or two children side by side, `ratio` (0 to 1) of the width to the first, with a
    /// divider between them that reports `resize` with the ratio it was dragged to.
    Split {
        ratio: f32,
        /// The divider's accessible name.
        #[serde(default)]
        label: String,
        /// Narrower than this many logical pixels, only the first child draws.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        narrow: Option<f32>,
    },
    /// A file's text being edited; its children are buttons at the end of its strip.
    Code(Box<Code>),
    /// A shell's commands over its live rows; drawn in a shell's `shell` place.
    Log(Box<Log>),
}

/// Where a board's cards come from when the client reads them itself.
///
/// A board of thousands cannot travel as a tree, so the tree names the
/// source and its columns, and the client windows the cards, drags them
/// between columns and opens them, as it always drew the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cards {
    /// The core's task list, read a page at a time; a column's id is the status it holds.
    Tasks,
}

/// A list's row.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Entry {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A mono line before the detail, as a branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A dot in a tone before the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Badge>,
    /// A figure at the right, in a column of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aside: Option<String>,
    /// A short bar under the figure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter: Option<Meter>,
    /// Small `key: value` labels under the detail.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chips: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more: Vec<Node>,
    /// In a list that selects, whether this one can be checked.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub selectable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Badge {
    pub text: String,
    #[serde(default)]
    pub tone: Tone,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Meter {
    pub max: f64,
    pub segments: Vec<Segment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aside: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub value: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default)]
    pub tone: Tone,
}

/// One option of a [`Kind::Choice`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choose {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
}

/// What a colour means; the client picks the colour on each surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Warn,
    Bad,
    Info,
    Accent,
    /// At work, as a running agent.
    Busy,
    /// Quiet, as an idle one.
    Muted,
    /// Waiting its turn, as a task to do.
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextStyle {
    #[default]
    Body,
    Title,
    Secondary,
    Mono,
    /// A figure, right-aligned in a column of its own.
    Number,
    Heading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Plain,
    Primary,
    Danger,
}

impl ButtonStyle {
    fn is_plain(&self) -> bool {
        *self == ButtonStyle::Plain
    }
}

/// Theme colours by meaning, set 1 of [`TOKENS`]; a later set's token draws as ink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Token {
    /// The window's ground.
    Base,
    /// Below the ground, as a field or a notice.
    Sunken,
    /// Just above the ground, as a card.
    Raised,
    /// A menu or a popover.
    Overlay,
    /// A hairline between things.
    Line,
    Ink,
    InkSecondary,
    InkFaint,
    Accent,
    Good,
    Warn,
    Bad,
    Info,
    White,
    #[serde(other)]
    Unknown,
}

/// A token, or a token at an alpha.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Paint {
    Token(Token),
    Tint { token: Token, alpha: f32 },
}

/// How a box sits: a column unless `row`, sizes in logical pixels.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Props {
    #[serde(default, skip_serializing_if = "is_false")]
    pub row: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub px: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub py: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<f32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub grow: bool,
    /// Children centred across the box.
    #[serde(default, skip_serializing_if = "is_false")]
    pub center: bool,
    /// Children spread to both ends.
    #[serde(default, skip_serializing_if = "is_false")]
    pub between: bool,
    /// Children wrap onto more lines.
    #[serde(default, skip_serializing_if = "is_false")]
    pub wrap: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Paint>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub rounded: bool,
    /// Its accessible name, for a box assistive technology should announce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Props {
    pub fn is_empty(&self) -> bool {
        *self == Props::default()
    }
}

/// What the client tells `ui.render` about where the tree draws.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Surface {
    /// A terminal's cells rather than pixels.
    #[serde(default)]
    pub cells: bool,
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub height: f32,
    /// Which of the point's places it draws: the session a pane or a shell's
    /// log draws for, or the row a card draws; none for a point drawn once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// `ui.render`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rendered {
    pub tokens: u32,
    pub tree: Node,
}

impl Rendered {
    pub fn new(tree: Node) -> Rendered {
        Rendered {
            tokens: TOKENS,
            tree,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Click,
    Submit,
    Select,
    Toggle,
    /// A double click: a tab or a row kept open.
    Open,
    /// A tab's ✕.
    Close,
    /// What a live field or a code holds changed.
    Change,
    /// A split's divider was dragged to the ratio in `value`.
    Resize,
}

/// Something that happened to node `id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub kind: EventKind,
    /// A field's text, the option chosen, or the state a toggle switches to.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// A bulk button's list's checked ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected: Vec<String>,
    /// The scope the tree was drawn for ([`Surface::scope`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// How wide, in logical pixels, the place the tree drew in was, for a tree that lays itself out by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
}

impl Event {
    /// `kind` on node `id`, with `value`.
    pub fn new(id: impl Into<String>, kind: EventKind, value: impl Into<String>) -> Event {
        Event {
            id: id.into(),
            kind,
            value: value.into(),
            selected: Vec::new(),
            scope: None,
            width: None,
        }
    }
}

/// A question the client asks with its own alert before anything is done:
/// `title`, Cancel and `ok`; OK tells the extension `event`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ask {
    pub title: String,
    /// The default button's words; `OK` when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<String>,
    pub event: Event,
}

/// What `ui.event` answers: what follows the event, each part optional.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Answer {
    /// A line that says what happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toast: Option<String>,
    /// A line that says what failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Nodes changed in place, without a render.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patch: Vec<Patch>,
    /// One of the extension's dialogs to open, by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
    /// One of its windows to open, by id, in place of the one open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    /// Closes the dialog or window the tree is in.
    #[serde(default, skip_serializing_if = "is_false")]
    pub close: bool,
    /// A session to show in the main window, which comes forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// Brings the main window forward.
    #[serde(default, skip_serializing_if = "is_false")]
    pub show: bool,
    /// An `https:` page to open in the browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Vorn's sign-in window to open: the target a `signin` method answers, with the `id` it signs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signin: Option<serde_json::Value>,
    /// A question to ask first; its OK tells the extension the event it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
    /// The tree stands as drawn: nothing is asked for again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub same: bool,
}

impl Answer {
    /// An answer as an extension sent it; one that does not read is an empty one.
    pub fn read(v: &serde_json::Value) -> Answer {
        if v.is_object() {
            serde_json::from_value(v.clone()).unwrap_or_default()
        } else {
            Answer::default()
        }
    }
}

/// The child of a per-session tree (a badge's, a card footer's, a sidebar row's marks) drawn for `session`.
///
/// Such a tree is a box holding one child per session, its id the session's id.
pub fn for_session<'a>(tree: &'a Node, session: &str) -> Option<&'a Node> {
    tree.children
        .iter()
        .find(|c| c.id.as_deref() == Some(session))
}

/// Whether `tree` draws nothing: a box, or text of only spaces, with nothing else in it.
pub fn is_blank(tree: &Node) -> bool {
    let own = match &tree.kind {
        Kind::Box => true,
        Kind::Text { text, .. } => text.trim().is_empty(),
        _ => false,
    };
    own && tree.children.iter().all(is_blank)
}

/// A change to a drawn tree: node `id` becomes `node`, or goes when there is none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<Node>,
}

/// Applies `patches` to `tree` in order and checks the result; on an error `tree` is unchanged.
pub fn patch(tree: &mut Node, patches: &[Patch]) -> Result<(), Vec<String>> {
    let mut next = tree.clone();
    for (i, p) in patches.iter().enumerate() {
        if next.id.as_deref() == Some(p.id.as_str()) {
            match &p.node {
                Some(n) => next = n.clone(),
                None => return Err(vec![format!("patch[{i}]: the root cannot go")]),
            }
        } else if !splice(&mut next, p) {
            return Err(vec![format!("patch[{i}]: no node is '{}'", p.id)]);
        }
        // Checking each step bounds the depth chained patches can build.
        check(&next).map_err(|e| {
            e.into_iter()
                .map(|m| format!("patch[{i}]: {m}"))
                .collect::<Vec<_>>()
        })?;
    }
    *tree = next;
    Ok(())
}

fn splice(n: &mut Node, p: &Patch) -> bool {
    let at = n
        .children
        .iter()
        .position(|c| c.id.as_deref() == Some(p.id.as_str()));
    if let Some(at) = at {
        match &p.node {
            Some(new) => n.children[at] = new.clone(),
            None => {
                n.children.remove(at);
            }
        }
        return true;
    }
    n.children.iter_mut().any(|c| splice(c, p))
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_true(b: &bool) -> bool {
    *b
}

fn yes() -> bool {
    true
}

/// Reads and checks a `ui.render` answer as a program or a Wasm guest sent it.
pub fn read(answer: &serde_json::Value) -> Result<Rendered, Vec<String>> {
    let r: Rendered =
        serde_json::from_value(answer.clone()).map_err(|e| vec![format!("$: {e}")])?;
    if r.tokens == 0 || r.tokens > TOKENS {
        return Err(vec![format!(
            "$.tokens: {} is not a token set this build knows (1 to {TOKENS})",
            r.tokens
        )]);
    }
    check(&r.tree)?;
    Ok(r)
}

/// Every reason `tree` cannot be drawn, each with its path, as `$.children[2].id: ...`.
pub fn check(tree: &Node) -> Result<(), Vec<String>> {
    let mut c = Checker::default();
    c.node(tree, "$", 0);
    if c.count > MAX_NODES {
        c.errors.push(format!(
            "$: {} nodes; a tree has {MAX_NODES} at most",
            c.count
        ));
    }
    if c.errors.is_empty() {
        Ok(())
    } else {
        Err(c.errors)
    }
}

#[derive(Default)]
struct Checker {
    errors: Vec<String>,
    count: usize,
    ids: std::collections::HashSet<String>,
}

impl Checker {
    fn err(&mut self, at: &str, what: impl std::fmt::Display) {
        if self.errors.len() < 50 {
            self.errors.push(format!("{at}: {what}"));
        }
    }

    fn text(&mut self, at: &str, field: &str, s: &str) {
        if s.len() > MAX_TEXT {
            self.err(
                &format!("{at}.{field}"),
                format_args!("{} bytes; text is {MAX_TEXT} at most", s.len()),
            );
        }
    }

    fn paint(&mut self, at: &str, p: &Paint) {
        if let Paint::Tint { alpha, .. } = p {
            if !(0.0..=1.0).contains(alpha) {
                self.err(at, "alpha goes from 0 to 1");
            }
        }
    }

    fn size(&mut self, at: &str, v: Option<f32>) {
        if v.is_some_and(|v| !v.is_finite() || !(0.0..=4096.0).contains(&v)) {
            self.err(at, "a size goes from 0 to 4096");
        }
    }

    fn meter(&mut self, at: &str, m: &Meter) {
        if !m.max.is_finite() || m.max < 0.0 {
            self.err(&format!("{at}.max"), "is not a figure of 0 or more");
        }
        for (i, s) in m.segments.iter().enumerate() {
            if !s.value.is_finite() || s.value < 0.0 {
                self.err(
                    &format!("{at}.segments[{i}].value"),
                    "is not a figure of 0 or more",
                );
            }
            self.text(at, "label", &s.label);
        }
    }

    /// Node `n` must carry an id unique in the tree.
    fn named(&mut self, n: &Node, at: &str, what: &str) {
        match n.id.as_deref() {
            None | Some("") => self.err(at, format_args!("a {what} needs an id")),
            Some(id) => {
                if !self.ids.insert(id.to_owned()) {
                    self.err(
                        &format!("{at}.id"),
                        format_args!("'{id}' names two nodes; ids are unique"),
                    );
                }
            }
        }
    }

    fn tools(&mut self, at: &str, field: &str, tools: &[Tool]) {
        let mut ids = std::collections::HashSet::new();
        for (i, t) in tools.iter().enumerate() {
            let p = format!("{at}.{field}[{i}]");
            if t.id.is_empty() {
                self.err(&p, "a tool needs an id");
            } else if !ids.insert(t.id.as_str()) {
                self.err(
                    &format!("{p}.id"),
                    format_args!("'{}' names two tools", t.id),
                );
            }
            self.text(&p, "label", &t.label);
        }
    }

    fn tabs(&mut self, n: &Node, at: &str, t: &Tabs) {
        self.named(n, at, "tab strip");
        if t.tabs.len() > parts::MAX_TABS {
            self.err(
                &format!("{at}.tabs"),
                format_args!(
                    "{} tabs; a strip holds {} at most",
                    t.tabs.len(),
                    parts::MAX_TABS
                ),
            );
        }
        let mut ids = std::collections::HashSet::new();
        for (i, tab) in t.tabs.iter().enumerate() {
            let p = format!("{at}.tabs[{i}]");
            if tab.id.is_empty() {
                self.err(&p, "a tab needs an id");
            } else if !ids.insert(tab.id.as_str()) {
                self.err(
                    &format!("{p}.id"),
                    format_args!("'{}' names two tabs", tab.id),
                );
            }
            self.text(&p, "label", &tab.label);
        }
        self.tools(at, "lead", &t.lead);
        self.tools(at, "tools", &t.tools);
        self.tools(at, "more", &t.more);
    }

    fn outline(&mut self, n: &Node, at: &str, o: &Outline) {
        self.named(n, at, "outline");
        if o.rows.len() > parts::MAX_LINES {
            self.err(
                &format!("{at}.rows"),
                format_args!(
                    "{} rows; an outline holds {} at most",
                    o.rows.len(),
                    parts::MAX_LINES
                ),
            );
        }
        for (i, l) in o.rows.iter().enumerate().take(parts::MAX_LINES) {
            let p = format!("{at}.rows[{i}]");
            if l.id.is_empty() && l.kind != LineKind::Note {
                self.err(&p, "a row needs an id");
            }
            self.text(&p, "label", &l.label);
            if usize::from(l.depth) > MAX_DEPTH * 4 {
                self.err(&format!("{p}.depth"), "nests too deep");
            }
        }
        if let Some(f) = &o.filter {
            self.text(at, "filter", &f.value);
        }
        if let Some(a) = &o.action {
            self.tools(at, "action", std::slice::from_ref(a));
        }
    }

    fn buttons(&mut self, at: &str, field: &str, nodes: &[Node], depth: usize) {
        for (i, b) in nodes.iter().enumerate() {
            let p = format!("{at}.{field}[{i}]");
            if !matches!(b.kind, Kind::Button { .. }) {
                self.err(&p, "holds buttons only");
            }
            self.node(b, &p, depth + 1);
        }
    }

    fn node(&mut self, n: &Node, at: &str, depth: usize) {
        self.count += 1;
        if depth > MAX_DEPTH {
            self.err(at, format_args!("nests deeper than {MAX_DEPTH}"));
            return;
        }
        if let Some(id) = &n.id {
            self.text(at, "id", id);
        }
        let p = &n.props;
        for (k, v) in [
            ("gap", p.gap),
            ("p", p.p),
            ("px", p.px),
            ("py", p.py),
            ("w", p.w),
            ("h", p.h),
        ] {
            self.size(&format!("{at}.props.{k}"), v);
        }
        for paint in [&p.bg, &p.border].into_iter().flatten() {
            self.paint(&format!("{at}.props"), paint);
        }
        let holds = matches!(
            n.kind,
            Kind::Box
                | Kind::Section { .. }
                | Kind::Toolbar
                | Kind::List { .. }
                | Kind::Group { .. }
                | Kind::Entry(_)
                | Kind::Board { .. }
                | Kind::Column { .. }
                | Kind::Split { .. }
                | Kind::Code(_)
        );
        if !holds && !n.children.is_empty() {
            self.err(at, "holds no children");
        }
        match &n.kind {
            Kind::Text { text, .. } | Kind::Notice { text, .. } => self.text(at, "text", text),
            Kind::Shape {
                svg, glyph, w, h, ..
            } => {
                if svg.len() > MAX_SVG {
                    self.err(
                        &format!("{at}.svg"),
                        format_args!("{} bytes; a shape is {MAX_SVG} at most", svg.len()),
                    );
                }
                if !svg.trim_start().starts_with("<svg") {
                    self.err(&format!("{at}.svg"), "is not an <svg> element");
                }
                if glyph.chars().count() != 1 {
                    self.err(&format!("{at}.glyph"), "is one character, drawn on cells");
                }
                self.size(&format!("{at}.w"), Some(*w));
                self.size(&format!("{at}.h"), Some(*h));
            }
            Kind::Button { label, .. } => {
                self.named(n, at, "button");
                self.text(at, "label", label);
            }
            Kind::Field { value, .. } => {
                self.named(n, at, "field");
                self.text(at, "value", value);
            }
            Kind::Toggle { label, .. } => {
                self.named(n, at, "toggle");
                self.text(at, "label", label);
            }
            Kind::Choice { options } => {
                self.named(n, at, "choice");
                for (i, o) in options.iter().enumerate() {
                    if o.id.is_empty() {
                        self.err(&format!("{at}.options[{i}].id"), "is empty");
                    }
                }
            }
            Kind::Meter(m) => self.meter(at, m),
            Kind::List { bulk, select, .. } => {
                if *select || !bulk.is_empty() {
                    self.named(n, at, "list that selects");
                }
                if !bulk.is_empty() && !select {
                    self.err(&format!("{at}.bulk"), "acts on checked rows; set select");
                }
                self.buttons(at, "bulk", bulk, depth);
                for (i, c) in n.children.iter().enumerate() {
                    if !matches!(c.kind, Kind::Entry(_) | Kind::Group { .. }) {
                        self.err(
                            &format!("{at}.children[{i}]"),
                            "a list holds entries and groups",
                        );
                    }
                }
            }
            Kind::Group { actions, .. } => {
                self.buttons(at, "actions", actions, depth);
                for (i, c) in n.children.iter().enumerate() {
                    if !matches!(c.kind, Kind::Entry(_)) {
                        self.err(&format!("{at}.children[{i}]"), "a group holds entries");
                    }
                }
            }
            Kind::Entry(e) => {
                self.named(n, at, "entry");
                self.text(at, "title", &e.title);
                for c in &e.chips {
                    self.text(at, "chips", c);
                }
                if let Some(m) = &e.meter {
                    self.meter(&format!("{at}.meter"), m);
                }
                self.buttons(at, "actions", &e.actions, depth);
                self.buttons(at, "more", &e.more, depth);
            }
            Kind::Tabs(t) => self.tabs(n, at, t),
            Kind::Outline(o) => self.outline(n, at, o),
            Kind::Split { ratio, label, .. } => {
                self.text(at, "label", label);
                if !ratio.is_finite() || !(0.0..=1.0).contains(ratio) {
                    self.err(&format!("{at}.ratio"), "goes from 0 to 1");
                }
                if n.children.is_empty() || n.children.len() > 2 {
                    self.err(at, "a split holds one child or two");
                }
                if n.children.len() == 2 {
                    self.named(n, at, "split that can be dragged");
                }
            }
            Kind::Code(code) => {
                self.named(n, at, "code");
                self.text(at, "path", &code.path);
                let size = code.text.len() + code.draft.as_ref().map_or(0, String::len);
                if size > parts::MAX_CODE {
                    self.err(
                        &format!("{at}.text"),
                        format_args!("{size} bytes; code is {} at most", parts::MAX_CODE),
                    );
                }
                if let Some(e) = &code.error {
                    self.text(at, "error", e);
                }
                for (i, c) in n.children.iter().enumerate() {
                    if !matches!(c.kind, Kind::Button { .. }) {
                        self.err(
                            &format!("{at}.children[{i}]"),
                            "a code's strip holds buttons only",
                        );
                    }
                }
            }
            Kind::Log(log) => {
                if log.commands.len() > parts::MAX_COMMANDS {
                    self.err(
                        &format!("{at}.commands"),
                        format_args!(
                            "{} commands; a log holds {} at most",
                            log.commands.len(),
                            parts::MAX_COMMANDS
                        ),
                    );
                }
                for (i, c) in log.commands.iter().enumerate() {
                    if let Some(t) = &c.command {
                        self.text(&format!("{at}.commands[{i}]"), "command", t);
                    }
                }
            }
            Kind::Board { cards, empty } => {
                if let Some(e) = empty {
                    self.text(at, "empty", e);
                }
                for (i, c) in n.children.iter().enumerate() {
                    let p = format!("{at}.children[{i}]");
                    if !matches!(c.kind, Kind::Column { .. }) {
                        self.err(&p, "a board holds columns");
                    } else if cards.is_some() {
                        // The client places each card by the value its column's id names.
                        self.named(c, &p, "column of a board that reads its cards");
                        if !c.children.is_empty() {
                            self.err(&p, "holds no cards; the board reads them");
                        }
                    }
                }
            }
            _ => {}
        }
        if let Kind::Shape { ink: Some(p), .. } = &n.kind {
            self.paint(&format!("{at}.ink"), p);
        }
        for (i, c) in n.children.iter().enumerate() {
            self.node(c, &format!("{at}.children[{i}]"), depth + 1);
        }
    }
}
