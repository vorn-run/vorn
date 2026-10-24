//! What the settings page's controls do and the answers to the calls they
//! make.

use std::sync::mpsc;

use serde_json::{json, Value};

use super::{list, App, Call, MOD};
use crate::act::Act;
use crate::client::RpcError;
use crate::screen::settings::{ROW_SLIDER, SHELL_PATH};
use crate::settings::{
    self, split_args, AgentField, Category, SetAct, Shell, UpdateStatus, CONNECTIONS_PAGE,
};
use crate::ui::{mods, Event as UiEvent, Id, Input, Ui};

/// A settings call whose answer changes the page.
pub(super) enum SetCall {
    Shells,
    Detected,
    /// vornd stopping for Restart Vorn.
    Restart,
    /// A terminal started to install or connect an agent.
    Setup(settings::AgentJob),
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
        let agent = [
            ("agent:", false, false),
            ("agent-advanced:", true, false),
            ("agent-remove:", false, true),
        ]
        .into_iter()
        .find_map(|(p, adv, rm)| Some((name.strip_prefix(p)?, adv, rm)));
        if let Some((key, advanced, remove)) = agent {
            if self.store.agents.get(key).is_none() {
                return Err(format!("--open {name:?}: no agent {key}"));
            }
            let st = &mut self.st.settings;
            st.agent = Some(key.to_owned());
            if advanced {
                st.agents_open.insert(key.to_owned());
            }
            st.confirm_remove = remove.then(|| key.to_owned());
            return Ok(());
        }
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
        if let Some(states) = name.strip_prefix("net-") {
            return self.shot_net(ui, states);
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
                    "--open {name:?}: theme-picker, theme-info, font-picker, agent-picker, shell-picker, channel-menu, holder-menu, sound-menu, notice-sources, agent:KEY, agent-advanced:KEY, agent-remove:KEY or extension:ID"
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

    /// `shot --open net-A+B`: Remote Access states, once vornd has answered
    /// the page: `tailnet` (a running tailnet and its address), `advanced`,
    /// `add-device`, `pair-request` (a phone asking), `device-menu` and
    /// `confirm-remove` on the first device.
    fn shot_net(&mut self, ui: &mut Ui, states: &str) -> Result<(), String> {
        use crate::network::{NetAct, PairingRequest, Tailscale};
        use crate::state::MenuKind;
        let n = &self.st.network;
        if !n.open || n.loading || n.tokens.is_none() {
            return Err(format!("--open net-{states}: Remote Access has not loaded"));
        }
        for state in states.split('+') {
            let first = self.st.network.active().first().map(|t| t.id.clone());
            match state {
                "tailnet" => {
                    let n = &mut self.st.network;
                    n.status = Some(Tailscale {
                        installed: true,
                        running: true,
                    });
                    let r = n.reachable.get_or_insert_with(Default::default);
                    r.remote = true;
                    r.urls.insert(0, format!("https://100.101.102.103:{}/app/", r.port));
                }
                "advanced" => self.st.network.advanced = true,
                "add-device" => self.run_net(ui, NetAct::AddDevice),
                "pair-request" => {
                    self.run_net(ui, NetAct::AddDevice);
                    self.st.network.asking = Some(PairingRequest {
                        request_id: "shot".to_owned(),
                        device_name: "Studio iPhone".to_owned(),
                        address: "100.101.102.104".to_owned(),
                    });
                }
                "device-menu" | "confirm-remove" => {
                    let id = first.ok_or(format!("--open net-{state}: no device"))?;
                    if state == "confirm-remove" {
                        self.st.network.confirm_revoke = Some(id);
                        continue;
                    }
                    let kind = MenuKind::Device(id);
                    let src = super::Src {
                        id: Some(crate::act::Acts::id("net-device", &Act::Open(kind.clone()))),
                        at: None,
                    };
                    self.open_menu(ui, kind, src);
                }
                other => return Err(format!("--open net-{other}: tailnet, advanced, add-device, pair-request, device-menu or confirm-remove")),
            }
        }
        Ok(())
    }

    pub(super) fn set_answered(&mut self, call: SetCall, answer: Result<Value, RpcError>) {
        let st = &mut self.st.settings;
        match call {
            SetCall::Shells => {
                st.shells = Some(list(answer).iter().filter_map(Shell::from_value).collect());
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
            SetCall::Setup(job) => self.setup_started(job, answer),
        }
    }

    /// Looks for the agents again.
    fn redetect(&mut self) {
        if self.st.settings.detecting {
            return;
        }
        self.st.settings.detecting = true;
        self.call(
            Call::Set(SetCall::Detected),
            "agent:locate",
            json!({ "refresh": true }),
        );
    }

    /// Where an agent's setup terminal starts: the composer's project, else
    /// the first one, else home.
    fn setup_place(&self) -> Option<(String, String)> {
        self.composer_project().or_else(|| {
            let p = self.store.projects.first()?;
            Some((p.name.clone(), p.path.clone()))
        })
    }

    /// Runs the agent's installer, or its Vorn tools setup, in a terminal card.
    fn start_setup(&mut self, key: &str, install: bool) {
        let Some(a) = self.store.agents.get(key) else {
            return;
        };
        let place = self.setup_place();
        let cwd = place.as_ref().map_or(Value::Null, |p| json!(p.1));
        let (job, params) = match (&a.mcp, install) {
            (_, true) if !a.install.is_empty() => {
                (settings::AgentJob::Install(a.key.clone()), None)
            }
            (Some(m), false) if m.in_agent => {
                let Some((name, path)) = place else {
                    self.st
                        .toast(format!("Add a project to connect {}.", a.name), true);
                    return;
                };
                let job = settings::AgentJob::ConnectInAgent {
                    agent: a.key.clone(),
                    pending: m.commands.iter().cloned().collect(),
                    since: web_time::Instant::now(),
                };
                let params =
                    json!({ "agentType": a.key, "projectName": name, "projectPath": path });
                (job, Some(params))
            }
            (Some(_), false) => (settings::AgentJob::Connect(a.key.clone()), None),
            _ => return,
        };
        let call = Call::Set(SetCall::Setup(job));
        match params {
            Some(p) => self.call(call, "terminal:create", p),
            None => self.call(call, "shell:create", cwd),
        }
    }

    /// The setup terminal exists: shows it, and types a shell setup into it.
    fn setup_started(&mut self, mut job: settings::AgentJob, answer: Result<Value, RpcError>) {
        let info =
            match answer.and_then(crate::client::rpc::decode::<vorn_protocol::TerminalSession>) {
                Ok(info) => info,
                Err(e) => return self.st.toast(e.to_string(), true),
            };
        let id = info.id.clone();
        self.st.selected = Some(id.clone());
        self.st.focus_terminal = true;
        self.st.view = crate::screen::MainView::Sessions;
        self.st.settings.open = false;
        self.store.upsert(info);
        let line = match &mut job {
            settings::AgentJob::Install(k) => self
                .store
                .agents
                .get(k)
                .map(|a| format!("{} && exit\r", a.install)),
            settings::AgentJob::Connect(k) => self
                .store
                .agents
                .get(k)
                .and_then(|a| a.mcp.as_ref())
                .map(|m| settings::setup_line(&m.commands)),
            settings::AgentJob::ConnectInAgent { since, .. } => {
                *since = web_time::Instant::now();
                None
            }
        };
        if let (Some(line), Some(g)) = (line, &self.grid) {
            g.text(&id, &line);
        }
        self.agent_jobs.insert(id, job);
    }

    /// A setup terminal ended: detection runs again after an installer, and
    /// a shell setup that exited cleanly marks the agent connected.
    pub(super) fn agent_notified(&mut self, method: &str, params: &Value) {
        if method != "terminal:exit" {
            return;
        }
        let Some(id) = params.get("id").and_then(Value::as_str) else {
            return;
        };
        let Some(job) = self.agent_jobs.remove(id) else {
            return;
        };
        let ok = params.get("exitCode").and_then(Value::as_i64).unwrap_or(0) == 0;
        match job {
            settings::AgentJob::Install(_) => self.redetect(),
            settings::AgentJob::Connect(k) if ok => self.mark_connected(&k),
            _ => {}
        }
    }

    fn mark_connected(&mut self, agent: &str) {
        self.write_defaults(|d| {
            if !d.get(settings::AGENT_TOOLS).is_some_and(Value::is_object) {
                d[settings::AGENT_TOOLS] = json!({});
            }
            d[settings::AGENT_TOOLS][agent] = json!(true);
        });
    }

    /// Types the next setup command into each agent session that has gone quiet.
    pub(super) fn agent_tick(&mut self, ui: &mut Ui, now: web_time::Instant) {
        let mut done = Vec::new();
        for (id, job) in &mut self.agent_jobs {
            let settings::AgentJob::ConnectInAgent {
                agent,
                pending,
                since,
            } = job
            else {
                continue;
            };
            let due = settings::type_due(self.st.last_output.get(id).copied(), *since);
            if now < due {
                ui.wake_at(due);
                continue;
            }
            if let (Some(cmd), Some(g)) = (pending.pop_front(), &self.grid) {
                g.text(id, &format!("{cmd}\r"));
            }
            *since = now;
            if pending.is_empty() {
                done.push((id.clone(), agent.clone()));
            }
        }
        for (id, agent) in done {
            self.agent_jobs.remove(&id);
            self.mark_connected(&agent);
        }
    }

    /// Opens the page on `cat` and asks for what that panel shows.
    pub(super) fn open_settings(&mut self, cat: Category) {
        let s = &mut self.st.settings;
        s.open = true;
        s.category = cat;
        s.ext_page = (cat == Category::Connections).then(|| CONNECTIONS_PAGE.to_owned());
        match cat {
            Category::Agents => {
                s.agent = None;
                s.confirm_remove = None;
                self.redetect();
                if self.st.plugins.packs.is_empty() && self.st.plugins.loads == 0 {
                    self.plug_load(false);
                }
            }
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
            Category::Notifications => {
                crate::notify::refresh();
                if self.st.plugins.packs.is_empty() && self.st.plugins.loads == 0 {
                    self.plug_load(false);
                }
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
    pub(super) fn settings_tick(&mut self) {
        self.cli_tick();
        if let Some(rx) = &self.health {
            if let Ok(v) = rx.try_recv() {
                self.st.settings.holders = v.as_ref().and_then(settings::Holders::from_health);
                self.health = None;
            }
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
            SetAct::Redetect => self.redetect(),
            SetAct::OpenAgent(key) => {
                let st = &mut self.st.settings;
                st.agent = key;
                st.confirm_remove = None;
                ui.set_focus(None);
            }
            SetAct::InstallAgent(k) => self.start_setup(&k, true),
            SetAct::ConnectAgent(k) => self.start_setup(&k, false),
            SetAct::RemoveAgent(k) => {
                if self.st.settings.confirm_remove.as_deref() != Some(k.as_str()) {
                    self.st.settings.confirm_remove = Some(k);
                    return;
                }
                self.st.settings.confirm_remove = None;
                let Some(ext) = self.store.agents.get(&k).map(|a| a.extension.clone()) else {
                    return;
                };
                self.st.settings.agent = None;
                self.run_plug(ui, crate::plugins::PlugAct::Remove(ext));
            }
            SetAct::KeepAgent => self.st.settings.confirm_remove = None,
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
            SetAct::AgentText(..) => {}
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
        if self.st.dialog == Some(crate::state::Dialog::AddDevice) {
            self.run_net(ui, crate::network::NetAct::CloseAdding);
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
