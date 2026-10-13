//! The terminal's selection and clipboard: a drag or double-click selects,
//! the copy key copies it, the paste keys paste, and a paste vornd flags
//! asks first.

use vorn_term_proto::msg::GridPoint;

use super::{App, Src};
use crate::act::Act;
use crate::grid::Reply;
use crate::screen::term_id;
use crate::select::Sel;
use crate::state::Dialog;
use crate::ui::{mods, Event as UiEvent, Id, Ui};

/// What a key does with a focused terminal's clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clip {
    Copy,
    Paste,
    SelectAll,
}

/// The renderer's clipboard keys: Cmd-C, Cmd-V and Cmd-A on a Mac;
/// elsewhere Ctrl-Shift-C, Ctrl-V and Ctrl-Shift-V, and Ctrl-C only over a
/// selection, else it interrupts. Shift-Insert pastes everywhere.
pub fn clipboard_key(code: &str, m: u16, selection: bool, mac: bool) -> Option<Clip> {
    let held = m & (mods::SHIFT | mods::CTRL | mods::ALT | mods::SUPER);
    if code == "Insert" && held == mods::SHIFT {
        return Some(Clip::Paste);
    }
    if mac {
        return match code {
            "KeyC" if held == mods::SUPER => Some(Clip::Copy),
            "KeyV" if held == mods::SUPER => Some(Clip::Paste),
            "KeyA" if held == mods::SUPER => Some(Clip::SelectAll),
            _ => None,
        };
    }
    let ctrl = held == mods::CTRL;
    let ctrl_shift = held == mods::CTRL | mods::SHIFT;
    match code {
        "KeyC" if ctrl_shift || ctrl && selection => Some(Clip::Copy),
        "KeyV" if ctrl || ctrl_shift => Some(Clip::Paste),
        _ => None,
    }
}

impl App {
    /// A clipboard key while a terminal has the keyboard. True when used.
    pub(super) fn clip_key(&mut self, ui: &mut Ui, code: &str, m: u16) -> bool {
        let Some(sid) = self.typing_target() else {
            return false;
        };
        let selection = self.st.sel.as_ref().is_some_and(|(s, _)| *s == sid);
        let Some(clip) = clipboard_key(code, m, selection, cfg!(target_os = "macos")) else {
            return false;
        };
        let act = match clip {
            Clip::Copy => Act::CopySelection(sid),
            Clip::Paste => Act::Paste(sid),
            Clip::SelectAll => {
                if let Some(v) = self.views.get(&sid) {
                    self.st.sel = Some((sid, Sel::all(v)));
                }
                return true;
            }
        };
        self.run(ui, act, Src::default());
        true
    }

    /// Copies `session`'s selection, once vornd has its text; the selection goes.
    pub(super) fn copy_selection(&mut self, session: &str) {
        let Some((sid, sel)) = self.st.sel.take() else {
            return;
        };
        if sid != session {
            self.st.sel = Some((sid, sel));
            return;
        }
        if let Some(g) = &self.grid {
            g.copy(session, sel.from, sel.to);
        }
    }

    /// The clipboard into `session`: bracketed, and asked about first when
    /// vornd says the program would run it line by line.
    pub(super) fn paste_clipboard(&mut self, ui: &mut Ui, session: &str) {
        self.st.sel = None;
        if let (Some(g), Some(text)) = (&self.grid, ui.clipboard().get()) {
            if !text.is_empty() {
                g.paste_clipboard(session, &text);
            }
        }
    }

    /// The paste confirm dialog's answer.
    pub(super) fn confirm_paste(&mut self, ui: &mut Ui, run: bool) {
        let pending = self.st.paste.take();
        if self.st.dialog == Some(Dialog::Paste) {
            self.st.dialog = None;
            ui.set_focus(None);
        }
        if let (true, Some(g), Some((session, text))) = (run, &self.grid, pending) {
            g.paste(&session, &text);
        }
    }

    /// What vornd answered: a word to select, text to copy, a paste to confirm.
    pub(super) fn grid_replies(&mut self, ui: &mut Ui) {
        let Some(g) = &self.grid else {
            return;
        };
        for reply in g.take_replies() {
            match reply {
                Reply::Selection { session, range } => {
                    self.word_pending = false;
                    let primary = self.views.get(&session).is_some_and(|v| v.primary);
                    self.st.sel = range.map(|(a, b)| (session, Sel::new(a, b, primary)));
                }
                Reply::Copied(text) => {
                    if !text.is_empty() {
                        ui.clipboard().set(&text);
                    }
                }
                Reply::ConfirmPaste { session, text } => {
                    // A second paste while one is asked about waits for the next.
                    if self.st.dialog.is_none() {
                        self.st.paste = Some((session, text));
                        self.st.dialog = Some(Dialog::Paste);
                    }
                }
            }
        }
    }

    /// The session whose terminal box is `id`.
    fn term_at(&self, id: Id) -> Option<String> {
        self.term_boxes.keys().find(|s| term_id(s) == id).cloned()
    }

    /// The cell under `(x, y)` logical pixels in `session`'s terminal,
    /// held inside the screen.
    fn cell_at(&self, ui: &Ui, session: &str, (x, y): (f32, f32)) -> Option<GridPoint> {
        let (r, v) = (self.term_boxes.get(session)?, self.views.get(session)?);
        let (cell, scale) = (ui.text.cell(), ui.scale());
        let at = |p: f32, o: f32, size: f32, n: u16| {
            ((p * scale - o.round()) / size)
                .floor()
                .clamp(0.0, f32::from(n.saturating_sub(1))) as u16
        };
        Some(v.point(at(x, r.x, cell.w, v.cols), at(y, r.y, cell.h, v.rows)))
    }

    /// A drag or double-click in a terminal selects; a click clears the
    /// selection. True when the event was the terminal's alone.
    pub(super) fn term_event(&mut self, ui: &mut Ui, e: &UiEvent) -> bool {
        match *e {
            UiEvent::Move { id, dx, dy, x, y } | UiEvent::Drop { id, dx, dy, x, y } => {
                let Some(sid) = self.term_at(id) else {
                    return false;
                };
                let from = self.cell_at(ui, &sid, (x - dx, y - dy));
                let to = self.cell_at(ui, &sid, (x, y));
                if let (Some(a), Some(b)) = (from, to) {
                    let primary = self.views.get(&sid).is_some_and(|v| v.primary);
                    self.st.sel = Some((sid, Sel::new(a, b, primary)));
                }
                // A drag that selected still focuses the terminal, as a click would.
                if matches!(e, UiEvent::Drop { .. }) {
                    if let Some(act) = self.acts.clicked(id) {
                        self.run(ui, act, Src::default());
                    }
                }
                true
            }
            UiEvent::DoubleClick(id) => {
                let Some(sid) = self.term_at(id) else {
                    return false;
                };
                let at = ui.pointer().and_then(|p| self.cell_at(ui, &sid, p));
                if let (Some(g), Some(at)) = (&self.grid, at) {
                    g.select_word(&sid, at);
                    self.word_pending = true;
                }
                true
            }
            UiEvent::Click(id) if self.term_at(id).is_some() => {
                if !std::mem::take(&mut self.word_pending) {
                    self.st.sel = None;
                }
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: u16 = mods::SHIFT;
    const CTRL: u16 = mods::CTRL;
    const CMD: u16 = mods::SUPER;

    #[test]
    fn mac_clipboard_keys_are_cmd_c_v_and_a() {
        let key = |code, m| clipboard_key(code, m, false, true);
        assert_eq!(key("KeyC", CMD), Some(Clip::Copy));
        assert_eq!(key("KeyV", CMD), Some(Clip::Paste));
        assert_eq!(key("KeyA", CMD), Some(Clip::SelectAll));
        assert_eq!(key("Insert", SHIFT), Some(Clip::Paste));
        // Ctrl stays the terminal's: Ctrl-C interrupts, Ctrl-V is quoted insert.
        assert_eq!(clipboard_key("KeyC", CTRL, true, true), None);
        assert_eq!(key("KeyV", CTRL), None);
        assert_eq!(key("KeyA", CTRL), None);
        assert_eq!(key("KeyC", CMD | SHIFT), None);
    }

    #[test]
    fn ctrl_c_copies_only_over_a_selection_off_the_mac() {
        let key = |code, m, sel| clipboard_key(code, m, sel, false);
        assert_eq!(key("KeyC", CTRL, true), Some(Clip::Copy));
        assert_eq!(key("KeyC", CTRL, false), None);
        assert_eq!(key("KeyC", CTRL | SHIFT, false), Some(Clip::Copy));
        assert_eq!(key("KeyV", CTRL, false), Some(Clip::Paste));
        assert_eq!(key("KeyV", CTRL | SHIFT, false), Some(Clip::Paste));
        assert_eq!(key("Insert", SHIFT, false), Some(Clip::Paste));
        assert_eq!(key("KeyA", CTRL, false), None);
        assert_eq!(key("KeyC", CMD, true), None);
        assert_eq!(key("Insert", 0, false), None);
    }
}
