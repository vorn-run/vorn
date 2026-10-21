//! The app: vornd's models and the grid folded into one state, the keys
//! routed to the shortcuts, and each frame built
//! from that state. It knows nothing of windows, so the same `App` draws
//! into a window or offscreen.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::act::{Act, Acts};
use crate::client::rpc::{decode, Wake};
use crate::client::{Endpoint, Event, Pending, Rpc, RpcError, Store};
use crate::grid::Grid;
use crate::look;
use crate::models::Catalog;
use crate::paint;
use crate::screen::{
    card_index, intent_id, menus, rename_id, term_id, Composer, MainView, Screen, COMPOSER_INPUT,
    PROJECT_COLOR, PROJECT_ICON, PROJECT_NAME, PROJECT_PATH, VIEWS,
};
use crate::scroll::{self, Route, Rows, Scroll};
use crate::state::{
    basename, now_ms, Dialog, Filter, LayoutMode, Menu, MenuKind, Recent, State, Worktree,
};
use crate::ui::{
    decode_png, mods, El, Event as UiEvent, Id, Input, Laid, Role, TreeUpdate, Ui, WheelPhase,
    WindowRequest,
};
use crate::view::PaneView;
use vorn_protocol::TerminalSession;

pub mod canvas;
mod extensions;
mod flex;
pub mod forms;
mod gates;
mod git;
mod network;
mod onboarding;
mod palette;
mod plugins;
mod points;
#[cfg(not(target_arch = "wasm32"))]
mod relink;
mod review;
mod settings;
mod shell;
mod side;
pub use side::FILES;
pub mod signin;
mod tasks;
mod term;
mod video;
mod web;
mod workflows;

/// The modifier the app's shortcuts use: Cmd on a Mac, Ctrl elsewhere.
#[allow(non_snake_case)]
pub fn MOD() -> u16 {
    mods::primary()
}

/// What a key does before it reaches a field, a terminal or the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    View(MainView),
    /// Ctrl+`: a shell in the active project.
    NewShell,
    /// Mod+N: the composer, over the cards.
    NewSession,
    /// Mod+] and Mod+[: the next or previous card.
    Cycle(bool),
    /// Mod+1..9: the card at that position.
    Jump(usize),
    /// Mod+B.
    Sidebar,
    /// Mod+/.
    Shortcuts,
    /// Mod+J.
    ViewOptions,
    /// Alt+1..5.
    Filter(Filter),
    /// Mod+O: the selected card onto the focused stage.
    Expand,
    /// Mod+W: the focused stage back into the grid.
    Collapse,
    /// F2.
    Rename,
    /// F11.
    Fullscreen,
    /// Mod+K.
    Palette,
    /// Mod+Shift+A and Mod+Shift+D: allow or deny the oldest pending
    /// permission request, wherever its session is.
    Permission(bool),
    /// Mod+Shift+Up and Mod+Shift+Down: the selected terminal scrolled to
    /// the command before or after, by the shell's prompt marks.
    Prompt(bool),
    /// Mod+Alt+arrow: the flexible grid's pane beside the selected one.
    PaneFocus(crate::flex::Side),
    /// Mod+Alt+Shift+arrow: the divider nearest the selected pane moved.
    PaneResize(crate::flex::Side),
    /// Mod+Alt+=: every flexible pane back to an equal share.
    PaneEqualize,
}

/// The global shortcuts. Escape is not one: it closes what is open, else a
/// terminal takes it, as xterm does.
pub fn shortcut(code: &str, m: u16) -> Option<Shortcut> {
    let only = |want: u16| m & (mods::CTRL | mods::SUPER | mods::ALT | mods::SHIFT) == want;
    if code == "Backquote" && only(mods::CTRL) {
        return Some(Shortcut::NewShell);
    }
    if only(MOD() | mods::SHIFT) {
        return match code {
            "KeyA" => Some(Shortcut::Permission(true)),
            "KeyD" => Some(Shortcut::Permission(false)),
            "ArrowUp" => Some(Shortcut::Prompt(true)),
            "ArrowDown" => Some(Shortcut::Prompt(false)),
            _ => None,
        };
    }
    if only(MOD() | mods::ALT) && code == "Equal" {
        return Some(Shortcut::PaneEqualize);
    }
    for (with, make) in [
        (
            MOD() | mods::ALT,
            Shortcut::PaneFocus as fn(crate::flex::Side) -> Shortcut,
        ),
        (MOD() | mods::ALT | mods::SHIFT, Shortcut::PaneResize),
    ] {
        if only(with) {
            use crate::flex::Side;
            return match code {
                "ArrowLeft" => Some(make(Side::Left)),
                "ArrowRight" => Some(make(Side::Right)),
                "ArrowUp" => Some(make(Side::Up)),
                "ArrowDown" => Some(make(Side::Down)),
                _ => None,
            };
        }
    }
    if only(0) {
        return match code {
            "F2" => Some(Shortcut::Rename),
            "F11" => Some(Shortcut::Fullscreen),
            _ => None,
        };
    }
    if only(mods::ALT) {
        let d = code.strip_prefix("Digit")?.parse::<usize>().ok()?;
        return Filter::ALL
            .get(d.checked_sub(1)?)
            .copied()
            .map(Shortcut::Filter);
    }
    if !only(MOD()) {
        return None;
    }
    Some(match code {
        "KeyS" => Shortcut::View(MainView::Sessions),
        "KeyN" => Shortcut::NewSession,
        "KeyB" => Shortcut::Sidebar,
        "KeyJ" => Shortcut::ViewOptions,
        "KeyO" => Shortcut::Expand,
        "KeyW" => Shortcut::Collapse,
        "Slash" => Shortcut::Shortcuts,
        "KeyK" => Shortcut::Palette,
        "BracketRight" => Shortcut::Cycle(true),
        "BracketLeft" => Shortcut::Cycle(false),
        _ => {
            let d = code.strip_prefix("Digit")?.parse::<usize>().ok()?;
            Shortcut::Jump(d.checked_sub(1)?)
        }
    })
}

/// Keys a terminal holding the keyboard takes before the app's shortcuts
/// and focus traversal: Tab and Shift+Tab with Ctrl or Alt or neither.
pub fn terminal_key(code: &str, m: u16) -> bool {
    code == "Tab" && m & mods::SUPER == 0
}

/// Keys that only hold a modifier: pressing one keeps a terminal's selection.
const MODIFIER_KEYS: [&str; 9] = [
    "ShiftLeft",
    "ShiftRight",
    "ControlLeft",
    "ControlRight",
    "AltLeft",
    "AltRight",
    "MetaLeft",
    "MetaRight",
    "CapsLock",
];

/// The card after or before `current` among `n`, wrapping; with none
/// selected, the first or the last.
fn cycle(current: Option<usize>, n: usize, forward: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
    })
}

/// `EndedStrip`'s cause for a session carried over from the last run.
pub fn carried_cause(entry: &Value) -> &'static str {
    let flag = |k: &str| entry.get(k).and_then(Value::as_bool).unwrap_or(false);
    if flag("closedCleanly") {
        "Vorn was closed"
    } else if flag("rebooted") {
        "the machine restarted"
    } else {
        "the server stopped unexpectedly"
    }
}

/// What an open file picker was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
// A browser installs no packs and downloads exports instead.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
enum Picked {
    /// The add-project dialog's folder.
    Folder,
    /// Where a workflow is exported to.
    Export(String),
    /// A workflow file to import.
    Import,
    /// A folder to install an extension from, or link.
    ExtFolder(crate::plugins::Origin),
    /// A file a tree point's button asked for, told to its `node`.
    Tree { key: String, node: String },
}

/// A call whose answer changes the screen.
enum Call {
    /// `terminal:listActive` as the app starts.
    Sessions,
    /// The composer's `terminal:create`.
    Launch,
    /// A session or shell that should be selected once it exists.
    Created,
    /// An ended session started again in place; an automatic one says
    /// nothing when it fails.
    Resumed {
        id: String,
        auto: bool,
    },
    /// Toasts this on success, the error otherwise.
    Toast(String),
    /// Toasts only an error.
    Quiet,
    Recent,
    /// `layout:list`, for the palette.
    Layouts,
    /// A `layout:open`: toasts what started.
    LayoutOpened(String),
    Headless,
    Restored,
    Installed,
    /// `agent:list` after an adapter was switched, installed or removed.
    Agents,
    Worktrees(String),
    Branches(String),
    /// `agent:listModels` for an agent in a project.
    Models(String, String),
    /// A `config:save`; the next one waits for it.
    Saved(Box<Call>),
    /// `config:load` after the app's saves, for a change they held back.
    Config,
    Task(tasks::TaskCall),
    Workflow(workflows::WfCall),
    Set(settings::SetCall),
    Git(git::GitCall),
    Side(side::SideCall),
    /// Nothing to do with the answer, an error included.
    Ignore,
    Pane(web::PaneCall),
    Plug(plugins::PlugCall),
    Ext(extensions::ExtCall),
    Point(points::PointCall),
    /// `connection:signedIn` after the sheet saw someone sign in.
    SignedIn(Option<String>),
    Net(network::NetCall),
    Intent(shell::IntentCall),
    /// What a built-in took over from a file the desktop kept: the file goes once it has.
    Imported(std::path::PathBuf),
    TermImage(term::TermImage),
}

/// The launcher model picker's id field.
pub const MODEL_ID: Id = Id(0x6d6f_6465_6c00_0001);

/// The field a group's name is edited in.
pub const GROUP_NAME: Id = Id(0x6772_6f75_7000_0001);

/// The workspace switcher's name field.
pub const WS_NAME: Id = Id(0x7773_6e61_6d00_0001);

/// Sets `defaults.activeWorkspace`, making `defaults` when there is none.
fn set_active_workspace(config: &mut Value, id: String) {
    if !config.get("defaults").is_some_and(Value::is_object) {
        config["defaults"] = json!({});
    }
    config["defaults"]["activeWorkspace"] = Value::String(id);
}

/// Appends `group` to the config's `sessionGroups`, making the list when
/// there is none.
fn push_group(config: &mut Value, group: Value) {
    match config
        .get_mut("sessionGroups")
        .and_then(Value::as_array_mut)
    {
        Some(list) => list.push(group),
        None => config["sessionGroups"] = Value::Array(vec![group]),
    }
}

/// Where an act came from: the control's id and, for a right click, the
/// point.
#[derive(Debug, Clone, Copy, Default)]
struct Src {
    id: Option<Id>,
    at: Option<(f32, f32)>,
}

/// The app's state between frames.
pub struct App {
    rpc: Rpc,
    events: Receiver<Event>,
    pub store: Store,
    grid: Option<Arc<Grid>>,
    /// Why the grid is not there, when it is not.
    pub grid_error: Option<String>,
    views: HashMap<String, PaneView>,
    /// The system's appearance and Reduce transparency, as the window last said.
    system_look: (bool, bool),
    /// Where each session's terminal was painted (physical pixels), for the pointer.
    term_boxes: HashMap<String, crate::ui::Rect>,
    /// A double-click asked vornd for a word; the click that ends it keeps the selection.
    word_pending: bool,
    /// Each terminal's scroll through its history; only painted ones move.
    scrolls: HashMap<String, Scroll>,
    /// The wheel gesture under way: its pane, where it goes, and its last event.
    wheel: Option<(String, Route, Rows, web_time::Instant)>,
    /// A scrollbar thumb being dragged: its session and where the thumb was when the drag began.
    thumb_drag: Option<(String, f32)>,
    /// Each terminal's screen as text, for assistive technology; made when
    /// its view changes.
    spoken: HashMap<String, String>,
    pub st: State,
    pub acts: Acts,
    /// The web client of the vornd this app talks to.
    pub web_url: String,
    preedit: String,
    /// The composer showed last frame: it takes the keyboard when it appears.
    was_composing: bool,
    /// The install sheet just opened, so its field takes the keys next frame.
    focus_confirm: bool,
    calls: Vec<(Call, Pending)>,
    /// Config saves not yet sent: vornd runs them in parallel, so a later
    /// one could land first.
    saves: VecDeque<(Value, Call)>,
    saving: bool,
    /// A `config:changed` came while saves were on their way: it may predate
    /// them, so the configuration is read again once they are done.
    config_stale: bool,
    logo: Option<u32>,
    /// The agents' marks, registered as images.
    agents: crate::components::AgentMarks,
    /// vornd hung up.
    pub closed: bool,
    notifier: crate::notify::Notifier,
    /// The session `terminal:seen` was last asked for, until it is seen.
    seen_asked: Option<String>,
    /// Where admitted notices are shown: the OS, unless a test swaps it.
    notice_sink: Arc<dyn crate::notify::Sink>,
    /// Notifications heard since the last frame, for the frame to admit.
    notices: Vec<crate::notify::Notice>,
    /// The app draws its own title bar and window controls.
    pub chrome: bool,
    /// macOS: the traffic lights sit over the app's top bar.
    pub traffic: bool,
    /// In the terminal UI: how it draws, shown in Settings.
    pub term_drawing: Option<crate::screen::TermDrawing>,
    /// The last frame's size.
    size: (f32, f32),
    /// vornd's HTTP port, for pages it serves (a gate's review page).
    port: u16,
    /// The sessions the banner offers, with why each ended.
    carried: Vec<(TerminalSession, &'static str)>,
    /// vornd's data directory, where its session holders announce
    /// themselves.
    pub data_dir: Option<std::path::PathBuf>,
    /// The health check on its way.
    health: Option<std::sync::mpsc::Receiver<Option<Value>>>,
    /// New links on their way after vornd went away.
    #[cfg(not(target_arch = "wasm32"))]
    relinking: relink::Relinking,
    /// The field that had focus last frame, to tell when one loses it.
    last_focus: Option<Id>,
    /// The Files extension was told which sessions are left, and has not answered with fewer.
    pruning: bool,
    /// What the desktop kept on its own was handed to Files and to Shell blocks.
    imported: (bool, bool),
    /// Ghosts a composer took before they came, to put in: its shell and the text.
    fills: Vec<(String, String)>,
    /// The Browser's web views and calls in flight.
    web: web::Web,
    /// The simulators the Device's video parts show, and their frames.
    video: video::Video,
    /// Wakes the window from another thread.
    wake: Wake,
    /// The connections' hidden pages and the calls in flight on them.
    sessions: signin::Sessions,
    /// A real window is open, so system-wide keys may be held.
    windowed: bool,
    /// The system-wide approve and deny keys, once an agent has asked.
    hotkeys: Option<Option<crate::hotkeys::Presses>>,
    /// Window points' processes, by point key.
    panels: HashMap<String, points::Running>,
    /// Window points a new row asked to bring up.
    raised: Vec<String>,
    /// A tree's answer asked for the main window to come forward.
    come_forward: bool,
    /// The OS file picker, what it is for, while it is open.
    picking: Option<(
        Picked,
        std::sync::mpsc::Receiver<Option<std::path::PathBuf>>,
    )>,
    /// Answers the file pickers; the OS dialogs outside tests.
    picker: crate::system::Picker,
    /// When `headless:list` was last asked, and exited sessions last pruned.
    headless_polled: web_time::Instant,
    headless_pruned: web_time::Instant,
    /// The sidebar's left edge while its handle is dragged.
    sidebar_left: Option<f32>,
    /// The window's updater; none offscreen.
    updater: Option<Arc<crate::update::Updater>>,
    /// The launch step's notice about the `vorn` command, on its way.
    cli_launch: Option<std::sync::mpsc::Receiver<Option<crate::cli_link::Notice>>>,
    /// The administrator prompt that adds the command's folder to PATH, while it is up.
    cli_prompt: Option<std::sync::mpsc::Receiver<Result<bool, String>>>,
}

/// Everything connecting to vornd waits on (the RPC socket, the models, the
/// grid), gathered on any thread; the window's thread makes the app of it.
#[cfg(not(target_arch = "wasm32"))]
pub struct Connected {
    ep: Endpoint,
    rpc: Rpc,
    events: Receiver<Event>,
    store: Store,
    grid: std::io::Result<Arc<Grid>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Connected {
    /// Connects to the vornd at `ep`, loads the models and opens the grid;
    /// `wake` runs whenever either has news for the next frame.
    pub fn open(ep: &Endpoint, wake: Wake) -> Result<Connected, RpcError> {
        let (rpc, events) = Rpc::connect(ep, Arc::clone(&wake))?;
        let store = Store::load(&rpc)?;
        let grid = Grid::connect(&ep.grid, &ep.token, wake);
        Ok(Connected {
            ep: ep.clone(),
            rpc,
            events,
            store,
            grid,
        })
    }
}

impl App {
    /// Connects to the vornd at `ep`, loads the models and opens the grid;
    /// `wake` runs whenever either has news for the next frame.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect(ep: &Endpoint, wake: Wake) -> Result<App, RpcError> {
        Ok(App::from_connected(
            Connected::open(ep, Arc::clone(&wake))?,
            wake,
        ))
    }

    /// The app over a connection made elsewhere: cheap, nothing here waits on vornd.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_connected(c: Connected, wake: Wake) -> App {
        App::with(&c.ep, wake, c.rpc, c.events, c.store, c.grid)
    }

    /// [`App::connect`] without holding the page's one thread.
    #[cfg(target_arch = "wasm32")]
    pub async fn connect_async(ep: &Endpoint, wake: Wake) -> Result<App, RpcError> {
        let (rpc, events) = Rpc::connect(ep, Arc::clone(&wake))?;
        let store = Store::load_async(&rpc).await?;
        let grid = Grid::connect(&ep.grid, &ep.token, Arc::clone(&wake));
        let mut app = App::with(ep, wake, rpc, events, store, grid);
        app.chrome = false;
        app.traffic = false;
        Ok(app)
    }

    fn with(
        ep: &Endpoint,
        wake: Wake,
        rpc: Rpc,
        events: Receiver<Event>,
        store: Store,
        grid: std::io::Result<Arc<Grid>>,
    ) -> App {
        let app_wake = Arc::clone(&wake);
        let web = web::Web::new(Arc::clone(&wake));
        // A grid that will not open still leaves the screen and its data.
        let (grid, grid_error) = match grid {
            Ok(g) => (Some(g), None),
            Err(e) => (None, Some(format!("grid {}: {e}", ep.grid))),
        };
        let st = State {
            sidebar: true,
            layout: LayoutMode::from_config(&store.config),
            tasks: crate::tasks::Tasks::new(&store.config),
            settings: crate::settings::Settings::new(),
            // A first run, or one before this guide, opens the welcome guide.
            dialog: (!crate::onboarding::seen(&store.config)).then_some(Dialog::Welcome),
            ..State::default()
        };
        let mut app = App {
            rpc,
            events,
            store,
            grid,
            grid_error,
            views: HashMap::new(),
            system_look: (true, false),
            term_boxes: HashMap::new(),
            word_pending: false,
            scrolls: HashMap::new(),
            wheel: None,
            thumb_drag: None,
            spoken: HashMap::new(),
            st,
            acts: Acts::default(),
            web_url: ep.web_url(),
            preedit: String::new(),
            was_composing: false,
            focus_confirm: false,
            calls: Vec::new(),
            saves: VecDeque::new(),
            saving: false,
            config_stale: false,
            logo: None,
            agents: Default::default(),
            closed: false,
            notifier: Default::default(),
            seen_asked: None,
            notice_sink: Arc::new(crate::notify::Os),
            notices: Vec::new(),
            chrome: !cfg!(target_os = "macos"),
            traffic: cfg!(target_os = "macos"),
            term_drawing: None,
            size: look::WINDOW,
            port: ep.port,
            carried: Vec::new(),
            data_dir: None,
            health: None,
            #[cfg(not(target_arch = "wasm32"))]
            relinking: Default::default(),
            last_focus: None,
            pruning: false,
            imported: (false, false),
            fills: Vec::new(),
            web,
            video: video::Video::new(Arc::clone(&app_wake)),
            wake: app_wake,
            sessions: Default::default(),
            windowed: false,
            panels: HashMap::new(),
            raised: Vec::new(),
            come_forward: false,
            hotkeys: None,
            picking: None,
            picker: crate::system::os_picker(),
            headless_polled: web_time::Instant::now(),
            headless_pruned: web_time::Instant::now(),
            sidebar_left: None,
            updater: None,
            cli_launch: None,
            cli_prompt: None,
        };
        app.st.grids =
            crate::flex::grids_from_json(app.store.config.pointer("/defaults/gridLayout"));
        app.load_columns();
        app.see_projects();
        // Asked, not waited on: vornd holds it back while its session holder catches up.
        app.call(Call::Sessions, "terminal:listActive", Value::Null);
        app.call(Call::Restored, "sessions:restored", Value::Null);
        app.call(Call::Headless, "headless:list", Value::Null);
        app.call(Call::Installed, "agent:locate", Value::Null);
        app
    }

    /// A project first seen without sessions
    /// starts folded and stays so until opened.
    fn see_projects(&mut self) {
        for p in &self.store.projects {
            if self.st.seen_projects.insert(p.name.clone())
                && !self
                    .store
                    .sessions
                    .iter()
                    .any(|s| s.info.project_name == p.name)
            {
                self.st.collapsed.insert(p.name.clone());
            }
        }
    }

    /// Where vornd keeps its data: the session holders announce themselves
    /// there, and this app keeps its own view of it (open panes, drafts)
    /// beside them.
    /// The launch step's answer, which `cli_link::start` sends.
    pub fn set_cli_launch(
        &mut self,
        rx: std::sync::mpsc::Receiver<Option<crate::cli_link::Notice>>,
    ) {
        self.cli_launch = Some(rx);
    }

    /// Shows notices through `sink` instead of the OS: tests and offscreen
    /// frames must never post a real notification.
    /// Answers file pickers with `picker` in place of the OS dialogs.
    pub fn set_picker(&mut self, picker: crate::system::Picker) {
        self.picker = picker;
    }

    /// Opens the OS picker for `what`, unless one is already up.
    fn start_pick(&mut self, purpose: Picked, what: crate::system::Pick) {
        if self.picking.is_none() {
            let wake = Arc::clone(&self.wake);
            self.picking = Some((purpose, crate::system::pick(&self.picker, what, wake)));
        }
    }

    /// `headless:exit` and `headless:data` between polls.
    fn headless_notified(&mut self, method: &str, params: &Value) {
        let Some(id) = params.get("id").and_then(Value::as_str) else {
            return;
        };
        if method == "headless:exit" {
            let code = params.get("exitCode").and_then(Value::as_i64);
            self.st.headless.exited(id, code, now_ms());
        } else if let Some(data) = params.get("data").and_then(Value::as_str) {
            self.st.headless.data(id, data);
        }
    }

    /// Polls `headless:list` every 5 s, drops exited sessions past
    /// `headlessRetentionMinutes` every 30 s, and folds the pills once
    /// their popover closes.
    fn headless_tick(&mut self, ui: &mut Ui, now: web_time::Instant) {
        if now >= self.headless_polled + crate::headless::POLL {
            self.headless_polled = now;
            self.call(Call::Headless, "headless:list", Value::Null);
        }
        if now >= self.headless_pruned + crate::headless::PRUNE {
            self.headless_pruned = now;
            let minutes = self
                .store
                .config
                .pointer("/defaults/headlessRetentionMinutes")
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            self.st.headless.prune(minutes * 60_000.0, now_ms());
        }
        ui.wake_at(self.headless_polled + crate::headless::POLL);
        if !self
            .st
            .menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::Headless)
        {
            self.st.headless.collapse_all();
        } else if crate::headless::shown(&self.store, &self.st).is_empty() {
            // The badge, and the popover under it, go with the last pill.
            self.st.menu = None;
            self.st.headless.collapse_all();
        }
        for id in self.st.headless.take_grew() {
            ui.scroll_to_end(crate::headless::log_id(&id));
        }
    }

    /// Once a frame: what the open picker chose, sent where it was asked for.
    fn pick_tick(&mut self, ui: &mut Ui) {
        let Some((_, rx)) = &self.picking else { return };
        let Ok(picked) = rx.try_recv() else { return };
        let Some((purpose, _)) = self.picking.take() else {
            return;
        };
        let Some(path) = picked else { return };
        let path = path.to_string_lossy().into_owned();
        match purpose {
            Picked::Folder => {
                if matches!(self.st.dialog, Some(Dialog::Project(_))) {
                    ui.set_edit_text(PROJECT_PATH, &path);
                    self.project_path_changed(ui);
                }
            }
            Picked::Export(id) => self.export(&id, &path),
            Picked::Import => self.import(&path),
            Picked::ExtFolder(origin) => self.plug_picked(ui, origin, &path),
            Picked::Tree { key, node } => {
                let event = vorn_tree::Event::new(node, vorn_tree::EventKind::Submit, path);
                self.tell(&key, &event);
            }
        }
    }

    pub fn set_notice_sink(&mut self, sink: Arc<dyn crate::notify::Sink>) {
        self.notice_sink = sink;
    }

    pub fn set_data_dir(&mut self, dir: std::path::PathBuf) {
        self.st.side = crate::side::Side::load(Some(crate::side::view_file(&dir)));
        self.st.sidebar_view =
            crate::sidebar_view::SidebarView::load(&crate::sidebar_view::file(&dir));
        self.web.host.set_profiles(dir.join("connection-profiles"));
        self.data_dir = Some(dir);
    }

    /// Resets the selection on a workspace switch: one from the last workspace
    /// would narrow the new one to nothing.
    fn workspace_switched(&mut self) {
        self.st.project = None;
        self.st.group = None;
        self.st.worktree = None;
        self.st.focused = None;
        self.load_columns();
    }

    /// Changes the sidebar's view options and writes them down.
    fn set_sidebar_view(&mut self, change: impl FnOnce(&mut crate::sidebar_view::SidebarView)) {
        change(&mut self.st.sidebar_view);
        if let Some(dir) = &self.data_dir {
            self.st.sidebar_view.save(&crate::sidebar_view::file(dir));
        }
    }

    /// The vornd connection, for calls the screen does not make itself.
    pub fn rpc(&self) -> &Rpc {
        &self.rpc
    }

    /// The grid connection, when it opened.
    pub fn grid(&self) -> Option<&Arc<Grid>> {
        self.grid.as_ref()
    }

    /// The latest screen of `session`'s terminal.
    pub fn pane(&self, session: &str) -> Option<&PaneView> {
        self.views.get(session)
    }

    /// Where `session`'s terminal was last painted, in physical pixels.
    pub fn term_box(&self, session: &str) -> Option<crate::ui::Rect> {
        self.term_boxes.get(session).copied()
    }

    /// Whether the composer is what the sessions view shows.
    pub fn composing(&self) -> bool {
        self.st.composing(&self.store)
    }

    /// Whether calls are still on their way.
    pub fn busy(&self) -> bool {
        !self.calls.is_empty()
    }

    fn call(&mut self, kind: Call, method: &str, params: Value) {
        let pending = self.rpc.request(method, params);
        self.calls.push((kind, pending));
    }

    /// Folds in what arrived since the last frame: notifications, answers
    /// to calls, and terminals that changed.
    /// Gives the settings page and the sidebar the window's updater.
    pub fn set_updater(&mut self, updater: Arc<crate::update::Updater>) {
        self.updater = Some(updater);
        self.sync_update();
    }

    /// The updater's state, for this frame, and the person's choices, for it.
    fn sync_update(&mut self) {
        let Some(updater) = &self.updater else {
            return;
        };
        updater.set_prefs(crate::update::Prefs::from_config(&self.store.config));
        let status = updater.status();
        if self.st.settings.update.as_ref() != Some(&status) {
            self.st.settings.update = Some(status);
        }
    }

    /// The update card's or notice's button: restart into a ready update,
    /// download an offered one, or check again.
    fn update_action(&mut self) {
        let Some(updater) = self.updater.clone() else {
            return;
        };
        match updater.status() {
            crate::settings::UpdateStatus::Ready { .. } => {
                if updater.install(self.data_dir.as_deref()).is_ok() {
                    std::process::exit(0);
                }
            }
            crate::settings::UpdateStatus::Available { .. } => updater.download(),
            _ => updater.check(),
        }
    }

    /// Check now.
    fn check_updates(&self) {
        if let Some(updater) = &self.updater {
            updater.check();
        }
    }

    /// The theme whose tile in Settings › Appearance is under the pointer.
    fn hovered_theme(&self, ui: &Ui) -> Option<String> {
        if !self.st.settings.open {
            return None;
        }
        let bundled = vorn_theme::bundled().iter();
        bundled
            .chain(self.st.settings.appearance.installed())
            .find(|t| ui.hovered(crate::screen::settings::theme_tile_id(&t.id)))
            .map(|t| t.id.clone())
    }

    /// Installs the palette the appearance settings resolve to; when the
    /// colours change, every terminal is drawn again in the new ones.
    fn retheme(&mut self) {
        let (dark, reduced) = self.system_look;
        let preview = self.st.settings.theme_preview.as_deref();
        let changed = self.st.settings.appearance.apply(
            self.store.config.get("defaults"),
            dark,
            reduced,
            preview,
        );
        if let Some(dir) = &self.data_dir {
            self.st.settings.appearance.keep_materials(
                self.store.config.get("defaults"),
                &crate::appearance::launch_file(dir),
            );
        }
        if changed {
            if let Some(g) = &self.grid {
                g.touch_all();
            }
        }
    }

    pub fn pump(&mut self) {
        self.sync_update();
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Notification { method, params } => {
                    if method == "workflow:runUpdated" {
                        self.task_run_updated(&params);
                        self.wf_run_updated(&params);
                    }
                    self.pane_notified(&method, &params);
                    if method == "terminal:command" {
                        self.shell_notified(&method, &params);
                        continue;
                    }
                    if method == "headless:exit" || method == "headless:data" {
                        self.headless_notified(&method, &params);
                        continue;
                    }
                    if method == "server:identity" {
                        self.st.server_version = params
                            .get("appVersion")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        continue;
                    }
                    if method == "connector:catalogChanged" {
                        self.wf_catalog(&params);
                        self.plug_notified(&method, &params);
                        continue;
                    }
                    if method == "scheduler:missed" {
                        self.wf_missed(&params);
                        continue;
                    }
                    if method.starts_with("extension:") {
                        if method == "extension:contributionsChanged" {
                            self.call(Call::Agents, "agent:list", Value::Null);
                        }
                        self.points_notified(&method, &params);
                    }
                    let store = &self.store;
                    let heard = self.notifier.heard(
                        &method,
                        &params,
                        &store.config,
                        |id| {
                            store.session(id).map(|s| crate::notify::Who {
                                agent: if s.is_shell() {
                                    "Shell"
                                } else {
                                    store.agents.label(s.info.agent_type.as_str())
                                },
                                name: s.title(),
                            })
                        },
                        web_time::Instant::now(),
                    );
                    self.notices.extend(heard);
                    self.plug_notified(&method, &params);
                    self.net_notified(&method, &params);
                    self.ext_notified(&method, &params);
                    if method == "layout:opened" {
                        self.layout_opened(&params);
                    }
                    let before = self.store.workspace().to_owned();
                    // A read running may predate these rows, and rows may predate the app's
                    // own writes: either way the list is read again once both have landed.
                    let rows = method == "task:rows";
                    self.st.tasks.reload_stale |= rows && self.st.tasks.reloading;
                    if rows && self.st.tasks.writing > 0 {
                        self.reload_tasks();
                    } else if method == "config:changed" && self.saving {
                        self.config_stale = true;
                    } else if self.store.apply(&method, params) == crate::client::Change::TasksStale
                    {
                        self.reload_tasks();
                    }
                    if self.store.workspace() != before {
                        self.workspace_switched();
                    }
                }
                Event::Request { id, method, params } => self.bridge(id, &method, params),
                Event::Closed => self.closed = true,
            }
        }
        // After the events, so a vornd gone takes both its links in one dial.
        #[cfg(not(target_arch = "wasm32"))]
        self.relink();
        self.see_projects();
        let calls = std::mem::take(&mut self.calls);
        for (kind, pending) in calls {
            match pending.poll() {
                None => self.calls.push((kind, pending)),
                Some(answer) => self.answered(kind, answer),
            }
        }
        self.retheme();
        let mut shells = Vec::new();
        if let Some(g) = &self.grid {
            for id in g.take_dirty() {
                if let Some(v) = g.view(&id) {
                    let old = self.views.get(&id);
                    // History arriving for a scrolled view is not output.
                    if old.is_none_or(|o| o.rev != v.rev) {
                        self.st
                            .last_output
                            .insert(id.clone(), web_time::Instant::now());
                    }
                    if let (Some(s), Some(o)) = (self.scrolls.get_mut(&id), old) {
                        if !v.primary {
                            s.reset();
                        } else if o.sb_epoch == v.sb_epoch {
                            s.grew(v.live_line.saturating_sub(o.live_line));
                        }
                    }
                    // A resize or a cleared history renumbers the lines under a selection.
                    if self
                        .st
                        .sel
                        .as_ref()
                        .is_some_and(|(s, sel)| *s == id && sel.from.sb_epoch != v.sb_epoch)
                    {
                        self.st.sel = None;
                    }
                    if self.store.session(&id).is_some_and(|s| s.is_shell()) {
                        shells.push(id.clone());
                    }
                    self.spoken.insert(id.clone(), v.text());
                    self.views.insert(id, v);
                }
            }
            for id in g.attached() {
                if self.store.session(&id).is_none() && self.st.ext.owner_of_terminal(&id).is_none()
                {
                    g.detach(&id);
                    self.views.remove(&id);
                    self.scrolls.remove(&id);
                }
            }
        }
        for id in shells {
            self.sync_shell(&id);
        }
        self.st
            .shells
            .retain(|id, _| self.store.session(id).is_some());
        self.pump_web();
        self.pump_video();
        self.detect_mobile();
        let now = web_time::Instant::now();
        self.st.toasts.retain(|t| t.until > now);
        let gone = |id: &Option<String>, store: &Store| {
            id.as_deref().is_some_and(|id| store.session(id).is_none())
        };
        if gone(&self.st.selected, &self.store) {
            self.st.selected = None;
        }
        if gone(&self.st.focused, &self.store) {
            self.st.focused = None;
        }
    }

    fn answered(&mut self, kind: Call, answer: Result<Value, RpcError>) {
        match kind {
            Call::Launch => {
                self.st.composer.launching = false;
                match answer.and_then(decode::<TerminalSession>) {
                    Ok(info) => {
                        self.st.selected = Some(info.id.clone());
                        self.store.upsert(info);
                        self.st.composer = Composer {
                            project: self.st.composer.project.take(),
                            ..Composer::default()
                        };
                        self.st.launch = Default::default();
                        self.st.composer_open = false;
                    }
                    Err(e) => self.st.composer.error = Some(e.to_string()),
                }
            }
            Call::Created => match answer.and_then(decode::<TerminalSession>) {
                Ok(info) => {
                    self.st.selected = Some(info.id.clone());
                    self.st.focus_terminal = true;
                    if self.st.focused.is_some() {
                        self.st.focused = Some(info.id.clone());
                    }
                    self.st.view = MainView::Sessions;
                    self.store.upsert(info);
                }
                Err(e) => self.st.toast(e.to_string(), true),
            },
            Call::Resumed { id, auto } => self.resumed(&id, auto, answer),
            Call::Task(c) => self.task_answered(c, answer),
            Call::Workflow(c) => self.wf_answered(c, answer),
            Call::Set(c) => self.set_answered(c, answer),
            Call::Git(c) => self.git_answered(c, answer),
            Call::Side(c) => self.side_answered(c, answer),
            Call::Ignore => {}
            Call::Intent(c) => self.intent_answered(c, answer),
            Call::Imported(file) => {
                #[cfg(not(target_arch = "wasm32"))]
                if answer.is_ok() {
                    let _ = std::fs::remove_file(file);
                }
                #[cfg(target_arch = "wasm32")]
                let _ = file;
            }
            Call::Pane(c) => self.pane_answered(c, answer),
            Call::Plug(c) => self.plug_answered(c, answer),
            Call::Ext(c) => self.ext_answered(c, answer),
            Call::Point(c) => self.point_answered(c, answer),
            Call::SignedIn(identity) => {
                self.finish_signin(answer.map(|_| identity).map_err(|e| e.to_string()))
            }
            Call::Net(c) => self.net_answered(c, answer),
            Call::TermImage(c) => self.term_image_answered(c, answer),
            Call::Saved(call) => {
                self.saving = false;
                self.answered(*call, answer);
                self.next_save();
                if !self.saving && std::mem::take(&mut self.config_stale) {
                    self.call(Call::Config, "config:load", Value::Null);
                }
            }
            Call::Config => {
                if self.saving {
                    self.config_stale = true;
                } else if let Ok(config) = answer {
                    self.config_changed(config);
                }
            }
            Call::Toast(text) => match answer {
                Ok(_) => self.st.toast(text, false),
                Err(e) => self.st.toast(e.to_string(), true),
            },
            Call::Quiet => {
                if let Err(e) = answer {
                    self.st.toast(e.to_string(), true);
                }
            }
            Call::Recent => {
                self.st.recent = list(answer).iter().filter_map(Recent::from_value).collect();
            }
            Call::Layouts => {
                self.st.layouts = list(answer)
                    .iter()
                    .filter_map(crate::state::LayoutRow::from_value)
                    .collect();
            }
            Call::LayoutOpened(name) => match answer {
                Ok(done) => {
                    let started = done
                        .get("sessions")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    let problem = done
                        .get("errors")
                        .and_then(Value::as_array)
                        .and_then(|e| e.first())
                        .and_then(Value::as_str);
                    match problem {
                        Some(p) => self.st.toast(format!("Layout {name}: {p}"), true),
                        None => self
                            .st
                            .toast(format!("Opened layout {name}: {started} started"), false),
                    }
                }
                Err(e) => self.st.toast(e.to_string(), true),
            },
            Call::Headless => {
                if answer.is_ok() {
                    let list = list(answer);
                    let sessions = list
                        .iter()
                        .filter_map(crate::headless::Headless::from_value);
                    self.st.headless.sync(sessions.collect());
                }
            }
            Call::Restored => self.restored(list(answer)),
            Call::Sessions => match answer {
                Ok(v) => self.store.take_sessions(v),
                Err(e) => {
                    self.store.sessions_known = true;
                    self.st
                        .toast(format!("Could not list the sessions: {e}"), true);
                }
            },
            Call::Agents => {
                if let Ok(v) = answer {
                    self.store.agents = crate::agents::Agents::from_json(&v);
                    // A new agent may be installed where none was.
                    self.call(Call::Installed, "agent:locate", Value::Null);
                }
            }
            Call::Installed => {
                if let Some((found, paths)) =
                    answer.ok().as_ref().and_then(crate::settings::located)
                {
                    self.st.installed = Some(found);
                    self.st.agent_paths = paths;
                }
            }
            Call::Worktrees(path) => {
                let wts = list(answer)
                    .iter()
                    .filter_map(Worktree::from_value)
                    .collect();
                self.st.worktrees.insert(path, wts);
            }
            Call::Models(agent, path) => {
                let catalog = answer.map_or_else(|_| Catalog::failed(), |v| Catalog::from_json(&v));
                let listing = self.st.models.lists.entry((agent, path)).or_default();
                listing.catalog = Some(catalog);
                listing.loading = false;
            }
            Call::Branches(path) => {
                let Ok(v) = answer else { return };
                if v.get("isGitRepo").and_then(Value::as_bool) != Some(true) {
                    return;
                }
                let mut branches: Vec<String> = v
                    .get("local")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|b| b.as_str().map(str::to_owned))
                    .collect();
                // The current branch first: the chip's default.
                if let Some(cur) = v.get("current").and_then(Value::as_str) {
                    branches.retain(|b| b != cur);
                    branches.insert(0, cur.to_owned());
                }
                self.st.branches.insert(path, branches);
            }
        }
    }

    /// Takes `sessions:resume`'s answer for an ended session.
    fn resumed(&mut self, old: &str, auto: bool, answer: Result<Value, RpcError>) {
        let v = match answer {
            Ok(v) => v,
            Err(e) => {
                if !auto {
                    self.st.toast(e.to_string(), true);
                }
                return;
            }
        };
        if v.get("ok").and_then(Value::as_bool) != Some(true) {
            if auto {
                return;
            }
            if v.get("reason").and_then(Value::as_str) == Some("gone") {
                // Resumed elsewhere under the same id, so its updates must still bring it back.
                self.store.forget(old);
                self.views.remove(old);
                self.st
                    .toast("That session was resumed somewhere else", false);
            } else {
                let msg = v.get("message").and_then(Value::as_str);
                self.st
                    .toast(msg.unwrap_or("Could not resume that session"), true);
            }
            return;
        }
        let Ok(info) = decode::<TerminalSession>(v.get("session").cloned().unwrap_or_default())
        else {
            return;
        };
        let bound = v.get("boundTo").is_some_and(|b| !b.is_null());
        let id = info.id.clone();
        if id != old {
            self.store.remove(old);
            self.views.remove(old);
            for s in [&mut self.st.selected, &mut self.st.focused] {
                if s.as_deref() == Some(old) {
                    *s = Some(id.clone());
                }
            }
        }
        self.store.upsert(info);
        if let Some(s) = self.store.sessions.iter_mut().find(|s| s.info.id == id) {
            s.cause = None;
            s.exited = None;
        }
        self.views.remove(&id);
        if bound {
            self.st.toast(
                "That conversation was already running. This pane shows it.",
                false,
            );
        }
    }

    /// The sessions the last run left: with `reopenSessions` on they come
    /// back as ended panes and start again; off, the banner offers them.
    fn restored(&mut self, entries: Vec<Value>) {
        let carried: Vec<(TerminalSession, &'static str)> = entries
            .iter()
            .filter_map(|e| {
                let s = serde_json::from_value(e.get("session")?.clone()).ok()?;
                Some((s, carried_cause(e)))
            })
            .collect();
        if carried.is_empty() {
            return;
        }
        let reopen = self
            .store
            .config
            .pointer("/defaults/reopenSessions")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if reopen {
            for (s, cause) in carried {
                let id = s.id.clone();
                self.show_carried(s, cause);
                let call = Call::Resumed {
                    id: id.clone(),
                    auto: true,
                };
                self.call(call, "sessions:resume", json!({ "id": id }));
            }
        } else {
            self.st.banner = carried.iter().map(|(s, _)| s.clone()).collect();
            self.carried = carried;
        }
    }

    /// A carried-over session as an ended pane, unless it is live.
    fn show_carried(&mut self, info: TerminalSession, cause: &'static str) {
        if self.store.session(&info.id).is_some() {
            return;
        }
        let id = info.id.clone();
        self.store.upsert(info);
        if let Some(s) = self.store.sessions.iter_mut().find(|s| s.info.id == id) {
            s.cause = Some(cause);
        }
    }

    fn visible_ids(&self) -> Vec<String> {
        self.st
            .visible(&self.store)
            .into_iter()
            .map(|i| self.store.sessions[i].info.id.clone())
            .collect()
    }

    /// The card a shortcut acts on: the focused one, else the selected.
    fn current(&self) -> Option<String> {
        self.st.focused.clone().or_else(|| self.st.selected.clone())
    }

    /// Where typing goes: a panel shell holding the keyboard while its
    /// owner is the current card, else the current card.
    fn typing_target(&self) -> Option<String> {
        let current = self.current();
        if let (Some(f), Some(c)) = (&self.st.ext.focus, &current) {
            if f == c {
                if let Some(t) = self.st.ext.panes.get(f).and_then(|p| p.terminal_id.clone()) {
                    return Some(t);
                }
            }
        }
        if let Some(t) = &self.st.side.focus {
            if current.as_deref().is_some() && self.st.side.holder(t) == current.as_deref() {
                return Some(t.clone());
            }
        }
        current
    }

    /// Puts the card `id` on the focused stage, full window, keys going to it.
    pub fn focus_session(&mut self, id: &str) {
        self.st.focused = Some(id.to_owned());
        self.go_to(id.to_owned());
    }

    /// The card in front of a focused window: what its program finished is seen.
    fn seen_tick(&mut self, ui: &Ui) {
        if !ui.window_focused() || !matches!(self.st.view, MainView::Sessions) {
            return;
        }
        let Some(id) = self.st.focused.clone().or_else(|| self.st.selected.clone()) else {
            return;
        };
        let unseen = self.store.sessions.iter().any(|s| {
            s.info.id == id
                && s.info
                    .program_status
                    .as_ref()
                    .is_some_and(|p| p.unseen == Some(true))
        });
        if !unseen {
            self.seen_asked = None;
        } else if self.seen_asked.as_deref() != Some(id.as_str()) {
            self.call(Call::Quiet, "terminal:seen", json!({ "id": id }));
            self.seen_asked = Some(id);
        }
    }

    /// Moves to the card `id`: onto the stage when one is focused.
    fn go_to(&mut self, id: String) {
        self.st.view = MainView::Sessions;
        if self.st.focused.is_some() {
            self.st.focused = Some(id.clone());
        }
        self.st.selected = Some(id);
        self.st.focus_terminal = true;
    }

    /// The project new things start in: the sidebar's, else the selected
    /// card's, else the composer's.
    fn active_project(&self) -> Option<(String, String)> {
        let by_name = |n: &str| {
            self.store
                .projects
                .iter()
                .find(|p| p.name == n)
                .map(|p| (p.name.clone(), p.path.clone()))
        };
        if let Some(p) = self.st.project.as_deref().and_then(by_name) {
            return Some(p);
        }
        if let Some(s) = self
            .current()
            .and_then(|id| self.store.session(&id).cloned())
        {
            if !s.info.project_path.is_empty() {
                return Some((s.info.project_name.clone(), s.info.project_path.clone()));
            }
        }
        self.st.composer.project.as_deref().and_then(by_name)
    }

    fn run_shortcut(&mut self, ui: &mut Ui, s: Shortcut) {
        match s {
            Shortcut::View(v) => self.run(ui, Act::View(v), Src::default()),
            Shortcut::NewShell => self.run(ui, Act::NewShellIn(String::new()), Src::default()),
            Shortcut::NewSession => self.run(ui, Act::NewSession, Src::default()),
            Shortcut::Cycle(forward) => {
                let ids = self.visible_ids();
                let at = self
                    .current()
                    .and_then(|c| ids.iter().position(|i| *i == c));
                if let Some(next) = cycle(at, ids.len(), forward) {
                    self.go_to(ids[next].clone());
                }
            }
            Shortcut::Jump(i) => {
                if let Some(id) = self.visible_ids().get(i).cloned() {
                    self.go_to(id);
                }
            }
            Shortcut::Sidebar => self.run(ui, Act::ToggleSidebar, Src::default()),
            Shortcut::Shortcuts => {
                self.st.dialog = match self.st.dialog {
                    Some(Dialog::Shortcuts) => None,
                    _ => Some(Dialog::Shortcuts),
                };
            }
            Shortcut::ViewOptions => {
                let id = crate::act::Acts::id("toolbar", &Act::Open(MenuKind::ViewOptions));
                self.open_menu(
                    ui,
                    MenuKind::ViewOptions,
                    Src {
                        id: Some(id),
                        at: None,
                    },
                );
            }
            Shortcut::Filter(f) => self.st.filter = f,
            Shortcut::Expand => {
                if let Some(id) = self.st.selected.clone() {
                    self.run(ui, Act::Expand(id), Src::default());
                }
            }
            Shortcut::Collapse => self.st.focused = None,
            Shortcut::Rename => {
                if let Some(id) = self.current() {
                    self.run(ui, Act::Rename(id), Src::default());
                }
            }
            Shortcut::Fullscreen => ui.request(WindowRequest::ToggleFullscreen),
            Shortcut::Palette => self.toggle_palette(ui),
            Shortcut::Permission(allow) => self.answer_oldest(ui, allow),
            Shortcut::Prompt(up) => self.jump_to_command(ui, up),
            Shortcut::PaneFocus(_) | Shortcut::PaneResize(_) | Shortcut::PaneEqualize => {
                self.flex_key(ui, s);
            }
        }
    }

    /// Jump by command: the current terminal scrolled to the prompt before
    /// or after the top of its view; past the last, the live screen. A shell
    /// showing its commands as blocks has none of its history on screen.
    fn jump_to_command(&mut self, ui: &mut Ui, up: bool) {
        let Some(id) = self.current() else { return };
        let as_blocks = self.store.session(&id).is_some_and(|s| {
            crate::log::split(
                &self.store.config,
                &self.st.points,
                s,
                self.st.shells.get(&id),
            )
            .is_some()
        });
        if as_blocks {
            return;
        }
        let (Some(lines), Some(view)) = (
            self.grid.as_ref().and_then(|g| g.lines(&id)),
            self.views.get(&id),
        ) else {
            return;
        };
        if lines.alt {
            return;
        }
        match crate::log::prompt_jump(&lines.prompts, view.line0, view.live_line, up) {
            Some(0) => self.return_to_live(&id),
            Some(back) => self.scroll_back_to(ui, &id, back),
            None => {}
        }
    }

    /// Mod+Shift+A or D: an agent asking is answered first, as always;
    /// with none asking, the oldest thing an extension says waits for a
    /// person (a workflow's gate), approved or denied.
    fn answer_oldest(&mut self, ui: &mut Ui, allow: bool) {
        let agent_asking = self.store.sessions.iter().any(|s| s.status() == "waiting");
        let oldest = self
            .st
            .points
            .attention(self.store.workspace())
            .first()
            .map(|(p, r)| (p.key(), r.id.clone()));
        match (agent_asking, oldest) {
            (false, Some((key, row))) => {
                self.run_point(ui, crate::points::PointAct::Answer { key, row, allow });
            }
            _ => self.call(
                Call::Quiet,
                "permission:resolve-top",
                json!({ "allow": allow }),
            ),
        }
    }

    /// Holds Mod+Shift+A and D system-wide once an agent has asked or a
    /// gate waits, as the desktop does, and answers their presses.
    fn hotkeys_tick(&mut self, ui: &mut Ui) {
        if !self.st.permission_keys {
            self.st.permission_keys = self.store.sessions.iter().any(|s| s.status() == "waiting")
                || !self.st.points.attention(self.store.workspace()).is_empty();
        }
        if self.hotkeys.is_none() && self.st.permission_keys && self.windowed {
            self.hotkeys = Some(crate::hotkeys::register(Arc::clone(&self.wake)));
        }
        let presses: Vec<bool> = match &self.hotkeys {
            Some(Some(rx)) => rx.try_iter().collect(),
            _ => return,
        };
        for allow in presses {
            self.answer_oldest(ui, allow);
        }
    }

    /// Opens `kind` at its control, or closes it when it is already open.
    fn open_menu(&mut self, ui: &mut Ui, kind: MenuKind, src: Src) {
        if self.st.menu.as_ref().is_some_and(|m| m.kind == kind) {
            self.close_menu(ui);
            return;
        }
        let w = menus::width(&kind);
        // Opened from the group's menu, under (and kept alive by) the group's row.
        let trigger = match &kind {
            MenuKind::GroupIcon(g) => Some(Acts::id("sidebar-group", &Act::Group(Some(g.clone())))),
            _ => src.id,
        };
        // In the units the anchor is written in, which on cells are not the layout's.
        let rect = trigger.and_then(|id| ui.authored_rect(id));
        let src_at = src.at.map(|p| ui.authored_point(p));
        let mut at = match (src_at, rect) {
            // `right-0 top-full mt-1` of an empty box at the row's end, so
            // from the row's middle; the row ends 8 px past its More.
            (None, Some(r)) if matches!(kind, MenuKind::Group(_) | MenuKind::Project(_)) => {
                (r.x + r.w + 8.0 - w, r.y + r.h / 2.0 + 4.0)
            }
            (None, Some(r)) if matches!(kind, MenuKind::SessionGroups(_)) => {
                (r.x + r.w - w, r.y + r.h / 2.0 + 4.0)
            }
            (Some(p), _) => p,
            (None, Some(r)) if kind.is_launch() => (r.x, r.y - 4.0),
            (None, Some(r))
                if matches!(
                    kind,
                    MenuKind::Grid
                        | MenuKind::ViewOptions
                        | MenuKind::TaskOptions
                        | MenuKind::ExtInstall
                ) =>
            {
                (r.x + r.w - w, r.y + r.h + 4.0)
            }
            // `OpenInButton` opens upwards from the chip's top, its right edge on the chip's.
            (None, Some(r)) if matches!(kind, MenuKind::OpenIn(_)) => (r.x + r.w - w, r.y - 4.0),
            (None, Some(r)) => (r.x, r.y + r.h + 4.0),
            (None, None) => (self.size.0 - w - 12.0, look::TOP_BAR_H),
        };
        at.0 = at.0.min(self.size.0 - w - 8.0).max(8.0);
        match &kind {
            MenuKind::Grid => {
                let paths: Vec<String> = self
                    .store
                    .workspace_projects()
                    .map(|p| p.path.clone())
                    .collect();
                for path in paths {
                    let arg = json!(path);
                    self.call(Call::Worktrees(path), "git:listWorktrees", arg);
                }
            }
            MenuKind::Recent => {
                let path = self.active_project().map(|p| json!(p.1));
                self.call(
                    Call::Recent,
                    "sessions:getRecent",
                    path.unwrap_or(Value::Null),
                );
            }
            MenuKind::Headless => self.call(Call::Headless, "headless:list", Value::Null),
            MenuKind::LaunchModel => {
                self.st.models.invalid = None;
                self.st.models.opened_at = now_ms() as u64;
                ui.set_edit_text(MODEL_ID, "");
                self.list_models(false);
            }
            MenuKind::LaunchWorktree => {
                if let Some((_, path)) = self.composer_project() {
                    self.call(
                        Call::Worktrees(path.clone()),
                        "git:listWorktrees",
                        json!(path),
                    );
                }
            }
            _ => {}
        }
        self.st.confirm_delete = None;
        self.st.menu = Some(Menu {
            kind,
            at,
            sub: Default::default(),
        });
        ui.open_menu(menus::MENU, trigger.unwrap_or(menus::MENU));
        if self
            .st
            .menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::LaunchModel)
        {
            ui.set_focus(Some(MODEL_ID));
        }
    }

    /// Enter in the model picker's id field: the listed model it names,
    /// else the id as typed once it passes `validate_id`.
    fn choose_typed_model(&mut self, ui: &mut Ui) {
        let query = ui.edit_text(MODEL_ID).to_owned();
        if query.trim().is_empty() {
            return;
        }
        let key = (
            self.launch_agent().to_owned(),
            self.composer_project().map(|p| p.1).unwrap_or_default(),
        );
        let exact = self
            .st
            .models
            .lists
            .get(&key)
            .and_then(|l| l.exact(&query))
            .map(|c| c.id.clone());
        let pick = match exact {
            Some(id) => id,
            None => match crate::models::validate_id(&query) {
                Ok(id) => id.to_owned(),
                Err(why) => {
                    self.st.models.invalid = Some(why);
                    return;
                }
            },
        };
        self.st.models.picked.insert(key.0, pick);
        self.close_menu(ui);
    }

    fn close_menu(&mut self, ui: &mut Ui) {
        self.st.menu = None;
        self.st.confirm_delete = None;
        if self.st.ws_form.take().is_some() && ui.focused(WS_NAME) {
            ui.set_focus(None);
        }
        ui.close_menu();
    }

    fn composer_project(&self) -> Option<(String, String)> {
        let name = self.st.composer.project.as_deref()?;
        let p = self.store.projects.iter().find(|p| p.name == name)?;
        Some((p.name.clone(), p.path.clone()))
    }

    /// Picks the composer's project and learns whether it is a repository.
    fn pick_project(&mut self, name: String) {
        self.st.composer.project = Some(name);
        self.st.composer.error = None;
        self.st.launch = Default::default();
        if let Some((_, path)) = self.composer_project() {
            if !self.st.branches.contains_key(&path) {
                self.call(
                    Call::Branches(path.clone()),
                    "git:listBranches",
                    json!(path),
                );
            }
        }
        self.list_models(false);
    }

    /// The agent the launcher starts.
    fn launch_agent(&self) -> &str {
        self.st
            .launch
            .agent
            .as_deref()
            .unwrap_or(self.store.default_agent())
    }

    /// Asks the launcher's agent for the models it runs in the launcher's
    /// project, once unless `refresh`, as the picker prefetches.
    fn list_models(&mut self, refresh: bool) {
        let agent = self.launch_agent().to_owned();
        let Some((_, path)) = self.composer_project() else {
            return;
        };
        if !self.store.agents.selects_models(&agent) {
            return;
        }
        let listing = self
            .st
            .models
            .lists
            .entry((agent.clone(), path.clone()))
            .or_default();
        if listing.loading || (listing.catalog.is_some() && !refresh) {
            return;
        }
        listing.loading = true;
        self.call(
            Call::Models(agent.clone(), path.clone()),
            "agent:listModels",
            json!({ "agentType": agent, "projectPath": path, "refresh": refresh }),
        );
    }

    /// "New group" after the last, its name being
    /// edited, holding `session` when one asked for it.
    fn new_group(&mut self, ui: &mut Ui, session: Option<String>) {
        let id = crate::graph::new_id();
        let order = self
            .store
            .groups
            .iter()
            .map(|g| g.order)
            .fold(-1.0, f64::max)
            + 1.0;
        let group = json!({
            "id": id,
            "name": "New group",
            "order": order,
            "workspaceId": self.store.workspace(),
        });
        let mut config = self.store.config.clone();
        push_group(&mut config, group);
        self.save_config(config, Call::Quiet);
        if let Some(sid) = session {
            self.move_to_group(sid, Some(id.clone()), Call::Quiet);
        }
        self.run(ui, Act::RenameGroup(id), Src::default());
    }

    fn rename_group(&mut self, ui: &mut Ui) {
        let Some(id) = self.st.renaming_group.take() else {
            return;
        };
        ui.set_focus(None);
        let name = ui.edit_text(GROUP_NAME).trim().to_owned();
        if !name.is_empty() && self.store.group(&id).is_some_and(|g| g.name != name) {
            self.patch_group(&id, "name", name);
        }
    }

    /// Saved in the config, which every app on this
    /// vornd follows.
    fn switch_workspace(&mut self, id: String) {
        let mut config = self.store.config.clone();
        set_active_workspace(&mut config, id);
        self.save_config(config, Call::Quiet);
    }

    /// The switcher's form, filled from workspace `id` or for a new one.
    fn edit_workspace(&mut self, ui: &mut Ui, id: Option<String>) {
        let ws = id
            .as_deref()
            .and_then(|id| self.store.workspaces().into_iter().find(|w| w.id == id));
        let (name, icon, color) = match &ws {
            Some(w) => (
                w.name.clone(),
                w.icon.clone().unwrap_or_else(|| "User".to_owned()),
                w.icon_color.clone().unwrap_or_else(|| "#6b7280".to_owned()),
            ),
            None => (String::new(), "Briefcase".to_owned(), "#3b82f6".to_owned()),
        };
        self.st.confirm_delete = None;
        self.st.ws_form = Some(crate::state::WorkspaceForm {
            id: ws.map(|w| w.id),
            icon,
            color,
            picker: false,
        });
        ui.set_edit_text(WS_NAME, &name);
        ui.set_focus(Some(WS_NAME));
    }

    /// Creates the workspace (then switches to it) or updates it.
    fn save_workspace(&mut self, ui: &mut Ui) {
        let name = ui.edit_text(WS_NAME).trim().to_owned();
        let Some(form) = self.st.ws_form.as_ref().filter(|_| !name.is_empty()) else {
            return;
        };
        let mut config = self.store.config.clone();
        if !config.get("workspaces").is_some_and(Value::is_array) {
            config["workspaces"] = json!([]);
        }
        let Some(list) = config["workspaces"].as_array_mut() else {
            return;
        };
        let call = match &form.id {
            Some(id) => {
                if let Some(w) = list
                    .iter_mut()
                    .find(|w| w.get("id").and_then(Value::as_str) == Some(id))
                {
                    w["name"] = json!(name);
                    w["icon"] = json!(form.icon);
                    w["iconColor"] = json!(form.color);
                }
                Call::Toast("Workspace updated".to_owned())
            }
            None => {
                let id = crate::graph::new_id();
                let order = list
                    .iter()
                    .filter_map(|w| w.get("order").and_then(Value::as_f64))
                    .fold(0.0, f64::max)
                    + 1.0;
                list.push(json!({
                    "id": id,
                    "name": name,
                    "icon": form.icon,
                    "iconColor": form.color,
                    "order": order,
                }));
                set_active_workspace(&mut config, id);
                Call::Toast(format!("Workspace \"{name}\" created"))
            }
        };
        self.st.ws_form = None;
        ui.set_focus(None);
        self.save_config(config, call);
    }

    /// Its groups go with it, its projects and
    /// workflows move to `personal`, which it hands the screen to if it
    /// had it.
    fn delete_workspace(&mut self, id: &str) {
        use crate::client::store::WORKSPACE;
        self.st.confirm_delete = None;
        if id == WORKSPACE {
            return;
        }
        let name = self
            .store
            .workspaces()
            .into_iter()
            .find(|w| w.id == id)
            .map(|w| w.name);
        let mut config = self.store.config.clone();
        let mine = |v: &Value| {
            v.get("workspaceId")
                .and_then(Value::as_str)
                .unwrap_or(WORKSPACE)
                == id
        };
        if let Some(l) = config.get_mut("workspaces").and_then(Value::as_array_mut) {
            l.retain(|w| w.get("id").and_then(Value::as_str) != Some(id));
        }
        if let Some(l) = config
            .get_mut("sessionGroups")
            .and_then(Value::as_array_mut)
        {
            l.retain(|g| g.get("workspaceId").and_then(Value::as_str) != Some(id));
        }
        for key in ["projects", "workflows"] {
            if let Some(l) = config.get_mut(key).and_then(Value::as_array_mut) {
                for item in l.iter_mut().filter(|v| mine(v)) {
                    item["workspaceId"] = json!(WORKSPACE);
                }
            }
        }
        if self.store.workspace() == id {
            set_active_workspace(&mut config, WORKSPACE.to_owned());
        }
        let label = name.unwrap_or_default();
        self.save_config(
            config,
            Call::Toast(format!("Workspace \"{label}\" deleted")),
        );
    }

    /// One field of a group, saved quietly.
    fn patch_group(&mut self, id: &str, key: &str, value: String) {
        let mut config = self.store.config.clone();
        let Some(g) = config
            .get_mut("sessionGroups")
            .and_then(Value::as_array_mut)
            .and_then(|l| {
                l.iter_mut()
                    .find(|g| g.get("id").and_then(Value::as_str) == Some(id))
            })
        else {
            return;
        };
        g[key] = Value::String(value);
        self.save_config(config, Call::Quiet);
    }

    /// The group goes, its sessions let go through
    /// the server, which owns membership.
    fn delete_group(&mut self, id: &str) {
        let Some(name) = self.store.group(id).map(|g| g.name.clone()) else {
            return;
        };
        let mut config = self.store.config.clone();
        if let Some(list) = config
            .get_mut("sessionGroups")
            .and_then(Value::as_array_mut)
        {
            list.retain(|g| g.get("id").and_then(Value::as_str) != Some(id));
        }
        if self.st.group.as_deref() == Some(id) {
            self.st.group = None;
        }
        self.st.groups_folded.remove(id);
        self.save_config(config, Call::Toast(format!("Group \"{name}\" deleted")));
        let members: Vec<String> = self
            .store
            .sessions
            .iter()
            .filter(|s| s.info.group_id.as_deref() == Some(id))
            .map(|s| s.info.id.clone())
            .collect();
        for sid in members {
            self.move_to_group(sid, None, Call::Quiet);
        }
    }

    /// Shown at once, then told to the server.
    fn move_to_group(&mut self, session: String, group: Option<String>, call: Call) {
        if let Some(s) = self
            .store
            .sessions
            .iter_mut()
            .find(|s| s.info.id == session)
        {
            s.info.group_id = group.clone();
        }
        self.call(
            call,
            "terminal:setGroup",
            json!({ "id": session, "groupId": group }),
        );
    }

    fn save_config(&mut self, config: Value, call: Call) {
        // Tasks and workflows are not vornd's configuration: their changes go as calls of their own.
        let mut calls =
            crate::client::sync::tasks(self.store.config.get("tasks"), config.get("tasks"));
        calls.extend(crate::client::sync::workflows(
            self.store.config.get("workflows"),
            config.get("workflows"),
        ));
        for (method, params) in calls {
            self.call(Call::Quiet, method, params);
        }
        self.config_changed(config.clone());
        self.queue_save(config, call);
    }

    /// vornd's configuration, read again: taken as a `config:changed`.
    fn config_changed(&mut self, config: Value) {
        let before = self.store.workspace().to_owned();
        self.store.apply("config:changed", config);
        if self.store.workspace() != before {
            self.workspace_switched();
        }
    }

    /// Sends `config` to vornd once the saves before it are done.
    fn queue_save(&mut self, config: Value, call: Call) {
        self.saves.push_back((config, call));
        self.next_save();
    }

    fn next_save(&mut self) {
        if self.saving {
            return;
        }
        if let Some((mut config, call)) = self.saves.pop_front() {
            if let Some(map) = config.as_object_mut() {
                map.remove("tasks");
                map.remove("workflows");
            }
            self.saving = true;
            self.call(Call::Saved(Box::new(call)), "config:save", config);
        }
    }

    /// Does what a control on screen does.
    fn run(&mut self, ui: &mut Ui, act: Act, src: Src) {
        match act {
            Act::Minimize => ui.request(WindowRequest::Minimize),
            Act::ToggleMaximize => ui.request(WindowRequest::ToggleMaximize),
            Act::CloseWindow => ui.request(WindowRequest::Close),
            Act::View(v) => {
                // A view whose extension is off has no button; one asked for anyway stays shut.
                let gone = v != MainView::Sessions
                    && self.st.points.loaded
                    && crate::screen::view_point(&self.st.points, v, None).is_none();
                if gone {
                    return;
                }
                self.st.view = v;
                self.st.ext_view = None;
                if v != MainView::Sessions {
                    self.st.focused = None;
                }
                if v == MainView::Workflows {
                    self.wf_refresh();
                }
            }
            Act::ExtensionView(key) => {
                self.st.view = MainView::Extension;
                self.st.ext_view = Some(key);
                self.st.focused = None;
            }
            Act::ToggleSidebar => {
                self.st.sidebar = !self.st.sidebar;
                if self.st.sidebar {
                    // Reopened, the projects take their first fold again.
                    self.st.seen_projects.clear();
                    self.st.collapsed.clear();
                    self.see_projects();
                }
            }
            Act::Intent(a) => self.run_intent(ui, a),
            Act::Select(id) => {
                self.st.view = MainView::Sessions;
                self.st.selected = Some(id);
                self.st.side.focus = None;
                self.st.ext.focus = None;
                ui.set_focus(None);
            }
            Act::Git(a) => self.run_git(ui, a, src),
            Act::Side(a) => self.run_side(ui, a, src),
            Act::Code(a) => self.run_code(ui, a),
            Act::Answer(ok) => self.answer_ask(ok),
            Act::Expand(id) => {
                self.st.view = MainView::Sessions;
                self.st.selected = Some(id.clone());
                self.st.focused = Some(id);
                self.st.composer_open = false;
            }
            Act::Collapse => self.st.focused = None,
            Act::Open(kind) => self.open_menu(ui, kind, src),
            Act::CloseMenu => self.close_menu(ui),
            Act::Filter(f) => self.st.filter = f,
            Act::Sort(s) => self.st.sort = s,
            Act::Columns(n) => self.set_columns(ui, n),
            Act::EqualizePanes => self.flex_equalize(),
            Act::Layout(mode) => {
                self.st.layout = mode;
                let mut config = self.store.config.clone();
                if !config.get("defaults").is_some_and(Value::is_object) {
                    config["defaults"] = json!({});
                }
                config["defaults"]["layoutMode"] = json!(mode.key());
                self.save_config(config, Call::Quiet);
            }
            Act::NewSession => {
                self.st.view = MainView::Sessions;
                self.st.focused = None;
                self.st.composer_open = true;
                if self.st.composer.project.is_none() {
                    if let Some((name, _)) = self.active_project() {
                        self.pick_project(name);
                    }
                }
            }
            Act::QuickSession => match self.active_project() {
                Some((name, path)) => {
                    let params = json!({
                        "agentType": self.store.default_agent(),
                        "projectName": name,
                        "projectPath": path,
                    });
                    self.create_session(Call::Created, params);
                }
                None => self.run(ui, Act::NewSession, src),
            },
            Act::NewSessionIn {
                project,
                worktree,
                agent,
            } => {
                let Some(p) = self.store.projects.iter().find(|p| p.name == project) else {
                    return;
                };
                let mut params = json!({
                    "agentType": agent.as_deref().unwrap_or(self.store.default_agent()),
                    "projectName": p.name,
                    "projectPath": p.path,
                });
                match worktree.as_deref() {
                    Some("") => params["useWorktree"] = json!(true),
                    Some(path) if path != p.path => params["existingWorktreePath"] = json!(path),
                    _ => {}
                }
                self.create_session(Call::Created, params);
            }
            Act::NewShellIn(dir) => {
                let cwd = if dir.is_empty() {
                    self.active_project().map_or(Value::Null, |p| json!(p.1))
                } else {
                    json!(dir)
                };
                self.call(Call::Created, "shell:create", cwd);
            }
            Act::RunWorkflow { id, source } => {
                let source = source
                    .and_then(|s| self.store.session(&s))
                    .map(|s| json!(s.info));
                self.start_run(ui, &id, source, None);
            }
            Act::OpenLayout { name, source } => {
                let mut ask = json!({ "name": name, "source": source });
                if let Some((_, path)) = self.active_project() {
                    // Only for a layout that names no project of its own.
                    ask["defaultProject"] = json!(path);
                }
                self.call(Call::LayoutOpened(name), "layout:open", ask);
            }
            Act::OpenLayoutMenu => {}
            Act::Workflow(a) => self.run_wf(ui, a, src),
            Act::Set(a) => {
                let was = self.st.shows_plugins();
                self.run_set(ui, a);
                match (was, self.st.shows_plugins()) {
                    (true, false) => self.plug_leave(),
                    (false, true) => self.run_plug(ui, crate::plugins::PlugAct::Refresh),
                    _ => {}
                }
            }
            Act::Rename(id) => {
                let Some(title) = self.store.session(&id).map(|s| s.title()) else {
                    return;
                };
                let field = rename_id(&id);
                ui.set_edit_text(field, &title);
                ui.set_focus(Some(field));
                self.st.renaming = Some(id);
            }
            Act::ToggleMinimized(id) => {
                if self.st.minimized.remove(&id) {
                    self.st.selected = Some(id);
                } else {
                    if self.st.focused.as_deref() == Some(id.as_str()) {
                        self.st.focused = None;
                    }
                    self.st.minimized.insert(id);
                }
            }
            Act::ToggleDock => {
                self.st.dock_collapsed = !self.st.dock_collapsed;
                ui.close_menu();
            }
            Act::DockPopover => {
                if ui.menu_open(crate::screen::DOCK) {
                    ui.close_menu();
                } else {
                    self.close_menu(ui);
                    ui.open_menu(crate::screen::DOCK, src.id.unwrap_or(crate::screen::DOCK));
                }
            }
            Act::ShowRun(key) => {
                self.run_wf(ui, crate::workflows::WfAct::AllRuns, Src::default());
                self.st.wf.tab = crate::workflows::Tab::All;
                self.st.wf.bucket = crate::workflows::Bucket::All;
                self.st.wf.selected = Some(key);
            }
            Act::AskClose(id) => {
                self.st.confirm_close = (!id.is_empty()).then_some(id);
            }
            Act::CloseSession(id) => {
                let Some(title) = self.store.session(&id).map(|s| s.title()) else {
                    return;
                };
                let from_sidebar =
                    src.id == Some(Acts::id("sidebar-session", &Act::CloseSession(id.clone())));
                self.call(Call::Quiet, "terminal:kill", json!(id));
                self.store.remove(&id);
                self.views.remove(&id);
                self.st.confirm_close = None;
                self.st.minimized.remove(&id);
                if self.st.selected.as_deref() == Some(id.as_str()) {
                    self.st.selected = None;
                }
                if self.st.focused.as_deref() == Some(id.as_str()) {
                    self.st.focused = None;
                }
                let text = if from_sidebar {
                    "Session closed".to_owned()
                } else {
                    format!("Session \"{title}\" closed")
                };
                self.st.toast(text, false);
            }
            Act::Resume(id) => {
                let call = Call::Resumed {
                    id: id.clone(),
                    auto: false,
                };
                self.call(call, "sessions:resume", json!({ "id": id }));
            }
            Act::Paste(id) => self.paste_clipboard(ui, &id),
            Act::CopySelection(id) => self.copy_selection(&id),
            Act::ConfirmPaste(run) => self.confirm_paste(ui, run),
            Act::ScrollToBottom(id) => self.return_to_live(&id),
            Act::ScrollPage(id, up) => {
                if let Some(v) = self.views.get(&id) {
                    let page = crate::scrollbar::page(v.rows);
                    let back = if up {
                        v.back().saturating_add(page)
                    } else {
                        v.back().saturating_sub(page)
                    };
                    self.scroll_back_to(ui, &id, back);
                }
            }
            Act::FitToDevice(id) => {
                if let Some(g) = &self.grid {
                    g.take_size(&id);
                }
            }
            Act::LockSize(id, locked) => {
                if let Some(g) = &self.grid {
                    g.lock_size(&id, locked);
                }
                if locked {
                    self.st.locked.insert(id);
                } else {
                    self.st.locked.remove(&id);
                }
            }
            Act::Shortcuts => self.run_shortcut(ui, Shortcut::Shortcuts),
            Act::Task(a) => self.run_task(ui, a),
            Act::Pane(a) => self.run_pane(ui, a),
            Act::Run(a) => self.run_run(a),
            Act::Gate(op) => self.run_gate(ui, op),
            Act::OpenUrl(u) => tasks::open_url(&u),
            Act::OpenWebClient => tasks::open_url(&self.web_url),
            Act::CloseDialog if self.st.dialog == Some(Dialog::AddTask) => {
                self.close_task_dialog(ui);
            }
            Act::CloseDialog if self.st.dialog == Some(Dialog::ExtInstall) => {
                self.run_plug(ui, crate::plugins::PlugAct::CancelInstall);
            }
            Act::CloseDialog => {
                self.st.dialog = None;
                self.st.composer_open = false;
                self.st.wf.prompt = None;
            }
            Act::Settings => {
                self.st.dialog = None;
                let cat = self.st.settings.category;
                self.open_settings(cat);
            }
            Act::Welcome => self.open_onboard(ui),
            Act::Ext(a) => self.run_ext(a),
            Act::Point(a) => self.run_point(ui, a),
            Act::AddGroup => {
                let mut config = self.store.config.clone();
                let workspace = config
                    .pointer("/defaults/activeWorkspace")
                    .and_then(Value::as_str)
                    .unwrap_or("personal")
                    .to_owned();
                let groups = config["sessionGroups"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let order = groups
                    .iter()
                    .filter_map(|g| g["order"].as_i64())
                    .max()
                    .unwrap_or(-1)
                    + 1;
                let mut groups = groups;
                groups.push(json!({
                    "id": settings::new_id(),
                    "name": "New group",
                    "order": order,
                    "workspaceId": workspace,
                }));
                config["sessionGroups"] = Value::Array(groups);
                self.save_config(config, Call::Quiet);
            }
            Act::SignInClose => self.finish_signin(Err(crate::signin::CLOSED.to_owned())),
            Act::Palette(a) => self.run_palette(ui, a),
            Act::Onboard(a) => self.run_onboard(ui, a),
            Act::Plug(a) => self.run_plug(ui, a),
            Act::Net(a) => self.run_net(ui, a),
            Act::Copy { text, toast } => {
                ui.clipboard().set(&text);
                if !toast.is_empty() {
                    self.st.toast(toast, false);
                }
            }
            Act::StopServer => {
                self.call(Call::Quiet, "server:shutdown", Value::Null);
                ui.request(WindowRequest::Close);
            }
            Act::Project(p) => {
                self.st.project = p;
                self.st.group = None;
                self.st.worktree = None;
                if self.st.view == MainView::Workflows {
                    self.st.view = MainView::Sessions;
                }
            }
            Act::Worktree { project, path } => {
                let again = self.st.project.as_deref() == Some(project.as_str())
                    && self.st.worktree.as_deref() == Some(path.as_str());
                self.st.project = Some(project);
                self.st.worktree = (!again).then_some(path);
                self.st.group = None;
                self.st.focused = None;
                if self.st.view == MainView::Workflows {
                    self.st.view = MainView::Sessions;
                }
            }
            Act::ToggleWorktree { project, path } => {
                let key = (project, path);
                if !self.st.worktrees_folded.remove(&key) {
                    self.st.worktrees_folded.insert(key);
                }
            }
            Act::AddProject => {
                ui.set_edit_text(PROJECT_PATH, "");
                ui.set_edit_text(PROJECT_NAME, "");
                ui.set_focus(Some(PROJECT_NAME));
                self.st.form = Default::default();
                self.st.dialog = Some(Dialog::Project(None));
            }
            Act::ToggleProject(name) => {
                if self.st.collapsed.remove(&name) {
                    let path = self
                        .store
                        .projects
                        .iter()
                        .find(|p| p.name == name)
                        .map(|p| p.path.clone());
                    if let Some(path) = path {
                        self.call(
                            Call::Worktrees(path.clone()),
                            "git:listWorktrees",
                            json!(path),
                        );
                    }
                } else {
                    self.st.collapsed.insert(name);
                }
            }
            Act::EditProject(name) => {
                let Some(p) = self.store.projects.iter().find(|p| p.name == name) else {
                    return;
                };
                ui.set_edit_text(PROJECT_PATH, &p.path);
                ui.set_edit_text(PROJECT_NAME, &p.name);
                ui.set_focus(Some(PROJECT_NAME));
                let host = p
                    .host_ids
                    .as_ref()
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .find(|h| *h != "local")
                    .map(str::to_owned);
                self.st.form = crate::state::ProjectForm {
                    icon: p.icon.clone().filter(|i| !i.is_empty()),
                    icon_color: p.icon_color.clone().filter(|c| !c.is_empty()),
                    host,
                    auto_name: String::new(),
                };
                self.st.dialog = Some(Dialog::Project(Some(name)));
            }
            Act::SidebarMode(m) => self.set_sidebar_view(|v| v.view_mode = m),
            Act::ProjectSort(m) => self.set_sidebar_view(|v| v.project_sort = m),
            Act::WorktreeSort(m) => self.set_sidebar_view(|v| v.worktree_sort = m),
            Act::WorktreeFilter(f) => self.set_sidebar_view(|v| v.worktree_filter = f),
            Act::ToggleSection => self.st.section_folded = !self.st.section_folded,
            Act::Group(g) => {
                // A group row toggles its filter; All Sessions clears both.
                self.st.group = g.filter(|g| self.st.group.as_ref() != Some(g));
                self.st.project = None;
                self.st.worktree = None;
                self.st.focused = None;
            }
            Act::ToggleGroup(id) => {
                if !self.st.groups_folded.remove(&id) {
                    self.st.groups_folded.insert(id);
                }
            }
            Act::NewGroup(session) => self.new_group(ui, session),
            Act::RenameGroup(id) => {
                let Some(name) = self.store.group(&id).map(|g| g.name.clone()) else {
                    return;
                };
                ui.set_edit_text(GROUP_NAME, &name);
                ui.set_focus(Some(GROUP_NAME));
                self.st.renaming_group = Some(id);
            }
            Act::GroupIcon(id) => self.open_menu(ui, MenuKind::GroupIcon(id), src),
            Act::SwitchWorkspace(id) => self.switch_workspace(id),
            Act::EditWorkspace(id) => self.edit_workspace(ui, id),
            Act::WorkspaceIcon(icon) => {
                if let Some(f) = &mut self.st.ws_form {
                    f.icon = icon;
                }
            }
            Act::WorkspaceColor(color) => {
                if let Some(f) = &mut self.st.ws_form {
                    f.color = color;
                }
            }
            Act::WorkspacePicker => {
                if let Some(f) = &mut self.st.ws_form {
                    f.picker = !f.picker;
                }
            }
            Act::SaveWorkspace => self.save_workspace(ui),
            Act::CancelWorkspace => {
                self.st.ws_form = None;
                self.st.confirm_delete = None;
                ui.set_focus(None);
            }
            Act::AskDeleteWorkspace(id) => self.st.confirm_delete = Some(id),
            Act::DeleteWorkspace(id) => self.delete_workspace(&id),
            Act::GroupIconName(id, icon) => self.patch_group(&id, "icon", icon),
            Act::GroupIconColor(id, color) => self.patch_group(&id, "iconColor", color),
            Act::UngroupAll(id) => {
                let name = self.store.group(&id).map(|g| g.name.clone());
                let members: Vec<String> = self
                    .store
                    .sessions
                    .iter()
                    .filter(|s| s.info.group_id.as_deref() == Some(id.as_str()))
                    .map(|s| s.info.id.clone())
                    .collect();
                for sid in members {
                    self.move_to_group(sid, None, Call::Quiet);
                }
                if let Some(name) = name {
                    self.st
                        .toast(format!("Sessions moved out of \"{name}\""), false);
                }
            }
            Act::AskDeleteGroup(id) => self.st.confirm_delete = Some(id),
            Act::DeleteGroup(id) => self.delete_group(&id),
            Act::MoveToGroup { session, group } => self.move_to_group(session, group, Call::Quiet),
            Act::AskDeleteProject(name) => self.st.confirm_delete = Some(name),
            Act::DeleteProject(name) => {
                let mut config = self.store.config.clone();
                if let Some(list) = config.get_mut("projects").and_then(Value::as_array_mut) {
                    list.retain(|p| p.get("name").and_then(Value::as_str) != Some(name.as_str()));
                }
                if self.st.project.as_deref() == Some(name.as_str()) {
                    self.st.project = None;
                }
                self.save_config(config, Call::Toast(format!("Project \"{name}\" deleted")));
            }
            Act::SaveProject => self.save_project(ui),
            Act::BrowseFolder => self.start_pick(
                Picked::Folder,
                crate::system::Pick::Folder("Select Project Folder".to_owned()),
            ),
            Act::ProjectIcon(icon) => self.st.form.icon = Some(icon),
            Act::ProjectColor(color) => self.st.form.icon_color = Some(color),
            Act::ProjectHost(host) => self.st.form.host = host,
            Act::Restore => {
                for (s, cause) in std::mem::take(&mut self.carried) {
                    self.show_carried(s, cause);
                }
                self.st.banner.clear();
            }
            Act::DismissToast(text) => {
                if let Some(i) = self.st.toasts.iter().position(|t| t.text == text) {
                    self.st.toasts.remove(i);
                }
            }
            Act::DismissBanner => {
                self.carried.clear();
                self.st.banner.clear();
                self.call(Call::Quiet, "sessions:clear", Value::Null);
            }
            Act::ResumeRecent(i) => {
                let Some(r) = self.st.recent.get(i).cloned() else {
                    return;
                };
                // Running already: show its card instead of starting a second agent.
                if let Some(id) = r.running_in {
                    self.go_to(id);
                    return;
                }
                let name = self
                    .store
                    .projects
                    .iter()
                    .find(|p| p.path == r.project_path)
                    .map_or_else(|| basename(&r.project_path).to_owned(), |p| p.name.clone());
                let params = json!({
                    "agentType": r.agent,
                    "projectName": name,
                    "projectPath": r.project_path,
                    "resumeSessionId": r.session_id,
                });
                self.call(Call::Created, "terminal:create", params);
            }
            Act::KillHeadless(id) => {
                self.call(Call::Quiet, "headless:kill", json!(id));
            }
            Act::HideHeadless(id) => self.st.headless.dismiss(&id),
            Act::ToggleHeadless(id) => self.st.headless.toggle(&id),
            Act::PickProject(name) => self.pick_project(name),
            Act::PickAgent(agent) => {
                self.st.launch.agent = Some(agent);
                self.list_models(false);
            }
            Act::PickModel(model) => {
                let agent = self.launch_agent().to_owned();
                match model {
                    Some(m) => self.st.models.picked.insert(agent, m),
                    None => self.st.models.picked.remove(&agent),
                };
                self.close_menu(ui);
            }
            Act::RefreshModels => self.list_models(true),
            Act::PickWorktree(path) => {
                self.st.launch.worktree = path;
                self.st.launch.new_worktree = false;
            }
            Act::NewWorktree(on) => {
                self.st.launch.new_worktree = on;
                self.st.launch.worktree = None;
            }
            Act::PickBranch(b) => self.st.launch.branch = b,
            Act::Launch => self.launch(),
            Act::Nothing => {}
        }
    }

    /// The folder's name, until another name is typed.
    fn project_path_changed(&mut self, ui: &mut Ui) {
        let name = ui.edit_text(PROJECT_NAME).to_owned();
        if name.is_empty() || name == self.st.form.auto_name {
            let auto = basename(ui.edit_text(PROJECT_PATH)).to_owned();
            ui.set_edit_text(PROJECT_NAME, &auto);
            self.st.form.auto_name = auto;
        }
    }

    /// AddProjectDialog's submit: the project added, or the one being
    /// edited replaced.
    fn save_project(&mut self, ui: &mut Ui) {
        let name = ui.edit_text(PROJECT_NAME).trim().to_owned();
        let path = ui.edit_text(PROJECT_PATH).trim().to_owned();
        if name.is_empty() || path.is_empty() {
            return;
        }
        let Some(Dialog::Project(editing)) = self.st.dialog.clone() else {
            return;
        };
        let mut config = self.store.config.clone();
        if !config.get("projects").is_some_and(Value::is_array) {
            config["projects"] = json!([]);
        }
        let Some(list) = config["projects"].as_array_mut() else {
            return;
        };
        let at = |n: &str| {
            list.iter()
                .position(|p| p.get("name").and_then(Value::as_str) == Some(n))
        };
        let old = editing.as_deref().and_then(at);
        if at(&name).is_some_and(|i| Some(i) != old) {
            self.st
                .toast(format!("A project named \"{name}\" already exists"), true);
            return;
        }
        let first = self.store.agents.iter().next().map(|a| a.key.clone());
        let mut project = old.map_or_else(
            || json!({ "preferredAgents": first.into_iter().collect::<Vec<_>>() }),
            |i| list[i].clone(),
        );
        let form = &self.st.form;
        project["name"] = json!(name);
        project["path"] = json!(path);
        project["icon"] = json!(form.icon.as_deref().unwrap_or(PROJECT_ICON));
        project["iconColor"] = json!(form.icon_color.as_deref().unwrap_or(PROJECT_COLOR));
        project["hostIds"] = json!([form.host.as_deref().unwrap_or("local")]);
        if project.get("workspaceId").is_none_or(Value::is_null) {
            project["workspaceId"] = json!(self.store.workspace());
        }
        let verb = match old {
            Some(i) => {
                list[i] = project;
                "updated"
            }
            None => {
                list.push(project);
                "added"
            }
        };
        if let Some(prev) = editing.filter(|p| *p != name) {
            for s in [&mut self.st.project, &mut self.st.composer.project] {
                if s.as_deref() == Some(prev.as_str()) {
                    *s = Some(name.clone());
                }
            }
        }
        self.st.dialog = None;
        ui.set_focus(None);
        self.save_config(config, Call::Toast(format!("Project \"{name}\" {verb}")));
    }

    /// Something the UI reported.
    pub fn event(&mut self, ui: &mut Ui, event: UiEvent) {
        if self.settings_event(ui, &event)
            || self.gate_event(ui, &event)
            || self.canvas_event(ui, &event)
            || self.form_event(ui, &event)
            || self.task_event(ui, &event)
            || self.wf_event(ui, &event)
            || self.pane_event(ui, &event)
            || self.palette_event(ui, &event)
            || self.plug_event(ui, &event)
            || self.net_event(ui, &event)
            || self.term_event(ui, &event)
        {
            return;
        }
        match event {
            UiEvent::Click(id) => {
                if let Some(act) = self.acts.clicked(id) {
                    let keep = matches!(
                        act,
                        Act::Open(_)
                            | Act::Nothing
                            | Act::Filter(_)
                            | Act::Sort(_)
                            | Act::Columns(_)
                            | Act::Layout(_)
                    ) || self.st.menu.as_ref().is_some_and(|m| {
                        matches!(m.kind, MenuKind::Recent | MenuKind::Headless)
                            && matches!(
                                act,
                                Act::ToggleHeadless(_)
                                    | Act::KillHeadless(_)
                                    | Act::HideHeadless(_)
                            )
                            || m.kind == MenuKind::WfIcon
                                && matches!(
                                    act,
                                    Act::Workflow(
                                        crate::workflows::WfAct::Icon(_)
                                            | crate::workflows::WfAct::IconColor(_)
                                    )
                                )
                            || matches!(m.kind, MenuKind::GroupIcon(_))
                                && matches!(act, Act::GroupIconName(..) | Act::GroupIconColor(..))
                            || m.kind == MenuKind::Workspace
                                && matches!(
                                    act,
                                    Act::EditWorkspace(_)
                                        | Act::WorkspaceIcon(_)
                                        | Act::WorkspaceColor(_)
                                        | Act::WorkspacePicker
                                        | Act::SaveWorkspace
                                        | Act::CancelWorkspace
                                        | Act::AskDeleteWorkspace(_)
                                        | Act::DeleteWorkspace(_)
                                )
                            || matches!(m.kind, MenuKind::SidebarOptions)
                                && matches!(
                                    act,
                                    Act::SidebarMode(_)
                                        | Act::ProjectSort(_)
                                        | Act::WorktreeSort(_)
                                        | Act::WorktreeFilter(_)
                                )
                    });
                    if !keep && self.st.menu.is_some() {
                        self.close_menu(ui);
                    }
                    if ui.menu_open(crate::screen::DOCK)
                        && !matches!(act, Act::DockPopover | Act::Gate(_))
                    {
                        ui.close_menu();
                    }
                    self.run(
                        ui,
                        act,
                        Src {
                            id: Some(id),
                            at: None,
                        },
                    );
                }
            }
            UiEvent::Context { id, x, y } => {
                if let Some(act) = self.acts.context_of(id) {
                    if let Act::Open(kind) = act {
                        self.st.menu = None;
                        self.open_menu(
                            ui,
                            kind,
                            Src {
                                id: Some(id),
                                at: Some((x, y)),
                            },
                        );
                    } else {
                        self.run(
                            ui,
                            act,
                            Src {
                                id: Some(id),
                                at: Some((x, y)),
                            },
                        );
                    }
                }
            }
            UiEvent::DoubleClick(id) => {
                if let Some(act) = self.acts.double_of(id) {
                    self.run(
                        ui,
                        act,
                        Src {
                            id: Some(id),
                            at: None,
                        },
                    );
                }
            }
            UiEvent::Wheel {
                id,
                dy,
                x,
                y,
                phase,
            } => self.wheel(ui, id, dy, (x, y), phase),
            UiEvent::Select { group, index } if group == VIEWS => {
                let act = match index.checked_sub(1) {
                    None => Some(Act::View(MainView::Sessions)),
                    Some(i) => self.st.points.top_bar().get(i).map(|p| MainView::act(p)),
                };
                if let Some(act) = act {
                    self.run(ui, act, Src::default());
                }
            }
            UiEvent::Select { group, index } if group == menus::MENU || group == menus::SUB => {
                self.choose(ui, group, index);
            }
            UiEvent::Dismiss(id) if self.st.points.more == Some(id.0) => {
                self.st.points.more = None;
            }
            UiEvent::Dismiss(id) if id == menus::MENU => {
                self.st.menu = None;
                self.st.confirm_delete = None;
            }
            UiEvent::Submit(id)
                if matches!(
                    self.acts.text_of(id),
                    Some(Act::Point(crate::points::PointAct::TreeField { .. }))
                ) =>
            {
                if let Some(Act::Point(a)) = self.acts.text_of(id) {
                    self.run_point(ui, a);
                }
            }
            UiEvent::Submit(id) => self.submit(ui, id),
            UiEvent::Changed(id) if self.pane_changed(ui, id) => {}
            UiEvent::Changed(id)
                if matches!(
                    self.acts.text_of(id),
                    Some(Act::Point(crate::points::PointAct::TreeLive { .. }))
                ) =>
            {
                if let Some(Act::Point(a)) = self.acts.text_of(id) {
                    self.run_point(ui, a);
                }
            }
            UiEvent::Resized { id, ratio } => {
                if !self.flex_resized(ui, id, ratio, false) {
                    self.pane_resized(ui, id, ratio, false);
                }
            }
            UiEvent::ResizeEnd { id, ratio } => {
                if !self.flex_resized(ui, id, ratio, true) {
                    self.pane_resized(ui, id, ratio, true);
                }
            }
            UiEvent::Changed(id) if id == COMPOSER_INPUT => {
                ui.edit_text(id).clone_into(&mut self.st.composer.text);
            }
            UiEvent::Changed(id) if id == MODEL_ID => self.st.models.invalid = None,
            UiEvent::Changed(id) if id == PROJECT_PATH => self.project_path_changed(ui),
            _ => {}
        }
    }

    fn submit(&mut self, ui: &mut Ui, id: Id) {
        if id == COMPOSER_INPUT {
            self.launch();
            return;
        }
        if id == PROJECT_NAME || id == PROJECT_PATH {
            self.save_project(ui);
            return;
        }
        if id == GROUP_NAME {
            self.rename_group(ui);
            return;
        }
        if id == WS_NAME {
            self.save_workspace(ui);
            return;
        }
        if id == MODEL_ID {
            self.choose_typed_model(ui);
            return;
        }
        if let Some(sid) = self.st.renaming.clone().filter(|s| rename_id(s) == id) {
            let name = ui.edit_text(id).trim().to_owned();
            self.st.renaming = None;
            ui.set_focus(None);
            if name.is_empty() || self.store.session(&sid).map(|s| s.title()) == Some(name.clone())
            {
                return;
            }
            if let Some(s) = self.store.sessions.iter_mut().find(|s| s.info.id == sid) {
                s.info.display_name = Some(name.clone());
            }
            self.call(
                Call::Toast(format!("Renamed to \"{name}\"")),
                "terminal:rename",
                json!({ "id": sid, "displayName": name }),
            );
            return;
        }
        let Some(sid) = self
            .store
            .sessions
            .iter()
            .find(|s| intent_id(&s.info.id) == id)
            .map(|s| s.info.id.clone())
        else {
            return;
        };
        let line = ui.edit_text(id).to_owned();
        self.run_line(ui, &sid, &line);
    }

    /// A menu entry chosen: its submenu opened, or its act run.
    fn choose(&mut self, ui: &mut Ui, group: Id, index: usize) {
        let Some(menu) = self.st.menu.clone() else {
            return;
        };
        let items = menus::items(&self.screen(ui), &menu.kind);
        let Some(item) = menus::chosen(items, menu.sub.open, group, index) else {
            return;
        };
        // An entry with a submenu toggles it.
        if group == menus::MENU && !item.sub.is_empty() {
            if let Some(m) = &mut self.st.menu {
                m.sub.click(index, ui.now());
            }
            ui.reopen_menu();
            return;
        }
        let Some(act) = item.act else { return };
        let stays = matches!(
            act,
            Act::Filter(_)
                | Act::Sort(_)
                | Act::Columns(_)
                | Act::Layout(_)
                | Act::AskDeleteProject(_)
                | Act::AskDeleteGroup(_)
                | Act::SidebarMode(_)
                | Act::ProjectSort(_)
                | Act::WorktreeSort(_)
                | Act::WorktreeFilter(_)
                | Act::Task(crate::tasks::TaskAct::AskDelete(_))
        );
        if stays {
            ui.reopen_menu();
        } else {
            self.close_menu(ui);
        }
        self.run(ui, act, Src::default());
    }

    /// Which of the open menu's entries have a submenu.
    fn menu_subs(&self, ui: &Ui) -> Option<Vec<bool>> {
        let menu = self.st.menu.as_ref()?;
        let items = menus::items(&self.screen(ui), &menu.kind);
        Some(items.iter().map(|i| !i.sub.is_empty()).collect())
    }

    /// The open menu's submenu keys: Right and Enter open one, Left and
    /// Escape close it.
    fn menu_key(&mut self, ui: &mut Ui, input: &Input) -> bool {
        let Input::Key { code, .. } = input else {
            return false;
        };
        let Some(subs) = self.menu_subs(ui) else {
            return false;
        };
        let Some(m) = &mut self.st.menu else {
            return false;
        };
        crate::ui::menu::key(ui, &mut m.sub, menus::MENU, menus::SUB, &subs, code)
    }

    /// Input before the UI sees it: the global shortcuts, and Escape
    /// closing what is open. True when it was used.
    pub fn intercept(&mut self, ui: &mut Ui, input: &Input) -> bool {
        if self.st.menu.is_some() && self.menu_key(ui, input) {
            return true;
        }
        if self.palette_key(ui, input) || self.onboard_key(ui, input) {
            return true;
        }
        if self.settings_key(ui, input) {
            return true;
        }
        if let Input::Key { code, mods: m, .. } = input {
            let with_mod = *m & MOD() != 0;
            if !self.st.settings.open
                && (self.git_key(ui, code, with_mod)
                    || self.pane_key(ui, code, with_mod, *m & mods::SHIFT != 0))
            {
                return true;
            }
            if self.st.dialog == Some(Dialog::Ask) && code == "Escape" {
                self.answer_ask(false);
                return true;
            }
            if self.st.dialog == Some(Dialog::Paste) && matches!(code.as_str(), "Escape" | "Enter")
            {
                self.confirm_paste(ui, code == "Enter");
                return true;
            }
        }
        if self.st.settings.open {
            return false;
        }
        if self.st.menu.is_none() && (self.task_key(ui, input) || self.wf_key(ui, input)) {
            return true;
        }
        if self.st.menu.is_none() && self.st.dialog.is_none() && self.intent_key(ui, input) {
            return true;
        }
        let Input::Key { code, mods: m, .. } = input else {
            return false;
        };
        if self.terminal_has_keys(ui) {
            if self.clip_key(ui, code, *m) {
                return true;
            }
            if terminal_key(code, *m) {
                self.input(input.clone());
                return true;
            }
        }
        // The approve and deny keys exist once an agent has asked for permission.
        let s = shortcut(code, *m)
            .filter(|s| !matches!(s, Shortcut::Permission(_)) || self.st.permission_keys);
        if let Some(s) = s {
            self.run_shortcut(ui, s);
            return true;
        }
        let view = self
            .st
            .points
            .views()
            .into_iter()
            .find(|p| {
                p.key
                    .as_deref()
                    .is_some_and(|k| crate::points::key_hit(k, code, *m, MOD()))
            })
            .map(MainView::act);
        if let Some(act) = view {
            self.run(ui, act, Src::default());
            return true;
        }
        let command = self
            .st
            .points
            .at(crate::points::kind::COMMANDS)
            .find(|p| {
                p.key
                    .as_deref()
                    .is_some_and(|k| crate::points::key_hit(k, code, *m, MOD()))
            })
            .map(crate::points::Point::key);
        if let Some(key) = command {
            let args = "{}".to_owned();
            self.run_point(ui, crate::points::PointAct::Choose { key, args });
            return true;
        }
        // Tab in the composer picks the next project, not the next control.
        let bare = self.st.dialog.is_none()
            && self.st.menu.is_none()
            && matches!(ui.focus(), None | Some(COMPOSER_INPUT));
        if code == "Tab" && bare && self.composing() {
            self.next_project(*m & mods::SHIFT != 0);
            return true;
        }
        if code == "Escape" && *m == 0 && self.address_revert(ui) {
            return true;
        }
        // Escape in the switcher's form cancels the form, not the switcher.
        if code == "Escape" && self.st.ws_form.is_some() {
            self.run(ui, Act::CancelWorkspace, Src::default());
            return true;
        }
        if self.st.menu.is_some() || code != "Escape" || *m & (MOD() | mods::ALT) != 0 {
            return false;
        }
        if self.st.dialog == Some(Dialog::AddTask) {
            self.close_task_dialog(ui);
        } else if self.st.dialog == Some(Dialog::Missed) {
        } else if self.st.dialog.is_some() {
            self.st.dialog = None;
            self.st.wf.prompt = None;
            ui.set_focus(None);
        } else if self.st.renaming.is_some() {
            self.st.renaming = None;
            ui.set_focus(None);
        } else if self.st.renaming_group.is_some() {
            self.st.renaming_group = None;
            ui.set_focus(None);
        } else if self.st.confirm_close.is_some() {
            self.st.confirm_close = None;
        } else if self.pane_escape() {
        } else if self.st.composer_overlay(&self.store) {
            self.st.composer_open = false;
        } else {
            return false;
        }
        true
    }

    /// Whether a terminal holds the keyboard: typing reaches a card and no
    /// control, dialog, menu or composer has it.
    pub fn terminal_has_keys(&self, ui: &Ui) -> bool {
        ui.focus().is_none()
            && self.st.dialog.is_none()
            && self.st.menu.is_none()
            && !self.st.settings.open
            && self.st.view == MainView::Sessions
            && !self.composing()
            && self.grid.is_some()
            && self.typing_target().is_some()
    }

    /// One key or IME event the UI did not use: the composer's or the
    /// terminal's.
    pub fn input(&mut self, input: Input) {
        if self.st.dialog.is_some() || self.st.menu.is_some() || self.st.settings.open {
            return;
        }
        if self.composing() || self.st.view != MainView::Sessions {
            return;
        }
        let (Some(g), Some(id)) = (self.grid.clone(), self.typing_target()) else {
            return;
        };
        let modifier = matches!(&input, Input::Key { code, .. }
            if ["Shift", "Control", "Alt", "Meta"].iter().any(|m| code.starts_with(m)));
        if !matches!(input, Input::Preedit(_)) && !modifier {
            self.return_to_live(&id);
        }
        match input {
            Input::Key { code, mods, text } => {
                if !MODIFIER_KEYS.contains(&code.as_str()) {
                    self.st.sel = None;
                }
                g.key(&id, &code, mods, text.as_deref());
            }
            Input::Preedit(s) => self.preedit = s,
            Input::Commit(s) => {
                self.preedit.clear();
                self.st.sel = None;
                g.text(&id, &s);
            }
        }
    }

    /// Text a page pasted: into the focused field, else as a paste into the
    /// terminal being typed into.
    pub fn paste(&mut self, ui: &mut Ui, text: String) {
        let chord = Input::Key {
            code: "KeyV".into(),
            mods: MOD(),
            text: None,
        };
        let busy = self.st.dialog.is_some() || self.st.menu.is_some() || self.st.settings.open;
        if self.intercept(ui, &chord) || ui.key(&chord) || busy {
            return self.events(ui);
        }
        if self.composing() {
            ui.set_focus(Some(COMPOSER_INPUT));
            ui.key(&Input::Commit(text));
        } else if self.st.view != MainView::Sessions {
            self.input(Input::Commit(text));
        } else if let (Some(g), Some(id)) = (self.grid.clone(), self.typing_target()) {
            self.return_to_live(&id);
            g.paste_clipboard(&id, &text);
        }
        self.events(ui);
    }

    /// Whether vornd refused this client's key.
    pub fn rejected(&self) -> bool {
        self.rpc.rejected()
    }

    /// Input as a window delivers it: shortcuts first, then the UI, then
    /// the composer or the terminal, and what the UI reported after.
    pub fn send(&mut self, ui: &mut Ui, input: Input) {
        if !self.intercept(ui, &input) && !ui.key(&input) {
            if self.claim_typing(ui, &input) {
                ui.key(&input);
            } else {
                self.input(input);
            }
        }
        self.events(ui);
    }

    /// Runs what the UI reported.
    pub fn events(&mut self, ui: &mut Ui) {
        for e in ui.take_events() {
            self.event(ui, e);
        }
    }

    /// A press and release in the middle of the element `id` as the last
    /// frame laid it out, and what it did.
    pub fn click(&mut self, ui: &mut Ui, id: Id) -> bool {
        let Some(r) = ui.rect_of(id) else {
            return false;
        };
        ui.pointer_move(r.x + r.w / 2.0, r.y + r.h / 2.0);
        ui.pointer_down();
        ui.pointer_up();
        self.events(ui);
        true
    }

    /// A secondary press in the middle of the element `id`, and what it did.
    pub fn right_click(&mut self, ui: &mut Ui, id: Id) -> bool {
        let Some(r) = ui.rect_of(id) else {
            return false;
        };
        ui.pointer_move(r.x + r.w / 2.0, r.y + r.h / 2.0);
        ui.pointer_down_secondary();
        self.events(ui);
        true
    }

    /// The open menu's top-level entries, by label.
    pub fn menu_labels(&self, ui: &Ui) -> Vec<String> {
        let Some(m) = &self.st.menu else {
            return Vec::new();
        };
        menus::items(&self.screen(ui), &m.kind)
            .into_iter()
            .map(|i| i.label)
            .collect()
    }

    /// The id the control doing `act` in `place` has on screen.
    pub fn control(place: &str, act: &Act) -> Id {
        Acts::id(place, act)
    }

    /// Tab in the composer: the next project, or the one before with Shift.
    fn next_project(&mut self, back: bool) {
        let names: Vec<String> = self.store.projects.iter().map(|p| p.name.clone()).collect();
        let at = self
            .st
            .composer
            .project
            .as_deref()
            .and_then(|p| names.iter().position(|n| n == p));
        if let Some(i) = cycle(at, names.len(), !back) {
            self.pick_project(names[i].clone());
        }
    }

    /// Typing no control took goes to the composer while it shows.
    fn claim_typing(&self, ui: &mut Ui, input: &Input) -> bool {
        let typing = match input {
            Input::Key {
                code,
                mods: m,
                text,
            } => {
                m & (mods::CTRL | mods::SUPER) == 0
                    && (text.is_some() || matches!(code.as_str(), "Backspace" | "Delete"))
                    && !matches!(code.as_str(), "Tab" | "Escape")
            }
            Input::Preedit(_) | Input::Commit(_) => true,
        };
        let free = self.st.dialog.is_none() && self.st.menu.is_none() && !self.st.settings.open;
        let claim = typing && free && self.composing();
        if claim {
            ui.set_focus(Some(COMPOSER_INPUT));
        }
        claim
    }

    /// `terminal:create` with `params`; with every agent off the session is
    /// a plain shell, started where the agent would have been.
    fn create_session(&mut self, call: Call, params: Value) {
        if params["agentType"] != "shell" {
            self.call(call, "terminal:create", params);
            return;
        }
        let cwd = params
            .get("existingWorktreePath")
            .filter(|w| w.is_string())
            .unwrap_or(&params["projectPath"])
            .clone();
        self.call(call, "shell:create", cwd);
    }

    /// PromptLauncher's launch: the chosen agent in the chosen project (or
    /// its worktree), with what was typed as its first prompt.
    fn launch(&mut self) {
        if self.st.composer.launching {
            return;
        }
        let Some((name, path)) = self.composer_project() else {
            return;
        };
        let l = &self.st.launch;
        let mut params = json!({
            "agentType": l.agent.as_deref().unwrap_or(self.store.default_agent()),
            "projectName": name,
            "projectPath": path,
        });
        if l.new_worktree {
            params["useWorktree"] = json!(true);
        } else if let Some(wt) = &l.worktree {
            params["existingWorktreePath"] = json!(wt);
        }
        if let Some(b) = &l.branch {
            params["branch"] = json!(b);
        }
        if let Some(m) = self.st.models.picked.get(self.launch_agent()) {
            params["model"] = json!(m);
        }
        let prompt = self.st.composer.text.trim();
        if !prompt.is_empty() {
            params["initialPrompt"] = json!(prompt);
        }
        self.create_session(Call::Launch, params);
        self.st.composer.launching = true;
        self.st.composer.error = None;
    }

    fn screen<'a>(&'a self, ui: &'a Ui) -> Screen<'a> {
        Screen {
            ui,
            acts: &self.acts,
            store: &self.store,
            st: &self.st,
            panes: &self.views,
            logo: self.logo,
            agents: &self.agents,
            size: self.size,
            chrome: self.chrome,
            traffic: self.traffic,
            web_url: &self.web_url,
            term_drawing: self.term_drawing,
        }
    }

    /// Builds, lays out and paints a frame into `ui.scene`, attaching and
    /// sizing each card's terminal to the box it got.
    pub fn frame(&mut self, ui: &mut Ui, size: (f32, f32)) -> TreeUpdate {
        let clear = if ui.see_through() {
            crate::ui::theme::shadow(0.0)
        } else {
            look::SURFACE_BASE.rgba()
        };
        ui.begin(clear);
        let root = self.view(ui, size);
        let laid = ui.layout(root, size);
        self.paint(ui, &laid);
        laid.tree
    }

    /// The frame's element tree for a `size` window, after taking in what
    /// vornd sent.
    pub fn view(&mut self, ui: &mut Ui, size: (f32, f32)) -> El {
        self.system_look = (ui.system_dark(), ui.reduced_transparency());
        self.st.settings.theme_preview = self.hovered_theme(ui);
        self.pump();
        self.flex_tick(ui);
        self.grid_replies(ui);
        if std::mem::take(&mut self.st.focus_terminal) {
            ui.set_focus(None);
        }
        let composing = self.composing();
        if composing {
            if ui.edit_text(COMPOSER_INPUT) != self.st.composer.text {
                ui.set_edit_text(COMPOSER_INPUT, &self.st.composer.text);
            }
            let free = self.st.dialog.is_none() && self.st.menu.is_none() && !self.st.settings.open;
            if !self.was_composing && free && ui.focus().is_none() {
                ui.set_focus(Some(COMPOSER_INPUT));
            }
        } else if ui.focus() == Some(COMPOSER_INPUT) {
            ui.set_focus(None);
        }
        self.was_composing = composing;
        let now = web_time::Instant::now();
        for notice in std::mem::take(&mut self.notices) {
            if self.notifier.admit(&notice, ui.window_focused(), now) {
                self.notice_sink.show(notice);
            }
        }
        self.seen_tick(ui);
        self.git_tick();
        self.panes_tick(ui);
        self.tasks_tick(ui);
        self.net_tick(ui);
        self.plug_tick(ui);
        self.pick_tick(ui);
        self.headless_tick(ui, now);
        if self.st.signin.is_some() && self.st.dialog != Some(Dialog::SignIn) {
            self.finish_signin(Err(crate::signin::CLOSED.to_owned()));
        }
        self.hotkeys_tick(ui);
        self.ext_tick();
        self.points_tick(ui);
        if let Some(at) = self.session_tick(now) {
            ui.wake_at(at);
        }
        self.settings_tick(now);
        let focus = ui.focus();
        if focus != self.last_focus {
            if let Some(old) = self.last_focus {
                self.settings_blur(ui, old);
                self.tree_blur(ui, old);
            }
            self.last_focus = focus;
        }
        // `fontSize` 13 is the size the terminal font was matched at.
        let font = self
            .store
            .config
            .pointer("/defaults/fontSize")
            .and_then(Value::as_f64)
            .map_or(look::TERM_FONT, |f| look::TERM_FONT * f as f32 / 13.0);
        ui.text.set_term_size(font);
        self.size = size;
        self.agents.load(
            ui,
            self.store
                .agents
                .iter()
                .filter_map(|a| Some((a.key.as_str(), a.icon.as_deref()?, a.mark.as_deref()))),
        );
        if self.logo.is_none() {
            // A logo that does not decode costs only the logo.
            self.logo = decode_png(look::LOGO_PNG)
                .ok()
                .and_then(|(w, h, rgba)| ui.image(w, h, &rgba));
        }
        // A hovered entry with a submenu opens it, unless the pointer is
        // on its way to the one open.
        if let Some(subs) = self.menu_subs(ui) {
            if let Some(m) = &mut self.st.menu {
                crate::ui::menu::track(ui, &mut m.sub, menus::MENU, menus::SUB, &subs);
            }
        }
        if self.st.menu.is_some() && !ui.menu_open(menus::MENU) {
            self.st.menu = None;
        }
        if let Some(id) = self.st.wf.open_next.take() {
            self.run_wf(ui, crate::workflows::WfAct::Edit(id), Src::default());
        }
        self.web_slots(ui);
        self.acts.clear();
        self.shell_frame(ui);
        let root = self.screen(ui).build();
        self.seed_texts(ui);
        ui.custom_a11y.clear();
        let views = &self.views;
        self.spoken.retain(|id, _| views.contains_key(id));
        for (i, s) in self.store.sessions.iter().enumerate() {
            let id = crate::screen::CARD_ID_BASE + i as u64;
            let text = self.spoken.get(&s.info.id).cloned().unwrap_or_default();
            ui.custom_a11y.insert(id, (Role::Terminal, s.title(), text));
        }
        root
    }

    /// A wheel or trackpad event over a terminal: the program's when it
    /// tracks the mouse, arrow keys on the alternate screen, else a scroll
    /// through history.
    fn wheel(&mut self, ui: &mut Ui, id: Id, dy: f32, (x, y): (f32, f32), phase: WheelPhase) {
        let Some(g) = self.grid.clone() else { return };
        let now = ui.now();
        // A gesture stays with the pane it began on, momentum and all.
        let latched = self
            .wheel
            .take()
            .filter(|w| now.duration_since(w.3).as_millis() < 200);
        let (session, route, mut rows) = match latched {
            Some((s, route, rows, _)) => (s, route, rows),
            None => {
                let Some(s) = self
                    .store
                    .sessions
                    .iter()
                    .find(|s| term_id(&s.info.id) == id)
                else {
                    return;
                };
                let Some(v) = self.views.get(&s.info.id) else {
                    return;
                };
                (
                    s.info.id.clone(),
                    scroll::route(v.mouse, !v.primary),
                    Rows::default(),
                )
            }
        };
        let k = ui.scale();
        let cell = ui.text.cell();
        let Some(r) = ui.rect_of(id) else { return };
        // Pixel travel meets physical rows; a notch counts in logical WHEEL_LINE units.
        let dy = if phase == WheelPhase::Line {
            dy
        } else {
            dy * k
        };
        if route == Route::History {
            let history = self.views.get(&session).map_or(0, |v| v.history);
            let s = self.scrolls.entry(session.clone()).or_default();
            s.set_metrics(cell.h, r.h * k, history);
            s.reduced = ui.reduced_motion();
            s.lines = vornui::input::wheel_lines();
            s.wheel(dy, phase, now);
            self.st.scrolled_at.insert(session.clone(), now);
            ui.wake_at(now);
        } else {
            let n = rows.take(dy, phase, cell.h, route);
            let col = ((x - r.x) * k / cell.w).floor().max(0.0);
            let row = ((y - r.y) * k / cell.h).floor().max(0.0);
            g.wheel_rows(&session, n, route, col, row);
        }
        self.wheel = Some((session, route, rows, now));
    }

    /// `session`'s terminal at `back` rows up its history, at once: where a
    /// scrollbar drag or page put it.
    pub(crate) fn scroll_back_to(&mut self, ui: &mut Ui, session: &str, back: u64) {
        let Some(v) = self.views.get(session) else {
            return;
        };
        let back = back.min(v.history);
        let row = ui.text.cell().h;
        let span = self.term_boxes.get(session).map_or(row, |r| r.h);
        let s = self.scrolls.entry(session.to_owned()).or_default();
        s.set_metrics(row, span, v.history);
        s.jump(back as f32 * row);
        let now = ui.now();
        self.st.scrolled_at.insert(session.to_owned(), now);
        ui.wake_at(now);
    }

    /// Back to the live screen at once, as typing into a terminal does.
    fn return_to_live(&mut self, session: &str) {
        if let Some(s) = self.scrolls.get_mut(session) {
            s.reset();
        }
        if let Some(g) = &self.grid {
            if g.scroll_to(session, 0) {
                if let Some(v) = g.view(session) {
                    self.views.insert(session.to_owned(), v);
                }
            }
        }
    }

    /// Pixels `session`'s terminal is scrolled above the live bottom.
    pub fn scroll_position(&self, session: &str) -> f32 {
        self.scrolls.get(session).map_or(0.0, Scroll::pos)
    }

    /// Whether `session`'s terminal shows history rather than the live screen.
    pub fn scrolled_back(&self, session: &str) -> bool {
        self.scrolls.get(session).is_some_and(Scroll::scrolled)
    }

    /// Moves a painted pane's scroll on a frame and lines its view up with
    /// it; returns the pixels to draw the view down by.
    fn scroll_frame(&mut self, ui: &Ui, session: &str, h: f32) -> f32 {
        let (Some(s), Some(v), Some(g)) = (
            self.scrolls.get_mut(session),
            self.views.get(session),
            &self.grid,
        ) else {
            return 0.0;
        };
        let row = ui.text.cell().h;
        let history = v.history;
        s.set_metrics(row, h, history);
        let now = ui.now();
        if let Some(at) = s.tick(now) {
            ui.wake_at(at);
        }
        let (back, _) = s.split();
        if back != v.back() && g.scroll_to(session, back) {
            if let Some(nv) = g.view(session) {
                self.views.insert(session.to_owned(), nv);
            }
        }
        let back = self.views.get(session).map_or(0, PaneView::back);
        let dy = (s.pos() - back as f32 * row).round();
        if s.resting() && !s.scrolled() {
            self.scrolls.remove(session);
        }
        dy
    }

    /// Paints each card's terminal into the box it got, attaching and
    /// sizing the terminal to it.
    pub fn paint(&mut self, ui: &mut Ui, laid: &Laid) {
        let cell = ui.text.cell();
        let current = self.typing_target();
        self.term_boxes.clear();
        for (id, r) in &laid.customs {
            if let Some(tid) =
                crate::extensions::term_index(*id).and_then(|i| self.st.ext.slots.get(i))
            {
                let tid = tid.clone();
                self.paint_terminal(ui, &tid, *r, current.as_deref() == Some(tid.as_str()));
                continue;
            }
            if self.paint_block(ui, *id, *r) {
                continue;
            }
            let Some(i) = card_index(*id) else { continue };
            let Some(session) = self.store.sessions.get(i) else {
                continue;
            };
            let sid = session.info.id.as_str();
            let split = crate::log::split(
                &self.store.config,
                &self.st.points,
                session,
                self.st.shells.get(sid),
            )
            .is_some();
            if !split {
                if let Some(g) = &self.grid {
                    if !session.ended() {
                        // Whole cells only; f32 to u16 saturates.
                        let cols = (r.w / cell.w).floor() as u16;
                        let rows = (r.h / cell.h).floor() as u16;
                        g.attach(
                            sid,
                            cols,
                            rows,
                            cell_px(ui),
                            if session.is_shell() {
                                crate::grid::SHELL_HISTORY
                            } else {
                                0
                            },
                        );
                        if !self.st.locked.contains(sid) {
                            g.resize(sid, cols, rows, cell_px(ui));
                        }
                    }
                }
            }
            let sid = sid.to_owned();
            let dy = if split {
                0.0
            } else {
                self.scroll_frame(ui, &sid, r.h)
            };
            let sid = sid.as_str();
            let caret = if current.as_deref() != Some(sid) {
                paint::Caret::Hidden
            } else if self.st.renaming.is_none() && self.st.dialog.is_none() && ui.focus().is_none()
            {
                paint::Caret::Live
            } else {
                paint::Caret::Hollow
            };
            let pre = if caret == paint::Caret::Live {
                self.preedit.as_str()
            } else {
                ""
            };
            if split {
                if let Some(at) = self.paint_live(ui, i, *r, caret, pre) {
                    self.term_boxes.insert(sid.to_owned(), at);
                }
                continue;
            }
            if let Some(v) = self.views.get(sid) {
                let sel = match &self.st.sel {
                    Some((s, sel)) if s == sid => sel.rows(v),
                    _ => Vec::new(),
                };
                if ui.is_cells() {
                    paint::pane_cells(ui, v, *r, caret);
                } else {
                    // A screen not yet resized to a narrower box stays inside it.
                    ui.scene.push_clip(*r);
                    paint::pane(ui, v, *r, caret, pre, &sel, dy);
                    ui.scene.pop_clip();
                }
            }
            self.term_boxes.insert(sid.to_owned(), *r);
        }
        self.paint_web(ui, laid);
        self.paint_video(ui, laid);
    }

    /// An extension's program terminal, attached and sized to its box.
    fn paint_terminal(&self, ui: &mut Ui, id: &str, r: crate::ui::Rect, focused: bool) {
        let cell = ui.text.cell();
        if let Some(g) = &self.grid {
            let cols = (r.w / cell.w).floor() as u16;
            let rows = (r.h / cell.h).floor() as u16;
            g.attach(id, cols, rows, cell_px(ui), 0);
            g.resize(id, cols, rows, cell_px(ui));
        }
        if let Some(v) = self.views.get(id) {
            let focused = focused && self.st.dialog.is_none() && ui.focus().is_none();
            let pre = if focused { self.preedit.as_str() } else { "" };
            let caret = if focused {
                paint::Caret::Live
            } else {
                paint::Caret::Hollow
            };
            if ui.is_cells() {
                paint::pane_cells(ui, v, r, caret);
                return;
            }
            ui.scene.push_clip(r);
            paint::pane(ui, v, r, caret, pre, &[], 0.0);
            ui.scene.pop_clip();
        }
    }
}

/// One terminal cell in device pixels, for the pixel size a program reads
/// from its terminal; unknown (0) when drawing to a character grid.
fn cell_px(ui: &Ui) -> (u32, u32) {
    if ui.is_cells() {
        return (0, 0);
    }
    let cell = ui.text.cell();
    // f32 to u32 saturates.
    (cell.w.round() as u32, cell.h.round() as u32)
}

/// An answer that should be a list, else none.
fn list(answer: Result<Value, RpcError>) -> Vec<Value> {
    match answer {
        Ok(Value::Array(v)) => v,
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_match_the_renderer() {
        assert_eq!(
            shortcut("KeyS", MOD()),
            Some(Shortcut::View(MainView::Sessions))
        );
        // Tasks' and Workflows' keys come from their manifests, with their views.
        assert_eq!(shortcut("KeyT", MOD()), None);
        assert_eq!(shortcut("KeyW", MOD() | mods::SHIFT), None);
        assert_eq!(shortcut("Digit3", MOD()), Some(Shortcut::Jump(2)));
        assert_eq!(shortcut("Digit0", MOD()), None);
        assert_eq!(shortcut("BracketRight", MOD()), Some(Shortcut::Cycle(true)));
        assert_eq!(
            shortcut("ArrowUp", MOD() | mods::SHIFT),
            Some(Shortcut::Prompt(true))
        );
        assert_eq!(shortcut("ArrowDown", MOD()), None);
        use crate::flex::Side;
        assert_eq!(
            shortcut("ArrowLeft", MOD() | mods::ALT),
            Some(Shortcut::PaneFocus(Side::Left))
        );
        assert_eq!(
            shortcut("ArrowDown", MOD() | mods::ALT | mods::SHIFT),
            Some(Shortcut::PaneResize(Side::Down))
        );
        assert_eq!(
            shortcut("Equal", MOD() | mods::ALT),
            Some(Shortcut::PaneEqualize)
        );
        assert_eq!(shortcut("Backquote", mods::CTRL), Some(Shortcut::NewShell));
        assert_eq!(shortcut("Slash", MOD()), Some(Shortcut::Shortcuts));
        assert_eq!(shortcut("KeyK", MOD()), Some(Shortcut::Palette));
        assert_eq!(
            shortcut("KeyA", MOD() | mods::SHIFT),
            Some(Shortcut::Permission(true))
        );
        assert_eq!(
            shortcut("KeyD", MOD() | mods::SHIFT),
            Some(Shortcut::Permission(false))
        );
        assert_eq!(shortcut("KeyJ", MOD()), Some(Shortcut::ViewOptions));
        assert_eq!(shortcut("KeyB", MOD()), Some(Shortcut::Sidebar));
        assert_eq!(shortcut("F2", 0), Some(Shortcut::Rename));
        assert_eq!(
            shortcut("Digit3", mods::ALT),
            Some(Shortcut::Filter(Filter::Waiting))
        );
        assert_eq!(shortcut("Digit6", mods::ALT), None);
        // Unmodified and extra-modified keys belong to the terminal.
        assert_eq!(shortcut("KeyS", 0), None);
        assert_eq!(shortcut("KeyS", MOD() | mods::ALT), None);
        assert_eq!(shortcut("Escape", 0), None);
    }

    #[test]
    fn a_terminal_takes_tab_before_focus_and_shortcuts() {
        for m in [
            0,
            mods::SHIFT,
            mods::CTRL,
            mods::CTRL | mods::SHIFT,
            mods::ALT,
        ] {
            assert!(terminal_key("Tab", m), "Tab with {m:#b}");
        }
        assert!(!terminal_key("Tab", mods::SUPER));
        assert!(!terminal_key("Escape", 0));
        assert!(!terminal_key("KeyA", 0));
        // App shortcuts on the command key stay the app's.
        for code in ["KeyK", "KeyN", "KeyW", "KeyB", "BracketRight", "Slash"] {
            assert!(!terminal_key(code, MOD()), "{code}");
        }
        assert!(!terminal_key("KeyW", MOD() | mods::SHIFT));
        assert!(!terminal_key("Backquote", mods::CTRL));
        assert!(!terminal_key("KeyJ", MOD()));
    }

    #[test]
    fn cycling_wraps_and_starts_at_either_end() {
        assert_eq!(cycle(None, 3, true), Some(0));
        assert_eq!(cycle(None, 3, false), Some(2));
        assert_eq!(cycle(Some(2), 3, true), Some(0));
        assert_eq!(cycle(Some(0), 3, false), Some(2));
        assert_eq!(cycle(None, 0, true), None);
    }

    #[test]
    fn carried_sessions_say_why_they_ended() {
        assert_eq!(
            carried_cause(&json!({ "closedCleanly": true })),
            "Vorn was closed"
        );
        assert_eq!(
            carried_cause(&json!({ "rebooted": true })),
            "the machine restarted"
        );
        assert_eq!(carried_cause(&json!({})), "the server stopped unexpectedly");
    }
}
