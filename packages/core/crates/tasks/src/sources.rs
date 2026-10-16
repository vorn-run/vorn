//! Tasks made from the items of a connection, each linked to the item it came
//! from (`task_source_links`), so the next import updates it rather than
//! adding it again.
//!
//! This is the board's side of a backfill or a "save as task" step: the
//! connection reads the items, and these write them. An import does not fire
//! the board's triggers, as a backfill never has.

use serde::Deserialize;
use serde_json::{json, Value};
use vorn_store::{Result, Store};

use vorn_work::js::number_value;

/// One external item, as a task is made from it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    pub external_id: String,
    pub title: String,
    pub description: String,
    pub external_url: String,
    pub status_raw: String,
    pub updated_at: String,
}

/// Where the items come from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub connection_id: String,
    /// The connection's own connector: `sdk`, `mcp` or `http`.
    pub connector_id: String,
    /// The connector a task names as its source: a package's id, else [`Source::connector_id`].
    pub source_connector_id: String,
}

/// An item and the status a new task from it starts in.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    #[serde(flatten)]
    pub item: Item,
    #[serde(default)]
    pub status: Value,
}

/// `task.importItems`: what a connection imports in one go.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Import {
    #[serde(flatten)]
    pub source: Source,
    /// The project a new task goes in.
    pub project: String,
    pub items: Vec<Imported>,
}

/// Puts `item` on the board (`upsertExternalItem`): the task it is linked
/// to, else one an earlier link left behind, else a new one in `project`.
/// Answers `{taskId, created}`.
pub fn upsert(
    store: &mut Store,
    source: &Source,
    item: &Item,
    project: &str,
    initial_status: &Value,
    now: &str,
    new_task_id: impl FnOnce() -> String,
) -> Result<Value> {
    let task_fields = json!({
        "title": item.title,
        "description": item.description,
        "updatedAt": now,
        "sourceExternalUrl": item.external_url,
        "sourceExternalId": item.external_id,
    });
    let keys = json!([
        "title",
        "description",
        "updatedAt",
        "sourceExternalUrl",
        "sourceExternalId"
    ]);
    let existing = store.call(
        "dbGetTaskSourceLinkByExternalId",
        json!([source.connection_id, item.external_id]),
    )?;
    if let Some(task_id) = existing.get("taskId").and_then(Value::as_str) {
        let task_id = task_id.to_owned();
        store.call("dbUpdateTask", json!([task_id, task_fields, keys]))?;
        store.call(
            "dbUpdateTaskSourceLink",
            json!([task_id, {
                "sourceStatusRaw": item.status_raw,
                "sourceUpdatedAt": item.updated_at,
                "lastSyncedAt": now,
            }]),
        )?;
        return Ok(json!({ "taskId": task_id, "created": false }));
    }
    let link = |task_id: &str| {
        json!({
            "taskId": task_id,
            "connectionId": source.connection_id,
            "connectorId": source.connector_id,
            "externalId": item.external_id,
            "externalUrl": item.external_url,
            "sourceStatusRaw": item.status_raw,
            "sourceUpdatedAt": item.updated_at,
            "lastSyncedAt": now,
            "conflictState": "none",
        })
    };
    let orphan = store.call(
        "dbFindTaskByConnectorExternalId",
        json!([source.source_connector_id, item.external_id]),
    )?;
    if let Some(task_id) = orphan.get("id").and_then(Value::as_str) {
        let task_id = task_id.to_owned();
        store.call("dbUpdateTask", json!([task_id, task_fields, keys]))?;
        store.call("dbInsertTaskSourceLink", json!([link(&task_id)]))?;
        return Ok(json!({ "taskId": task_id, "created": false }));
    }
    let task_id = new_task_id();
    let max = store
        .call("dbGetMaxTaskOrder", json!([project]))?
        .as_f64()
        .unwrap_or(0.0);
    let mut task = json!({
        "id": task_id,
        "projectName": project,
        "title": item.title,
        "description": item.description,
        "status": initial_status,
        "order": number_value(max + 1.0),
        "createdAt": now,
        "updatedAt": now,
        "sourceConnectorId": source.source_connector_id,
        "sourceExternalId": item.external_id,
    });
    if !item.external_url.is_empty() {
        task["sourceExternalUrl"] = json!(item.external_url);
    }
    store.call("dbInsertTask", json!([task]))?;
    store.call("dbInsertTaskSourceLink", json!([link(&task_id)]))?;
    Ok(json!({ "taskId": task_id, "created": true }))
}

/// Every item of `import` on the board: each answer `{taskId, created}` in
/// the order given, as far as the first that fails.
pub fn import(
    store: &mut Store,
    import: &Import,
    now: &str,
    mut new_task_id: impl FnMut() -> String,
) -> Result<Vec<Value>> {
    import
        .items
        .iter()
        .map(|i| {
            upsert(
                store,
                &import.source,
                &i.item,
                &import.project,
                &i.status,
                now,
                &mut new_task_id,
            )
        })
        .collect()
}

/// `task.sourceLink`: where task `task_id` was imported from, or null.
pub fn link(store: &mut Store, task_id: &str) -> Result<Value> {
    store.call("dbGetTaskSourceLink", json!([task_id]))
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
        s.call(
            "dbInsertProject",
            json!([{ "name": "app", "path": "/app", "preferredAgents": [] }]),
        )
        .unwrap();
        for id in ["c1", "c2"] {
            s.call(
                "dbInsertSourceConnection",
                json!([{ "id": id, "connectorId": "sdk", "name": id, "filters": {}, "syncIntervalMinutes": 0, "statusMapping": {}, "createdAt": "2026-01-01T00:00:00.000Z" }]),
            )
            .unwrap();
        }
        s
    }

    fn imported(id: &str, title: &str) -> Imported {
        Imported {
            item: Item {
                external_id: id.into(),
                title: title.into(),
                external_url: format!("https://x.example/{id}"),
                status_raw: "open".into(),
                updated_at: "2026-01-01T00:00:00.000Z".into(),
                ..Item::default()
            },
            status: json!("todo"),
        }
    }

    #[test]
    fn imports_once_and_updates_the_linked_task_after() {
        let mut s = store();
        let args: Import = serde_json::from_value(json!({
            "connectionId": "c1", "connectorId": "sdk", "sourceConnectorId": "tickets",
            "project": "app",
            "items": [
                { "externalId": "1", "title": "One", "status": "todo" },
                { "externalId": "2", "title": "Two", "externalUrl": "https://x.example/2", "status": "todo" },
            ],
        }))
        .unwrap();
        let mut n = 0;
        let mut ids = || {
            n += 1;
            format!("t{n}")
        };
        let first = import(&mut s, &args, "2026-01-01T00:00:00.000Z", &mut ids).unwrap();
        assert_eq!(
            first,
            [
                json!({ "taskId": "t1", "created": true }),
                json!({ "taskId": "t2", "created": true })
            ]
        );
        let again = Import {
            items: vec![imported("1", "One, renamed")],
            ..args.clone()
        };
        let second = import(&mut s, &again, "2026-01-02T00:00:00.000Z", &mut ids).unwrap();
        assert_eq!(second, [json!({ "taskId": "t1", "created": false })]);
        let task = s.call("dbGetTask", json!(["t1"])).unwrap();
        assert_eq!(task["title"], "One, renamed");
        assert_eq!(task["sourceConnectorId"], "tickets");
        assert_eq!(link(&mut s, "t1").unwrap()["connectionId"], "c1");

        // Another connection of the same connector takes over a task whose link was lost.
        s.call("dbDeleteTaskSourceLink", json!(["t2"])).unwrap();
        let other = Import {
            source: Source {
                connection_id: "c2".into(),
                ..args.source.clone()
            },
            items: vec![imported("2", "Two")],
            ..args
        };
        let third = import(&mut s, &other, "2026-01-03T00:00:00.000Z", &mut ids).unwrap();
        assert_eq!(third, [json!({ "taskId": "t2", "created": false })]);
        assert_eq!(link(&mut s, "t2").unwrap()["connectionId"], "c2");
        assert!(link(&mut s, "nope").unwrap().is_null());
    }
}
