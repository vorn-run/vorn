//! What the command palette does: opening it with what it lists loaded,
//! the keys that move through it, and running the chosen command.

use serde_json::{json, Value};

use super::{App, Call, Src};
use crate::act::Act;
use crate::palette::{self, PalAct};
use crate::screen::{alt_label, mod_label, PALETTE_INPUT, PALETTE_LIST};
use crate::state::Dialog;
use crate::ui::{mods, Event as UiEvent, Input, Ui};

impl App {
    /// The rows the palette shows now, in order.
    pub(super) fn palette_rows(&self) -> Vec<palette::Entry> {
        let all = palette::commands(&self.store, &self.st, mod_label(), alt_label());
        palette::shown(all, &self.st.palette.query)
    }

    /// Mod+K: opens the palette empty, or closes it.
    pub(super) fn toggle_palette(&mut self, ui: &mut Ui) {
        if self.st.dialog == Some(Dialog::Palette) {
            self.st.dialog = None;
            ui.set_focus(None);
            return;
        }
        self.st.palette = Default::default();
        self.st.dialog = Some(Dialog::Palette);
        self.st.menu = None;
        ui.close_menu();
        ui.set_edit_text(PALETTE_INPUT, "");
        ui.set_focus(Some(PALETTE_INPUT));
        ui.set_scroll_offset(PALETTE_LIST, 0.0);
        self.call(Call::Recent, "sessions:getRecent", Value::Null);
        self.call(Call::Layouts, "layout:list", Value::Null);
        for p in self.store.projects.clone() {
            self.call(
                Call::Worktrees(p.path.clone()),
                "git:listWorktrees",
                json!(p.path),
            );
        }
    }

    pub(super) fn run_palette(&mut self, ui: &mut Ui, act: PalAct) {
        match act {
            PalAct::Execute(i) => {
                let Some(cmd) = self.palette_rows().into_iter().nth(i) else {
                    return;
                };
                if cmd.act == Act::OpenLayoutMenu {
                    // A submenu in the palette's own terms: the list narrows to the layouts.
                    self.st.palette.query = palette::LAYOUT_MENU.to_owned();
                    self.st.palette.active = 0;
                    ui.set_edit_text(PALETTE_INPUT, palette::LAYOUT_MENU);
                    ui.set_focus(Some(PALETTE_INPUT));
                    return;
                }
                self.st.dialog = None;
                ui.set_focus(None);
                self.run(ui, cmd.act, Src::default());
            }
        }
    }

    /// ↑ and ↓ move the active row, wrapping; Enter runs it.
    pub(super) fn palette_key(&mut self, ui: &mut Ui, input: &Input) -> bool {
        if self.st.dialog != Some(Dialog::Palette) {
            return false;
        }
        let Input::Key { code, mods: m, .. } = input else {
            return false;
        };
        if *m & (mods::CTRL | mods::SUPER | mods::ALT | mods::SHIFT) != 0 {
            return false;
        }
        let n = self.palette_rows().len();
        match code.as_str() {
            "ArrowDown" => self.st.palette.step(n, true),
            "ArrowUp" => self.st.palette.step(n, false),
            "Enter" => {
                let i = self.st.palette.active;
                self.run_palette(ui, PalAct::Execute(i));
                return true;
            }
            _ => return false,
        }
        self.reveal_active(ui);
        true
    }

    /// `scrollIntoView({ block: 'nearest' })` for the active row.
    fn reveal_active(&self, ui: &mut Ui) {
        let row = crate::act::Acts::id(
            "palette",
            &Act::Palette(PalAct::Execute(self.st.palette.active)),
        );
        let (Some(r), Some(list)) = (ui.rect_of(row), ui.rect_of(PALETTE_LIST)) else {
            return;
        };
        let at = ui.scroll_offset(PALETTE_LIST);
        if r.y < list.y {
            ui.set_scroll_offset(PALETTE_LIST, at - (list.y - r.y));
        } else if r.y + r.h > list.y + list.h {
            ui.set_scroll_offset(PALETTE_LIST, at + (r.y + r.h - list.y - list.h));
        }
    }

    pub(super) fn palette_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        match event {
            UiEvent::Changed(id) if *id == PALETTE_INPUT => {
                self.st.palette.query = ui.edit_text(PALETTE_INPUT).to_owned();
                self.st.palette.active = 0;
                ui.set_scroll_offset(PALETTE_LIST, 0.0);
                true
            }
            _ => false,
        }
    }
}
