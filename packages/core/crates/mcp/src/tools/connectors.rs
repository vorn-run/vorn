//! `tools/connectors.ts`: discovering, installing and invoking connectors,
//! each a thin call into the server methods the settings screen uses.

use futures_util::future::try_join5;
use serde_json::{json, Map, Value};

use super::{items, object, pretty, Args, Cx, Outcome};
use crate::json;
use crate::rpc::{Rpc, PROBE_TIMEOUT};

/// `SDK_CONNECTOR_ID`.
const SDK_CONNECTOR_ID: &str = "sdk";

/// `failure(message)`.
fn failure(message: impl Into<String>) -> Value {
    super::failed(format!("Error: {}", message.into()))
}

/// `connectionConnectorId(conn)`: the packaged connector's id when a
/// connection runs one, else the connection's own.
pub(crate) fn connection_connector_id(conn: &Value) -> Result<Option<Value>, String> {
    let filters = json::prop(Some(conn), "filters")?;
    match json::field(filters, "sdkConnectorId") {
        Some(Value::String(id)) if !id.is_empty() => Ok(Some(Value::from(id.as_str()))),
        _ => Ok(conn.get("connectorId").cloned()),
    }
}

/// `connectionConnectorId(conn) === id`.
fn belongs_to(conn: &Value, id: Option<&Value>) -> Result<bool, String> {
    Ok(json::strict_equals(
        connection_connector_id(conn)?.as_ref(),
        id,
    ))
}

fn summarize(entry: &Value) -> Value {
    object([
        ("type", entry.get("type").cloned()),
        ("label", entry.get("label").cloned()),
    ])
}

/// "a, b and c".
fn and_list(values: &[&str]) -> String {
    match values {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

fn contribution_summary(contributes: Option<&Value>) -> Value {
    let named = |key: &str| {
        Value::Array(
            items(json::field(contributes, key))
                .iter()
                .map(|e| {
                    object([
                        ("id", e.get("id").cloned()),
                        ("title", e.get("title").cloned()),
                    ])
                })
                .collect(),
        )
    };
    json!({
        "panes": named("panes"),
        "footers": named("footers"),
        "linkHandlers": named("linkHandlers"),
    })
}

/// `a ?? b`.
fn or<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    match a {
        None | Some(Value::Null) => b,
        some => some,
    }
}

pub async fn list_connectors<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let (built_ins, snapshot, connections, statuses, packs) = try_join5(
        cx.call("connector:list", None),
        cx.call("connector:catalog", None),
        cx.call("connection:list", Some(json!({}))),
        cx.call("connector:status", None),
        cx.call("connector:listPacks", None),
    )
    .await?;
    let connections = items(Some(&connections));
    let count_for = |id: Option<&Value>| -> Result<usize, String> {
        let mut n = 0;
        for conn in connections {
            if belongs_to(conn, id)? {
                n += 1;
            }
        }
        Ok(n)
    };
    let status_for = |id: Option<&Value>| {
        items(Some(&statuses))
            .iter()
            .find(|s| json::strict_equals(s.get("connectorId"), id))
    };
    let pack_for = |id: Option<&Value>| {
        items(Some(&packs))
            .iter()
            .find(|p| json::strict_equals(p.get("id"), id))
    };

    let mut entries: Vec<Value> = Vec::new();
    for c in items(Some(&built_ins)) {
        if c.get("addable") == Some(&Value::Bool(false)) {
            continue;
        }
        let id = c.get("id");
        let mut entry = Map::new();
        put(&mut entry, "id", id);
        put(&mut entry, "name", c.get("name"));
        entry.insert("source".into(), json!("built-in"));
        entry.insert("kind".into(), json!("connector"));
        put(&mut entry, "capabilities", c.get("capabilities"));
        entry.insert("connections".into(), json!(count_for(id)?));
        if let Some(status) = status_for(id) {
            put(&mut entry, "authenticated", status.get("authed"));
            if json::truthy(status.get("message")) {
                put(&mut entry, "authMessage", status.get("message"));
            }
        }
        entries.push(Value::Object(entry));
    }
    for e in items(json::prop(Some(&snapshot), "items")?) {
        let id = e.get("id");
        let pack = pack_for(id);
        let kind = or(or(pack.and_then(|p| p.get("kind")), e.get("kind")), None)
            .cloned()
            .unwrap_or_else(|| json!("connector"));
        let mut entry = Map::new();
        put(&mut entry, "id", id);
        put(&mut entry, "name", e.get("name"));
        entry.insert("source".into(), json!("package"));
        entry.insert("kind".into(), kind.clone());
        put(&mut entry, "description", e.get("description"));
        put(&mut entry, "package", e.get("packageName"));
        if json::truthy(e.get("version")) {
            put(&mut entry, "version", e.get("version"));
        }
        put(&mut entry, "capabilities", e.get("capabilities"));
        entry.insert("connections".into(), json!(count_for(id)?));
        if let Some(pack) = pack {
            put(&mut entry, "installed", pack.get("version"));
        }
        if json::truthy(e.get("auth")) {
            put(&mut entry, "auth", e.get("auth"));
        }
        for (key, map) in [("triggers", true), ("actions", true), ("env", false)] {
            if !json::truthy(e.get(key)) {
                continue;
            }
            let listed = items(e.get(key));
            let value = if map {
                Value::Array(listed.iter().map(summarize).collect())
            } else {
                Value::Array(
                    listed
                        .iter()
                        .map(|v| v.get("name").cloned().unwrap_or(Value::Null))
                        .collect(),
                )
            };
            entry.insert(key.into(), value);
        }
        if kind.as_str() == Some("extension") {
            let pick = |key: &str| or(pack.and_then(|p| p.get(key)), e.get(key));
            if let Some(contributes) = pick("contributes").filter(|v| json::truthy(Some(v))) {
                entry.insert(
                    "contributes".into(),
                    contribution_summary(Some(contributes)),
                );
            }
            for key in ["permissions", "activates"] {
                if let Some(v) = pick(key).filter(|v| json::truthy(Some(v))) {
                    entry.insert(key.into(), v.clone());
                }
            }
        }
        entries.push(Value::Object(entry));
    }

    if let Some(kind) = args.nonempty("kind") {
        entries.retain(|e| e.get("kind").and_then(Value::as_str) == Some(kind));
    }
    if args.truthy("installable_only") {
        entries.retain(|e| {
            if e.get("kind").and_then(Value::as_str) == Some("extension") {
                !json::truthy(e.get("installed"))
            } else {
                e.get("connections") == Some(&json!(0))
            }
        });
    }
    Ok(pretty(&Value::Array(entries)))
}

/// Sets `key` when there is a value: an `undefined` property drops out of JSON.
fn put(entry: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(v) = value {
        entry.insert(key.to_owned(), v.clone());
    }
}

/// Connection config minus anything holding a credential.
fn public_filters(conn: &Value) -> Value {
    let filters = or(conn.get("filters"), None);
    let mut out = Map::new();
    if let Some(Value::Object(map)) = filters {
        for (k, v) in map {
            if k != "secretEnv" && k != "discoveredTools" {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

pub async fn list_connections<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let connections = cx.call("connection:list", Some(json!({}))).await?;
    let mut shown = Vec::new();
    for conn in items(Some(&connections)) {
        if let Some(id) = args.get("connector_id").filter(|v| json::truthy(Some(v))) {
            if !belongs_to(conn, Some(id))? {
                continue;
            }
        }
        if args.truthy("failing_only") && !json::truthy(conn.get("lastSyncError")) {
            continue;
        }
        shown.push(object([
            ("id", conn.get("id").cloned()),
            ("name", conn.get("name").cloned()),
            ("connectorId", connection_connector_id(conn)?),
            ("project", conn.get("executionProject").cloned()),
            (
                "syncIntervalMinutes",
                conn.get("syncIntervalMinutes").cloned(),
            ),
            ("lastSyncAt", conn.get("lastSyncAt").cloned()),
            ("lastSyncError", conn.get("lastSyncError").cloned()),
            ("config", Some(public_filters(conn))),
        ]));
    }
    Ok(pretty(&Value::Array(shown)))
}

pub async fn list_connector_actions<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = json::display(args.get("connection_id"));
    let actions = match cx
        .call("connection:listActions", args.get("connection_id").cloned())
        .await
    {
        Ok(actions) => actions,
        Err(err) => {
            return Ok(failure(format!(
                "Could not list actions for connection \"{id}\": {err}"
            )))
        }
    };
    // `actions.length === 0`.
    let empty = match &actions {
        Value::Array(list) => list.is_empty(),
        Value::String(s) => s.is_empty(),
        other => json::prop(Some(other), "length")? == Some(&json!(0)),
    };
    if empty {
        return Ok(failure(format!(
            "No actions for connection \"{id}\". Either the connection does not exist, or its connector exposes no actions yet — for an MCP connection, tool discovery may still be running."
        )));
    }
    Ok(pretty(&actions))
}

/// `parseLaunch(spec)`: a bare package name runs through `npx`; anything with
/// spaces is already a command, which is how a local build is loaded.
fn parse_launch(spec: &str) -> Value {
    // `trim().split(/\s+/)`, on JavaScript's whitespace; a blank spec is one empty part.
    let trimmed = json::trim(spec);
    let mut parts: Vec<&str> = trimmed
        .split(json::is_whitespace)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        parts.push("");
    }
    match parts.as_slice() {
        [one] => json!({ "command": "npx", "args": ["-y", one] }),
        [first, rest @ ..] => json!({ "command": first, "args": rest }),
        [] => unreachable!("an empty spec is one empty part"),
    }
}

/// Reads a connector by starting it. A string is a package name or a
/// command; anything else is a launch spec, sent as it is.
async fn probe<R: Rpc>(cx: &Cx<'_, R>, launch: &Value) -> Result<Value, String> {
    Ok(cx
        .call_for("connector:probeSdk", Some(launch.clone()), PROBE_TIMEOUT)
        .await?)
}

pub async fn inspect_connector_package<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let result = probe(cx, &parse_launch(args.str("package").unwrap_or_default())).await?;
    if !json::truthy(json::prop(Some(&result), "ok")?) {
        return Ok(failure(json::display(result.get("error"))));
    }
    Ok(pretty(result.get("manifest").unwrap_or(&Value::Null)))
}

fn describe_env(entry: &Value) -> String {
    let name = json::display(entry.get("name"));
    if json::truthy(entry.get("description")) {
        format!("{name} ({})", json::display(entry.get("description")))
    } else {
        name
    }
}

/// `list.map((x) => x[key]).join(', ') || '(none)'`.
fn names_or_none(list: &[Value], key: &str) -> String {
    let names: Vec<Value> = list
        .iter()
        .map(|x| x.get(key).cloned().unwrap_or(Value::Null))
        .collect();
    let joined = json::join(&names, ", ");
    if joined.is_empty() {
        "(none)".to_owned()
    } else {
        joined
    }
}

/// Installs the pack a call names, answering the pack or why it was refused.
async fn install_pack<R: Rpc>(
    cx: &Cx<'_, R>,
    params: Value,
) -> Result<Result<Option<Value>, Value>, String> {
    let outcome = cx.call("connector:installPack", Some(params)).await?;
    if !json::truthy(json::prop(Some(&outcome), "ok")?) {
        return Ok(Err(failure(format!(
            "The pack was refused: {}",
            json::display(outcome.get("error"))
        ))));
    }
    Ok(Ok(outcome.get("pack").cloned()))
}

pub async fn install_connector<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let snapshot = cx.call("connector:catalog", None).await?;
    let catalog = items(super::browser::destructure(&snapshot, "items")?);
    let connector_id = args.nonempty("connector_id");
    let entry = connector_id.and_then(|id| {
        catalog
            .iter()
            .find(|c| c.get("id").and_then(Value::as_str) == Some(id))
    });

    if let (Some(id), None) = (connector_id, entry) {
        return Ok(failure(format!(
            "No connector \"{id}\" in the catalog. Known: {}. To install something not in the catalog, pass `package` instead.",
            names_or_none(catalog, "id")
        )));
    }

    // Installed before probing, so the manifest read is the one that will run.
    let mut installed: Option<Value> = None;
    if let Some(pack_path) = args.nonempty("pack_path") {
        match install_pack(cx, json!({ "kind": "file", "path": pack_path })).await? {
            Ok(pack) => installed = pack,
            Err(refused) => return Ok(refused),
        }
    } else if let Some(entry) =
        entry.filter(|e| e.get("kind").and_then(Value::as_str) == Some("extension"))
    {
        if !json::truthy(entry.get("packUrl")) {
            return Ok(failure(format!(
                "{} is in the catalog but no release has published a pack for it yet.",
                json::display(entry.get("name"))
            )));
        }
        let params = object([
            ("kind", Some(json!("url"))),
            ("url", entry.get("packUrl").cloned()),
            (
                "sha256",
                entry
                    .get("sha256")
                    .filter(|v| json::truthy(Some(v)))
                    .cloned(),
            ),
        ]);
        match install_pack(cx, params).await? {
            Ok(pack) => installed = pack,
            Err(refused) => return Ok(refused),
        }
    }
    let installed = installed.filter(|p| json::truthy(Some(p)));

    if let Some(pack) = installed
        .as_ref()
        .filter(|p| p.get("kind").and_then(Value::as_str) == Some("extension"))
    {
        let ignored: Vec<&str> = ["trigger", "sync_interval_minutes", "env", "name", "project"]
            .into_iter()
            .filter(|name| args.get(name).is_some())
            .collect();
        let note = format!(
            "Extensions have no connection: they show on the cards their activation names.{}",
            if ignored.is_empty() {
                String::new()
            } else {
                format!(
                    " Ignored {}, which only a connection uses.",
                    and_list(&ignored)
                )
            }
        );
        return Ok(pretty(&object([
            ("installed", pack.get("name").cloned()),
            ("kind", Some(json!("extension"))),
            ("version", pack.get("version").cloned()),
            ("path", pack.get("path").cloned()),
            (
                "contributes",
                Some(
                    or(pack.get("contributes"), None)
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                ),
            ),
            (
                "permissions",
                Some(
                    or(pack.get("permissions"), None)
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                ),
            ),
            (
                "activates",
                pack.get("activates")
                    .filter(|v| json::truthy(Some(v)))
                    .cloned(),
            ),
            ("note", Some(Value::from(note))),
        ])));
    }

    let launch = match &installed {
        Some(pack) => json!({
            "command": "node",
            "args": [format!("{}/index.js", json::display(pack.get("path")))]
        }),
        None => match or(entry.and_then(|e| e.get("launch")), args.get("package")) {
            Some(Value::String(spec)) if !spec.is_empty() => parse_launch(spec),
            Some(spec) if json::truthy(Some(spec)) => spec.clone(),
            _ => {
                return Ok(failure(
                    "Provide either connector_id, package, or pack_path.",
                ))
            }
        },
    };

    let result = probe(cx, &launch).await?;
    if !json::truthy(json::prop(Some(&result), "ok")?) {
        return Ok(failure(json::display(result.get("error"))));
    }
    let manifest = result.get("manifest");
    let env_defs = items(json::prop(manifest, "env")?);
    let manifest_name = json::display(json::field(manifest, "name"));
    let supplied = match args.get("env") {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    let supplied_value = |e: &Value| {
        e.get("name")
            .and_then(Value::as_str)
            .and_then(|n| supplied.get(n))
    };

    let unknown: Vec<&str> = supplied
        .keys()
        .filter(|name| {
            !env_defs
                .iter()
                .any(|e| e.get("name").and_then(Value::as_str) == Some(name.as_str()))
        })
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Ok(failure(format!(
            "{manifest_name} does not use {}. It accepts: {}.",
            unknown.join(", "),
            names_or_none(env_defs, "name")
        )));
    }

    // Refused rather than stored in the clear: encryption runs in the desktop
    // process, which this one cannot reach.
    let secrets: Vec<Value> = env_defs
        .iter()
        .filter(|e| {
            json::truthy(e.get("secret"))
                && (json::truthy(e.get("required")) || json::truthy(supplied_value(e)))
        })
        .cloned()
        .collect();
    if !secrets.is_empty() {
        return Ok(failure(format!(
            "{manifest_name} uses the secret {} {}, which this tool cannot accept: it runs outside the desktop process, where encryption lives, so it could only store them unprotected. They must be entered by a person in Settings > Connectors to reach the OS keychain. Everything else about the connector is ready to install.",
            if secrets.len() == 1 { "value" } else { "values" },
            json::join(
                &secrets.iter().map(|e| e.get("name").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
                ", "
            )
        )));
    }

    let missing: Vec<String> = env_defs
        .iter()
        .filter(|e| {
            json::truthy(e.get("required"))
                && !supplied_value(e)
                    .and_then(Value::as_str)
                    .is_some_and(|v| !json::trim(v).is_empty())
        })
        .map(describe_env)
        .collect();
    if !missing.is_empty() {
        return Ok(failure(format!(
            "{manifest_name} needs {}. Pass them in `env`.",
            missing.join(", ")
        )));
    }

    let triggers = items(json::field(manifest, "triggers"));
    let wanted = args.nonempty("trigger");
    let trigger = match wanted {
        Some(t) => triggers
            .iter()
            .find(|x| x.get("type").and_then(Value::as_str) == Some(t)),
        None => triggers.first(),
    };
    if let (Some(t), None) = (wanted, trigger) {
        return Ok(failure(format!(
            "{manifest_name} has no trigger \"{t}\". It offers: {}.",
            names_or_none(triggers, "type")
        )));
    }
    let trigger = trigger.filter(|t| json::truthy(Some(t)));
    let trigger_type = trigger.and_then(|t| t.get("type").cloned());

    let mut filters = Map::new();
    if let Some(command) = launch.get("command") {
        filters.insert("command".into(), command.clone());
    }
    if let Some(launch_args) = launch.get("args") {
        filters.insert("args".into(), Value::from(json::stringify(launch_args)));
    }
    filters.insert(
        "env".into(),
        Value::from(json::stringify(&Value::Object(supplied.clone()))),
    );
    // sdkConnectionFilters(manifest, trigger?.type).
    for (key, from) in [("sdkConnectorId", "id"), ("sdkVersion", "version")] {
        if let Some(v) = json::field(manifest, from) {
            filters.insert(key.into(), v.clone());
        }
    }
    if let Some(icon) = json::field(manifest, "icon").filter(|v| json::truthy(Some(v))) {
        filters.insert("sdkIcon".into(), Value::from(json::stringify(icon)));
    }
    if let Some(t) = trigger_type.as_ref().filter(|v| json::truthy(Some(v))) {
        filters.insert("sdkTrigger".into(), t.clone());
    }

    let name = match args.get("name") {
        Some(name) => Some(name.clone()),
        None => match trigger {
            Some(t) => Some(Value::from(format!(
                "{manifest_name}: {}",
                json::display(t.get("label"))
            ))),
            None => json::field(manifest, "name").cloned(),
        },
    };
    let connection = cx
        .call(
            "connection:create",
            Some(object([
                ("connectorId", Some(json!(SDK_CONNECTOR_ID))),
                ("name", name),
                ("filters", Some(Value::Object(filters))),
                (
                    "syncIntervalMinutes",
                    Some(
                        args.get("sync_interval_minutes")
                            .cloned()
                            .unwrap_or_else(|| json!(5)),
                    ),
                ),
                ("statusMapping", Some(json!({}))),
                (
                    "executionProject",
                    args.nonempty("project").map(Value::from),
                ),
            ])),
        )
        .await?;
    let connection_id = json::prop(Some(&connection), "id")?.cloned();

    let mut answer = vec![
        ("installed", json::field(manifest, "name").cloned()),
        ("connectionId", connection_id),
        ("trigger", trigger_type),
    ];
    if let Some(pack) = &installed {
        answer.push(("version", pack.get("version").cloned()));
        answer.push(("path", pack.get("path").cloned()));
    }
    answer.push((
        "note",
        Some(json!(
            "Poll it now with backfill_connection, or reference it from a workflow."
        )),
    ));
    Ok(pretty(&object(answer)))
}
pub async fn run_connector_action<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let result = cx
        .call(
            "connection:executeAction",
            Some(object([
                ("connectionId", args.get("connection_id").cloned()),
                ("action", args.get("action").cloned()),
                (
                    "args",
                    Some(args.get("args").cloned().unwrap_or_else(|| json!({}))),
                ),
            ])),
        )
        .await?;
    if !json::truthy(json::prop(Some(&result), "success")?) {
        let error = or(result.get("error"), None)
            .map_or_else(|| "Action failed".to_owned(), |e| json::display(Some(e)));
        return Ok(failure(error));
    }
    Ok(pretty(&result))
}

pub async fn backfill_connection<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let result = cx
        .call_for(
            "connection:backfill",
            Some(object([(
                "connectionId",
                args.get("connection_id").cloned(),
            )])),
            PROBE_TIMEOUT,
        )
        .await?;
    if let Some(error) = json::prop(Some(&result), "error")?.filter(|e| json::truthy(Some(e))) {
        return Ok(failure(json::display(Some(error))));
    }
    Ok(pretty(&result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_runs_through_npx() {
        assert_eq!(
            parse_launch("  @acme/connector "),
            json!({ "command": "npx", "args": ["-y", "@acme/connector"] })
        );
        assert_eq!(
            parse_launch("node /x/dist/index.js --flag"),
            json!({ "command": "node", "args": ["/x/dist/index.js", "--flag"] })
        );
        assert_eq!(
            parse_launch(" "),
            json!({ "command": "npx", "args": ["-y", ""] })
        );
    }

    #[test]
    fn lists_read_aloud() {
        assert_eq!(and_list(&[]), "");
        assert_eq!(and_list(&["a"]), "a");
        assert_eq!(and_list(&["a", "b", "c"]), "a, b and c");
    }

    #[test]
    fn a_packaged_connection_belongs_to_its_package() {
        let conn = json!({ "connectorId": "sdk", "filters": { "sdkConnectorId": "linear" } });
        assert_eq!(
            connection_connector_id(&conn).unwrap(),
            Some(json!("linear"))
        );
        let conn = json!({ "connectorId": "github", "filters": { "sdkConnectorId": "" } });
        assert_eq!(
            connection_connector_id(&conn).unwrap(),
            Some(json!("github"))
        );
    }
}
