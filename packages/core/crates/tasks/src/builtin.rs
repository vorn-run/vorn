//! The Tasks built-in: every `task:*` call but the reads (the list and one task by id), the connectors'
//! imports (`task.importItems`, `task.sourceLink`), the conversation an
//! agent starts kept on its terminal's task, and the badge rows a session's
//! card draws. Each write tells clients the rows it changed, and a task
//! created or moved is a public event the workflows' triggers fire from.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_extensions::api::types::RowOp;
use vorn_extensions::events::{TASK_CREATED, TASK_STATUS_CHANGED};
use vorn_store::Store;

use crate::calls;
use crate::host::{Change, Host};
use crate::images::TaskImages;
use crate::sources;
use crate::triggers;

/// The id the built-in goes by.
pub const ID: &str = "vorn.tasks";

/// Its manifest: what it declares it uses, what it draws and the MCP tools it adds.
pub const MANIFEST: &str = include_str!("../vorn-extension.toml");

/// What a call to a task method is told while Tasks is off.
pub const OFF: &str = "Tasks is off. Turn it on in Settings → Extensions.";

/// The collection the card badge draws: one row per session working on a task.
pub const BADGES: &str = "working";

/// Every call this built-in answers.
pub const METHODS: &[&str] = &[
    "task:setStatus",
    "task:create",
    "task:update",
    "task:patch",
    "task:delete",
    "task:reorder",
    "task:archive",
    "task:imageSave",
    "task:imageDelete",
    "task:imageGetPath",
    "task:imageCleanup",
    "task:imageUpload",
];

/// What a call came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Result(Value),
    /// Answered with nothing.
    Void,
    Error(String),
    /// Not a call this built-in answers.
    Unanswered,
}

/// The built-in, over the host it reaches Vorn through.
pub struct Tasks {
    host: Arc<dyn Host>,
    /// The badge rows last published, by session, so a change publishes only what moved.
    badges: Mutex<BTreeMap<String, Value>>,
}

impl std::fmt::Debug for Tasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tasks").finish_non_exhaustive()
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn text(params: &Value, key: &str) -> String {
    match params.get(key) {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => String::new(),
        Some(other) => vorn_work::js::to_string(other),
    }
}

/// Whether `method` writes one task, whose change may fire a trigger.
fn written(method: &str) -> bool {
    matches!(
        method,
        "task:create" | "task:setStatus" | "task:update" | "task:patch"
    )
}

/// The store's part of a write: its answer, the change clients are told,
/// and the trigger it fires.
#[derive(Debug, Default)]
struct Done {
    value: Option<Value>,
    change: Option<Change>,
    trigger: Option<Value>,
}

fn row(store: &mut Store, id: &Value) -> vorn_store::Result<Option<Value>> {
    let found = store.call("dbGetTask", json!([id]))?;
    Ok((!found.is_null()).then_some(found))
}

/// Runs `method` on the store.
fn write(store: &mut Store, method: &str, params: &Value) -> vorn_store::Result<Done> {
    let now = now_iso();
    // A task being created was not there before, whatever id it is given.
    let before = if written(method) && method != "task:create" {
        row(store, params.get("id").unwrap_or(&Value::Null))?
    } else {
        None
    };
    let (value, changed) = match method {
        "task:setStatus" => calls::set_status(store, params, &now)?,
        "task:create" => calls::create(store, params, uuid::Uuid::new_v4().to_string(), &now)?,
        "task:update" => calls::update(store, params, &now)?,
        "task:patch" => calls::patch(store, params, &now)?,
        "task:delete" => calls::delete(store, params)?,
        "task:reorder" => calls::reorder(store, params, &now)?,
        "task:archive" => calls::archive(store, params, &now)?,
        _ => return Ok(Done::default()),
    };
    let mut done = Done {
        value: Some(value),
        ..Done::default()
    };
    if !changed {
        return Ok(done);
    }
    let ids: Vec<Value> = match method {
        "task:reorder" => params
            .get("ids")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        "task:create" => done
            .value
            .as_ref()
            .and_then(|v| v.pointer("/task/id"))
            .cloned()
            .into_iter()
            .collect(),
        _ => params.get("id").cloned().into_iter().collect(),
    };
    if method == "task:delete" {
        done.change = Some(Change::Removed(
            ids.iter().map(vorn_work::js::to_string).collect(),
        ));
        return Ok(done);
    }
    let mut rows: Vec<Value> = Vec::with_capacity(ids.len());
    for id in &ids {
        let Some(after) = row(store, id)? else {
            continue;
        };
        if rows.iter().any(|r| r.get("id") == after.get("id")) {
            continue;
        }
        if written(method) {
            done.trigger = triggers::for_task(before.as_ref(), &after);
        }
        rows.push(after);
    }
    done.change = Some(Change::Rows(rows));
    Ok(done)
}

impl Tasks {
    pub fn new(host: Arc<dyn Host>) -> Arc<Tasks> {
        Arc::new(Tasks {
            host,
            badges: Mutex::new(BTreeMap::new()),
        })
    }

    /// Runs `f` on the board's tables and hands back what it found.
    async fn on_store<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> vorn_store::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.host
            .tables(Box::new(move |store| {
                let _ = tx.send(f(store));
                Ok(())
            }))
            .await?;
        rx.await
            .map_err(|_| "the task call was dropped".to_owned())?
            .map_err(|e| e.to_string())
    }

    /// Answers `method`.
    pub async fn answer(&self, method: &str, params: Value) -> Answer {
        if method.starts_with("task:image") {
            return self.images(method, &params);
        }
        if !METHODS.contains(&method) {
            return Answer::Unanswered;
        }
        let m = method.to_owned();
        let done = self.on_store(move |store| write(store, &m, &params)).await;
        match done {
            Ok(done) => {
                if let Some(trigger) = done.trigger {
                    self.fire(trigger).await;
                }
                if let Some(change) = done.change {
                    let badged = self.touches_badges(&change);
                    self.host.changed(change).await;
                    if badged {
                        self.badges().await;
                    }
                }
                match done.value {
                    Some(value) => Answer::Result(value),
                    None => Answer::Void,
                }
            }
            Err(message) => Answer::Error(message),
        }
    }

    fn images(&self, method: &str, params: &Value) -> Answer {
        let Some(dir) = self.host.data_dir() else {
            return Answer::Error("vornd has no database".to_owned());
        };
        let images = TaskImages::new(&dir);
        let task = text(params, "taskId");
        let done = match method {
            "task:imageSave" => images
                .save(&task, &text(params, "sourcePath"))
                .map(|n| json!(n)),
            "task:imageUpload" => images
                .upload(&task, &text(params, "base64"), &text(params, "filename"))
                .map(|n| json!(n)),
            "task:imageDelete" => images
                .delete(&task, &text(params, "filename"))
                .map(|()| Value::Null),
            "task:imageGetPath" => images
                .path(&task, &text(params, "filename"))
                .map(|p| json!(p)),
            "task:imageCleanup" => {
                let task = params
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| vorn_work::js::to_string(params));
                images.cleanup(&task).map(|()| Value::Null)
            }
            _ => return Answer::Unanswered,
        };
        match done {
            Ok(Value::Null) => Answer::Void,
            Ok(value) => Answer::Result(value),
            Err(message) => Answer::Error(message),
        }
    }

    /// `task.importItems`: a connection's items on the board, each answered `{taskId, created}`.
    pub async fn import(&self, args: Value) -> Result<Value, String> {
        let import: sources::Import = serde_json::from_value(args)
            .map_err(|e| format!("task.importItems takes a connection and its items ({e})"))?;
        let now = now_iso();
        let (answers, rows) = self
            .on_store(move |store| {
                let answers =
                    sources::import(store, &import, &now, || uuid::Uuid::new_v4().to_string())?;
                let mut rows = Vec::with_capacity(answers.len());
                for a in &answers {
                    if let Some(r) = row(store, a.get("taskId").unwrap_or(&Value::Null))? {
                        rows.push(r);
                    }
                }
                Ok((answers, rows))
            })
            .await?;
        if !rows.is_empty() {
            let change = Change::Rows(rows);
            let badged = self.touches_badges(&change);
            self.host.changed(change).await;
            if badged {
                self.badges().await;
            }
        }
        Ok(Value::Array(answers))
    }

    /// `task.take`: task `id` in progress for a workflow step's session and
    /// agent; `task.handBack`: back in the queue, unassigned. Each fires the
    /// workflows watching for the move, as any write does.
    pub async fn take_or_hand_back(&self, method: &str, args: &Value) -> Result<Value, String> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{method} takes the id of the task"))?
            .to_owned();
        let now = now_iso();
        let mut updates = serde_json::Map::new();
        match method {
            "task.take" => {
                let text = |k: &str| args.get(k).cloned().unwrap_or(Value::Null);
                updates.insert("status".into(), json!("in_progress"));
                updates.insert("assignedSessionId".into(), text("sessionId"));
                updates.insert("assignedAgent".into(), text("agent"));
                updates.insert("updatedAt".into(), json!(now));
                updates.insert("archivedAt".into(), Value::Null);
            }
            _ => {
                updates.insert("status".into(), json!("todo"));
                updates.insert("updatedAt".into(), json!(now));
                updates.insert("completedAt".into(), Value::Null);
                updates.insert("archivedAt".into(), Value::Null);
                updates.insert("assignedSessionId".into(), Value::Null);
                updates.insert("assignedAgent".into(), Value::Null);
            }
        }
        let moved = self
            .on_store(move |store| {
                let Some(before) = row(store, &json!(id))? else {
                    return Ok(None);
                };
                let cleared: Vec<String> = updates
                    .iter()
                    .filter(|(_, v)| v.is_null())
                    .map(|(k, _)| k.clone())
                    .collect();
                store.call("dbUpdateTask", json!([id, updates, cleared]))?;
                Ok(row(store, &json!(id))?.map(|after| (before, after)))
            })
            .await?;
        let Some((before, after)) = moved else {
            return Ok(Value::Null);
        };
        if let Some(trigger) = triggers::for_task(Some(&before), &after) {
            self.fire(trigger).await;
        }
        let change = Change::Rows(vec![after.clone()]);
        let badged = self.touches_badges(&change);
        self.host.changed(change).await;
        if badged {
            self.badges().await;
        }
        Ok(after)
    }

    /// A trigger a write fired: the public event every subscriber hears,
    /// and the workflows watching for it started (`workflow.fire`).
    async fn fire(&self, trigger: Value) {
        let name = match trigger.get("kind").and_then(Value::as_str) {
            Some("taskCreated") => TASK_CREATED,
            _ => TASK_STATUS_CHANGED,
        };
        let args = triggers::fire_args(&trigger);
        self.host.emit(name, trigger);
        if let Some(args) = args {
            if let Err(err) = self.host.call("workflow.fire", args).await {
                info!(%err, "no workflow was started for a task's move");
            }
        }
    }

    /// Runs a step of a node type Tasks declares; `None` for another type.
    pub async fn run_node(&self, node_type: &str, request: &Value) -> Option<Value> {
        crate::nodes::run(node_type, request, |method, params| {
            self.core(method, params)
        })
        .await
    }

    /// `task.sourceLink`: where task `task_id` was imported from, or null.
    pub async fn source_link(&self, task_id: String) -> Result<Value, String> {
        self.on_store(move |store| sources::link(store, &task_id))
            .await
    }

    /// Keeps the conversation a terminal started on the in-progress task it
    /// was assigned, when that task has none yet.
    pub async fn keep_conversation(&self, terminal: String, conversation: String) {
        let (t, c) = (terminal.clone(), conversation.clone());
        let kept = self
            .on_store(move |store| {
                let tasks = store.call("dbListTasks", json!([null, "in_progress"]))?;
                let task = tasks.as_array().into_iter().flatten().find(|task| {
                    task.get("assignedSessionId").and_then(Value::as_str) == Some(t.as_str())
                        && task.get("agentSessionId").is_none_or(Value::is_null)
                });
                let Some(id) = task.and_then(|t| t.get("id")).cloned() else {
                    return Ok(None);
                };
                store.call(
                    "dbUpdateTask",
                    json!([id, { "agentSessionId": c, "updatedAt": now_iso() }, ["agentSessionId", "updatedAt"]]),
                )?;
                row(store, &id)
            })
            .await;
        match kept {
            Ok(Some(after)) => {
                info!(%terminal, %conversation, "the conversation is kept on the terminal's task");
                self.host.changed(Change::Rows(vec![after])).await;
            }
            Ok(None) => {}
            Err(err) => warn!(%err, "could not keep the conversation on the task"),
        }
    }

    /// Whether `change` can move a badge: a task in progress on a session
    /// now, or one a badge shows. Others leave the badges as they are, so a
    /// write never reads the tasks in progress for nothing.
    fn touches_badges(&self, change: &Change) -> bool {
        let shown = self
            .badges
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let on_a_badge = |id: &str| {
            shown
                .values()
                .any(|b| b.get("task").and_then(Value::as_str) == Some(id))
        };
        match change {
            Change::Removed(ids) => ids.iter().any(|id| on_a_badge(id)),
            Change::Rows(rows) => rows.iter().any(|r| {
                let working = r.get("status").and_then(Value::as_str) == Some("in_progress")
                    && r.get("assignedSessionId").is_some_and(|s| !s.is_null());
                working || r.get("id").and_then(Value::as_str).is_some_and(on_a_badge)
            }),
        }
    }

    /// Publishes the card badge's rows again, as the board is now: one per
    /// session with a task in progress, publishing only what moved.
    pub async fn badges(&self) {
        let working = self
            .on_store(|store| store.call("dbListTasks", json!([null, "in_progress"])))
            .await;
        let working = match working {
            Ok(w) => w,
            Err(err) => {
                warn!(%err, "could not read the tasks in progress for their badges");
                return;
            }
        };
        let mut now: BTreeMap<String, Value> = BTreeMap::new();
        for task in working.as_array().into_iter().flatten() {
            let Some(session) = task.get("assignedSessionId").and_then(Value::as_str) else {
                continue;
            };
            // The first task a session took is the one its card shows, as the chip showed it.
            now.entry(session.to_owned()).or_insert_with(|| {
                json!({
                    "id": session,
                    "task": task.get("id").cloned().unwrap_or(Value::Null),
                    "title": task.get("title").cloned().unwrap_or(Value::Null),
                })
            });
        }
        let ops = {
            let mut was = self
                .badges
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut ops: Vec<RowOp> = was
                .keys()
                .filter(|k| !now.contains_key(*k))
                .map(|k| RowOp::Delete(k.clone()))
                .collect();
            ops.extend(
                now.iter()
                    .filter(|(k, v)| was.get(*k) != Some(*v))
                    .map(|(_, v)| RowOp::Upsert(v.clone())),
            );
            *was = now;
            ops
        };
        if !ops.is_empty() {
            self.host.publish(BADGES, ops);
        }
    }

    /// Forgets the badge rows it published, as it is switched off.
    pub fn forget_badges(&self) {
        self.badges
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// A core call, for the tools.
    pub(crate) async fn core(&self, method: &str, params: Value) -> Result<Value, String> {
        self.host.call(method, params).await
    }
}

#[cfg(test)]
pub(crate) mod tests;
