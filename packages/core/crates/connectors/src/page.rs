//! The Connectors settings page, drawn from a template
//! (`views/connectors.kdl`): the rows it shows (connections, installed
//! connectors and the catalog, each saying what it reaches) and what its
//! buttons do. The rows come from the built-in's own calls, so the page
//! says what the rest of the app says.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::builtin::{Answer, Connectors};
use crate::connections::{self as conns};

/// The page's templates, by the path its manifest names them.
pub const VIEWS: &[(&str, &str)] = &[(
    "views/connectors.kdl",
    include_str!("../views/connectors.kdl"),
)];

/// The collections the page draws.
pub const CONNECTIONS: &str = "connections";
pub const INSTALLED: &str = "installed";
pub const CATALOG: &str = "catalog";

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn strings<'a>(v: &'a Value, key: &str) -> Vec<&'a str> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// What a connector reaches, in words: its hosts, then the settings naming more.
pub fn reaches(hosts: &[&str], from: &[&str]) -> String {
    let mut said: Vec<String> = hosts.iter().map(|h| (*h).to_owned()).collect();
    said.extend(from.iter().map(|s| format!("the hosts in {s}")));
    if said.is_empty() {
        "no host it declares".to_owned()
    } else {
        said.join(", ")
    }
}

async fn ask(c: &Arc<Connectors>, method: &str, params: Value) -> Result<Value, String> {
    match c.answer(method, params).await {
        Answer::Result(v) => Ok(v),
        Answer::Void | Answer::Unanswered => Ok(Value::Null),
        Answer::Error(e) => Err(e),
    }
}

fn list(v: Result<Value, String>) -> Vec<Value> {
    v.ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

/// A connection as the page lists it.
fn connection_row(conn: &Value) -> Value {
    let error = text(conn, "lastSyncError");
    let (state, detail) = if !error.is_empty() {
        ("failing", error.to_owned())
    } else if !text(conn, "lastSyncAt").is_empty() {
        ("ok", format!("Last synced {}", text(conn, "lastSyncAt")))
    } else {
        ("idle", "Not synced yet".to_owned())
    };
    json!({
        "id": conn.get("id"),
        "name": text(conn, "name"),
        "connector": conns::connector_id_of(conn),
        "state": state,
        "detail": detail,
    })
}

/// Every row the page shows, by collection.
pub async fn rows(c: &Arc<Connectors>) -> Vec<(&'static str, Vec<Value>)> {
    let connections = list(ask(c, "connection:list", json!({})).await)
        .iter()
        .filter(|conn| !conns::is_implicit(conn))
        .map(connection_row)
        .collect();
    let packs: Vec<Value> = list(ask(c, "connector:listPacks", Value::Null).await)
        .into_iter()
        .filter(|p| text(p, "kind") != "extension")
        .collect();
    let installed = packs
        .iter()
        .map(|p| {
            let previous = text(p, "previousVersion");
            let declares = p.get("declares").cloned().unwrap_or(Value::Null);
            json!({
                "id": p.get("id"),
                "name": text(p, "name"),
                "version": text(p, "version"),
                "reaches": reaches(&strings(&declares, "network"), &strings(&declares, "networkFrom")),
                "can_roll_back": !previous.is_empty(),
                "previous": previous,
            })
        })
        .collect();
    let snapshot = ask(c, "connector:catalog", Value::Null)
        .await
        .unwrap_or(Value::Null);
    let catalog = snapshot
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| {
            let id = text(e, "id");
            let installed = packs.iter().any(|p| text(p, "id") == id);
            let rung = text(e, "authRung");
            let borrows = if rung == "cli" {
                format!("It borrows your own sign-in: {}", text(e, "auth"))
            } else {
                String::new()
            };
            json!({
                "id": id,
                "name": text(e, "name"),
                "description": text(e, "description"),
                "reaches": reaches(&strings(e, "hosts"), &strings(e, "hostsFrom")),
                "borrows": borrows,
                "verified": e.get("verified").is_some_and(|v| !v.is_null()),
                "installed": installed,
                "installable": !installed && !text(e, "git").is_empty(),
            })
        })
        .collect();
    vec![
        (CONNECTIONS, connections),
        (INSTALLED, installed),
        (CATALOG, catalog),
    ]
}

/// What a button of the page asked: `refresh` reads the rows again; the
/// others act on the row `args.id` names and answer a toast.
pub async fn act(c: &Arc<Connectors>, action: &str, args: &Value) -> Result<Value, String> {
    let id = text(args, "id").to_owned();
    let toast = |t: String| Ok(json!({ "toast": t }));
    match action {
        "refresh" => Ok(Value::Null),
        "test" => {
            let r = ask(c, "connection:preflight", json!(id)).await?;
            let said = text(&r, "message");
            match r.get("ok") {
                Some(Value::Bool(true)) => toast(format!("It answers. {said}").trim().to_owned()),
                Some(Value::Bool(false)) => Err(said.to_owned()),
                _ => toast("It has no test of its own".to_owned()),
            }
        }
        "backfill" => {
            let r = ask(c, "connection:backfill", json!({ "connectionId": id })).await?;
            if let Some(e) = r.get("error").and_then(Value::as_str) {
                return Err(e.to_owned());
            }
            toast(format!(
                "Imported {} new and updated {}",
                r["imported"].as_u64().unwrap_or(0),
                r["updated"].as_u64().unwrap_or(0)
            ))
        }
        "delete" => {
            ask(c, "connection:delete", json!(id)).await?;
            toast("Deleted".to_owned())
        }
        "remove" => {
            let r = ask(c, "connector:removePack", json!(id)).await?;
            if r["ok"] != true {
                return Err(text(&r, "error").to_owned());
            }
            toast(format!("Removed {id}"))
        }
        "rollback" => {
            let r = ask(c, "connector:rollbackPack", json!(id)).await?;
            if r["ok"] != true {
                return Err(text(&r, "error").to_owned());
            }
            toast(format!(
                "Back to v{}",
                r.pointer("/pack/version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ))
        }
        "install" => {
            let snapshot = ask(c, "connector:catalog", Value::Null).await?;
            let entry = snapshot
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|e| text(e, "id") == id)
                .cloned()
                .ok_or_else(|| format!("No connector \"{id}\" in the catalog"))?;
            let mut source = json!({ "kind": "git", "url": text(&entry, "git") });
            for key in ["path", "branch"] {
                if !text(&entry, key).is_empty() {
                    source[key] = json!(text(&entry, key));
                }
            }
            let r = ask(c, "connector:installPack", source).await?;
            if r["ok"] != true {
                return Err(text(&r, "error").to_owned());
            }
            toast(format!(
                "Installed {} v{}; add a connection under Plugins › Connections",
                text(&entry, "name"),
                r.pointer("/pack/version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ))
        }
        other => Err(format!("Connectors has no action {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_template_reads_without_an_error() {
        let manifest =
            vorn_extensions::manifest::file::parse_builtin(crate::builtin::MANIFEST).unwrap();
        let load = |path: &str, place| {
            let text = VIEWS
                .iter()
                .find(|(p, _)| *p == path)
                .map_or("", |(_, t)| *t);
            vorn_extensions::views::read(path, text, place)
        };
        let listed = vorn_extensions::views::listed(&manifest, &load);
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert!(
            listed[0]
                .get("errors")
                .is_none_or(|e| e.as_array().is_some_and(Vec::is_empty)),
            "{}",
            listed[0]
        );
        assert!(listed[0]["view"].is_object(), "{}", listed[0]);
    }

    #[test]
    fn says_what_a_connector_reaches() {
        assert_eq!(
            reaches(&["api.x.com"], &["X_HOSTS"]),
            "api.x.com, the hosts in X_HOSTS"
        );
        assert_eq!(reaches(&[], &[]), "no host it declares");
    }

    #[test]
    fn reads_a_connection_state() {
        let row = connection_row(&json!({ "id": "c", "name": "N", "connectorId": "sdk",
            "filters": { "sdkConnectorId": "rss" }, "lastSyncError": "boom" }));
        assert_eq!(row["state"], "failing");
        assert_eq!(row["connector"], "rss");
        assert_eq!(row["detail"], "boom");
        let idle = connection_row(&json!({ "id": "c", "name": "N", "connectorId": "http" }));
        assert_eq!(idle["state"], "idle");
    }
}
