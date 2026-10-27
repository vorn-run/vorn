//! The pane column's controls: opening and closing panes, the terminals
//! panel, the dividers, and the Files extension's pane and cards, whose
//! codes the client edits and tells the extension about.

use std::collections::HashSet;
use web_time::Instant;

use serde_json::{json, Value};
use vorn_protocol::TerminalSession;
use vorn_tree::{CodeState, Event, EventKind, Kind, Node};

use super::git::cwd_of;
use super::{App, Call, Src};
use crate::client::rpc::{decode, RpcError};
use crate::code::{self, CodeAct, HIGHLIGHT_SETTLE_MS, LINE_HEIGHT};
use crate::points::{kind, scoped};
use crate::screen::code::{editor_id, find_id, scroller_id};
use crate::screen::side::PaneDivider;
use crate::side::{SideAct, SideKind};
use crate::state::Dialog;
use crate::ui::{Id, Ui};

/// Files, which keeps the Files pane and the cards files pop out into.
pub const FILES: &str = "vorn.files";
/// Its collection of the sessions whose pane is open.
const OPEN: &str = "open";

/// A pane call whose answer changes the screen.
pub(super) enum SideCall {
    /// A shell for `owner`'s panel.
    Shell(String),
    /// What the Files extension answered opening or closing `owner`'s pane.
    Toggled(String),
    /// The desktop's own Files state handed over, with its drafts file and
    /// what it held: only the drafts Files could not take stay in it.
    Imported(std::path::PathBuf, Option<Value>),
}

/// Every code node in `tree`, depth first, with its node id.
fn codes(tree: &Node) -> Vec<(&str, &vorn_tree::Code)> {
    let mut out = Vec::new();
    let mut stack = vec![tree];
    while let Some(n) = stack.pop() {
        if let Kind::Code(c) = &n.kind {
            out.push((n.id.as_deref().unwrap_or_default(), &**c));
        }
        stack.extend(&n.children);
    }
    out
}

/// The code node `node` of `tree`.
pub(crate) fn code_in<'a>(tree: &'a Node, node: &str) -> Option<&'a vorn_tree::Code> {
    codes(tree)
        .into_iter()
        .find(|(id, _)| *id == node)
        .map(|(_, c)| c)
}

impl App {
    /// The working directory of the session that owns `owner`'s panes.
    pub(super) fn owner_cwd(&self, owner: &str) -> Option<String> {
        self.store.session(owner).map(|s| cwd_of(&s.info))
    }

    /// The Files pane's tree key for `owner`, when Files is on.
    pub(crate) fn files_key(&self, owner: &str) -> Option<String> {
        let p = self.st.points.find(FILES, kind::PANES, "files")?;
        Some(scoped(&p.key(), owner))
    }

    /// The trees the Files extension draws now: each open pane's, and each card's.
    pub(super) fn files_trees(&self) -> Vec<String> {
        let pts = &self.st.points;
        let mut out: Vec<String> = self
            .st
            .side
            .files
            .iter()
            .filter_map(|o| self.files_key(o))
            .collect();
        if let Some(p) = pts.find(FILES, kind::CARDS, "file") {
            let key = p.key();
            out.extend(
                pts.listed_rows(p, "")
                    .into_iter()
                    .map(|r| scoped(&key, &r.id)),
            );
        }
        out
    }

    /// Before a frame: the panes Files lists, its codes kept in step with
    /// the edits they hold, and what is left of the dead sessions' panes.
    pub(super) fn panes_tick(&mut self, ui: &mut Ui) {
        let live: HashSet<&str> = self
            .store
            .sessions
            .iter()
            .map(|s| s.info.id.as_str())
            .collect();
        if self.st.side.prune(&live) {
            self.st.side.save();
        }
        let open = (FILES.to_owned(), OPEN.to_owned());
        if !self.st.points.wanted.contains(&open) {
            self.st.points.wanted.insert(open);
        }
        let files_on = self.st.points.find(FILES, kind::PANES, "files").is_some();
        self.st.side.files = if files_on {
            self.st
                .points
                .rows_of(FILES)
                .get(OPEN)
                .into_iter()
                .flatten()
                .map(|r| r.id.clone())
                .filter(|id| live.contains(id.as_str()))
                .collect()
        } else {
            HashSet::new()
        };
        // Panes and cards of sessions that are gone go with them, once the store has read them.
        let gone = files_on
            && self.store.sessions_known
            && self.st.points.rows_of(FILES).values().flatten().any(|r| {
                let owner = r.str("session").unwrap_or(&r.id);
                !live.contains(owner)
            });
        if gone && !self.pruning {
            self.pruning = true;
            let ids: Vec<&str> = live.iter().copied().collect();
            self.call(
                Call::Ignore,
                "extension:call",
                json!({ "id": FILES, "action": "prune", "args": { "live": ids } }),
            );
        } else if !gone {
            self.pruning = false;
        }
        if !self.imported.0 && files_on {
            self.imported.0 = true;
            self.import_files();
        }
        if !self.imported.1 && self.shell_key("").is_some() {
            self.imported.1 = true;
            self.import_history();
        }
        for owner in self.st.side.files.clone() {
            let (Some(key), Some(r)) = (
                self.files_key(&owner),
                ui.rect_of(crate::screen::side::files_body_id(&owner)),
            ) else {
                continue;
            };
            self.st.points.widths.insert(key, r.w);
        }
        self.browser_tick();
        self.video_tick();
        self.codes_tick(ui);
    }

    /// Each code on screen: a new `rev` taken into its edit, the edit told
    /// as it starts or stops differing and once it settles, its colours.
    fn codes_tick(&mut self, ui: &mut Ui) {
        let now = Instant::now();
        let trees = self.files_trees();
        let mut seen = HashSet::new();
        let mut tell = Vec::new();
        for key in &trees {
            let Some(tree) = self.st.points.tree(key) else {
                continue;
            };
            for (node, c) in codes(tree) {
                let inst = code::instance(key, node);
                seen.insert(inst.clone());
                let v = self.st.codes.views.entry(inst.clone()).or_default();
                let id = editor_id(&inst);
                if v.rev != Some(c.rev) {
                    v.rev = Some(c.rev);
                    v.text = c.draft.clone().unwrap_or_else(|| c.text.clone());
                    v.told = Some((v.text.clone(), c.draft.is_some()));
                    v.typed = None;
                }
                if !ui.focused(id) && ui.edit_text(id) != v.text {
                    ui.set_edit_text(id, &v.text);
                }
                let editing = c.state == CodeState::Edit;
                let text = if editing {
                    v.text.as_str()
                } else {
                    c.text.as_str()
                };
                if editing {
                    if let Some(t) = code::to_tell(v, text, &c.text, now) {
                        v.told = Some((t.clone(), t != c.text));
                        tell.push((key.clone(), node.to_owned(), t));
                    }
                }
                let settled = v
                    .typed
                    .is_none_or(|t| now.duration_since(t).as_millis() >= HIGHLIGHT_SETTLE_MS);
                let stale = v.coloured.as_ref().is_none_or(|(t, _)| t != text);
                if stale && settled && c.state != CodeState::Loading {
                    let name = crate::state::basename(&c.path);
                    let spans = match crate::highlight::lang_for(name) {
                        Some(l) if text.split('\n').count() <= code::MAX_HIGHLIGHT_LINES => {
                            crate::highlight::highlight(text, l)
                        }
                        _ => Vec::new(),
                    };
                    v.coloured = Some((text.to_owned(), spans));
                }
                if v.typed.is_some_and(|t| {
                    now.duration_since(t).as_millis()
                        < code::DRAFT_SETTLE_MS.max(HIGHLIGHT_SETTLE_MS)
                }) {
                    ui.wake_at(now + std::time::Duration::from_millis(50));
                }
            }
        }
        self.st.codes.views.retain(|k, _| seen.contains(k));
        for (key, node, text) in tell {
            self.tell(&key, &Event::new(node, EventKind::Change, text));
        }
    }

    /// Tells tree `key`'s extension each edit its codes hold back while
    /// typing settles, so a tab switch, close or pop never overtakes it.
    pub(super) fn flush_codes(&mut self, key: &str) {
        let Some(tree) = self.st.points.tree(key) else {
            return;
        };
        let mut tell = Vec::new();
        for (node, c) in codes(tree) {
            let Some(v) = self.st.codes.views.get_mut(&code::instance(key, node)) else {
                continue;
            };
            if c.state != CodeState::Edit || v.rev != Some(c.rev) {
                continue;
            }
            if let Some(t) = code::unsaid(v, &v.text, &c.text) {
                v.told = Some((t.clone(), t != c.text));
                tell.push(Event::new(node, EventKind::Change, t));
            }
        }
        for e in tell {
            self.send_told(key, &e);
        }
    }

    pub(super) fn side_answered(&mut self, call: SideCall, answer: Result<Value, RpcError>) {
        match call {
            SideCall::Shell(owner) => match answer.and_then(decode::<TerminalSession>) {
                Ok(info) => {
                    let id = info.id.clone();
                    self.store.upsert(info);
                    self.st.side.open_panel(&owner, &id);
                    self.st.side.focus = Some(id);
                    self.st.side.save();
                }
                Err(e) => self
                    .st
                    .toast(format!("Failed to start terminal: {e}"), true),
            },
            SideCall::Imported(file, drafts) => {
                let Ok(v) = answer else {
                    return;
                };
                let skipped: HashSet<&str> = v
                    .get("skipped")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let mut left = drafts.unwrap_or_default();
                if let Some(m) = left.as_object_mut() {
                    m.retain(|k, _| skipped.contains(k.as_str()));
                }
                if left.as_object().is_none_or(|m| m.is_empty()) {
                    #[cfg(not(target_arch = "wasm32"))]
                    let _ = std::fs::remove_file(&file);
                } else {
                    let _ = crate::keep::write(&file, &left.to_string());
                }
                self.st.side.saved.files = None;
                self.st.side.saved.cards = None;
                self.st.side.save();
            }
            SideCall::Toggled(owner) => match answer {
                Ok(v) => {
                    let a = vorn_tree::Answer::read(&v);
                    if let (Some(ask), Some(key)) = (a.ask, self.files_key(&owner)) {
                        self.ask(key, ask);
                    }
                }
                Err(e) => self.st.toast(e.to_string(), true),
            },
        }
    }

    /// Asks `ask` with the app's own alert; its OK tells tree `key` the event it holds.
    pub(super) fn ask(&mut self, key: String, ask: vorn_tree::Ask) {
        self.st.ask = Some((key, ask));
        self.st.dialog = Some(Dialog::Ask);
    }

    /// The alert's answer: OK tells the tree, Cancel only closes.
    pub(super) fn answer_ask(&mut self, ok: bool) {
        self.st.dialog = None;
        if let Some((key, ask)) = self.st.ask.take().filter(|_| ok) {
            self.tell(&key, &ask.event);
        }
    }

    fn panel_shell(&mut self, owner: &str) {
        let Some(cwd) = self.owner_cwd(owner) else {
            return;
        };
        self.call(
            Call::Side(SideCall::Shell(owner.to_owned())),
            "shell:create",
            json!(cwd),
        );
    }

    /// Stops `ids` and lets go of their cards.
    fn kill_shells(&mut self, ids: Vec<String>) {
        for id in ids {
            self.call(Call::Ignore, "terminal:kill", json!(id));
            self.store.remove(&id);
            if let Some(g) = &self.grid {
                g.detach(&id);
            }
        }
    }

    /// Mod+Alt+O or T: the current card's folder or shells, when its More menu offers them.
    pub(super) fn side_key(&mut self, ui: &mut Ui, folder: bool) {
        let Some(s) = self.current().and_then(|id| self.store.session(&id)) else {
            return;
        };
        let id = s.info.id.clone();
        let act = if folder {
            let files = self.st.points.find(FILES, kind::PANES, "files");
            files.is_some().then_some(SideAct::ToggleFiles(id))
        } else {
            let has = self.st.side.saved.panels.contains_key(&id);
            (!s.is_shell() || has).then_some(SideAct::ToggleTerminals(id))
        };
        if let Some(act) = act {
            self.run_side(ui, act, Src::default());
        }
    }

    pub(super) fn run_side(&mut self, ui: &mut Ui, act: SideAct, _src: Src) {
        match act {
            SideAct::ToggleFiles(owner) => {
                self.call(
                    Call::Side(SideCall::Toggled(owner.clone())),
                    "extension:call",
                    json!({ "id": FILES, "action": "toggle", "args": { "session": owner } }),
                );
            }
            SideAct::ToggleTerminals(owner) => {
                if self.st.side.saved.panels.contains_key(&owner) {
                    let held = self.st.side.close_panel(&owner);
                    self.kill_shells(held);
                    self.st.side.save();
                } else {
                    self.panel_shell(&owner);
                }
            }
            SideAct::AddTerminal(owner) => self.panel_shell(&owner),
            SideAct::Maximize(pane) => {
                self.st.side.saved.maximized =
                    if self.st.side.saved.maximized.as_deref() == Some(pane.as_str()) {
                        None
                    } else {
                        Some(pane)
                    };
                self.st.side.save();
            }
            SideAct::Close(kind, owner) => match kind {
                SideKind::Files => {
                    if let Some(key) = self.files_key(&owner) {
                        self.tell(&key, &Event::new("pane", EventKind::Close, ""));
                    }
                }
                SideKind::Terminals => {
                    let held = self.st.side.close_panel(&owner);
                    self.kill_shells(held);
                    self.st.side.save();
                }
            },
            SideAct::PanelTab(owner, i) => {
                self.st.side.set_active_panel(&owner, i);
                self.st.side.focus = self
                    .st
                    .side
                    .saved
                    .panels
                    .get(&owner)
                    .and_then(|x| x.terminals.get(i))
                    .cloned();
                self.st.selected = Some(owner);
                self.st.side.save();
            }
            SideAct::Extract(_, id) => {
                self.st.side.release(&id);
                self.st.side.save();
                self.st.selected = Some(id);
            }
            SideAct::CloseTerminal(id) => {
                self.st.side.release(&id);
                self.st.side.save();
                self.kill_shells(vec![id]);
            }
            SideAct::FocusTerminal(id) => {
                if let Some(owner) = self.st.side.holder(&id).map(str::to_owned) {
                    self.st.side.focus = Some(id);
                    self.st.selected = Some(owner);
                    ui.set_focus(None);
                }
            }
            SideAct::SelectCard(card) => {
                self.st.selected = Some(card);
                self.st.side.focus = None;
            }
        }
    }

    /// The code `inst` draws from its tree.
    fn code_node(&self, inst: &str) -> Option<&vorn_tree::Code> {
        let (key, node) = code::parts(inst);
        code_in(self.st.points.tree(key)?, node)
    }

    /// Whether code `inst` holds an edit not on disk.
    pub(crate) fn code_dirty(&self, inst: &str) -> bool {
        self.code_node(inst).is_some_and(|c| {
            c.state == CodeState::Edit
                && self
                    .st
                    .codes
                    .views
                    .get(inst)
                    .is_some_and(|v| v.rev == Some(c.rev) && v.text != c.text)
        })
    }

    fn tell_code(&mut self, inst: &str, kind: EventKind, value: String) {
        let (key, node) = code::parts(inst);
        let (key, node) = (key.to_owned(), node.to_owned());
        self.tell(&key, &Event::new(node, kind, value));
    }

    pub(super) fn run_code(&mut self, ui: &mut Ui, act: CodeAct) {
        match act {
            CodeAct::ToggleFind(inst) => {
                let can = self
                    .code_node(&inst)
                    .is_some_and(|c| matches!(c.state, CodeState::Edit | CodeState::Read));
                let Some(v) = self.st.codes.views.get_mut(&inst).filter(|_| can) else {
                    return;
                };
                if v.find.take().is_none() {
                    v.find = Some(code::Find::default());
                    ui.set_edit_text(find_id(&inst), "");
                    ui.set_focus(Some(find_id(&inst)));
                }
            }
            CodeAct::FindStep(inst, d) => {
                let n = self.matches(&inst).len();
                if let Some(f) = self
                    .st
                    .codes
                    .views
                    .get_mut(&inst)
                    .and_then(|v| v.find.as_mut())
                    .filter(|_| n > 0)
                {
                    f.index = (f.index as i64 + i64::from(d)).rem_euclid(n as i64) as usize;
                }
                self.reveal_match(ui, &inst);
            }
            CodeAct::CloseFind(inst) => {
                if let Some(v) = self.st.codes.views.get_mut(&inst) {
                    v.find = None;
                }
                ui.set_focus(None);
            }
            CodeAct::Save(inst) => self.save_code(&inst),
            CodeAct::Tell(inst, what) => {
                self.tell_code(&inst, EventKind::Click, what.to_owned());
            }
        }
    }

    /// Writes code `inst`'s edit, through its extension.
    fn save_code(&mut self, inst: &str) {
        if !self.code_dirty(inst) {
            return;
        }
        let Some(v) = self.st.codes.views.get_mut(inst) else {
            return;
        };
        let text = v.text.clone();
        v.told = Some((text.clone(), false));
        self.tell_code(inst, EventKind::Submit, text);
    }

    /// The find hits in what code `inst` shows.
    pub(crate) fn matches(&self, inst: &str) -> Vec<code::Match> {
        let Some(f) = self.st.codes.views.get(inst).and_then(|v| v.find.as_ref()) else {
            return Vec::new();
        };
        let text = match self.code_node(inst) {
            Some(c) if c.state == CodeState::Edit => self
                .st
                .codes
                .views
                .get(inst)
                .map_or("", |v| v.text.as_str()),
            Some(c) => c.text.as_str(),
            None => return Vec::new(),
        };
        code::matches(text, &f.query)
    }

    /// Scrolls the current find match to the middle of the editor.
    fn reveal_match(&mut self, ui: &mut Ui, inst: &str) {
        let m = self.matches(inst);
        let Some(f) = self.st.codes.views.get(inst).and_then(|v| v.find.as_ref()) else {
            return;
        };
        let Some(hit) = m.get(f.index % m.len().max(1)) else {
            return;
        };
        let scroller = scroller_id(inst);
        let h = ui.rect_of(scroller).map_or(0.0, |r| r.h);
        let y = hit.line as f32 * LINE_HEIGHT - h / 2.0;
        ui.set_scroll_offset(scroller, y.max(0.0));
    }

    /// Keeps the caret's line inside the editor's view as it moves.
    pub(super) fn follow_caret(&mut self, ui: &mut Ui, inst: &str) {
        let id = editor_id(inst);
        let Some(caret) = ui.edit(id).map(|e| e.caret()) else {
            return;
        };
        let text = ui.edit_text(id);
        let line = text[..caret.min(text.len())].matches('\n').count() as f32;
        let scroller = scroller_id(inst);
        let Some(r) = ui.rect_of(scroller) else {
            return;
        };
        let top = line * LINE_HEIGHT;
        let off = ui.scroll_offset(scroller);
        if top < off {
            ui.set_scroll_offset(scroller, top);
        } else if top + LINE_HEIGHT + 8.0 > off + r.h {
            ui.set_scroll_offset(scroller, top + LINE_HEIGHT + 8.0 - r.h);
        }
    }

    /// A code's instance from its editor or find field, when `id` is one.
    pub(super) fn code_of(&self, id: Id) -> Option<(String, bool)> {
        self.st.codes.views.keys().find_map(|k| {
            if editor_id(k) == id {
                Some((k.clone(), false))
            } else if find_id(k) == id {
                Some((k.clone(), true))
            } else {
                None
            }
        })
    }

    /// Typing in a code or its find bar.
    pub(super) fn pane_changed(&mut self, ui: &mut Ui, id: Id) -> bool {
        let Some((inst, find)) = self.code_of(id) else {
            return false;
        };
        let text = ui.edit_text(id).to_owned();
        let Some(v) = self.st.codes.views.get_mut(&inst) else {
            return true;
        };
        if find {
            if let Some(f) = v.find.as_mut() {
                f.query = text;
                f.index = 0;
            }
            self.reveal_match(ui, &inst);
        } else {
            v.text = text;
            v.typed = Some(Instant::now());
            self.follow_caret(ui, &inst);
        }
        true
    }

    /// A code's keys before the UI's: Mod+S saves, the find bar's Enter
    /// steps and Escape closes it.
    pub(super) fn pane_key(
        &mut self,
        ui: &mut Ui,
        code: &str,
        with_mod: bool,
        shift: bool,
    ) -> bool {
        let Some(focus) = ui.focus() else {
            return false;
        };
        // A tree's live filter: Escape clears it.
        if let Some(crate::act::Act::Point(a @ crate::points::PointAct::TreeLive { .. })) =
            self.acts.text_of(focus)
        {
            if code != "Escape" || ui.edit_text(focus).is_empty() {
                return false;
            }
            ui.set_edit_text(focus, "");
            self.run_point(ui, a);
            return true;
        }
        let Some((inst, find)) = self.code_of(focus) else {
            return false;
        };
        if code == "KeyS" && with_mod {
            self.save_code(&inst);
            return true;
        }
        if find && code == "Enter" {
            self.run_code(ui, CodeAct::FindStep(inst, if shift { -1 } else { 1 }));
            return true;
        }
        if find && code == "Escape" {
            self.run_code(ui, CodeAct::CloseFind(inst));
            return true;
        }
        false
    }

    /// A divider moved: the card's split, a column's weights, a tree's split,
    /// or the Changes panel's width. Saved when it ends.
    pub(super) fn pane_resized(&mut self, ui: &Ui, id: Id, ratio: f32, end: bool) -> bool {
        if id == crate::screen::sidebar::SIDEBAR_HANDLE {
            // The handle sits on the sidebar's right edge, so the sidebar's
            // left is that edge less its width, taken as the drag starts.
            let now = self.st.sidebar_width.unwrap_or(crate::look::SIDEBAR_W);
            let left = self
                .sidebar_left
                .or_else(|| ui.rect_of(id).map(|r| r.x + r.w - now));
            if let (Some(left), Some((x, _))) = (left, ui.pointer()) {
                let w = (x - left).clamp(crate::look::SIDEBAR_MIN, crate::look::SIDEBAR_MAX);
                self.st.sidebar_width = Some(w.round());
            }
            self.sidebar_left = if end { None } else { left };
            return true;
        }
        if let Some((key, node)) = crate::screen::points::split_of(&self.st.points, id) {
            let ratio = ratio.clamp(vorn_tree::MIN_SPLIT, vorn_tree::MAX_SPLIT);
            if end {
                self.st.split_drag = None;
                self.tell(
                    &key,
                    &Event::new(node, EventKind::Resize, ratio.to_string()),
                );
            } else {
                self.st.split_drag = Some((key, node, ratio));
            }
            return true;
        }
        let Some(which) = crate::screen::side::divider_of(&self.st, id) else {
            return false;
        };
        let p = &mut self.st.side;
        match which {
            PaneDivider::Card(owner) => {
                if end {
                    p.drag_card = None;
                    p.set_terminal_ratio(&owner, ratio);
                    p.save();
                } else {
                    p.drag_card = Some((owner, ratio));
                }
            }
            PaneDivider::Column(owner, i) => {
                let w = crate::side::resize_weights(&p.weights(&owner), i, ratio);
                if end {
                    p.drag_panes = None;
                    p.set_weights(&owner, w);
                    p.save();
                } else {
                    p.drag_panes = Some((owner, w));
                }
            }
            PaneDivider::Changes => {
                // The handle sits inside the panel; the width is the row's
                // right edge less the pointer.
                let (Some(row), Some((x, _))) = (
                    ui.rect_of(crate::screen::git::changes_row_id()),
                    ui.pointer(),
                ) else {
                    return true;
                };
                self.st.git.maximized = false;
                self.st.git.width =
                    Some((row.x + row.w - x).clamp(crate::git::PANEL_MIN, crate::git::PANEL_MAX));
            }
        }
        true
    }

    /// Hands what the desktop kept of the Files pane before (its panes,
    /// cards and drafts) to the Files extension once, then lets it go.
    pub(super) fn import_files(&mut self) {
        let Some(dir) = self.data_dir.clone() else {
            return;
        };
        let drafts_file = dir.join("desktop").join("drafts.json");
        let read = |p: &std::path::Path| {
            crate::keep::read(p).and_then(|s| serde_json::from_str::<Value>(&s).ok())
        };
        let drafts = read(&drafts_file);
        let saved = &self.st.side.saved;
        if drafts.is_none() && saved.files.is_none() && saved.cards.is_none() {
            return;
        }
        let view = json!({ "files": saved.files, "cards": saved.cards });
        let args = json!({ "drafts": drafts, "view": view });
        self.call(
            Call::Side(SideCall::Imported(drafts_file, drafts)),
            "extension:call",
            json!({ "id": FILES, "action": "import", "args": args }),
        );
    }
}
