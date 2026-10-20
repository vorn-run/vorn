//! What vornd puts behind the bridge's gate: the core methods a program may
//! call, the network `net.fetch` reaches, and the vault its credentials come
//! from. The gate has already checked every call against the manifest.

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tracing::info;
use vorn_connectors::fetch::{Http, Request as HttpRequest};
use vorn_extensions::api::types::{Header, Request, Response, SessionStart};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{Answer, Caller, StorageCall, World};
use vorn_store::extension::{ExtensionTables, Grant};

use crate::native::config::Viewer;
use crate::native::{secrets::Known, Answer as NativeAnswer, Native};

/// How long one `net.fetch` may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// vornd's world: its session records, git, the network, the vault and the extensions' tables.
pub struct Wired {
    pub native: Weak<Native>,
}

/// Each extension's tables on a connection of its own, opened at its first
/// storage call and kept: opening one per call cost each of Tasks' writes
/// about a millisecond. Calls on one extension's tables take turns.
#[derive(Default)]
pub struct Tables {
    open: std::sync::Mutex<std::collections::HashMap<String, Kept>>,
}

/// One extension's tables, opened on the blocking thread of their first call.
pub type Kept = Arc<std::sync::Mutex<Option<ExtensionTables>>>;

impl std::fmt::Debug for Tables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tables").finish_non_exhaustive()
    }
}

impl Tables {
    /// Where extension `id`'s tables are kept, not yet open before its first call.
    pub fn of(&self, id: &str) -> Kept {
        let mut open = self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(open.entry(id.to_owned()).or_default())
    }

    /// Closes extension `id`'s tables, as it is removed.
    pub fn forget(&self, id: &str) {
        self.open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }
}

/// The core's tables extension `id` is lent: Tasks keeps the board's own.
fn lent(id: &str) -> &'static [Grant] {
    if id == super::builtin::tasks::ID {
        vorn_store::extension::TASKS
    } else {
        &[]
    }
}

/// `files.read` with `encoding: base64`: the file's bytes, `{base64}`.
async fn read_bytes(path: String) -> Result<Value, ExtError> {
    tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .map_err(|e| ExtError::new("failed", e.to_string()))?
        .map(|bytes| json!({ "base64": vorn_extensions::api::base64::encode(&bytes) }))
        .map_err(|e| ExtError::new("failed", e.to_string()))
}

/// `task.changed`: the events a task's write fires, emitted as the core's,
/// then the rows it changed or removed, told to every client.
async fn task_changed(native: &Arc<Native>, args: &Value) {
    use vorn_extensions::events::{TASK_CREATED, TASK_STATUS_CHANGED};
    for e in args
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = match e.get("kind").and_then(Value::as_str) {
            Some("taskCreated") => TASK_CREATED,
            _ => TASK_STATUS_CHANGED,
        };
        native.events().emit(name, || e.clone());
    }
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    let change = match (args.get("rows"), args.get("removed")) {
        (Some(Value::Array(rows)), _) => crate::native::task_list::Change::Rows(rows.clone()),
        (_, Some(removed)) => crate::native::task_list::Change::Removed(strings(removed)),
        _ => return,
    };
    crate::native::task_list::told(native, change).await;
}

fn stopping() -> ExtError {
    ExtError::new("stopping", "vornd is stopping")
}

/// The secret `field` of `connection`, which must be `owner`'s.
async fn secret_of(
    native: Option<Arc<Native>>,
    owner: String,
    connection: String,
    field: String,
) -> Result<Option<String>, ExtError> {
    let native = native.ok_or_else(stopping)?;
    let connectors = native
        .connectors()
        .cloned()
        .ok_or_else(|| ExtError::new("signed-out", "vornd holds no connections"))?;
    if connectors.inner().owner_of(&connection).await.as_deref() != Some(owner.as_str()) {
        return Err(ExtError::new(
            "denied",
            format!("the connection {connection} is not {owner}'s"),
        ));
    }
    tokio::task::spawn_blocking(move || match native.secrets.lookup(&connection) {
        Known::Fields(fields) => fields.get(&field).map(|v| v.expose().to_owned()),
        Known::None | Known::Unknown => None,
    })
    .await
    .map_err(|e| ExtError::new("failed", e.to_string()))
}

/// A client's call answered inside vornd, for a core method the gate allowed.
async fn ask(native: &Arc<Native>, method: &str, params: Value) -> Result<Value, ExtError> {
    match native
        .answer(method.to_owned(), params, &Viewer::Local)
        .await
    {
        NativeAnswer::Result(v) => Ok(v),
        NativeAnswer::Void => Ok(Value::Null),
        NativeAnswer::Error(message) => Err(ExtError::new("failed", message)),
        NativeAnswer::Unanswered => Err(ExtError::new(
            "unsupported",
            format!("this vornd does not answer {method}"),
        )),
    }
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn needs(method: &str, what: &str) -> ExtError {
    ExtError::new("bad-request", format!("{method} takes {what}"))
}

/// Runs `f` on the store off the runtime, a failure the call's.
async fn on_store(
    native: &Arc<Native>,
    method: &str,
    f: impl FnOnce(&mut vorn_store::Store) -> vorn_store::Result<Value> + Send + 'static,
) -> Result<Value, ExtError> {
    let n = Arc::clone(native);
    let answered = tokio::task::spawn_blocking(move || crate::native::config::with_store(&n, f))
        .await
        .map_err(|e| ExtError::new("failed", e.to_string()))?;
    answered.map_err(|e| ExtError::new("failed", format!("{method}: {e}")))
}

/// Starts `req` for extension `extension` the way the app and the MCP tools
/// do: a shell through `shell:create`, an agent through `terminal:create`, or
/// `headless:create` naming the extension as what started it.
async fn start_session(
    native: &Arc<Native>,
    extension: &str,
    req: SessionStart,
) -> Result<String, ExtError> {
    let project = req.project.trim().to_owned();
    if !std::path::Path::new(&project).is_absolute() {
        return Err(ExtError::new(
            "bad-request",
            format!("a session starts in a project's absolute folder, not \"{project}\""),
        ));
    }
    let given = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let (agent, prompt, branch, name) = (
        given(req.agent).filter(|a| a != "shell"),
        given(req.prompt),
        given(req.branch),
        given(req.name),
    );
    let started = match agent {
        None => {
            if req.worktree || req.headless || prompt.is_some() || branch.is_some() {
                return Err(ExtError::new(
                    "bad-request",
                    "a shell starts in the project's folder; a worktree, a branch, a prompt and headless are for an agent",
                ));
            }
            let mut params = json!({ "cwd": project });
            if let Some(name) = name {
                params["displayName"] = json!(name);
            }
            ask(native, "shell:create", params).await?
        }
        Some(agent) => {
            let project_name = std::path::Path::new(&project)
                .file_name()
                .map_or_else(|| project.clone(), |n| n.to_string_lossy().into_owned());
            let mut params = json!({
                "agentType": agent, "projectName": project_name, "projectPath": project,
            });
            for (key, value) in [
                ("initialPrompt", prompt),
                ("branch", branch),
                ("displayName", name),
            ] {
                if let Some(v) = value {
                    params[key] = json!(v);
                }
            }
            if req.worktree {
                params["useWorktree"] = json!(true);
            }
            if req.headless {
                params["origin"] = json!({ "extension": extension, "id": "", "label": extension });
                ask(native, "headless:create", params).await?
            } else {
                ask(native, "terminal:create", params).await?
            }
        }
    };
    started
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ExtError::new("failed", "Vorn started no session"))
}

/// `sessions.launch`'s args as the session to start, its project the call's when it names none.
fn launch_args(args: &Value, project: Option<String>) -> Result<SessionStart, ExtError> {
    let project = project.ok_or_else(|| needs("sessions.launch", "the project to start it in"))?;
    let flag = |k: &str| args.get(k).and_then(Value::as_bool).unwrap_or(false);
    Ok(SessionStart {
        project,
        agent: text(args, "agent"),
        prompt: text(args, "prompt"),
        worktree: flag("worktree"),
        branch: text(args, "branch"),
        name: text(args, "name"),
        headless: flag("headless"),
    })
}

/// A running session as `sessions.list` and `sessions.get` describe it.
fn session_json(t: &crate::registry::TerminalSession) -> Value {
    let name = t
        .display_name
        .as_deref()
        .filter(|n| !n.is_empty())
        .unwrap_or(&t.project_name);
    json!({
        "id": t.id, "agent": t.agent_type, "status": t.status, "name": name,
        "projectPath": t.project_path, "worktreePath": t.worktree_path,
    })
}

/// `worktrees.list`: every worktree of `project`, or of every project, as the inventory scans it.
async fn worktrees(
    native: &Arc<Native>,
    project: Option<String>,
    refresh: bool,
) -> Result<Value, ExtError> {
    let params = json!({ "projectPaths": project.map(|p| vec![p]), "refresh": refresh });
    let inventory = ask(native, "worktree:inventory", params).await?;
    let mut out = Vec::new();
    for p in inventory["projects"].as_array().into_iter().flatten() {
        for e in p["entries"].as_array().into_iter().flatten() {
            let reasons: Vec<&str> = e["verdict"]["reasons"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            out.push(json!({
                "projectPath": p["projectPath"], "projectName": p["projectName"],
                "base": p["defaultBranch"], "path": e["path"], "name": e["name"], "branch": e["branch"],
                "main": e["isMain"], "orphan": e["kind"] == "orphan-dir",
                "dirty": e["isDirty"], "merged": e["isMerged"],
                "sizeBytes": e["sizeBytes"], "artifactBytes": e["artifactBytes"],
                "idleDays": e["idleDays"],
                "sessionIds": e["activeSessionIds"],
                "verdict": e["verdict"]["level"], "reasons": reasons.join("; "),
            }));
        }
    }
    Ok(Value::Array(out))
}

impl World for Wired {
    fn call(&self, caller: &Caller, method: &str, args: Value) -> Answer<Value> {
        let native = self.native.upgrade();
        let method = method.to_owned();
        let extension = caller.extension.clone();
        let project = text(&args, "project").or_else(|| caller.project.clone());
        let weak = self.native.clone();
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            let m = method.as_str();
            if vorn_extensions::gate::BUILTIN_ONLY.contains(&m)
                && !super::builtin::SHIPPED.contains(&extension.as_str())
            {
                return Err(ExtError::new(
                    "denied",
                    format!("{m} is open only to the extensions Vorn ships"),
                ));
            }
            match m {
                "sessions.list" => {
                    let sessions = native
                        .registry
                        .get()
                        .and_then(|r| {
                            r.read(|r| r.live_terminals().map(session_json).collect::<Vec<_>>())
                        })
                        .unwrap_or_default();
                    Ok(Value::Array(sessions))
                }
                "sessions.get" => {
                    let id = text(&args, "sessionId")
                        .ok_or_else(|| needs(m, "the sessionId to read"))?;
                    native
                        .registry
                        .get()
                        .and_then(|r| {
                            r.read(|r| r.live_terminals().find(|t| t.id == id).map(session_json))
                        })
                        .flatten()
                        .ok_or_else(|| {
                            ExtError::new("not-found", format!("Session not found: {id}"))
                        })
                }
                "sessions.launch" => {
                    let req = launch_args(&args, project)?;
                    let id = start_session(&native, &extension, req).await?;
                    Ok(json!({ "sessionId": id }))
                }
                "terminal.read" | "terminal.send" | "terminal.selection" | "card.rename"
                | "agent.usage" => {
                    let host = native.extensions().cloned().ok_or_else(|| {
                        ExtError::new("unsupported", "vornd is not hosting extensions")
                    })?;
                    super::routes::session_call(&host, &extension, m, &args)
                        .await
                        .map_err(|e| ExtError::new("failed", e))
                }
                "git.status" => {
                    let project = project.ok_or_else(|| needs(m, "the project to read"))?;
                    let git = vorn_git::repo::Git {
                        bin: native.env.git_bin(),
                        env: native.env.get(),
                        ssh: None,
                    };
                    tokio::task::spawn_blocking(move || {
                        let dir = PathBuf::from(&project);
                        json!({ "branch": git.branch(&dir), "dirty": git.is_worktree_dirty(&dir) })
                    })
                    .await
                    .map_err(|e| ExtError::new("failed", e.to_string()))
                }
                "sessions.close" => {
                    let id = text(&args, "sessionId")
                        .ok_or_else(|| needs(m, "the sessionId to close"))?;
                    ask(&native, "terminal:kill", json!(id)).await
                }
                "worktrees.list" => {
                    let refresh = args.get("refresh").and_then(Value::as_bool) == Some(true);
                    worktrees(&native, text(&args, "project"), refresh).await
                }
                "worktrees.reclaim" | "worktrees.prune" => {
                    let path =
                        text(&args, "worktree").ok_or_else(|| needs(m, "the worktree to clean"))?;
                    let method = if m == "worktrees.reclaim" {
                        "worktree:reclaimArtifacts"
                    } else {
                        "worktree:pruneOrphans"
                    };
                    ask(&native, method, json!({ "paths": [path] })).await
                }
                "worktrees.remove" => {
                    let worktree = text(&args, "worktree")
                        .ok_or_else(|| needs(m, "the worktree to remove"))?;
                    let project = project.ok_or_else(|| needs(m, "the project it belongs to"))?;
                    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
                    let params =
                        json!({ "projectPath": project, "worktreePath": worktree, "force": force });
                    let removed = ask(&native, "git:removeWorktree", params).await?;
                    Ok(json!({ "removed": removed.as_bool() == Some(true) }))
                }
                "permissions.resolve" => {
                    let id = text(&args, "requestId")
                        .ok_or_else(|| needs(m, "the requestId to answer"))?;
                    let allow = args.get("allow").and_then(Value::as_bool).unwrap_or(false);
                    // A rule is picked among those the agent offered, never written.
                    let mut answer = json!({ "requestId": id, "allow": allow });
                    if let Some(i) = args.get("suggestion").and_then(Value::as_u64) {
                        answer["suggestion"] = json!(i);
                    }
                    if let Some(a) = args.get("answers").filter(|a| a.is_object()) {
                        answer["answers"] = a.clone();
                    }
                    ask(&native, "permission:resolve", answer).await
                }
                "git.diff" => {
                    let dir = text(&args, "worktree")
                        .or(project)
                        .ok_or_else(|| needs(m, "the worktree to read"))?;
                    ask(&native, "git:diffFull", json!(dir)).await
                }
                "git.commit" => {
                    let dir = text(&args, "worktree")
                        .or(project)
                        .ok_or_else(|| needs(m, "the worktree to commit in"))?;
                    let message =
                        text(&args, "message").ok_or_else(|| needs(m, "the commit message"))?;
                    let unstaged = args
                        .get("includeUnstaged")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let params =
                        json!({ "cwd": dir, "message": message, "includeUnstaged": unstaged });
                    ask(&native, "git:commit", params).await
                }
                "files.read" => {
                    let path =
                        text(&args, "path").ok_or_else(|| needs(m, "the absolute path to read"))?;
                    if text(&args, "encoding").as_deref() == Some("base64") {
                        return read_bytes(path).await;
                    }
                    let mut params = json!({ "filePath": path });
                    if let Some(max) = args.get("maxBytes").filter(|v| v.is_u64()) {
                        params["maxBytes"] = max.clone();
                    }
                    ask(&native, "file:readContent", params).await
                }
                "files.list" => {
                    let dir = text(&args, "path")
                        .ok_or_else(|| needs(m, "the absolute folder to list"))?;
                    ask(&native, "file:listDir", json!({ "dirPath": dir })).await
                }
                "files.stat" => {
                    let path =
                        text(&args, "path").ok_or_else(|| needs(m, "the absolute path to read"))?;
                    ask(&native, "file:stamp", json!({ "filePath": path })).await
                }
                "shell.executables" => ask(&native, "shell:listExecutables", Value::Null).await,
                "git.branches" => {
                    let dir = text(&args, "worktree")
                        .or(project)
                        .ok_or_else(|| needs(m, "the repository to read"))?;
                    ask(&native, "git:listBranches", json!(dir)).await
                }
                "files.write" => {
                    let path = text(&args, "path")
                        .ok_or_else(|| needs(m, "the absolute path to write"))?;
                    let content = args
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| needs(m, "the content to write"))?;
                    ask(
                        &native,
                        "file:writeContent",
                        json!({ "filePath": path, "content": content }),
                    )
                    .await
                }
                "secrets.get" => {
                    let connection = text(&args, "connection")
                        .ok_or_else(|| needs(m, "the connection to read"))?;
                    let field =
                        text(&args, "field").ok_or_else(|| needs(m, "the field to read"))?;
                    let value = secret_of(weak.upgrade(), extension, connection, field).await?;
                    Ok(json!({ "value": value }))
                }
                "task.take" | "task.handBack" => {
                    let tasks = native
                        .tasks()
                        .cloned()
                        .ok_or_else(|| ExtError::new("unsupported", "this vornd keeps no tasks"))?;
                    tasks.api(m, args).await
                }
                "task.changed" => {
                    task_changed(&native, &args).await;
                    Ok(Value::Null)
                }
                "connection.itemImport" | "connection.synced" => {
                    let connectors = native.connectors().cloned().ok_or_else(|| {
                        ExtError::new("unsupported", "this vornd keeps no connections")
                    })?;
                    connectors.items(m, &args).await
                }
                "workflow.fire" => {
                    let work = native.work().cloned().ok_or_else(|| {
                        ExtError::new("unsupported", "this vornd runs no workflows")
                    })?;
                    work.fire_api(&extension, &args)
                        .await
                        .map_err(|e| ExtError::new("failed", e))
                }
                "task.importItems" | "task.sourceLink" => {
                    let tasks = native
                        .tasks()
                        .cloned()
                        .ok_or_else(|| ExtError::new("unsupported", "this vornd keeps no tasks"))?;
                    tasks.api(m, args).await
                }
                "task.list" => {
                    on_store(&native, m, move |s| {
                        crate::native::task_list::page(s, &args)
                    })
                    .await
                }
                "task.get" => {
                    let id = text(&args, "id").ok_or_else(|| needs(m, "the id of the task"))?;
                    on_store(&native, m, move |s| s.call("dbGetTask", json!([id]))).await
                }
                "agents.list" => ask(&native, "agent:list", Value::Null).await,
                "project.list" => {
                    on_store(&native, m, |s| s.call("dbListProjects", json!([]))).await
                }
                "workflow.get" => {
                    let id = text(&args, "workflowId")
                        .ok_or_else(|| needs(m, "the workflowId to read"))?;
                    on_store(&native, m, move |s| s.call("dbGetWorkflow", json!([id]))).await
                }
                "workflow.list" => {
                    on_store(&native, m, |s| s.call("dbListWorkflows", json!([]))).await
                }
                "workflow.gates" => {
                    let waiting = ask(&native, "workflowRun:listWaiting", Value::Null).await?;
                    let workflows = ask(&native, "workflow:list", Value::Null).await?;
                    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
                    Ok(Value::Array(crate::native::work::waiting::all_rows(
                        &list(&waiting),
                        &list(&workflows),
                    )))
                }
                "workflow.run" => {
                    let id = text(&args, "workflowId")
                        .ok_or_else(|| needs(m, "the workflowId to run"))?;
                    let inputs = args.get("inputs").cloned().unwrap_or(Value::Null);
                    ask(
                        &native,
                        "workflow:runManual",
                        json!({ "workflowId": id, "inputs": inputs }),
                    )
                    .await
                }
                "workflow.resolveGate" => {
                    let run = text(&args, "runId").ok_or_else(|| needs(m, "the runId"))?;
                    let node = text(&args, "nodeId").ok_or_else(|| needs(m, "the nodeId"))?;
                    let decision = text(&args, "decision")
                        .ok_or_else(|| needs(m, "the decision: approve or reject"))?;
                    ask(
                        &native,
                        "workflow:resolveGate",
                        json!({ "runId": run, "nodeId": node, "decision": decision }),
                    )
                    .await
                }
                "workflow.seed" => {
                    let workflows = args
                        .get("workflows")
                        .and_then(Value::as_array)
                        .cloned()
                        .ok_or_else(|| needs(m, "the workflows to seed"))?;
                    on_store(&native, m, move |s| {
                        let mut created = Vec::with_capacity(workflows.len());
                        for wf in workflows {
                            let id = wf.get("id").cloned().unwrap_or(Value::Null);
                            let fresh = s.call("dbGetWorkflow", json!([id]))?.is_null();
                            if fresh {
                                s.call("dbInsertWorkflow", json!([wf]))?;
                            }
                            created.push(Value::Bool(fresh));
                        }
                        Ok(json!({ "created": created }))
                    })
                    .await
                }
                "workflow.unseed" => {
                    let prefix = text(&args, "prefix")
                        .filter(|p| p.ends_with(':'))
                        .ok_or_else(|| {
                            needs(m, "the prefix its workflows' ids start with, ending in :")
                        })?;
                    on_store(&native, m, move |s| {
                        let workflows = s.call("dbListWorkflows", json!([]))?;
                        let mut removed = 0u64;
                        for wf in workflows.as_array().into_iter().flatten() {
                            if let Some(id) = wf
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|i| i.starts_with(&prefix))
                            {
                                s.call("dbDeleteWorkflow", json!([id]))?;
                                removed += 1;
                            }
                        }
                        Ok(json!({ "removed": removed }))
                    })
                    .await
                }
                "inbox.cursor" => {
                    let (wf, conn) = (
                        text(&args, "workflowId").ok_or_else(|| needs(m, "the workflowId"))?,
                        text(&args, "connectionId").ok_or_else(|| needs(m, "the connectionId"))?,
                    );
                    on_store(&native, m, move |s| {
                        s.call("dbGetConnectorPollCursor", json!([wf, conn]))
                    })
                    .await
                }
                "inbox.recordPage" => {
                    on_store(&native, m, move |s| {
                        s.call("dbRecordConnectorPollPage", json!([args]))
                    })
                    .await
                }
                "inbox.recordError" => {
                    on_store(&native, m, move |s| {
                        s.call("dbRecordConnectorPollError", json!([args]))
                    })
                    .await
                }
                "signin.check" => {
                    let key = text(&args, "key").ok_or_else(|| needs(m, "the window's key"))?;
                    let browser = args.get("browser").cloned().unwrap_or(Value::Null);
                    crate::native::signin::check(&native, &key, browser)
                        .await
                        .map_err(|e| ExtError::new("failed", e))
                }
                "signin.forget" => {
                    let key = text(&args, "key").ok_or_else(|| needs(m, "the window's key"))?;
                    crate::native::signin::forget(&native, &key)
                        .await
                        .map_err(|e| ExtError::new("failed", e))
                }
                "signin.completed" => {
                    let key = text(&args, "key").ok_or_else(|| needs(m, "the window's key"))?;
                    crate::native::signin::completed(&native, &key).await;
                    Ok(Value::Null)
                }
                "web.call" => {
                    let view = text(&args, "view").ok_or_else(|| needs(m, "the view"))?;
                    let op = text(&args, "op").ok_or_else(|| needs(m, "the op"))?;
                    let mut params = match args.get("params") {
                        Some(Value::Object(p)) => p.clone(),
                        _ => serde_json::Map::new(),
                    };
                    params.insert("view".into(), Value::String(view));
                    crate::bridge::Bridge::request(
                        native.main_process().as_ref(),
                        &format!("web:{op}"),
                        Value::Object(params),
                        crate::native::desktop::TIMEOUT,
                    )
                    .await
                    .map_err(|e| ExtError::new("failed", e))
                }
                other => Err(ExtError::new(
                    "unknown-method",
                    format!("Vorn has no method {other}"),
                )),
            }
        })
    }

    fn fetch(&self, _caller: &Caller, request: Request) -> Answer<Response> {
        Box::pin(async move {
            let sent = HttpRequest {
                method: request.method,
                url: request.url,
                headers: request
                    .headers
                    .into_iter()
                    .map(|h| (h.name, h.value))
                    .collect(),
                body: request.body,
            };
            let answer = tokio::task::spawn_blocking(move || Http.send(&sent, FETCH_TIMEOUT)).await;
            match answer {
                Ok(Ok(r)) => Ok(Response {
                    status: r.status,
                    headers: r
                        .headers
                        .into_iter()
                        .map(|(name, value)| Header { name, value })
                        .collect(),
                    body: r.body,
                }),
                Ok(Err(message)) => Err(ExtError::new("network", message)),
                Err(e) => Err(ExtError::new("failed", e.to_string())),
            }
        })
    }

    fn secret(&self, caller: &Caller, connection: &str, field: &str) -> Answer<Option<String>> {
        let (native, owner, connection, field) = (
            self.native.upgrade(),
            caller.extension.clone(),
            connection.to_owned(),
            field.to_owned(),
        );
        Box::pin(secret_of(native, owner, connection, field))
    }

    fn storage(
        &self,
        caller: &Caller,
        migrations: Option<PathBuf>,
        call: StorageCall,
    ) -> Answer<Value> {
        let (native, id) = (self.native.upgrade(), caller.extension.clone());
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            let db = native
                .database()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| ExtError::new("unsupported", "vornd has no database"))?;
            let kept = native.ext_tables().of(&id);
            let done = tokio::task::spawn_blocking(move || {
                let mut held = kept
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if held.is_none() {
                    *held = Some(ExtensionTables::open_with(&db, &id, lent(&id))?);
                }
                let tables = held
                    .as_ref()
                    .ok_or_else(|| vorn_store::Error::Refused("its tables did not open".into()))?;
                // Checked each call: a linked package gains files, and a reinstall starts with none run.
                if let Some(dir) = migrations.filter(|d| d.is_dir()) {
                    let ran = tables.migrate(&dir)?;
                    if !ran.is_empty() {
                        info!("[extensions] {id} ran its migrations {}", ran.join(", "));
                    }
                }
                Ok::<_, vorn_store::Error>(match call {
                    StorageCall::Prefix => json!(tables.prefix()),
                    StorageCall::Query { sql, params } => {
                        Value::Array(tables.query(&sql, &params)?)
                    }
                    StorageCall::Exec { sql, params } => json!(tables.exec(&sql, &params)?),
                    StorageCall::Get { key } => tables.get(&key)?.unwrap_or(Value::Null),
                    StorageCall::Set { key, value } => {
                        tables.set(&key, &value)?;
                        Value::Null
                    }
                    StorageCall::Transaction(statements) => {
                        Value::Array(tables.transaction(&statements)?)
                    }
                })
            })
            .await
            .map_err(|e| ExtError::new("failed", e.to_string()))?;
            done.map_err(|e| {
                let code = if e.is_refused() { "denied" } else { "failed" };
                ExtError::new(code, e.to_string())
            })
        })
    }

    fn start_session(&self, caller: &Caller, req: SessionStart) -> Answer<String> {
        let (native, extension) = (self.native.upgrade(), caller.extension.clone());
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            start_session(&native, &extension, req).await
        })
    }

    fn published(&self, caller: &Caller, collection: &str, ops: &[Value]) {
        if let Some(native) = self.native.upgrade() {
            native.broadcast_to(
                "extension:rowsChanged",
                json!({ "extensionId": caller.extension, "collection": collection, "ops": ops }),
                None,
            );
        }
    }
}
