//! A shell's command blocks and its composer's history and completions, as
//! the Shell blocks extension answers them: the commands it lists, cut into
//! blocks from the grid, and the menu's keys.

use web_time::Instant;

use serde_json::{json, Value};

use super::{App, Call};
use crate::client::rpc::RpcError;
use crate::look;
use crate::paint;
use crate::points::{kind, scoped};
use crate::prompt::{self, IntentAct, Row, BLOCKS};
use crate::screen::{block_of, intent_id, log_id, term_id, SETTLE};
use crate::ui::{mods, Input, Rect, Ui};

/// A composer's question to Shell blocks.
#[derive(Debug)]
pub(super) enum IntentCall {
    /// Past commands for what was typed; `open` opens the menu on them (↑).
    History {
        session: String,
        text: String,
        open: bool,
    },
    /// Completions for what was typed.
    Complete { session: String, text: String },
}

/// The chords a running command takes from the composer: ^C, ^D, ^Z, ^\.
fn control(code: &str) -> Option<&'static str> {
    match code {
        "KeyC" => Some("\u{3}"),
        "KeyD" => Some("\u{4}"),
        "KeyZ" => Some("\u{1a}"),
        "Backslash" => Some("\u{1c}"),
        _ => None,
    }
}

impl App {
    /// Shell `id`'s commands tree, when Shell blocks is on.
    pub(crate) fn shell_key(&self, id: &str) -> Option<String> {
        let p = self.st.points.find(BLOCKS, kind::SHELL, "commands")?;
        Some(scoped(&p.key(), id))
    }

    /// The trees Shell blocks draws now: one per live shell that has shown a prompt mark.
    /// A shell that never reports its commands has none to show, so its extension is not woken.
    pub(super) fn shell_trees(&self) -> Vec<String> {
        self.store
            .sessions
            .iter()
            .filter(|s| s.is_shell() && !s.ended())
            .filter(|s| {
                self.st
                    .shells
                    .get(&s.info.id)
                    .is_some_and(|sh| !sh.lines.prompts.is_empty() || !sh.commands.is_empty())
            })
            .filter_map(|s| self.shell_key(&s.info.id))
            .collect()
    }

    /// A `terminal:command` mark: a command starting closes the composer's menu.
    pub(super) fn shell_notified(&mut self, method: &str, params: &Value) {
        if method != "terminal:command"
            || params.get("phase").and_then(Value::as_str) != Some("start")
        {
            return;
        }
        if let Some(m) = params
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| self.st.intent.menus.get_mut(id))
        {
            m.close();
        }
    }

    /// Cuts the shell's lines into blocks again, after its screen or its commands changed.
    pub(super) fn sync_shell(&mut self, id: &str) {
        let Some(g) = &self.grid else { return };
        if !self.store.session(id).is_some_and(|s| s.is_shell()) {
            return;
        }
        let Some(lines) = g.lines(id) else { return };
        let sh = self.st.shells.entry(id.to_owned()).or_default();
        sh.sync(lines, |from, to| {
            let mut v = g.lines_view(id, from, to)?;
            // Output reads in the log's grey, as the desktop's; colours a program chose stay.
            for r in &mut v.runs {
                if r.fg == v.fg {
                    r.fg = look::GRAY_300.hex();
                }
            }
            Some(v)
        });
    }

    /// Takes each shell's commands from its tree when a new one came.
    fn shells_listed(&mut self) {
        let now = Instant::now();
        let ids: Vec<String> = self
            .store
            .sessions
            .iter()
            .filter(|s| s.is_shell())
            .map(|s| s.info.id.clone())
            .collect();
        for id in ids {
            let Some(key) = self.shell_key(&id) else {
                self.st.shells.remove(&id);
                continue;
            };
            let gen = self.st.points.generation(&key);
            if self
                .st
                .shells
                .get(&id)
                .is_some_and(|sh| sh.generation == gen)
            {
                continue;
            }
            let commands = match self.st.points.tree(&key).map(|t| &t.kind) {
                Some(vorn_tree::Kind::Log(l)) => l.commands.clone(),
                _ => Vec::new(),
            };
            let sh = self.st.shells.entry(id.clone()).or_default();
            sh.generation = gen;
            sh.listed(commands, now);
            self.sync_shell(&id);
        }
    }

    /// Before a frame: the logs kept to their ends, the composers' menus
    /// asked for over what is typed, and the keyboard handed between a
    /// composer and the command it stood aside for.
    pub(super) fn shell_frame(&mut self, ui: &mut Ui) {
        self.fill_ghosts(ui);
        self.shells_listed();
        let now = ui.now();
        let mut asks = Vec::new();
        for s in &self.store.sessions {
            if !s.is_shell() {
                continue;
            }
            let sid = s.info.id.as_str();
            ui.follow_end(log_id(sid));
            let aside = self.st.shells.get(sid).is_some_and(|sh| {
                sh.lines.alt
                    || sh
                        .running()
                        .is_some_and(|r| now.saturating_duration_since(r.since) >= SETTLE)
            });
            let field = intent_id(sid);
            if aside && ui.focused(field) {
                ui.set_focus(None);
                self.st.intent.stood_aside.insert(sid.to_owned());
                self.st.selected = Some(sid.to_owned());
            } else if !aside && self.st.intent.stood_aside.remove(sid) && ui.focus().is_none() {
                ui.set_focus(Some(field));
            }
            let typed = ui.edit_text(field);
            let m = self.st.intent.menus.entry(sid.to_owned()).or_default();
            if m.text != typed {
                m.text = typed.to_owned();
                m.changed = Some(if std::mem::take(&mut m.filled) {
                    now - prompt::DEBOUNCE
                } else {
                    now
                });
                m.completed = false;
                m.highlight = None;
                m.take_ghost = false;
                m.open = !typed.is_empty();
                if typed.is_empty() {
                    m.completions.clear();
                    m.tab = None;
                    m.history.clear();
                    m.ghost = None;
                } else {
                    m.asking = Some(typed.to_owned());
                    asks.push(IntentCall::History {
                        session: sid.to_owned(),
                        text: typed.to_owned(),
                        open: false,
                    });
                }
            }
            if m.completed || m.text.is_empty() {
                continue;
            }
            let due = m.changed.map_or(now, |c| c + prompt::DEBOUNCE);
            if due > now {
                ui.wake_at(due);
                continue;
            }
            m.completed = true;
            asks.push(IntentCall::Complete {
                session: sid.to_owned(),
                text: m.text.clone(),
            });
        }
        for a in asks {
            self.ask_blocks(a);
        }
    }

    /// Asks Shell blocks for a composer's history or completions.
    fn ask_blocks(&mut self, a: IntentCall) {
        let Some(s) = (match &a {
            IntentCall::History { session, .. } | IntentCall::Complete { session, .. } => {
                self.store.session(session)
            }
        }) else {
            return;
        };
        let project = s.info.project_path.clone();
        let cwd = s.info.shell_cwd.clone();
        let repo = s
            .info
            .worktree_path
            .clone()
            .unwrap_or_else(|| project.clone());
        let (action, args) = match &a {
            IntentCall::History { text, .. } => {
                ("history", json!({ "text": text, "project": project }))
            }
            IntentCall::Complete { text, .. } => (
                "complete",
                json!({ "text": text, "cwd": cwd, "repo": repo }),
            ),
        };
        self.call(
            Call::Intent(a),
            "extension:call",
            json!({ "id": BLOCKS, "action": action, "args": args }),
        );
    }

    /// Shell blocks' answer: kept when it is still for what is typed.
    pub(super) fn intent_answered(&mut self, call: IntentCall, answer: Result<Value, RpcError>) {
        let Ok(v) = answer else { return };
        match call {
            IntentCall::History {
                session,
                text,
                open,
            } => {
                let Some(m) = self.st.intent.menus.get_mut(&session) else {
                    return;
                };
                // ↑ opens on the newest whatever is typed; a search only for what is still typed.
                if m.text != text && !open {
                    return;
                }
                let (rows, ghost) = prompt::read_history(&v);
                m.history = rows;
                if !open {
                    m.ghost = ghost;
                    if m.asking.as_deref() == Some(text.as_str()) {
                        m.asking = None;
                    }
                }
                if std::mem::take(&mut m.take_ghost) {
                    let g = prompt::ghost_over(m.ghost.as_deref(), &text).map(str::to_owned);
                    if let Some(g) = g {
                        self.fills.push((session.clone(), g));
                    }
                    return;
                }
                if open {
                    m.completions.clear();
                    m.completed = true;
                    m.open = true;
                    m.highlight = (!m.is_empty()).then_some(0);
                }
            }
            IntentCall::Complete { session, text } => {
                let Some(m) = self.st.intent.menus.get_mut(&session) else {
                    return;
                };
                if m.text != text {
                    return;
                }
                let (list, tab) = prompt::read_completions(&v);
                m.completions = list;
                m.tab = tab;
                if m.highlight.is_some_and(|h| h >= m.len()) {
                    m.highlight = None;
                }
            }
        }
    }

    /// The shell whose composer has the keyboard.
    fn intent_focus(&self, ui: &Ui) -> Option<String> {
        let f = ui.focus()?;
        self.store
            .sessions
            .iter()
            .find(|s| s.is_shell() && intent_id(&s.info.id) == f)
            .map(|s| s.info.id.clone())
    }

    /// The past command the ghost shows for what is typed now.
    pub(crate) fn ghost(&self, ui: &Ui, sid: &str) -> Option<String> {
        let typed = ui.edit_text(intent_id(sid));
        let m = self.st.intent.menus.get(sid)?;
        prompt::ghost_over(m.ghost.as_deref(), typed).map(str::to_owned)
    }

    /// → or Tab before Shell blocks answered what is typed: the ghost is taken when it comes.
    fn waits_for_ghost(&mut self, sid: &str, typed: &str) -> bool {
        let Some(m) = self.st.intent.menus.get_mut(sid) else {
            return false;
        };
        if m.asking.as_deref() != Some(typed) {
            return false;
        }
        m.take_ghost = true;
        true
    }

    /// Ghosts that came after → or Tab asked for them, put in.
    pub(super) fn fill_ghosts(&mut self, ui: &mut Ui) {
        for (sid, g) in std::mem::take(&mut self.fills) {
            if self.ghost(ui, &sid).as_deref() == Some(g.as_str()) {
                self.fill(ui, &sid, &g);
            }
        }
    }

    /// IntentBar's keys: the menu's, and a running command's chords.
    pub(super) fn intent_key(&mut self, ui: &mut Ui, input: &Input) -> bool {
        let Input::Key { code, mods: m, .. } = input else {
            return false;
        };
        let Some(sid) = self.intent_focus(ui) else {
            return false;
        };
        let field = intent_id(&sid);
        let ctrl = *m & mods::CTRL != 0;
        let plain = *m & (mods::CTRL | mods::ALT | mods::SUPER) == 0;
        let running = self
            .st
            .shells
            .get(&sid)
            .is_some_and(|sh| sh.running().is_some());
        if ctrl && running {
            // ^C with text selected still copies.
            let selected = ui.edit(field).is_some_and(|e| !e.selection().is_empty());
            if let Some(c) = control(code).filter(|_| !(code == "KeyC" && selected)) {
                if let Some(g) = &self.grid {
                    g.text(&sid, c);
                }
                return true;
            }
        }
        if !plain && code != "Tab" {
            return false;
        }
        let typed = ui.edit_text(field).to_owned();
        let ghost = self.ghost(ui, &sid);
        let menu = self.st.intent.menus.entry(sid.clone()).or_default();
        match code.as_str() {
            "ArrowUp" if !menu.shown() => {
                self.ask_blocks(IntentCall::History {
                    session: sid,
                    text: String::new(),
                    open: true,
                });
                true
            }
            "ArrowUp" => {
                menu.step(-1);
                true
            }
            "ArrowDown" if menu.shown() => {
                menu.step(1);
                true
            }
            "Escape" if menu.shown() => {
                menu.close();
                true
            }
            "Escape" => {
                ui.set_focus(None);
                true
            }
            "Enter" if menu.shown() && menu.highlight.is_some() => {
                let index = menu.highlight.unwrap_or(0);
                self.pick(ui, &sid, index);
                true
            }
            "Tab" => {
                if let Some(h) = menu.highlight.filter(|_| menu.shown()) {
                    self.pick_fill(ui, &sid, h);
                } else if let Some(t) = menu.tab.clone().filter(|_| !menu.completions.is_empty()) {
                    self.fill(ui, &sid, &t);
                } else if let Some(g) = ghost {
                    self.fill(ui, &sid, &g);
                } else {
                    self.waits_for_ghost(&sid, &typed);
                }
                true
            }
            "ArrowRight" => {
                let at_end = ui.edit(field).is_some_and(|e| e.caret() == typed.len());
                match ghost.filter(|_| at_end) {
                    Some(g) => {
                        self.fill(ui, &sid, &g);
                        true
                    }
                    None => at_end && self.waits_for_ghost(&sid, &typed),
                }
            }
            _ => false,
        }
    }

    /// Puts `text` in the composer, menu worked out over it at once.
    fn fill(&mut self, ui: &mut Ui, sid: &str, text: &str) {
        ui.set_edit_text(intent_id(sid), text);
        if let Some(m) = self.st.intent.menus.get_mut(sid) {
            m.filled = m.text != text;
        }
    }

    /// Row `index` taken with Tab: its text put in, nothing run.
    fn pick_fill(&mut self, ui: &mut Ui, sid: &str, index: usize) {
        let Some(m) = self.st.intent.menus.get(sid) else {
            return;
        };
        let text = match m.rows().get(index) {
            Some(Row::History(e)) => e.text.clone(),
            Some(Row::Completion(c)) => c.result.clone(),
            None => return,
        };
        self.fill(ui, sid, &text);
    }

    /// Row `index` chosen: a past command runs, a completion is put in.
    fn pick(&mut self, ui: &mut Ui, sid: &str, index: usize) {
        let history = self
            .st
            .intent
            .menus
            .get(sid)
            .and_then(|m| match m.rows().get(index) {
                Some(Row::History(e)) => Some(e.text.clone()),
                _ => None,
            });
        match history {
            Some(text) => self.run_line(ui, sid, &text),
            None => self.pick_fill(ui, sid, index),
        }
    }

    /// Runs `line` in the shell, and Shell blocks keeps it.
    pub(super) fn run_line(&mut self, ui: &mut Ui, sid: &str, line: &str) {
        let line = line.trim();
        ui.set_edit_text(intent_id(sid), "");
        if let Some(m) = self.st.intent.menus.get_mut(sid) {
            m.close();
            m.text.clear();
            m.completions.clear();
            m.history.clear();
            m.ghost = None;
        }
        if line.is_empty() {
            return;
        }
        let project = self.store.session(sid).map(|s| s.info.project_path.clone());
        self.call(
            Call::Ignore,
            "extension:call",
            json!({ "id": BLOCKS, "action": "record", "args": { "text": line, "project": project } }),
        );
        if let Some(g) = self.grid.clone() {
            self.return_to_live(sid);
            g.paste(sid, line);
            g.text(sid, "\r");
        }
    }

    pub(super) fn run_intent(&mut self, ui: &mut Ui, act: IntentAct) {
        match act {
            IntentAct::Copy { session, key } => {
                let text = self
                    .st
                    .shells
                    .get(&session)
                    .and_then(|sh| sh.blocks.iter().find(|b| b.key == key))
                    .map(crate::log::Block::text);
                if let Some(t) = text {
                    ui.clipboard().set(&t);
                    self.st.intent.copied = Some((key, ui.now()));
                    ui.wake_at(ui.now() + std::time::Duration::from_millis(1200));
                }
            }
            IntentAct::Pick { session, index } => {
                self.pick(ui, &session, index);
                ui.set_focus(Some(intent_id(&session)));
            }
        }
    }

    /// Hands the desktop's old command history to Shell blocks once, then lets it go.
    pub(super) fn import_history(&mut self) {
        let Some(dir) = self.data_dir.clone() else {
            return;
        };
        let file = dir.join("desktop").join("command-history.json");
        let Some(entries) =
            crate::keep::read(&file).and_then(|s| serde_json::from_str::<Value>(&s).ok())
        else {
            return;
        };
        self.call(
            Call::Imported(file),
            "extension:call",
            json!({ "id": BLOCKS, "action": "import", "args": { "entries": entries } }),
        );
    }

    /// Paints a finished command's output into its box, inside the log.
    /// False when `id` is not a block.
    pub(super) fn paint_block(&self, ui: &mut Ui, id: u64, r: Rect) -> bool {
        let Some((i, j)) = block_of(id) else {
            return false;
        };
        let Some(session) = self.store.sessions.get(i) else {
            return true;
        };
        let sid = session.info.id.as_str();
        let Some(b) = self.st.shells.get(sid).and_then(|sh| sh.blocks.get(j)) else {
            return true;
        };
        let Some(log) = ui.rect_of(log_id(sid)) else {
            return true;
        };
        let k = ui.scale();
        let clip = Rect::new(log.x * k, log.y * k, log.w * k, log.h * k);
        ui.scene.push_clip(clip);
        paint::pane(ui, &b.view, r, paint::Caret::Hidden, "", &[], 0.0);
        ui.scene.pop_clip();
        true
    }

    /// The shell's blocks when it shows them as a log over its live rows.
    pub(crate) fn split_of<'a>(
        &'a self,
        session: &crate::client::Session,
    ) -> Option<&'a crate::log::Shell> {
        crate::log::split(
            &self.st.points,
            session,
            self.st.shells.get(&session.info.id),
        )
    }

    /// Whether `id` shows as command blocks over its live rows.
    pub fn is_split(&self, id: &str) -> bool {
        self.store
            .session(id)
            .is_some_and(|s| self.split_of(s).is_some())
    }

    /// A split shell's live rows: its PTY sized to the whole pane, the rows
    /// from its prompt to the cursor drawn in the box they got. Returns where
    /// its whole screen sits, for pointer selection.
    pub(super) fn paint_live(
        &self,
        ui: &mut Ui,
        i: usize,
        r: Rect,
        caret: paint::Caret,
        preedit: &str,
    ) -> Option<Rect> {
        let session = self.store.sessions.get(i)?;
        let sid = session.info.id.as_str();
        let sh = self.split_of(session)?;
        let cell = ui.text.cell();
        if let (Some(g), Some(pane)) = (&self.grid, ui.rect_of(term_id(sid))) {
            let cols = (r.w / cell.w).floor() as u16;
            let rows = (pane.h * ui.scale() / cell.h).floor() as u16;
            g.attach(
                sid,
                cols,
                rows,
                super::cell_px(ui),
                crate::grid::SHELL_HISTORY,
            );
            if !self.st.locked.contains(sid) {
                g.resize(sid, cols, rows, super::cell_px(ui));
            }
        }
        let v = self.views.get(sid)?;
        let at = Rect::new(r.x, r.y - f32::from(sh.live.0) * cell.h, r.w, r.h);
        let sel = match &self.st.sel {
            Some((s, sel)) if s == sid => sel.rows(v),
            _ => Vec::new(),
        };
        ui.scene.push_clip(r);
        paint::pane(ui, v, at, caret, preedit, &sel, 0.0);
        ui.scene.pop_clip();
        Some(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_command_takes_its_chords() {
        assert_eq!(control("KeyC"), Some("\u{3}"));
        assert_eq!(control("Backslash"), Some("\u{1c}"));
        assert_eq!(control("KeyA"), None);
    }
}
