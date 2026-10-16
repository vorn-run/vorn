//! What vornd puts behind the bridge's gate: the core methods a program may
//! call, the network `net.fetch` reaches, and the vault its credentials come
//! from. The gate has already checked every call against the manifest.

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tracing::info;
use vorn_connectors::fetch::{Http, Request as HttpRequest};
use vorn_extensions::api::types::{Header, Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{Answer, Caller, StorageCall, World};
use vorn_store::extension::ExtensionTables;

use crate::native::config::Viewer;
use crate::native::{secrets::Known, Answer as NativeAnswer, Native};

/// How long one `net.fetch` may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// vornd's world: its session records, git, the network, the vault and the extensions' tables.
pub struct Wired {
    pub native: Weak<Native>,
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
                "path": e["path"], "name": e["name"], "branch": e["branch"],
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
                && extension != vorn_connectors::builtin::ID
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
                    let project = project.ok_or_else(|| needs(m, "the project to start it in"))?;
                    let agent = text(&args, "agent").unwrap_or_else(|| "shell".to_owned());
                    let started = if agent == "shell" {
                        ask(&native, "shell:create", json!(project)).await?
                    } else {
                        let name = std::path::Path::new(&project)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| project.clone());
                        let params = json!({ "agentType": agent, "projectName": name, "projectPath": project });
                        ask(&native, "terminal:create", params).await?
                    };
                    Ok(json!({ "sessionId": started.get("id").cloned().unwrap_or(Value::Null) }))
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
                    let mut params = json!({ "filePath": path });
                    if let Some(max) = args.get("maxBytes").filter(|v| v.is_u64()) {
                        params["maxBytes"] = max.clone();
                    }
                    ask(&native, "file:readContent", params).await
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
            let done = tokio::task::spawn_blocking(move || {
                let tables = ExtensionTables::open(&db, &id)?;
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
                })
            })
            .await
            .map_err(|e| ExtError::new("failed", e.to_string()))?;
            done.map_err(|e| match e {
                vorn_store::Error::Refused(message) => ExtError::new("denied", message),
                other => ExtError::new("failed", other.to_string()),
            })
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
