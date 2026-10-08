//! Connections: rows of `vorn.db` that tie Vorn to one account of one
//! connector, and what comes with them: the built-in connectors' manifests,
//! the workflows a connection is seeded with, the tasks its items become,
//! and the secrets it holds, described without being disclosed.

use std::collections::HashMap;

use serde_json::{json, Map, Value};
use vorn_store::Store;

/// The connector every package connection belongs to.
pub const SDK: &str = "sdk";
/// The built-in MCP connector.
pub const MCP: &str = "mcp";
/// The built-in HTTP connector, whose connections are auth profiles.
pub const HTTP: &str = "http";
/// The event a package connection's poll fires on.
pub const POLL_EVENT: &str = "mcpPoll";
/// The field that carries a set of secret variables rather than one value.
pub const SECRET_ENV: &str = "secretEnv";
/// What a connection's row holds where a secret was, once the vault has it.
pub const IN_VAULT: &str = "vorn-vault";

/// The filters that tie a connection to its package (`SDK_FILTER_KEYS`).
pub mod filter {
    pub const CONNECTOR_ID: &str = "sdkConnectorId";
    pub const VERSION: &str = "sdkVersion";
    pub const ICON: &str = "sdkIcon";
    pub const IMPLICIT: &str = "implicit";
    pub const TRIGGER: &str = "sdkTrigger";
}

/// The built-in connectors as `connector:list` describes them.
pub fn builtins() -> Vec<Value> {
    serde_json::from_str(include_str!("../data/builtin-connectors.json"))
        .expect("the built-in connectors are JSON")
}

/// The built-in connector `id`, as `connector:get` describes it.
pub fn builtin(id: &str) -> Option<Value> {
    builtins()
        .into_iter()
        .find(|c| c.get("id").and_then(Value::as_str) == Some(id))
}

/// A connector's auth fields, from its manifest.
pub fn auth_fields(connector_id: &str) -> Vec<Value> {
    builtin(connector_id)
        .and_then(|c| c.pointer("/manifest/auth").cloned())
        .and_then(|a| a.as_array().cloned())
        .unwrap_or_default()
}

/// The fields a connector says hold secrets (`passwordFields`).
pub fn password_fields(connector_id: &str) -> Vec<Value> {
    auth_fields(connector_id)
        .into_iter()
        .filter(|f| f.get("type").and_then(Value::as_str) == Some("password"))
        .collect()
}

pub fn filters_of(conn: &Value) -> Map<String, Value> {
    conn.get("filters")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn text<'a>(conn: &'a Value, key: &str) -> &'a str {
    conn.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The connector a connection belongs to as people know it: its package's
/// id for a package connection (`connectionConnectorId`).
pub fn connector_id_of(conn: &Value) -> String {
    match conn.pointer(&format!("/filters/{}", filter::CONNECTOR_ID)) {
        Some(Value::String(id)) if !id.is_empty() => id.clone(),
        _ => text(conn, "connectorId").to_owned(),
    }
}

/// The package a connection runs, when it names one (`sdkIdOf`).
pub fn sdk_id_of(conn: &Value) -> String {
    match conn.pointer(&format!("/filters/{}", filter::CONNECTOR_ID)) {
        None | Some(Value::Null) => String::new(),
        Some(v) => crate::js::trim(&crate::js::to_string(v)).to_owned(),
    }
}

/// The trigger a package connection polls (`sdkTriggerOf`).
pub fn sdk_trigger_of(conn: &Value) -> String {
    match conn.pointer(&format!("/filters/{}", filter::TRIGGER)) {
        None | Some(Value::Null) => String::new(),
        Some(v) => crate::js::trim(&crate::js::to_string(v)).to_owned(),
    }
}

/// Whether a connection came with its pack rather than being added.
pub fn is_implicit(conn: &Value) -> bool {
    conn.pointer(&format!("/filters/{}", filter::IMPLICIT)) == Some(&Value::Bool(true))
}

/// The filters that tie a connection to a pack (`sdkConnectionFilters`).
pub fn sdk_connection_filters(pack: &Value, trigger: Option<&str>) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert(filter::CONNECTOR_ID.into(), pack.get("id").cloned().unwrap_or(Value::Null));
    out.insert(filter::VERSION.into(), pack.get("version").cloned().unwrap_or(Value::Null));
    if let Some(icon) = pack.get("icon").filter(|i| crate::js::truthy(i)) {
        out.insert(filter::ICON.into(), json!(icon.to_string()));
    }
    if let Some(t) = trigger.filter(|t| !t.is_empty()) {
        out.insert(filter::TRIGGER.into(), json!(t));
    }
    out
}

/// The id of the workflow a connection is seeded with for `event`.
pub fn seeded_workflow_id(connection_id: &str, event: &str) -> String {
    format!("connector:{connection_id}:{event}")
}

/// The prefix of every workflow seeded for a connection.
pub fn seeded_workflow_prefix(connection_id: &str) -> String {
    format!("connector:{connection_id}:")
}

/// `cronEveryMinutes`.
fn cron_every(minutes: i64) -> String {
    if minutes <= 1 {
        return "* * * * *".to_owned();
    }
    if minutes < 60 {
        return format!("*/{minutes} * * * *");
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours <= 1 {
        "0 * * * *".to_owned()
    } else if hours >= 24 {
        "0 0 * * *".to_owned()
    } else {
        format!("0 */{hours} * * *")
    }
}

/// The workflow a connection is seeded with for one of its manifest's events
/// (`buildConnectorSeededWorkflow`): a poll trigger feeding a task.
pub fn seeded_workflow(conn: &Value, manifest: &Value, event: &Value) -> Value {
    let event_name = event.get("event").and_then(Value::as_str).unwrap_or("");
    let minutes = event
        .get("defaultCronFromMinutes")
        .and_then(Value::as_f64)
        .map_or(1, |m| (m.round() as i64).max(1));
    let initial = manifest
        .pointer("/statusMapping/0/suggestedLocal")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("todo");
    let trigger_label = manifest
        .get("triggers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|t| t.get("type").and_then(Value::as_str) == Some(event_name))
        .and_then(|t| t.get("label").and_then(Value::as_str))
        .filter(|l| !l.is_empty())
        .unwrap_or(event_name);
    let id = text(conn, "id");
    json!({
        "id": seeded_workflow_id(id, event_name),
        "name": event.get("name"),
        "icon": conn.get("connectorId"),
        "iconColor": "#64748b",
        "enabled": true,
        "workspaceId": "personal",
        "nodes": [
            {
                "id": "trigger-1",
                "type": "trigger",
                "label": format!("Poll {trigger_label}"),
                "position": { "x": 0, "y": 0 },
                "config": {
                    "triggerType": "connectorPoll",
                    "connectionId": id,
                    "event": event_name,
                    "cron": cron_every(minutes),
                },
            },
            {
                "id": "create-1",
                "type": "createTaskFromItem",
                "label": "Create task from item",
                "position": { "x": 0, "y": 120 },
                "config": { "nodeType": "createTaskFromItem", "project": "fromConnection", "initialStatus": initial },
            },
        ],
        "edges": [{ "id": "e1", "source": "trigger-1", "target": "create-1" }],
    })
}

/// What a connection is made from (`connection:create`'s params).
pub struct NewConnection<'a> {
    pub id: String,
    pub params: &'a Map<String, Value>,
    pub now: String,
}

/// Inserts a connection and the workflows it is seeded with; the row as stored.
pub fn create(store: &mut Store, new: NewConnection<'_>) -> vorn_store::Result<Value> {
    let p = new.params;
    let mut conn = Map::new();
    conn.insert("id".into(), json!(new.id));
    for key in ["connectorId", "name", "filters", "syncIntervalMinutes", "statusMapping"] {
        conn.insert(key.into(), p.get(key).cloned().unwrap_or(Value::Null));
    }
    if let Some(project) = p.get("executionProject").filter(|v| crate::js::truthy(v)) {
        conn.insert("executionProject".into(), project.clone());
    }
    conn.insert("createdAt".into(), json!(new.now));
    let conn = Value::Object(conn);
    store.call("dbInsertSourceConnection", json!([conn]))?;

    let connector = text(&conn, "connectorId");
    let manifest = builtin(connector).and_then(|c| c.get("manifest").cloned());
    if let Some(mut manifest) = manifest {
        let mut events: Vec<Value> = manifest
            .get("defaultWorkflows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let (Some(seed), true) = (p.get("seedWorkflow").filter(|s| s.is_object()), connector == SDK) {
            events.push(json!({
                "name": seed.get("name"),
                "event": POLL_EVENT,
                "defaultCronFromMinutes": seed.get("defaultCronFromMinutes"),
                "downstream": "createTaskFromItem",
            }));
        }
        let mapping: Vec<Value> = conn
            .get("statusMapping")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(upstream, local)| json!({ "upstream": upstream, "suggestedLocal": local }))
            .collect();
        manifest["statusMapping"] = Value::Array(mapping);
        for event in &events {
            let name = event.get("event").and_then(Value::as_str).unwrap_or("");
            let wf_id = seeded_workflow_id(&new.id, name);
            if !store.call("dbGetWorkflow", json!([wf_id]))?.is_null() {
                continue;
            }
            let wf = seeded_workflow(&conn, &manifest, event);
            store.call("dbInsertWorkflow", json!([wf]))?;
        }
    }
    Ok(conn)
}

/// Deletes a connection and the workflows seeded for it; its task links go
/// with it by foreign key.
pub fn delete(store: &mut Store, id: &str) -> vorn_store::Result<()> {
    let prefix = seeded_workflow_prefix(id);
    let workflows = store.call("dbListWorkflows", json!([]))?;
    for wf in workflows.as_array().into_iter().flatten() {
        if let Some(wf_id) = wf.get("id").and_then(Value::as_str).filter(|i| i.starts_with(&prefix)) {
            store.call("dbDeleteWorkflow", json!([wf_id]))?;
        }
    }
    store.call("dbDeleteSourceConnection", json!([id]))?;
    Ok(())
}

/// One external item, as a task is made from it.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub external_id: String,
    pub title: String,
    pub description: String,
    pub external_url: String,
    pub status_raw: String,
    pub updated_at: String,
}

/// Puts an external item on the task board (`upsertExternalItem`): the task
/// it is linked to, else one an earlier link left behind, else a new one.
pub fn upsert_item(
    store: &mut Store,
    conn: &Value,
    item: &Item,
    project: &str,
    initial_status: &Value,
    now: &str,
    new_task_id: impl FnOnce() -> String,
) -> vorn_store::Result<Value> {
    let conn_id = text(conn, "id");
    let task_fields = json!({
        "title": item.title,
        "description": item.description,
        "updatedAt": now,
        "sourceExternalUrl": item.external_url,
        "sourceExternalId": item.external_id,
    });
    let keys = json!(["title", "description", "updatedAt", "sourceExternalUrl", "sourceExternalId"]);
    let existing = store.call(
        "dbGetTaskSourceLinkByExternalId",
        json!([conn_id, item.external_id]),
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
            "connectionId": conn_id,
            "connectorId": conn.get("connectorId"),
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
        json!([connector_id_of(conn), item.external_id]),
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
        "order": crate::js::json_number(max + 1.0),
        "createdAt": now,
        "updatedAt": now,
        "sourceConnectorId": connector_id_of(conn),
        "sourceExternalId": item.external_id,
    });
    if !item.external_url.is_empty() {
        task["sourceExternalUrl"] = json!(item.external_url);
    }
    store.call("dbInsertTask", json!([task]))?;
    store.call("dbInsertTaskSourceLink", json!([link(&task_id)]))?;
    Ok(json!({ "taskId": task_id, "created": true }))
}

/// Updates a connection's row; a key named in `cleared` and absent from
/// `updates` is cleared, as `undefined` clears it.
pub fn update(store: &mut Store, id: &str, updates: Map<String, Value>, cleared: &[&str]) -> vorn_store::Result<()> {
    let mut keys: Vec<String> = updates.keys().cloned().collect();
    keys.extend(cleared.iter().map(|k| (*k).to_owned()));
    store.call("dbUpdateSourceConnection", json!([id, updates, keys]))?;
    Ok(())
}

/// The environment a script step's named connection gives it
/// (`secretEnvFor`): its `secretEnv` blob's variables, and each other secret
/// field under its name in capitals.
pub fn script_env(fields: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();
    let mut put = |k: String, v: String| match env.iter_mut().find(|(key, _)| *key == k) {
        Some(slot) => slot.1 = v,
        None => env.push((k, v)),
    };
    for (key, value) in fields {
        if key != SECRET_ENV {
            put(env_name_for(key), value.clone());
            continue;
        }
        if let Ok(Value::Object(blob)) = serde_json::from_str::<Value>(value) {
            for (name, v) in blob {
                if let (Some(v), true) = (v.as_str(), is_env_name(&name)) {
                    put(name, v.to_owned());
                }
            }
        }
    }
    env
}

/// `fooBar` as `FOO_BAR`.
fn env_name_for(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    let mut prev: Option<char> = None;
    for c in key.chars() {
        if c.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit()) {
            out.push('_');
        }
        out.push(c);
        prev = Some(c);
    }
    out.to_uppercase()
}

// ---- keys ----

/// Published key prefixes: naming one says which service a value belongs to.
const VENDOR_MARKERS: [&str; 21] = [
    "sk_live_", "sk_test_", "pk_live_", "pk_test_", "rk_live_", "whsec_", "github_pat_", "ghp_",
    "gho_", "ghs_", "ghu_", "glpat-", "xoxb-", "xoxp-", "xoxa-", "xapp-", "shpat_", "npm_",
    "dop_v1_", "AKIA", "ASIA",
];

/// A known marker plus the last four characters (`maskSecret`).
pub fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if crate::js::len16(value) < 12 {
        return "••••".to_owned();
    }
    let marker = VENDOR_MARKERS.iter().find(|m| value.starts_with(*m)).copied().unwrap_or("");
    format!("{marker}••••{}", crate::js::tail16(value, 4))
}

/// A name a shell accepts that cannot reach the prototype (`isEnvName`).
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(name, "__proto__" | "constructor" | "prototype")
}

/// The variable names a `secretEnv` blob carries (`envNamesOf`).
pub fn env_names_of(blob: Option<&str>) -> Vec<String> {
    match blob.filter(|b| !b.is_empty()).and_then(|b| serde_json::from_str::<Value>(b).ok()) {
        Some(Value::Object(map)) => map.keys().filter(|k| is_env_name(k)).cloned().collect(),
        _ => Vec::new(),
    }
}

/// The field naming the connection a step runs against (`boundConnectionKey`).
fn bound_key(node: &Value, config: &Map<String, Value>) -> Option<&'static str> {
    match node.get("type").and_then(Value::as_str) {
        Some("trigger") if config.get("triggerType").and_then(Value::as_str) == Some("connectorPoll") => {
            Some("connectionId")
        }
        Some("callConnectorAction") => Some("connectionId"),
        Some("httpRequest") => Some("profileConnectionId"),
        Some("script") => Some("secretsFrom"),
        _ => None,
    }
}

/// How many workflow steps run against each connection (`usageCounts`).
pub fn usage_counts(workflows: &[Value]) -> HashMap<String, u64> {
    let mut counts = HashMap::new();
    for wf in workflows {
        for node in wf.get("nodes").and_then(Value::as_array).into_iter().flatten() {
            let config = node.get("config").and_then(Value::as_object).cloned().unwrap_or_default();
            let Some(key) = bound_key(node, &config) else { continue };
            let id = match config.get(key) {
                None | Some(Value::Null) => String::new(),
                Some(v) => crate::js::to_string(v),
            };
            if !id.is_empty() {
                *counts.entry(id).or_default() += 1;
            }
        }
    }
    counts
}

/// Every connection that holds a secret, with what rotating it touches
/// (`listKeys`). `secrets` gives a connection's readable fields.
pub fn list_keys(
    connections: &[Value],
    auth_of: impl Fn(&str) -> Vec<Value>,
    workflows: &[Value],
    secrets: impl Fn(&str) -> Option<HashMap<String, String>>,
) -> Vec<Value> {
    let counts = usage_counts(workflows);
    let mut keys: Vec<Value> = Vec::new();
    for conn in connections {
        let id = text(conn, "id");
        let readable = secrets(id).unwrap_or_default();
        let stored = filters_of(conn);
        let fields: Vec<Value> = auth_of(text(conn, "connectorId"))
            .into_iter()
            .filter(|f| f.get("type").and_then(Value::as_str) == Some("password"))
            .filter_map(|field| {
                let key = field.get("key").and_then(Value::as_str)?;
                let label = field.get("label").cloned().unwrap_or(Value::Null);
                stored.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())?;
                let value = readable.get(key);
                Some(if key == SECRET_ENV {
                    json!({ "key": key, "label": label, "readable": value.is_some(),
                            "envNames": env_names_of(value.map(String::as_str)) })
                } else {
                    json!({ "key": key, "label": label, "readable": value.is_some(),
                            "hint": mask_secret(value.map_or("", String::as_str)) })
                })
            })
            .collect();
        if fields.is_empty() {
            continue;
        }
        keys.push(json!({
            "connectionId": id,
            "name": conn.get("name"),
            "connectorId": connector_id_of(conn),
            "fields": fields,
            "usageCount": counts.get(id).copied().unwrap_or(0),
        }));
    }
    keys.sort_by(|a, b| crate::js::locale_compare(text(a, "name"), text(b, "name")));
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_script_step_s_secrets() {
        let fields = vec![
            ("apiKey".to_owned(), "k".to_owned()),
            ("secretEnv".to_owned(), r#"{"A":"1","B":2,"__proto__":"x"}"#.to_owned()),
        ];
        assert_eq!(script_env(&fields), [("API_KEY".to_owned(), "k".to_owned()), ("A".to_owned(), "1".to_owned())]);
        assert_eq!(env_name_for("token2Value"), "TOKEN2_VALUE");
    }

    #[test]
    fn masks_a_secret_as_a_card_is_quoted() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("short"), "••••");
        assert_eq!(mask_secret("ghp_abcdefghijklmnop"), "ghp_••••mnop");
        assert_eq!(mask_secret("plainvalue12345"), "••••2345");
    }

    #[test]
    fn reads_env_names_and_refuses_the_prototype() {
        assert_eq!(env_names_of(Some(r#"{"A":"1","__proto__":"x","1B":"y","_c":"z"}"#)), ["A", "_c"]);
        assert!(env_names_of(Some("[1]")).is_empty());
        assert!(env_names_of(None).is_empty());
    }

    #[test]
    fn seeds_a_poll_workflow_for_a_connection() {
        let conn = json!({ "id": "c1", "connectorId": "sdk" });
        let manifest = json!({ "statusMapping": [{ "upstream": "Open", "suggestedLocal": "in_progress" }],
                               "triggers": [{ "type": "mcpPoll", "label": "Poll" }] });
        let wf = seeded_workflow(&conn, &manifest, &json!({ "name": "Tickets", "event": "mcpPoll", "defaultCronFromMinutes": 120 }));
        assert_eq!(wf["id"], "connector:c1:mcpPoll");
        assert_eq!(wf["nodes"][0]["config"]["cron"], "0 */2 * * *");
        assert_eq!(wf["nodes"][0]["label"], "Poll Poll");
        assert_eq!(wf["nodes"][1]["config"]["initialStatus"], "in_progress");
        assert_eq!(cron_every(5), "*/5 * * * *");
        assert_eq!(cron_every(1), "* * * * *");
        assert_eq!(cron_every(80), "0 * * * *");
        assert_eq!(cron_every(1440), "0 0 * * *");
    }

    #[test]
    fn counts_the_steps_bound_to_each_connection() {
        let wf = json!({ "nodes": [
            { "type": "trigger", "config": { "triggerType": "connectorPoll", "connectionId": "c1" } },
            { "type": "callConnectorAction", "config": { "connectionId": "c1" } },
            { "type": "httpRequest", "config": { "profileConnectionId": "h" } },
            { "type": "script", "config": { "secretsFrom": "" } },
        ] });
        let counts = usage_counts(&[wf]);
        assert_eq!(counts["c1"], 2);
        assert_eq!(counts["h"], 1);
        assert_eq!(counts.len(), 2);
    }

    #[test]
    fn lists_the_keys_a_connection_holds_without_their_values() {
        let conns = vec![
            json!({ "id": "b", "name": "Beta", "connectorId": "http", "filters": { "secret": IN_VAULT } }),
            json!({ "id": "a", "name": "alpha", "connectorId": "sdk",
                    "filters": { "secretEnv": IN_VAULT, "sdkConnectorId": "ado" } }),
            json!({ "id": "n", "name": "None", "connectorId": "http", "filters": {} }),
        ];
        let secrets = |id: &str| -> Option<HashMap<String, String>> {
            (id == "a").then(|| HashMap::from([("secretEnv".to_owned(), r#"{"TOKEN":"x"}"#.to_owned())]))
        };
        let keys = list_keys(&conns, password_fields, &[], secrets);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0]["name"], "alpha");
        assert_eq!(keys[0]["connectorId"], "ado");
        assert_eq!(keys[0]["fields"][0]["envNames"], json!(["TOKEN"]));
        assert_eq!(keys[1]["fields"][0], json!({ "key": "secret", "label": "Secret", "readable": false, "hint": "" }));
    }

    #[test]
    fn knows_a_package_connection() {
        let conn = json!({ "connectorId": "sdk", "filters": { "sdkConnectorId": " ado ", "sdkTrigger": "items", "implicit": true } });
        assert_eq!(sdk_id_of(&conn), "ado");
        assert_eq!(connector_id_of(&conn), " ado ");
        assert_eq!(sdk_trigger_of(&conn), "items");
        assert!(is_implicit(&conn));
        let filters = sdk_connection_filters(&json!({ "id": "x", "version": "1", "icon": { "paths": [] } }), Some("t"));
        assert_eq!(filters[filter::ICON], json!(r#"{"paths":[]}"#));
        assert_eq!(filters[filter::TRIGGER], "t");
        assert_eq!(password_fields("mcp").len(), 1);
        assert!(builtin("nope").is_none());
    }
}
