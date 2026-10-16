//! The task list, the one read of the task table the core keeps: `task:list`
//! for clients and `task.list` for extensions, both a page of it. Writing
//! tasks is the Tasks built-in's; telling clients what a write changed
//! ([`told`]) is the core's, so a client hears the rows that changed and
//! never reads the whole table again.

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::warn;
use vorn_store::{Result, Store};

use super::config::{blocking, with_store};
use super::{Answer, Native};

/// Every call this module answers.
pub const METHODS: &[&str] = &["task:list"];

/// The longest page `task.list` gives an extension, and its default.
pub const MAX_PAGE: u64 = 1000;
const DEFAULT_PAGE: u64 = 100;

/// `task:list`: the board, without descriptions unless asked for, and
/// from `offset` at most `limit` tasks when given.
pub fn list(store: &mut Store, params: &Value) -> Result<Value> {
    let get = |k: &str| params.get(k).cloned().unwrap_or(Value::Null);
    let page = |k: &str| get(k).as_f64().filter(|n| n.is_finite() && *n >= 0.0);
    let mut tasks = store.call(
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
        .is_some_and(vorn_work::js::truthy)
    {
        return Ok(tasks);
    }
    for task in tasks.as_array_mut().into_iter().flatten() {
        task["description"] = json!("");
    }
    Ok(tasks)
}

/// `task.list` for an extension: always a page, `{ tasks, next }`, `next`
/// the offset of the page after it or null after the last.
pub fn page(store: &mut Store, args: &Value) -> Result<Value> {
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_PAGE)
        .clamp(1, MAX_PAGE);
    let mut params = json!({ "offset": offset, "limit": limit });
    for key in ["projectName", "status", "includeDescription"] {
        if let Some(v) = args.get(key) {
            params[key] = v.clone();
        }
    }
    let tasks = list(store, &params)?;
    let n = tasks.as_array().map_or(0, Vec::len) as u64;
    let next = (n == limit).then_some(offset + n);
    Ok(json!({ "tasks": tasks, "next": next }))
}

/// Answers `method`.
pub async fn answer(native: &Arc<Native>, method: &str, params: Value) -> Answer {
    if native.database().is_none() {
        // Only a test starts vornd without one.
        return Answer::Unanswered;
    }
    if method != "task:list" {
        return Answer::Unanswered;
    }
    let n = Arc::clone(native);
    match blocking(method, move || with_store(&n, |store| list(store, &params))).await {
        Ok(tasks) => Answer::Result(tasks),
        Err(message) => Answer::Error(message),
    }
}

/// What changed on the board, as clients are told it (`task:rows`).
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// These rows, whole, as `task:get` reads them: new or changed.
    Rows(Vec<Value>),
    /// These tasks are gone.
    Removed(Vec<String>),
    /// Something wrote the table without saying what: read it again.
    Reload,
}

impl Change {
    /// The `task:rows` notification.
    pub fn note(&self) -> Value {
        match self {
            Change::Rows(rows) => json!({ "upsert": rows }),
            Change::Removed(ids) => json!({ "remove": ids }),
            Change::Reload => json!({ "reload": true }),
        }
    }
}

/// Tells every client the board changed: the configuration, as a client of
/// before rereads the board on it, then the rows. Told in the order the
/// changes were stored.
pub async fn told(native: &Arc<Native>, change: Change) {
    let n = Arc::clone(native);
    let _order = native.config_order.lock().await;
    let loaded = blocking("config:changed", move || {
        with_store(&n, |store| store.load_config())
    })
    .await;
    match loaded {
        Ok(config) => {
            native.broadcast("config:changed", config);
            native.broadcast("task:rows", change.note());
            native.config_changed();
        }
        Err(err) => warn!(%err, "could not read the configuration to tell of a task"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
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
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            let task = json!({
                "id": id, "projectName": "app", "title": id, "description": "long",
                "status": if i == 2 { "done" } else { "todo" }, "order": i,
                "createdAt": "t0", "updatedAt": "t0",
            });
            s.call("dbInsertTask", json!([task])).unwrap();
        }
        s
    }

    fn ids(v: &Value) -> Vec<&str> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn lists_a_page_of_the_board() {
        let mut s = store();
        assert_eq!(ids(&list(&mut s, &json!({})).unwrap()), ["a", "b", "c"]);
        let page = list(&mut s, &json!({ "offset": 1, "limit": 1 })).unwrap();
        assert_eq!(page[0]["description"], "");
        assert_eq!(ids(&page), ["b"]);
        assert_eq!(ids(&list(&mut s, &json!({ "offset": 2 })).unwrap()), ["c"]);
        // What is not a count is no page.
        let odd = json!({ "offset": "x", "limit": -1 });
        assert_eq!(ids(&list(&mut s, &odd).unwrap()).len(), 3);
        assert_eq!(
            ids(&list(&mut s, &json!({ "status": "done" })).unwrap()),
            ["c"]
        );
    }

    #[test]
    fn lists_without_descriptions_unless_asked() {
        let mut s = store();
        assert_eq!(list(&mut s, &json!(null)).unwrap()[0]["description"], "");
        assert_eq!(
            list(&mut s, &json!({ "includeDescription": true })).unwrap()[0]["description"],
            "long"
        );
    }

    #[test]
    fn an_extension_reads_the_board_a_page_at_a_time() {
        let mut s = store();
        let first = page(&mut s, &json!({ "limit": 2 })).unwrap();
        assert_eq!(ids(&first["tasks"]), ["a", "b"]);
        assert_eq!(first["next"], 2);
        let last = page(&mut s, &json!({ "limit": 2, "offset": 2 })).unwrap();
        assert_eq!(ids(&last["tasks"]), ["c"]);
        assert!(last["next"].is_null());
        // A page is never the whole table at once, however much is asked for.
        let big = page(
            &mut s,
            &json!({ "limit": 1_000_000, "includeDescription": true }),
        )
        .unwrap();
        assert_eq!(big["tasks"][0]["description"], "long");
        assert!(big["next"].is_null());
        let none = page(&mut s, &json!({ "limit": 0 })).unwrap();
        assert_eq!(ids(&none["tasks"]), ["a"]);
    }

    #[test]
    fn tells_a_change_as_rows_removals_or_a_reload() {
        assert_eq!(
            Change::Rows(vec![json!({ "id": "a" })]).note(),
            json!({ "upsert": [{ "id": "a" }] })
        );
        assert_eq!(
            Change::Removed(vec!["a".into()]).note(),
            json!({ "remove": ["a"] })
        );
        assert_eq!(Change::Reload.note(), json!({ "reload": true }));
    }
}
