//! The task board's calls (`task:*`): create, update, move, reorder,
//! archive and delete. Reading and listing are the core's. A task in a project that does
//! not exist is refused; finishing one stamps `completedAt` and reopening one
//! clears it and `archivedAt`; a reorder permutes the places the named tasks
//! already hold rather than numbering them.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::db::Db;
use crate::triggers;
use crate::Change;

type Result<T> = std::result::Result<T, String>;

/// Whether a status is one a task ends in (`isTerminalTaskStatus`).
pub fn is_terminal(status: Option<&str>) -> bool {
    matches!(status, Some("done" | "cancelled"))
}

/// What a status change does to the dates that hang off it.
fn stamps(
    from: Option<&str>,
    to: Option<&str>,
    now: &str,
    updates: &mut Map<String, Value>,
    keys: &mut Vec<String>,
) {
    let (was, is) = (is_terminal(from), is_terminal(to));
    if is && !was {
        updates.insert("completedAt".into(), json!(now));
        keys.push("completedAt".into());
    } else if !is && was {
        keys.push("completedAt".into());
        keys.push("archivedAt".into());
    }
}

fn ok(ok: bool) -> Value {
    json!({ "ok": ok })
}

fn task(store: &mut dyn Db, id: &Value) -> Result<Option<Value>> {
    let found = store.call("dbGetTask", json!([id]))?;
    Ok((!found.is_null()).then_some(found))
}

fn project_exists(store: &mut dyn Db, name: &Value) -> Result<bool> {
    Ok(!store.call("dbGetProject", json!([name]))?.is_null())
}

fn max_order(store: &mut dyn Db, project: &Value) -> Result<f64> {
    Ok(store
        .call("dbGetMaxTaskOrder", json!([project]))?
        .as_f64()
        .unwrap_or(-1.0))
}

fn num(n: f64) -> Value {
    crate::js::num(n)
}

/// `task:setStatus`; whether anything changed is the second value.
pub fn set_status(store: &mut dyn Db, params: &Value, now: &str) -> Result<(Value, bool)> {
    let id = params.get("id").cloned().unwrap_or(Value::Null);
    let Some(task) = task(store, &id)? else {
        return Ok((ok(false), false));
    };
    let status = params.get("status").cloned().unwrap_or(Value::Null);
    let mut updates = Map::new();
    let mut keys = vec!["status".to_owned(), "updatedAt".to_owned()];
    if !status.is_null() {
        updates.insert("status".into(), status.clone());
    }
    updates.insert("updatedAt".into(), json!(now));
    stamps(
        task.get("status").and_then(Value::as_str),
        status.as_str(),
        now,
        &mut updates,
        &mut keys,
    );
    store.call("dbUpdateTask", json!([id, updates, keys]))?;
    Ok((ok(true), true))
}

/// `task:create`: the task as stored, in a project that exists. A client
/// may name the task's id (its images are kept under it already); one that
/// is taken is refused, and without one the task gets `id`.
pub fn create(store: &mut dyn Db, params: &Value, id: String, now: &str) -> Result<(Value, bool)> {
    let project = params.get("projectName").cloned().unwrap_or(Value::Null);
    if !project_exists(store, &project)? {
        return Ok((ok(false), false));
    }
    let id = match params
        .get("id")
        .and_then(Value::as_str)
        .filter(|i| !i.is_empty())
    {
        Some(asked) if task(store, &json!(asked))?.is_some() => return Ok((ok(false), false)),
        Some(asked) => asked.to_owned(),
        None => id,
    };
    let status = params
        .get("status")
        .filter(|s| !s.is_null())
        .cloned()
        .unwrap_or_else(|| json!("todo"));
    let mut task = Map::new();
    task.insert("id".into(), json!(id));
    task.insert("projectName".into(), project.clone());
    task.insert(
        "title".into(),
        params.get("title").cloned().unwrap_or(Value::Null),
    );
    task.insert(
        "description".into(),
        params
            .get("description")
            .filter(|d| !d.is_null())
            .cloned()
            .unwrap_or_else(|| json!("")),
    );
    task.insert("status".into(), status.clone());
    task.insert("order".into(), num(max_order(store, &project)? + 1.0));
    task.insert("createdAt".into(), json!(now));
    task.insert("updatedAt".into(), json!(now));
    for key in ["branch", "useWorktree", "assignedAgent"] {
        if let Some(v) = params.get(key).filter(|v| crate::js::truthy(v)) {
            task.insert(key.into(), v.clone());
        }
    }
    if is_terminal(status.as_str()) {
        task.insert("completedAt".into(), json!(now));
    }
    let task = Value::Object(task);
    store.call("dbInsertTask", json!([task]))?;
    Ok((json!({ "ok": true, "task": task }), true))
}

/// `task:update`: only the six fields it names, never a date a caller sends.
pub fn update(store: &mut dyn Db, params: &Value, now: &str) -> Result<(Value, bool)> {
    let id = params.get("id").cloned().unwrap_or(Value::Null);
    let Some(task) = task(store, &id)? else {
        return Ok((ok(false), false));
    };
    let project = params.get("projectName").filter(|p| !p.is_null());
    let moving = project.is_some_and(|p| Some(p) != task.get("projectName"));
    if moving && !project_exists(store, project.unwrap_or(&Value::Null))? {
        return Ok((ok(false), false));
    }
    let mut updates = Map::new();
    let mut keys: Vec<String> = Vec::new();
    let field = |k: &str, updates: &mut Map<String, Value>, keys: &mut Vec<String>| {
        keys.push(k.to_owned());
        if let Some(v) = params.get(k).filter(|v| !v.is_null()) {
            updates.insert(k.to_owned(), v.clone());
        }
    };
    field("projectName", &mut updates, &mut keys);
    if moving {
        let order = max_order(store, project.unwrap_or(&Value::Null))? + 1.0;
        updates.insert("order".into(), num(order));
        keys.push("order".into());
    }
    for k in [
        "title",
        "description",
        "status",
        "branch",
        "useWorktree",
        "assignedAgent",
    ] {
        field(k, &mut updates, &mut keys);
    }
    updates.insert("updatedAt".into(), json!(now));
    keys.push("updatedAt".into());
    let status = params
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if status.is_some() {
        stamps(
            task.get("status").and_then(Value::as_str),
            status,
            now,
            &mut updates,
            &mut keys,
        );
    }
    store.call("dbUpdateTask", json!([id, updates, keys]))?;
    let after = store.call("dbGetTask", json!([id]))?;
    let mut answer = json!({ "ok": true });
    if !after.is_null() {
        answer["task"] = after;
    }
    Ok((answer, true))
}

/// The fields `task:patch` sets; a date it does not set follows the status.
const PATCHED: [&str; 12] = [
    "projectName",
    "title",
    "description",
    "status",
    "order",
    "branch",
    "useWorktree",
    "assignedAgent",
    "assignedSessionId",
    "agentSessionId",
    "completedAt",
    "archivedAt",
];

/// The fields a task may be without, which `null` clears.
const CLEARABLE: [&str; 7] = [
    "branch",
    "useWorktree",
    "assignedAgent",
    "assignedSessionId",
    "agentSessionId",
    "completedAt",
    "archivedAt",
];

/// `task:patch {id, set}`: sets exactly the fields `set` names, `null`
/// clearing one, and nothing else. A move to another project needs that
/// project; a status change stamps the dates the caller did not set.
pub fn patch(store: &mut dyn Db, params: &Value, now: &str) -> Result<(Value, bool)> {
    let id = params.get("id").cloned().unwrap_or(Value::Null);
    let Some(before) = task(store, &id)? else {
        return Ok((ok(false), false));
    };
    let Some(set) = params.get("set").and_then(Value::as_object) else {
        return Ok((ok(false), false));
    };
    if let Some(project) = set
        .get("projectName")
        .filter(|p| Some(*p) != before.get("projectName"))
    {
        if !project_exists(store, project)? {
            return Ok((ok(false), false));
        }
    }
    let mut updates = Map::new();
    let mut keys: Vec<String> = Vec::new();
    for (k, v) in set.iter().filter(|(k, _)| PATCHED.contains(&k.as_str())) {
        if v.is_null() && !CLEARABLE.contains(&k.as_str()) {
            continue;
        }
        keys.push(k.clone());
        updates.insert(k.clone(), v.clone());
    }
    if let Some(status) = set.get("status").and_then(Value::as_str) {
        let (mut stamped, mut stamped_keys) = (Map::new(), Vec::new());
        stamps(
            before.get("status").and_then(Value::as_str),
            Some(status),
            now,
            &mut stamped,
            &mut stamped_keys,
        );
        for k in stamped_keys.into_iter().filter(|k| !set.contains_key(k)) {
            if let Some(v) = stamped.get(&k) {
                updates.insert(k.clone(), v.clone());
            }
            keys.push(k);
        }
    }
    updates.insert("updatedAt".into(), json!(now));
    keys.push("updatedAt".into());
    store.call("dbUpdateTask", json!([id, updates, keys]))?;
    let after = store.call("dbGetTask", json!([id]))?;
    Ok((json!({ "ok": true, "task": after }), true))
}

/// `task:delete`.
pub fn delete(store: &mut dyn Db, params: &Value) -> Result<(Value, bool)> {
    let id = params.get("id").cloned().unwrap_or(Value::Null);
    if task(store, &id)?.is_none() {
        return Ok((ok(false), false));
    }
    store.call("dbDeleteTask", json!([id]))?;
    Ok((ok(true), true))
}

/// `task:reorder`: the named tasks take the places they already hold, in
/// the order asked; an id sent twice counts once, and one naming nothing
/// takes no place.
pub fn reorder(store: &mut dyn Db, params: &Value, now: &str) -> Result<(Value, bool)> {
    let mut seen: Vec<Value> = Vec::new();
    for id in params
        .get("ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !seen.contains(id) {
            seen.push(id.clone());
        }
    }
    let mut named = Vec::new();
    for id in &seen {
        if let Some(t) = task(store, id)? {
            named.push(t);
        }
    }
    if named.is_empty() {
        return Ok((ok(false), false));
    }
    let order_of = |t: &Value| t.get("order").and_then(Value::as_f64).unwrap_or(0.0);
    let mut slots: Vec<f64> = named.iter().map(order_of).collect();
    slots.sort_by(f64::total_cmp);
    let mut moved = 0;
    for (task, slot) in named.iter().zip(slots) {
        if slot == order_of(task) {
            continue;
        }
        let updates = json!({ "order": num(slot), "updatedAt": now });
        store.call(
            "dbUpdateTask",
            json!([task.get("id"), updates, ["order", "updatedAt"]]),
        )?;
        moved += 1;
    }
    Ok((ok(true), moved > 0))
}

/// `task:archive`: only work that is over may be filed away.
pub fn archive(store: &mut dyn Db, params: &Value, now: &str) -> Result<(Value, bool)> {
    let id = params.get("id").cloned().unwrap_or(Value::Null);
    let Some(task) = task(store, &id)? else {
        return Ok((ok(false), false));
    };
    let archived = params.get("archived").is_some_and(crate::js::truthy);
    if archived && !is_terminal(task.get("status").and_then(Value::as_str)) {
        return Ok((ok(false), false));
    }
    let mut updates = Map::new();
    if archived {
        updates.insert("archivedAt".into(), json!(now));
    }
    updates.insert("updatedAt".into(), json!(now));
    store.call(
        "dbUpdateTask",
        json!([id, updates, ["archivedAt", "updatedAt"]]),
    )?;
    Ok((ok(true), true))
}

/// Whether `method` writes one task, whose change may fire a trigger.
fn written(method: &str) -> bool {
    matches!(
        method,
        "task:create" | "task:setStatus" | "task:update" | "task:patch"
    )
}

/// What a write did: its answer, the change clients are told, and the trigger it fires.
#[derive(Debug, Default)]
pub struct Done {
    pub value: Option<Value>,
    pub change: Option<Change>,
    pub trigger: Option<Value>,
}

fn row(store: &mut dyn Db, id: &Value) -> Result<Option<Value>> {
    let found = store.call("dbGetTask", json!([id]))?;
    Ok((!found.is_null()).then_some(found))
}

/// Runs `method` on the board's tables; a task it creates without an id of its own gets `new_id()`.
pub fn write(
    store: &mut dyn Db,
    method: &str,
    params: &Value,
    now: &str,
    new_id: impl FnOnce() -> String,
) -> Result<Done> {
    // A task being created was not there before, whatever id it is given.
    let before = if written(method) && method != "task:create" {
        row(store, params.get("id").unwrap_or(&Value::Null))?
    } else {
        None
    };
    let (value, changed) = match method {
        "task:setStatus" => set_status(store, params, now)?,
        "task:create" => create(store, params, new_id(), now)?,
        "task:update" => update(store, params, now)?,
        "task:patch" => patch(store, params, now)?,
        "task:delete" => delete(store, params)?,
        "task:reorder" => reorder(store, params, now)?,
        "task:archive" => archive(store, params, now)?,
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
            ids.iter().map(crate::js::to_string).collect(),
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

/// `task.take`: task `id` in progress for a workflow step's session and
/// agent; `task.handBack`: back in the queue, unassigned. The task before
/// and after, or `None` when there is no such task.
pub fn take_or_hand_back(
    store: &mut dyn Db,
    method: &str,
    id: &str,
    args: &Value,
    now: &str,
) -> Result<Option<(Value, Value)>> {
    let mut updates = Map::new();
    if method == "task.take" {
        let text = |k: &str| args.get(k).cloned().unwrap_or(Value::Null);
        updates.insert("status".into(), json!("in_progress"));
        updates.insert("assignedSessionId".into(), text("sessionId"));
        updates.insert("assignedAgent".into(), text("agent"));
        updates.insert("updatedAt".into(), json!(now));
        updates.insert("archivedAt".into(), Value::Null);
    } else {
        updates.insert("status".into(), json!("todo"));
        updates.insert("updatedAt".into(), json!(now));
        updates.insert("completedAt".into(), Value::Null);
        updates.insert("archivedAt".into(), Value::Null);
        updates.insert("assignedSessionId".into(), Value::Null);
        updates.insert("assignedAgent".into(), Value::Null);
    }
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
}

/// Keeps conversation `conversation` on the in-progress task terminal
/// `terminal` was assigned, when that task has none yet; the task after.
pub fn keep_conversation(
    store: &mut dyn Db,
    terminal: &str,
    conversation: &str,
    now: &str,
) -> Result<Option<Value>> {
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
        json!([id, { "agentSessionId": conversation, "updatedAt": now }, ["agentSessionId", "updatedAt"]]),
    )?;
    row(store, &id)
}

/// The card badges the tasks in progress make: one per session, the first task it took.
pub fn badge_rows(working: &Value) -> BTreeMap<String, Value> {
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
    now
}
