//! What the settings page's controls do and the answers to the calls they
//! make.

use std::sync::mpsc;

use serde_json::{json, Value};

use super::{list, App, Call, MOD};
use crate::act::Act;
use crate::client::RpcError;
use crate::screen::settings::{rotate_id, ROW_SLIDER, SHELL_PATH};
use crate::settings::{
    self, split_args, AgentField, Category, ConnKey, KeyRowState, SetAct, Shell, UpdateStatus,
    CONNECTIONS_PAGE,
};
use crate::ui::{mods, Event as UiEvent, Id, Input, Ui};

/// A settings call whose answer changes the page.
pub(super) enum SetCall {
    Shells,
    ConnKeys,
    Preflight(String),
    Rotated(String),
    Detected,
    /// vornd stopping for Restart Vorn.
    Restart,
}

impl App {
    /// `shot --settings PAGE [--set key=json]…`: the defaults written, then
    /// Settings open on `page`.
    pub fn shot_settings(&mut self, page: &str, set: &[(&str, &str)]) -> Result<(), String> {
        let cat = Category::from_id(page).ok_or(format!("no settings page {page:?}"))?;
        for (key, v) in set {
            let value: Value = serde_json::from_str(v).map_err(|e| format!("--set {key}: {e}"))?;
            self.write_defaults(|d| d[*key] = value);
        }
        self.open_settings(cat);
        Ok(())
    }

    /// `shot --update`, `--holder` and `--outdated`: About's states, faked
    /// once vornd has answered. `update` is `idle`, `checking`, `ready:VER`
    /// or `error:TEXT`; `holder` is `BUILD:SESSIONS`; `outdated` lists pack ids.
    pub fn shot_about(
        &mut self,
        update: Option<&str>,
        holder: Option<&str>,
        outdated: Option<&str>,
    ) -> Result<(), String> {
        if let Some(u) = update {
            self.st.settings.update = Some(match u.split_once(':') {
                None if u == "idle" => UpdateStatus::Idle {
                    last_checked: Some(crate::state::now_ms() - 120_000.0),
                },
                None if u == "checking" => UpdateStatus::Checking,
                Some(("ready", v)) => UpdateStatus::Ready {
                    version: v.to_owned(),
                },
                Some(("error", m)) => UpdateStatus::Error {
                    message: m.to_owned(),
                },
                _ => {
                    return Err(format!(
                        "--update {u:?}: idle, checking, ready:VER or error:TEXT"
                    ))
                }
            });
        }
        if let Some(h) = holder {
            let (build, n) = h
                .split_once(':')
                .and_then(|(b, n)| Some((b, n.parse().ok()?)))
                .ok_or(format!("--holder {h:?}: BUILD:SESSIONS"))?;
            self.st.settings.holders = Some(settings::Holders {
                current: true,
                older: vec![settings::Holder {
                    pid: 1,
                    instance: "shot".to_owned(),
                    build: build.to_owned(),
                    sessions: Some(n),
                    compatible: true,
                }],
                error: None,
            });
        }
        for id in outdated.into_iter().flat_map(|o| o.split(',')) {
            let mut found = false;
            for l in self.st.plugins.listings.iter_mut().filter(|l| l.id == id) {
                if let Some(p) = l.pack.as_mut() {
                    p["version"] = Value::from("0.0.1");
                    found = true;
                }
            }
            if !found {
                return Err(format!("--outdated {id}: not installed"));
            }
        }
        Ok(())
    }

    /// `shot --open NAME`: an Appearance, General or About popover opened from its
    /// control, as a click on it would; `theme-picker` and `theme-info` use the first
    /// theme row. `extension:ID` selects that extension's page, and fails
    /// until vornd has listed it.
    pub fn shot_open(&mut self, ui: &mut Ui, name: &str) -> Result<(), String> {
        use crate::state::MenuKind;
        use vorn_theme::Appearance as Look;
        if let Some(id) = name.strip_prefix("extension:") {
            if self.st.plugins.listings.is_empty() && self.st.plugins.loads == 0 {
                self.run_plug(ui, crate::plugins::PlugAct::Refresh);
            }
            let p = &mut self.st.plugins;
            let key = p
                .listings
                .iter()
                .find(|l| l.id == id)
                .map(|l| l.key.clone());
            let key = key
                .filter(|_| p.loads == 0)
                .ok_or(format!("--open {name:?}: {id} is not listed"))?;
            p.selected = Some(key);
            return Ok(());
        }
        let prefs = crate::appearance::prefs(self.store.config.get("defaults"));
        let look = crate::appearance::theme_rows(prefs.scheme)[0].look;
        let (place, kind) = match name {
            "theme-picker" => ("theme-chip", MenuKind::ThemePicker(look)),
            "theme-info" => {
                let id = match look {
                    Look::Light => prefs.light,
                    Look::Dark => prefs.dark,
                };
                ("theme-info", MenuKind::ThemeInfo(id))
            }
            "font-picker" => ("font-chip", MenuKind::FontPicker),
            "agent-picker" => ("agent-chip", MenuKind::AgentPicker),
            "shell-picker" => ("shell-picker", MenuKind::SetShell),
            "channel-menu" => ("update-channel", MenuKind::UpdateChannel),
            "sound-menu" => ("notif-sound", MenuKind::NotifSound),
            "notice-sources" => ("notif-sources", MenuKind::NotifSources),
            "holder-menu" => ("holder-more", MenuKind::Holder("shot".to_owned())),
            _ => {
                return Err(format!(
                    "--open {name:?}: theme-picker, theme-info, font-picker, agent-picker, shell-picker, channel-menu, holder-menu, sound-menu, notice-sources or extension:ID"
                ))
            }
        };
        let src = super::Src {
            id: Some(crate::act::Acts::id(place, &Act::Open(kind.clone()))),
            at: None,
        };
        self.open_menu(ui, kind, src);
        Ok(())
    }

    pub(super) fn set_answered(&mut self, call: SetCall, answer: Result<Value, RpcError>) {
        let st = &mut self.st.settings;
        match call {
            SetCall::Shells => {
                st.shells = Some(list(answer).iter().filter_map(Shell::from_value).collect());
            }
            SetCall::ConnKeys => {
                st.keys = Some(
                    list(answer)
                        .iter()
                        .filter_map(ConnKey::from_value)
                        .collect(),
                );
            }
            SetCall::Preflight(id) => {
                let row = st.key_rows.entry(id).or_default();
                row.testing = false;
                row.result = Some(match answer {
                    Ok(v) => (
                        v.get("ok").and_then(Value::as_bool),
                        v.get("message").and_then(Value::as_str).map(str::to_owned),
                    ),
                    Err(e) => (Some(false), Some(e.to_string())),
                });
            }
            SetCall::Rotated(id) => {
                let row = st.key_rows.entry(id.clone()).or_default();
                row.saving = false;
                let failure = match answer {
                    Ok(v) if v.get("ok").and_then(Value::as_bool) == Some(true) => None,
                    Ok(v) => Some(
                        v.get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("The key could not be replaced")
                            .to_owned(),
                    ),
                    Err(e) => Some(e.to_string()),
                };
                match failure {
                    Some(e) => row.error = Some(e),
                    None => {
                        *row = KeyRowState::default();
                        self.call(
                            Call::Set(SetCall::ConnKeys),
                            "connection:listKeys",
                            Value::Null,
                        );
                    }
                }
            }
            SetCall::Restart => self.relaunch(),
            SetCall::Detected => {
                st.detecting = false;
                if let Some((found, paths)) =
                    answer.ok().as_ref().and_then(crate::settings::located)
                {
                    self.st.installed = Some(found);
                    self.st.agent_paths = paths;
                }
            }
        }
    }

    /// Opens the page on `cat` and asks for what that panel shows.
    pub(super) fn open_settings(&mut self, cat: Category) {
        let s = &mut self.st.settings;
        s.open = true;
        s.category = cat;
        s.ext_page = (cat == Category::Connections).then(|| CONNECTIONS_PAGE.to_owned());
        match cat {
            Category::General => {
                if s.shells.is_none() {
                    self.call(
                        Call::Set(SetCall::Shells),
                        "shell:listInstalled",
                        Value::Null,
                    );
                }
                if !crate::settings::IN_BROWSER {
                    self.st.settings.cli = Some(crate::cli_link::state());
                }
            }
            Category::Notifications => crate::notify::refresh(),
            Category::Keys => {
                self.call(
                    Call::Set(SetCall::ConnKeys),
                    "connection:listKeys",
                    Value::Null,
                );
            }
            Category::About => {
                self.health = Some(health(self.port));
                if !crate::settings::IN_BROWSER {
                    self.st.settings.cli = Some(crate::cli_link::state());
                }
                if self.st.plugins.listings.is_empty() && self.st.plugins.loads == 0 {
                    self.plug_load(true);
                }
            }
            _ => {}
        }
    }

    /// After Install or Remove: the row's state, and what happened as a toast.
    fn cli_done(&mut self, done: Result<String, String>) {
        self.st.cli_notice = None;
        self.st.settings.cli_error = done.as_ref().err().cloned();
        match done {
            Ok(text) => self.st.toast(text, false),
            Err(e) => self.st.toast(e, true),
        }
        self.st.settings.cli = Some(crate::cli_link::state());
    }

    /// Takes in the `vorn` command's launch notice and the PATH prompt's answer.
    fn cli_tick(&mut self) {
        if let Some(rx) = &self.cli_launch {
            if let Ok(notice) = rx.try_recv() {
                self.st.cli_notice = notice;
                self.cli_launch = None;
            }
        }
        let Some(rx) = &self.cli_prompt else { return };
        let Ok(answer) = rx.try_recv() else { return };
        self.cli_prompt = None;
        self.st.settings.cli_busy = false;
        match answer {
            Ok(true) => {
                self.st.cli_notice = None;
                self.st.toast("New terminals find the vorn command.", false);
            }
            Ok(false) => {}
            Err(e) => self.st.toast(e, true),
        }
        self.st.settings.cli = Some(crate::cli_link::state());
    }

    /// Takes in the health check once it answers.
    pub(super) fn settings_tick(&mut self, now: web_time::Instant) {
        self.cli_tick();
        if let Some(rx) = &self.health {
            if let Ok(v) = rx.try_recv() {
                self.st.settings.holders = v.as_ref().and_then(settings::Holders::from_health);
                self.health = None;
            }
        }
        if self
            .st
            .settings
            .copied
            .as_ref()
            .is_some_and(|(_, at)| now.duration_since(*at).as_secs() >= 2)
        {
            self.st.settings.copied = None;
        }
    }

    fn write_defaults(&mut self, f: impl FnOnce(&mut Value)) {
        let mut config = self.store.config.clone();
        if !config.get("defaults").is_some_and(Value::is_object) {
            config["defaults"] = json!({});
        }
        f(&mut config["defaults"]);
        let login = crate::system::login_item_for(&config, crate::system::packaged());
        self.save_config(config.clone(), Call::Quiet);
        if login.is_some() {
            if let Err(e) = crate::system::apply_login_item(&config) {
                self.st.toast(e, true);
            }
        }
    }

    fn write_agent(&mut self, key: &str, f: impl FnOnce(&mut Value)) {
        let Some(a) = self.store.agents.get(key).cloned() else {
            return;
        };
        let mut config = self.store.config.clone();
        if !config.get("agentCommands").is_some_and(Value::is_object) {
            config["agentCommands"] = json!({});
        }
        for d in &self.store.agents {
            if config["agentCommands"].get(&d.key).is_none() {
                config["agentCommands"][&d.key] = settings::default_command(d);
            }
        }
        let mut cmd = settings::command_of(&config, &a);
        f(&mut cmd);
        config["agentCommands"][key] = cmd;
        self.save_config(config, Call::Quiet);
    }

    pub(super) fn run_set(&mut self, ui: &mut Ui, a: SetAct) {
        match a {
            SetAct::Open(cat) => self.open_settings(cat),
            SetAct::Close => {
                self.st.settings.open = false;
                ui.set_focus(None);
            }
            SetAct::Default(key, v) => {
                let Ok(value) = serde_json::from_str::<Value>(&v) else {
                    return;
                };
                if key == "layoutMode" {
                    let mode = crate::state::LayoutMode::from_key(value.as_str().unwrap_or(""));
                    self.st.layout = mode;
                    if mode != crate::state::LayoutMode::Grid {
                        self.st.focused = None;
                    }
                }
                self.write_defaults(|d| d[key.as_str()] = value);
            }
            SetAct::Notify(key, value) => {
                let Ok(value) = serde_json::from_str::<Value>(&value) else {
                    return;
                };
                self.write_defaults(|d| {
                    if !d.get("notifications").is_some_and(Value::is_object) {
                        d["notifications"] = json!({});
                    }
                    d["notifications"][key.as_str()] = value;
                });
            }
            SetAct::PickTheme(id) => {
                let installed = self.st.settings.appearance.installed();
                let Some(look) = crate::appearance::find(installed, &id).map(|t| t.appearance)
                else {
                    return;
                };
                let scheme = crate::appearance::prefs(self.store.config.get("defaults")).scheme;
                let writes = crate::appearance::pick_theme(scheme, &id, look);
                self.st.settings.theme_preview = None;
                self.write_defaults(|d| {
                    for (k, v) in writes {
                        d[k] = v;
                    }
                });
            }
            SetAct::ThemeFilter(f) => {
                let on = &mut self.st.settings.theme_filter;
                *on = (*on != Some(f)).then_some(f);
            }
            SetAct::Browse(query, listing) => {
                self.open_settings(Category::Connectors);
                ui.set_focus(None);
                let p = &mut self.st.plugins;
                p.tab = crate::plugins::Tab::Browse;
                p.kind = Some(crate::plugins::Kind::Extension);
                p.query = query;
                p.selected = listing;
                p.adding = None;
            }
            SetAct::OpenNotifSettings => crate::notify::open_system_settings(),
            SetAct::ShellMenu => {}
            SetAct::PickShell(path) => {
                let cur = self
                    .store
                    .config
                    .pointer("/defaults/shell")
                    .and_then(Value::as_str);
                if cur != Some(path.as_str()) {
                    self.write_defaults(|d| d["shell"] = json!(path));
                }
            }
            SetAct::EditShell => {
                let cur = self
                    .store
                    .config
                    .pointer("/defaults/shell")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                self.st.settings.shell_editing = true;
                ui.set_edit_text(SHELL_PATH, &cur);
                ui.set_focus(Some(SHELL_PATH));
            }
            SetAct::InstallCli => {
                let done = crate::cli_link::install(self.data_dir.as_deref());
                self.cli_done(done.map(|at| {
                    format!(
                        "The vorn command is installed at {}.",
                        crate::cli_link::tidy(&at)
                    )
                }));
            }
            SetAct::RemoveCli => {
                let done = crate::cli_link::remove(self.data_dir.as_deref());
                self.cli_done(done.map(|()| "Removed the vorn command.".to_owned()));
            }
            SetAct::CliAddToPath(dir) => {
                if self.cli_prompt.is_none() {
                    let wake = std::sync::Arc::clone(&self.wake);
                    self.cli_prompt = Some(crate::cli_link::add_to_path(dir, wake));
                    self.st.settings.cli_busy = true;
                }
            }
            SetAct::DismissCliNotice => self.st.cli_notice = None,
            SetAct::CheckUpdates => self.check_updates(),
            SetAct::UpdateNow => self.update_action(),
            SetAct::DismissUpdate => self.st.settings.update_dismissed = true,
            SetAct::Redetect => {
                self.st.settings.detecting = true;
                self.call(
                    Call::Set(SetCall::Detected),
                    "agent:locate",
                    json!({ "refresh": true }),
                );
            }
            SetAct::ResetAgent(k) => {
                let Some(a) = self.store.agents.get(&k) else {
                    return;
                };
                let d = settings::default_command(a);
                self.write_agent(&k, |c| *c = d);
                self.st.settings.args.retain(|(agent, _), _| *agent != k);
                ui.set_focus(None);
            }
            SetAct::ExpandAgent(k) => {
                let o = &mut self.st.settings.agents_open;
                if !o.remove(&k) {
                    o.insert(k);
                }
            }
            SetAct::CopyInstall(k) => {
                if let Some(a) = self.store.agents.get(&k) {
                    ui.clipboard().set(&a.install);
                    self.st.settings.copied = Some((k, web_time::Instant::now()));
                }
            }
            SetAct::Copy(text) => {
                ui.clipboard().set(&text);
                self.st.settings.copied = Some((text, web_time::Instant::now()));
            }
            SetAct::AgentText(..) => {}
            SetAct::TestKey(id) => {
                let row = self.st.settings.key_rows.entry(id.clone()).or_default();
                row.testing = true;
                row.result = None;
                self.call(
                    Call::Set(SetCall::Preflight(id.clone())),
                    "connection:preflight",
                    json!(id),
                );
            }
            SetAct::Rotate(id, field) => {
                let row = self.st.settings.key_rows.entry(id.clone()).or_default();
                row.rotating = if row.rotating.as_deref() == Some(field.as_str()) {
                    None
                } else {
                    Some(field)
                };
                row.error = None;
                ui.set_edit_text(rotate_id(&id), "");
                if row.rotating.is_some() {
                    ui.set_focus(Some(rotate_id(&id)));
                }
            }
            SetAct::SaveRotate(id, field) => {
                let value = ui.edit_text(rotate_id(&id)).to_owned();
                if value.trim().is_empty() {
                    return;
                }
                let row = self.st.settings.key_rows.entry(id.clone()).or_default();
                row.saving = true;
                row.error = None;
                self.call(
                    Call::Set(SetCall::Rotated(id.clone())),
                    "connection:rotateSecret",
                    json!({ "connectionId": id, "field": field, "plaintext": value }),
                );
            }
            SetAct::CancelRotate(id) => {
                let row = self.st.settings.key_rows.entry(id.clone()).or_default();
                row.rotating = None;
                row.error = None;
                ui.set_edit_text(rotate_id(&id), "");
            }
            SetAct::EndHolder(instance) => {
                if self.st.settings.confirm_end.as_deref() != Some(instance.as_str()) {
                    self.st.settings.confirm_end = Some(instance);
                    return;
                }
                self.st.settings.confirm_end = None;
                let outcome = match (&self.data_dir, &self.st.settings.holders) {
                    (Some(home), Some(h)) => crate::system::end_older_holder(home, h, &instance),
                    _ => Err("vornd is not in use".to_owned()),
                };
                self.st.settings.end_failure =
                    outcome.err().map(|d| format!("Could not end them: {d}."));
                self.health = Some(health(self.port));
            }
            SetAct::KeepHolder => self.st.settings.confirm_end = None,
            SetAct::RestartVorn => {
                self.call(Call::Set(SetCall::Restart), "server:shutdown", Value::Null)
            }
            SetAct::UpdateExtensions => {
                let keys: Vec<String> = crate::plugins::pack_updates(&self.st.plugins.listings)
                    .1
                    .iter()
                    .map(|l| l.key.clone())
                    .collect();
                for key in keys {
                    self.plug_inspect(&key, true);
                }
            }
        }
    }

    /// Text typed in a settings field, and the sliders' drags.
    pub(super) fn settings_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        match *event {
            UiEvent::Move { id, x, .. } | UiEvent::Drop { id, x, .. } if id == ROW_SLIDER => {
                let Some(r) = ui.rect_of(id) else { return true };
                let f = ((x - r.x - 8.0) / (r.w - 16.0)).clamp(0.0, 1.0);
                self.st.settings.row_height = (100.0 + f * 500.0).round();
                true
            }
            UiEvent::Changed(id) => {
                let Some(Act::Set(a)) = self.acts.text_of(id) else {
                    return false;
                };
                let text = ui.edit_text(id).to_owned();
                if let SetAct::AgentText(agent, f) = a {
                    match f {
                        AgentField::Command => {
                            self.write_agent(&agent, |c| c["command"] = json!(text))
                        }
                        AgentField::Args | AgentField::Headless => {
                            let k = if f == AgentField::Args {
                                "args"
                            } else {
                                "headlessArgs"
                            };
                            self.st
                                .settings
                                .args
                                .insert((agent.clone(), f), text.clone());
                            self.write_agent(&agent, |c| c[k] = json!(split_args(&text)));
                        }
                    }
                }
                true
            }
            UiEvent::Submit(id) => {
                if id == SHELL_PATH {
                    self.commit_shell(ui);
                    return true;
                }
                if let Some(k) = self
                    .st
                    .settings
                    .keys
                    .iter()
                    .flatten()
                    .find(|k| rotate_id(&k.connection) == id)
                {
                    let row = self
                        .st
                        .settings
                        .key_rows
                        .get(&k.connection)
                        .cloned()
                        .unwrap_or_default();
                    if let Some(f) = row.rotating {
                        let c = k.connection.clone();
                        self.run_set(ui, SetAct::SaveRotate(c, f));
                    }
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn commit_shell(&mut self, ui: &mut Ui) {
        let next = ui.edit_text(SHELL_PATH).trim().to_owned();
        self.st.settings.shell_editing = false;
        if ui.focus() == Some(SHELL_PATH) {
            ui.set_focus(None);
        }
        let cur = self
            .store
            .config
            .pointer("/defaults/shell")
            .and_then(Value::as_str);
        if !next.is_empty() && cur != Some(next.as_str()) {
            self.write_defaults(|d| d["shell"] = json!(next));
        }
    }

    /// A field lost focus: what `onBlur` does there.
    pub(super) fn settings_blur(&mut self, ui: &mut Ui, id: Id) {
        if id == SHELL_PATH && self.st.settings.shell_editing {
            self.commit_shell(ui);
        } else if let Some(Act::Set(SetAct::AgentText(agent, f))) = self.acts.text_of(id) {
            self.st.settings.args.remove(&(agent, f));
        }
    }

    /// Escape closes the page, and Mod+Comma opens it.
    pub(super) fn settings_key(&mut self, ui: &mut Ui, input: &Input) -> bool {
        let Input::Key { code, mods: m, .. } = input else {
            return false;
        };
        let held = *m & (mods::CTRL | mods::SUPER | mods::ALT | mods::SHIFT);
        if code == "Comma" && held == MOD() {
            let cat = self.st.settings.category;
            self.open_settings(cat);
            return true;
        }
        if !self.st.settings.open || code != "Escape" || self.st.menu.is_some() {
            return false;
        }
        if self.st.dialog == Some(crate::state::Dialog::ExtInstall) {
            self.run_plug(ui, crate::plugins::PlugAct::CancelInstall);
            ui.set_focus(None);
            return true;
        }
        if ui.focus() == Some(SHELL_PATH) {
            self.st.settings.shell_editing = false;
            ui.set_focus(None);
            return true;
        }
        self.st.settings.open = false;
        ui.set_focus(None);
        true
    }
}

/// A fresh id, as `crypto.randomUUID` gives one.
pub(super) fn new_id() -> String {
    let n = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let a = n as u64;
    let b = (n >> 64) as u64 ^ u64::from(std::process::id()).rotate_left(17);
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        a as u32,
        (a >> 32) as u16,
        (a >> 48) as u16 & 0xfff,
        b as u16 & 0xfff,
        b >> 16 & 0xffff_ffff_ffff
    )
}

/// `/vornd/health`, read on a thread so the frame does not wait on it.
fn health(port: u16) -> mpsc::Receiver<Option<Value>> {
    let (tx, rx) = mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let _ = tx.send(crate::system::http_get_json(port, "/vornd/health"));
    });
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        let _ = port;
        let _ = tx.send(crate::page::get_json("/vornd/health").await);
    });
    rx
}
