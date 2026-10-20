//! The parts a pane or a shell draws whose moment-to-moment state the client
//! keeps: a strip of tabs, an outline of rows, a split, a file's text being
//! edited, and a shell's log of commands. The tree says what they hold; the
//! client draws them, keeps hover, typing and dragging local, and reports
//! what was done as events.

use serde::{Deserialize, Serialize};

/// A strip of tabs with its tools. A tab `select`s on a click, `open`s (is
/// kept) on a double click and `close`s from its ✕; a tool `click`s with its
/// id. Past its width the strip offers every tab and `more` in a list.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Tabs {
    #[serde(default)]
    pub tabs: Vec<Tab>,
    /// What the strip says while it holds no tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<String>,
    /// Tools before the tabs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lead: Vec<Tool>,
    /// Tools after the tabs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    /// Entries under the tabs in the list of every tab.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more: Vec<Tool>,
}

/// One tab of a [`Tabs`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    pub label: String,
    /// A quieter word after the label, as the folder of a name two tabs share.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Draws the file-type icon of its label.
    #[serde(default, skip_serializing_if = "is_false")]
    pub file: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub active: bool,
    /// In italics: a preview the next one opened replaces until it is kept.
    #[serde(default, skip_serializing_if = "is_false")]
    pub preview: bool,
    /// Holds edits not saved: a dot where its ✕ is, until hovered.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dirty: bool,
    /// A kit icon before the label, as what kind of page a browser tab shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// A small control in a strip: an icon with its label as the tooltip, or a
/// line of a list. `on` draws it pressed.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Tool {
    pub id: String,
    pub label: String,
    /// A kit icon by name; a list line may go without.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub on: bool,
}

/// Rows in a tree of folders, as a file tree: a row `select`s on a click (a
/// folder opens or shuts), a leaf `open`s on a double click, and the
/// `action` shown on a hovered leaf `click`s as the action's id, with the
/// row's id as its value; `{}` in its label stands for the row's.
/// A `filter` field reports `change` with its text as it is typed.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Outline {
    #[serde(default)]
    pub rows: Vec<Line>,
    /// The row drawn as chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    /// Draws a spinner instead of the rows.
    #[serde(default, skip_serializing_if = "is_false")]
    pub loading: bool,
    /// What it says when it holds no row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<String>,
    /// A quiet line under the rows, as when a filter matched nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Tool>,
    /// Leaves draw the file-type icon of their label.
    #[serde(default, skip_serializing_if = "is_false")]
    pub files: bool,
}

/// The field over an [`Outline`] that narrows its rows.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub placeholder: String,
}

/// One row of an [`Outline`], `depth` folders in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub depth: u16,
    #[serde(flatten)]
    pub kind: LineKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "snake_case")]
pub enum LineKind {
    Leaf,
    /// A folder, open or shut; `busy` while what it holds is being read.
    Folder {
        #[serde(default)]
        open: bool,
        #[serde(default)]
        busy: bool,
    },
    /// Words in place of rows, as an open folder that holds nothing.
    Note,
}

/// How far a [`Kind::Split`](crate::Kind::Split)'s divider may go either way.
pub const MIN_SPLIT: f32 = 0.15;
pub const MAX_SPLIT: f32 = 0.85;

/// A file's text and how it may be edited. The client holds the edit: it
/// takes `draft`, else `text`, when `rev` changes, colours and finds in what
/// it holds, and reports `change` with the text when it starts or stops
/// differing from `text` (what is on disk) and once typing settles, `submit` with the text to save it, and
/// `click` with `discard`, `force` (save over a file that moved), `disk`
/// (take what is on disk) or `keep` (keep editing over a moved file).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Code {
    pub path: String,
    /// The strip shows `path` under this folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// What is on disk.
    #[serde(default)]
    pub text: String,
    /// An edit kept from before, to start from instead of `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    /// Changes whenever the client must take `draft` or `text` in place of what it holds.
    #[serde(default)]
    pub rev: u64,
    #[serde(default)]
    pub state: CodeState,
    /// The file changed on disk under the edit.
    #[serde(default, skip_serializing_if = "is_false")]
    pub conflict: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub saving: bool,
    /// What went wrong saving or reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What a [`Code`] can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeState {
    #[default]
    Loading,
    /// Text that can be typed in.
    Edit,
    /// Text shown but not editable: a capped read.
    Read,
    Binary,
    Unreadable,
}

/// A web page the client shows in a native web view, under the bar the client
/// draws for it (back, forward, reload and the address, or a review's
/// controls). `view` names the web view: the same view keeps its page, its
/// history and what it logged while the tree is drawn again, so one per
/// session's pane, as its id. The view loads `url` whenever `url` changes;
/// where the page wanders since is reported, not loaded again.
///
/// What the page does is reported as [`EventKind::Page`](crate::EventKind::Page)
/// on the node, its `value` a JSON object with a `type`: `state` (`url`,
/// `title` and, for a design, `design` with its title) whenever one
/// changes, `submit` (`text` typed in the address), `opened` (`url` a page
/// asked to open in a new window) and `go` (`url` the client's own controls
/// moved the page to, as another version of a review).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Web {
    pub view: String,
    pub url: String,
    /// A line over the page saying what went wrong, as an address that could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// A device's live screen the client draws itself, as a simulator's, under a
/// header (an icon, `title`, `tools`, the pane's own controls) and over a
/// bar (`controls`, then the zoom the client draws). `view` names what it is
/// shown for, one per session's pane as its id; `device` names the device
/// the client polls for frames and draws. Frames never travel in the tree:
/// the client fetches and paints them itself, for as long as the part shows.
/// A `picker` shows under the title, to choose the device from.
///
/// A click on a tool reports [`EventKind::Click`](crate::EventKind::Click)
/// with the tool's id, and a choice in the picker
/// [`EventKind::Select`](crate::EventKind::Select) with the choice's id. The
/// rest is reported as
/// [`EventKind::Page`](crate::EventKind::Page) on the node, its `value` a
/// JSON object with a `type`: `switch` (the title was clicked, or the open
/// picker dismissed), `dismiss`
/// (the notice's ✕), `zoom` (`zoom`, the scale chosen, absent to fit the
/// pane) and, for a part with `boot` set, `booted` (`bootedHere` when the
/// client started the device) or `failed` (`message`) once the client has
/// started `device` or could not.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Video {
    pub view: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default)]
    pub title: String,
    /// The title shows it is working, as while a device starts.
    #[serde(default, skip_serializing_if = "is_false")]
    pub busy: bool,
    /// The client starts `device` before it draws it, and reports how that went.
    #[serde(default, skip_serializing_if = "is_false")]
    pub boot: bool,
    /// Screen points per logical pixel; none fits the screen to the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
    /// A line over the screen saying what went wrong; its ✕ reports `dismiss`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    /// The header's tools, after the title.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    /// The bar's tools, before the zoom.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<Tool>,
    /// The list open under the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker: Option<Picker>,
}

/// What a [`Video`]'s title offers to show instead: its `heading`, then
/// what went wrong, `empty` with no choices, or the choices.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Picker {
    pub heading: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub loading: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
}

/// One device a [`Picker`] offers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub title: String,
    /// What tells it apart from another of the same title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Its icon's tone, as a running device's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<crate::Badge>,
    /// Shown, but not to be chosen, as one another session holds.
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
}

/// A shell's commands, one per prompt it reported, oldest first: the client
/// cuts each one's output from the shell's screen, pairing the newest prompt
/// with the newest command, and draws the last one while it runs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Log {
    #[serde(default)]
    pub commands: Vec<Command>,
}

/// What the shell said about one prompt.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Command {
    /// The command line; none when the shell did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The folder it ran in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// A command ran from the prompt; none did after an empty line or ^C.
    #[serde(default, skip_serializing_if = "is_false")]
    pub started: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ended: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// When it started, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub at: u64,
}

impl Command {
    /// Running now: started and not ended.
    pub fn running(&self) -> bool {
        self.started && !self.ended
    }
}

impl Log {
    /// The command running now, if one is: always the newest.
    pub fn running(&self) -> Option<&Command> {
        self.commands.last().filter(|c| c.running())
    }
}

/// Past these a part is a mistake, not a page.
pub const MAX_CODE: usize = 2 << 20;
pub const MAX_LINES: usize = 100_000;
pub const MAX_TABS: usize = 500;
pub const MAX_COMMANDS: usize = 1000;

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}
