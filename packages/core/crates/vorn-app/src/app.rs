//! The app: vornd's models and the grid folded into one state, the keys
//! routed the way the renderer's shortcuts route them, and each frame built
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
use crate::paint;
use crate::screen::{
    card_index, intent_id, menus, rename_id, term_id, Composer, MainView, Screen, PROJECT_NAME,
    PROJECT_PATH,
};
use crate::state::{
    basename, Dialog, Filter, Headless, LayoutMode, Menu, MenuKind, Recent, State, Worktree,
};
use crate::ui::{
    decode_png, item_id, mods, El, Event as UiEvent, Id, Input, Laid, Rgba, Role, TreeUpdate, Ui,
    WindowRequest,
};
use crate::view::PaneView;
use vorn_protocol::TerminalSession;

pub mod canvas;
pub mod forms;
mod gates;
mod git;
mod panes;
mod settings;
mod tasks;
mod workflows;

/// The modifier the renderer's shortcuts use (`modKey`).
pub const MOD: u16 = if cfg!(target_os = "macos") {
    mods::SUPER
} else {
    mods::CTRL
};

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
}

/// The renderer's global shortcuts (`useKeyboardShortcuts`). Escape is not
/// one: it closes what is open, else a terminal takes it, as xterm does.
pub fn shortcut(code: &str, m: u16) -> Option<Shortcut> {
    let only = |want: u16| m & (mods::CTRL | mods::SUPER | mods::ALT | mods::SHIFT) == want;
    if code == "Backquote" && only(mods::CTRL) {
        return Some(Shortcut::NewShell);
    }
    if only(MOD | mods::SHIFT) && code == "KeyW" {
        return Some(Shortcut::View(MainView::Workflows));
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
    if !only(MOD) {
        return None;
    }
    Some(match code {
        "KeyS" => Shortcut::View(MainView::Sessions),
        "KeyT" => Shortcut::View(MainView::Tasks),
        "KeyN" => Shortcut::NewSession,
        "KeyB" => Shortcut::Sidebar,
        "KeyJ" => Shortcut::ViewOptions,
        "KeyO" => Shortcut::Expand,
        "KeyW" => Shortcut::Collapse,
        "Slash" => Shortcut::Shortcuts,
        "BracketRight" => Shortcut::Cycle(true),
        "BracketLeft" => Shortcut::Cycle(false),
        _ => {
            let d = code.strip_prefix("Digit")?.parse::<usize>().ok()?;
            Shortcut::Jump(d.checked_sub(1)?)
        }
    })
}

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

/// A call whose answer changes the screen.
enum Call {
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
    Headless,
    Restored,
    Installed,
    Worktrees(String),
    Branches(String),
    /// A `config:save`; the next one waits for it.
    Saved(Box<Call>),
    Task(tasks::TaskCall),
    Workflow(workflows::WfCall),
    Set(settings::SetCall),
    Git(git::GitCall),
    Pane(panes::PaneCall),
    /// Nothing to do with the answer, an error included.
    Ignore,
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
    pub st: State,
    pub acts: Acts,
    preedit: String,
    calls: Vec<(Call, Pending)>,
    /// Config saves not yet sent: vornd runs them in parallel, so a later
    /// one could land first.
    saves: VecDeque<(Value, Call)>,
    saving: bool,
    logo: Option<u32>,
    /// The agents' marks, registered as images.
    agents: HashMap<&'static str, u32>,
    /// vornd hung up.
    pub closed: bool,
    /// The app draws its own title bar and window controls.
    pub chrome: bool,
    /// macOS: the traffic lights sit over the app's top bar.
    pub traffic: bool,
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
    /// The field that had focus last frame, to tell when one loses it.
    last_focus: Option<Id>,
    /// Worktree removals waiting for the sessions they closed.
    removals: Vec<git::Pending>,
    /// A discard the person agreed to is being run.
    discarding: bool,
}

impl App {
    /// Connects to the vornd at `ep`, loads the models and opens the grid;
    /// `wake` runs whenever either has news for the next frame.
    pub fn connect(ep: &Endpoint, wake: Wake) -> Result<App, RpcError> {
        let (rpc, events) = Rpc::connect(ep, Arc::clone(&wake))?;
        let store = Store::load(&rpc)?;
        // A grid that will not open still leaves the screen and its data.
        let (grid, grid_error) = match Grid::connect(&ep.grid, wake) {
            Ok(g) => (Some(g), None),
            Err(e) => (None, Some(format!("grid {}: {e}", ep.grid))),
        };
        let st = State {
            sidebar: true,
            layout: LayoutMode::from_config(&store.config),
            tasks: crate::tasks::Tasks::new(&store.config),
            settings: crate::settings::Settings::new(),
            ..State::default()
        };
        let mut app = App {
            rpc,
            events,
            store,
            grid,
            grid_error,
            views: HashMap::new(),
            st,
            acts: Acts::default(),
            preedit: String::new(),
            calls: Vec::new(),
            saves: VecDeque::new(),
            saving: false,
            logo: None,
            agents: HashMap::new(),
            closed: false,
            chrome: !cfg!(target_os = "macos"),
            traffic: cfg!(target_os = "macos"),
            size: look::WINDOW,
            port: ep.port,
            carried: Vec::new(),
            data_dir: None,
            health: None,
            last_focus: None,
            removals: Vec::new(),
            discarding: false,
        };
        app.call(Call::Restored, "sessions:restored", Value::Null);
        app.call(Call::Headless, "headless:list", Value::Null);
        app.call(Call::Installed, "agent:detectInstalled", Value::Null);
        Ok(app)
    }

    /// Where vornd keeps its data: the session holders announce themselves
    /// there, and this app keeps its own view of it (open panes, drafts)
    /// beside them.
    pub fn set_data_dir(&mut self, dir: std::path::PathBuf) {
        self.st.panes = crate::panes::Panes::load(Some(crate::panes::view_file(&dir)));
        self.st.editors.drafts =
            crate::editor::Drafts::load(Some(crate::editor::drafts_file(&dir)));
        self.data_dir = Some(dir);
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
    pub fn pump(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Notification { method, params } => {
                    if method == "workflow:runUpdated" {
                        self.task_run_updated(&params);
                        self.wf_run_updated(&params);
                    }
                    if method == "server:identity" {
                        self.st.server_version = params
                            .get("appVersion")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        continue;
                    }
                    if method == "scheduler:missed" {
                        self.wf_missed(&params);
                        continue;
                    }
                    if method == "worktree:confirmCleanup" {
                        self.git_offer(&params);
                        continue;
                    }
                    self.store.apply(&method, params);
                }
                Event::Closed => self.closed = true,
            }
        }
        let calls = std::mem::take(&mut self.calls);
        for (kind, pending) in calls {
            match pending.poll() {
                None => self.calls.push((kind, pending)),
                Some(answer) => self.answered(kind, answer),
            }
        }
        if let Some(g) = &self.grid {
            for id in g.take_dirty() {
                if let Some(v) = g.view(&id) {
                    self.st
                        .last_output
                        .insert(id.clone(), std::time::Instant::now());
                    self.views.insert(id, v);
                }
            }
            for id in g.attached() {
                if self.store.session(&id).is_none() {
                    g.detach(&id);
                    self.views.remove(&id);
                }
            }
        }
        let now = std::time::Instant::now();
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
            Call::Pane(c) => self.pane_answered(c, answer),
            Call::Ignore => {}
            Call::Saved(call) => {
                self.saving = false;
                self.answered(*call, answer);
                self.next_save();
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
            Call::Headless => {
                self.st.headless = list(answer)
                    .iter()
                    .filter_map(Headless::from_value)
                    .collect();
            }
            Call::Restored => self.restored(list(answer)),
            Call::Installed => {
                if let Ok(Value::Object(m)) = answer {
                    let found = m
                        .into_iter()
                        .filter(|(_, v)| v.as_bool() == Some(true))
                        .map(|(k, _)| k)
                        .collect();
                    self.st.installed = Some(found);
                }
            }
            Call::Worktrees(path) => {
                let wts = list(answer)
                    .iter()
                    .filter_map(Worktree::from_value)
                    .collect();
                self.st.worktrees.insert(path, wts);
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

    /// `sessions:resume`'s answer, as the renderer's `resumeEndedSession`
    /// takes it.
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
                self.store.remove(old);
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
        if let Some(t) = &self.st.panes.focus {
            if current.as_deref().is_some() && self.st.panes.holder(t) == current.as_deref() {
                return Some(t.clone());
            }
        }
        current
    }

    /// Moves to the card `id`: onto the stage when one is focused.
    fn go_to(&mut self, id: String) {
        self.st.view = MainView::Sessions;
        if self.st.focused.is_some() {
            self.st.focused = Some(id.clone());
        }
        self.st.selected = Some(id);
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
            Shortcut::Sidebar => self.st.sidebar = !self.st.sidebar,
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
        }
    }

    /// Opens `kind` at its control, or closes it when it is already open.
    fn open_menu(&mut self, ui: &mut Ui, kind: MenuKind, src: Src) {
        if self.st.menu.as_ref().is_some_and(|m| m.kind == kind) {
            self.close_menu(ui);
            return;
        }
        let w = menus::width(&kind);
        let rect = src.id.and_then(|id| ui.rect_of(id));
        let mut at = match (src.at, rect) {
            (Some(p), _) => p,
            (None, Some(r)) if kind.is_launch() => (r.x, r.y - 4.0),
            (None, Some(r))
                if matches!(
                    kind,
                    MenuKind::Grid | MenuKind::TaskOptions | MenuKind::TaskCard(_)
                ) =>
            {
                (r.x + r.w - w, r.y + r.h + 4.0)
            }
            // `OpenInButton` opens upwards, its right edge on the chip's.
            (None, Some(r)) if matches!(kind, MenuKind::OpenIn(_)) => {
                let n = self.st.git.ides.as_ref().map_or(0, Vec::len);
                let h = crate::screen::git::open_in_height(n);
                (r.x + r.w - w, (r.y - h - 4.0).max(8.0))
            }
            (None, Some(r)) => (r.x, r.y + r.h + 4.0),
            (None, None) => (self.size.0 - w - 12.0, look::TOP_BAR_H),
        };
        at.0 = at.0.min(self.size.0 - w - 8.0).max(8.0);
        match &kind {
            MenuKind::Grid => {
                for p in self.store.projects.clone() {
                    self.call(
                        Call::Worktrees(p.path.clone()),
                        "git:listWorktrees",
                        json!(p.path),
                    );
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
            sub: None,
        });
        ui.open_menu(menus::MENU, src.id.unwrap_or(menus::MENU));
    }

    fn close_menu(&mut self, ui: &mut Ui) {
        self.st.menu = None;
        self.st.confirm_delete = None;
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
    }

    fn save_config(&mut self, config: Value, call: Call) {
        self.store.apply("config:changed", config.clone());
        self.saves.push_back((config, call));
        self.next_save();
    }

    fn next_save(&mut self) {
        if self.saving {
            return;
        }
        if let Some((config, call)) = self.saves.pop_front() {
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
                self.st.view = v;
                if v != MainView::Sessions {
                    self.st.focused = None;
                }
                if v == MainView::Workflows {
                    self.wf_refresh();
                }
            }
            Act::ToggleSidebar => self.st.sidebar = !self.st.sidebar,
            Act::Select(id) => {
                self.st.view = MainView::Sessions;
                self.st.selected = Some(id);
                self.st.panes.focus = None;
                ui.set_focus(None);
            }
            Act::Git(a) => self.run_git(ui, a, src),
            Act::Pane(a) => self.run_pane(ui, a, src),
            Act::Edit(a) => self.run_edit(ui, a),
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
            Act::Columns(n) => self.st.columns = n,
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
                    self.call(Call::Created, "terminal:create", params);
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
                self.call(Call::Created, "terminal:create", params);
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
            Act::Workflow(a) => self.run_wf(ui, a, src),
            Act::Set(a) => self.run_set(ui, a),
            Act::Rename(id) => {
                let Some(title) = self.store.session(&id).map(|s| s.title()) else {
                    return;
                };
                let field = rename_id(&id);
                ui.set_edit_text(field, &title);
                ui.set_focus(Some(field));
                self.st.renaming = Some(id);
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
            Act::Paste(id) => {
                if let (Some(g), Some(text)) = (&self.grid, ui.clipboard().get()) {
                    g.paste(&id, &text);
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
            Act::Run(a) => self.run_run(a),
            Act::Gate(op) => self.run_gate(ui, op),
            Act::OpenUrl(u) => tasks::open_url(&u),
            Act::CloseDialog if self.st.dialog == Some(Dialog::AddTask) => {
                self.close_task_dialog(ui);
            }
            Act::CloseDialog => {
                self.st.dialog = None;
                self.st.composer_open = false;
                self.st.wf.prompt = None;
                self.st.wf.file = None;
            }
            Act::Settings => {
                self.st.dialog = None;
                let cat = self.st.settings.category;
                self.open_settings(cat);
            }
            // The welcome guide is a later surface of the port.
            Act::Welcome => self.st.dialog = None,
            Act::Project(p) => {
                self.st.project = p;
                if self.st.view == MainView::Workflows {
                    self.st.view = MainView::Sessions;
                }
            }
            Act::AddProject => {
                ui.set_edit_text(PROJECT_PATH, "");
                ui.set_edit_text(PROJECT_NAME, "");
                ui.set_focus(Some(PROJECT_PATH));
                self.st.form = Default::default();
                self.st.dialog = Some(Dialog::Project(None));
            }
            Act::ToggleProject(name) => {
                if !self.st.collapsed.remove(&name) {
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
                self.st.form = crate::state::ProjectForm {
                    icon: p.icon.clone(),
                    icon_color: p.icon_color.clone(),
                    auto_name: String::new(),
                };
                self.st.dialog = Some(Dialog::Project(Some(name)));
            }
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
            Act::BrowseFolder => ui.set_focus(Some(PROJECT_PATH)),
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
                self.st.headless.retain(|h| h.id != id);
            }
            Act::HideHeadless(id) => self.st.headless.retain(|h| h.id != id),
            Act::ToggleHeadless(_) => {}
            Act::PickProject(name) => self.pick_project(name),
            Act::PickAgent(agent) => self.st.launch.agent = Some(agent),
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
        let mut project = old.map_or_else(
            || json!({ "preferredAgents": ["claude"] }),
            |i| list[i].clone(),
        );
        project["name"] = json!(name);
        project["path"] = json!(path);
        if let Some(icon) = &self.st.form.icon {
            project["icon"] = json!(icon);
        }
        if let Some(color) = &self.st.form.icon_color {
            project["iconColor"] = json!(color);
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
                            && matches!(act, Act::ToggleHeadless(_))
                            || m.kind == MenuKind::WfIcon
                                && matches!(
                                    act,
                                    Act::Workflow(
                                        crate::workflows::WfAct::Icon(_)
                                            | crate::workflows::WfAct::IconColor(_)
                                    )
                                )
                    });
                    if !keep && self.st.menu.is_some() {
                        self.close_menu(ui);
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
            UiEvent::Wheel { id, dy, x, y } => {
                let Some(g) = &self.grid else { return };
                let Some(s) = self
                    .store
                    .sessions
                    .iter()
                    .find(|s| term_id(&s.info.id) == id)
                else {
                    return;
                };
                let (Some(r), cell) = (ui.rect_of(id), ui.text.cell()) else {
                    return;
                };
                g.wheel(
                    &s.info.id,
                    dy,
                    ((x - r.x) / cell.w).floor(),
                    ((y - r.y) / cell.h).floor(),
                );
            }
            UiEvent::Select { group, index } if group == menus::MENU || group == menus::SUB => {
                self.choose(ui, group, index);
            }
            UiEvent::Dismiss(id) if id == menus::MENU => {
                self.st.menu = None;
                self.st.confirm_delete = None;
            }
            UiEvent::Submit(id) => self.submit(ui, id),
            UiEvent::Changed(id) if self.pane_changed(ui, id) => {}
            UiEvent::Resized { id, ratio } => {
                self.pane_resized(ui, id, ratio, false);
            }
            UiEvent::ResizeEnd { id, ratio } => {
                self.pane_resized(ui, id, ratio, true);
            }
            UiEvent::Changed(id) if id == PROJECT_PATH => {
                // The folder's name, until another name is typed.
                let name = ui.edit_text(PROJECT_NAME).to_owned();
                if name.is_empty() || name == self.st.form.auto_name {
                    let auto = basename(ui.edit_text(PROJECT_PATH)).to_owned();
                    ui.set_edit_text(PROJECT_NAME, &auto);
                    self.st.form.auto_name = auto;
                }
            }
            _ => {}
        }
    }

    fn submit(&mut self, ui: &mut Ui, id: Id) {
        if id == PROJECT_NAME || id == PROJECT_PATH {
            self.save_project(ui);
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
        ui.set_edit_text(id, "");
        if let Some(g) = &self.grid {
            g.paste(&sid, &line);
            g.text(&sid, "\r");
        }
    }

    /// A menu entry chosen: its submenu opened, or its act run.
    fn choose(&mut self, ui: &mut Ui, group: Id, index: usize) {
        let Some(menu) = self.st.menu.clone() else {
            return;
        };
        let items = menus::items(&self.screen(ui), &menu.kind);
        let Some(item) = menus::chosen(items, menu.sub, group, index) else {
            return;
        };
        if group == menus::MENU && !item.sub.is_empty() {
            if let Some(m) = &mut self.st.menu {
                m.sub = Some(index);
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
                | Act::Task(crate::tasks::TaskAct::AskDelete(_))
        );
        if stays {
            ui.reopen_menu();
        } else {
            self.close_menu(ui);
        }
        self.run(ui, act, Src::default());
    }

    /// Input before the UI sees it: the global shortcuts, and Escape
    /// closing what is open. True when it was used.
    pub fn intercept(&mut self, ui: &mut Ui, input: &Input) -> bool {
        if self.settings_key(ui, input) {
            return true;
        }
        if let Input::Key { code, mods: m, .. } = input {
            let with_mod = *m & MOD != 0;
            if !self.st.settings.open
                && (self.git_key(ui, code, with_mod)
                    || self.pane_key(ui, code, with_mod, *m & mods::SHIFT != 0))
            {
                return true;
            }
            if self.st.dialog == Some(Dialog::Discard) && code == "Escape" {
                self.run_edit(ui, crate::editor::EditAct::CancelDiscard);
                return true;
            }
        }
        if self.st.settings.open {
            return false;
        }
        if self.st.menu.is_none() && (self.task_key(ui, input) || self.wf_key(ui, input)) {
            return true;
        }
        let Input::Key { code, mods: m, .. } = input else {
            return false;
        };
        if let Some(s) = shortcut(code, *m) {
            self.run_shortcut(ui, s);
            return true;
        }
        // Tab in the composer picks the next project, not the next control.
        let bare = self.st.dialog.is_none() && self.st.menu.is_none() && ui.focus().is_none();
        if code == "Tab" && bare && self.composing() {
            self.compose(input.clone());
            return true;
        }
        if self.st.menu.is_some() || code != "Escape" || *m & (MOD | mods::ALT) != 0 {
            return false;
        }
        if self.st.dialog == Some(Dialog::AddTask) {
            self.close_task_dialog(ui);
        } else if self.st.dialog == Some(Dialog::Missed) {
        } else if self.st.dialog.is_some() {
            self.st.dialog = None;
            self.st.wf.prompt = None;
            self.st.wf.file = None;
            ui.set_focus(None);
        } else if self.st.renaming.is_some() {
            self.st.renaming = None;
            ui.set_focus(None);
        } else if self.st.confirm_close.is_some() {
            self.st.confirm_close = None;
        } else if self.st.composer_overlay(&self.store) {
            self.st.composer_open = false;
        } else {
            return false;
        }
        true
    }

    /// One key or IME event the UI did not use: the composer's or the
    /// terminal's.
    pub fn input(&mut self, input: Input) {
        if self.st.dialog.is_some() || self.st.menu.is_some() || self.st.settings.open {
            return;
        }
        if self.composing() {
            self.compose(input);
            return;
        }
        if self.st.view != MainView::Sessions {
            return;
        }
        let (Some(g), Some(id)) = (&self.grid, self.typing_target()) else {
            return;
        };
        match input {
            Input::Key { code, mods, text } => g.key(&id, &code, mods, text.as_deref()),
            Input::Preedit(s) => self.preedit = s,
            Input::Commit(s) => {
                self.preedit.clear();
                g.text(&id, &s);
            }
        }
    }

    /// Input as a window delivers it: shortcuts first, then the UI, then
    /// the composer or the terminal, and what the UI reported after.
    pub fn send(&mut self, ui: &mut Ui, input: Input) {
        if !self.intercept(ui, &input) && !ui.key(&input) {
            self.input(input);
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

    /// A key in the composer: typing, Tab through the projects, Enter to
    /// launch.
    fn compose(&mut self, input: Input) {
        match input {
            Input::Key {
                code,
                mods: m,
                text,
            } => match code.as_str() {
                "Enter" if m & mods::SHIFT == 0 => self.launch(),
                "Enter" => self.st.composer.text.push('\n'),
                "Backspace" => {
                    self.st.composer.text.pop();
                }
                "Tab" => {
                    let back = m & mods::SHIFT != 0;
                    let names: Vec<String> =
                        self.store.projects.iter().map(|p| p.name.clone()).collect();
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
                _ if m & (mods::CTRL | mods::SUPER) == 0 => {
                    if let Some(t) = text {
                        self.st
                            .composer
                            .text
                            .extend(t.chars().filter(|ch| !ch.is_control()));
                    }
                }
                _ => {}
            },
            Input::Preedit(s) => self.preedit = s,
            Input::Commit(s) => {
                self.preedit.clear();
                self.st.composer.text.push_str(&s);
            }
        }
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
        let prompt = self.st.composer.text.trim();
        if !prompt.is_empty() {
            params["initialPrompt"] = json!(prompt);
        }
        self.call(Call::Launch, "terminal:create", params);
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
        }
    }

    /// Builds, lays out and paints a frame into `ui.scene`, attaching and
    /// sizing each card's terminal to the box it got.
    pub fn frame(&mut self, ui: &mut Ui, size: (f32, f32)) -> TreeUpdate {
        ui.begin(Rgba::hex(look::SURFACE_BASE));
        let root = self.view(ui, size);
        let laid = ui.layout(root, size);
        self.paint(ui, &laid);
        laid.tree
    }

    /// The frame's element tree for a `size` window, after taking in what
    /// vornd sent.
    pub fn view(&mut self, ui: &mut Ui, size: (f32, f32)) -> El {
        self.pump();
        self.git_tick();
        self.panes_tick(ui);
        self.tasks_tick(ui);
        self.settings_tick(std::time::Instant::now());
        let focus = ui.focus();
        if focus != self.last_focus {
            if let Some(old) = self.last_focus {
                self.settings_blur(ui, old);
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
        if self.agents.is_empty() {
            for (name, svg) in look::AGENT_ICONS {
                let px = 48;
                if let Some(id) = crate::ui::rasterize_svg_rgba(svg, px, px)
                    .and_then(|rgba| ui.image(px, px, &rgba))
                {
                    self.agents.insert(name, id);
                }
            }
        }
        if self.logo.is_none() {
            // A logo that does not decode costs only the logo.
            self.logo = decode_png(look::LOGO_PNG)
                .ok()
                .and_then(|(w, h, rgba)| ui.image(w, h, &rgba));
        }
        // A hovered entry with a submenu opens it; another closes it.
        if let Some(menu) = self.st.menu.clone() {
            let items = menus::items(&self.screen(ui), &menu.kind);
            let hovered = items
                .iter()
                .enumerate()
                .find(|(i, _)| ui.hovered(item_id(menus::MENU, *i)));
            if let Some((i, item)) = hovered {
                let sub = (!item.sub.is_empty()).then_some(i);
                if let Some(m) = &mut self.st.menu {
                    m.sub = sub;
                }
            }
        }
        if self.st.menu.is_some() && !ui.menu_open(menus::MENU) {
            self.st.menu = None;
        }
        if let Some(id) = self.st.wf.open_next.take() {
            self.run_wf(ui, crate::workflows::WfAct::Edit(id), Src::default());
        }
        self.acts.clear();
        let root = self.screen(ui).build();
        self.seed_texts(ui);
        ui.custom_a11y.clear();
        for (i, s) in self.store.sessions.iter().enumerate() {
            let id = crate::screen::CARD_ID_BASE + i as u64;
            ui.custom_a11y
                .insert(id, (Role::Terminal, s.title(), String::new()));
        }
        root
    }

    /// Paints each card's terminal into the box it got, attaching and
    /// sizing the terminal to it.
    pub fn paint(&mut self, ui: &mut Ui, laid: &Laid) {
        let cell = ui.text.cell();
        let current = self.typing_target();
        for (id, r) in &laid.customs {
            let Some(session) = card_index(*id).and_then(|i| self.store.sessions.get(i)) else {
                continue;
            };
            let sid = session.info.id.as_str();
            if let Some(g) = &self.grid {
                if !session.ended() {
                    // Whole cells only; f32 to u16 saturates.
                    let cols = (r.w / cell.w).floor() as u16;
                    let rows = (r.h / cell.h).floor() as u16;
                    g.attach(sid, cols, rows);
                    if !self.st.locked.contains(sid) {
                        g.resize(sid, cols.max(2), rows.max(1));
                    }
                }
            }
            if let Some(v) = self.views.get(sid) {
                let focused = current.as_deref() == Some(sid)
                    && self.st.renaming.is_none()
                    && self.st.dialog.is_none()
                    && ui.focus().is_none();
                let pre = if focused { self.preedit.as_str() } else { "" };
                // A screen not yet resized to a narrower box stays inside it.
                ui.scene.push_clip(*r);
                paint::pane(ui, v, *r, focused, pre);
                ui.scene.pop_clip();
            }
        }
    }
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
            shortcut("KeyS", MOD),
            Some(Shortcut::View(MainView::Sessions))
        );
        assert_eq!(shortcut("KeyT", MOD), Some(Shortcut::View(MainView::Tasks)));
        assert_eq!(
            shortcut("KeyW", MOD | mods::SHIFT),
            Some(Shortcut::View(MainView::Workflows))
        );
        assert_eq!(shortcut("Digit3", MOD), Some(Shortcut::Jump(2)));
        assert_eq!(shortcut("Digit0", MOD), None);
        assert_eq!(shortcut("BracketRight", MOD), Some(Shortcut::Cycle(true)));
        assert_eq!(shortcut("Backquote", mods::CTRL), Some(Shortcut::NewShell));
        assert_eq!(shortcut("Slash", MOD), Some(Shortcut::Shortcuts));
        assert_eq!(shortcut("KeyJ", MOD), Some(Shortcut::ViewOptions));
        assert_eq!(shortcut("KeyB", MOD), Some(Shortcut::Sidebar));
        assert_eq!(shortcut("F2", 0), Some(Shortcut::Rename));
        assert_eq!(
            shortcut("Digit3", mods::ALT),
            Some(Shortcut::Filter(Filter::Waiting))
        );
        assert_eq!(shortcut("Digit6", mods::ALT), None);
        // Unmodified and extra-modified keys belong to the terminal.
        assert_eq!(shortcut("KeyS", 0), None);
        assert_eq!(shortcut("KeyS", MOD | mods::ALT), None);
        assert_eq!(shortcut("Escape", 0), None);
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
