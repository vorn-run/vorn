//! The Connections page, drawn as an element tree (`page/tree.rs`):
//! every connection with what it triggers, when it last synced and what its
//! key is; Import now and Sign in when its window is signed out on hover;
//! Test, Rotate key, Roll back and Remove in its menu; and adding one, from
//! picking an installed connector to its settings.
//! The rows come from the built-in's own calls, so the page says what the
//! rest of the app says.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::builtin::{Answer, Connectors};
use crate::connections::{self as conns};

mod tree;

/// The collections the page draws.
pub const CONNECTIONS: &str = "connections";
pub const FORM: &str = "form";
pub const CHOICES: &str = "connectors";
pub const TRIGGERS: &str = "triggers";
pub const PROJECTS: &str = "projects";
pub const SETTINGS: &str = "settings";
pub const FILTERS: &str = "filters";

/// Filters left off a connection's chips: its package tie and an MCP server's tools.
const UNSHOWN: &[&str] = &[
    conns::filter::CONNECTOR_ID,
    conns::filter::VERSION,
    conns::filter::ICON,
    conns::filter::IMPLICIT,
    conns::filter::TRIGGER,
    "discoveredTools",
];
const MASK: &str = "••••••";
const CHIP_CHARS: usize = 60;

/// A connection being added: the connector picked, its manifest as it
/// reports it, and what has been entered so far.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    connector: Option<String>,
    /// What the connector's program reports: its triggers, settings and sign-in.
    manifest: Option<Value>,
    trigger: Option<String>,
    project: Option<String>,
    values: BTreeMap<String, String>,
    error: Option<String>,
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
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

/// "4 min ago", from an ISO time and now in ms.
pub fn ago(iso: &str, now_ms: i64) -> Option<String> {
    let then = chrono::DateTime::parse_from_rfc3339(iso)
        .ok()?
        .timestamp_millis();
    let mins = (now_ms - then).max(0) / 60_000;
    Some(match mins {
        0 => "just now".to_owned(),
        1..=59 => format!("{mins} min ago"),
        60..=1439 => format!("{} h ago", mins / 60),
        _ => format!("{} d ago", mins / 1440),
    })
}

fn draft(c: &Connectors) -> std::sync::MutexGuard<'_, Option<Draft>> {
    c.draft.lock().unwrap_or_else(|e| e.into_inner())
}

/// The page's keys: the field being replaced, by connection, what replacing it
/// last said, and the connections whose last test failed.
#[derive(Debug, Clone, Default)]
pub struct Keys {
    rotating: Option<(String, String)>,
    error: Option<String>,
    failing: BTreeSet<String>,
}

fn keys(c: &Connectors) -> std::sync::MutexGuard<'_, Keys> {
    c.keys.lock().unwrap_or_else(|e| e.into_inner())
}

/// A connection's key line from its `listKeys` entry, and its state: `failing`
/// after a failed test, `locked` when this machine cannot read it, else `ok`.
fn key_line(key: &Value, failing: bool) -> (String, &'static str) {
    if failing {
        return ("Key no longer works · Rotate".to_owned(), "failing");
    }
    let fields = key
        .get("fields")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if fields
        .iter()
        .any(|f| f.get("readable") == Some(&Value::Bool(false)))
    {
        return (
            "Key locked: this machine cannot read it".to_owned(),
            "locked",
        );
    }
    let parts: Vec<String> = fields
        .iter()
        .map(|f| {
            let label = Some(text(f, "label"))
                .filter(|l| !l.is_empty())
                .unwrap_or("Key");
            let shown = match f.get("envNames").and_then(Value::as_array) {
                Some(names) => names
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                None => text(f, "hint").to_owned(),
            };
            if shown.is_empty() {
                format!("{label} saved")
            } else {
                format!("{label} · {shown}")
            }
        })
        .collect();
    (parts.join(" · "), "ok")
}

/// The env a connection asks for: none when nothing is signed in to, and
/// none a borrowed sign-in already covers, as the desktop's form asks.
fn fields(manifest: &Value) -> Vec<Value> {
    let rung = manifest.pointer("/auth/rung").and_then(Value::as_str);
    if rung == Some("none") {
        return Vec::new();
    }
    let borrowed: Vec<String> = if rung == Some("cli") {
        manifest
            .pointer("/auth/borrow/env")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_uppercase)
            .collect()
    } else {
        Vec::new()
    };
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let every = manifest
        .get("env")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            manifest
                .get("triggers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| t.pointer("/setup/env").and_then(Value::as_array))
                .flatten(),
        );
    for e in every {
        let name = text(e, "name");
        let secret = e.get("secret") == Some(&Value::Bool(true));
        if name.is_empty() || seen.iter().any(|s| s == name) {
            continue;
        }
        seen.push(name.to_owned());
        if secret && borrowed.contains(&name.to_uppercase()) {
            continue;
        }
        out.push(e.clone());
    }
    out
}

/// Every row the page shows, by collection.
pub async fn rows(c: &Arc<Connectors>) -> Vec<(&'static str, Vec<Value>)> {
    let now = chrono::Utc::now().timestamp_millis();
    let packs: Vec<Value> = list(ask(c, "connector:listPacks", Value::Null).await)
        .into_iter()
        .filter(|p| text(p, "kind") != "extension")
        .collect();
    let builtins = list(ask(c, "connector:list", Value::Null).await);
    let held_keys = list(ask(c, "connection:listKeys", Value::Null).await);
    let (rotating, key_error, failing) = {
        let k = keys(c);
        (k.rotating.clone(), k.error.clone(), k.failing.clone())
    };
    let listed: Vec<Value> = list(ask(c, "connection:list", json!({})).await)
        .into_iter()
        .filter(|conn| !conns::is_implicit(conn))
        .collect();
    let mut filters = Vec::new();
    for conn in &listed {
        let id = text(conn, "id");
        for (n, chip) in chips(conn).into_iter().enumerate() {
            filters.push(json!({ "id": format!("{id}/{n}"), "parent": id, "text": chip }));
        }
    }
    let connections = listed
        .iter()
        .map(|conn| {
            let id = conns::connector_id_of(conn);
            let pack = packs.iter().find(|p| text(p, "id") == id);
            let service = pack
                .map(|p| text(p, "name"))
                .or_else(|| {
                    builtins
                        .iter()
                        .find(|b| text(b, "id") == id)
                        .map(|b| text(b, "name"))
                })
                .filter(|n| !n.is_empty())
                .unwrap_or(id.as_str())
                .to_owned();
            let trigger = conns::sdk_trigger_of(conn);
            let triggers = pack
                .and_then(|p| p.get("triggers").and_then(Value::as_array))
                .into_iter()
                .flatten()
                .find(|t| text(t, "type") == trigger)
                .map(|t| text(t, "label").to_owned())
                .unwrap_or_default();
            let browser = pack
                .and_then(|p| p.pointer("/auth/rung"))
                .and_then(Value::as_str)
                == Some("browser");
            let needs_signin = browser && text(conn, "signedInAs").is_empty();
            let error = text(conn, "lastSyncError");
            let (state, synced) = if needs_signin {
                ("signin", "not signed in".to_owned())
            } else if !error.is_empty() {
                ("failing", error.to_owned())
            } else {
                match ago(text(conn, "lastSyncAt"), now) {
                    Some(a) => ("ok", format!("synced {a}")),
                    None => ("idle", "not synced yet".to_owned()),
                }
            };
            let line: Vec<&str> = [service.as_str(), triggers.as_str(), synced.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            let previous = pack.map_or("", |p| text(p, "previousVersion"));
            let count = listed
                .iter()
                .filter(|o| conns::connector_id_of(o) == id)
                .count();
            let mut group = vec![if count == 1 {
                "1 connection".to_owned()
            } else {
                format!("{count} connections")
            }];
            if let Some(v) = pack.map(|p| text(p, "version")).filter(|v| !v.is_empty()) {
                group.push(format!("v{v}"));
            }
            let conn_id = text(conn, "id");
            let key = held_keys.iter().find(|k| text(k, "connectionId") == conn_id);
            let (key_line, key_state) = key.map_or((String::new(), ""), |k| {
                key_line(k, failing.contains(conn_id))
            });
            let key_fields: Vec<Value> = key
                .and_then(|k| k.get("fields").and_then(Value::as_array))
                .into_iter()
                .flatten()
                .map(|f| json!({ "key": text(f, "key"), "label": text(f, "label") }))
                .collect();
            let rotating_field = rotating
                .as_ref()
                .filter(|(id, _)| id == conn_id)
                .map(|(_, field)| field.as_str());
            let rotating_label = rotating_field.map_or(String::new(), |field| {
                key_fields
                    .iter()
                    .find(|f| text(f, "key") == field)
                    .map(|f| text(f, "label"))
                    .filter(|l| !l.is_empty())
                    .unwrap_or("key")
                    .to_owned()
            });
            json!({
                "id": conn.get("id"),
                "name": text(conn, "name"),
                "key_line": key_line,
                "key_state": key_state,
                "key_fields": key_fields,
                "rotating": rotating_field.is_some(),
                "rotating_label": rotating_label,
                "rotate_error": if rotating_field.is_some() { key_error.clone().unwrap_or_default() } else { String::new() },
                "connector": id,
                "connector_name": service,
                "connector_line": group.join(" · "),
                "can_add": pack.is_some(),
                "icon": format!("connector:{id}"),
                "line": line.join(" · "),
                "state": state,
                "needs_signin": needs_signin,
                "can_roll_back": !previous.is_empty(),
                "previous": previous,
            })
        })
        .collect();
    let held = draft(c).clone();
    let Some(d) = held else {
        return vec![
            (CONNECTIONS, connections),
            (FILTERS, filters),
            (FORM, Vec::new()),
            (CHOICES, Vec::new()),
            (TRIGGERS, Vec::new()),
            (PROJECTS, Vec::new()),
            (SETTINGS, Vec::new()),
        ];
    };
    let choices = packs
        .iter()
        .map(|p| {
            json!({
                "id": p.get("id"),
                "name": text(p, "name"),
                "selected": d.connector.as_deref() == Some(text(p, "id")),
            })
        })
        .collect();
    let m = d.manifest.clone().unwrap_or(Value::Null);
    let triggers = m
        .get("triggers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| {
            json!({
                "id": text(t, "type"),
                "name": text(t, "label"),
                "selected": d.trigger.as_deref() == Some(text(t, "type")),
            })
        })
        .collect();
    let projects = if d.manifest.is_some() {
        list(c.core("project.list", json!({})).await)
            .iter()
            .map(|p| {
                let name = text(p, "name");
                json!({ "id": name, "name": name, "selected": d.project.as_deref() == Some(name) })
            })
            .collect()
    } else {
        Vec::new()
    };
    let settings: Vec<Value> = fields(&m)
        .iter()
        .map(|e| {
            let name = text(e, "name");
            let required = e.get("required") == Some(&Value::Bool(true));
            let secret = e.get("secret") == Some(&Value::Bool(true));
            let label = if required {
                format!("{name} (required)")
            } else {
                name.to_owned()
            };
            let kept = d.values.get(name).is_some_and(|v| !v.is_empty());
            json!({
                "id": name,
                "label": label,
                "hint": text(e, "description"),
                "secret": secret,
                "value": if secret { "" } else { d.values.get(name).map_or("", String::as_str) },
                "kept": kept,
            })
        })
        .collect();
    let picked = d.manifest.is_some();
    let form = vec![json!({
        "id": "form",
        "picked": picked,
        "can_save": picked && missing(&d).is_none(),
        "none_installed": packs.is_empty(),
        "error": d.error.clone().unwrap_or_default(),
    })];
    vec![
        (CONNECTIONS, connections),
        (FILTERS, filters),
        (FORM, form),
        (CHOICES, choices),
        (TRIGGERS, triggers),
        (PROJECTS, projects),
        (SETTINGS, settings),
    ]
}

/// A connection's filters as "key: value" chips in key order, secrets masked and long values cut.
fn chips(conn: &Value) -> Vec<String> {
    let secret: Vec<String> = conns::password_fields(text(conn, "connectorId"))
        .iter()
        .map(|f| text(f, "key").to_owned())
        .collect();
    let parsed = |v: &Value| {
        v.as_str()
            .and_then(|s| serde_json::from_str::<Map<String, Value>>(s).ok())
    };
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in conns::filters_of(conn) {
        let set = parsed(&value).filter(|_| key == "env" || key == conns::SECRET_ENV);
        let masked = key == conns::SECRET_ENV || secret.contains(&key);
        match set {
            _ if UNSHOWN.contains(&key.as_str()) || blank(&value) => {}
            Some(set) => out.extend(set.into_iter().filter(|(_, v)| !blank(v)).map(|(k, v)| {
                let v = if masked { MASK.to_owned() } else { shown(&v) };
                (k, v)
            })),
            None => {
                let v = if masked {
                    MASK.to_owned()
                } else {
                    shown(&value)
                };
                out.insert(key, v);
            }
        }
    }
    out.into_iter().map(|(k, v)| format!("{k}: {v}")).collect()
}

fn blank(v: &Value) -> bool {
    v.is_null() || v.as_str().is_some_and(str::is_empty)
}

/// A filter's value as a chip says it, cut past 60 characters.
fn shown(v: &Value) -> String {
    let s = match v {
        Value::String(s) if s == conns::IN_VAULT => return MASK.to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > CHIP_CHARS {
        let cut: String = s.chars().take(CHIP_CHARS - 3).collect();
        format!("{cut}…")
    } else {
        s
    }
}

/// The first required setting not yet entered.
fn missing(d: &Draft) -> Option<String> {
    let m = d.manifest.as_ref()?;
    fields(m)
        .iter()
        .filter(|e| e.get("required") == Some(&Value::Bool(true)))
        .map(|e| text(e, "name").to_owned())
        .find(|n| d.values.get(n).is_none_or(|v| v.trim().is_empty()))
}

/// What the connector's program reports about itself, read by starting it.
async fn probe(c: &Arc<Connectors>, id: &str) -> Result<Value, String> {
    let pack = c
        .installed(id)
        .ok_or_else(|| format!("{id} is not installed"))?;
    let argv: Vec<String> = pack
        .pointer("/service/command")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let dir = text(&pack, "path").to_owned();
    let resolve = |a: &String| {
        let p = std::path::Path::new(&dir).join(a);
        if !a.starts_with('-') && std::path::Path::new(a).is_relative() && p.is_file() {
            p.to_string_lossy().into_owned()
        } else {
            a.clone()
        }
    };
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| format!("{id} names no program to run"))?;
    let launch = json!({
        "command": resolve(program),
        "args": args.iter().map(resolve).collect::<Vec<_>>(),
    });
    let probed = ask(c, "connector:probeSdk", launch).await?;
    if probed["ok"] != true {
        return Err(text(&probed, "error").to_owned());
    }
    let mut manifest = probed.get("manifest").cloned().unwrap_or(Value::Null);
    // The installed manifest names the version that runs and how it signs in.
    for key in ["version", "auth", "icon"] {
        if let Some(v) = pack.get(key) {
            manifest[key] = v.clone();
        }
    }
    Ok(manifest)
}

/// The `connection:create` params a finished draft makes, as the desktop's form makes them.
fn create_params(d: &Draft) -> Option<Value> {
    let m = d.manifest.as_ref()?;
    let mut plain = Map::new();
    let mut secret = Map::new();
    for e in fields(m) {
        let name = text(&e, "name");
        let Some(v) = d
            .values
            .get(name)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let into = if e.get("secret") == Some(&Value::Bool(true)) {
            &mut secret
        } else {
            &mut plain
        };
        into.insert(name.to_owned(), json!(v));
    }
    let all = m.get("triggers").and_then(Value::as_array);
    let trigger = all
        .into_iter()
        .flatten()
        .find(|t| Some(text(t, "type")) == d.trigger.as_deref())
        .or_else(|| all.and_then(|t| t.first()));
    let mut filters = json!({
        "env": Value::Object(plain).to_string(),
        "sdkConnectorId": text(m, "id"),
        "sdkVersion": text(m, "version"),
    });
    if let Some(icon) = m.get("icon").filter(|i| !i.is_null()) {
        filters["sdkIcon"] = json!(icon.to_string());
    }
    if let Some(t) = trigger.map(|t| text(t, "type")).filter(|t| !t.is_empty()) {
        filters["sdkTrigger"] = json!(t);
    }
    if !secret.is_empty() {
        filters["secretEnv"] = json!(Value::Object(secret).to_string());
    }
    let name = text(m, "name");
    let mapping: Map<String, Value> = trigger
        .and_then(|t| t.get("statusMapping"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some((
                text(e, "upstream").to_owned(),
                e.get("suggestedLocal")?.clone(),
            ))
        })
        .collect();
    let label = trigger.map(|t| text(t, "label")).filter(|l| !l.is_empty());
    let mut params = json!({
        "connectorId": conns::SDK,
        "name": label.map_or(name.to_owned(), |l| format!("{name}: {l}")),
        "filters": filters,
        "syncIntervalMinutes": 5,
        "statusMapping": mapping,
        "executionProject": d.project.clone().unwrap_or_default(),
    });
    if let Some(w) = trigger
        .and_then(|t| t.get("defaultWorkflow"))
        .filter(|w| !w.is_null())
    {
        params["seedWorkflow"] = w.clone();
    }
    Some(params)
}

/// Picks connector `id` for the connection being added, with what its program reports.
async fn pick(c: &Arc<Connectors>, id: String) -> Result<Value, String> {
    let manifest = probe(c, &id).await;
    let mut held = draft(c);
    let project = held.as_ref().and_then(|d| d.project.clone());
    let mut d = Draft {
        connector: Some(id),
        project,
        ..Draft::default()
    };
    match manifest {
        Ok(m) => {
            d.trigger = m
                .pointer("/triggers/0/type")
                .and_then(Value::as_str)
                .map(str::to_owned);
            d.manifest = Some(m);
        }
        Err(e) => d.error = Some(e),
    }
    *held = Some(d);
    Ok(Value::Null)
}

/// What a control of the page asked: `refresh` reads the rows again; the
/// others act on the row `args.id` names, and answer a toast or the sign-in
/// a window opens on.
pub async fn act(c: &Arc<Connectors>, action: &str, args: &Value) -> Result<Value, String> {
    if action == vorn_tree::RENDER {
        return Ok(tree::render(&rows(c).await));
    }
    if action == vorn_tree::EVENT {
        let event: vorn_tree::Event = serde_json::from_value(args["event"].clone())
            .map_err(|e| format!("a page event reads as {e}"))?;
        let (does, args) = tree::event(&event)?;
        return Box::pin(act(c, does, &args)).await;
    }
    let id = text(args, "id").to_owned();
    let toast = |t: String| Ok(json!({ "toast": t }));
    match action {
        "refresh" => Ok(Value::Null),
        "test" => {
            let r = ask(c, "connection:preflight", json!(id)).await?;
            let said = text(&r, "message");
            let failed = r.get("ok") == Some(&Value::Bool(false));
            if failed {
                keys(c).failing.insert(id.clone());
            } else {
                keys(c).failing.remove(&id);
            }
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
        // What a sign-in action asks: the window's target, `{name, browser}`, which the client opens.
        "signin" => {
            let target = ask(c, "connection:browserAuth", json!(id)).await?;
            if target.is_null() {
                return Err("This connection signs in without a window".to_owned());
            }
            Ok(target)
        }
        "remove" => {
            ask(c, "connection:delete", json!(id)).await?;
            toast("Removed".to_owned())
        }
        "rollback" => {
            let all = ask(c, "connection:list", json!({})).await?;
            let conn = all
                .as_array()
                .into_iter()
                .flatten()
                .find(|x| text(x, "id") == id)
                .cloned()
                .ok_or("That connection is gone")?;
            let pack = conns::connector_id_of(&conn);
            let r = ask(c, "connector:rollbackPack", json!(pack)).await?;
            if r["ok"] != true {
                return Err(text(&r, "error").to_owned());
            }
            let v = r
                .pointer("/pack/version")
                .and_then(Value::as_str)
                .unwrap_or("");
            toast(format!("Back to v{v}"))
        }
        // `id` is `connection|field`: the field whose value is replaced.
        "rotate" => {
            let (conn, field) = id
                .rsplit_once('|')
                .ok_or("Which key to rotate is missing")?;
            let mut k = keys(c);
            k.rotating = Some((conn.to_owned(), field.to_owned()));
            k.error = None;
            Ok(Value::Null)
        }
        "unrotate" => {
            let mut k = keys(c);
            k.rotating = None;
            k.error = None;
            Ok(Value::Null)
        }
        "newkey" => {
            let value = text(args, "value").to_owned();
            let field = keys(c)
                .rotating
                .clone()
                .filter(|(conn, _)| *conn == id)
                .map(|(_, field)| field)
                .ok_or("Nothing is being rotated")?;
            if value.trim().is_empty() {
                return Err("Paste the new key first".to_owned());
            }
            let params = json!({ "connectionId": id, "field": field, "plaintext": value });
            let r = ask(c, "connection:rotateSecret", params).await?;
            if r.get("ok") != Some(&Value::Bool(true)) {
                let e = Some(text(&r, "error"))
                    .filter(|e| !e.is_empty())
                    .unwrap_or("Vorn could not save the new key")
                    .to_owned();
                keys(c).error = Some(e.clone());
                return Err(e);
            }
            let mut k = keys(c);
            k.rotating = None;
            k.error = None;
            k.failing.remove(&id);
            toast("Saved the new key".to_owned())
        }
        "add" => {
            *draft(c) = Some(Draft::default());
            Ok(Value::Null)
        }
        "cancel" => {
            *draft(c) = None;
            Ok(Value::Null)
        }
        "add_to" => {
            *draft(c) = Some(Draft::default());
            pick(c, id).await
        }
        "pick" => pick(c, id).await,
        "trigger" | "project" => {
            let mut held = draft(c);
            let d = held.as_mut().ok_or("Nothing is being added")?;
            if action == "trigger" {
                d.trigger = Some(id);
            } else {
                d.project = Some(id);
            }
            Ok(Value::Null)
        }
        "set" => {
            let value = text(args, "value").to_owned();
            let mut held = draft(c);
            let d = held.as_mut().ok_or("Nothing is being added")?;
            d.values.insert(id, value);
            d.error = None;
            Ok(Value::Null)
        }
        "save" => {
            let held = draft(c).clone().ok_or("Nothing is being added")?;
            if let Some(name) = missing(&held) {
                let e = format!("{name} is required; type it and press Return");
                if let Some(d) = draft(c).as_mut() {
                    d.error = Some(e.clone());
                }
                return Err(e);
            }
            let params = create_params(&held).ok_or("Pick a connector first")?;
            match ask(c, "connection:create", params).await {
                Ok(conn) => {
                    *draft(c) = None;
                    toast(format!("Added {}", text(&conn, "name")))
                }
                Err(e) => {
                    if let Some(d) = draft(c).as_mut() {
                        d.error = Some(e.clone());
                    }
                    Err(e)
                }
            }
        }
        other => Err(format!("Connectors has no action {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_a_connections_filters_as_chips_with_secrets_masked() {
        let long = "x".repeat(70);
        let sdk = json!({ "connectorId": "sdk", "filters": {
            "env": "{\"OWNER\":\"acme\",\"EMPTY\":\"\"}",
            "secretEnv": "{\"TOKEN\":\"vorn-vault\"}",
            "sdkConnectorId": "acme", "sdkVersion": "1.0.0", "sdkIcon": "{}",
            "discoveredTools": ["a"], "repo": long } });
        let cut = format!("repo: {}…", "x".repeat(57));
        assert_eq!(chips(&sdk), ["OWNER: acme", "TOKEN: ••••••", cut.as_str()]);
        let http = json!({ "connectorId": "http", "filters": {
            "baseUrl": "https://api.acme.dev", "secret": "s3cret", "authQuery": "" } });
        assert_eq!(
            chips(&http),
            ["baseUrl: https://api.acme.dev", "secret: ••••••"]
        );
    }

    #[test]
    fn a_key_line_says_what_the_key_is_or_that_it_stopped_working() {
        let token = json!({ "fields": [{ "key": "secret", "label": "Token", "readable": true,
            "hint": "ghp_••••4f2a" }] });
        assert_eq!(
            key_line(&token, false),
            ("Token · ghp_••••4f2a".to_owned(), "ok")
        );
        assert_eq!(
            key_line(&token, true),
            ("Key no longer works · Rotate".to_owned(), "failing")
        );
        let env = json!({ "fields": [{ "key": "secretEnv", "label": "Secrets", "readable": true,
            "envNames": ["GH_TOKEN", "NPM_TOKEN"] }] });
        assert_eq!(key_line(&env, false).0, "Secrets · GH_TOKEN, NPM_TOKEN");
        let locked = json!({ "fields": [{ "key": "secret", "label": "", "readable": false }] });
        assert_eq!(key_line(&locked, false).1, "locked");
        let bare =
            json!({ "fields": [{ "key": "secret", "label": "", "readable": true, "hint": "" }] });
        assert_eq!(key_line(&bare, false).0, "Key saved");
    }

    #[test]
    fn says_how_long_ago() {
        let t = "2026-10-15T10:00:00.000Z";
        let base = chrono::DateTime::parse_from_rfc3339(t)
            .unwrap()
            .timestamp_millis();
        assert_eq!(ago(t, base + 4 * 60_000).as_deref(), Some("4 min ago"));
        assert_eq!(ago(t, base + 3 * 3_600_000).as_deref(), Some("3 h ago"));
        assert_eq!(ago(t, base).as_deref(), Some("just now"));
        assert_eq!(ago("", base), None);
    }

    #[test]
    fn asks_for_what_no_borrow_covers_once_each() {
        let m = json!({ "auth": { "rung": "cli", "borrow": { "env": ["GH_TOKEN"] } },
            "triggers": [
                { "type": "a", "setup": { "env": [{ "name": "GH_TOKEN", "secret": true }, { "name": "OWNER", "required": true }] } },
                { "type": "b", "setup": { "env": [{ "name": "OWNER", "required": true }] } },
            ] });
        let f = fields(&m);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["name"], "OWNER");
        assert!(
            fields(&json!({ "auth": { "rung": "none" }, "env": [{ "name": "X" }] })).is_empty()
        );
    }

    #[test]
    fn a_draft_makes_the_desktops_params() {
        let d = Draft {
            connector: Some("acme".into()),
            manifest: Some(json!({ "id": "acme", "name": "Acme", "version": "1.0.0",
                "auth": { "rung": "key" },
                "triggers": [{ "type": "issues", "label": "Issues",
                    "statusMapping": [{ "upstream": "open", "suggestedLocal": "todo" }],
                    "setup": { "env": [{ "name": "TOKEN", "secret": true, "required": true }, { "name": "HOST" }] } }] })),
            trigger: Some("issues".into()),
            project: Some("app".into()),
            values: [
                ("TOKEN".to_owned(), "t".to_owned()),
                ("HOST".to_owned(), "h".to_owned()),
            ]
            .into(),
            error: None,
        };
        assert_eq!(missing(&d), None);
        let p = create_params(&d).unwrap();
        assert_eq!(p["name"], "Acme: Issues");
        assert_eq!(p["filters"]["env"], "{\"HOST\":\"h\"}");
        assert_eq!(p["filters"]["secretEnv"], "{\"TOKEN\":\"t\"}");
        assert_eq!(p["filters"]["sdkTrigger"], "issues");
        assert_eq!(p["statusMapping"], json!({ "open": "todo" }));
        assert_eq!(p["executionProject"], "app");
        let empty = Draft {
            values: BTreeMap::new(),
            ..d
        };
        assert_eq!(missing(&empty).as_deref(), Some("TOKEN"));
    }
}
