//! What the main screen shows, as vornd last told it: the config, projects,
//! sessions, tasks and workflows, loaded once and then kept current by
//! folding notifications in, the way the renderer's stores do.
//!
//! vornd's configuration carries neither tasks nor workflows; the app keeps
//! them beside its copy of it, under `tasks` and `workflows`, so the screens
//! read one value, and [`super::sync`] carries a change to them back.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{json, Value};
use vorn_protocol::{
    ProjectConfig, SessionGroupConfig, TaskConfig, TerminalSession, WorkflowDefinition,
    WorkspaceConfig,
};

use super::rpc::{Pending, Rpc, RpcError, CALL_TIMEOUT};

/// The notifications the store folds in; everything else stays on the server.
pub const TOPICS: &[&str] = &[
    "session:created",
    "session:updated",
    "session:reordered",
    "terminal:exit",
    "config:changed",
    "task:changed",
    "workflow:changed",
    "workflow:runUpdated",
    "artifact:published",
    "artifact:commentsChanged",
    "connector:*",
    "connection:*",
    "extension:*",
    "pairing:*",
    "terminal:bell",
    "terminal:notify",
    "terminal:command",
];

/// The agent a new composer session starts when the config names none, as
/// the renderer defaults it.
pub const DEFAULT_AGENT: &str = "claude";

/// One terminal session and what is known about how it ended.
#[derive(Debug, Clone)]
pub struct Session {
    pub info: TerminalSession,
    /// The exit code once its program has ended; the card stays until closed.
    pub exited: Option<i64>,
    /// Why a session carried over from the last run ended, when it did.
    pub cause: Option<&'static str>,
}

impl Session {
    /// `getDisplayName`: the name given, else the project's.
    pub fn title(&self) -> String {
        match self.info.display_name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => self.info.project_name.clone(),
        }
    }

    /// Whether this is a plain shell rather than an agent.
    pub fn is_shell(&self) -> bool {
        self.info.agent_type.as_str() == "shell"
    }

    /// Whether its program is gone and the card shows the ended strip.
    pub fn ended(&self) -> bool {
        self.exited.is_some() || self.cause.is_some()
    }

    /// The ended strip's cause: `exited with N`, or why a carried-over
    /// session stopped.
    pub fn ended_cause(&self) -> Option<String> {
        if let Some(cause) = self.cause {
            return Some(cause.to_owned());
        }
        self.exited.map(|code| format!("exited with {code}"))
    }

    /// The status the card shows: an ended session is idle.
    pub fn status(&self) -> &str {
        if self.ended() {
            "idle"
        } else {
            self.info.status.as_str()
        }
    }
}

/// What a notification changed, so the screen knows what to redo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Nothing,
    /// A session appeared, changed or was reordered.
    Sessions,
    /// The session with this id ended.
    Exited(String),
    Config,
    /// The number of steps waiting on an approval changed.
    Approvals,
}

/// The models the main screen reads.
#[derive(Debug, Clone, Default)]
pub struct Store {
    /// vornd's `AppConfig`, kept whole: the app reads few fields of it and
    /// must write back the rest unchanged. Its `tasks` and `workflows` are the
    /// app's own copies, which vornd's configuration does not carry.
    pub config: Value,
    pub projects: Vec<ProjectConfig>,
    /// In vornd's order, which is the grid's.
    pub sessions: Vec<Session>,
    pub tasks: Vec<TaskConfig>,
    pub workflows: Vec<WorkflowDefinition>,
    /// The active workspace's session groups, in their order.
    pub groups: Vec<SessionGroupConfig>,
    /// Steps waiting on an approval, by run id: what the session dock counts.
    pub approvals: HashMap<String, usize>,
    /// The latest of every run vornd reported since loading, by run id.
    pub runs: HashMap<String, Value>,
    /// Whether vornd has said which sessions run: asked after connecting, since
    /// it waits for the session holder while vornd starts.
    pub sessions_known: bool,
}

fn subscription() -> Value {
    json!({ "topics": TOPICS, "terminalBytes": false })
}

/// The loads [`Store::load`] waits on, sent together.
fn load_calls(rpc: &Rpc) -> [Pending; 5] {
    [
        rpc.request("config:load", Value::Null),
        rpc.request("project:list", Value::Null),
        rpc.request("task:list", json!({ "includeDescription": true })),
        rpc.request("workflow:list", Value::Null),
        rpc.request("workflowRun:listWaiting", Value::Null),
    ]
}

impl Store {
    /// Subscribes to [`TOPICS`] and loads everything, the calls in flight
    /// together. Subscribing first means nothing is missed between the two.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(rpc: &Rpc) -> Result<Store, RpcError> {
        rpc.call("subscribe:set", subscription())?;
        Store::from_answers(load_calls(rpc).map(|p| p.wait(CALL_TIMEOUT)))
    }

    /// [`Store::load`] without holding the page's one thread.
    #[cfg(target_arch = "wasm32")]
    pub async fn load_async(rpc: &Rpc) -> Result<Store, RpcError> {
        rpc.request("subscribe:set", subscription())
            .answer(CALL_TIMEOUT)
            .await?;
        let mut answers = Vec::new();
        for p in load_calls(rpc) {
            answers.push(p.answer(CALL_TIMEOUT).await);
        }
        let answers: [Result<Value, RpcError>; 5] = answers
            .try_into()
            .map_err(|_| RpcError::Decode("missing answers".into()))?;
        Store::from_answers(answers)
    }

    fn from_answers(answers: [Result<Value, RpcError>; 5]) -> Result<Store, RpcError> {
        let [config, projects, tasks, workflows, waiting] = answers;
        let (mut config, tasks, workflows) = (config?, tasks?, workflows?);
        let mut store = Store {
            tasks: records_in(Some(&tasks)),
            workflows: records_in(Some(&workflows)),
            projects: records(projects?),
            ..Store::default()
        };
        if config.is_object() {
            config["tasks"] = tasks;
            config["workflows"] = workflows;
        }
        store.config = config;
        store.groups = groups(&store.config);
        // Runs are optional to the screen: a vornd without them still shows it.
        if let Ok(Value::Array(runs)) = waiting {
            for run in runs {
                store.run_updated(&run);
            }
        }
        Ok(store)
    }

    /// `terminal:listActive`'s answer: every session that runs.
    pub fn take_sessions(&mut self, list: Value) {
        for info in records::<TerminalSession>(list) {
            self.upsert(info);
        }
        self.sessions_known = true;
    }

    /// The agent the composer starts: `defaults.defaultAgent`, as the
    /// renderer reads it.
    pub fn default_agent(&self) -> &str {
        self.config
            .pointer("/defaults/defaultAgent")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty())
            .unwrap_or(DEFAULT_AGENT)
    }

    /// The workspace the screen shows: `defaults.activeWorkspace`, which
    /// every app on this vornd saves on a switch, else `personal`.
    pub fn workspace(&self) -> &str {
        active_workspace(&self.config)
    }

    /// `config.workspaces ?? [DEFAULT_WORKSPACE]`, skipping any it cannot read.
    pub fn workspaces(&self) -> Vec<WorkspaceConfig> {
        match self.config.get("workspaces").and_then(Value::as_array) {
            Some(list) => list
                .iter()
                .filter_map(|w| serde_json::from_value(w.clone()).ok())
                .collect(),
            None => vec![default_workspace()],
        }
    }

    /// The active workspace, else the default one, as the switcher shows it.
    pub fn current_workspace(&self) -> WorkspaceConfig {
        let id = self.workspace();
        self.workspaces()
            .into_iter()
            .find(|w| w.id == id)
            .unwrap_or_else(default_workspace)
    }

    /// Whether `p` belongs to the active workspace; a project with none
    /// belongs to `personal`.
    pub fn in_workspace(&self, p: &ProjectConfig) -> bool {
        p.workspace_id.as_deref().unwrap_or(WORKSPACE) == self.workspace()
    }

    /// `useWorkspaceProjects`: the active workspace's projects, in order.
    pub fn workspace_projects(&self) -> impl Iterator<Item = &ProjectConfig> {
        self.projects.iter().filter(|p| self.in_workspace(p))
    }

    /// Whether a session's project is in the active workspace, which is
    /// what `useSessionScope` lets any surface show. A session outside every
    /// project (a shell in the home directory) belongs to no workspace, so
    /// it shows in all of them rather than nowhere.
    pub fn in_scope(&self, s: &Session) -> bool {
        match self.projects.iter().find(|p| p.name == s.info.project_name) {
            Some(p) => self.in_workspace(p),
            None => true,
        }
    }

    /// The active workspace's sessions.
    pub fn scoped_sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.iter().filter(|s| self.in_scope(s))
    }

    /// The group with `id`.
    pub fn group(&self, id: &str) -> Option<&SessionGroupConfig> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The session with `id`.
    pub fn session(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.info.id == id)
    }

    /// Steps waiting on an approval, which the session dock counts.
    pub fn approvals(&self) -> usize {
        self.approvals.values().sum()
    }

    /// Counts a run's steps waiting on an approval; a step parked until its
    /// connection signs in again is not one (`isSignInWait`).
    fn run_updated(&mut self, run: &Value) -> bool {
        let Some(id) = run.get("runId").and_then(Value::as_str) else {
            return false;
        };
        self.runs.insert(id.to_owned(), run.clone());
        let waiting = run
            .get("nodeStates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|n| {
                n.get("status").and_then(Value::as_str) == Some("waiting")
                    && n.get("waitingFor").and_then(Value::as_str) != Some("signIn")
            })
            .count();
        let before = if waiting == 0 {
            self.approvals.remove(id)
        } else {
            self.approvals.insert(id.to_owned(), waiting)
        };
        before.unwrap_or(0) != waiting
    }

    /// Adds a session, or replaces the one with its id in place; a record
    /// without a branch keeps the one already learned.
    pub fn upsert(&mut self, mut info: TerminalSession) {
        match self.sessions.iter_mut().find(|s| s.info.id == info.id) {
            Some(s) => {
                if info.branch.is_none() {
                    info.branch = s.info.branch.take();
                }
                s.info = info;
            }
            None => self.sessions.push(Session {
                info,
                exited: None,
                cause: None,
            }),
        }
    }

    /// Takes the session with `id` off the screen.
    pub fn remove(&mut self, id: &str) {
        self.sessions.retain(|s| s.info.id != id);
    }

    /// Folds one notification in.
    pub fn apply(&mut self, method: &str, params: Value) -> Change {
        match method {
            "session:created" | "session:updated" => {
                match serde_json::from_value::<TerminalSession>(params) {
                    Ok(info) => {
                        self.upsert(info);
                        Change::Sessions
                    }
                    Err(_) => Change::Nothing,
                }
            }
            "session:reordered" => {
                let Some(order) = params.as_array() else {
                    return Change::Nothing;
                };
                let rank = |id: &str| {
                    order
                        .iter()
                        .position(|o| o.as_str() == Some(id))
                        .unwrap_or(usize::MAX)
                };
                self.sessions.sort_by_key(|s| rank(&s.info.id));
                Change::Sessions
            }
            "terminal:exit" => {
                let Some(id) = params.get("id").and_then(Value::as_str) else {
                    return Change::Nothing;
                };
                let Some(s) = self.sessions.iter_mut().find(|s| s.info.id == id) else {
                    return Change::Nothing;
                };
                s.exited = Some(params.get("exitCode").and_then(Value::as_i64).unwrap_or(0));
                Change::Exited(id.to_owned())
            }
            "workflow:runUpdated" => {
                if self.run_updated(&params) {
                    Change::Approvals
                } else {
                    Change::Nothing
                }
            }
            "config:changed" => {
                let mut params = params;
                // vornd's configuration carries no tasks or workflows: the app's copies stay.
                for key in ["tasks", "workflows"] {
                    if params.is_object() && params.get(key).is_none() {
                        params[key] = self.config.get(key).cloned().unwrap_or(json!([]));
                    }
                }
                let was = std::mem::replace(&mut self.config, params);
                self.projects = records_in(self.config.get("projects"));
                let tasks = std::mem::take(&mut self.tasks);
                self.tasks = records_since(self.config.get("tasks"), was.get("tasks"), tasks);
                self.workflows = records_in(self.config.get("workflows"));
                self.groups = groups(&self.config);
                Change::Config
            }
            "task:changed" => {
                if !params.is_array() || !self.config.is_object() {
                    return Change::Nothing;
                }
                let was = std::mem::replace(&mut self.config["tasks"], params);
                let tasks = std::mem::take(&mut self.tasks);
                self.tasks = records_since(self.config.get("tasks"), Some(&was), tasks);
                Change::Config
            }
            "workflow:changed" => {
                if !params.is_array() || !self.config.is_object() {
                    return Change::Nothing;
                }
                self.workflows = records_in(Some(&params));
                self.config["workflows"] = params;
                Change::Config
            }
            _ => Change::Nothing,
        }
    }
}

impl Store {
    /// Rereads the task `id` from `config` after a change there: changed,
    /// added (last, as it was added) or gone.
    pub fn reread_task(&mut self, id: &str) {
        let read = self
            .config
            .get("tasks")
            .and_then(Value::as_array)
            .and_then(|l| {
                l.iter()
                    .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
            })
            .and_then(|t| TaskConfig::deserialize(t).ok());
        let at = self.tasks.iter().position(|t| t.id == id);
        match (read, at) {
            (Some(t), Some(i)) => self.tasks[i] = t,
            (Some(t), None) => self.tasks.push(t),
            (None, Some(i)) => {
                self.tasks.remove(i);
            }
            (None, None) => {}
        }
    }
}

/// `DEFAULT_WORKSPACE`.
pub fn default_workspace() -> WorkspaceConfig {
    WorkspaceConfig {
        id: WORKSPACE.to_owned(),
        name: "Personal".to_owned(),
        icon: Some("User".to_owned()),
        icon_color: Some("#6b7280".to_owned()),
        order: 0.0,
    }
}

/// The workspace every config has (`DEFAULT_WORKSPACE_ID`).
pub const WORKSPACE: &str = "personal";

/// `defaults.activeWorkspace`, else `personal`.
fn active_workspace(config: &Value) -> &str {
    config
        .pointer("/defaults/activeWorkspace")
        .and_then(Value::as_str)
        .filter(|w| !w.is_empty())
        .unwrap_or(WORKSPACE)
}

/// `useWorkspaceSessionGroups`: the active workspace's groups by `order`.
fn groups(config: &Value) -> Vec<SessionGroupConfig> {
    let workspace = active_workspace(config);
    let mut out: Vec<SessionGroupConfig> =
        records(config.get("sessionGroups").cloned().unwrap_or_default());
    out.retain(|g| g.workspace_id == workspace);
    out.sort_by(|a, b| a.order.total_cmp(&b.order));
    out
}

/// [`records`] without taking `list`.
fn records_in<T: serde::de::DeserializeOwned>(list: Option<&Value>) -> Vec<T> {
    match list {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| T::deserialize(v).ok())
            .collect(),
        _ => Vec::new(),
    }
}

/// The records of `list`, keeping those of `old` (read from `was`) whose
/// entry is unchanged, so a change to one of thousands reads only that one.
fn records_since<T: serde::de::DeserializeOwned>(
    list: Option<&Value>,
    was: Option<&Value>,
    old: Vec<T>,
) -> Vec<T> {
    if let (Some(Value::Array(now)), Some(Value::Array(was))) = (list, was) {
        if now.len() == was.len() && old.len() == was.len() {
            let mut out = Vec::with_capacity(now.len());
            for ((n, w), o) in now.iter().zip(was).zip(old) {
                if n == w {
                    out.push(o);
                } else if let Ok(t) = T::deserialize(n) {
                    out.push(t);
                } else {
                    return records_in(list);
                }
            }
            return out;
        }
    }
    records_in(list)
}

/// The entries of a list that read as `T`; one malformed record should not
/// empty the screen.
fn records<T: serde::de::DeserializeOwned>(list: Value) -> Vec<T> {
    match list {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, status: &str) -> Value {
        json!({ "id": id, "agentType": "shell", "projectName": "", "projectPath": "/tmp",
                "status": status, "createdAt": 1.0, "pid": 10.0 })
    }

    fn store(ids: &[&str]) -> Store {
        let mut s = Store::default();
        for id in ids {
            assert_eq!(
                s.apply("session:created", session(id, "running")),
                Change::Sessions
            );
        }
        s
    }

    fn ids(s: &Store) -> Vec<&str> {
        s.sessions.iter().map(|s| s.info.id.as_str()).collect()
    }

    #[test]
    fn updates_replace_in_place() {
        let mut s = store(&["a", "b"]);
        let mut renamed = session("a", "waiting");
        renamed["displayName"] = json!("Shell 1");
        s.apply("session:updated", renamed);
        assert_eq!(ids(&s), ["a", "b"]);
        assert_eq!(s.sessions[0].title(), "Shell 1");
        assert_eq!(s.sessions[0].status(), "waiting");
    }

    #[test]
    fn reorder_follows_vornd_and_keeps_strangers_last() {
        let mut s = store(&["a", "b", "c"]);
        s.apply("session:reordered", json!(["c", "a"]));
        assert_eq!(ids(&s), ["c", "a", "b"]);
    }

    #[test]
    fn an_exit_keeps_the_card_and_idles_it() {
        let mut s = store(&["a"]);
        assert_eq!(
            s.apply("terminal:exit", json!({ "id": "a", "exitCode": 3 })),
            Change::Exited("a".into())
        );
        assert_eq!(
            (s.sessions[0].exited, s.sessions[0].status()),
            (Some(3), "idle")
        );
        assert_eq!(
            s.apply("terminal:exit", json!({ "id": "zz" })),
            Change::Nothing
        );
    }

    #[test]
    fn the_dock_counts_approvals_not_sign_ins() {
        let mut s = Store::default();
        let run = |states: Value| json!({ "runId": "r", "nodeStates": states });
        let gate = json!({ "nodeId": "a", "status": "waiting" });
        let sign_in = json!({ "nodeId": "b", "status": "waiting", "waitingFor": "signIn" });
        let changed = s.apply("workflow:runUpdated", run(json!([gate, sign_in])));
        assert_eq!((changed, s.approvals()), (Change::Approvals, 1));
        let same = s.apply("workflow:runUpdated", run(json!([gate])));
        assert_eq!(same, Change::Nothing);
        let done = json!([{ "nodeId": "a", "status": "completed" }]);
        assert_eq!(s.apply("workflow:runUpdated", run(done)), Change::Approvals);
        assert_eq!(s.approvals(), 0);
    }

    fn task(id: &str, title: &str) -> Value {
        json!({ "id": id, "projectName": "web", "title": title, "description": "",
                "status": "todo", "order": 0, "createdAt": "2026-10-14T00:00:00.000Z",
                "updatedAt": "2026-10-14T00:00:00.000Z" })
    }

    fn titles(s: &Store) -> Vec<&str> {
        s.tasks.iter().map(|t| t.title.as_str()).collect()
    }

    #[test]
    fn a_config_change_rereads_only_the_tasks_that_changed() {
        let mut s = Store::default();
        s.apply(
            "config:changed",
            json!({ "tasks": [task("a", "A"), task("b", "B")] }),
        );
        assert_eq!(titles(&s), ["A", "B"]);
        s.apply(
            "config:changed",
            json!({ "tasks": [task("a", "A"), task("b", "B2")] }),
        );
        assert_eq!(titles(&s), ["A", "B2"]);
        let three = json!({ "tasks": [task("c", "C"), { "bad": 1 }, task("a", "A")] });
        s.apply("config:changed", three);
        assert_eq!(titles(&s), ["C", "A"]);
        s.apply(
            "config:changed",
            json!({ "tasks": [task("c", "C"), task("a", "A1")] }),
        );
        assert_eq!(titles(&s), ["C", "A1"]);
    }

    #[test]
    fn tasks_and_workflows_come_by_their_own_notes_and_stay_through_a_config_change() {
        let mut s = Store::default();
        s.apply("config:changed", json!({ "projects": [] }));
        s.apply("task:changed", json!([task("a", "A")]));
        s.apply(
            "workflow:changed",
            json!([{ "id": "w", "name": "W", "icon": "Zap",
            "iconColor": "#fff", "nodes": [], "edges": [], "enabled": true }]),
        );
        assert_eq!(titles(&s), ["A"]);
        assert_eq!(s.workflows.len(), 1);
        // vornd's configuration has neither: the copies stay.
        s.apply(
            "config:changed",
            json!({ "projects": [], "defaults": { "fontSize": 14 } }),
        );
        assert_eq!(titles(&s), ["A"]);
        assert_eq!(s.workflows.len(), 1);
        assert_eq!(s.config["tasks"][0]["id"], "a");
        assert_eq!(s.config["workflows"][0]["id"], "w");
        s.apply("task:changed", json!([task("a", "A2"), task("b", "B")]));
        assert_eq!(titles(&s), ["A2", "B"]);
        assert_eq!(s.apply("task:changed", json!({})), Change::Nothing);
    }

    #[test]
    fn a_task_rereads_alone_when_changed_added_or_gone() {
        let mut s = Store::default();
        s.apply(
            "config:changed",
            json!({ "tasks": [task("a", "A"), task("b", "B")] }),
        );
        s.config["tasks"][0]["title"] = json!("A2");
        s.reread_task("a");
        assert_eq!(titles(&s), ["A2", "B"]);
        s.config["tasks"]
            .as_array_mut()
            .unwrap()
            .push(task("c", "C"));
        s.reread_task("c");
        assert_eq!(titles(&s), ["A2", "B", "C"]);
        s.config["tasks"].as_array_mut().unwrap().remove(1);
        s.reread_task("b");
        assert_eq!(titles(&s), ["A2", "C"]);
        s.reread_task("zz");
        assert_eq!(titles(&s), ["A2", "C"]);
    }

    #[test]
    fn config_brings_projects_and_the_default_agent() {
        let mut s = Store::default();
        assert_eq!(s.default_agent(), DEFAULT_AGENT);
        let config = json!({
            "defaults": { "defaultAgent": "codex" },
            "projects": [{ "name": "web", "path": "/w", "preferredAgents": [] }, { "bad": 1 }],
        });
        assert_eq!(s.apply("config:changed", config), Change::Config);
        assert_eq!(s.default_agent(), "codex");
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects[0].name, "web");
    }

    #[test]
    fn groups_are_the_workspaces_own_in_order() {
        let mut s = Store::default();
        let g = |id: &str, order: f64, ws: &str| json!({ "id": id, "name": id, "order": order, "workspaceId": ws });
        let config = json!({ "sessionGroups": [g("b", 1.0, "personal"), g("x", 0.0, "work"), g("a", 0.0, "personal")] });
        s.apply("config:changed", config);
        let ids: Vec<&str> = s.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(s.group("b").map(|g| g.order), Some(1.0));
    }

    #[test]
    fn sessions_show_in_their_projects_workspace_and_loose_ones_in_all() {
        let mut s = Store::default();
        for (id, project) in [("w", "web"), ("a", "api"), ("h", "me")] {
            let mut v = session(id, "running");
            v["projectName"] = json!(project);
            s.apply("session:created", v);
        }
        let config = json!({
            "defaults": { "activeWorkspace": "work" },
            "projects": [
                { "name": "web", "path": "/w", "preferredAgents": [], "workspaceId": "work" },
                { "name": "api", "path": "/a", "preferredAgents": [] },
            ],
        });
        s.apply("config:changed", config);
        let scoped: Vec<&str> = s.scoped_sessions().map(|s| s.info.id.as_str()).collect();
        assert_eq!(scoped, ["w", "h"]);
    }

    #[test]
    fn malformed_notifications_change_nothing() {
        let mut s = store(&["a"]);
        assert_eq!(
            s.apply("session:created", json!({ "id": 1 })),
            Change::Nothing
        );
        assert_eq!(s.apply("session:reordered", json!({})), Change::Nothing);
        assert_eq!(s.apply("nope", Value::Null), Change::Nothing);
        assert_eq!(ids(&s), ["a"]);
    }
}
