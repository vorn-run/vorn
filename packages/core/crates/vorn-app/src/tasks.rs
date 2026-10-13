//! The Tasks view's model: the statuses as `task-status.ts` lists them, the
//! board's filters and sections, and each task mutation as the renderer's
//! tasks slice makes it, on the config that `config:save` writes back.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use serde_json::{json, Map, Value};
use vorn_protocol::TaskConfig;

use crate::look::{self, Rgb};

/// One task status: its key, label, lucide icon, colour and empty text.
pub struct Status {
    pub key: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    /// `TASK_STATUS_TEXT`; `None` is `text-ink-faint`.
    pub color: Option<Rgb>,
    pub empty: &'static str,
}

pub const SLATE: Rgb = look::STATUS_SLATE;
pub const BLUE: Rgb = look::STATUS_BLUE;
pub const SAGE: Rgb = look::STATUS_SAGE;

/// `TASK_STATUS_ORDER` with what each shows.
pub const STATUSES: [Status; 5] = [
    Status {
        key: "todo",
        label: "Todo",
        icon: "circle",
        color: Some(SLATE),
        empty: "No tasks in queue",
    },
    Status {
        key: "in_progress",
        label: "In Progress",
        icon: "clock",
        color: Some(BLUE),
        empty: "No active tasks",
    },
    Status {
        key: "in_review",
        label: "In Review",
        icon: "eye",
        color: Some(look::BRONZO),
        empty: "No tasks awaiting review",
    },
    Status {
        key: "done",
        label: "Done",
        icon: "circle-check",
        color: Some(SAGE),
        empty: "No completed tasks",
    },
    Status {
        key: "cancelled",
        label: "Cancelled",
        icon: "circle-x",
        color: None,
        empty: "No cancelled tasks",
    },
];

/// The status `key`, or todo for one this build does not know.
pub fn status(key: &str) -> &'static Status {
    STATUSES
        .iter()
        .find(|s| s.key == key)
        .unwrap_or(&STATUSES[0])
}

pub fn is_terminal(status: &str) -> bool {
    status == "done" || status == "cancelled"
}

/// `supportsExactSessionResume`.
pub fn resumable(agent: &str) -> bool {
    agent != "gemini" && agent != "shell"
}

/// `TASK_TEMPLATE`: what a new task's description starts as.
pub const TEMPLATE: &str = "## Description\nDescribe what needs to be done...\n\n## Acceptance Criteria\n- [ ] Criterion 1\n- [ ] Criterion 2\n\n## Notes\nAny additional context or references.\n";

/// `getTaskShortId`: three letters of the project and four of the id.
pub fn short_id(project: &str, id: &str) -> String {
    let prefix: String = project
        .chars()
        .filter(char::is_ascii_alphabetic)
        .take(3)
        .collect::<String>()
        .to_uppercase();
    let prefix = if prefix.is_empty() {
        "TSK".to_owned()
    } else {
        prefix
    };
    let suffix: String = id.chars().take(4).collect::<String>().to_uppercase();
    format!("{prefix}-{suffix}")
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The year, month, day, hour and minute of an ISO date.
fn parts(iso: &str) -> Option<(u32, usize, u32, u32, u32)> {
    let num = |r: std::ops::Range<usize>| iso.get(r)?.parse::<u32>().ok();
    let month = num(5..7)? as usize;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some((
        num(0..4)?,
        month,
        num(8..10)?,
        num(11..13).unwrap_or(0),
        num(14..16).unwrap_or(0),
    ))
}

/// `Date.toLocaleString()` in en-US: "1/1/2030, 9:00:00 AM".
pub fn locale_string(iso: &str) -> String {
    let sec = iso
        .get(17..19)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    match parts(iso) {
        Some((y, m, d, h, min)) => {
            let (h12, half) = match h {
                0 => (12, "AM"),
                1..=11 => (h, "AM"),
                12 => (12, "PM"),
                _ => (h - 12, "PM"),
            };
            format!("{m}/{d}/{y}, {h12}:{min:02}:{sec:02} {half}")
        }
        None => String::new(),
    }
}

/// `formatTaskDate`: "Mar 5".
pub fn short_date(iso: &str) -> String {
    match parts(iso) {
        Some((_, m, d, _, _)) => format!("{} {d}", MONTHS[m - 1]),
        None => String::new(),
    }
}

/// The detail panel's dates: "Mar 5, 02:30 PM".
pub fn long_date(iso: &str) -> String {
    match parts(iso) {
        Some((_, m, d, h, min)) => {
            let (h12, half) = match h {
                0 => (12, "AM"),
                1..=11 => (h, "AM"),
                12 => (12, "PM"),
                _ => (h - 12, "PM"),
            };
            format!("{} {d}, {h12:02}:{min:02} {half}", MONTHS[m - 1])
        }
        None => String::new(),
    }
}

/// `new Date().toISOString()` at `ms` since the epoch.
pub fn iso(ms: f64) -> String {
    let ms = ms.max(0.0) as i64;
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// The ms since the epoch of an ISO date, `iso`'s inverse.
pub fn ms(iso: &str) -> Option<f64> {
    let (y, m, d, h, min) = parts(iso)?;
    let sec: f64 = iso.get(17..19).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let frac: f64 = iso
        .get(19..)
        .and_then(|r| r.strip_prefix('.'))
        .map(|r| {
            r.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|d| format!("0.{d}").parse().ok())
        .unwrap_or(0.0);
    let (y, m) = (i64::from(y), m as i64);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + i64::from(h) * 3600 + i64::from(min) * 60;
    Some((secs as f64 + sec + frac) * 1000.0)
}

/// `Intl.DateTimeFormat().resolvedOptions().timeZone`: `TZ`, else the
/// zone `/etc/localtime` points at, else UTC.
pub fn local_zone() -> &'static str {
    static ZONE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ZONE.get_or_init(|| {
        if let Some(tz) = std::env::var("TZ").ok().filter(|t| t.contains('/')) {
            return tz.trim_start_matches(':').to_owned();
        }
        std::fs::read_link("/etc/localtime")
            .ok()
            .and_then(|p| {
                let s = p.to_string_lossy().into_owned();
                s.split_once("zoneinfo/").map(|(_, z)| z.to_owned())
            })
            .unwrap_or_else(|| "UTC".to_owned())
    })
}

pub fn now_iso() -> String {
    iso(crate::state::now_ms())
}

/// A random version-4 UUID, as `crypto.randomUUID()` makes the ids.
pub fn uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for (i, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::UNIX_EPOCH.elapsed().map_or(0, |d| d.as_nanos()));
        h.write_usize(i);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The board's source filter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Source {
    #[default]
    All,
    Local,
    Connector(String),
}

impl Source {
    pub fn keeps(&self, t: &TaskConfig) -> bool {
        match self {
            Source::All => true,
            Source::Local => t.source_connector_id.is_none(),
            Source::Connector(c) => t.source_connector_id.as_deref() == Some(c.as_str()),
        }
    }
}

/// List or Board (`defaults.taskViewMode`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    List,
    Board,
}

impl Mode {
    pub fn from_config(config: &Value) -> Mode {
        match config
            .pointer("/defaults/taskViewMode")
            .and_then(Value::as_str)
        {
            Some("kanban") => Mode::Board,
            _ => Mode::List,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mode::List => "list",
            Mode::Board => "kanban",
        }
    }
}

/// Which form a picker writes to: the add dialog's or the detail panel's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Form {
    Dialog,
    Detail,
}

/// The floating toolbar's marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mark {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
}

/// What a task control does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TaskAct {
    /// Selects a task and opens its detail panel.
    Open(String),
    CloseDetail,
    /// The add dialog, starting in a status.
    New(String),
    Edit(String),
    CloseDialog,
    /// The add dialog's "Expand to full panel".
    Expand,
    Submit,
    /// The detail panel's Create Task.
    Create,
    Filter(Option<String>),
    Source(Source),
    ToggleArchived,
    Mode(Mode),
    ToggleSection(String),
    SetStatus(Form, String),
    PickProject(Form, String),
    PickAgent(Form, Option<String>),
    ToggleWorktree(Form),
    Complete(String),
    Cancel(String),
    Reopen(String),
    Archive(String),
    Unarchive(String),
    AskDelete(String),
    Delete(String),
    /// Opens the task's detail at its changes.
    ReviewDiff(String),
    OpenSession(String),
    /// Shows the path field that attaches an image.
    AttachImages(Form),
    RemoveImage(Form, String),
    /// A slash command, by index into [`SLASH`].
    Slash(Form, usize),
    /// The rendered description clicked: edit its markdown.
    EditDesc(Form),
    /// A task item's box in the rendered description, by source line.
    CheckItem(Form, usize),
    Format(Form, Mark),
    /// "Add link": the toolbar asks for the address.
    StartLink(Form),
    ApplyLink,
    /// The full-output modal's Copy.
    CopyOutput,
    /// A kanban card dropped on a column.
    Drop(String, String),
    ToggleRuns,
    ToggleDiff,
    RefreshDiff,
    PickFile(Option<String>),
    /// A diff line clicked to comment on.
    Line(String, usize),
    AddComment,
    RemoveComment(usize),
    ClearComments,
    SendFeedback,
    CancelComment,
    OpenCommit,
    /// The commit dialog's "Include unstaged" switch.
    ToggleUnstaged,
    /// The commit dialog's next step: push after committing or not.
    CommitPush(bool),
    Commit,
    /// The sidebar's Tasks section folded or opened.
    ToggleProjects,
}

/// A slash command: title, description, icon and what it inserts at the
/// start of the line (`slash-commands.ts`).
pub const SLASH: [(&str, &str, &str, &str); 9] = [
    ("Heading 1", "Large section heading", "heading-1", "# "),
    ("Heading 2", "Medium section heading", "heading-2", "## "),
    ("Heading 3", "Small section heading", "heading-3", "### "),
    ("Bullet List", "Unordered list of items", "list", "- "),
    (
        "Numbered List",
        "Ordered list of items",
        "list-ordered",
        "1. ",
    ),
    (
        "Task List",
        "Checklist with checkboxes",
        "square-check",
        "- [ ] ",
    ),
    ("Code Block", "Fenced code snippet", "code", "```\n"),
    ("Blockquote", "Quoted text block", "quote", "> "),
    ("Divider", "Horizontal separator line", "minus", "---\n"),
];

/// `filterSlashCommands`: the commands whose title or description holds
/// `query`, as indexes into [`SLASH`].
pub fn slash_matches(query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    (0..SLASH.len())
        .filter(|&i| {
            q.is_empty()
                || SLASH[i].0.to_lowercase().contains(&q)
                || SLASH[i].1.to_lowercase().contains(&q)
        })
        .collect()
}

/// The slash query being typed at `caret`: the text after a `/` that
/// starts the line or follows a space, with no space in it.
pub fn slash_query(text: &str, caret: usize) -> Option<(usize, String)> {
    let before = text.get(..caret)?;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = &before[line_start..];
    let slash = line.rfind('/')?;
    if slash > 0 && !line[..slash].ends_with(' ') {
        return None;
    }
    let query = &line[slash + 1..];
    if query.contains(' ') {
        return None;
    }
    Some((line_start + slash, query.to_owned()))
}

/// Runs slash command `index` over the `/query` at `start..caret`: the
/// query goes and the line gets the command's prefix. The new text and
/// caret.
pub fn apply_slash(text: &str, start: usize, caret: usize, index: usize) -> (String, usize) {
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str(&text[..start]);
    let rest = &text[caret..];
    let line_start = out.rfind('\n').map_or(0, |i| i + 1);
    let prefix = SLASH[index].3;
    let caret = if index == 6 {
        // The fence closes under the caret.
        out.push_str("```\n");
        let at = out.len();
        out.push_str("\n```");
        at
    } else if index == 8 {
        out.push_str("---\n");
        out.len()
    } else {
        out.insert_str(line_start, prefix);
        out.len()
    };
    out.push_str(rest);
    (out, caret)
}

/// Wraps `range` in the mark's markdown; a link asks for its address.
pub fn apply_mark(text: &str, range: std::ops::Range<usize>, mark: Mark) -> (String, usize) {
    let (open, close) = match mark {
        Mark::Bold => ("**", "**"),
        Mark::Italic => ("*", "*"),
        Mark::Strike => ("~~", "~~"),
        Mark::Code => ("`", "`"),
        Mark::Link => {
            let end = range.end;
            return unlink(text, range).unwrap_or_else(|| (text.to_owned(), end));
        }
    };
    let mut out = String::with_capacity(text.len() + 12);
    out.push_str(&text[..range.start]);
    out.push_str(open);
    out.push_str(&text[range.clone()]);
    out.push_str(close);
    let caret = out.len();
    out.push_str(&text[range.end..]);
    (out, caret)
}

/// The markdown link `[label](url)` that `range` lies in, as its span.
pub fn link_at(text: &str, range: &std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
    let start = range.start.min(text.len());
    let upto = start + usize::from(text[start..].starts_with('['));
    let open = text[..upto].rfind('[')?;
    let mid = open + text[open..].find("](")?;
    let close = mid + text[mid..].find(')')?;
    (range.end <= close + 1 && !text[open..close].contains('\n')).then_some(open..close + 1)
}

/// "Remove link": the link around `range` back to its label.
pub fn unlink(text: &str, range: std::ops::Range<usize>) -> Option<(String, usize)> {
    let span = link_at(text, &range)?;
    let mid = span.start + text[span.clone()].find("](")?;
    let mut out = text[..span.start].to_owned();
    out.push_str(&text[span.start + 1..mid]);
    let caret = out.len();
    out.push_str(&text[span.end..]);
    Some((out, caret))
}

/// "Apply": the selection made a link to `url`, `https://` added when it
/// has no scheme.
pub fn link(text: &str, range: std::ops::Range<usize>, url: &str) -> (String, usize) {
    let url = url.trim();
    let url = if url.contains("://") || url.starts_with("mailto:") {
        url.to_owned()
    } else {
        format!("https://{url}")
    };
    let mut out = text[..range.start].to_owned();
    out.push('[');
    out.push_str(&text[range.clone()]);
    out.push_str("](");
    out.push_str(&url);
    out.push(')');
    let caret = out.len();
    out.push_str(&text[range.end..]);
    (out, caret)
}

/// Enter in the editor: a new line that carries on the list it is in.
pub fn newline(text: &str, caret: usize) -> (String, usize) {
    let before = &text[..caret];
    let line = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
    let carry = ["- [ ] ", "- [x] ", "- ", "* ", "> "]
        .into_iter()
        .find(|p| line.starts_with(p))
        .map(|p| if p == "- [x] " { "- [ ] " } else { p }.to_owned())
        .or_else(|| {
            let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
            let n: u32 = digits.parse().ok()?;
            line[digits.len()..]
                .starts_with(". ")
                .then(|| format!("{}. ", n + 1))
        });
    let insert = match carry {
        // An empty item ends the list.
        Some(c) if line.trim_end() == c.trim_end() || line.len() <= c.len() => {
            let mut out = text[..caret - line.len()].to_owned();
            out.push('\n');
            let at = out.len();
            out.push_str(&text[caret..]);
            return (out, at);
        }
        Some(c) => format!("\n{c}"),
        None => "\n".to_owned(),
    };
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(before);
    out.push_str(&insert);
    let at = out.len();
    out.push_str(&text[caret..]);
    (out, at)
}

/// The fields a task form holds besides its title and description, which
/// live in their text fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskForm {
    /// The task being edited; the id images are saved under for a new one.
    pub id: String,
    pub editing: bool,
    pub status: String,
    pub project: String,
    pub agent: Option<String>,
    pub branch: String,
    pub worktree: bool,
    pub images: Vec<String>,
    /// The image path field is showing.
    pub attaching: bool,
}

/// One review comment on a diff line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub file: String,
    pub line: usize,
    pub content: String,
    pub text: String,
}

/// One file of `git:diffFull`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub status: String,
    pub insertions: u64,
    pub deletions: u64,
    pub diff: String,
}

impl DiffFile {
    pub fn from_value(v: &Value) -> Option<DiffFile> {
        Some(DiffFile {
            path: v.get("filePath")?.as_str()?.to_owned(),
            status: v
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            insertions: v.get("insertions").and_then(Value::as_u64).unwrap_or(0),
            deletions: v.get("deletions").and_then(Value::as_u64).unwrap_or(0),
            diff: v
                .get("diff")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        })
    }
}

/// `formatReviewFeedback`.
pub fn feedback(comments: &[Comment]) -> String {
    let mut out = "Please address the following review comments:\n\n".to_owned();
    let mut files: Vec<&str> = Vec::new();
    for c in comments {
        if !files.contains(&c.file.as_str()) {
            files.push(&c.file);
        }
    }
    for f in files {
        out.push_str(&format!("**{f}:**\n"));
        for c in comments.iter().filter(|c| c.file == f) {
            let code: String = c.content.chars().skip(1).collect();
            out.push_str(&format!("- Line `{}`: {}\n", code.trim(), c.text));
        }
        out.push('\n');
    }
    out
}

/// The detail panel's changes section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// The directory diffed.
    pub cwd: String,
    pub loading: bool,
    pub files: Vec<DiffFile>,
    pub file: Option<String>,
    pub commenting: Option<(String, usize, String)>,
    pub comments: Vec<Comment>,
}

/// The commit dialog (`CommitDialog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub unstaged: bool,
    pub push: bool,
    pub loading: bool,
    pub error: Option<String>,
}

impl Default for Commit {
    fn default() -> Commit {
        Commit {
            unstaged: true,
            push: false,
            loading: false,
            error: None,
        }
    }
}

/// What the Tasks view keeps besides vornd's tasks.
#[derive(Debug, Default)]
pub struct Tasks {
    pub mode: Mode,
    /// The status filter; none is All.
    pub filter: Option<String>,
    pub source: Source,
    pub archived: bool,
    /// List sections folded shut, by status.
    pub collapsed: HashSet<String>,
    /// The task in the detail panel; `new` is the create form.
    pub selected: Option<String>,
    pub dialog_form: TaskForm,
    pub detail_form: TaskForm,
    /// When the detail form last changed and has not been saved.
    pub dirty: Option<Instant>,
    pub confirm_delete: Option<String>,
    pub diff: Diff,
    /// The task and directory the diff is of (`diffKey`).
    pub diff_key: Option<String>,
    pub diff_open: bool,
    pub runs_open: bool,
    /// Workflow runs that touched the selected task.
    pub runs: Vec<Value>,
    /// The logs the full-output modal shows.
    pub output: String,
    /// When the full output was last copied.
    pub copied: Option<Instant>,
    /// Image paths by file name, for thumbnails.
    pub image_paths: HashMap<String, String>,
    /// Decoded thumbnails waiting to be registered with the UI.
    pub decoded: Vec<(String, u32, u32, Vec<u8>)>,
    /// Registered thumbnails by file name.
    pub thumbs: HashMap<String, u32>,
    pub commit: Commit,
    /// The sidebar's Tasks section is folded.
    pub projects_folded: bool,
    /// The slash menu's highlighted entry.
    pub slash: usize,
    /// Where the slash query Escape dismissed starts.
    pub slash_closed: Option<usize>,
    /// How far left of the caret the open `/` sits, for the menu under it.
    pub slash_dx: f32,
    /// The selection a link is being typed for.
    pub linking: Option<(Form, std::ops::Range<usize>)>,
    /// Where the link bar floats: the caret when it opened.
    pub link_pos: (f32, f32),
    /// The kanban card being dragged and the column under it.
    pub dragging: Option<(String, Option<String>)>,
}

impl Tasks {
    pub fn new(config: &Value) -> Tasks {
        Tasks {
            mode: Mode::from_config(config),
            diff_open: true,
            runs_open: true,
            ..Tasks::default()
        }
    }

    /// Whether a filter differs from its default (the toolbar's dot).
    pub fn options_changed(&self) -> bool {
        self.filter.is_some()
            || self.mode != Mode::List
            || self.source != Source::All
            || self.archived
    }

    /// The form a picker in `form` writes to.
    pub fn form(&mut self, form: Form) -> &mut TaskForm {
        match form {
            Form::Dialog => &mut self.dialog_form,
            Form::Detail => &mut self.detail_form,
        }
    }

    pub fn form_ref(&self, form: Form) -> &TaskForm {
        match form {
            Form::Dialog => &self.dialog_form,
            Form::Detail => &self.detail_form,
        }
    }

    /// The tasks the board shows: the project's (or the workspace's),
    /// unarchived unless asked, through the status and source filters.
    pub fn visible<'a>(
        &self,
        tasks: &'a [TaskConfig],
        project: Option<&str>,
        projects: &[&str],
    ) -> Vec<&'a TaskConfig> {
        tasks
            .iter()
            .filter(|t| match project {
                Some(p) => t.project_name == p,
                None => projects.contains(&t.project_name.as_str()),
            })
            .filter(|t| self.archived || t.archived_at.is_none())
            .filter(|t| {
                self.filter
                    .as_deref()
                    .is_none_or(|f| t.status.as_str() == f)
            })
            .filter(|t| self.source.keeps(t))
            .collect()
    }
}

/// The connectors tasks came from, for the source filter.
pub fn connectors(tasks: &[TaskConfig]) -> Vec<String> {
    let mut out: Vec<String> = tasks
        .iter()
        .filter_map(|t| t.source_connector_id.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The tasks of `status` among `tasks`, by their order.
pub fn in_status<'a>(tasks: &[&'a TaskConfig], status: &str) -> Vec<&'a TaskConfig> {
    let mut out: Vec<&TaskConfig> = tasks
        .iter()
        .copied()
        .filter(|t| t.status.as_str() == status)
        .collect();
    out.sort_by(|a, b| a.order.total_cmp(&b.order));
    out
}

fn tasks_mut(config: &mut Value) -> &mut Vec<Value> {
    if !config.get("tasks").is_some_and(Value::is_array) {
        config["tasks"] = json!([]);
    }
    match config.get_mut("tasks") {
        Some(Value::Array(list)) => list,
        _ => unreachable!("tasks was just made an array"),
    }
}

/// Changes the task `id` in `config` with `f`, stamping `updatedAt`.
/// False when there is no such task.
pub fn patch(
    config: &mut Value,
    id: &str,
    now: &str,
    f: impl FnOnce(&mut Map<String, Value>),
) -> bool {
    let Some(Value::Object(t)) = tasks_mut(config)
        .iter_mut()
        .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
    else {
        return false;
    };
    f(t);
    t.insert("updatedAt".into(), json!(now));
    true
}

fn set(t: &mut Map<String, Value>, k: &str, v: Option<Value>) {
    match v {
        Some(v) => {
            t.insert(k.into(), v);
        }
        None => {
            t.remove(k);
        }
    }
}

/// A status change as the slice makes it (`completeTask`, `reviewTask`,
/// `cancelTask`, `reopenTask`, or `updateTask` to in progress).
pub fn set_status(config: &mut Value, id: &str, status: &str, now: &str) -> bool {
    patch(config, id, now, |t| {
        t.insert("status".into(), json!(status));
        match status {
            "done" | "cancelled" => {
                t.insert("completedAt".into(), json!(now));
                t.remove("assignedSessionId");
            }
            "in_review" => {
                t.remove("archivedAt");
                t.remove("assignedSessionId");
            }
            "todo" => {
                for k in [
                    "completedAt",
                    "archivedAt",
                    "assignedSessionId",
                    "assignedAgent",
                ] {
                    t.remove(k);
                }
            }
            _ => {
                t.remove("archivedAt");
            }
        }
    })
}

/// `startTask`: the task in progress in `session`.
pub fn start(
    config: &mut Value,
    id: &str,
    session: &str,
    agent: &str,
    worktree: Option<&str>,
    now: &str,
) -> bool {
    patch(config, id, now, |t| {
        t.insert("status".into(), json!("in_progress"));
        t.insert("assignedSessionId".into(), json!(session));
        t.insert("assignedAgent".into(), json!(agent));
        if let Some(w) = worktree {
            t.insert("worktreePath".into(), json!(w));
        }
        t.remove("archivedAt");
    })
}

/// `archiveTask` and `unarchiveTask`: only finished work is filed away.
pub fn set_archived(config: &mut Value, id: &str, archived: bool, now: &str) -> bool {
    let Some(t) = config.get("tasks").and_then(Value::as_array).and_then(|l| {
        l.iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
    }) else {
        return false;
    };
    let status = t.get("status").and_then(Value::as_str).unwrap_or("");
    let is_archived = t.get("archivedAt").is_some_and(|a| !a.is_null());
    if archived == is_archived || (archived && !is_terminal(status)) {
        return false;
    }
    patch(config, id, now, |t| {
        set(t, "archivedAt", archived.then(|| json!(now)));
    })
}

pub fn remove(config: &mut Value, id: &str) -> bool {
    let list = tasks_mut(config);
    let before = list.len();
    list.retain(|t| t.get("id").and_then(Value::as_str) != Some(id));
    list.len() != before
}

/// The form's fields onto a task, as `updateTask` gets them; `useWorktree`
/// and the rest go when unset.
pub fn apply_form(t: &mut Map<String, Value>, f: &TaskForm, title: &str, description: &str) {
    t.insert("title".into(), json!(title.trim()));
    t.insert("projectName".into(), json!(f.project));
    t.insert("description".into(), json!(description.trim()));
    let branch = f.branch.trim();
    set(t, "branch", (!branch.is_empty()).then(|| json!(branch)));
    set(t, "useWorktree", f.worktree.then(|| json!(true)));
    set(t, "assignedAgent", f.agent.as_ref().map(|a| json!(a)));
    set(t, "images", (!f.images.is_empty()).then(|| json!(f.images)));
}

/// `addTask`: a new task from the form, ordered last among its project's
/// tasks of `status`.
pub fn add(config: &mut Value, f: &TaskForm, title: &str, description: &str, now: &str) {
    let order = config
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| {
            t.get("projectName").and_then(Value::as_str) == Some(f.project.as_str())
                && t.get("status").and_then(Value::as_str) == Some(f.status.as_str())
        })
        .count();
    let mut t = Map::new();
    t.insert("id".into(), json!(f.id));
    t.insert("status".into(), json!(f.status));
    t.insert("order".into(), json!(order));
    t.insert("createdAt".into(), json!(now));
    t.insert("updatedAt".into(), json!(now));
    apply_form(&mut t, f, title, description);
    tasks_mut(config).push(Value::Object(t));
}

/// The form for an existing task.
pub fn form_of(t: &TaskConfig) -> TaskForm {
    TaskForm {
        id: t.id.clone(),
        editing: true,
        status: t.status.to_string(),
        project: t.project_name.clone(),
        agent: t.assigned_agent.as_ref().map(|a| a.to_string()),
        branch: t.branch.clone().unwrap_or_default(),
        worktree: t.use_worktree.unwrap_or(false),
        images: t.images.clone(),
        attaching: false,
    }
}

/// The kanban drop rules (`handleKanbanDrop`): the status change a card
/// dropped from `from` onto `to` makes, if any.
pub fn drop_status(from: &str, to: &str) -> Option<&'static str> {
    if from == to || (to == "in_progress" && from != "todo") {
        return None;
    }
    STATUSES.iter().find(|s| s.key == to).map(|s| s.key)
}

/// The toast a status change from the menus says.
pub fn status_toast(status: &str) -> &'static str {
    match status {
        "todo" => "Task reopened",
        "in_review" => "Task moved to review",
        "done" => "Task completed",
        "cancelled" => "Task cancelled",
        _ => "Task updated",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, project: &str, status: &str, order: f64) -> TaskConfig {
        serde_json::from_value(json!({
            "id": id, "projectName": project, "title": id, "description": "",
            "status": status, "order": order, "createdAt": "2025-03-05T14:30:00.000Z",
            "updatedAt": "2025-03-05T14:30:00.000Z",
        }))
        .unwrap()
    }

    #[test]
    fn short_ids_and_dates_read_as_the_renderer_writes_them() {
        assert_eq!(short_id("my-web app", "abcd1234"), "MYW-ABCD");
        assert_eq!(short_id("123", "zz"), "TSK-ZZ");
        assert_eq!(short_date("2025-03-05T14:30:00.000Z"), "Mar 5");
        assert_eq!(long_date("2025-03-05T14:30:00.000Z"), "Mar 5, 02:30 PM");
        assert_eq!(long_date("2025-12-01T00:05:00Z"), "Dec 1, 12:05 AM");
        assert_eq!(short_date("nope"), "");
        assert_eq!(iso(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_741_185_000_123.0), "2025-03-05T14:30:00.123Z");
    }

    #[test]
    fn uuids_are_version_four_and_differ() {
        let (a, b) = (uuid(), uuid());
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert_ne!(a, b);
    }

    #[test]
    fn the_board_filters_by_project_archive_status_and_source() {
        let mut archived = task("c", "web", "done", 0.0);
        archived.archived_at = Some("x".into());
        let mut synced = task("d", "web", "todo", 1.0);
        synced.source_connector_id = Some("github".into());
        let tasks = vec![
            task("a", "web", "todo", 2.0),
            task("b", "api", "todo", 0.0),
            archived,
            synced,
        ];
        let mut b = Tasks::default();
        let ids = |v: Vec<&TaskConfig>| v.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(b.visible(&tasks, Some("web"), &[])), ["a", "d"]);
        assert_eq!(ids(b.visible(&tasks, None, &["api"])), ["b"]);
        b.archived = true;
        assert_eq!(ids(b.visible(&tasks, Some("web"), &[])), ["a", "c", "d"]);
        b.source = Source::Local;
        assert_eq!(ids(b.visible(&tasks, Some("web"), &[])), ["a", "c"]);
        b.source = Source::Connector("github".into());
        b.filter = Some("todo".into());
        assert_eq!(ids(b.visible(&tasks, Some("web"), &[])), ["d"]);
        assert!(b.options_changed());
        let all = b.visible(&tasks, Some("web"), &[]);
        assert_eq!(connectors(&tasks), ["github"]);
        assert_eq!(ids(in_status(&all, "todo")), ["d"]);
        let web: Vec<&TaskConfig> = tasks.iter().filter(|t| t.project_name == "web").collect();
        assert_eq!(ids(in_status(&web, "todo")), ["d", "a"]);
    }

    #[test]
    fn status_changes_stamp_and_clear_as_the_slice_does() {
        let mut config = json!({ "tasks": [{ "id": "a", "status": "in_progress",
            "assignedSessionId": "s", "assignedAgent": "claude" }] });
        assert!(set_status(&mut config, "a", "done", "T1"));
        let t = &config["tasks"][0];
        assert_eq!(
            (t["status"].as_str(), t["completedAt"].as_str()),
            (Some("done"), Some("T1"))
        );
        assert!(t.get("assignedSessionId").is_none());
        assert!(set_archived(&mut config, "a", true, "T2"));
        assert!(!set_archived(&mut config, "a", true, "T3"));
        assert!(set_status(&mut config, "a", "todo", "T4"));
        let t = &config["tasks"][0];
        for k in ["completedAt", "archivedAt", "assignedAgent"] {
            assert!(t.get(k).is_none(), "{k}");
        }
        assert!(!set_archived(&mut config, "a", true, "T5"));
        assert!(!set_status(&mut config, "zz", "done", "T6"));
        assert!(remove(&mut config, "a"));
        assert!(!remove(&mut config, "a"));
    }

    #[test]
    fn new_tasks_go_last_in_their_status_with_the_form_fields() {
        let mut config = json!({ "tasks": [
            { "id": "x", "projectName": "web", "status": "todo" },
            { "id": "y", "projectName": "web", "status": "done" },
        ] });
        let f = TaskForm {
            id: "n".into(),
            status: "todo".into(),
            project: "web".into(),
            agent: Some("codex".into()),
            branch: " feat ".into(),
            ..TaskForm::default()
        };
        add(&mut config, &f, " Title ", "Body\n", "T");
        let t = &config["tasks"][2];
        assert_eq!(t["order"], json!(1));
        assert_eq!(t["title"], json!("Title"));
        assert_eq!(t["description"], json!("Body"));
        assert_eq!(t["branch"], json!("feat"));
        assert_eq!(t["assignedAgent"], json!("codex"));
        assert!(t.get("useWorktree").is_none() && t.get("images").is_none());
    }

    #[test]
    fn slash_commands_filter_and_rewrite_the_line() {
        assert_eq!(slash_matches("").len(), 9);
        assert_eq!(slash_matches("head"), [0, 1, 2]);
        assert_eq!(slash_matches("check"), [5]);
        assert_eq!(slash_query("abc\n/he", 7), Some((4, "he".into())));
        assert_eq!(slash_query("a /x", 4), Some((2, "x".into())));
        assert_eq!(slash_query("a/x", 3), None);
        assert_eq!(slash_query("/a b", 4), None);
        let (t, c) = apply_slash("x\n/h2", 2, 5, 1);
        assert_eq!((t.as_str(), c), ("x\n## ", 5));
        let (t, c) = apply_slash("/co", 0, 3, 6);
        assert_eq!((t.as_str(), c), ("```\n\n```", 4));
    }

    #[test]
    fn marks_wrap_the_selection_and_lists_carry_on() {
        assert_eq!(apply_mark("a b c", 2..3, Mark::Bold).0, "a **b** c");
        assert_eq!(link("a go", 2..4, "x.dev").0, "a [go](https://x.dev)");
        assert_eq!(link("go", 0..2, "http://y").0, "[go](http://y)");
        assert_eq!(unlink("a [go](u) b", 3..5), Some(("a go b".to_owned(), 4)));
        assert_eq!(apply_mark("a [go](u)", 3..5, Mark::Link).0, "a go");
        assert_eq!(unlink("plain", 0..2), None);
        assert_eq!(newline("- one", 5), ("- one\n- ".to_owned(), 8));
        assert_eq!(newline("- [x] a", 7).0, "- [x] a\n- [ ] ");
        assert_eq!(newline("3. c", 4).0, "3. c\n4. ");
        assert_eq!(newline("x\n- ", 4), ("x\n\n".to_owned(), 3));
        assert_eq!(newline("plain", 5).0, "plain\n");
    }

    #[test]
    fn kanban_drops_follow_the_board_rules() {
        assert_eq!(drop_status("todo", "in_progress"), Some("in_progress"));
        assert_eq!(drop_status("done", "in_progress"), None);
        assert_eq!(drop_status("todo", "todo"), None);
        assert_eq!(drop_status("in_progress", "done"), Some("done"));
        assert_eq!(drop_status("done", "todo"), Some("todo"));
        assert_eq!(drop_status("todo", "nope"), None);
    }

    #[test]
    fn feedback_groups_comments_by_file() {
        let c = |f: &str, t: &str| Comment {
            file: f.into(),
            line: 0,
            content: "+ let x = 1;".into(),
            text: t.into(),
        };
        let out = feedback(&[c("a.rs", "one"), c("b.rs", "two"), c("a.rs", "three")]);
        assert_eq!(
            out,
            "Please address the following review comments:\n\n**a.rs:**\n- Line `let x = 1;`: one\n- Line `let x = 1;`: three\n\n**b.rs:**\n- Line `let x = 1;`: two\n\n"
        );
    }
}
