//! The screen's own state, which vornd does not keep: filters, the layout,
//! the open menu or dialog, toasts. Kept apart from [`crate::app`] so the
//! screens can read it whole.

use web_time::{Duration, Instant};

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

/// How the sessions view's sidebar arranges its sessions
/// (`SidebarViewMode`), worktrees and their sessions by default.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum SidebarMode {
    #[serde(rename = "worktrees")]
    Worktrees,
    #[default]
    #[serde(rename = "worktrees-sessions")]
    WorktreesSessions,
    #[serde(rename = "sessions")]
    Projects,
    #[serde(rename = "groups-sessions")]
    Groups,
    #[serde(rename = "sessions-flat")]
    Flat,
}

impl SidebarMode {
    pub const ALL: [SidebarMode; 5] = [
        SidebarMode::Worktrees,
        SidebarMode::WorktreesSessions,
        SidebarMode::Projects,
        SidebarMode::Groups,
        SidebarMode::Flat,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SidebarMode::Worktrees => "Projects > Worktrees",
            SidebarMode::WorktreesSessions => "Projects > Worktrees > Sessions",
            SidebarMode::Projects => "Projects > Sessions",
            SidebarMode::Groups => "Groups > Sessions",
            SidebarMode::Flat => "Sessions (Flat)",
        }
    }
}

/// The workspace being created (`id` none) or edited, and its icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceForm {
    pub id: Option<String>,
    pub icon: String,
    pub color: String,
    /// The icon and colour picker is showing.
    pub picker: bool,
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
    /// The sidebar's Filter & sort.
    SidebarOptions,
    /// A session group's More menu.
    Group(String),
    /// A sidebar session's More menu: the groups it can move to.
    SessionGroups(String),
    /// A group's icon and colour picker.
    GroupIcon(String),
    /// The sidebar header's workspace switcher.
    Workspace,
    /// The launcher's pickers.
    LaunchProject,
    LaunchAgent,
    LaunchModel,
    LaunchWorktree,
    LaunchBranch,
    /// The Tasks toolbar's view options.
    TaskOptions,
    /// The task form pickers.
    TaskStatus(crate::tasks::Form),
    TaskProject(crate::tasks::Form),
    TaskAgent(crate::tasks::Form),
    /// Settings › Extensions' Install…: from Git, a folder, or a linked folder.
    ExtInstall,
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
    /// The editor's icon and colour picker.
    WfIcon,
    /// The settings shell picker's list.
    SetShell,
    /// A settings `<select>`, by the default it writes.
    SetSelect(String),
    /// A host's stored-key select.
    SshKey(String),
    /// A session's branch switcher.
    Branch(String),
    /// The editors a session's project opens in.
    OpenIn(String),
    /// A Files pane's list of every open tab.
    FileTabs(String),
}

impl MenuKind {
    /// The menus that open above their control.
    pub fn opens_up(&self) -> bool {
        self.is_launch() || matches!(self, MenuKind::OpenIn(_))
    }

    /// The launcher's pickers.
    pub fn is_launch(&self) -> bool {
        matches!(
            self,
            MenuKind::LaunchProject
                | MenuKind::LaunchAgent
                | MenuKind::LaunchModel
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
    pub sub: crate::ui::menu::Cascade,
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
    /// GateReviewModal: the review page of a waiting gate.
    GateReview,
    /// The Changes panel's CommitDialog.
    GitCommit,
    /// "Discard unsaved changes?" before a buffer goes.
    Discard,
    /// "Paste N lines?" before a paste the program could run line by line.
    Paste,
    /// CommandPalette.
    Palette,
    /// Install…'s address or folder, and the sheet read before any extension installs or links.
    ExtInstall,
    /// OnboardingModal: the welcome guide.
    Welcome,
    /// A connection's sign-in page, in its own profile.
    SignIn,
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
            // `DEFAULT_DURATIONS`: errors stay 4 s, the rest 2.5 s.
            until: now + Duration::from_millis(if error { 4000 } else { 2500 }),
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
    /// The terminal writing this conversation now, when one is.
    pub running_in: Option<String>,
}

impl Recent {
    /// `formatRecentSessionActivity`: "3 messages".
    pub fn activity_text(&self) -> String {
        let label = if self.activity_label.is_empty() {
            "message"
        } else {
            self.activity_label.as_str()
        };
        let plural = if self.activity == 1 { "" } else { "s" };
        format!("{} {label}{plural}", self.activity)
    }

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
            running_in: v
                .get("runningIn")
                .and_then(Value::as_str)
                .map(str::to_owned),
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
    /// The remote host picked; none for local.
    pub host: Option<String>,
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

/// The launcher's model picker: each agent's model and what was learned
/// of the models it lists.
#[derive(Debug, Default)]
pub struct Models {
    /// The model picked per agent; kept across launches, as today's
    /// launch preferences.
    pub picked: HashMap<String, String>,
    /// By agent and project path.
    pub lists: HashMap<(String, String), crate::models::Listing>,
    /// Why the typed id was refused.
    pub invalid: Option<&'static str>,
    /// When the menu opened (ms), which the footer's age is measured from.
    pub opened_at: u64,
}

/// A saved layout in `layout:list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRow {
    pub name: String,
    /// `user`, or the id of the extension that offers it.
    pub source: String,
    pub sessions: u64,
}

impl LayoutRow {
    /// A row that read; one with an error is left out of the palette.
    pub fn from_value(v: &Value) -> Option<LayoutRow> {
        if v.get("error").is_some_and(|e| !e.is_null()) {
            return None;
        }
        Some(LayoutRow {
            name: v.get("name")?.as_str()?.to_owned(),
            source: v.get("source")?.as_str()?.to_owned(),
            sessions: v.get("sessions").and_then(Value::as_u64).unwrap_or(0),
        })
    }
}

/// Everything the screen shows that vornd does not keep.
#[derive(Debug, Default)]
pub struct State {
    pub view: MainView,
    pub selected: Option<String>,
    /// The card on the focused stage.
    pub focused: Option<String>,
    pub sidebar: bool,
    /// Dragged by its right edge, between 180 and 400; 256 until then.
    pub sidebar_width: Option<f32>,
    /// The grid's project filter (the sidebar's active project).
    pub project: Option<String>,
    /// The grid's group filter (the sidebar's active group).
    pub group: Option<String>,
    /// The grid's worktree filter inside `project`: a worktree path, or `""`
    /// for the main checkout.
    pub worktree: Option<String>,
    /// Sidebar worktrees whose sessions are folded, as project + path.
    pub worktrees_folded: HashSet<(String, String)>,
    /// The sessions sidebar's arrangement, order and filter.
    pub sidebar_view: crate::sidebar_view::SidebarView,
    /// The sessions sidebar's section folded shut.
    pub section_folded: bool,
    /// Session groups folded shut.
    pub groups_folded: HashSet<String>,
    /// The group whose name is being edited.
    pub renaming_group: Option<String>,
    /// The workspace switcher's create or edit form, its name in `WS_NAME`.
    pub ws_form: Option<WorkspaceForm>,
    pub filter: Filter,
    pub sort: Sort,
    /// 0 for the automatic layout, -1 flexible, else that many columns:
    /// the active workspace's, from `grids`.
    pub columns: i32,
    /// Each workspace's columns and flexible tree (`gridLayout`).
    pub grids: std::collections::BTreeMap<String, crate::flex::Grid>,
    /// The pane a new session splits in the flexible grid: the last selected one.
    pub flex_anchor: Option<String>,
    /// The flexible divider being dragged, until it is let go.
    pub flex_drag: Option<crate::flex::Path>,
    /// A layout just set the tree: the workspace's other cards go beside it, not into it.
    pub flex_beside: bool,
    pub layout: LayoutMode,
    pub menu: Option<Menu>,
    pub dialog: Option<Dialog>,
    /// The session whose title is being edited.
    pub renaming: Option<String>,
    /// The session whose close asks first.
    pub confirm_close: Option<String>,
    /// A paste waiting on its confirm dialog: the session and the text.
    pub paste: Option<(String, String)>,
    /// The terminal selection and the session it is in.
    pub sel: Option<(String, crate::select::Sel)>,
    /// The project whose delete asks first.
    pub confirm_delete: Option<String>,
    pub toasts: Vec<Toast>,
    /// What the launch step did with the `vorn` command, until dismissed.
    pub cli_notice: Option<crate::cli_link::Notice>,
    /// Sessions the restored banner offers.
    pub banner: Vec<TerminalSession>,
    pub restoring: bool,
    pub recent: Vec<Recent>,
    /// The saved layouts the palette offers (`layout:list`).
    pub layouts: Vec<LayoutRow>,
    pub headless: crate::headless::Tracker,
    pub worktrees: HashMap<String, Vec<Worktree>>,
    pub branches: HashMap<String, Vec<String>>,
    pub composer: Composer,
    /// The launcher over the cards (Mod+N).
    pub composer_open: bool,
    pub launch: Launch,
    /// The launcher's model picker.
    pub models: Models,
    pub form: ProjectForm,
    /// When each terminal last scrolled: its scrollbar shows for a moment after.
    pub scrolled_at: HashMap<String, web_time::Instant>,
    /// Agents installed here, when known.
    pub installed: Option<Vec<String>>,
    /// Where each installed agent's program is.
    pub agent_paths: HashMap<String, String>,
    /// The selected card's terminal takes the keys on the next frame, as
    /// today's `term.focus()` after a jump or a new session.
    pub focus_terminal: bool,
    /// When each session last drew, for the recent sort.
    pub last_output: HashMap<String, Instant>,
    /// Panes whose size this device does not drive.
    pub locked: HashSet<String>,
    /// Each shell's command blocks.
    pub shells: HashMap<String, crate::blocks::Shell>,
    /// Each shell's typed commands, and the composer's menu.
    pub intent: crate::intent::Intent,
    /// Sidebar projects folded shut.
    pub collapsed: HashSet<String>,
    /// Projects the sidebar has drawn, so only a new one takes its fold.
    pub seen_projects: HashSet<String>,
    /// Cards stowed in the session dock rather than the grid.
    pub minimized: HashSet<String>,
    /// The session dock folded into its count chip.
    pub dock_collapsed: bool,
    pub tasks: crate::tasks::Tasks,
    pub runs: crate::runs::RunView,
    pub wf: crate::workflows::Workflows,
    pub settings: crate::settings::Settings,
    pub git: crate::git::Git,
    pub side: crate::side::Side,
    pub editors: crate::editor::Editors,
    /// The build of the vornd this app talks to, from `server:identity`.
    pub server_version: Option<String>,
    pub bench: crate::panes::Workbench,
    pub palette: crate::palette::Palette,
    pub onboard: crate::onboarding::Onboard,
    pub plugins: crate::plugins::Plugins,
    pub network: crate::network::Network,
    pub ext: crate::extensions::Extensions,
    /// What extensions contribute to the screens.
    pub points: crate::points::Points,
    /// The sign-in sheet, while one is open.
    pub signin: Option<crate::signin::Window>,
    /// An agent has asked for permission, so Mod+Shift+A and D answer it.
    pub permission_keys: bool,
}

impl State {
    /// Settings is open on `cat`.
    pub fn shown(&self, cat: crate::settings::Category) -> bool {
        self.settings.open && self.settings.category == cat
    }

    /// Settings is open on Extensions.
    pub fn shows_plugins(&self) -> bool {
        self.shown(crate::settings::Category::Connectors)
    }

    /// The cards the grid shows, as indexes into `store.sessions`: the
    /// active project's, through the status filter, in the chosen order.
    pub fn visible(&self, store: &Store) -> Vec<usize> {
        let mut out: Vec<usize> = store
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                store.in_scope(s)
                    && self
                        .project
                        .as_deref()
                        .is_none_or(|p| s.info.project_name == p)
                    && self
                        .group
                        .as_deref()
                        .is_none_or(|g| s.info.group_id.as_deref() == Some(g))
                    && self
                        .worktree
                        .as_deref()
                        .is_none_or(|w| s.info.worktree_path.as_deref().unwrap_or("") == w)
                    && self.filter.keeps(s)
                    && self.side.holder(&s.info.id).is_none()
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

    /// The cards the grid lays out: [`State::visible`] less the minimized,
    /// which the dock holds instead. The tabs layout shows them all.
    pub fn gridded(&self, store: &Store) -> Vec<usize> {
        let mut shown = self.visible(store);
        if self.layout == LayoutMode::Grid {
            shown.retain(|&i| !self.minimized.contains(&store.sessions[i].info.id));
        }
        shown
    }

    /// The minimized cards the dock shows, in the grid's order; none in the
    /// tabs layout, where every session has its tab.
    pub fn docked(&self, store: &Store) -> Vec<usize> {
        if self.layout != LayoutMode::Grid {
            return Vec::new();
        }
        let mut shown = self.visible(store);
        shown.retain(|&i| self.minimized.contains(&store.sessions[i].info.id));
        shown
    }

    /// Whether the sessions view has no card to lay out and no filter to
    /// blame, so the composer stands in for the grid (`PromptLauncher`
    /// inline).
    pub fn composer_inline(&self, store: &Store) -> bool {
        store.sessions_known
            && self.view == MainView::Sessions
            && self.focused.is_none()
            && (store.scoped_sessions().next().is_none()
                || (self.layout == LayoutMode::Grid
                    && self.filter == Filter::All
                    && self.gridded(store).is_empty()))
    }

    /// Whether the composer is what the sessions view shows.
    pub fn composing(&self, store: &Store) -> bool {
        self.composer_inline(store)
            || (self.view == MainView::Sessions && self.focused.is_none() && self.composer_open)
    }

    /// Whether the composer floats over the cards rather than replacing them.
    pub fn composer_overlay(&self, store: &Store) -> bool {
        self.composing(store) && !self.composer_inline(store)
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

/// Milliseconds since the epoch, as vornd stamps things.
pub fn now_ms() -> f64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
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
    fn a_recent_session_knows_the_terminal_running_it() {
        let past = Recent::from_value(&json!({ "sessionId": "s1", "agentType": "acme" })).unwrap();
        assert_eq!(past.running_in, None);
        let live = Recent::from_value(
            &json!({ "sessionId": "s1", "agentType": "acme", "runningIn": "t9" }),
        )
        .unwrap();
        assert_eq!(live.running_in.as_deref(), Some("t9"));
    }

    #[test]
    fn times_read_as_the_renderer_writes_them() {
        assert_eq!(time_ago(0.0, 30_000.0), "just now");
        assert_eq!(time_ago(0.0, 5.0 * 60_000.0), "5m ago");
        assert_eq!(time_ago(0.0, 3.0 * 3_600_000.0), "3h ago");
        assert_eq!(time_ago(0.0, 2.0 * 86_400_000.0), "2d ago");
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
        let r = Recent::from_value(&json!({ "sessionId": "s", "agentType": "acme" }));
        assert_eq!(r.map(|r| r.agent), Some("acme".to_owned()));
        assert!(Recent::from_value(&json!({ "agentType": "acme" })).is_none());
    }
}
