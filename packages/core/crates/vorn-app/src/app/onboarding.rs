//! What the welcome guide's controls and keys do.

use serde_json::Value;

use super::{App, Call, Src};
use crate::act::Act;
use crate::onboarding::{self, OnbAct, Then};
use crate::screen::MainView;
use crate::state::Dialog;
use crate::ui::{mods, Input, Ui};

impl App {
    /// Opens the guide at its first step.
    pub(super) fn open_onboard(&mut self, ui: &mut Ui) {
        self.st.onboard = Default::default();
        self.st.dialog = Some(crate::state::Dialog::Welcome);
        ui.set_focus(None);
    }

    pub(super) fn run_onboard(&mut self, ui: &mut Ui, act: OnbAct) {
        match act {
            OnbAct::Close => self.close_onboard(ui),
            OnbAct::Next if self.st.onboard.is_last() => self.close_onboard(ui),
            OnbAct::Next => self.st.onboard.next(),
            OnbAct::Back => self.st.onboard.back(),
            OnbAct::Go(i) => self.st.onboard.go(i),
            OnbAct::ToggleMcp => self.st.onboard.mcp_open = !self.st.onboard.mcp_open,
            OnbAct::Copy(text) => {
                ui.clipboard().set(&text);
                self.st.onboard.copied = Some((text, web_time::Instant::now()));
            }
            OnbAct::Redetect => {
                // What was found stays on screen until the answer replaces it.
                if let Some(found) = self.st.installed.take() {
                    self.st.onboard.detected = Some(found);
                }
                self.call(Call::Installed, "agent:detectInstalled", Value::Null);
            }
            OnbAct::Then(then) => {
                self.close_onboard(ui);
                match then {
                    Then::AddProject => self.run(ui, Act::AddProject, Src::default()),
                    Then::OpenTasks => self.run(ui, Act::View(MainView::Tasks), Src::default()),
                    Then::NewSession => self.run(ui, Act::NewSession, Src::default()),
                    Then::Palette => self.toggle_palette(ui),
                    Then::Settings(id) => match crate::settings::Category::from_id(id) {
                        Some(cat) => self.run(
                            ui,
                            Act::Set(crate::settings::SetAct::Open(cat)),
                            Src::default(),
                        ),
                        None => self.run(ui, Act::Settings, Src::default()),
                    },
                }
            }
        }
    }

    /// Closes the guide and saves `hasSeenOnboarding`, as every way out does.
    fn close_onboard(&mut self, ui: &mut Ui) {
        if self.st.dialog == Some(Dialog::Welcome) {
            self.st.dialog = None;
            ui.set_focus(None);
        }
        if let Some(config) = onboarding::mark_seen(&self.store.config) {
            self.save_config(config, Call::Quiet);
        }
    }

    /// Escape closes the guide; ← and → change its step.
    pub(super) fn onboard_key(&mut self, ui: &mut Ui, input: &Input) -> bool {
        if self.st.dialog != Some(Dialog::Welcome) {
            return false;
        }
        let Input::Key { code, mods: m, .. } = input else {
            return false;
        };
        if *m & (mods::CTRL | mods::SUPER | mods::ALT) != 0 {
            return false;
        }
        match code.as_str() {
            "Escape" => self.close_onboard(ui),
            "ArrowRight" => self.st.onboard.next(),
            "ArrowLeft" => self.st.onboard.back(),
            _ => return false,
        }
        true
    }
}
