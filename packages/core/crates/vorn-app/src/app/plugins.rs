//! What the Extensions and Connections pages' controls do, and the answers to their calls.

use serde_json::{json, Value};

use super::{tasks::open_url, Act, App, Call, Src};
use crate::client::RpcError;
use crate::plugins::{
    self as plug, failure, field_id, kinds_id, pick_id, tabs_id, Activity, BuiltinForm, Field,
    Form, Origin, Pending, Pick, PlugAct, ProfileForm, Progress, Rung, SdkForm, Tab, Verb,
};
use crate::runs::s;
use crate::settings::Category;
use crate::settings::SetAct;
use crate::state::{Dialog, MenuKind};
use crate::templates::Fix;
use crate::ui::{widgets, Event as UiEvent, Ui};
use crate::workflows::WfAct;

/// A Plugins call whose answer changes the screen.
pub(super) enum PlugCall {
    Builtins,
    Connections,
    Statuses,
    /// `extension:installed`.
    Packs,
    Catalog {
        load: bool,
    },
    CatalogRefresh,
    Inspect {
        key: String,
        direct: bool,
    },
    /// The install dialog's repository or folder, read for its sheet.
    ReadSource(Origin),
    Install {
        id: String,
    },
    /// An extension from Git or a folder, installed or linked once its id was typed.
    ExtInstall {
        linked: bool,
    },
    Sandbox,
    Remove(String),
    Rollback(String),
    /// A pack's manifest, read by starting it from this launch line.
    Probe((String, Vec<String>)),
    Identity,
    Detect(String),
    Create {
        browser: bool,
    },
    Profile,
    Preflight,
    Backfill(String),
    Test(String),
    Delete(String),
    Run(String),
    Reseed,
    SignIn(String),
    SignOut(String),
}

fn arr(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// A `{ok:false,error}` answer as the error it carries.
fn refused(v: &Value) -> Option<String> {
    (v.get("ok") == Some(&Value::Bool(false))).then(|| failure(s(v, "error")))
}

impl App {
    pub(super) fn run_plug(&mut self, ui: &mut Ui, act: PlugAct) {
        let p = &mut self.st.plugins;
        match act {
            PlugAct::Refresh => {
                p.tab = Tab::Installed;
                p.selected = None;
                p.source = None;
                p.adding = None;
                p.pending = None;
                p.report = None;
                p.from_editor = false;
                self.plug_load(true);
            }
            PlugAct::Tab(tab) => {
                p.tab = tab;
                p.selected = None;
                p.adding = None;
            }
            PlugAct::Select(key) => p.selected = key,
            PlugAct::Add(key) => self.plug_open(&key),
            PlugAct::CloseForm if p.from_editor => self.plug_leave(),
            PlugAct::CloseForm => p.adding = None,
            // The editor's panel installs a listed pack outright; the page shows its sheet first.
            PlugAct::Install(key) => {
                let direct = p.from_editor;
                self.plug_inspect(&key, direct);
            }
            PlugAct::InstallMenu => {
                self.plug_show_page(ui);
                self.st.plugins.menu_soon = true;
            }
            PlugAct::InstallFrom(origin) => {
                self.plug_show_page(ui);
                let p = &mut self.st.plugins;
                p.pending = None;
                p.report = None;
                p.source = None;
                // A browser cannot hand vornd a folder on its machine, so it is typed there too.
                if origin == Origin::Git || cfg!(target_arch = "wasm32") {
                    self.plug_source(ui, origin, "");
                } else {
                    let title = origin.title().to_owned();
                    self.start_pick(
                        super::Picked::ExtFolder(origin),
                        crate::system::Pick::Folder(title),
                    );
                }
            }
            PlugAct::ReadSource => {
                let Some(g) = p.source.as_mut().filter(|g| !g.busy) else {
                    return;
                };
                let url = g.url.trim().to_owned();
                if url.is_empty() {
                    return;
                }
                g.busy = true;
                g.error = None;
                let origin = g.origin;
                self.call(
                    Call::Plug(PlugCall::ReadSource(origin)),
                    "extension:inspect",
                    json!({ "url": url, "link": origin == Origin::Link }),
                );
            }
            PlugAct::ConfirmInstall => {
                let Some(pending) = p.pending.clone() else {
                    return;
                };
                // The page asks for the id typed; the editor's inline sheet does not.
                if p.installing || !p.from_editor && !pending.confirmed() {
                    return;
                }
                if let Some(path) = &pending.link {
                    p.installing = true;
                    p.report = None;
                    self.call(
                        Call::Plug(PlugCall::ExtInstall { linked: true }),
                        "extension:link",
                        json!({ "path": path }),
                    );
                } else if pending.git {
                    p.installing = true;
                    p.report = None;
                    let token = s(&pending.preview, "token").unwrap_or("").to_owned();
                    self.call(
                        Call::Plug(PlugCall::ExtInstall { linked: false }),
                        "extension:install",
                        json!({ "token": token, "confirm": pending.typed.trim() }),
                    );
                } else {
                    self.plug_keep(pending);
                }
            }
            PlugAct::CancelInstall => {
                if !p.installing {
                    self.plug_close_sheet();
                }
            }
            PlugAct::Remove(id) => {
                if self.plug_start(Verb::Remove, &id) {
                    self.st.plugins.selected = None;
                    self.call(
                        Call::Plug(PlugCall::Remove(id.clone())),
                        "extension:remove",
                        json!({ "id": id }),
                    );
                }
            }
            PlugAct::Sandbox(id, on) => {
                if let Some(pack) = p.packs.iter_mut().find(|k| s(k, "id") == Some(id.as_str())) {
                    pack["sandboxed"] = json!(on);
                }
                p.rebuild();
                self.call(
                    Call::Plug(PlugCall::Sandbox),
                    "extension:setSandbox",
                    json!({ "id": id, "on": on }),
                );
            }
            PlugAct::ShowFolder(path) => super::tasks::open_folder(&path),
            PlugAct::AddConnection => {
                self.run(
                    ui,
                    Act::Set(SetAct::Open(Category::Connectors)),
                    Src::default(),
                );
                let p = &mut self.st.plugins;
                p.tab = Tab::Browse;
                p.kind = Some(plug::Kind::Connector);
                p.selected = None;
                p.adding = None;
            }
            PlugAct::Rollback(id) => {
                if self.plug_start(Verb::Rollback, &id) {
                    self.call(
                        Call::Plug(PlugCall::Rollback(id.clone())),
                        "connector:rollbackPack",
                        json!(id),
                    );
                }
            }
            PlugAct::CatalogRefresh => {
                if !p.refreshing {
                    p.refreshing = true;
                    self.call(
                        Call::Plug(PlugCall::CatalogRefresh),
                        "connector:catalogRefresh",
                        Value::Null,
                    );
                }
            }
            PlugAct::Text(_) => {}
            PlugAct::Pick(_) => {}
            PlugAct::McpManual(manual) => {
                if let Some(a) = p.adding.as_mut().filter(|a| a.mcp_tabs) {
                    let on_manual = matches!(a.form, Form::Builtin(_));
                    if manual != on_manual {
                        a.error = None;
                        a.form = if manual {
                            let mut f = BuiltinForm {
                                connector: "mcp".to_owned(),
                                detected: Some(None),
                                ..BuiltinForm::default()
                            };
                            f.values.clear();
                            Form::Builtin(f)
                        } else {
                            Form::Sdk(SdkForm::default())
                        };
                    }
                }
            }
            PlugAct::Probe => {
                let spec = match p.adding.as_ref().map(|a| &a.form) {
                    Some(Form::Sdk(f)) if !f.probing && !f.spec.trim().is_empty() => f.spec.clone(),
                    _ => return,
                };
                self.plug_probe(plug::parse_launch_spec(&spec));
            }
            PlugAct::UseToken => {
                if let Some(Form::Sdk(f)) = p.adding.as_mut().map(|a| &mut a.form) {
                    if let Some(m) = &f.manifest {
                        for name in plug::borrowable(m) {
                            f.values.remove(&name);
                        }
                    }
                    f.use_token = !f.use_token;
                }
            }
            PlugAct::Save => self.plug_save(ui),
            PlugAct::Backfill(id) => {
                self.st.plugins.backfilled.remove(&id);
                if self.plug_start(Verb::Backfill, &id) {
                    self.call(
                        Call::Plug(PlugCall::Backfill(id.clone())),
                        "connection:backfill",
                        json!({ "connectionId": id }),
                    );
                }
            }
            PlugAct::Test(id) => {
                if p.testing.is_none() {
                    p.tested.remove(&id);
                    p.testing = Some(id.clone());
                    self.call(
                        Call::Plug(PlugCall::Test(id.clone())),
                        "connection:preflight",
                        json!(id),
                    );
                }
            }
            PlugAct::Delete(id) => {
                if self.plug_start(Verb::Delete, &id) {
                    self.call(
                        Call::Plug(PlugCall::Delete(id.clone())),
                        "connection:delete",
                        json!(id),
                    );
                }
            }
            PlugAct::RunWorkflow(wf, _conn) => {
                if self.plug_start(Verb::Run, &wf) {
                    self.call(
                        Call::Plug(PlugCall::Run(wf.clone())),
                        "workflow:runManual",
                        json!({ "workflowId": wf }),
                    );
                }
            }
            PlugAct::Reseed(conn, event) => self.call(
                Call::Plug(PlugCall::Reseed),
                "connector:seedWorkflow",
                json!({ "connectionId": conn, "event": event }),
            ),
            PlugAct::SignIn(id) => {
                if self.plug_start(Verb::SignIn, &id) {
                    self.call(
                        Call::Plug(PlugCall::SignIn(id.clone())),
                        "connection:browserAuth",
                        json!(id),
                    );
                }
            }
            PlugAct::SignOut(id) => {
                if self.plug_start(Verb::SignOut, &id) {
                    self.forget_profile(&id);
                    self.call(
                        Call::Plug(PlugCall::SignOut(id.clone())),
                        "connection:signedOut",
                        json!(id),
                    );
                }
            }
            PlugAct::OpenUrl(url) => open_url(&url),
            PlugAct::Use => {
                self.run(ui, Act::Set(SetAct::Close), Src::default());
                self.run(ui, Act::Workflow(WfAct::New), Src::default());
            }
            PlugAct::OpenWorkflow(id) => {
                self.run(ui, Act::Set(SetAct::Close), Src::default());
                self.run(ui, Act::Workflow(WfAct::Edit(id)), Src::default());
            }
            PlugAct::Fix(fix) => {
                p.intent = Some(fix);
                if p.loads == 0 && p.loaded {
                    self.plug_apply_intent();
                }
            }
        }
    }

    /// Re-read what the page shows; the catalog only when it opens.
    fn plug_load(&mut self, catalog: bool) {
        let p = &mut self.st.plugins;
        p.loads += 4 + u8::from(catalog);
        self.st.ext.loaded = false;
        self.call(
            Call::Plug(PlugCall::Builtins),
            "connector:list",
            Value::Null,
        );
        self.call(
            Call::Plug(PlugCall::Connections),
            "connection:list",
            json!({}),
        );
        self.call(
            Call::Plug(PlugCall::Statuses),
            "connector:status",
            Value::Null,
        );
        self.call(
            Call::Plug(PlugCall::Packs),
            "extension:installed",
            Value::Null,
        );
        if catalog {
            self.call(
                Call::Plug(PlugCall::Catalog { load: true }),
                "connector:catalog",
                Value::Null,
            );
        }
    }

    /// Marks a row busy; false when the same row is already at it.
    fn plug_start(&mut self, verb: Verb, id: &str) -> bool {
        let p = &mut self.st.plugins;
        if p.busy(id) {
            return false;
        }
        p.activity.insert(id.to_owned(), Activity::Busy(verb));
        true
    }

    /// Ends a row's activity, keeping the reason when it failed.
    fn plug_finish(&mut self, id: &str, error: Option<String>) {
        let p = &mut self.st.plugins;
        match error {
            Some(e) => p.activity.insert(id.to_owned(), Activity::Failed(e)),
            None => p.activity.remove(id),
        };
    }

    fn plug_open(&mut self, key: &str) {
        let project = self.store.projects.first().map(|p| p.name.clone());
        let p = &mut self.st.plugins;
        let Some(l) = p.listing(key).cloned() else {
            return;
        };
        let adding = p.adding_for(&l, project);
        p.adding = Some(adding);
        p.selected = None;
        self.plug_kick();
    }

    /// Starts what a fresh form reads on its own: a pack's manifest, a repo.
    fn plug_kick(&mut self) {
        let p = &self.st.plugins;
        let Some(a) = &p.adding else { return };
        match &a.form {
            Form::Sdk(f) if f.manifest.is_none() && !f.probing => {
                if let Some(launch) = f.listing.as_deref().and_then(|k| p.launch_for(k)) {
                    self.plug_probe(launch);
                }
            }
            Form::Builtin(f) if f.detected.is_none() => {
                let path = a
                    .project
                    .as_deref()
                    .and_then(|n| self.store.projects.iter().find(|p| p.name == n))
                    .map(|p| p.path.clone());
                match path {
                    Some(path) => self.call(
                        Call::Plug(PlugCall::Detect(a.project.clone().unwrap_or_default())),
                        "connector:detectRepo",
                        json!(path),
                    ),
                    None => {
                        if let Some(Form::Builtin(f)) =
                            self.st.plugins.adding.as_mut().map(|a| &mut a.form)
                        {
                            f.detected = Some(None);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn plug_probe(&mut self, launch: (String, Vec<String>)) {
        let Some(a) = self.st.plugins.adding.as_mut() else {
            return;
        };
        a.error = None;
        let Form::Sdk(f) = &mut a.form else { return };
        f.manifest = None;
        f.identity = None;
        f.probing = true;
        let params = json!({ "command": launch.0, "args": launch.1 });
        self.call(
            Call::Plug(PlugCall::Probe(launch)),
            "connector:probeSdk",
            params,
        );
    }

    /// `inspect`: stage the pack, then install it or show the sheet.
    fn plug_inspect(&mut self, key: &str, direct: bool) {
        let p = &mut self.st.plugins;
        p.report = None;
        p.pending = None;
        let Some(l) = p.listing(key) else { return };
        let id = l.id.clone();
        let Some(catalog) = l
            .catalog
            .as_ref()
            .filter(|c| crate::plugins::installable(c))
        else {
            p.progress.remove(&id);
            return;
        };
        let mut source = json!({ "kind": "git", "url": s(catalog, "git") });
        for key in ["path", "branch"] {
            if let Some(v) = s(catalog, key).filter(|v| !v.is_empty()) {
                source[key] = json!(v);
            }
        }
        p.progress.insert(id, Progress::phase("checking"));
        self.call(
            Call::Plug(PlugCall::Inspect {
                key: key.to_owned(),
                direct,
            }),
            "connector:inspectPack",
            source,
        );
    }

    /// `keep`: install what was staged.
    fn plug_keep(&mut self, pending: Pending) {
        let p = &mut self.st.plugins;
        // A pack from a file names itself; a listed one goes by its row.
        let Some(id) = p
            .listing(&pending.key)
            .map(|l| l.id.clone())
            .or_else(|| s(&pending.preview, "id").map(str::to_owned))
        else {
            p.pending = None;
            return;
        };
        let token = pending.preview.get("token").cloned().unwrap_or(Value::Null);
        p.installing = true;
        p.pending = Some(pending);
        self.call(
            Call::Plug(PlugCall::Install { id }),
            "connector:installPack",
            json!({ "kind": "staged", "token": token }),
        );
    }

    fn plug_save(&mut self, _ui: &mut Ui) {
        let projects: Vec<String> = self.store.projects.iter().map(|p| p.name.clone()).collect();
        let p = &mut self.st.plugins;
        let Some(a) = p.adding.as_ref() else { return };
        if a.saving {
            return;
        }
        let project = a.project.clone().or_else(|| projects.first().cloned());
        let (call, method, params) = match &a.form {
            Form::Builtin(f) => {
                let builtin = p.builtin(&f.connector);
                let manifest = builtin.and_then(|b| b.get("manifest"));
                if !f.can_save(manifest) {
                    return;
                }
                let name = builtin.and_then(|b| s(b, "name")).unwrap_or(&f.connector);
                let params = f.create_params(name, manifest, project.as_deref());
                (
                    PlugCall::Create { browser: false },
                    "connection:create",
                    params,
                )
            }
            Form::Sdk(f) => {
                if f.rung() == Some(Rung::None) {
                    self.plug_done();
                    return;
                }
                if f.missing() {
                    return;
                }
                let Some(params) = f.create_params(project.as_deref()) else {
                    return;
                };
                let browser = f.rung() == Some(Rung::Browser);
                (PlugCall::Create { browser }, "connection:create", params)
            }
            Form::Profile(f) => {
                if !f.ready() {
                    return;
                }
                let named = match f.name.trim() {
                    "" => a.name.clone(),
                    n => n.to_owned(),
                };
                match &f.made {
                    Some(id) => (
                        PlugCall::Profile,
                        "connection:update",
                        json!({ "id": id, "updates": { "name": named, "filters": f.filters() } }),
                    ),
                    None => (
                        PlugCall::Profile,
                        "connection:create",
                        json!({
                            "connectorId": "http",
                            "name": named,
                            "filters": f.filters(),
                            "syncIntervalMinutes": 5,
                            "statusMapping": {},
                        }),
                    ),
                }
            }
            Form::NoMcp => return,
        };
        if let Some(a) = p.adding.as_mut() {
            a.saving = true;
            a.error = None;
            if let Form::Profile(f) = &mut a.form {
                f.result = None;
            }
        }
        self.call(Call::Plug(call), method, params);
    }

    /// A form finished: back to what is installed, re-read; the connection
    /// shows on Settings › Connections.
    fn plug_done(&mut self) {
        if self.st.plugins.from_editor {
            self.plug_leave();
            return;
        }
        let p = &mut self.st.plugins;
        p.adding = None;
        p.selected = None;
        self.open_settings(Category::Connections);
        // The page reads its rows again, with the connection just made.
        self.st.points.shown.clear();
        self.plug_load(false);
    }

    /// Opens Settings › Extensions unless it shows.
    fn plug_show_page(&mut self, ui: &mut Ui) {
        if !self.st.shows_plugins() {
            self.run(
                ui,
                Act::Set(SetAct::Open(Category::Connectors)),
                Src::default(),
            );
        }
    }

    /// Opens the install dialog on `origin`, its field holding `url`.
    fn plug_source(&mut self, ui: &mut Ui, origin: Origin, url: &str) {
        let p = &mut self.st.plugins;
        p.pending = None;
        p.report = None;
        p.source = Some(plug::SourceDialog {
            origin,
            url: url.to_owned(),
            ..plug::SourceDialog::default()
        });
        self.st.dialog = Some(Dialog::ExtInstall);
        let id = field_id(&Field::Source);
        ui.set_edit_text(id, url);
        ui.set_focus(Some(id));
    }

    /// The folder picker chose `path` to install from or link: its sheet is read at once.
    pub(super) fn plug_picked(&mut self, ui: &mut Ui, origin: Origin, path: &str) {
        self.plug_source(ui, origin, path);
        self.run_plug(ui, PlugAct::ReadSource);
    }

    /// Puts the install dialog away, and what it was showing.
    fn plug_close_sheet(&mut self) {
        let p = &mut self.st.plugins;
        if let Some(id) = p
            .pending
            .as_ref()
            .and_then(|pd| p.listing(&pd.key))
            .map(|l| l.id.clone())
        {
            p.progress.remove(&id);
        }
        p.pending = None;
        p.source = None;
        p.report = None;
        p.installing = false;
        if self.st.dialog == Some(Dialog::ExtInstall) {
            self.st.dialog = None;
        }
    }

    /// Leaves the page; a workflow editor under it re-reads what its requirements check.
    pub(super) fn plug_leave(&mut self) {
        let p = &mut self.st.plugins;
        p.from_editor = false;
        p.adding = None;
        p.pending = None;
        if self.st.wf.editor.is_some() {
            self.load_library();
        }
    }

    fn plug_apply_intent(&mut self) {
        let Some(fix) = self.st.plugins.intent.take() else {
            return;
        };
        let p = &mut self.st.plugins;
        match fix {
            Fix::None => {}
            Fix::Install { id, name } => {
                p.tab = Tab::Browse;
                p.adding = None;
                match p.listing_by_id(&id).map(|l| l.key.clone()) {
                    Some(key) => {
                        p.selected = Some(key.clone());
                        self.plug_inspect(&key, false);
                    }
                    None => self
                        .st
                        .toast(format!("{name} is not in the connector catalog"), true),
                }
            }
            Fix::AddConnection { id, name } => match p.listing_by_id(&id).map(|l| l.key.clone()) {
                Some(key) => self.plug_open(&key),
                None => self
                    .st
                    .toast(format!("{name} is not in the connector catalog"), true),
            },
            Fix::CreateProfile(name) => {
                p.selected = None;
                p.adding = Some(plug::Adding {
                    key: "http".to_owned(),
                    name: "HTTP profile".to_owned(),
                    form: Form::Profile(ProfileForm::new(&name)),
                    mcp_tabs: false,
                    project: None,
                    saving: false,
                    error: None,
                });
            }
        }
    }

    /// One load answered; the last one settles the view and any fix.
    fn plug_loaded(&mut self) {
        let p = &mut self.st.plugins;
        p.loads = p.loads.saturating_sub(1);
        p.loaded = true;
        p.rebuild();
        if p.loads == 0 && p.intent.is_some() {
            self.plug_apply_intent();
        }
    }

    pub(super) fn plug_answered(&mut self, call: PlugCall, answer: Result<Value, RpcError>) {
        let p = &mut self.st.plugins;
        match call {
            PlugCall::Builtins => {
                p.builtins = answer.as_ref().map(arr).unwrap_or_default();
                self.plug_loaded();
            }
            PlugCall::Connections => {
                if let Ok(v) = &answer {
                    p.connections = arr(v);
                }
                self.plug_loaded();
            }
            PlugCall::Statuses => {
                if let Ok(v) = &answer {
                    p.statuses = arr(v);
                }
                self.plug_loaded();
            }
            PlugCall::Packs => {
                if let Ok(v) = &answer {
                    p.packs = arr(v);
                }
                for id in std::mem::take(&mut p.forget) {
                    p.progress.remove(&id);
                }
                self.plug_loaded();
            }
            PlugCall::Catalog { load } => {
                if let Ok(v) = &answer {
                    self.plug_catalog(v);
                }
                if load {
                    self.plug_loaded();
                }
            }
            PlugCall::CatalogRefresh => {
                p.refreshing = false;
                if let Ok(v) = &answer {
                    self.plug_catalog(v);
                }
            }
            PlugCall::ReadSource(origin) => {
                let Some(g) = p.source.as_mut().filter(|g| g.origin == origin) else {
                    return;
                };
                g.busy = false;
                match answer {
                    Err(e) => g.error = Some(e.to_string()),
                    Ok(sheet) => {
                        let key = s(&sheet, "id").unwrap_or("").to_owned();
                        let link = (origin == Origin::Link).then(|| {
                            sheet
                                .pointer("/source/linked")
                                .and_then(Value::as_str)
                                .unwrap_or(g.url.trim())
                                .to_owned()
                        });
                        p.pending = Some(Pending {
                            key,
                            preview: sheet,
                            git: true,
                            link,
                            ..Pending::default()
                        });
                        self.focus_confirm = true;
                    }
                }
            }
            PlugCall::ExtInstall { linked } => {
                p.installing = false;
                match answer {
                    Err(e) => p.report = Some(e.to_string()),
                    Ok(pack) => {
                        let name = s(&pack, "name").unwrap_or("The extension").to_owned();
                        self.plug_close_sheet();
                        let verb = if linked { "Linked" } else { "Installed" };
                        self.st.toast(format!("{verb} {name}"), false);
                        self.st.points.stale = true;
                        self.st.points.builtins_loaded = false;
                        self.plug_load(false);
                    }
                }
            }
            PlugCall::Sandbox => {
                if let Err(e) = answer {
                    self.st.toast(e.to_string(), true);
                }
                self.plug_load(false);
            }
            PlugCall::Inspect { key, direct } => {
                let Some(l) = p.listing(&key).cloned() else {
                    return;
                };
                let fail = |p: &mut plug::Plugins, error: String| {
                    // The editor's panel has no row to carry it, so it says it at the top.
                    if p.from_editor {
                        p.report = Some(error.clone());
                    }
                    p.progress.insert(
                        l.id.clone(),
                        Progress {
                            phase: "failed".into(),
                            percent: None,
                            error: Some(error),
                        },
                    );
                };
                match answer {
                    Err(e) => fail(p, e.to_string()),
                    Ok(v) => match refused(&v) {
                        Some(e) => fail(p, e),
                        None => {
                            let preview = v.get("preview").cloned().unwrap_or(Value::Null);
                            let pending = Pending {
                                key,
                                preview,
                                ..Pending::default()
                            };
                            if direct && plug::matches_listing(&pending.preview, &l) {
                                self.plug_keep(pending);
                            } else {
                                p.progress.remove(&l.id);
                                p.pending = Some(pending);
                                if !p.from_editor {
                                    self.st.dialog = Some(Dialog::ExtInstall);
                                    self.focus_confirm = true;
                                }
                            }
                        }
                    },
                }
            }
            PlugCall::Install { id } => {
                p.installing = false;
                let error = match &answer {
                    Ok(v) => refused(v),
                    Err(e) => Some(e.to_string()),
                };
                match error {
                    Some(e) => {
                        let progress = Progress {
                            phase: "failed".into(),
                            percent: None,
                            error: Some(e.clone()),
                        };
                        p.progress.insert(id, progress);
                        p.report = Some(e);
                        if p.from_editor {
                            p.pending = None;
                        }
                    }
                    None => {
                        self.plug_close_sheet();
                        let p = &mut self.st.plugins;
                        p.progress.insert(id.clone(), Progress::phase("installing"));
                        p.forget.push(id);
                        let editor = p.from_editor;
                        self.st.points.stale = true;
                        self.plug_load(false);
                        if editor && self.st.wf.editor.is_some() {
                            self.load_library();
                        }
                    }
                }
            }
            PlugCall::Remove(id) => {
                let error = match &answer {
                    Ok(v) => refused(v),
                    Err(e) => Some(e.to_string()),
                };
                if error.is_none() {
                    let n = answer
                        .as_ref()
                        .ok()
                        .and_then(|v| v.get("connections"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    if n > 0 {
                        p.report = Some(format!(
                            "Removed the files. {} will stop working until the connector is installed again.",
                            plug::count(n as usize, "connection")
                        ));
                    }
                }
                self.plug_finish(&id, error);
                self.st.points.stale = true;
                self.plug_load(false);
            }
            PlugCall::Rollback(id) => {
                let error = match &answer {
                    Ok(v) => refused(v),
                    Err(e) => Some(e.to_string()),
                };
                self.plug_finish(&id, error);
                self.plug_load(false);
            }
            PlugCall::Probe(launch) => {
                let Some(a) = p.adding.as_mut() else { return };
                let Form::Sdk(f) = &mut a.form else { return };
                if !f.probing {
                    return;
                }
                f.probing = false;
                let error = match &answer {
                    Ok(v) if v.get("ok") == Some(&Value::Bool(true)) => None,
                    Ok(v) => Some(failure(s(v, "error"))),
                    Err(e) => Some(e.to_string()),
                };
                if let Some(e) = error {
                    a.error = Some(e);
                    return;
                }
                let manifest = answer
                    .ok()
                    .and_then(|v| v.get("manifest").cloned())
                    .unwrap_or(Value::Null);
                f.values = manifest
                    .get("env")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| s(e, "name"))
                    .map(|n| (n.to_owned(), String::new()))
                    .collect();
                f.trigger = 0;
                f.launch = Some(launch);
                let cli = manifest.pointer("/auth/rung").and_then(Value::as_str) == Some("cli");
                let id = s(&manifest, "id").unwrap_or("").to_owned();
                f.manifest = Some(manifest);
                if cli {
                    self.call(
                        Call::Plug(PlugCall::Identity),
                        "connector:probeAuth",
                        json!(id),
                    );
                }
            }
            PlugCall::Identity => {
                if let Some(Form::Sdk(f)) = p.adding.as_mut().map(|a| &mut a.form) {
                    f.identity = Some(answer.unwrap_or_else(|_| json!({ "ok": null })));
                }
            }
            PlugCall::Detect(project) => {
                if let Some(a) = p
                    .adding
                    .as_mut()
                    .filter(|a| a.project.as_deref() == Some(&project))
                {
                    if let Form::Builtin(f) = &mut a.form {
                        let found = answer.ok().and_then(|v| {
                            Some((s(&v, "owner")?.to_owned(), s(&v, "repo")?.to_owned()))
                        });
                        f.detected = Some(found);
                    }
                }
            }
            PlugCall::Create { browser } => match answer {
                Ok(v) => {
                    if browser {
                        if let Some(id) = s(&v, "id") {
                            self.call(
                                Call::Plug(PlugCall::SignIn(id.to_owned())),
                                "connection:browserAuth",
                                json!(id),
                            );
                            self.st
                                .plugins
                                .activity
                                .insert(id.to_owned(), Activity::Busy(Verb::SignIn));
                        }
                    }
                    self.plug_done();
                }
                Err(e) => {
                    if let Some(a) = p.adding.as_mut() {
                        a.saving = false;
                        a.error = Some(e.to_string());
                    }
                }
            },
            PlugCall::Profile => {
                let Some(a) = p.adding.as_mut() else { return };
                let Form::Profile(f) = &mut a.form else {
                    return;
                };
                match answer {
                    Ok(v) => {
                        let id = s(&v, "id")
                            .map(str::to_owned)
                            .or_else(|| f.made.clone())
                            .unwrap_or_default();
                        f.made = Some(id.clone());
                        self.call(
                            Call::Plug(PlugCall::Preflight),
                            "connection:preflight",
                            json!(id),
                        );
                    }
                    Err(e) => {
                        a.saving = false;
                        a.error = Some(match e.to_string() {
                            m if m.trim().is_empty() => "The profile could not be saved".to_owned(),
                            m => m,
                        });
                    }
                }
            }
            PlugCall::Preflight => {
                let Some(a) = p.adding.as_mut() else { return };
                let Form::Profile(f) = &mut a.form else {
                    return;
                };
                a.saving = false;
                match answer {
                    Ok(v) if v.get("ok") != Some(&Value::Bool(false)) => {
                        let name = f.name.trim().to_owned();
                        self.st.toast(format!("Saved the {name} profile"), false);
                        self.plug_done();
                    }
                    Ok(v) => {
                        let message = s(&v, "message").unwrap_or("No answer to check.").to_owned();
                        f.result = Some((false, message));
                    }
                    Err(e) => f.result = Some((false, e.to_string())),
                }
            }
            PlugCall::Backfill(id) => {
                let result = match &answer {
                    Ok(v) => match s(v, "error") {
                        Some(e) => Err(e.to_owned()),
                        None => Ok((
                            v.get("imported").and_then(Value::as_u64).unwrap_or(0),
                            v.get("updated").and_then(Value::as_u64).unwrap_or(0),
                        )),
                    },
                    Err(e) => Err(e.to_string()),
                };
                let error = answer.err().map(|e| e.to_string());
                if error.is_none() {
                    p.backfilled.insert(id.clone(), result);
                }
                self.plug_finish(&id, error);
                self.plug_load(false);
            }
            PlugCall::Test(id) => {
                p.testing = None;
                let result = match answer {
                    Ok(v) => {
                        let ok = v.get("ok") != Some(&Value::Bool(false));
                        let fallback = if ok { "Reachable" } else { "Failed" };
                        (
                            ok,
                            s(&v, "message")
                                .filter(|m| !m.is_empty())
                                .unwrap_or(fallback)
                                .to_owned(),
                        )
                    }
                    Err(e) => (false, e.to_string()),
                };
                p.tested.insert(id, result);
            }
            PlugCall::Delete(id) => {
                let error = answer.err().map(|e| e.to_string());
                self.plug_finish(&id, error);
                self.plug_load(false);
            }
            PlugCall::Run(wf) => {
                let error = answer.err().map(|e| e.to_string());
                self.plug_finish(&wf, error);
                self.plug_load(false);
            }
            PlugCall::Reseed => {
                if let Err(e) = answer {
                    self.st.toast(e.to_string(), true);
                }
                self.plug_load(false);
            }
            // The row stays busy until the sheet closes, as the desktop's window did.
            PlugCall::SignIn(id) => {
                let error = match &answer {
                    Ok(v) => self.open_signin(&id, v, crate::signin::From::Plugins),
                    Err(e) => Some(e.to_string()),
                };
                if error.is_some() {
                    self.plug_finish(&id, error);
                }
            }
            PlugCall::SignOut(id) => {
                let error = answer.err().map(|e| e.to_string());
                self.plug_finish(&id, error);
                self.plug_load(false);
            }
        }
    }

    /// A `connector:catalog` snapshot: items, MCP servers and when it was fetched.
    fn plug_catalog(&mut self, v: &Value) {
        let p = &mut self.st.plugins;
        p.catalog = v.get("items").map(arr).unwrap_or_default();
        p.mcp = v.get("mcpServers").map(arr).unwrap_or_default();
        p.fetched_at = v.get("fetchedAt").and_then(Value::as_f64);
        p.rebuild();
    }

    /// A vornd notification the page follows.
    pub(super) fn plug_notified(&mut self, method: &str, params: &Value) {
        match method {
            "connector:installProgress" => {
                if let Some(id) = s(params, "id") {
                    let progress = Progress::from_value(params);
                    self.st.plugins.progress.insert(id.to_owned(), progress);
                }
            }
            "connector:catalogChanged" => self.plug_catalog(params),
            _ if !self.st.shows_plugins() && !self.st.plugins.from_editor => {}
            "config:changed" => {
                self.call(
                    Call::Plug(PlugCall::Connections),
                    "connection:list",
                    json!({}),
                );
                self.st.plugins.loads += 1;
            }
            _ => {}
        }
    }

    /// How a sign-in the page started ended.
    pub(super) fn plug_signed_in(&mut self, conn: &str, outcome: super::signin::Outcome) {
        self.plug_finish(conn, outcome.err());
        self.plug_load(false);
    }

    /// A workflow requirement's fix, shown in the editor panel that asked.
    pub(super) fn plug_fix(&mut self, fix: Fix) {
        let p = &mut self.st.plugins;
        p.from_editor = true;
        p.adding = None;
        p.pending = None;
        p.report = None;
        p.intent = Some(fix);
        if p.loaded && p.loads == 0 {
            self.plug_apply_intent();
        } else if p.loads == 0 {
            self.plug_load(true);
        }
    }

    /// Once a frame: an editor fix is put away once its panel is, and the
    /// sheet's field takes the keys once it shows.
    pub(super) fn plug_tick(&mut self, ui: &mut Ui) {
        if std::mem::take(&mut self.focus_confirm) {
            let id = field_id(&Field::Confirm);
            ui.set_edit_text(id, "");
            ui.set_focus(Some(id));
        }
        if self.st.dialog == Some(Dialog::ExtInstall)
            && self.st.plugins.pending.is_none()
            && self.st.plugins.source.is_none()
        {
            self.st.dialog = None;
        }
        if self.st.plugins.menu_soon {
            let id = super::Acts::id("plug-install", &Act::Open(MenuKind::ExtInstall));
            if !self.st.shows_plugins() {
                self.st.plugins.menu_soon = false;
            } else if ui.authored_rect(id).is_some() {
                self.st.plugins.menu_soon = false;
                if self.st.menu.as_ref().map(|m| &m.kind) != Some(&MenuKind::ExtInstall) {
                    let src = Src {
                        id: Some(id),
                        at: None,
                    };
                    self.open_menu(ui, MenuKind::ExtInstall, src);
                }
            }
        }
        let panel_open = self.st.wf.editor.is_some()
            && (self.st.wf.panel == crate::workflows::Panel::StartFrom
                || !self.st.wf.still_needed().is_empty());
        if self.st.plugins.from_editor && !panel_open && !self.st.shows_plugins() {
            let p = &mut self.st.plugins;
            p.from_editor = false;
            p.adding = None;
            p.pending = None;
            p.intent = None;
            p.report = None;
        }
    }

    pub(super) fn plug_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        if !self.st.shows_plugins() && !self.st.plugins.from_editor {
            return false;
        }
        match event {
            UiEvent::Changed(id) => {
                let Some(Act::Plug(PlugAct::Text(field))) = self.acts.text_of(*id) else {
                    return false;
                };
                let value = ui.edit_text(*id).to_owned();
                self.plug_text(field, value);
                true
            }
            UiEvent::Submit(id) => match self.acts.text_of(*id) {
                Some(Act::Plug(PlugAct::Text(Field::Spec))) => {
                    self.run_plug(ui, PlugAct::Probe);
                    true
                }
                Some(Act::Plug(PlugAct::Text(Field::Query))) => true,
                Some(Act::Plug(PlugAct::Text(Field::Source))) => {
                    self.run_plug(ui, PlugAct::ReadSource);
                    true
                }
                Some(Act::Plug(PlugAct::Text(Field::Confirm))) => {
                    self.run_plug(ui, PlugAct::ConfirmInstall);
                    true
                }
                Some(Act::Plug(PlugAct::Text(_))) => {
                    self.run_plug(ui, PlugAct::Save);
                    true
                }
                _ => false,
            },
            UiEvent::Select { group, index } if *group == kinds_id() => {
                let p = &mut self.st.plugins;
                p.kind = match index {
                    1 => Some(plug::Kind::Connector),
                    2 => Some(plug::Kind::Extension),
                    _ => None,
                };
                true
            }
            UiEvent::Select { group, index } if *group == tabs_id() => {
                if let Some(tab) = Tab::ALL.get(*index) {
                    self.run_plug(ui, PlugAct::Tab(*tab));
                }
                true
            }
            UiEvent::Select { group, index } => {
                let Some(pick) = self
                    .plug_picks()
                    .into_iter()
                    .find(|p| widgets::menu_id(pick_id(p)) == *group)
                else {
                    return false;
                };
                ui.close_menu();
                self.plug_pick(pick, *index);
                true
            }
            _ => false,
        }
    }

    /// Every picker the page could be showing.
    fn plug_picks(&self) -> Vec<Pick> {
        let mut picks = vec![
            Pick::Category,
            Pick::Kind,
            Pick::Rung,
            Pick::Project,
            Pick::Trigger,
        ];
        if let Some(Form::Builtin(f)) = self.st.plugins.adding.as_ref().map(|a| &a.form) {
            let manifest = self
                .st
                .plugins
                .builtin(&f.connector)
                .and_then(|b| b.get("manifest"));
            for key in ["auth", "taskFilters"] {
                picks.extend(
                    plug::manifest_fields(manifest, key)
                        .into_iter()
                        .filter_map(|f| s(f, "key"))
                        .map(|k| Pick::Select(k.to_owned())),
                );
            }
        }
        picks
    }

    fn plug_pick(&mut self, pick: Pick, index: usize) {
        let projects: Vec<String> = self.store.projects.iter().map(|p| p.name.clone()).collect();
        let p = &mut self.st.plugins;
        match pick {
            Pick::Category => {
                p.category = index
                    .checked_sub(1)
                    .and_then(|i| plug::categories(&p.listings).get(i).cloned())
            }
            Pick::Kind => {
                p.kind = index
                    .checked_sub(1)
                    .and_then(|i| plug::kinds(&p.listings).get(i).copied())
            }
            Pick::Rung => {
                p.rung = index
                    .checked_sub(1)
                    .and_then(|i| plug::rungs(&p.listings).get(i).copied())
            }
            Pick::Project => {
                let detects = |p: &plug::Plugins, id: &str| {
                    p.builtin(id)
                        .and_then(|b| b.pointer("/manifest/detectRepo"))
                        == Some(&Value::Bool(true))
                };
                let redetect = match p.adding.as_ref().map(|a| &a.form) {
                    Some(Form::Builtin(f)) => detects(p, &f.connector),
                    _ => false,
                };
                let Some(a) = p.adding.as_mut() else { return };
                a.project = projects.get(index).cloned();
                if let Form::Builtin(f) = &mut a.form {
                    if redetect {
                        f.detected = None;
                    }
                }
                self.plug_kick();
            }
            Pick::Trigger => {
                if let Some(Form::Sdk(f)) = p.adding.as_mut().map(|a| &mut a.form) {
                    f.trigger = index;
                }
            }
            Pick::Select(key) => {
                let options: Vec<String> = {
                    let Some(Form::Builtin(f)) = p.adding.as_ref().map(|a| &a.form) else {
                        return;
                    };
                    let manifest = p.builtin(&f.connector).and_then(|b| b.get("manifest"));
                    ["auth", "taskFilters"]
                        .into_iter()
                        .flat_map(|k| plug::manifest_fields(manifest, k))
                        .find(|f| s(f, "key") == Some(key.as_str()))
                        .and_then(|f| f.get("options"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|o| {
                            s(o, "value")
                                .or_else(|| o.as_str())
                                .unwrap_or("")
                                .to_owned()
                        })
                        .collect()
                };
                if let Some(Form::Builtin(f)) = p.adding.as_mut().map(|a| &mut a.form) {
                    let value = index
                        .checked_sub(1)
                        .and_then(|i| options.get(i).cloned())
                        .unwrap_or_default();
                    f.values.insert(key, value);
                }
            }
        }
    }

    fn plug_text(&mut self, field: Field, value: String) {
        let p = &mut self.st.plugins;
        match field {
            Field::Query => return p.query = value,
            Field::Source => {
                if let Some(g) = p.source.as_mut() {
                    g.url = value;
                    g.error = None;
                }
                return;
            }
            Field::Confirm => {
                if let Some(pd) = p.pending.as_mut() {
                    pd.typed = value;
                }
                return;
            }
            _ => {}
        }
        let Some(a) = p.adding.as_mut() else { return };
        match (field, &mut a.form) {
            (Field::Auth(k), Form::Builtin(f)) => {
                f.values.insert(k, value);
            }
            (Field::Owner, Form::Builtin(f)) => f.owner = value,
            (Field::Repo, Form::Builtin(f)) => f.repo = value,
            (Field::Spec, Form::Sdk(f)) => f.spec = value,
            (Field::Env(n), Form::Sdk(f)) => {
                f.values.insert(n, value);
            }
            (Field::ProfileName, Form::Profile(f)) => f.name = value,
            (Field::ProfileUrl, Form::Profile(f)) => f.base_url = value,
            (Field::ProfileHeader, Form::Profile(f)) => f.header = value,
            (Field::ProfileSecret, Form::Profile(f)) => f.secret = value,
            _ => {}
        }
    }
}
