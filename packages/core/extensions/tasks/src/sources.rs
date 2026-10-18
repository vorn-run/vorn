//! Tasks made from the items of a connection, each linked to the item it came
//! from (`task_source_links`), so the next import updates it rather than
//! adding it again.
//!
//! This is the board's side of a backfill or a "save as task" step: the
//! connection reads the items, and these write them. An import does not fire
//! the board's triggers, as a backfill never has.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::Db;
use crate::js::num as number_value;

type Result<T> = std::result::Result<T, String>;

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

/// Puts `item` on the board: the task it is linked to, else one an earlier
/// link left behind, else a new one in `project`. Answers
/// `{taskId, created}`.
pub fn upsert(
    store: &mut dyn Db,
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
    store: &mut dyn Db,
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
pub fn link(store: &mut dyn Db, task_id: &str) -> Result<Value> {
    store.call("dbGetTaskSourceLink", json!([task_id]))
}
