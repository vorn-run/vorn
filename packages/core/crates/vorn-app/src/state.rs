//! The screen's own state, which vornd does not keep: filters, the layout,
//! the open menu or dialog, toasts. Kept apart from [`crate::app`] so the
//! screens can read it whole.

use std::time::{Duration, Instant};

use serde_json::Value;

use std::collections::{HashMap, HashSet};

use crate::client::{Session, Store};
use crate::screen::{Composer, MainView};
use vorn_protocol::TerminalSession;

/// The grid's status filter (`StatusFilter`), Alt+1..5.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    #[default]
    All,
    Running,
    Waiting,
    Idle,
    Error,
}

impl Filter {
    pub const ALL: [Filter; 5] = [
        Filter::All,
        Filter::Running,
        Filter::Waiting,
        Filter::Idle,
        Filter::Error,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Running => "Running",
            Filter::Waiting => "Waiting",
            Filter::Idle => "Idle",
            Filter::Error => "Error",
        }
    }

    pub fn keeps(self, s: &Session) -> bool {
        match self {
            Filter::All => true,
            Filter::Running => s.status() == "running",
            Filter::Waiting => s.status() == "waiting",
            Filter::Idle => s.status() == "idle",
            Filter::Error => s.status() == "error",
        }
    }
}

/// The grid's order (`SortMode`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Sort {
    #[default]
    Manual,
    Created,
    Recent,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Manual, Sort::Created, Sort::Recent];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Manual => "Manual",
            Sort::Created => "Created",
            Sort::Recent => "Recent",
        }
    }
}

/// Cards in a grid, or one at a time under tabs (`defaults.layoutMode`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LayoutMode {
    #[default]
    Grid,
    Tabs,
}

impl LayoutMode {
    pub fn from_config(config: &Value) -> LayoutMode {
        match config
            .pointer("/defaults/layoutMode")
            .and_then(Value::as_str)
        {
            Some("tabs") => LayoutMode::Tabs,
            _ => LayoutMode::Grid,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            LayoutMode::Grid => "grid",
            LayoutMode::Tabs => "tabs",
        }
    }
}

/// Which menu is open.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MenuKind {
    /// The toolbar's plus, or a right click on empty grid space.
    Grid,
    /// A card's More button.
    Card(String),
    /// A right click in a terminal.
    Terminal(String),
    /// The toolbar's grid options: filter, sort, layout, columns.
    ViewOptions,
    Recent,
    Headless,
    /// A project row's context menu in the sidebar.
    Project(String),
    /// The launcher's pickers.
    LaunchProject,
    LaunchAgent,
    LaunchWorktree,
    LaunchBranch,
    /// The Tasks toolbar's view options.
    TaskOptions,
    /// A task's More menu (kanban card or list row).
    TaskCard(String),
    /// The task form pickers.
    TaskStatus(crate::tasks::Form),
    TaskProject(crate::tasks::Form),
    TaskAgent(crate::tasks::Form),
    /// The landing's status filter.
    RunStatus,
    /// The sidebar's workflow filter.
    WorkflowFilter,
    /// A workflow row's More and context menu.
    WorkflowItem(String),
    /// The editor's More options.
    EditorMore,
    /// A config select, by the path it writes.
    WfChoice(String),
    /// The run prompt's project picker.
    PromptProject,
    /// The node panel's More menu.
    NodeMore(String),
}

impl MenuKind {
    /// The launcher's pickers, which open above their chip.
    pub fn is_launch(&self) -> bool {
        matches!(
            self,
            MenuKind::LaunchProject
                | MenuKind::LaunchAgent
                | MenuKind::LaunchWorktree
                | MenuKind::LaunchBranch
        )
    }
}

/// The open menu: what it is, where it opens, and its open submenu.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub kind: MenuKind,
    pub at: (f32, f32),
    pub sub: Option<usize>,
}

/// The dialog over everything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    Shortcuts,
    /// The add-project dialog, or editing the project with this name.
    Project(Option<String>),
    /// AddTaskDialog.
    AddTask,
    /// The task panel's CommitDialog.
    Commit,
    /// LogReplayModal: a workflow step's full output.
    Output,
    /// SourcePromptDialog: what a run asks for first.
    RunPrompt,
    /// MissedSchedulesDialog.
    Missed,
    /// The export or import path.
    WfFile,
}

/// A toast in the bottom right.
#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>, error: bool, now: Instant) -> Toast {
        Toast {
            text: text.into(),
            error,
            until: now + Duration::from_secs(4),
        }
    }
}

/// One of `sessions:getRecent`'s entries.
#[derive(Debug, Clone, PartialEq)]
pub struct Recent {
    pub session_id: String,
    pub agent: String,
    pub display: String,
    pub project_path: String,
    pub timestamp: f64,
    pub activity: u64,
    pub activity_label: String,
    pub can_resume: bool,
}

impl Recent {
    pub fn from_value(v: &Value) -> Option<Recent> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        Some(Recent {
            session_id: v.get("sessionId")?.as_str()?.to_owned(),
            agent: s("agentType"),
            display: s("display"),
            project_path: s("projectPath"),
            timestamp: v.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0),
            activity: v.get("activityCount").and_then(Value::as_u64).unwrap_or(0),
            activity_label: s("activityLabel"),
            can_resume: v
                .get("canResumeExact")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }
}

/// One of `headless:list`'s sessions.
#[derive(Debug, Clone, PartialEq)]
pub struct Headless {
    pub id: String,
    pub name: String,
    pub branch: Option<String>,
    pub status: String,
    pub exit: Option<i64>,
    pub started: f64,
}

impl Headless {
    pub fn from_value(v: &Value) -> Option<Headless> {
        let name = ["displayName", "name", "agentType"]
            .iter()
            .find_map(|k| v.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
            .unwrap_or("headless");
        Some(Headless {
            id: v.get("id")?.as_str()?.to_owned(),
            name: name.to_owned(),
            branch: v.get("branch").and_then(Value::as_str).map(str::to_owned),
            status: v
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("running")
                .to_owned(),
            exit: v.get("exitCode").and_then(Value::as_i64),
            started: v.get("createdAt").and_then(Value::as_f64).unwrap_or(0.0),
        })
    }
}

/// A git worktree of a project (`git:listWorktrees`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub name: String,
    pub main: bool,
}

impl Worktree {
    pub fn from_value(v: &Value) -> Option<Worktree> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        Some(Worktree {
            path: v.get("path")?.as_str()?.to_owned(),
            branch: s("branch"),
            name: s("name"),
            main: v.get("isMain").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// The add-project dialog's choices besides its two text fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectForm {
    pub icon: Option<String>,
    pub icon_color: Option<String>,
    /// The name last filled in from the folder, replaced while untouched.
    pub auto_name: String,
}

/// What the launcher is set to besides the prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launch {
    /// The agent picked, when not the default.
    pub agent: Option<String>,
    /// An existing worktree's path, or none for the project itself.
    pub worktree: Option<String>,
    /// A new worktree off `branch`.
    pub new_worktree: bool,
    pub branch: Option<String>,
}

/// Everything the screen shows that vornd does not keep.
#[derive(Debug, Default)]
pub struct State {
    pub view: MainView,
    pub selected: Option<String>,
    /// The card on the focused stage.
    pub focused: Option<String>,
    pub sidebar: bool,
    /// The grid's project filter (the sidebar's active project).
    pub project: Option<String>,
    pub filter: Filter,
    pub sort: Sort,
    /// 0 for the automatic layout, -1 flexible, else that many columns.
    pub columns: i32,
    pub layout: LayoutMode,
    pub menu: Option<Menu>,
    pub dialog: Option<Dialog>,
    /// The session whose title is being edited.
    pub renaming: Option<String>,
    /// The session whose close asks first.
    pub confirm_close: Option<String>,
    /// The project whose delete asks first.
    pub confirm_delete: Option<String>,
    pub toasts: Vec<Toast>,
    /// Sessions the restored banner offers.
    pub banner: Vec<TerminalSession>,
    pub restoring: bool,
    pub recent: Vec<Recent>,
    pub headless: Vec<Headless>,
    pub worktrees: HashMap<String, Vec<Worktree>>,
    pub branches: HashMap<String, Vec<String>>,
    pub composer: Composer,
    /// The launcher over the cards (Mod+N).
    pub composer_open: bool,
    pub launch: Launch,
    pub form: ProjectForm,
    /// Agents installed here, when known.
    pub installed: Option<Vec<String>>,
    /// When each session last drew, for the recent sort.
    pub last_output: HashMap<String, Instant>,
    /// Panes whose size this device does not drive.
    pub locked: HashSet<String>,
    /// Sidebar projects folded shut.
    pub collapsed: HashSet<String>,
    pub tasks: crate::tasks::Tasks,
    pub runs: crate::runs::RunView,
    pub wf: crate::workflows::Workflows,
}

impl State {
    /// The cards the grid shows, as indexes into `store.sessions`: the
    /// active project's, through the status filter, in the chosen order.
    pub fn visible(&self, store: &Store) -> Vec<usize> {
        let mut out: Vec<usize> = store
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                self.project
                    .as_deref()
                    .is_none_or(|p| s.info.project_name == p)
                    && self.filter.keeps(s)
            })
            .map(|(i, _)| i)
            .collect();
        match self.sort {
            Sort::Manual => {}
            Sort::Created => out.sort_by(|a, b| {
                let (a, b) = (&store.sessions[*a].info, &store.sessions[*b].info);
                b.created_at.total_cmp(&a.created_at)
            }),
            Sort::Recent => {
                let at = |i: &usize| self.last_output.get(&store.sessions[*i].info.id).copied();
                out.sort_by_key(|i| std::cmp::Reverse(at(i)));
            }
        }
        out
    }

    /// Whether the composer is what the sessions view shows.
    pub fn composing(&self, store: &Store) -> bool {
        self.view == MainView::Sessions
            && self.focused.is_none()
            && (store.sessions.is_empty() || self.composer_open)
    }

    /// Whether the composer floats over the cards rather than replacing them.
    pub fn composer_overlay(&self, store: &Store) -> bool {
        self.composing(store) && !store.sessions.is_empty()
    }

    /// Whether any view option differs from its default (the toolbar
    /// button's dot).
    pub fn options_changed(&self) -> bool {
        self.filter != Filter::All
            || self.sort != Sort::Manual
            || self.columns != 0
            || self.layout != LayoutMode::Grid
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast::new(text, error, Instant::now()));
    }
}

/// `timeAgo`: just now, 5m ago, 3h ago, 2d ago.
pub fn time_ago(then_ms: f64, now_ms: f64) -> String {
    let s = ((now_ms - then_ms) / 1000.0).max(0.0) as u64;
    match s {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// A duration as the headless pill shows it: 45s, 3m, 2h 5m.
pub fn short_duration(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

/// Milliseconds since the epoch, as vornd stamps things.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// The last path segment, as a project name from its folder.
pub fn basename(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn times_read_as_the_renderer_writes_them() {
        assert_eq!(time_ago(0.0, 30_000.0), "just now");
        assert_eq!(time_ago(0.0, 5.0 * 60_000.0), "5m ago");
        assert_eq!(time_ago(0.0, 3.0 * 3_600_000.0), "3h ago");
        assert_eq!(time_ago(0.0, 2.0 * 86_400_000.0), "2d ago");
        assert_eq!(short_duration(45_000.0), "45s");
        assert_eq!(short_duration(7_500_000.0), "2h 5m");
    }

    #[test]
    fn layout_and_folders_read_from_config_and_paths() {
        let c = json!({ "defaults": { "layoutMode": "tabs" } });
        assert_eq!(LayoutMode::from_config(&c), LayoutMode::Tabs);
        assert_eq!(LayoutMode::from_config(&json!({})), LayoutMode::Grid);
        assert_eq!(basename("/a/b/demo/"), "demo");
        assert_eq!(basename(r"C:\src\demo"), "demo");
    }

    #[test]
    fn recent_entries_need_an_id() {
        let r = Recent::from_value(&json!({ "sessionId": "s", "agentType": "claude" }));
        assert_eq!(r.map(|r| r.agent), Some("claude".to_owned()));
        assert!(Recent::from_value(&json!({ "agentType": "claude" })).is_none());
    }
}
