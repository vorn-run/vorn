//! The `connection:*` calls vornd answers, and `connector:detectRepo`.
//!
//! Connections are rows of the server's database, which vornd opens beside
//! it ([`vorn_store::Store::open_beside`]) on each call, so a row the server
//! just wrote is the row read here. Reads answer as the server's handlers do.
//! An MCP connection's tools are discovered and its actions run on a child
//! vornd starts itself ([`super::mcp`]), with the secrets the desktop pushed
//! ([`super::secrets`]). Everything about other kinds of connection needs the
//! server's connector registry (built-in connectors, packages, checkouts),
//! and goes to the server, as does an MCP connection whose secrets vornd does
//! not know.
//!
//! The one write, after a discovery, updates the connection's row and then
//! touches `.db-signal` beside the database, as any writer beside the server
//! does: the server reloads its configuration on that and tells its clients
//! the configuration changed, as it does after its own discovery.

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Map, Value};
use tracing::warn;
use vorn_git::repo::Git;
use vorn_store::Store;

use super::secrets::{secret_env, Known};
use super::{absolute_str, mcp, Answer, Native};

/// The connector id of an MCP connection.
const MCP: &str = "mcp";

/// The filter naming the package a connection runs (`SDK_FILTER_KEYS.connectorId`).
const PACKAGE_FILTER: &str = "sdkConnectorId";

/// Whether `method` is answered on the async runtime ([`change`]).
pub fn is_async(method: &str) -> bool {
    matches!(
        method,
        "connection:executeAction" | "connection:refreshMcpTools"
    )
}

/// Answers a call that changes nothing, blocking this thread meanwhile.
pub fn read(native: &Native, method: &str, params: &Value) -> Answer {
    if method == "connector:detectRepo" {
        return match absolute_str(params) {
            Some(dir) => {
                let git = Git {
                    bin: native.env.git_bin(),
                    env: native.env.get(),
                };
                Answer::Result(
                    git.github_origin(Path::new(dir))
                        .map_or(Value::Null, |r| json!({ "owner": r.owner, "repo": r.repo })),
                )
            }
            None => Answer::Forward,
        };
    }
    let Some(mut store) = native.store() else {
        return Answer::Forward;
    };
    match read_rows(&mut store, method, params) {
        Ok(answer) => answer,
        Err(err) => {
            warn!(%method, %err, "could not read the connections; the server answers");
            Answer::Forward
        }
    }
}

fn read_rows(store: &mut Store, method: &str, params: &Value) -> vorn_store::Result<Answer> {
    Ok(match method {
        "connection:list" => {
            // `({ connectorId })` destructures anything but null and undefined.
            let connector = match params {
                Value::Null => return Ok(Answer::Forward),
                Value::Object(p) => match p.get("connectorId") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(_) => return Ok(Answer::Forward),
                },
                _ => None,
            };
            let mut list = from_store(store.call("dbListSourceConnections", json!([connector]))?);
            // The internal webhook row only satisfies the inbox's connection reference.
            if let Value::Array(rows) = &mut list {
                rows.retain(|c| c.get("connectorId").and_then(Value::as_str) != Some("webhook"));
            }
            Answer::Result(list)
        }
        "connection:getSourceLink" => match params.as_str() {
            Some(task) => Answer::Result(from_store(
                store.call("dbGetTaskSourceLink", json!([task]))?,
            )),
            None => Answer::Forward,
        },
        "connection:listMcpTools" => {
            let Some(id) = params.as_str() else {
                return Ok(Answer::Forward);
            };
            match connection(store, id)? {
                Some(conn) if connector_of(&conn) == Some(MCP) => {
                    Answer::Result(Value::Array(discovered_tools(&conn)))
                }
                _ => Answer::Result(json!([])),
            }
        }
        "connection:listActions" => {
            let Some(id) = params.as_str() else {
                return Ok(Answer::Forward);
            };
            match connection(store, id)? {
                None => Answer::Result(json!([])),
                Some(conn) if connector_of(&conn) == Some(MCP) => {
                    let tools = discovered_tools(&conn);
                    let actions: Option<Vec<Value>> = tools
                        .iter()
                        .map(|t| t.is_object().then(|| mcp::tool_action(t)).flatten())
                        .collect();
                    actions.map_or(Answer::Forward, |a| Answer::Result(Value::Array(a)))
                }
                Some(_) => Answer::Forward,
            }
        }
        "connection:preflight" => {
            let Some(id) = params.as_str() else {
                return Ok(Answer::Forward);
            };
            match connection(store, id)? {
                None => Answer::Error(format!("connection {id} not found")),
                // The MCP connector declares no preflight: nothing to check.
                Some(conn) if connector_of(&conn) == Some(MCP) => {
                    Answer::Result(json!({ "ok": null }))
                }
                Some(_) => Answer::Forward,
            }
        }
        _ => Answer::Forward,
    })
}

/// Answers a call that runs an MCP connection's child.
pub async fn change(native: Arc<Native>, method: String, params: Value) -> Answer {
    match method.as_str() {
        "connection:executeAction" => execute(native, params).await,
        "connection:refreshMcpTools" => refresh(native, params).await,
        _ => Answer::Forward,
    }
}

/// What a call on an MCP connection's child starts from.
enum Prepared {
    /// The server's to answer.
    Forward,
    /// No such connection, or not an MCP one: the server's answer for that.
    NotMcp(Option<Value>),
    /// The connection, and its secret variables.
    Ready {
        conn: Value,
        secret_env: Map<String, Value>,
    },
}

/// Reads the connection and what vornd knows of its secrets, off the runtime.
async fn prepare(native: &Arc<Native>, id: String) -> Prepared {
    let n = Arc::clone(native);
    let ready = tokio::task::spawn_blocking(move || {
        let mut store = n.store()?;
        let conn = match connection(&mut store, &id) {
            Ok(conn) => conn,
            Err(err) => {
                warn!(%err, "could not read a connection; the server answers");
                return None;
            }
        };
        let conn = match conn {
            Some(conn) if connector_of(&conn) == Some(MCP) => conn,
            other => return Some(Prepared::NotMcp(other)),
        };
        // A connection that runs a package resolves its launch and borrowed
        // sign-in through the server's packs and checkouts.
        let filters = filters_of(&conn);
        if filters
            .get(PACKAGE_FILTER)
            .is_some_and(|v| !mcp::js_trim(&filter_string(v)).is_empty())
        {
            return Some(Prepared::Forward);
        }
        let secret_env = match n.secrets.lookup(&id) {
            Known::Fields(fields) => secret_env(&fields),
            Known::None => Map::new(),
            // The server holds secrets vornd has not heard of since it
            // started, and the keychain does not have them.
            Known::Unknown if has_stored_secret(&filters) => return Some(Prepared::Forward),
            Known::Unknown => Map::new(),
        };
        Some(Prepared::Ready { conn, secret_env })
    })
    .await;
    ready.ok().flatten().unwrap_or(Prepared::Forward)
}

/// `connection:executeAction` on an MCP connection: `invokeMcpTool`.
async fn execute(native: Arc<Native>, params: Value) -> Answer {
    let Value::Object(p) = &params else {
        return Answer::Forward;
    };
    let (Some(id), Some(action)) = (
        p.get("connectionId").and_then(Value::as_str),
        p.get("action").and_then(Value::as_str),
    ) else {
        return Answer::Forward;
    };
    let args = match p.get("args") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(args)) => args.clone(),
        Some(_) => return Answer::Forward,
    };
    let (conn, secret_env) = match prepare(&native, id.to_owned()).await {
        Prepared::Forward | Prepared::NotMcp(Some(_)) => return Answer::Forward,
        Prepared::NotMcp(None) => {
            return Answer::Result(
                json!({ "success": false, "error": format!("Connection {id} not found") }),
            )
        }
        Prepared::Ready { conn, secret_env } => (conn, secret_env),
    };
    let filters = filters_of(&conn);
    let peer = match start(&native, id, &filters, &secret_env).await {
        Ok(peer) => peer,
        Err(error) => return Answer::Result(json!({ "success": false, "error": error })),
    };
    Answer::Result(mcp::invoke(&peer, action, &args, &discovered_tools(&conn)).await)
}

/// `connection:refreshMcpTools`: `runMcpDiscovery`.
async fn refresh(native: Arc<Native>, params: Value) -> Answer {
    let Some(id) = params.as_str().map(str::to_owned) else {
        return Answer::Forward;
    };
    let (conn, secret_env) = match prepare(&native, id.clone()).await {
        Prepared::Forward => return Answer::Forward,
        Prepared::NotMcp(_) => {
            return Answer::Result(json!({ "ok": false, "error": "Not an MCP connection" }))
        }
        Prepared::Ready { conn, secret_env } => (conn, secret_env),
    };
    let filters = filters_of(&conn);
    let discovered = match start(&native, &id, &filters, &secret_env).await {
        Ok(peer) => mcp::discover(&peer).await,
        Err(error) => Err(error),
    };
    let n = Arc::clone(&native);
    let written = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = n.store().ok_or("the database is gone")?;
        let answer = match discovered {
            Ok(tools) => {
                let count = tools.len();
                let mut filters = filters;
                filters.insert("discoveredTools".into(), Value::Array(tools));
                let mut updates = Map::new();
                updates.insert("filters".into(), Value::Object(filters));
                updates.insert("lastSyncAt".into(), json!(now_iso()));
                store
                    .db_update_source_connection(&id, &updates, &["lastSyncError".to_owned()])
                    .map_err(|e| e.to_string())?;
                json!({ "ok": true, "count": count })
            }
            Err(error) => {
                let mut updates = Map::new();
                updates.insert("lastSyncError".into(), json!(error));
                store
                    .db_update_source_connection(&id, &updates, &[])
                    .map_err(|e| e.to_string())?;
                json!({ "ok": false, "error": error })
            }
        };
        n.signal_change();
        Ok(answer)
    })
    .await;
    match written {
        Ok(Ok(answer)) => Answer::Result(answer),
        Ok(Err(error)) => Answer::Error(error),
        Err(err) => Answer::Error(err.to_string()),
    }
}

/// The connection's child, started or reused.
async fn start(
    native: &Arc<Native>,
    id: &str,
    filters: &Map<String, Value>,
    secret_env: &Map<String, Value>,
) -> Result<rmcp::service::Peer<rmcp::service::RoleClient>, String> {
    let launch = mcp::Launch::of(filters, secret_env)?;
    native.mcp.client(id, &launch, native.env.get()).await
}

/// A value the store read, with its numbers written as JavaScript writes
/// them: a whole number the store holds as a float (`syncIntervalMinutes`)
/// is `0`, not `0.0`.
fn from_store(mut value: Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Number(n) => {
                if let Some(f) = n.as_f64().filter(|_| n.is_f64()) {
                    if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
                        *v = json!(f as i64);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(walk),
            Value::Object(map) => map.values_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut value);
    value
}

/// One connection row, as the server's `dbGetSourceConnection` returns it.
fn connection(store: &mut Store, id: &str) -> vorn_store::Result<Option<Value>> {
    let conn = from_store(store.call("dbGetSourceConnection", json!([id]))?);
    Ok((!conn.is_null()).then_some(conn))
}

fn connector_of(conn: &Value) -> Option<&str> {
    conn.get("connectorId").and_then(Value::as_str)
}

fn filters_of(conn: &Value) -> Map<String, Value> {
    match conn.get("filters") {
        Some(Value::Object(filters)) => filters.clone(),
        _ => Map::new(),
    }
}

/// `visibleMcpTools`: the tools discovery stored on the row.
fn discovered_tools(conn: &Value) -> Vec<Value> {
    match conn.get("filters").and_then(|f| f.get("discoveredTools")) {
        Some(Value::Array(tools)) => tools.clone(),
        _ => Vec::new(),
    }
}

/// `String(v ?? '')`.
fn filter_string(v: &Value) -> String {
    if v.is_null() {
        String::new()
    } else {
        mcp::js_string(v)
    }
}

/// Whether the row holds an encrypted `secretEnv` the desktop would decrypt.
fn has_stored_secret(filters: &Map<String, Value>) -> bool {
    filters
        .get("secretEnv")
        .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()))
}

/// `new Date().toISOString()`.
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_whole_numbers_as_javascript_does() {
        let row = json!({ "n": 0.0, "m": [2.5, -0.0, 7], "o": { "p": 1e300 } });
        assert_eq!(
            from_store(row).to_string(),
            r#"{"n":0,"m":[2.5,0,7],"o":{"p":1e+300}}"#
        );
    }

    #[test]
    fn writes_times_as_javascript_does() {
        let now = now_iso();
        assert_eq!(now.len(), "2026-10-06T00:00:00.000Z".len(), "{now}");
        assert!(now.ends_with('Z'));
    }

    #[test]
    fn knows_a_stored_secret_and_a_package() {
        let filters = json!({ "secretEnv": "djEw...", "env": "{}" });
        assert!(has_stored_secret(filters.as_object().unwrap()));
        assert!(!has_stored_secret(
            json!({ "secretEnv": "" }).as_object().unwrap()
        ));
        assert_eq!(filter_string(&json!(null)), "");
        assert_eq!(filter_string(&json!(" pkg ")), " pkg ");
    }
}
