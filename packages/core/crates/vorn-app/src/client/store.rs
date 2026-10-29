//! What the main screen shows, as vornd last told it: the config, projects,
//! sessions, tasks and workflows, loaded once and then kept current by
//! folding notifications in.
//!
//! vornd's configuration carries neither tasks nor workflows; the app keeps
//! them beside its copy of it, under `tasks` and `workflows`, so the screens
//! read one value, and [`super::sync`] carries a change to them back.

use std::collections::{HashMap, HashSet};

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
    "layout:opened",
    "task:rows",
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

/// One terminal session and what is known about how it ended.
#[derive(Debug, Clone)]
pub struct Session {
    pub info: TerminalSession,
    /// The exit code once its program has ended; the card stays until closed.
    pub exited: Option<i64>,
    /// Why a session carried over from the last run ended, when it did.
    pub cause: Option<&'static str>,
    /// Its place among its project's unnamed sessions, from 0 ([`Store::number_titles`]).
    pub ordinal: usize,
}

impl Session {
    /// The name given, else the project's, numbered from its second unnamed session on (`vorn 2`).
    pub fn title(&self) -> String {
        match self.own_name() {
            Some(name) => name.to_owned(),
            None if self.ordinal > 0 => format!("{} {}", self.info.project_name, self.ordinal + 1),
            None => self.info.project_name.clone(),
        }
    }

    /// The name it was given, if any.
    fn own_name(&self) -> Option<&str> {
        let name = self.info.display_name.as_deref().map(str::trim)?;
        (!name.is_empty()).then_some(name)
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
    /// A workflow run moved.
    Runs,
    /// Tasks were written without saying which: the list is read again.
    TasksStale,
}

/// How many tasks one read of the core's task list takes.
pub const TASK_PAGE: u64 = 1000;

/// The params of the task list's page from `offset`.
pub fn task_page(offset: u64) -> Value {
    json!({ "includeDescription": true, "offset": offset, "limit": TASK_PAGE })
}

/// The offset of the page after `got`, a page read from `offset`; `None` after the last.
pub fn next_page(offset: u64, got: &Value) -> Option<u64> {
    let n = got.as_array().map_or(0, Vec::len) as u64;
    (n == TASK_PAGE).then_some(offset + n)
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
    /// Raised each time [`Store::tasks`] changes, so what is drawn from them can be kept.
    pub tasks_gen: u64,
    pub workflows: Vec<WorkflowDefinition>,
    /// The active workspace's session groups, in their order.
    pub groups: Vec<SessionGroupConfig>,
    /// The latest of every run vornd reported since loading, by run id.
    pub runs: HashMap<String, Value>,
    /// Whether vornd has said which sessions run: asked after connecting, since
    /// it waits for the session holder while vornd starts.
    pub sessions_known: bool,
    /// The shells a card's panel holds: off the grid, so left out of [`Store::number_titles`].
    pub held: HashSet<String>,
    /// The agents a session can start, from their adapters.
    pub agents: crate::agents::Agents,
    /// Sessions taken off the screen, whose updates still on their way are
    /// not to bring them back: vornd says nothing once one is gone.
    gone: HashSet<String>,
}

fn subscription() -> Value {
    json!({ "topics": TOPICS, "terminalBytes": false })
}

/// The loads [`Store::load`] waits on, sent together.
fn load_calls(rpc: &Rpc) -> [Pending; 6] {
    [
        rpc.request("config:load", Value::Null),
        rpc.request("project:list", Value::Null),
        rpc.request("task:list", task_page(0)),
        rpc.request("workflow:list", Value::Null),
        rpc.request("workflowRun:listWaiting", Value::Null),
        rpc.request("agent:list", Value::Null),
    ]
}

impl Store {
    /// Subscribes to [`TOPICS`] and loads everything, the calls in flight
    /// together. Subscribing first means nothing is missed between the two.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(rpc: &Rpc) -> Result<Store, RpcError> {
        rpc.call("subscribe:set", subscription())?;
        let mut answers = load_calls(rpc).map(|p| p.wait(CALL_TIMEOUT));
        // The board is read a page at a time, the rest after the first.
        if let Ok(first) = &mut answers[2] {
            let mut at = next_page(0, first);
            while let Some(offset) = at {
                let page = rpc.call("task:list", task_page(offset))?;
                at = next_page(offset, &page);
                if let (Value::Array(all), Value::Array(more)) = (&mut *first, page) {
                    all.extend(more);
                }
            }
        }
        Store::from_answers(answers)
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
        if let Some(Ok(first)) = answers.get_mut(2) {
            let mut at = next_page(0, first);
            while let Some(offset) = at {
                let page = rpc
                    .request("task:list", task_page(offset))
                    .answer(CALL_TIMEOUT)
                    .await?;
                at = next_page(offset, &page);
                if let (Value::Array(all), Value::Array(more)) = (&mut *first, page) {
                    all.extend(more);
                }
            }
        }
        let answers: [Result<Value, RpcError>; 6] = answers
            .try_into()
            .map_err(|_| RpcError::Decode("missing answers".into()))?;
        Store::from_answers(answers)
    }

    fn from_answers(answers: [Result<Value, RpcError>; 6]) -> Result<Store, RpcError> {
        let [config, projects, tasks, workflows, waiting, agents] = answers;
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
        if let Ok(agents) = agents {
            store.agents = crate::agents::Agents::from_json(&agents);
        }
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

    /// The agent the composer starts: `defaults.defaultAgent` while it is on,
    /// else the first agent that is, else a shell.
    pub fn default_agent(&self) -> &str {
        self.config
            .pointer("/defaults/defaultAgent")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty())
            .filter(|a| *a == "shell" || self.agents.get(a).is_some())
            .unwrap_or_else(|| self.agents.default_key())
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

    /// The active workspace's projects, in order.
    pub fn workspace_projects(&self) -> impl Iterator<Item = &ProjectConfig> {
        self.projects.iter().filter(|p| self.in_workspace(p))
    }

    /// Whether a session's project is in the active workspace, which is
    /// what any surface shows. A session outside every
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

    /// Keeps a run as it stands now. What waits on a person is not counted
    /// here: the extensions that own it say so as attention items.
    fn run_updated(&mut self, run: &Value) -> bool {
        let Some(id) = run.get("runId").and_then(Value::as_str) else {
            return false;
        };
        self.runs.insert(id.to_owned(), run.clone());
        true
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
                ordinal: 0,
            }),
        }
        self.number_titles();
    }

    /// Takes the session with `id` off the screen.
    pub fn remove(&mut self, id: &str) {
        self.sessions.retain(|s| s.info.id != id);
        self.gone.insert(id.to_owned());
        self.number_titles();
    }

    /// Takes the session with `id` off the screen while it may still live: its next update brings it back.
    pub fn forget(&mut self, id: &str) {
        self.sessions.retain(|s| s.info.id != id);
        self.number_titles();
    }

    /// Numbers each project's unnamed sessions by creation, so no two cards share a title.
    pub fn number_titles(&mut self) {
        let mut order: Vec<usize> = (0..self.sessions.len())
            .filter(|&i| {
                let s = &self.sessions[i];
                s.own_name().is_none() && !self.held.contains(&s.info.id)
            })
            .collect();
        order.sort_by(|&a, &b| {
            let (a, b) = (&self.sessions[a].info, &self.sessions[b].info);
            (a.project_name.cmp(&b.project_name)).then(a.created_at.total_cmp(&b.created_at))
        });
        for s in &mut self.sessions {
            s.ordinal = 0;
        }
        for pair in order.windows(2) {
            let (prev, i) = (&self.sessions[pair[0]], &self.sessions[pair[1]]);
            if prev.info.project_name == i.info.project_name {
                self.sessions[pair[1]].ordinal = prev.ordinal + 1;
            }
        }
    }

    /// Takes `held` as the panels' shells, numbering the titles again when it changed.
    pub fn hold(&mut self, held: HashSet<String>) {
        if held != self.held {
            self.held = held;
            self.number_titles();
        }
    }

    /// Folds one notification in.
    pub fn apply(&mut self, method: &str, params: Value) -> Change {
        match method {
            "session:created" | "session:updated" => {
                match serde_json::from_value::<TerminalSession>(params) {
                    // Created again (resumed under its id) is back; updated is not.
                    Ok(info) if method == "session:created" || !self.gone.contains(&info.id) => {
                        self.gone.remove(&info.id);
                        self.upsert(info);
                        Change::Sessions
                    }
                    Ok(_) | Err(_) => Change::Nothing,
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
                self.number_titles();
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
                    Change::Runs
                } else {
                    Change::Nothing
                }
            }
            "config:changed" => {
                let mut params = params;
                // vornd's configuration carries no tasks or workflows: the app's copies stay,
                // moved across rather than copied, as they have not changed.
                let carried = params.is_object() && params.get("tasks").is_none();
                for key in ["tasks", "workflows"] {
                    if params.is_object() && params.get(key).is_none() {
                        params[key] = self
                            .config
                            .get_mut(key)
                            .map(Value::take)
                            .unwrap_or(json!([]));
                    }
                }
                let was = std::mem::replace(&mut self.config, params);
                self.projects = records_in(self.config.get("projects"));
                if !carried {
                    let tasks = std::mem::take(&mut self.tasks);
                    self.tasks = records_since(self.config.get("tasks"), was.get("tasks"), tasks);
                    self.tasks_gen += 1;
                }
                self.workflows = records_in(self.config.get("workflows"));
                self.groups = groups(&self.config);
                Change::Config
            }
            "task:rows" => self.task_rows(&params),
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
    /// `task:rows`: the rows a write changed, put in place or added last, and
    /// the tasks gone; or word that the list is to be read again.
    fn task_rows(&mut self, params: &Value) -> Change {
        if !self.config.is_object() {
            return Change::Nothing;
        }
        if params.get("reload").and_then(Value::as_bool) == Some(true) {
            return Change::TasksStale;
        }
        let mut touched: Vec<String> = Vec::new();
        for row in params
            .get("upsert")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };
            let list = tasks_list(&mut self.config);
            match list
                .iter_mut()
                .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
            {
                Some(t) => *t = row.clone(),
                None => list.push(row.clone()),
            }
            touched.push(id.to_owned());
        }
        for id in params
            .get("remove")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            tasks_list(&mut self.config)
                .retain(|t| t.get("id").and_then(Value::as_str) != Some(id));
            touched.push(id.to_owned());
        }
        if touched.is_empty() {
            return Change::Nothing;
        }
        for id in &touched {
            self.reread_task(id);
        }
        Change::Config
    }

    /// Replaces every task with `list`, a fresh read of the core's list.
    pub fn set_tasks(&mut self, list: Value) {
        if !self.config.is_object() || !list.is_array() {
            return;
        }
        self.tasks = records_in(Some(&list));
        self.config["tasks"] = list;
        self.tasks_gen += 1;
    }

    /// Rereads the task `id` from `config` after a change there: changed,
    /// added (last, as it was added) or gone.
    pub fn reread_task(&mut self, id: &str) {
        self.tasks_gen += 1;
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

/// The configuration's task list, made one if it is missing.
fn tasks_list(config: &mut Value) -> &mut Vec<Value> {
    if !config.get("tasks").is_some_and(Value::is_array) {
        config["tasks"] = json!([]);
    }
    match config.get_mut("tasks") {
        Some(Value::Array(list)) => list,
        _ => unreachable!("tasks was just made an array"),
    }
}

/// The workspace a config starts with.
pub fn default_workspace() -> WorkspaceConfig {
    WorkspaceConfig {
        id: WORKSPACE.to_owned(),
        name: "Personal".to_owned(),
        icon: Some("User".to_owned()),
        icon_color: Some("#6b7280".to_owned()),
        order: 0.0,
    }
}

/// The id of the workspace every config has.
pub const WORKSPACE: &str = "personal";

/// `defaults.activeWorkspace`, else `personal`.
fn active_workspace(config: &Value) -> &str {
    config
        .pointer("/defaults/activeWorkspace")
        .and_then(Value::as_str)
        .filter(|w| !w.is_empty())
        .unwrap_or(WORKSPACE)
}

/// The active workspace's groups by `order`.
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
    fn unnamed_sessions_of_one_project_are_numbered_by_creation() {
        let mut s = Store::default();
        let make = |id: &str, project: &str, at: f64| {
            json!({ "id": id, "agentType": "claude", "projectName": project,
                    "projectPath": "/p", "status": "running", "createdAt": at, "pid": 1.0 })
        };
        // Listed out of creation order, as a reorder leaves them.
        s.apply("session:created", make("b", "vorn", 2.0));
        s.apply("session:created", make("a", "vorn", 1.0));
        s.apply("session:created", make("j", "journey", 3.0));
        s.apply("session:created", make("c", "vorn", 4.0));
        let titles = |s: &Store| s.sessions.iter().map(Session::title).collect::<Vec<_>>();
        assert_eq!(titles(&s), ["vorn 2", "vorn", "journey", "vorn 3"]);
        // A renamed session leaves the count; the rest close up.
        let mut named = make("a", "vorn", 1.0);
        named["displayName"] = json!("docs");
        s.apply("session:updated", named);
        assert_eq!(titles(&s), ["vorn", "docs", "journey", "vorn 2"]);
        s.remove("b");
        assert_eq!(titles(&s), ["docs", "journey", "vorn"]);
    }

    #[test]
    fn a_shell_a_panel_holds_takes_no_number() {
        let mut s = Store::default();
        let make = |id: &str, at: f64| {
            json!({ "id": id, "agentType": "claude", "projectName": "vorn",
                    "projectPath": "/p", "status": "running", "createdAt": at, "pid": 1.0 })
        };
        s.apply("session:created", make("a", 1.0));
        s.apply("session:created", make("panel", 2.0));
        s.apply("session:created", make("b", 3.0));
        s.hold(HashSet::from(["panel".to_owned()]));
        let titles = |s: &Store| s.sessions.iter().map(Session::title).collect::<Vec<_>>();
        assert_eq!(titles(&s), ["vorn", "vorn", "vorn 2"]);
        // Closing the panel puts its shell back in the count.
        s.hold(HashSet::new());
        assert_eq!(titles(&s), ["vorn", "vorn 2", "vorn 3"]);
    }

    #[test]
    fn a_removed_session_is_not_brought_back_by_a_late_update() {
        let mut s = store(&["a", "b"]);
        s.remove("a");
        assert_eq!(
            s.apply("session:updated", session("a", "exited")),
            Change::Nothing
        );
        assert_eq!(ids(&s), ["b"]);
        s.apply("session:created", session("a", "running"));
        s.apply("session:updated", session("a", "waiting"));
        assert_eq!(ids(&s), ["b", "a"]);
        assert_eq!(s.sessions[1].status(), "waiting");
        s.forget("b");
        s.apply("session:updated", session("b", "running"));
        assert_eq!(ids(&s), ["a", "b"]);
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
    fn a_run_is_kept_as_it_stands() {
        let mut s = Store::default();
        let run = |states: Value| json!({ "runId": "r", "nodeStates": states });
        let gate = json!({ "nodeId": "a", "status": "waiting" });
        assert_eq!(
            s.apply("workflow:runUpdated", run(json!([gate]))),
            Change::Runs
        );
        assert_eq!(s.runs["r"]["nodeStates"][0]["status"], "waiting");
        assert_eq!(s.apply("workflow:runUpdated", json!({})), Change::Nothing);
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
        s.set_tasks(json!([task("a", "A")]));
        s.apply(
            "workflow:changed",
            json!([{ "id": "w", "name": "W", "icon": "Zap",
            "iconColor": "#fff", "nodes": [], "edges": [], "enabled": true }]),
        );
        assert_eq!(titles(&s), ["A"]);
        assert_eq!(s.workflows.len(), 1);
        // vornd's configuration has neither: the copies stay, and the tasks are not read again.
        let gen = s.tasks_gen;
        s.apply(
            "config:changed",
            json!({ "projects": [], "defaults": { "fontSize": 14 } }),
        );
        assert_eq!(titles(&s), ["A"]);
        assert_eq!(s.tasks_gen, gen);
        assert_eq!(s.workflows.len(), 1);
        assert_eq!(s.config["tasks"][0]["id"], "a");
        assert_eq!(s.config["workflows"][0]["id"], "w");
    }

    #[test]
    fn the_rows_a_write_changed_come_in_place_new_ones_last() {
        let mut s = Store::default();
        s.apply("config:changed", json!({ "projects": [] }));
        s.set_tasks(json!([task("a", "A"), task("b", "B")]));
        let gen = s.tasks_gen;
        let told = s.apply(
            "task:rows",
            json!({ "upsert": [task("a", "A2"), task("c", "C")] }),
        );
        assert_eq!(told, Change::Config);
        assert_eq!(titles(&s), ["A2", "B", "C"]);
        assert!(s.tasks_gen > gen);
        s.apply("task:rows", json!({ "remove": ["b", "ghost"] }));
        assert_eq!(titles(&s), ["A2", "C"]);
        assert_eq!(s.config["tasks"].as_array().unwrap().len(), 2);
        assert_eq!(
            s.apply("task:rows", json!({ "reload": true })),
            Change::TasksStale
        );
        assert_eq!(s.apply("task:rows", json!({})), Change::Nothing);
        assert_eq!(next_page(0, &json!([])), None);
        let full = Value::Array(vec![json!({}); TASK_PAGE as usize]);
        assert_eq!(next_page(TASK_PAGE, &full), Some(2 * TASK_PAGE));
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
        assert_eq!(s.default_agent(), "shell");
        s.agents = crate::agents::testing::agents();
        assert_eq!(s.default_agent(), "acme");
        let config = json!({
            "defaults": { "defaultAgent": "beta" },
            "projects": [{ "name": "web", "path": "/w", "preferredAgents": [] }, { "bad": 1 }],
        });
        assert_eq!(s.apply("config:changed", config), Change::Config);
        assert_eq!(s.default_agent(), "beta");
        s.agents = crate::agents::Agents::new(Vec::new());
        assert_eq!(s.default_agent(), "shell");
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
