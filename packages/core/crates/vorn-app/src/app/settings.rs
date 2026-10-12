//! What the settings page's controls do and the answers to the calls they
//! make, as `SettingsPage` and its panels do them.

use std::sync::mpsc;

use serde_json::{json, Value};

use super::{list, App, Call, MOD};
use crate::act::Act;
use crate::client::RpcError;
use crate::screen::settings::{
    host_id, key_field_id, rotate_id, ROW_SLIDER, SHELL_PATH, VOLUME_SLIDER,
};
use crate::settings::{
    self, report, split_args, AgentField, Category, ConnKey, HostField, KeyField, KeyRowState,
    SetAct, Shell, SshKey, TestResult,
};
use crate::ui::{mods, Event as UiEvent, Id, Input, Ui};

/// A settings call whose answer changes the page.
pub(super) enum SetCall {
    Shells,
    Inventory,
    /// A worktree batch; its toasts say `verb`.
    Batch(&'static str),
    /// `git:deleteBranches`, reported as a batch.
    Branches,
    SshKeys,
    KeySaved,
    Test(String),
    ConnKeys,
    Preflight(String),
    Rotated(String),
    Core,
    Detected,
}

impl App {
    pub(super) fn set_answered(&mut self, call: SetCall, answer: Result<Value, RpcError>) {
        let st = &mut self.st.settings;
        match call {
            SetCall::Shells => {
                st.shells = Some(list(answer).iter().filter_map(Shell::from_value).collect());
            }
            SetCall::Inventory => {
                let page = &mut st.worktrees;
                page.loading = false;
                match answer {
                    Ok(v) => {
                        page.projects = Some(settings::inventory(&v));
                        page.prune();
                    }
                    Err(e) => self.st.toast(e.to_string(), true),
                }
            }
            SetCall::Batch(verb) => {
                st.worktrees.busy = false;
                match answer {
                    Ok(v) => {
                        let all_failed = v
                            .get("succeeded")
                            .and_then(Value::as_array)
                            .is_some_and(Vec::is_empty)
                            && v.get("failed")
                                .and_then(Value::as_array)
                                .is_some_and(|f| !f.is_empty());
                        let lines = report(&v, verb);
                        if all_failed {
                            if let Some((t, _)) = lines.into_iter().find(|(_, e)| *e) {
                                self.st.toast(t, true);
                            }
                        } else {
                            for (t, e) in lines {
                                self.st.toast(t, e);
                            }
                        }
                    }
                    Err(e) => self.st.toast(e.to_string(), true),
                }
                self.rescan();
            }
            SetCall::Branches => {
                st.worktrees.busy = false;
                match answer {
                    Ok(v) => {
                        let as_batch = json!({
                            "succeeded": v.get("deleted").cloned().unwrap_or(json!([])),
                            "failed": v.get("failed").and_then(Value::as_array).into_iter().flatten()
                                .map(|f| json!({ "path": f.get("branch"), "error": f.get("error") }))
                                .collect::<Vec<_>>(),
                            "freedBytes": 0,
                        });
                        for (t, e) in report(&as_batch, "Deleted") {
                            self.st.toast(t, e);
                        }
                    }
                    Err(e) => self.st.toast(e.to_string(), true),
                }
                self.rescan();
            }
            SetCall::SshKeys => {
                if let Ok(v) = answer {
                    st.ssh.keys = v
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(SshKey::from_value)
                        .collect();
                }
            }
            SetCall::KeySaved => match answer {
                Ok(_) => self.load_ssh_keys(),
                Err(e) => self.st.toast(e.to_string(), true),
            },
            SetCall::Test(id) => {
                let r = match answer {
                    Ok(v) => TestResult::Done {
                        ok: v.get("success").and_then(Value::as_bool) == Some(true),
                        message: v
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    },
                    Err(e) => TestResult::Done {
                        ok: false,
                        message: e.to_string(),
                    },
                };
                st.ssh.tests.insert(id, r);
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
            SetCall::Core => {
                if let Ok(v) = answer {
                    st.core_version = v.get("version").and_then(Value::as_str).map(str::to_owned);
                }
            }
            SetCall::Detected => {
                st.detecting = false;
                if let Ok(Value::Object(m)) = answer {
                    self.st.installed = Some(
                        m.into_iter()
                            .filter(|(_, v)| v.as_bool() == Some(true))
                            .map(|(k, _)| k)
                            .collect(),
                    );
                }
            }
        }
    }

    fn rescan(&mut self) {
        self.st.settings.worktrees.loading = true;
        self.call(
            Call::Set(SetCall::Inventory),
            "worktree:inventory",
            json!({ "refresh": true }),
        );
    }

    fn load_ssh_keys(&mut self) {
        self.call(
            Call::Set(SetCall::SshKeys),
            "credential:listKeys",
            Value::Null,
        );
    }

    /// Opens the page on `cat` and asks for what that panel shows.
    pub(super) fn open_settings(&mut self, cat: Category) {
        let s = &mut self.st.settings;
        s.open = true;
        s.category = cat;
        s.worktrees.confirming = None;
        match cat {
            Category::General => {
                if s.shells.is_none() {
                    self.call(
                        Call::Set(SetCall::Shells),
                        "shell:listInstalled",
                        Value::Null,
                    );
                }
                self.st.settings.cli = Some(crate::system::cli_status());
            }
            Category::Worktrees => {
                if s.worktrees.projects.is_none() {
                    s.worktrees.loading = true;
                    self.call(
                        Call::Set(SetCall::Inventory),
                        "worktree:inventory",
                        json!({ "refresh": false }),
                    );
                }
            }
            Category::Ssh => self.load_ssh_keys(),
            Category::Keys => {
                self.call(
                    Call::Set(SetCall::ConnKeys),
                    "connection:listKeys",
                    Value::Null,
                );
            }
            Category::About => {
                self.call(Call::Set(SetCall::Core), "core:status", Value::Null);
                self.health = Some(health(self.port));
            }
            _ => {}
        }
    }

    /// Takes in the health check once it answers.
    pub(super) fn settings_tick(&mut self, now: std::time::Instant) {
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
        let Some(a) = settings::agent(key) else {
            return;
        };
        let mut config = self.store.config.clone();
        if !config.get("agentCommands").is_some_and(Value::is_object) {
            config["agentCommands"] = json!({});
        }
        for d in settings::AGENTS {
            if config["agentCommands"].get(d.key).is_none() {
                config["agentCommands"][d.key] = settings::default_command(&d);
            }
        }
        let mut cmd = settings::command_of(&config, a);
        f(&mut cmd);
        config["agentCommands"][key] = cmd;
        self.save_config(config, Call::Quiet);
    }

    fn write_host(&mut self, id: &str, f: impl FnOnce(&mut Value)) {
        let mut config = self.store.config.clone();
        let Some(h) = config
            .get_mut("remoteHosts")
            .and_then(Value::as_array_mut)
            .and_then(|l| {
                l.iter_mut()
                    .find(|h| h.get("id").and_then(Value::as_str) == Some(id))
            })
        else {
            return;
        };
        f(h);
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
                    self.st.layout = if value == "tabs" {
                        crate::state::LayoutMode::Tabs
                    } else {
                        crate::state::LayoutMode::Grid
                    };
                    if value == "tabs" {
                        self.st.focused = None;
                    }
                }
                self.write_defaults(|d| d[key.as_str()] = value);
            }
            SetAct::Notify(key, on) => self.write_defaults(|d| {
                let mut n =
                    json!({ "enabled": false, "onWaiting": true, "onError": true, "onBell": true });
                if let Some(Value::Object(old)) = d.get("notifications") {
                    for (k, v) in old {
                        n[k] = v.clone();
                    }
                }
                n[key.as_str()] = json!(on);
                d["notifications"] = n;
            }),
            SetAct::FontSize(delta) => {
                let cur = self
                    .store
                    .config
                    .pointer("/defaults/fontSize")
                    .and_then(Value::as_i64)
                    .unwrap_or(13);
                let next = (cur + i64::from(delta)).clamp(8, 24);
                self.write_defaults(|d| d["fontSize"] = json!(next));
            }
            SetAct::TestSound => crate::system::bell(),
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
                self.st.settings.cli_busy = true;
                self.st.settings.cli_error = crate::system::install_cli().err();
                self.st.settings.cli = Some(crate::system::cli_status());
                self.st.settings.cli_busy = false;
            }
            SetAct::CheckUpdates => {}
            SetAct::Rescan => self.rescan(),
            SetAct::ToggleWorktree(p) => {
                let w = &mut self.st.settings.worktrees;
                w.confirming = None;
                if !w.selected.remove(&p) {
                    w.selected.insert(p);
                }
            }
            SetAct::SelectSuggested => {
                let w = &mut self.st.settings.worktrees;
                w.confirming = None;
                w.selected = w.suggested();
            }
            SetAct::AskReclaim | SetAct::AskRemove => {
                let w = &mut self.st.settings.worktrees;
                w.confirming = if w.confirming.as_ref() == Some(&a) {
                    None
                } else {
                    Some(a)
                };
            }
            SetAct::ClearSelection => {
                let w = &mut self.st.settings.worktrees;
                w.selected.clear();
                w.confirming = None;
            }
            SetAct::Reclaim => {
                let w = &mut self.st.settings.worktrees;
                if w.busy {
                    return;
                }
                let paths = w.reclaim_targets();
                w.busy = true;
                w.confirming = None;
                self.call(
                    Call::Set(SetCall::Batch("Reclaimed")),
                    "worktree:reclaimArtifacts",
                    json!({ "paths": paths }),
                );
            }
            SetAct::Remove => self.remove_worktrees(),
            SetAct::ExpandBranches(p) => {
                let e = &mut self.st.settings.worktrees.expanded;
                if !e.remove(&p) {
                    e.insert(p);
                }
            }
            SetAct::ToggleBranch(p, b) => {
                let s = &mut self.st.settings.worktrees.branches;
                let k = (p, b);
                if !s.remove(&k) {
                    s.insert(k);
                }
            }
            SetAct::SelectMerged(p) => {
                let w = &mut self.st.settings.worktrees;
                let merged: Vec<String> = w
                    .projects
                    .iter()
                    .flatten()
                    .filter(|x| x.path == p)
                    .flat_map(|x| x.stale.iter().filter(|b| b.merged).map(|b| b.name.clone()))
                    .collect();
                for b in merged {
                    w.branches.insert((p.clone(), b));
                }
            }
            SetAct::DeleteBranches(p) => {
                let w = &mut self.st.settings.worktrees;
                let names: Vec<String> = w
                    .branches
                    .iter()
                    .filter(|(proj, _)| *proj == p)
                    .map(|(_, b)| b.clone())
                    .collect();
                if names.is_empty() || w.busy {
                    return;
                }
                w.busy = true;
                self.call(
                    Call::Set(SetCall::Branches),
                    "git:deleteBranches",
                    json!({ "projectPath": p, "branches": names, "force": false }),
                );
            }
            SetAct::Redetect => {
                self.st.settings.detecting = true;
                self.call(
                    Call::Set(SetCall::Detected),
                    "agent:detectInstalled",
                    Value::Null,
                );
            }
            SetAct::ResetAgent(k) => {
                let Some(a) = settings::agent(&k) else { return };
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
                if let Some(a) = settings::agent(&k) {
                    ui.clipboard().set(a.install);
                    self.st.settings.copied = Some((k, std::time::Instant::now()));
                }
            }
            SetAct::Copy(text) => {
                ui.clipboard().set(&text);
                self.st.settings.copied = Some((text, std::time::Instant::now()));
            }
            SetAct::AgentText(..) | SetAct::HostText(..) | SetAct::KeyText(_) => {}
            SetAct::AddHost => {
                let id = new_id();
                let mut config = self.store.config.clone();
                if !config.get("remoteHosts").is_some_and(Value::is_array) {
                    config["remoteHosts"] = json!([]);
                }
                if let Some(l) = config["remoteHosts"].as_array_mut() {
                    l.push(json!({ "id": id, "label": "New Host", "hostname": "", "user": "", "port": 22, "authMethod": "agent" }));
                }
                self.st.settings.ssh.hosts_open.insert(id);
                self.save_config(config, Call::Quiet);
            }
            SetAct::ToggleHost(id) => {
                let o = &mut self.st.settings.ssh.hosts_open;
                if !o.remove(&id) {
                    o.insert(id);
                }
            }
            SetAct::Auth(id, method) => {
                self.write_host(&id, |h| {
                    h["authMethod"] = json!(method);
                    if let Some(o) = h.as_object_mut() {
                        if method != "key-file" {
                            o.remove("sshKeyPath");
                        }
                        if method != "key-stored" {
                            o.remove("credentialId");
                        }
                        if method != "password" {
                            o.remove("encryptedPassword");
                        }
                    }
                });
                if method != "password" {
                    ui.set_edit_text(host_id(&id, HostField::Password), "");
                }
            }
            SetAct::PickKey(host, key) => {
                self.write_host(&host, |h| h["credentialId"] = json!(key))
            }
            SetAct::Test(id) => {
                let host = self
                    .store
                    .config
                    .get("remoteHosts")
                    .and_then(Value::as_array)
                    .and_then(|l| {
                        l.iter()
                            .find(|h| h.get("id").and_then(Value::as_str) == Some(id.as_str()))
                    })
                    .cloned();
                let Some(host) = host else { return };
                let s = |k: &str| host.get(k).and_then(Value::as_str).unwrap_or("").is_empty();
                if s("hostname") || s("user") {
                    self.st.settings.ssh.tests.insert(
                        id,
                        TestResult::Done {
                            ok: false,
                            message: "Hostname and username are required".into(),
                        },
                    );
                    return;
                }
                self.st
                    .settings
                    .ssh
                    .tests
                    .insert(id.clone(), TestResult::Testing);
                self.call(Call::Set(SetCall::Test(id)), "ssh:testConnection", host);
            }
            SetAct::AskDelete(id) => self.st.settings.ssh.confirm = Some(id),
            SetAct::CancelDelete => self.st.settings.ssh.confirm = None,
            SetAct::DeleteHost(id) => {
                let mut config = self.store.config.clone();
                if let Some(l) = config.get_mut("remoteHosts").and_then(Value::as_array_mut) {
                    l.retain(|h| h.get("id").and_then(Value::as_str) != Some(id.as_str()));
                }
                self.st.settings.ssh.confirm = None;
                self.save_config(config, Call::Quiet);
            }
            SetAct::ToggleKey(id) => {
                let o = &mut self.st.settings.ssh.keys_open;
                if !o.remove(&id) {
                    o.insert(id);
                }
            }
            SetAct::DeleteKey(id) => {
                self.st.settings.ssh.confirm = None;
                self.call(
                    Call::Set(SetCall::KeySaved),
                    "credential:deleteKey",
                    json!(id),
                );
            }
            SetAct::PasteKey => {
                self.st.settings.ssh.adding = true;
                ui.set_focus(Some(key_field_id(KeyField::Label)));
            }
            SetAct::CancelKey => {
                self.st.settings.ssh.adding = false;
                for f in [
                    KeyField::Label,
                    KeyField::Private,
                    KeyField::Public,
                    KeyField::Certificate,
                ] {
                    ui.set_edit_text(key_field_id(f), "");
                }
            }
            SetAct::SaveKey => {
                let t = |ui: &Ui, f| ui.edit_text(key_field_id(f)).trim().to_owned();
                let private = t(ui, KeyField::Private);
                if private.is_empty() {
                    return;
                }
                let label = t(ui, KeyField::Label);
                let mut params = json!({
                    "label": if label.is_empty() { "Untitled Key".to_owned() } else { label },
                    "privateKey": private,
                });
                for (f, k) in [
                    (KeyField::Public, "publicKey"),
                    (KeyField::Certificate, "certificate"),
                ] {
                    let v = t(ui, f);
                    if !v.is_empty() {
                        params[k] = json!(v);
                    }
                }
                self.call(Call::Set(SetCall::KeySaved), "credential:storeKey", params);
                self.run_set(ui, SetAct::CancelKey);
            }
            SetAct::ImportKey => {
                self.st.settings.ssh.importing = true;
                ui.set_edit_text(key_field_id(KeyField::Import), "");
                ui.set_focus(Some(key_field_id(KeyField::Import)));
            }
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
        }
    }

    fn remove_worktrees(&mut self) {
        let w = &mut self.st.settings.worktrees;
        if w.busy {
            return;
        }
        let chosen = w.chosen();
        let items: Vec<Value> = chosen
            .iter()
            .filter(|e| !e.orphan)
            .map(|e| {
                json!({ "projectPath": e.project_path, "worktreePath": e.path, "force": e.dirty, "deleteBranch": e.merged })
            })
            .collect();
        let orphans: Vec<String> = chosen
            .iter()
            .filter(|e| e.orphan)
            .map(|e| e.path.clone())
            .collect();
        w.busy = true;
        w.confirming = None;
        if !items.is_empty() {
            self.call(
                Call::Set(SetCall::Batch("Removed")),
                "worktree:removeMany",
                json!({ "items": items }),
            );
        }
        if !orphans.is_empty() {
            self.call(
                Call::Set(SetCall::Batch("Removed")),
                "worktree:pruneOrphans",
                json!({ "paths": orphans }),
            );
        }
    }

    /// Text typed in a settings field, and the sliders' drags.
    pub(super) fn settings_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        match *event {
            UiEvent::Move { id, x, .. } | UiEvent::Drop { id, x, .. }
                if id == ROW_SLIDER || id == VOLUME_SLIDER =>
            {
                let Some(r) = ui.rect_of(id) else { return true };
                if id == ROW_SLIDER {
                    let f = ((x - r.x - 8.0) / (r.w - 16.0)).clamp(0.0, 1.0);
                    self.st.settings.row_height = (100.0 + f * 500.0).round();
                } else {
                    let f = ((x - r.x) / r.w).clamp(0.0, 1.0);
                    let v = (f * 100.0).round() / 100.0;
                    self.write_defaults(|d| {
                        if !d.get("notifications").is_some_and(Value::is_object) {
                            d["notifications"] = json!({});
                        }
                        d["notifications"]["soundVolume"] = json!(v);
                    });
                }
                true
            }
            UiEvent::Changed(id) => {
                let Some(Act::Set(a)) = self.acts.text_of(id) else {
                    return false;
                };
                let text = ui.edit_text(id).to_owned();
                match a {
                    SetAct::AgentText(agent, f) => match f {
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
                    },
                    SetAct::HostText(host, f) => {
                        let (key, value) = match f {
                            HostField::Label => ("label", json!(text)),
                            HostField::Hostname => ("hostname", json!(text)),
                            HostField::User => ("user", json!(text)),
                            HostField::Port => (
                                "port",
                                json!(text
                                    .trim()
                                    .parse::<i64>()
                                    .ok()
                                    .filter(|p| *p != 0)
                                    .unwrap_or(22)),
                            ),
                            HostField::KeyPath => ("sshKeyPath", json!(text)),
                            HostField::Options => ("sshOptions", json!(text)),
                            HostField::Password => return true,
                        };
                        self.write_host(&host, |h| h[key] = value);
                    }
                    _ => {}
                }
                true
            }
            UiEvent::Submit(id) => {
                if id == SHELL_PATH {
                    self.commit_shell(ui);
                    return true;
                }
                if id == key_field_id(KeyField::Import) {
                    let path = ui.edit_text(id).trim().to_owned();
                    self.st.settings.ssh.importing = false;
                    ui.set_focus(None);
                    if !path.is_empty() {
                        self.import_key(&path);
                    }
                    return true;
                }
                if let Some((host, _)) = self.password_field(id) {
                    self.commit_password(ui, &host);
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

    fn password_field(&self, id: Id) -> Option<(String, ())> {
        self.store
            .config
            .get("remoteHosts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|h| h.get("id").and_then(Value::as_str))
            .find(|h| host_id(h, HostField::Password) == id)
            .map(|h| (h.to_owned(), ()))
    }

    fn commit_password(&mut self, ui: &mut Ui, host: &str) {
        let id = host_id(host, HostField::Password);
        let pw = ui.edit_text(id).to_owned();
        if pw.is_empty() {
            return;
        }
        ui.set_edit_text(id, "");
        self.write_host(host, |h| h["password"] = json!(pw));
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

    /// `CREDENTIAL_IMPORT_KEY_FILE`: the key and the `.pub` beside it.
    fn import_key(&mut self, path: &str) {
        let expanded = crate::system::expand_home(path);
        let private = match std::fs::read_to_string(&expanded) {
            Ok(t) => t,
            Err(e) => return self.st.toast(format!("{path}: {e}"), true),
        };
        let public = std::fs::read_to_string(format!("{}.pub", expanded.display()))
            .ok()
            .map(|p| p.trim().to_owned());
        let label = crate::state::basename(path).to_owned();
        let mut params = json!({ "label": label, "privateKey": private });
        if let Some(p) = public {
            params["publicKey"] = json!(p);
        }
        self.call(Call::Set(SetCall::KeySaved), "credential:storeKey", params);
    }

    /// A field lost focus: what `onBlur` does there.
    pub(super) fn settings_blur(&mut self, ui: &mut Ui, id: Id) {
        if id == SHELL_PATH && self.st.settings.shell_editing {
            self.commit_shell(ui);
        } else if let Some((host, _)) = self.password_field(id) {
            self.commit_password(ui, &host);
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
        if code == "Comma" && held == MOD {
            let cat = self.st.settings.category;
            self.open_settings(cat);
            return true;
        }
        if !self.st.settings.open || code != "Escape" || self.st.menu.is_some() {
            return false;
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

/// A fresh id for a host, as `crypto.randomUUID` gives one.
fn new_id() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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
    std::thread::spawn(move || {
        let _ = tx.send(crate::system::http_get_json(port, "/vornd/health"));
    });
    rx
}
