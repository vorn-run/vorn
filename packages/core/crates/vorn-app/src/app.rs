//! The app: vornd's models and the grid folded into one state, the keys
//! routed the way the renderer's shortcuts route them, and each frame built
//! from that state. It knows nothing of windows, so the same `App` draws
//! into a window or offscreen.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::client::rpc::{decode, Wake};
use crate::client::{Endpoint, Event, Pending, Rpc, RpcError, Store};
use crate::grid::Grid;
use crate::look;
use crate::paint;
use crate::screen::{card_index, Composer, MainView, Screen};
use crate::ui::{decode_png, mods, Input, Rgba, Role, TreeUpdate, Ui};
use crate::view::PaneView;
use vorn_protocol::TerminalSession;

/// The modifier the renderer's shortcuts use (`modKey`).
const MOD: u16 = if cfg!(target_os = "macos") {
    mods::SUPER
} else {
    mods::CTRL
};

/// What a key does before it reaches a terminal or the composer.
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
    /// Mod+B: the sidebar, which this app does not have yet.
    Sidebar,
}

/// The renderer's global shortcuts (`useKeyboardShortcuts`). Escape is not
/// one: a terminal takes it, as xterm does.
pub fn shortcut(code: &str, m: u16) -> Option<Shortcut> {
    let only = |want: u16| m & (mods::CTRL | mods::SUPER | mods::ALT | mods::SHIFT) == want;
    if code == "Backquote" && only(mods::CTRL) {
        return Some(Shortcut::NewShell);
    }
    if only(MOD | mods::SHIFT) && code == "KeyW" {
        return Some(Shortcut::View(MainView::Workflows));
    }
    if !only(MOD) {
        return None;
    }
    Some(match code {
        "KeyS" => Shortcut::View(MainView::Sessions),
        "KeyT" => Shortcut::View(MainView::Tasks),
        "KeyN" => Shortcut::NewSession,
        "KeyB" => Shortcut::Sidebar,
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

/// A call whose answer changes the screen.
enum Call {
    /// The composer's `terminal:create`.
    Launch,
    /// Ctrl+`'s `shell:create`.
    Shell,
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
    pub selected: Option<String>,
    pub view: MainView,
    pub composer: Composer,
    composer_open: bool,
    preedit: String,
    calls: Vec<(Call, Pending)>,
    logo: Option<u32>,
    /// vornd hung up.
    pub closed: bool,
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
        Ok(App {
            rpc,
            events,
            store,
            grid,
            grid_error,
            views: HashMap::new(),
            selected: None,
            view: MainView::default(),
            composer: Composer::default(),
            composer_open: false,
            preedit: String::new(),
            calls: Vec::new(),
            logo: None,
            closed: false,
        })
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
        self.view == MainView::Sessions && (self.store.sessions.is_empty() || self.composer_open)
    }

    /// Folds in what arrived since the last frame: notifications, answers
    /// to calls, and terminals that changed.
    pub fn pump(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                // An ended terminal keeps its last screen, so every change is just state.
                Event::Notification { method, params } => {
                    self.store.apply(&method, params);
                }
                Event::Closed => self.closed = true,
            }
        }
        let calls = std::mem::take(&mut self.calls);
        for (kind, pending) in calls {
            match pending.poll() {
                None => self.calls.push((kind, pending)),
                Some(answer) => self.answered(&kind, answer),
            }
        }
        if let Some(g) = &self.grid {
            for id in g.take_dirty() {
                if let Some(v) = g.view(&id) {
                    self.views.insert(id, v);
                }
            }
            // A session that is gone needs no pane.
            for id in g.attached() {
                if self.store.session(&id).is_none() {
                    g.detach(&id);
                    self.views.remove(&id);
                }
            }
        }
    }

    fn answered(&mut self, kind: &Call, answer: Result<Value, RpcError>) {
        if matches!(kind, Call::Launch) {
            self.composer.launching = false;
        }
        match answer.and_then(decode::<TerminalSession>) {
            Ok(info) => {
                self.selected = Some(info.id.clone());
                self.store.upsert(info);
                if matches!(kind, Call::Launch) {
                    self.composer = Composer {
                        project: self.composer.project.take(),
                        ..Composer::default()
                    };
                    self.composer_open = false;
                }
            }
            Err(e) => match kind {
                Call::Launch => self.composer.error = Some(e.to_string()),
                Call::Shell => self.grid_error = Some(e.to_string()),
            },
        }
    }

    fn selected_index(&self) -> Option<usize> {
        let id = self.selected.as_deref()?;
        self.store.sessions.iter().position(|s| s.info.id == id)
    }

    fn select(&mut self, i: Option<usize>) {
        if let Some(s) = i.and_then(|i| self.store.sessions.get(i)) {
            self.selected = Some(s.info.id.clone());
            self.view = MainView::Sessions;
        }
    }

    /// The project a new shell starts in: the selected card's, else the
    /// composer's.
    fn active_project_path(&self) -> Option<String> {
        if let Some(i) = self.selected_index() {
            let path = &self.store.sessions[i].info.project_path;
            if !path.is_empty() {
                return Some(path.clone());
            }
        }
        let name = self.composer.project.as_deref()?;
        let project = self.store.projects.iter().find(|p| p.name == name)?;
        Some(project.path.clone())
    }

    fn run(&mut self, s: Shortcut) {
        match s {
            Shortcut::View(v) => self.view = v,
            Shortcut::NewShell => {
                let cwd = self
                    .active_project_path()
                    .map_or(Value::Null, Value::String);
                let call = self.rpc.request("shell:create", cwd);
                self.calls.push((Call::Shell, call));
            }
            Shortcut::NewSession => {
                self.view = MainView::Sessions;
                self.composer_open = true;
            }
            Shortcut::Cycle(forward) => {
                let next = cycle(self.selected_index(), self.store.sessions.len(), forward);
                self.select(next);
            }
            Shortcut::Jump(i) => self.select(Some(i)),
            Shortcut::Sidebar => {}
        }
    }

    /// One key or IME event, from a window or a test.
    pub fn input(&mut self, input: Input) {
        if let Input::Key { code, mods: m, .. } = &input {
            if let Some(s) = shortcut(code, *m) {
                self.run(s);
                return;
            }
        }
        if self.composing() {
            self.compose(input);
            return;
        }
        if self.view != MainView::Sessions {
            return;
        }
        let (Some(g), Some(id)) = (&self.grid, &self.selected) else {
            return;
        };
        match input {
            Input::Key { code, mods, text } => g.key(id, &code, mods, text.as_deref()),
            Input::Preedit(s) => self.preedit = s,
            Input::Commit(s) => {
                self.preedit.clear();
                g.text(id, &s);
            }
        }
    }

    /// A key in the composer: typing, Tab through the projects, Enter to
    /// launch, Escape to put it away when cards are under it.
    fn compose(&mut self, input: Input) {
        let c = &mut self.composer;
        match input {
            Input::Key {
                code,
                mods: m,
                text,
            } => match code.as_str() {
                "Enter" if m & mods::SHIFT == 0 => self.launch(),
                "Enter" => c.text.push('\n'),
                "Backspace" => {
                    c.text.pop();
                }
                "Escape" => self.composer_open = false,
                "Tab" => {
                    let back = m & mods::SHIFT != 0;
                    let names: Vec<&str> = self
                        .store
                        .projects
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect();
                    let at = c
                        .project
                        .as_deref()
                        .and_then(|p| names.iter().position(|n| *n == p));
                    c.project = cycle(at, names.len(), !back).map(|i| names[i].to_owned());
                    c.error = None;
                }
                _ if m & (mods::CTRL | mods::SUPER) == 0 => {
                    if let Some(t) = text {
                        c.text.extend(t.chars().filter(|ch| !ch.is_control()));
                    }
                }
                _ => {}
            },
            Input::Preedit(s) => self.preedit = s,
            Input::Commit(s) => {
                self.preedit.clear();
                c.text.push_str(&s);
            }
        }
    }

    /// PromptLauncher's launch: the default agent in the chosen project,
    /// with what was typed as its first prompt.
    fn launch(&mut self) {
        let c = &self.composer;
        if c.launching {
            return;
        }
        let Some(project) = c
            .project
            .as_deref()
            .and_then(|n| self.store.projects.iter().find(|p| p.name == n))
        else {
            return;
        };
        let prompt = c.text.trim();
        let mut params = json!({
            "agentType": self.store.default_agent(),
            "projectName": project.name,
            "projectPath": project.path,
        });
        if !prompt.is_empty() {
            params["initialPrompt"] = json!(prompt);
        }
        let call = self.rpc.request("terminal:create", params);
        self.calls.push((Call::Launch, call));
        self.composer.launching = true;
        self.composer.error = None;
    }

    /// Builds, lays out and paints a frame into `ui.scene`, attaching and
    /// sizing each card's terminal to the box it got.
    pub fn frame(&mut self, ui: &mut Ui, size: (f32, f32)) -> TreeUpdate {
        self.pump();
        if self.logo.is_none() {
            // A logo that does not decode costs only the logo.
            self.logo = decode_png(look::LOGO_PNG)
                .ok()
                .and_then(|(w, h, rgba)| ui.image(w, h, &rgba));
        }
        ui.begin(Rgba::hex(look::SURFACE_BASE));
        let root = Screen {
            store: &self.store,
            view: self.view,
            selected: self.selected.as_deref(),
            composer: &self.composer,
            composer_open: self.composer_open,
            logo: self.logo,
            size,
        }
        .build();
        ui.custom_a11y.clear();
        for (i, s) in self.store.sessions.iter().enumerate() {
            let id = crate::screen::CARD_ID_BASE + i as u64;
            ui.custom_a11y
                .insert(id, (Role::Terminal, s.title(), String::new()));
        }
        let laid = ui.layout(root, size);
        let cell = ui.text.cell();
        for (id, r) in &laid.customs {
            let Some(session) = card_index(*id).and_then(|i| self.store.sessions.get(i)) else {
                continue;
            };
            let sid = session.info.id.as_str();
            if let Some(g) = &self.grid {
                // Whole cells only; f32 to u16 saturates.
                let cols = (r.w / cell.w).floor() as u16;
                let rows = (r.h / cell.h).floor() as u16;
                g.attach(sid, cols, rows);
                g.resize(sid, cols.max(2), rows.max(1));
            }
            if let Some(v) = self.views.get(sid) {
                let focused = self.selected.as_deref() == Some(sid);
                let pre = if focused { self.preedit.as_str() } else { "" };
                paint::pane(ui, v, *r, focused, pre);
            }
        }
        laid.tree
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
}
