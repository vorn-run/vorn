//! The task board's calls (`task:*`): list, read, create, update, move,
//! reorder, archive and delete, with the rules the server kept. A task in a
//! project that does not exist is refused; finishing one stamps
//! `completedAt` and reopening one clears it and `archivedAt`; a reorder
//! permutes the places the named tasks already hold rather than numbering
//! them.

use serde_json::{json, Map, Value};
use vorn_store::{Result, Store};

/// Whether a status is one a task ends in (`isTerminalTaskStatus`).
pub fn is_terminal(status: Option<&str>) -> bool {
    matches!(status, Some("done" | "cancelled"))
}

/// What a status change does to the dates that hang off it (`terminalStamps`).
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

fn task(store: &mut Store, id: &Value) -> Result<Option<Value>> {
    let found = store.call("dbGetTask", json!([id]))?;
    Ok((!found.is_null()).then_some(found))
}

fn project_exists(store: &mut Store, name: &Value) -> Result<bool> {
    Ok(!store.call("dbGetProject", json!([name]))?.is_null())
}

fn max_order(store: &mut Store, project: &Value) -> Result<f64> {
    Ok(store
        .call("dbGetMaxTaskOrder", json!([project]))?
        .as_f64()
        .unwrap_or(-1.0))
}

fn num(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// `task:list`: the board, without descriptions unless asked for, and
/// from `offset` at most `limit` tasks when given.
pub fn list(store: &mut Store, params: &Value) -> Result<Value> {
    let get = |k: &str| params.get(k).cloned().unwrap_or(Value::Null);
    let page = |k: &str| get(k).as_f64().filter(|n| n.is_finite() && *n >= 0.0);
    let tasks = store.call(
        "dbListTasks",
        json!([
            get("projectName"),
            get("status"),
            page("offset"),
            page("limit")
        ]),
    )?;
    if params
        .get("includeDescription")
        .is_some_and(crate::js::truthy)
    {
        return Ok(tasks);
    }
    let mut tasks = tasks;
    for task in tasks.as_array_mut().into_iter().flatten() {
        task["description"] = json!("");
    }
    Ok(tasks)
}

/// `task:get`.
pub fn get(store: &mut Store, params: &Value) -> Result<Value> {
    store.call("dbGetTask", json!([params.get("id")]))
}

/// `task:setStatus`; whether anything changed is the second value.
pub fn set_status(store: &mut Store, params: &Value, now: &str) -> Result<(Value, bool)> {
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
pub fn create(store: &mut Store, params: &Value, id: String, now: &str) -> Result<(Value, bool)> {
    let project = params.get("projectName").cloned().unwrap_or(Value::Null);
    if !project_exists(store, &project)? {
        return Ok((ok(false), false));
    }
    let id = match params.get("id").and_then(Value::as_str).filter(|i| !i.is_empty()) {
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
pub fn update(store: &mut Store, params: &Value, now: &str) -> Result<(Value, bool)> {
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
pub fn patch(store: &mut Store, params: &Value, now: &str) -> Result<(Value, bool)> {
    let id = params.get("id").cloned().unwrap_or(Value::Null);
    let Some(before) = task(store, &id)? else {
        return Ok((ok(false), false));
    };
    let Some(set) = params.get("set").and_then(Value::as_object) else {
        return Ok((ok(false), false));
    };
    if let Some(project) = set.get("projectName").filter(|p| Some(*p) != before.get("projectName")) {
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
pub fn delete(store: &mut Store, params: &Value) -> Result<(Value, bool)> {
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
pub fn reorder(store: &mut Store, params: &Value, now: &str) -> Result<(Value, bool)> {
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
pub fn archive(store: &mut Store, params: &Value, now: &str) -> Result<(Value, bool)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_store::StoreOptions;

    fn store() -> Store {
        let options = StoreOptions {
            default_shell: "/bin/sh".into(),
            default_agent_commands: Map::new(),
            default_workspace: serde_json::from_value(json!({ "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#000", "order": 0 })).unwrap(),
            owner_name: "o".into(),
            seed_workflows: Vec::new(),
        };
        let mut s = Store::open_in_memory(options).unwrap();
        s.call(
            "dbInsertProject",
            json!([{ "name": "app", "path": "/app", "preferredAgents": [] }]),
        )
        .unwrap();
        s
    }

    #[test]
    fn a_patch_sets_only_what_it_names_and_stamps_a_finish() {
        let mut s = store();
        let made = json!({ "id": "mine", "projectName": "app", "title": "T", "branch": "b" });
        let (created, _) = create(&mut s, &made, "unused".into(), "2026-01-01T00:00:00.000Z").unwrap();
        assert_eq!(created["task"]["id"], "mine");
        // The id is the client's, so a second task under it is refused.
        assert_eq!(create(&mut s, &made, "x".into(), "n").unwrap().0, json!({ "ok": false }));

        let set = json!({ "id": "mine", "set": {
            "status": "in_progress", "assignedSessionId": "s1", "assignedAgent": "claude", "bogus": 1
        } });
        let (patched, changed) = patch(&mut s, &set, "2026-01-02T00:00:00.000Z").unwrap();
        assert!(changed);
        let t = &patched["task"];
        assert_eq!((t["status"].as_str(), t["assignedSessionId"].as_str()), (Some("in_progress"), Some("s1")));
        assert_eq!(t["branch"], "b", "a field it does not name stays");
        assert_eq!(t["title"], "T");

        let done = json!({ "id": "mine", "set": { "status": "done", "assignedSessionId": null } });
        let (patched, _) = patch(&mut s, &done, "2026-01-03T00:00:00.000Z").unwrap();
        let t = &patched["task"];
        assert_eq!(t["completedAt"], "2026-01-03T00:00:00.000Z");
        assert!(t.get("assignedSessionId").is_none_or(Value::is_null));
        assert_eq!(t["assignedAgent"], "claude");

        let elsewhere = json!({ "id": "mine", "set": { "projectName": "nowhere" } });
        assert_eq!(patch(&mut s, &elsewhere, "n").unwrap(), (json!({ "ok": false }), false));
        assert_eq!(patch(&mut s, &json!({ "id": "none", "set": {} }), "n").unwrap().1, false);
    }

    #[test]
    fn lists_a_page_of_the_board() {
        let mut s = store();
        for id in ["a", "b", "c"] {
            let params = json!({ "projectName": "app", "title": id, "description": "long" });
            create(&mut s, &params, id.into(), "t0").unwrap();
        }
        let ids = |v: Value| -> Vec<String> {
            let list = v.as_array().unwrap().iter();
            list.map(|t| t["id"].as_str().unwrap().to_owned()).collect()
        };
        assert_eq!(ids(list(&mut s, &json!({})).unwrap()), ["a", "b", "c"]);
        let page = list(&mut s, &json!({ "offset": 1, "limit": 1 })).unwrap();
        assert_eq!(page[0]["description"], "");
        assert_eq!(ids(page), ["b"]);
        assert_eq!(ids(list(&mut s, &json!({ "offset": 2 })).unwrap()), ["c"]);
        // What is not a count is no page.
        let odd = json!({ "offset": "x", "limit": -1 });
        assert_eq!(ids(list(&mut s, &odd).unwrap()).len(), 3);
    }

    #[test]
    fn creates_moves_and_archives_a_task_with_its_dates() {
        let mut s = store();
        let (answer, _) = create(
            &mut s,
            &json!({ "projectName": "nope", "title": "T" }),
            "x".into(),
            "t0",
        )
        .unwrap();
        assert_eq!(answer, json!({ "ok": false }));
        let (answer, changed) = create(
            &mut s,
            &json!({ "projectName": "app", "title": "T", "branch": "" }),
            "a".into(),
            "t0",
        )
        .unwrap();
        assert!(changed);
        assert_eq!(answer["task"]["order"], 0);
        assert!(answer["task"].get("branch").is_none());

        // Not over yet, so it cannot be filed away.
        assert_eq!(
            archive(&mut s, &json!({ "id": "a", "archived": true }), "t1")
                .unwrap()
                .0,
            ok(false)
        );
        set_status(&mut s, &json!({ "id": "a", "status": "done" }), "t2").unwrap();
        let done = get(&mut s, &json!({ "id": "a" })).unwrap();
        assert_eq!(done["completedAt"], "t2");
        assert_eq!(
            archive(&mut s, &json!({ "id": "a", "archived": true }), "t3")
                .unwrap()
                .0,
            ok(true)
        );
        assert_eq!(
            get(&mut s, &json!({ "id": "a" })).unwrap()["archivedAt"],
            "t3"
        );
        // Reopened, both dates go.
        let (answer, _) = update(&mut s, &json!({ "id": "a", "status": "todo" }), "t4").unwrap();
        assert!(answer["task"].get("completedAt").is_none(), "{answer}");
        assert!(answer["task"].get("archivedAt").is_none());
        assert_eq!(answer["task"]["title"], "T");
    }

    #[test]
    fn reorders_by_permuting_the_places_held() {
        let mut s = store();
        for id in ["a", "b", "c"] {
            create(
                &mut s,
                &json!({ "projectName": "app", "title": id }),
                id.into(),
                "t",
            )
            .unwrap();
        }
        let (answer, moved) =
            reorder(&mut s, &json!({ "ids": ["c", "a", "c", "ghost"] }), "t1").unwrap();
        assert_eq!(answer, ok(true));
        assert!(moved);
        let orders: Vec<(String, i64)> = list(&mut s, &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["id"].as_str().unwrap().to_owned(),
                    t["order"].as_f64().unwrap() as i64,
                )
            })
            .collect();
        assert_eq!(orders, [("c".into(), 0), ("b".into(), 1), ("a".into(), 2)]);
        assert!(
            !reorder(&mut s, &json!({ "ids": ["c", "a"] }), "t2")
                .unwrap()
                .1
        );
        assert_eq!(
            reorder(&mut s, &json!({ "ids": ["ghost"] }), "t")
                .unwrap()
                .0,
            ok(false)
        );
    }

    #[test]
    fn lists_without_descriptions_unless_asked_and_deletes() {
        let mut s = store();
        create(
            &mut s,
            &json!({ "projectName": "app", "title": "T", "description": "long" }),
            "a".into(),
            "t",
        )
        .unwrap();
        assert_eq!(list(&mut s, &json!(null)).unwrap()[0]["description"], "");
        assert_eq!(
            list(&mut s, &json!({ "includeDescription": true })).unwrap()[0]["description"],
            "long"
        );
        assert_eq!(delete(&mut s, &json!({ "id": "a" })).unwrap().0, ok(true));
        assert_eq!(delete(&mut s, &json!({ "id": "a" })).unwrap().0, ok(false));
    }
}
