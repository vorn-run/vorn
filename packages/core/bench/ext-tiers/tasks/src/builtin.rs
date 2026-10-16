//! The Tasks built-in (`vorn_tasks::builtin`) ported to what a package
//! reaches: every write and its trigger, the rows clients are told changed,
//! and the card badge's rows, through `host.call` and `host.publish`.
//!
//! The extension API has no database, so the board's tables are reached a
//! call at a time over the bridge (`task.store`), and the change and the
//! trigger, which vornd itself sends for the built-in, are asked of it
//! (`task.told`, `task.emit`). A component has no disk, and the API gives a
//! package no folder beside `vorn.db`, so a task's images are not served
//! under either tier.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ext_tiers_logic::{Host, Op};
use serde_json::{json, Value};
use vorn_store::Store;

use crate::{calls, triggers};

/// The collection the card badge draws: one row per session working on a task.
pub const BADGES: &str = "working";

/// Every write it answers: the built-in's, but the images.
pub const METHODS: &[&str] = &[
    "task:setStatus",
    "task:create",
    "task:update",
    "task:patch",
    "task:delete",
    "task:reorder",
    "task:archive",
];

/// What a call for a task's images is told.
pub const NO_IMAGES: &str =
    "no images: the extension API has no blob store, and a component no disk";

/// What a call came to, as the built-in answers it.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Result(Value),
    Void,
    Error(String),
    Unanswered,
}

/// What a write did to the board, as clients are told it (`task:rows`).
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Rows(Vec<Value>),
    Removed(Vec<String>),
}

impl Change {
    fn note(&self) -> Value {
        match self {
            Change::Rows(rows) => json!({ "upsert": rows }),
            Change::Removed(ids) => json!({ "remove": ids }),
        }
    }
}

/// The built-in, over the host it reaches Vorn through.
pub struct Tasks {
    host: Mutex<Box<dyn Host + Send>>,
    /// The badge rows last published, by session, so a change publishes only what moved.
    badges: Mutex<BTreeMap<String, Value>>,
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn written(method: &str) -> bool {
    matches!(
        method,
        "task:create" | "task:setStatus" | "task:update" | "task:patch"
    )
}

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

/// Runs `method` on the store, as the built-in does.
fn write(store: &mut Store, method: &str, params: &Value) -> vorn_store::Result<Done> {
    let now = now_iso();
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

/// Runs `f` on the board's tables, each store call a `task.store` over the bridge.
fn on_store<T>(
    host: &mut dyn Host,
    f: impl FnOnce(&mut Store) -> vorn_store::Result<T>,
) -> Result<T, String> {
    let mut call = |name: &str, args: Value| {
        host.call("task.store", json!({ "call": name, "args": args }))
            .map_err(|e| e.message)
    };
    f(&mut Store::new(&mut call)).map_err(|e| e.to_string())
}

impl Tasks {
    pub fn new(host: Box<dyn Host + Send>) -> Tasks {
        Tasks {
            host: Mutex::new(host),
            badges: Mutex::new(BTreeMap::new()),
        }
    }

    fn host(&self) -> MutexGuard<'_, Box<dyn Host + Send>> {
        self.host.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Answers `method`; the tools call it as they call the built-in.
    pub async fn answer(&self, method: &str, params: Value) -> Answer {
        self.answer_now(method, params)
    }

    /// Answers `method`, start to end: the write, its trigger, the change
    /// clients are told and the badges it moved.
    pub fn answer_now(&self, method: &str, params: Value) -> Answer {
        if method.starts_with("task:image") {
            return Answer::Error(NO_IMAGES.to_owned());
        }
        if !METHODS.contains(&method) {
            return Answer::Unanswered;
        }
        let mut host = self.host();
        let done = match on_store(&mut **host, |store| write(store, method, &params)) {
            Ok(done) => done,
            Err(message) => return Answer::Error(message),
        };
        if let Some(trigger) = done.trigger {
            let name = match trigger.get("kind").and_then(Value::as_str) {
                Some("taskCreated") => "task.created",
                _ => "task.statusChanged",
            };
            // A trigger that cannot be sent is lost, as an event no one hears is.
            let _ = host.call("task.emit", json!({ "name": name, "data": trigger }));
        }
        if let Some(change) = done.change {
            let badged = self.touches_badges(&change);
            let _ = host.call("task.told", change.note());
            if badged {
                self.publish_badges(&mut **host);
            }
        }
        match done.value {
            Some(value) => Answer::Result(value),
            None => Answer::Void,
        }
    }

    /// A core call, for the tools.
    pub(crate) async fn core(&self, method: &str, params: Value) -> Result<Value, String> {
        self.host().call(method, params).map_err(|e| e.message)
    }

    /// Keeps the conversation a terminal started on the in-progress task it
    /// was assigned, when that task has none yet.
    pub fn keep_conversation(&self, terminal: &str, conversation: &str) {
        let mut host = self.host();
        let kept = on_store(&mut **host, |store| {
            let tasks = store.call("dbListTasks", json!([null, "in_progress"]))?;
            let task = tasks.as_array().into_iter().flatten().find(|task| {
                task.get("assignedSessionId").and_then(Value::as_str) == Some(terminal)
                    && task.get("agentSessionId").is_none_or(Value::is_null)
            });
            let Some(id) = task.and_then(|t| t.get("id")).cloned() else {
                return Ok(None);
            };
            store.call(
                "dbUpdateTask",
                json!([id, { "agentSessionId": conversation, "updatedAt": now_iso() }, ["agentSessionId", "updatedAt"]]),
            )?;
            row(store, &id)
        });
        if let Ok(Some(after)) = kept {
            let _ = host.call("task.told", Change::Rows(vec![after]).note());
        }
    }

    fn touches_badges(&self, change: &Change) -> bool {
        let shown = self.badges.lock().unwrap_or_else(PoisonError::into_inner);
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

    /// Publishes the card badge's rows again, as the board is now.
    pub fn badges(&self) {
        let mut host = self.host();
        self.publish_badges(&mut **host);
    }

    fn publish_badges(&self, host: &mut dyn Host) {
        let Ok(working) = on_store(host, |store| {
            store.call("dbListTasks", json!([null, "in_progress"]))
        }) else {
            return;
        };
        let mut now: BTreeMap<String, Value> = BTreeMap::new();
        for task in working.as_array().into_iter().flatten() {
            let Some(session) = task.get("assignedSessionId").and_then(Value::as_str) else {
                continue;
            };
            now.entry(session.to_owned()).or_insert_with(|| {
                json!({
                    "id": session,
                    "task": task.get("id").cloned().unwrap_or(Value::Null),
                    "title": task.get("title").cloned().unwrap_or(Value::Null),
                })
            });
        }
        let ops = {
            let mut was = self.badges.lock().unwrap_or_else(PoisonError::into_inner);
            let mut ops: Vec<Op> = was
                .keys()
                .filter(|k| !now.contains_key(*k))
                .map(|k| Op::Delete(k.clone()))
                .collect();
            ops.extend(
                now.iter()
                    .filter(|(k, v)| was.get(*k) != Some(*v))
                    .map(|(_, v)| Op::Upsert(v.clone())),
            );
            *was = now;
            ops
        };
        if !ops.is_empty() {
            host.publish(BADGES, ops);
        }
    }
}
