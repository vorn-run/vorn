//! A shell's command blocks and its composer's history and completions:
//! the marks vornd reports, the blocks cut from the grid, the menu's keys.

use web_time::Instant;

use serde_json::Value;

use super::{App, Call};
use crate::blocks::{self, Mark};
use crate::client::rpc::RpcError;
use crate::intent::{self, IntentAct, Need, Place, Row};
use crate::look;
use crate::paint;
use crate::screen::{block_of, intent_id, log_id, term_id, SETTLE};
use crate::ui::{mods, Input, Rect, Ui};

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
    /// A `terminal:command` mark: folded into the shell's blocks.
    pub(super) fn shell_notified(&mut self, method: &str, params: &Value) {
        if method != "terminal:command" {
            return;
        }
        let Some((id, mark, at)) = blocks::parse(params) else {
            return;
        };
        let cwd = self
            .store
            .session(&id)
            .and_then(|s| s.info.shell_cwd.clone());
        let started = matches!(mark, Mark::Start(_));
        let sh = self.st.shells.entry(id.clone()).or_default();
        sh.mark(mark, at, cwd.as_deref(), Instant::now());
        if started {
            if let Some(m) = self.st.intent.menus.get_mut(&id) {
                m.close();
            }
        }
        self.sync_shell(&id);
    }

    /// Cuts the shell's lines into blocks again, after its screen changed.
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
                    r.fg = look::GRAY_300;
                }
            }
            Some(v)
        });
    }

    /// Before a frame: the logs kept to their ends, the composers' menus
    /// worked out over what is typed, and the keyboard handed between a
    /// composer and the command it stood aside for.
    pub(super) fn shell_frame(&mut self, ui: &mut Ui) {
        let now = ui.now();
        let mut wants = Vec::new();
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
            let project = Some(s.info.project_path.as_str());
            let m = self.st.intent.menus.entry(sid.to_owned()).or_default();
            if m.text != typed {
                m.text = typed.to_owned();
                m.history = self.st.intent.history.search(typed, project);
                m.changed = Some(if std::mem::take(&mut m.filled) {
                    now - intent::DEBOUNCE
                } else {
                    now
                });
                m.completed = false;
                m.highlight = None;
                m.open = !typed.is_empty();
                if typed.is_empty() {
                    m.completions.clear();
                }
            }
            if m.completed || m.text.is_empty() {
                continue;
            }
            let due = m.changed.map_or(now, |c| c + intent::DEBOUNCE);
            if due > now {
                ui.wake_at(due);
                continue;
            }
            let place = Place {
                cwd: s.info.shell_cwd.as_deref(),
                repo: Some(
                    s.info
                        .worktree_path
                        .as_deref()
                        .unwrap_or(&s.info.project_path),
                ),
            };
            let (found, need) = intent::complete(&m.text, &place, &self.st.intent.sources, now);
            m.completions = found;
            m.completed = true;
            if m.highlight.is_some_and(|h| h >= m.len()) {
                m.highlight = None;
            }
            wants.extend(need);
        }
        for need in wants {
            if self.st.intent.sources.asked.insert(need.clone()) {
                let (method, params) = need.call();
                self.call(Call::Intent(need), method, params);
            }
        }
    }

    /// vornd's answer for completions: kept, and the menus worked out again.
    pub(super) fn intent_answered(&mut self, need: Need, answer: Result<Value, RpcError>) {
        let now = Instant::now();
        self.st
            .intent
            .sources
            .answer(need, answer.ok().as_ref(), now);
        for m in self.st.intent.menus.values_mut() {
            if !m.text.is_empty() {
                m.completed = false;
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
        let project = self
            .store
            .session(&sid)
            .map(|s| s.info.project_path.clone());
        let menu = self.st.intent.menus.entry(sid.clone()).or_default();
        match code.as_str() {
            "ArrowUp" if !menu.shown() => {
                menu.history = self.st.intent.history.search("", project.as_deref());
                menu.completions.clear();
                menu.completed = true;
                menu.open = true;
                menu.highlight = (!menu.is_empty()).then_some(0);
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
                } else if !menu.completions.is_empty() {
                    if let Some(t) = intent::tab(&typed, &menu.completions) {
                        self.fill(ui, &sid, &t);
                    }
                } else if let Some(g) = self.st.intent.history.ghost(&typed, project.as_deref()) {
                    let g = g.to_owned();
                    self.fill(ui, &sid, &g);
                }
                true
            }
            "ArrowRight" => {
                let at_end = ui.edit(field).is_some_and(|e| e.caret() == typed.len());
                let ghost = at_end
                    .then(|| self.st.intent.history.ghost(&typed, project.as_deref()))
                    .flatten()
                    .map(str::to_owned);
                match ghost {
                    Some(g) => {
                        self.fill(ui, &sid, &g);
                        true
                    }
                    None => false,
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
        let typed = ui.edit_text(intent_id(sid)).to_owned();
        let Some(m) = self.st.intent.menus.get(sid) else {
            return;
        };
        let text = match m.rows().get(index) {
            Some(Row::History(e)) => e.text.clone(),
            Some(Row::Completion(c)) => intent::apply(&typed, c),
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

    /// Runs `line` in the shell and remembers it.
    pub(super) fn run_line(&mut self, ui: &mut Ui, sid: &str, line: &str) {
        let line = line.trim();
        ui.set_edit_text(intent_id(sid), "");
        if let Some(m) = self.st.intent.menus.get_mut(sid) {
            m.close();
            m.text.clear();
            m.completions.clear();
        }
        if line.is_empty() {
            return;
        }
        let project = self.store.session(sid).map(|s| s.info.project_path.clone());
        self.st
            .intent
            .history
            .record(line, project.as_deref(), crate::state::now_ms() as u64);
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
                    .map(blocks::Block::text);
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
        let sh = blocks::split(&self.store.config, session, self.st.shells.get(sid))?;
        let cell = ui.text.cell();
        if let (Some(g), Some(pane)) = (&self.grid, ui.rect_of(term_id(sid))) {
            let cols = (r.w / cell.w).floor() as u16;
            let rows = (pane.h * ui.scale() / cell.h).floor() as u16;
            g.attach(sid, cols, rows, crate::grid::SHELL_HISTORY);
            if !self.st.locked.contains(sid) {
                g.resize(sid, cols.max(2), rows.max(1));
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
