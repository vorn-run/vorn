//! What vornd puts behind the bridge's gate: the core methods a program may
//! call, the network `net.fetch` reaches, and the vault its credentials come
//! from. The gate has already checked every call against the manifest.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tracing::info;
use vorn_connectors::fetch::{Http, Request as HttpRequest};
use vorn_extensions::api::types::{CallCtx, Header, Request, Response, SessionStart};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{self, Answer, Caller, StorageCall, World};
use vorn_extensions::pack::Migrations;
use vorn_extensions::reach::{self, Access, Opened, Places};
use vorn_store::extension::{ExtensionTables, Grant};

use crate::native::config::Viewer;
use crate::native::{file, secrets::Known, Answer as NativeAnswer, Native};

/// How long one `net.fetch` may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long `video.call`'s `boot` waits for a simulator to start: a cold
/// boot takes tens of seconds, and the router gives the whole call 60.
const VIDEO_BOOT: Duration = Duration::from_secs(55);

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
    if id == super::hosted::tasks::ID {
        vorn_store::extension::TASKS
    } else {
        &[]
    }
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

/// The extension host, or the error for a vornd that runs none.
fn host_of(native: &Native) -> Result<Arc<super::Extensions>, ExtError> {
    native
        .extensions()
        .cloned()
        .ok_or_else(|| ExtError::new("unsupported", "vornd is not hosting extensions"))
}

/// `path` as the caller named it, once `caller` may `access` it as resolved:
/// within its projects' folders, its own, the folders its sessions' shells
/// are in, what a person handed it, or anywhere with `process.run`; never
/// Vorn's data, credentials or the shell's startup files.
async fn reached(
    native: &Arc<Native>,
    caller: &Caller,
    path: &str,
    access: Access,
) -> Result<PathBuf, ExtError> {
    let path = PathBuf::from(path);
    on_places(native, caller, move |places, extension, runs| {
        places.reach(extension, &path, access, runs)
    })
    .await?
}

/// `path` opened for `access` once `caller` may reach it ([`reached`]), the
/// handle then given to `then` off the runtime: what it reads or writes is
/// the file checked, not whatever the name points at by then.
async fn opened<T: Send + 'static>(
    native: &Arc<Native>,
    caller: &Caller,
    path: &str,
    access: Access,
    then: impl FnOnce(std::io::Result<Opened>) -> T + Send + 'static,
) -> Result<T, ExtError> {
    let path = PathBuf::from(path);
    on_places(native, caller, move |places, extension, runs| {
        places.open(extension, &path, access, runs).map(then)
    })
    .await?
}

/// `path` as git prints one: on Windows without the `\\?\` prefix and with `/`.
fn file_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        path.strip_prefix(r"\\?\")
            .unwrap_or(&path)
            .replace('\\', "/")
    } else {
        path.into_owned()
    }
}

/// Runs `check` off the runtime on the places `caller` may reach.
async fn on_places<T: Send + 'static>(
    native: &Arc<Native>,
    caller: &Caller,
    check: impl FnOnce(Places, &str, bool) -> T + Send + 'static,
) -> Result<T, ExtError> {
    let host = host_of(native)?;
    let projects = on_store(native, "files", |s| s.call("dbListProjects", json!([]))).await?;
    let mut roots: Vec<String> = Vec::new();
    for p in projects.as_array().into_iter().flatten() {
        if let Some(path) = p.get("path").and_then(Value::as_str) {
            roots.push(path.to_owned());
            roots.push(vorn_git::repo::worktree_base_dir(path));
        }
    }
    let mut here: Vec<String> = Vec::new();
    if let Some(registry) = native.registry.get() {
        registry.read(|r| {
            for t in r.live_terminals() {
                roots.push(t.project_path.clone());
                roots.extend(t.worktree_path.clone());
                if host.paned(&caller.extension, &t.id) {
                    let folder = t.shell_cwd.as_ref().or(t.worktree_path.as_ref());
                    here.push(folder.unwrap_or(&t.project_path).clone());
                }
            }
        });
    }
    let own = host.router().files_dir(&caller.extension);
    let handed = host.given().handed(&caller.extension);
    let home = PathBuf::from(crate::native::shell::home_dir());
    let data = native
        .database()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf);
    let (extension, runs) = (caller.extension.clone(), caller.runs_commands);
    tokio::task::spawn_blocking(move || {
        let resolved = |list: Vec<String>| -> Vec<PathBuf> {
            list.iter()
                .filter_map(|p| reach::resolve(std::path::Path::new(p)).ok())
                .collect()
        };
        let data: Vec<&std::path::Path> = data.as_deref().into_iter().collect();
        let places = Places {
            roots: resolved(roots),
            here: resolved(here),
            home: reach::resolve(&home).ok(),
            own: own.and_then(|o| reach::resolve(&o).ok()),
            handed,
            closed: reach::closed(&home, &data),
        };
        check(places, &extension, runs)
    })
    .await
    .map_err(|e| ExtError::new("failed", e.to_string()))
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
    let id = started
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ExtError::new("failed", "Vorn started no session"))?;
    if let Some(host) = native.extensions() {
        host.given().started(extension, &id);
    }
    Ok(id)
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
        "projectName": t.project_name, "projectPath": t.project_path,
        "worktreePath": t.worktree_path,
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

/// `remote.test`: logs in to `host` and answers how it went. With `then`,
/// it answers `{testing: true}` at once and tells the caller's action
/// `then` `{key, result}` once the test is done, so a component, which
/// answers one call at a time, can draw the test running for its ten seconds.
async fn remote_test(native: &Arc<Native>, caller: &str, args: &Value) -> Result<Value, ExtError> {
    let host = args
        .get("host")
        .cloned()
        .ok_or_else(|| needs("remote.test", "the host"))?;
    let Some(then) = text(args, "then") else {
        return crate::native::about::test_host(native, host)
            .await
            .map_err(|e| ExtError::new("failed", e));
    };
    let ext = native
        .extensions()
        .cloned()
        .ok_or_else(|| ExtError::new("unsupported", "vornd runs no extensions"))?;
    let (n, caller, key) = (
        Arc::clone(native),
        caller.to_owned(),
        args.get("key").cloned().unwrap_or(Value::Null),
    );
    tokio::spawn(async move {
        let result = match crate::native::about::test_host(&n, host).await {
            Ok(tested) => tested,
            Err(e) => json!({ "success": false, "message": e, "durationMs": 0 }),
        };
        let told = json!({ "key": key, "result": result });
        let ctx = CallCtx {
            caller: "client".to_owned(),
            project: None,
        };
        if let Err(e) = ext.call(&caller, &then, told, ctx).await {
            info!(
                "[extensions] {caller} did not take its test's result: {}",
                e.message
            );
        }
    });
    Ok(json!({ "testing": true }))
}

/// What `remote.importKey` answers for any path it does not import.
const NOT_A_KEY: &str =
    "That file is not an SSH key Vorn can import. Choose one in ~/.ssh or with Browse.";

/// `remote.importKey`: the private key at `path` (`~` is the home folder)
/// and the public key beside it (`.pub`), labelled with the file's name, as
/// `credential:storeKey` takes them. Only a key in `home`'s `.ssh` folder
/// or one a person chose (`handed`) is read, and only one that reads as a
/// private key; `None` for anything else, whatever the reason.
fn read_key(home: &std::path::Path, handed: &[PathBuf], path: &str) -> Option<Value> {
    let full = match path.trim().strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path.trim()),
    };
    let real = reach::resolve(&full).ok()?;
    let ssh = reach::resolve(&home.join(".ssh")).ok()?;
    if !real.starts_with(&ssh) && !handed.contains(&real) {
        return None;
    }
    let private = std::fs::read_to_string(&real).ok()?;
    if !private.trim_start().starts_with("-----BEGIN ") || !private.contains("PRIVATE KEY-----") {
        return None;
    }
    let label = real.file_name()?.to_string_lossy().into_owned();
    let mut key = json!({ "label": label, "privateKey": private });
    let mut public = real.into_os_string();
    public.push(".pub");
    if let Ok(p) = std::fs::read_to_string(public) {
        key["publicKey"] = json!(p.trim());
    }
    Some(key)
}

/// Whether a step of `node_type` runs commands or agents: one the Workflows
/// built-in runs, or one no extension declares, which runs as its agent.
pub(crate) fn runs_commands(native: &Native, node_type: &str) -> bool {
    let Some(step) = vorn_work::model::NodeKind::parse(node_type)
        .step()
        .map(str::to_owned)
    else {
        return false;
    };
    let provider = native
        .work()
        .and_then(|w| w.engine.registry().provider(&step));
    provider.is_none_or(|p| p == vorn_workflows::ID)
}

impl World for Wired {
    fn call(&self, caller: &Caller, method: &str, args: Value) -> Answer<Value> {
        let native = self.native.upgrade();
        let method = method.to_owned();
        let extension = caller.extension.clone();
        let project = text(&args, "project").or_else(|| caller.project.clone());
        let weak = self.native.clone();
        let caller = caller.clone();
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            let m = method.as_str();
            // What runs is read once vornd knows it, as a client's reads are.
            if matches!(m, "sessions.list" | "sessions.get") {
                if let Some(registry) = native.registry.get().cloned() {
                    let settle = move || registry.settle(crate::native::SETTLE_LIMIT);
                    let _ = tokio::task::spawn_blocking(settle).await;
                }
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
                    let host = host_of(&native)?;
                    if m.starts_with("terminal.") {
                        let id = text(&args, "sessionId").unwrap_or_default();
                        host.admit_terminal(&caller, m, &id)?;
                    }
                    super::routes::session_call(&host, &extension, m, &args)
                        .await
                        .map_err(|e| ExtError::new("failed", e))
                }
                "git.status" => {
                    let project = project.ok_or_else(|| needs(m, "the project to read"))?;
                    let dir = reached(&native, &caller, &project, Access::Read).await?;
                    let git = vorn_git::repo::Git {
                        bin: native.env.git_bin(),
                        env: native.env.get(),
                        ssh: None,
                    };
                    tokio::task::spawn_blocking(move || {
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
                    let path = reached(&native, &caller, &path, Access::Write).await?;
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
                    let worktree = reached(&native, &caller, &worktree, Access::Write).await?;
                    let project = reached(&native, &caller, &project, Access::Write).await?;
                    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
                    let params =
                        json!({ "projectPath": project, "worktreePath": worktree, "force": force });
                    let removed = ask(&native, "git:removeWorktree", params).await?;
                    Ok(json!({ "removed": removed.as_bool() == Some(true) }))
                }
                "permissions.resolve" => {
                    let by_person = host_of(&native)?.given().acting(&extension);
                    gate::admit_resolve(&caller, by_person)?;
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
                    let dir = reached(&native, &caller, &dir, Access::Read).await?;
                    ask(&native, "git:diffFull", json!(dir)).await
                }
                "git.commit" => {
                    let dir = text(&args, "worktree")
                        .or(project)
                        .ok_or_else(|| needs(m, "the worktree to commit in"))?;
                    let dir = reached(&native, &caller, &dir, Access::Write).await?;
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
                        let read = |o: std::io::Result<Opened>| -> std::io::Result<Vec<u8>> {
                            let mut bytes = Vec::new();
                            o?.file().read_to_end(&mut bytes)?;
                            Ok(bytes)
                        };
                        return opened(&native, &caller, &path, Access::Read, read)
                            .await?
                            .map(|b| json!({ "base64": vorn_extensions::api::base64::encode(&b) }))
                            .map_err(|e| ExtError::new("failed", e.to_string()));
                    }
                    let max = args
                        .get("maxBytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(file::MAX_READ_BYTES);
                    let text = opened(&native, &caller, &path, Access::Read, move |o| {
                        o.ok()
                            .and_then(|o| file::read_text(o.file(), o.metadata(), max))
                    })
                    .await?;
                    Ok(json!(text))
                }
                "files.list" => {
                    let dir = text(&args, "path")
                        .ok_or_else(|| needs(m, "the absolute folder to list"))?;
                    let (lister, named) = (Arc::clone(&native), dir.clone());
                    let list = move |o: std::io::Result<Opened>| {
                        let Ok(o) = o else { return Vec::new() };
                        let entries = o.entries().unwrap_or_default();
                        let real = file_path(o.real());
                        file::listing(&named, &real, entries, &lister.env, &lister.ignored)
                    };
                    let entries = opened(&native, &caller, &dir, Access::Look, list).await?;
                    Ok(Value::Array(
                        entries.iter().map(file::FileEntry::to_json).collect(),
                    ))
                }
                "files.stat" => {
                    let path = PathBuf::from(
                        text(&args, "path").ok_or_else(|| needs(m, "the absolute path to read"))?,
                    );
                    let looked = on_places(&native, &caller, move |places, extension, runs| {
                        places.stat(extension, &path, runs)
                    })
                    .await??;
                    let stamp = looked
                        .ok()
                        .filter(|l| l.is_file)
                        .and_then(|l| file::stamp_of(l.len, l.modified?));
                    Ok(stamp.unwrap_or(Value::Null))
                }
                "shell.executables" => ask(&native, "shell:listExecutables", Value::Null).await,
                "git.branches" => {
                    let dir = text(&args, "worktree")
                        .or(project)
                        .ok_or_else(|| needs(m, "the repository to read"))?;
                    let dir = reached(&native, &caller, &dir, Access::Read).await?;
                    ask(&native, "git:listBranches", json!(dir)).await
                }
                "files.write" => {
                    let path = text(&args, "path")
                        .ok_or_else(|| needs(m, "the absolute path to write"))?;
                    let content = args
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| needs(m, "the content to write"))?
                        .to_owned();
                    let named = path.clone();
                    opened(&native, &caller, &path, Access::Write, move |o| match o {
                        Ok(o) => file::write_to(o.file(), &named, &content),
                        Err(e) => file::written(&named, Err(e)),
                    })
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
                "task.take" | "task.handBack" | "task.importItems" | "task.sourceLink" => {
                    super::hosted::tasks::api(&native, m, args).await
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
                    gate::admit_run(&caller, &id)?;
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
                    for wf in &workflows {
                        gate::admit_seed(&caller, wf, |t| runs_commands(&native, t))?;
                    }
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
                    gate::admit_own(&caller, m, &prefix)?;
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
                    gate::admit_op(m, &op)?;
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
                // The app's simulators, asked as `video:{op}`. Starting one
                // takes as long as it takes, up to just under a call's own limit.
                "video.call" => {
                    let op = text(&args, "op").ok_or_else(|| needs(m, "the op"))?;
                    gate::admit_op(m, &op)?;
                    let mut params = match args.get("params") {
                        Some(Value::Object(p)) => p.clone(),
                        _ => serde_json::Map::new(),
                    };
                    if let Some(view) = text(&args, "view") {
                        params.insert("view".into(), Value::String(view));
                    }
                    let wait = if op == "boot" {
                        VIDEO_BOOT
                    } else {
                        crate::native::desktop::TIMEOUT
                    };
                    crate::bridge::Bridge::request(
                        native.main_process().as_ref(),
                        &format!("video:{op}"),
                        Value::Object(params),
                        wait,
                    )
                    .await
                    .map_err(|e| ExtError::new("failed", e))
                }
                "remote.hosts" => {
                    let config = ask(&native, "config:load", Value::Null).await?;
                    Ok(match config.get("remoteHosts") {
                        Some(Value::Array(hosts)) => Value::Array(hosts.clone()),
                        _ => json!([]),
                    })
                }
                "remote.saveHosts" => {
                    let Some(Value::Array(hosts)) = args.get("hosts").cloned() else {
                        return Err(needs(m, "the hosts, a list"));
                    };
                    crate::native::config::save_hosts(&native, hosts)
                        .await
                        .map(|()| Value::Null)
                        .map_err(|e| ExtError::new("failed", e))
                }
                "remote.test" => remote_test(&native, &extension, &args).await,
                "remote.keys" => ask(&native, "credential:listKeys", Value::Null).await,
                "remote.storeKey" => ask(&native, "credential:storeKey", args).await,
                "remote.importKey" => {
                    let path = text(&args, "path").ok_or_else(|| needs(m, "the key's path"))?;
                    let handed = host_of(&native)?.given().handed(&extension);
                    let home = PathBuf::from(crate::native::shell::home_dir());
                    let key = tokio::task::spawn_blocking(move || read_key(&home, &handed, &path))
                        .await
                        .map_err(|e| ExtError::new("failed", e.to_string()))?
                        .ok_or_else(|| ExtError::new("failed", NOT_A_KEY))?;
                    ask(&native, "credential:storeKey", key).await
                }
                "remote.deleteKey" => {
                    let id = text(&args, "id").ok_or_else(|| needs(m, "the key's id"))?;
                    ask(&native, "credential:deleteKey", json!(id)).await
                }
                "remote.vault" => Ok(json!({ "keychain": !native.secrets.keychain_missing() })),
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
        migrations: Option<Migrations>,
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
                if let Some(migrations) = migrations {
                    let ran = tables.migrate(migrations.names()?, |name| migrations.read(name))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str =
        "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\n";

    #[test]
    fn a_key_is_imported_only_from_ssh_or_where_a_person_chose() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        std::fs::write(ssh.join("id_ed25519"), KEY).unwrap();
        std::fs::write(ssh.join("id_ed25519.pub"), "ssh-ed25519 AAAA me\n").unwrap();
        std::fs::write(ssh.join("config"), "Host *\n").unwrap();
        let elsewhere = home.path().join("keys");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("deploy"), KEY).unwrap();
        std::fs::write(home.path().join(".zshrc"), "export A=1\n").unwrap();

        let key = read_key(home.path(), &[], "~/.ssh/id_ed25519").unwrap();
        assert_eq!(key["label"], "id_ed25519");
        assert_eq!(key["privateKey"], KEY);
        assert_eq!(key["publicKey"], "ssh-ed25519 AAAA me");

        let deploy = elsewhere.join("deploy");
        assert!(read_key(home.path(), &[], deploy.to_str().unwrap()).is_none());
        let chosen = vec![reach::resolve(&deploy).unwrap()];
        assert!(read_key(home.path(), &chosen, deploy.to_str().unwrap()).is_some());

        for refused in [
            "~/.zshrc",
            "~/.ssh/config",
            "~/.ssh/../keys/deploy",
            "~/.ssh/none",
        ] {
            assert!(read_key(home.path(), &[], refused).is_none(), "{refused}");
        }
    }
}
