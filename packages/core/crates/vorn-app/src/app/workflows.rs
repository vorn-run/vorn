//! What the workflow landing, sidebar and editor do, and the answers to the
//! calls they make, as `WorkflowsLandingView`, `WorkflowsSection`,
//! `WorkflowEditor`, `SourcePromptDialog` and `MissedScheduleDialog` do them.

use serde_json::{json, Map, Value};
use vorn_mcp::workflow::portability as port;

use super::{list, tasks::open_url, App, Call, Src, MOD};
use crate::client::RpcError;
use crate::runs::{self, s};
use crate::screen::MainView;
use crate::state::{now_ms, Dialog};
use crate::ui::{mods, Event as UiEvent, Input, Ui};
use crate::workflows::{self as wf, Draft, Missed, Panel, Prompt, WfAct};

/// `MAX_WORKFLOW_FILE_BYTES`.
const MAX_FILE: u64 = 500_000;

/// `WORKFLOW_FILE_SUFFIX`.
const SUFFIX: &str = ".vorn-workflow.json";

/// A workflow call whose answer changes the screen.
pub(super) enum WfCall {
    /// `workflowRun:listAll`.
    Runs,
    /// `workflowRun:list` for the editor's workflow.
    History(String),
    /// `workflow:resolveGate`.
    Gate,
    /// `connection:browserAuth`.
    SignIn,
    /// `connection:list`, for the sign-in button's name.
    Connections,
}

impl App {
    pub(super) fn wf_answered(&mut self, call: WfCall, answer: Result<Value, RpcError>) {
        match call {
            WfCall::Runs => self.st.wf.runs = list(answer),
            WfCall::History(id) => {
                if self.st.wf.editor.as_ref().is_some_and(|d| d.id() == id) {
                    self.st.wf.history = list(answer);
                }
            }
            WfCall::Gate => match answer {
                Ok(v) if v.get("accepted").and_then(Value::as_bool) == Some(false) => {
                    let why = s(&v, "reason").unwrap_or("That gate is no longer waiting");
                    self.st.toast(why.to_owned(), true);
                }
                Ok(_) => self.wf_refresh(),
                Err(e) => self.st.toast(e.to_string(), true),
            },
            WfCall::Connections => self.st.wf.connections = list(answer),
            WfCall::SignIn => match answer {
                Ok(v) => match v.pointer("/browser/signInUrl").and_then(Value::as_str) {
                    Some(url) => open_url(url),
                    None => self.st.toast("Nothing to sign in to here", true),
                },
                Err(e) => self.st.toast(e.to_string(), true),
            },
        }
    }

    /// The workspace new workflows and the landing's runs belong to.
    pub(super) fn workspace(&self) -> String {
        self.store
            .config
            .pointer("/defaults/activeWorkspace")
            .and_then(Value::as_str)
            .unwrap_or("personal")
            .to_owned()
    }

    /// The saved workflow `id`, as the config holds it.
    pub(super) fn workflow(&self, id: &str) -> Option<Value> {
        self.store
            .config
            .get("workflows")
            .and_then(Value::as_array)?
            .iter()
            .find(|w| s(w, "id") == Some(id))
            .cloned()
    }

    pub(super) fn wf_refresh(&mut self) {
        let params = json!({ "workspaceId": self.workspace(), "limit": 50 });
        self.call(Call::Workflow(WfCall::Runs), "workflowRun:listAll", params);
        self.call(
            Call::Workflow(WfCall::Connections),
            "connection:list",
            Value::Null,
        );
    }

    fn load_history(&mut self, id: &str) {
        self.call(
            Call::Workflow(WfCall::History(id.to_owned())),
            "workflowRun:list",
            json!({ "workflowId": id, "limit": 20 }),
        );
    }

    /// A run vornd reported: the editor's history and its launch.
    pub(super) fn wf_run_updated(&mut self, run: &Value) {
        let Some(id) = self.st.wf.editor.as_ref().map(|d| d.id().to_owned()) else {
            return;
        };
        if s(run, "workflowId") != Some(id.as_str()) {
            return;
        }
        self.st.wf.launching = None;
        let key = runs::key(run);
        let before = match self.st.wf.history.iter_mut().find(|r| runs::key(r) == key) {
            Some(r) => Some(std::mem::replace(r, run.clone())),
            None => {
                self.st.wf.history.insert(0, run.clone());
                None
            }
        };
        // Re-read on a status change or a gate, as an earlier read may answer last.
        if before.is_none_or(|b| wf::live_signature(&b) != wf::live_signature(run)) {
            self.load_history(&id);
        }
    }

    /// `scheduler:missed`: the schedules that passed while nothing ran.
    pub(super) fn wf_missed(&mut self, params: &Value) {
        let missed: Vec<Missed> = params
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(Missed {
                    workflow: m.pointer("/workflow/id")?.as_str()?.to_owned(),
                    name: m
                        .pointer("/workflow/name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    at: wf::text_of(m.get("scheduledFor")),
                    on: true,
                })
            })
            .collect();
        if missed.is_empty() {
            return;
        }
        self.st.wf.missed = missed;
        self.st.dialog = Some(Dialog::Missed);
    }

    /// `startManualRun`: the prompt when the run needs one, else the run.
    pub(super) fn start_run(
        &mut self,
        ui: &mut Ui,
        id: &str,
        source: Option<Value>,
        target: Option<String>,
    ) {
        let Some(w) = self.workflow(id) else {
            return;
        };
        let name = s(&w, "name").unwrap_or("").to_owned();
        let nodes = w
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if wf::trigger(&nodes).is_none() {
            let text = format!("\"{name}\" has no trigger \u{2014} add one in the editor first");
            self.st.toast(text, true);
            return;
        }
        if wf::needs_prompt(&nodes, source.is_some()) {
            self.open_prompt(ui, id, &name, &nodes, source, target);
            return;
        }
        self.launch_wf(id, &name, source.map(|s| ("source", s)), None, target);
    }

    fn launch_wf(
        &mut self,
        id: &str,
        name: &str,
        base: Option<(&str, Value)>,
        inputs: Option<Value>,
        target: Option<String>,
    ) {
        let mut context = Map::new();
        if let Some((k, v)) = base {
            context.insert(k.to_owned(), v);
        }
        if let Some(i) = inputs {
            context.insert("inputs".into(), i);
        }
        let toast = Call::Toast(format!("Started \"{name}\""));
        if context.is_empty() && target.is_none() {
            self.call(toast, "workflow:runManual", json!({ "workflowId": id }));
            return;
        }
        let mut params = json!({ "workflowId": id, "context": context });
        if let Some(t) = target {
            params["targetNodeId"] = json!(t);
        }
        self.call(toast, "workflow:run", params);
    }

    fn open_prompt(
        &mut self,
        ui: &mut Ui,
        id: &str,
        name: &str,
        nodes: &[Value],
        source: Option<Value>,
        target: Option<String>,
    ) {
        let contextual = wf::contextual(nodes);
        let needs_project = contextual && source.is_none();
        let (needs_branch, needs_worktree) = if needs_project {
            wf::required(nodes)
        } else {
            (false, false)
        };
        let inputs = wf::asked(nodes);
        let mut picked = std::collections::HashMap::new();
        for d in &inputs {
            let key = s(d, "key").unwrap_or("").to_owned();
            let value = wf::input_default(d);
            if matches!(s(d, "type"), Some("boolean" | "select" | "project")) {
                picked.insert(key, value);
            } else {
                ui.set_edit_text(wf::prompt_id(&key), &value);
            }
        }
        ui.set_edit_text(wf::prompt_branch(), "");
        ui.set_focus(None);
        self.st.wf.prompt = Some(Prompt {
            workflow: id.to_owned(),
            name: name.to_owned(),
            contextual,
            source,
            needs_project,
            needs_branch,
            needs_worktree,
            project: self.store.projects.first().map(|p| p.name.clone()),
            worktree: false,
            inputs,
            picked,
            target,
        });
        self.st.dialog = Some(Dialog::RunPrompt);
    }

    /// The prompt's value for input `key`.
    pub(super) fn prompt_value(&self, ui: &Ui, key: &str) -> String {
        self.st
            .wf
            .prompt
            .as_ref()
            .and_then(|p| p.picked.get(key).cloned())
            .unwrap_or_else(|| ui.edit_text(wf::prompt_id(key)).to_owned())
    }

    fn prompt_run(&mut self, ui: &mut Ui) {
        let Some(p) = self.st.wf.prompt.clone() else {
            return;
        };
        let project = p
            .project
            .as_deref()
            .and_then(|n| self.store.projects.iter().find(|x| x.name == n))
            .map(|x| (x.name.clone(), x.path.clone()));
        if p.needs_project && project.is_none() {
            return;
        }
        if !wf::inputs_valid(&p.inputs, |k| self.prompt_value(ui, k)) {
            return;
        }
        let inputs = (!p.inputs.is_empty()).then(|| {
            let mut m = Map::new();
            for d in &p.inputs {
                let key = s(d, "key").unwrap_or("");
                m.insert(
                    key.to_owned(),
                    wf::input_json(d, &self.prompt_value(ui, key)),
                );
            }
            Value::Object(m)
        });
        let base = match (p.source, p.contextual, project) {
            (Some(src), _, _) => Some(("source", src)),
            (None, true, Some((name, path))) => {
                let branch = ui.edit_text(wf::prompt_branch()).trim().to_owned();
                let now = now_ms();
                let mut src = json!({
                    "id": format!("prompt:{}:{}", p.workflow, now as u64),
                    "agentType": "shell",
                    "projectName": name,
                    "projectPath": path,
                    "status": "idle",
                    "createdAt": now as u64,
                    "pid": 0,
                    "isWorktree": p.worktree,
                });
                if !branch.is_empty() {
                    src["branch"] = json!(branch);
                }
                Some(("source", src))
            }
            _ => None,
        };
        self.launch_wf(&p.workflow, &p.name, base, inputs, p.target);
        self.st.wf.prompt = None;
        self.st.dialog = None;
        ui.set_focus(None);
    }

    /// Saves the config with the workflows `f` changed, toasting `text`.
    fn change_workflows(&mut self, text: Option<String>, f: impl FnOnce(&mut Vec<Value>) -> bool) {
        let mut config = self.store.config.clone();
        if !config.get("workflows").is_some_and(Value::is_array) {
            config["workflows"] = json!([]);
        }
        let Some(list) = config["workflows"].as_array_mut() else {
            return;
        };
        if !f(list) {
            return;
        }
        let call = match text {
            Some(t) => Call::Toast(t),
            None => Call::Quiet,
        };
        self.save_config(config, call);
    }

    /// `persistWorkflow`: the draft saved, under the workspace, its last run
    /// kept.
    fn persist(&mut self, text: Option<String>) -> Option<String> {
        let ws = self.workspace();
        let d = self.st.wf.editor.as_mut()?;
        let mut w = d.wf.clone();
        if w.get("workspaceId").is_none_or(Value::is_null) {
            w["workspaceId"] = json!(ws);
        }
        d.existing = true;
        d.wf = w.clone();
        let id = d.id().to_owned();
        self.change_workflows(text, |list| {
            match list.iter_mut().find(|x| s(x, "id") == Some(id.as_str())) {
                Some(old) => {
                    for k in ["lastRunAt", "lastRunStatus"] {
                        if let Some(v) = old.get(k).filter(|_| w.get(k).is_none()) {
                            w[k] = v.clone();
                        }
                    }
                    *old = w;
                }
                None => list.push(w),
            }
            true
        });
        Some(id)
    }

    fn open_editor(&mut self, ui: &mut Ui, draft: Draft, panel: Panel) {
        let name = s(&draft.wf, "name").unwrap_or("").to_owned();
        ui.set_edit_text(wf::field_id(None, "name"), &name);
        let stagger = wf::text_of(draft.wf.get("staggerDelayMs"));
        ui.set_edit_text(wf::field_id(None, "staggerDelayMs"), &stagger);
        ui.set_focus(None);
        self.st.wf.editor = Some(draft);
        self.st.wf.node = None;
        self.st.wf.panel = panel;
        self.st.wf.history.clear();
        self.st.wf.launching = None;
        self.st.view = MainView::Workflows;
    }

    /// The open node's fields filled from it.
    fn seed(&self, ui: &mut Ui) {
        let Some(d) = &self.st.wf.editor else { return };
        let Some(node) = self.st.wf.node.as_deref().and_then(|n| d.node(n)) else {
            return;
        };
        let nid = s(node, "id");
        for path in wf::text_paths(node) {
            ui.set_edit_text(wf::field_id(nid, &path), &wf::field_text(node, &path));
        }
    }

    fn select_node(&mut self, ui: &mut Ui, id: Option<String>) {
        self.st.wf.panel = if id.is_some() {
            Panel::Node
        } else {
            Panel::Properties
        };
        self.st.wf.node = id;
        ui.set_focus(None);
        self.seed(ui);
    }

    /// The node the form edits: none for the workflow's own properties.
    fn form_node(&self) -> Option<String> {
        match self.st.wf.panel {
            Panel::Node => self.st.wf.node.clone(),
            _ => None,
        }
    }

    fn edit(&mut self, f: impl FnOnce(&mut Draft)) {
        if let Some(d) = self.st.wf.editor.as_mut() {
            f(d);
        }
    }

    pub(super) fn run_wf(&mut self, ui: &mut Ui, act: WfAct, _src: Src) {
        match act {
            WfAct::Tab(t) => self.st.wf.tab = t,
            WfAct::Bucket(b) => self.st.wf.bucket = b,
            WfAct::Refresh => self.wf_refresh(),
            WfAct::SelectRun(k) => self.st.wf.selected = Some(k),
            WfAct::OpenRun(k) => {
                let wid = self
                    .find_wf_run(&k)
                    .and_then(|r| s(&r, "workflowId").map(str::to_owned));
                if let Some(wid) = wid {
                    self.run_wf(ui, WfAct::Edit(wid), Src::default());
                }
            }
            WfAct::Gate { run, node, approve } => {
                let decision = if approve { "approve" } else { "reject" };
                self.call(
                    Call::Workflow(WfCall::Gate),
                    "workflow:resolveGate",
                    json!({ "runId": run, "nodeId": node, "decision": decision }),
                );
            }
            WfAct::SignIn(conn) => {
                self.call(
                    Call::Workflow(WfCall::SignIn),
                    "connection:browserAuth",
                    json!(conn),
                );
            }
            WfAct::Side(f) => self.st.wf.side = f,
            WfAct::AllRuns | WfAct::Close => {
                self.st.wf.editor = None;
                self.st.wf.node = None;
                self.st.wf.panel = Panel::None;
                self.st.view = MainView::Workflows;
                ui.set_focus(None);
                self.wf_refresh();
            }
            WfAct::Edit(id) => {
                let Some(w) = self.workflow(&id) else { return };
                let draft = Draft {
                    wf: w,
                    existing: true,
                };
                self.open_editor(ui, draft, Panel::Properties);
            }
            WfAct::New => {
                let draft = Draft {
                    wf: wf::blank(Some(&self.workspace())),
                    existing: false,
                };
                self.open_editor(ui, draft, Panel::StartFrom);
            }
            WfAct::Run(id) => self.start_run(ui, &id, None, None),
            WfAct::ToggleSchedule(id) => {
                self.change_workflows(None, |list| {
                    let Some(w) = list.iter_mut().find(|w| s(w, "id") == Some(id.as_str())) else {
                        return false;
                    };
                    let on = w.get("enabled").and_then(Value::as_bool) != Some(false);
                    w["enabled"] = json!(!on);
                    true
                });
                if let Some(d) = self.st.wf.editor.as_mut().filter(|d| d.id() == id) {
                    let on = d.wf.get("enabled").and_then(Value::as_bool) != Some(false);
                    d.wf["enabled"] = json!(!on);
                }
            }
            WfAct::Export(id) => {
                let Some(w) = self.workflow(&id) else { return };
                let name = format!("{}{SUFFIX}", port::slugify(s(&w, "name").unwrap_or("")));
                let path = downloads().join(name);
                ui.set_edit_text(wf::file_id(), &path.to_string_lossy());
                ui.set_focus(Some(wf::file_id()));
                self.st.wf.file = Some(id);
                self.st.dialog = Some(Dialog::WfFile);
            }
            WfAct::Import => {
                ui.set_edit_text(wf::file_id(), "");
                ui.set_focus(Some(wf::file_id()));
                self.st.wf.file = None;
                self.st.dialog = Some(Dialog::WfFile);
            }
            WfAct::File => {
                let path = ui.edit_text(wf::file_id()).trim().to_owned();
                if path.is_empty() {
                    return;
                }
                self.st.dialog = None;
                ui.set_focus(None);
                match self.st.wf.file.take() {
                    Some(id) => self.export(&id, &path),
                    None => self.import(&path),
                }
            }
            WfAct::Delete(id) => {
                self.change_workflows(None, |list| {
                    let before = list.len();
                    list.retain(|w| s(w, "id") != Some(id.as_str()));
                    list.len() != before
                });
                if self.st.wf.editor.as_ref().is_some_and(|d| d.id() == id) {
                    self.run_wf(ui, WfAct::Close, Src::default());
                }
            }
            WfAct::Save => {
                let existing = self.st.wf.editor.as_ref().is_some_and(|d| d.existing);
                let text = if existing {
                    "Workflow saved"
                } else {
                    "Workflow created"
                };
                self.persist(Some(text.to_owned()));
            }
            WfAct::RunDraft => {
                let ready = self
                    .st
                    .wf
                    .editor
                    .as_ref()
                    .is_some_and(|d| wf::trigger(d.nodes()).is_some());
                if !ready {
                    return;
                }
                let Some(id) = self.persist(None) else { return };
                self.start_run(ui, &id, None, None);
                if self.st.dialog.is_none() {
                    self.st.wf.launching = Some(now_ms());
                }
                self.st.wf.panel = Panel::History;
                self.st.wf.node = None;
                self.load_history(&id);
            }
            WfAct::RunTo(node) => {
                let Some(id) = self.persist(None) else { return };
                self.start_run(ui, &id, None, Some(node));
                self.st.wf.panel = Panel::History;
                self.st.wf.node = None;
                self.load_history(&id);
            }
            WfAct::Stop => {
                let Some(id) = self.st.wf.editor.as_ref().map(|d| d.id().to_owned()) else {
                    return;
                };
                if let Some(run) = self.st.wf.active_run(&id, &self.store.runs) {
                    let key = runs::key(&run);
                    self.call(Call::Quiet, "workflow:stopRun", json!({ "runId": key }));
                }
            }
            WfAct::History => {
                if self.st.wf.panel == Panel::History {
                    self.st.wf.panel = Panel::Properties;
                    return;
                }
                let Some(id) = self.st.wf.editor.as_ref().map(|d| d.id().to_owned()) else {
                    return;
                };
                self.st.wf.panel = Panel::History;
                self.st.wf.node = None;
                self.load_history(&id);
            }
            WfAct::Settings => {
                self.st.wf.node = None;
                self.st.wf.panel = Panel::Properties;
            }
            WfAct::SelectNode(id) => self.select_node(ui, Some(id)),
            WfAct::Library(at) => {
                self.st.wf.search.clear();
                ui.set_edit_text(wf::search_id(), "");
                ui.set_focus(Some(wf::search_id()));
                self.st.wf.panel = Panel::Library(at);
            }
            WfAct::Pick(kind) => {
                let Panel::Library(at) = self.st.wf.panel.clone() else {
                    return;
                };
                let mut added = None;
                self.edit(|d| added = wf::add(d, &at, &kind));
                self.select_node(ui, added);
            }
            WfAct::DeleteNode(id) => {
                self.edit(|d| wf::remove(d, &id));
                if self.st.wf.node.as_deref() == Some(id.as_str()) {
                    self.select_node(ui, None);
                }
            }
            WfAct::Set(path, value) => {
                let node = self.form_node();
                self.set_value(node.as_deref(), &path, &value);
                self.seed(ui);
            }
            WfAct::Toggle(path) => {
                let node = self.form_node();
                self.edit(|d| {
                    let cur = match node.as_deref().and_then(|n| d.node(n)) {
                        Some(n) => n.pointer(&format!("/{}", path.replace('.', "/"))).cloned(),
                        None => d.wf.get(&path).cloned(),
                    };
                    let on = match path.as_str() {
                        "enabled" => cur.and_then(|v| v.as_bool()) != Some(false),
                        _ => cur.and_then(|v| v.as_bool()) == Some(true),
                    };
                    wf::set(d, node.as_deref(), &path, json!(!on));
                });
            }
            WfAct::AddInput => {
                let Some(node) = self.st.wf.node.clone() else {
                    return;
                };
                self.edit(|d| wf::add_input(d, &node));
                self.seed(ui);
            }
            WfAct::RemoveInput(i) => {
                let Some(node) = self.st.wf.node.clone() else {
                    return;
                };
                self.edit(|d| wf::remove_input(d, &node, i));
                self.seed(ui);
            }
            WfAct::StartBlank => self.st.wf.panel = Panel::Properties,
            WfAct::ClosePanel => {
                self.st.wf.panel = match self.st.wf.panel {
                    Panel::Properties => Panel::None,
                    _ => Panel::Properties,
                };
                self.st.wf.node = None;
                ui.set_focus(None);
            }
            WfAct::PromptProject(name) => {
                if let Some(p) = self.st.wf.prompt.as_mut() {
                    p.project = Some(name);
                }
            }
            WfAct::PromptWorktree => {
                if let Some(p) = self.st.wf.prompt.as_mut() {
                    p.worktree = !p.worktree;
                }
            }
            WfAct::PromptSet(key, value) => {
                if let Some(p) = self.st.wf.prompt.as_mut() {
                    p.picked.insert(key, value);
                }
            }
            WfAct::PromptRun => self.prompt_run(ui),
            WfAct::MissedToggle(i) => {
                if let Some(m) = self.st.wf.missed.get_mut(i) {
                    m.on = !m.on;
                }
            }
            WfAct::MissedRun => {
                let picked: Vec<String> = self
                    .st
                    .wf
                    .missed
                    .iter()
                    .filter(|m| m.on)
                    .map(|m| m.workflow.clone())
                    .collect();
                for id in picked {
                    if self.workflow(&id).is_some() {
                        self.call(Call::Quiet, "workflow:run", json!({ "workflowId": id }));
                    }
                }
                self.st.wf.missed.clear();
                self.st.dialog = None;
            }
            WfAct::MissedSkip => {
                self.st.wf.missed.clear();
                self.st.dialog = None;
            }
        }
    }

    /// A select's pick: the project's path with its name, and nothing for
    /// "Any status".
    fn set_value(&mut self, node: Option<&str>, path: &str, value: &str) {
        let path_of = |name: &str| {
            self.store
                .projects
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.path.clone())
        };
        let project = (path == "config.projectName").then(|| path_of(value));
        self.edit(|d| {
            let v = if value.is_empty() && path != "config.projectName" {
                Value::Null
            } else {
                json!(value)
            };
            wf::set(d, node, path, v);
            if let Some(p) = project {
                wf::set(d, node, "config.projectPath", json!(p.unwrap_or_default()));
            }
        });
    }

    pub(super) fn find_wf_run(&self, key: &str) -> Option<Value> {
        self.store.runs.get(key).cloned().or_else(|| {
            self.st
                .wf
                .runs
                .iter()
                .chain(self.st.wf.history.iter())
                .find(|r| runs::key(r) == key)
                .cloned()
        })
    }

    /// `handleExport`.
    fn export(&mut self, id: &str, path: &str) {
        let Some(w) = self.workflow(id) else { return };
        let named = w
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|n| n.pointer("/config/projectName").and_then(Value::as_str))
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        let active = self.st.project.clone();
        let project = named
            .and_then(|n| self.store.projects.iter().find(|p| p.name == n))
            .or_else(|| {
                active
                    .as_deref()
                    .and_then(|a| self.store.projects.iter().find(|p| p.name == a))
            })
            .map(|p| p.path.clone())
            .unwrap_or_default();
        let connections = self.connections();
        let portable = port::to_portable(&w, &project, &connections);
        let contents = match serde_json::to_string_pretty(&portable) {
            Ok(t) => t + "\n",
            Err(e) => return self.st.toast(e.to_string(), true),
        };
        if let Err(e) = std::fs::write(path, contents) {
            return self.st.toast(e.to_string(), true);
        }
        let residual = port::residual_absolute_paths(&portable);
        if residual.is_empty() {
            let name = s(&w, "name").unwrap_or("");
            self.st.toast(format!("Exported \"{name}\""), false);
        } else {
            let text = format!(
                "Exported, but these still name this machine: {}",
                residual.join(", ")
            );
            self.st.toast(text, true);
        }
    }

    fn connections(&self) -> Vec<Value> {
        self.rpc
            .call("connection:list", Value::Null)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
    }

    /// `handleImport`.
    fn import(&mut self, path: &str) {
        let active = self.st.project.clone();
        let Some(project) = active
            .as_deref()
            .and_then(|a| self.store.projects.iter().find(|p| p.name == a))
            .or_else(|| self.store.projects.first())
            .map(|p| (p.name.clone(), p.path.clone()))
        else {
            let text = "Add a project first, so the imported paths have somewhere to resolve";
            return self.st.toast(text, true);
        };
        let text = match read_file(path) {
            Ok(t) => t,
            Err(e) => return self.st.toast(e, true),
        };
        let all: Vec<Value> = self
            .store
            .config
            .get("workflows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let connections = self.connections();
        let (mut def, unresolved) = match definition_from_file(&text, &project, &connections, &all)
        {
            Ok(v) => v,
            Err(e) => return self.st.toast(e, true),
        };
        let id = s(&def, "id").unwrap_or("").to_owned();
        let existing = all
            .iter()
            .find(|w| s(w, "id") == Some(id.as_str()))
            .cloned();
        if let Some(e) = &existing {
            def["enabled"] = e.get("enabled").cloned().unwrap_or(json!(true));
        }
        def["workspaceId"] = existing
            .as_ref()
            .and_then(|e| e.get("workspaceId").cloned())
            .unwrap_or_else(|| json!(self.workspace()));
        let name = s(&def, "name").unwrap_or("").to_owned();
        let pending = unresolved
            .iter()
            .map(describe_requirement)
            .collect::<Vec<_>>()
            .join(", ");
        let note = if !pending.is_empty() {
            format!(" \u{2014} still needs {pending}")
        } else if existing.is_some() {
            String::new()
        } else {
            " \u{2014} enable it when ready".to_owned()
        };
        let verb = if existing.is_some() {
            "Updated"
        } else {
            "Imported"
        };
        self.change_workflows(Some(format!("{verb} \"{name}\"{note}")), move |list| {
            match list.iter_mut().find(|w| s(w, "id") == Some(id.as_str())) {
                Some(old) => *old = def,
                None => list.push(def),
            }
            true
        });
    }

    /// The editor's and the landing's own events: a field's edit.
    pub(super) fn wf_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        match *event {
            UiEvent::Changed(id) => {
                if id == wf::search_id() {
                    self.st.wf.search = ui.edit_text(id).to_owned();
                    return true;
                }
                let Some(d) = &self.st.wf.editor else {
                    return false;
                };
                let node = self.form_node();
                let paths: Vec<String> = match node.as_deref().and_then(|n| d.node(n)) {
                    Some(n) => wf::text_paths(n),
                    None => Vec::new(),
                };
                let own = ["name", "staggerDelayMs"];
                let hit = paths
                    .iter()
                    .map(|p| (node.as_deref(), p.as_str()))
                    .chain(own.iter().map(|p| (None, *p)))
                    .find(|(n, p)| wf::field_id(*n, p) == id)
                    .map(|(n, p)| (n.map(str::to_owned), p.to_owned()));
                let Some((n, path)) = hit else {
                    return false;
                };
                let (vpath, value) = wf::field_value(&path, ui.edit_text(id));
                self.edit(|d| wf::set(d, n.as_deref(), &vpath, value));
                true
            }
            UiEvent::Submit(id) => {
                if id == wf::file_id() {
                    self.run_wf(ui, WfAct::File, Src::default());
                    return true;
                }
                if id == wf::prompt_branch()
                    || self.st.wf.prompt.as_ref().is_some_and(|p| {
                        p.inputs
                            .iter()
                            .any(|d| wf::prompt_id(s(d, "key").unwrap_or("")) == id)
                    })
                {
                    self.prompt_run(ui);
                    return true;
                }
                if id == wf::search_id() {
                    let first = crate::screen::workflows::library_rows(&self.st.wf)
                        .into_iter()
                        .next();
                    if let Some((kind, _, _)) = first {
                        self.run_wf(ui, WfAct::Pick(kind.to_owned()), Src::default());
                    }
                    return true;
                }
                if self.st.wf.editor.is_some() && id == wf::field_id(None, "name") {
                    ui.set_focus(None);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Keys the workflow views take before the UI: the landing gate's
    /// Mod+Enter and R, and Escape closing the editor's panel.
    pub(super) fn wf_key(&mut self, ui: &mut Ui, input: &Input) -> bool {
        let Input::Key { code, mods: m, .. } = input else {
            return false;
        };
        if self.st.view != MainView::Workflows || self.st.dialog.is_some() {
            return false;
        }
        let held = *m & (mods::CTRL | mods::SUPER | mods::ALT | mods::SHIFT);
        if self.st.wf.editor.is_some() {
            if code == "Escape"
                && held == 0
                && self.st.wf.panel != Panel::None
                && (matches!(self.st.wf.panel, Panel::Library(_) | Panel::Node)
                    || ui.focus().is_none())
            {
                self.run_wf(ui, WfAct::ClosePanel, Src::default());
                return true;
            }
            return false;
        }
        if ui.focus().is_some() {
            return false;
        }
        let approve = code == "Enter" && held == MOD;
        let reject = code == "KeyR" && held == 0;
        if !approve && !reject {
            return false;
        }
        let visible = self.st.wf.visible(&self.store.runs);
        let Some(run) = self.st.wf.current(&visible) else {
            return false;
        };
        let Some(gate) = runs::states(run).find(|n| s(n, "status") == Some("waiting")) else {
            return false;
        };
        if wf::is_sign_in(gate) && reject {
            return false;
        }
        let node = s(gate, "nodeId").unwrap_or("").to_owned();
        let key = runs::key(run);
        let act = if wf::is_sign_in(gate) {
            WfAct::SignIn(s(gate, "connectionId").unwrap_or("").to_owned())
        } else {
            WfAct::Gate {
                run: key,
                node,
                approve,
            }
        };
        self.run_wf(ui, act, Src::default());
        true
    }
}

/// Where an export goes first: the Downloads folder, else home.
fn downloads() -> std::path::PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let d = home.join("Downloads");
    if d.is_dir() {
        d
    } else {
        home
    }
}

fn read_file(path: &str) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| "That is not a workflow file".to_owned())?;
    if meta.len() > MAX_FILE {
        return Err("That file is too large to be a workflow".into());
    }
    std::fs::read_to_string(path).map_err(|_| "That is not a workflow file".into())
}

/// `definitionFromFile`: the definition a file carries and what it still
/// needs here, or why it carries none.
fn definition_from_file(
    text: &str,
    project: &(String, String),
    connections: &[Value],
    existing: &[Value],
) -> Result<(Value, Vec<Value>), String> {
    let parsed: Value =
        serde_json::from_str(text).map_err(|_| "That file is not valid JSON".to_owned())?;
    let version = parsed.get("version").and_then(Value::as_f64);
    if version != Some(port::PORTABLE_FORMAT_VERSION) {
        let shown = parsed
            .get("version")
            .filter(|v| !v.is_null())
            .map_or_else(|| "unknown".to_owned(), |v| wf::text_of(Some(v)));
        return Err(format!(
            "That file is version {shown}; this build reads version {}",
            port::PORTABLE_FORMAT_VERSION
        ));
    }
    let name_ok = s(&parsed, "name").is_some_and(|n| !n.is_empty());
    if !name_ok
        || !parsed.get("nodes").is_some_and(Value::is_array)
        || !parsed.get("edges").is_some_and(Value::is_array)
    {
        return Err("That file is missing a name, nodes or edges".into());
    }
    graph_complaint(&parsed)?;
    let mut portable = parsed.clone();
    if portable.get("slug").is_none_or(Value::is_null) {
        portable["slug"] = json!(port::slugify(s(&parsed, "name").unwrap_or("")));
    }
    let bundle = port::slugify(&project.0);
    let mut def = port::from_portable(
        &portable,
        &bundle,
        &json!(project.0),
        &project.1,
        connections,
    );
    let slug = s(&portable, "slug").unwrap_or("").to_owned();
    def["id"] = json!(port::imported_workflow_id_for(
        &bundle,
        &slug,
        portable.get("name"),
        existing
    ));
    Ok((def, port::unresolved_requirements(&portable, connections)))
}

/// `graphComplaint`.
fn graph_complaint(p: &Value) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for n in p["nodes"].as_array().into_iter().flatten() {
        let (Some(id), Some(_)) = (s(n, "id"), s(n, "type")) else {
            return Err("That file has a step this build cannot read".into());
        };
        if !ids.insert(id.to_owned()) {
            return Err("That file carries the same step twice".into());
        }
    }
    for e in p["edges"].as_array().into_iter().flatten() {
        let (Some(a), Some(b)) = (s(e, "source"), s(e, "target")) else {
            return Err("That file has a connection this build cannot read".into());
        };
        if !ids.contains(a) || !ids.contains(b) {
            return Err("That file connects a step it does not carry".into());
        }
    }
    Ok(())
}

/// `describeRequirement`.
fn describe_requirement(r: &Value) -> String {
    let name = s(r, "name").filter(|n| !n.is_empty());
    if s(r, "kind") == Some("httpProfile") {
        return name.map_or_else(
            || "an HTTP profile".to_owned(),
            |n| format!("an HTTP profile like \"{n}\""),
        );
    }
    let connector = s(r, "connectorId")
        .filter(|c| !c.is_empty())
        .unwrap_or("connector");
    if s(r, "key") == Some("secretsFrom") {
        let used = name
            .map(|n| format!(" (it used \"{n}\")"))
            .unwrap_or_default();
        return format!("a {connector} key for its Secrets from{used}");
    }
    name.map_or_else(
        || format!("a {connector} connection"),
        |n| format!("a {connector} connection like \"{n}\""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(nodes: Value, edges: Value) -> String {
        json!({ "version": 1, "name": "Ship it", "nodes": nodes, "edges": edges }).to_string()
    }

    #[test]
    fn a_file_is_read_as_the_renderer_reads_it() {
        let project = ("web".to_owned(), "/tmp/web".to_owned());
        let bad = |t: &str| definition_from_file(t, &project, &[], &[]).unwrap_err();
        assert_eq!(bad("{"), "That file is not valid JSON");
        assert_eq!(
            bad(r#"{"version":2}"#),
            "That file is version 2; this build reads version 1"
        );
        assert_eq!(
            bad(r#"{"version":1}"#),
            "That file is missing a name, nodes or edges"
        );
        let twice = file(
            json!([{ "id": "a", "type": "script" }, { "id": "a", "type": "script" }]),
            json!([]),
        );
        assert_eq!(bad(&twice), "That file carries the same step twice");
        let dangling = file(
            json!([{ "id": "a", "type": "script" }]),
            json!([{ "source": "a", "target": "b" }]),
        );
        assert_eq!(
            bad(&dangling),
            "That file connects a step it does not carry"
        );
        let ok = file(
            json!([{ "id": "t", "type": "trigger", "config": { "triggerType": "manual" } }]),
            json!([]),
        );
        let (def, unresolved) = definition_from_file(&ok, &project, &[], &[]).unwrap();
        assert_eq!(def["id"], "import:web:ship-it");
        assert_eq!(def["name"], "Ship it");
        assert!(unresolved.is_empty());
    }

    #[test]
    fn requirements_read_as_the_toast_says_them() {
        assert_eq!(
            describe_requirement(&json!({ "kind": "httpProfile", "name": "api" })),
            "an HTTP profile like \"api\""
        );
        assert_eq!(
            describe_requirement(&json!({ "kind": "connection", "connectorId": "github" })),
            "a github connection"
        );
    }
}
