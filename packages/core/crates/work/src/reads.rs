//! The work model's read calls, answered from `vorn.db` as the server's
//! handlers answer them: workflows, their runs, the schedule log, the next
//! run of a schedule, the webhook address, and artifacts with the URLs that
//! open them.

use std::path::Path;

use jiff::tz::TimeZone;
use serde_json::{json, Map, Value};
use vorn_store::Store;

use crate::trigger::{js_date, Trigger};
use crate::{is_truthy, js_numbers};

/// The calls [`read`] answers.
pub const METHODS: &[&str] = &[
    "workflow:list",
    "workflow:get",
    "workflowRun:list",
    "workflowRun:listByTask",
    "workflowRun:listWaiting",
    "workflowRun:listRunning",
    "workflowRun:listAll",
    "scheduler:getLog",
    "scheduler:getNextRun",
    "webhook:info",
    "artifact:list",
    "artifact:versionUrl",
    "artifact:forGate",
    "artifact:readSource",
];

/// What the server's process knows that the database does not.
#[derive(Clone, Copy, Debug)]
pub struct Host<'a> {
    /// The data directory, where artifact bodies are kept.
    pub data_dir: &'a Path,
    /// The port the server listens on, which its URLs name.
    pub server_port: Option<u16>,
    /// Unix milliseconds now.
    pub now_ms: i64,
    /// The zone a date-time without an offset is read in.
    pub zone: &'a TimeZone,
}

/// What a read came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Value(Value),
    /// The handler returned `undefined`: the frame has no `result`.
    Void,
    /// Only the server can answer: the call needs its sessions or caches,
    /// its handler would throw, or the store refused the call.
    Server,
}

/// Answers `method` with `params` from `store`.
pub fn read(store: &mut Store, host: &Host<'_>, method: &str, params: &Value) -> Reply {
    reply(store, host, method, params).unwrap_or(Reply::Server)
}

fn reply(store: &mut Store, host: &Host<'_>, method: &str, params: &Value) -> Option<Reply> {
    let value = |v: Value| Some(Reply::Value(v));
    match method {
        "workflow:list" => value(call(store, "dbListWorkflows", json!([]))?),
        "workflow:get" => match destructure(params)?.get("id") {
            Some(Value::String(id)) => value(call(store, "dbGetWorkflow", json!([id]))?),
            _ => value(Value::Null),
        },
        "workflowRun:list" => runs(store, "listWorkflowRuns", params, "workflowId"),
        "workflowRun:listByTask" => runs(store, "listWorkflowRunsByTask", params, "taskId"),
        "workflowRun:listAll" => runs(store, "listAllWorkflowRuns", params, "workspaceId"),
        "workflowRun:listWaiting" => {
            without_definitions(call(store, "listRunsWithWaitingGates", json!([null]))?)
        }
        "workflowRun:listRunning" => {
            without_definitions(call(store, "listRunningRuns", json!([]))?)
        }
        // The server logs a failed read and answers an empty log.
        "scheduler:getLog" => value(
            call(store, "getScheduleLogEntries", json!([params])).unwrap_or_else(|| json!([])),
        ),
        "scheduler:getNextRun" => next_run(store, host, params),
        "webhook:info" => value(json!({ "baseUrl": loopback(host, "")? })),
        "artifact:list" => {
            let params = destructure(params)?;
            if is_truthy(params.get("sessionId")) {
                return None;
            }
            let mut filter = Map::new();
            if let Some(project) = params.get("projectName") {
                filter.insert("projectName".into(), project.clone());
            }
            let limit = params.get("limit").cloned().unwrap_or(Value::Null);
            value(call(store, "listArtifacts", json!([filter, limit]))?)
        }
        "artifact:versionUrl" => version_url(store, host, destructure(params)?),
        "artifact:forGate" => for_gate(store, host, destructure(params)?),
        "artifact:readSource" => read_source(store, host, destructure(params)?),
        _ => None,
    }
}

/// `({ a, b }) =>` reads fields of anything but `null` and `undefined`,
/// which it throws on.
fn destructure(params: &Value) -> Option<&Value> {
    (!params.is_null()).then_some(params)
}

/// A store call's answer, numbers as the server prints them; `None` when
/// the store refuses or fails it.
fn call(store: &mut Store, name: &str, args: Value) -> Option<Value> {
    store.call(name, args).ok().map(js_numbers)
}

/// `list(params[key], params.limit).map(withoutDefinition)`.
fn runs(store: &mut Store, name: &str, params: &Value, key: &str) -> Option<Reply> {
    let params = destructure(params)?;
    let arg = |k: &str| params.get(k).cloned().unwrap_or(Value::Null);
    without_definitions(call(store, name, json!([arg(key), arg("limit")]))?)
}

fn without_definitions(runs: Value) -> Option<Reply> {
    let Value::Array(mut runs) = runs else {
        return None;
    };
    for run in &mut runs {
        if let Value::Object(fields) = run {
            fields.shift_remove("definition");
        }
    }
    Some(Reply::Value(Value::Array(runs)))
}

/// `scheduler.getNextRun`: a `once` trigger's `runAt` while it is ahead,
/// a cron trigger's expression, or `null`.
fn next_run(store: &mut Store, host: &Host<'_>, params: &Value) -> Option<Reply> {
    let null = Some(Reply::Value(Value::Null));
    let Value::String(id) = params else {
        return null;
    };
    let workflow = call(store, "dbGetWorkflow", json!([id]))?;
    if workflow.is_null() || !is_truthy(workflow.get("enabled")) {
        return null;
    }
    match Trigger::of(&workflow) {
        Trigger::Unreadable => None,
        Trigger::Missing | Trigger::Other => null,
        Trigger::Cron { cron: None, .. } => Some(Reply::Void),
        Trigger::Cron {
            cron: Some(cron), ..
        } => Some(Reply::Value(cron.clone())),
        // `new Date(undefined)` and `new Date(null)` are never ahead.
        Trigger::Once {
            run_at: None | Some(Value::Null),
        } => null,
        Trigger::Once {
            run_at: Some(Value::String(text)),
        } => {
            let ahead = js_date(text, host.zone)? > host.now_ms;
            Some(Reply::Value(if ahead { json!(text) } else { Value::Null }))
        }
        Trigger::Once { .. } => None,
    }
}

/// `http://127.0.0.1:<port><path>`, the address the server's URLs use.
fn loopback(host: &Host<'_>, path: &str) -> Option<String> {
    Some(format!("http://127.0.0.1:{}{path}", host.server_port?))
}

/// `/artifact/<id>/<version>?t=<token>`, the route that serves a version.
pub fn artifact_path(artifact_id: &str, version: i64, token: &str) -> String {
    format!(
        "/artifact/{}/{version}?t={}",
        encode_uri_component(artifact_id),
        encode_uri_component(token)
    )
}

/// JavaScript's `encodeURIComponent`.
fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn string_field<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key)?.as_str()
}

/// The artifact and its token, or `None` when the store fails the read.
fn artifact_and_token(store: &mut Store, id: &str) -> Option<(Value, Value)> {
    let artifact = call(store, "getArtifact", json!([id]))?;
    let token = call(store, "getArtifactToken", json!([id]))?;
    Some((artifact, token))
}

fn latest_version(artifact: &Value) -> Option<i64> {
    artifact.get("latestVersion")?.as_i64()
}

fn version_url(store: &mut Store, host: &Host<'_>, params: &Value) -> Option<Reply> {
    let id = string_field(params, "artifactId")?;
    let (artifact, token) = artifact_and_token(store, id)?;
    let latest = if artifact.is_null() {
        None
    } else {
        Some(latest_version(&artifact)?)
    };
    // A version JavaScript would compare or print other than as an integer is the server's.
    let n = match params.get("version") {
        None | Some(Value::Null) => latest.unwrap_or(0),
        Some(v) => v.as_i64()?,
    };
    let (Some(latest), Value::String(token)) = (latest, &token) else {
        return Some(Reply::Value(Value::Null));
    };
    if token.is_empty() || n < 1 || n > latest {
        return Some(Reply::Value(Value::Null));
    }
    let path = artifact_path(id, n, token);
    let url = loopback(host, &path)?;
    Some(Reply::Value(json!({ "path": path, "url": url })))
}

fn for_gate(store: &mut Store, host: &Host<'_>, params: &Value) -> Option<Reply> {
    let run = string_field(params, "runId")?;
    let node = string_field(params, "nodeId")?;
    let artifact = call(store, "findGateArtifact", json!([run, node]))?;
    if artifact.is_null() {
        return Some(Reply::Value(Value::Null));
    }
    let id = artifact.get("id")?.as_str()?.to_owned();
    let token = call(store, "getArtifactToken", json!([id]))?;
    let n = latest_version(&artifact)?;
    let token = match token {
        Value::String(t) if !t.is_empty() && n >= 1 => t,
        _ => return Some(Reply::Value(Value::Null)),
    };
    let url = loopback(host, &artifact_path(&id, n, &token))?;
    Some(Reply::Value(
        json!({ "artifact": artifact, "version": n, "url": url }),
    ))
}

/// `readArtifactSource`: a version's row and the body kept for it.
fn read_source(store: &mut Store, host: &Host<'_>, params: &Value) -> Option<Reply> {
    if is_truthy(params.get("sessionId")) {
        return None;
    }
    let id = string_field(params, "artifactId")?;
    let artifact = call(store, "getArtifact", json!([id]))?;
    if artifact.is_null() {
        return Some(Reply::Value(Value::Null));
    }
    let n = match params.get("version") {
        None | Some(Value::Null) => artifact.get("latestVersion")?.as_f64()?,
        Some(Value::Number(v)) => v.as_f64()?,
        // `===` never matches a version row's number.
        Some(_) => return Some(Reply::Value(Value::Null)),
    };
    let versions = call(store, "listArtifactVersions", json!([id]))?;
    let found = versions
        .as_array()?
        .iter()
        .find(|v| v.get("version").and_then(Value::as_f64) == Some(n));
    let Some(found) = found else {
        return Some(Reply::Value(Value::Null));
    };
    let extension = if artifact.get("kind").and_then(Value::as_str) == Some("doc") {
        "md"
    } else {
        "html"
    };
    let file = host
        .data_dir
        .join("artifacts")
        .join(segment(id))
        .join(format!("{n}.{extension}"));
    let body = match std::fs::read(file) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => return Some(Reply::Value(Value::Null)),
    };
    Some(Reply::Value(json!({ "version": found, "body": body })))
}

/// An artifact id as a directory name: each UTF-16 unit but
/// `[A-Za-z0-9._-]` is `_`, as the server's regex replaces it.
fn segment(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else {
            out.extend(std::iter::repeat_n('_', c.len_utf16()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use vorn_store::StoreOptions;

    struct Fixture {
        _dir: tempfile::TempDir,
        data: PathBuf,
        store: Store,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().to_owned();
        let options = StoreOptions {
            default_shell: String::new(),
            default_agent_commands: Map::new(),
            default_workspace: serde_json::from_value(json!({
                "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#6b7280", "order": 0
            }))
            .unwrap(),
            owner_name: "owner".into(),
            seed_workflows: Vec::new(),
        };
        let store = Store::open(&data.join("vorn.db"), options).unwrap().0;
        Fixture {
            _dir: dir,
            data,
            store,
        }
    }

    const NOW: i64 = 1_900_000_000_000;

    fn ask(f: &mut Fixture, method: &str, params: Value) -> Reply {
        let zone = TimeZone::UTC;
        let host = Host {
            data_dir: &f.data,
            server_port: Some(4123),
            now_ms: NOW,
            zone: &zone,
        };
        read(&mut f.store, &host, method, &params)
    }

    fn value(reply: Reply) -> Value {
        match reply {
            Reply::Value(v) => v,
            other => panic!("expected a value, got {other:?}"),
        }
    }

    fn add_workflow(f: &mut Fixture, id: &str, enabled: bool, trigger: Value) {
        let wf = json!({
            "id": id, "name": id, "icon": "Zap", "iconColor": "#fff", "enabled": enabled, "edges": [],
            "nodes": [{ "id": "t", "type": "trigger", "label": "T", "position": { "x": 0, "y": 0 }, "config": trigger }]
        });
        f.store.call("dbInsertWorkflow", json!([wf])).unwrap();
    }

    fn add_artifact(f: &mut Fixture, kind: &str, gate: Option<(&str, &str)>) -> (String, String) {
        let mut fields =
            json!({ "kind": kind, "title": "T", "sessionId": null, "projectName": "p" });
        if let Some((run, node)) = gate {
            fields["gateRunId"] = json!(run);
            fields["gateNodeId"] = json!(node);
        }
        let out = f.store.call("insertArtifact", json!([fields])).unwrap();
        let id = out["artifact"]["id"].as_str().unwrap().to_owned();
        let token = out["token"].as_str().unwrap().to_owned();
        (id, token)
    }

    #[test]
    fn reads_workflows_and_their_next_run() {
        let mut f = fixture();
        add_workflow(
            &mut f,
            "cron",
            true,
            json!({ "triggerType": "recurring", "cron": "0 9 * * *" }),
        );
        add_workflow(
            &mut f,
            "off",
            false,
            json!({ "triggerType": "recurring", "cron": "* * * * *" }),
        );
        add_workflow(
            &mut f,
            "ahead",
            true,
            json!({ "triggerType": "once", "runAt": "2031-01-01T00:00:00Z" }),
        );
        add_workflow(
            &mut f,
            "past",
            true,
            json!({ "triggerType": "once", "runAt": "2020-01-01T00:00:00Z" }),
        );
        add_workflow(
            &mut f,
            "words",
            true,
            json!({ "triggerType": "once", "runAt": "next tuesday" }),
        );
        add_workflow(
            &mut f,
            "no-cron",
            true,
            json!({ "triggerType": "recurring" }),
        );
        add_workflow(&mut f, "manual", true, json!({ "triggerType": "manual" }));

        let list = value(ask(&mut f, "workflow:list", Value::Null));
        assert_eq!(list.as_array().unwrap().len(), 7);
        let got = value(ask(&mut f, "workflow:get", json!({ "id": "cron" })));
        assert_eq!(got["id"], "cron");
        assert_eq!(
            value(ask(&mut f, "workflow:get", json!({ "id": "nope" }))),
            Value::Null
        );
        assert_eq!(
            value(ask(&mut f, "workflow:get", json!({ "id": 3 }))),
            Value::Null
        );
        assert_eq!(ask(&mut f, "workflow:get", Value::Null), Reply::Server);

        let next = |f: &mut Fixture, id: &str| ask(f, "scheduler:getNextRun", json!(id));
        assert_eq!(next(&mut f, "cron"), Reply::Value(json!("0 9 * * *")));
        assert_eq!(next(&mut f, "off"), Reply::Value(Value::Null));
        assert_eq!(
            next(&mut f, "ahead"),
            Reply::Value(json!("2031-01-01T00:00:00Z"))
        );
        assert_eq!(next(&mut f, "past"), Reply::Value(Value::Null));
        assert_eq!(next(&mut f, "words"), Reply::Server);
        assert_eq!(next(&mut f, "no-cron"), Reply::Void);
        assert_eq!(next(&mut f, "manual"), Reply::Value(Value::Null));
        assert_eq!(next(&mut f, "missing"), Reply::Value(Value::Null));
    }

    #[test]
    fn lists_runs_without_their_definitions() {
        let mut f = fixture();
        add_workflow(&mut f, "wf", true, json!({ "triggerType": "manual" }));
        let run = json!({
            "runId": "r1", "workflowId": "wf", "startedAt": "2030-01-01T00:00:00.000Z", "status": "running",
            "nodeStates": [], "definition": { "id": "wf" }, "triggerTaskId": "t1"
        });
        f.store.call("saveWorkflowRun", json!([run])).unwrap();
        for (method, params) in [
            ("workflowRun:list", json!({ "workflowId": "wf" })),
            (
                "workflowRun:listByTask",
                json!({ "taskId": "t1", "limit": 5 }),
            ),
            ("workflowRun:listAll", json!({})),
            ("workflowRun:listRunning", Value::Null),
        ] {
            let runs = value(ask(&mut f, method, params));
            let runs = runs.as_array().unwrap();
            assert_eq!(runs.len(), 1, "{method}");
            assert_eq!(runs[0]["runId"], "r1");
            assert!(runs[0].get("definition").is_none(), "{method}");
        }
        assert_eq!(
            value(ask(&mut f, "workflowRun:listWaiting", Value::Null)),
            json!([])
        );
        assert_eq!(ask(&mut f, "workflowRun:list", Value::Null), Reply::Server);
        assert_eq!(
            ask(&mut f, "workflowRun:list", json!({ "workflowId": 7 })),
            Reply::Server
        );
    }

    #[test]
    fn reads_the_schedule_log_and_the_webhook_address() {
        let mut f = fixture();
        let entry = json!({ "workflowId": "wf", "workflowName": "W", "executedAt": "y", "status": "executed", "sessionsLaunched": 2 });
        f.store.call("addScheduleLogEntry", json!([entry])).unwrap();
        let log = value(ask(&mut f, "scheduler:getLog", json!("wf")));
        assert_eq!(log[0]["sessionsLaunched"], json!(2));
        assert!(log[0]["sessionsLaunched"].is_i64());
        assert_eq!(
            value(ask(&mut f, "scheduler:getLog", json!("other"))),
            json!([])
        );
        assert_eq!(
            value(ask(&mut f, "scheduler:getLog", Value::Null))
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(value(ask(&mut f, "scheduler:getLog", json!({}))), json!([]));
        assert_eq!(
            value(ask(&mut f, "webhook:info", Value::Null)),
            json!({ "baseUrl": "http://127.0.0.1:4123" })
        );
    }

    #[test]
    fn artifact_urls_name_the_version_and_token() {
        let mut f = fixture();
        let (id, token) = add_artifact(&mut f, "doc", Some(("run", "gate")));
        assert_eq!(
            value(ask(
                &mut f,
                "artifact:versionUrl",
                json!({ "artifactId": id })
            )),
            Value::Null
        );
        f.store
            .call("addArtifactVersion", json!([id, "agent", null]))
            .unwrap();
        f.store
            .call("addArtifactVersion", json!([id, "agent", null]))
            .unwrap();

        let path = format!("/artifact/{id}/2?t={token}");
        assert_eq!(
            value(ask(
                &mut f,
                "artifact:versionUrl",
                json!({ "artifactId": id })
            )),
            json!({ "path": path, "url": format!("http://127.0.0.1:4123{path}") })
        );
        let one = value(ask(
            &mut f,
            "artifact:versionUrl",
            json!({ "artifactId": id, "version": 1 }),
        ));
        assert_eq!(one["path"], format!("/artifact/{id}/1?t={token}"));
        for version in [json!(0), json!(3)] {
            let got = ask(
                &mut f,
                "artifact:versionUrl",
                json!({ "artifactId": id, "version": version }),
            );
            assert_eq!(got, Reply::Value(Value::Null));
        }
        assert_eq!(
            ask(
                &mut f,
                "artifact:versionUrl",
                json!({ "artifactId": id, "version": "1" })
            ),
            Reply::Server
        );
        assert_eq!(
            value(ask(
                &mut f,
                "artifact:versionUrl",
                json!({ "artifactId": "nope" })
            )),
            Value::Null
        );

        let gate = value(ask(
            &mut f,
            "artifact:forGate",
            json!({ "runId": "run", "nodeId": "gate" }),
        ));
        assert_eq!(gate["version"], 2);
        assert_eq!(gate["artifact"]["id"], json!(id));
        assert_eq!(gate["url"], format!("http://127.0.0.1:4123{path}"));
        assert_eq!(
            value(ask(
                &mut f,
                "artifact:forGate",
                json!({ "runId": "run", "nodeId": "x" })
            )),
            Value::Null
        );

        let listed = value(ask(&mut f, "artifact:list", json!({ "projectName": "p" })));
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(
            value(ask(&mut f, "artifact:list", json!({ "projectName": "q" }))),
            json!([])
        );
        assert_eq!(
            ask(&mut f, "artifact:list", json!({ "sessionId": "s" })),
            Reply::Server
        );
    }

    #[test]
    fn reads_a_versions_source_from_the_data_directory() {
        let mut f = fixture();
        let (id, _) = add_artifact(&mut f, "doc", None);
        f.store
            .call("addArtifactVersion", json!([id, "agent", null]))
            .unwrap();
        f.store
            .call("addArtifactVersion", json!([id, "agent", null]))
            .unwrap();
        let dir = f.data.join("artifacts").join(segment(&id));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.md"), "# one").unwrap();
        std::fs::write(dir.join("2.md"), b"# two \xff").unwrap();

        let latest = value(ask(
            &mut f,
            "artifact:readSource",
            json!({ "artifactId": id }),
        ));
        assert_eq!(latest["body"], "# two \u{fffd}");
        assert_eq!(latest["version"]["version"], 2);
        let first = value(ask(
            &mut f,
            "artifact:readSource",
            json!({ "artifactId": id, "version": 1 }),
        ));
        assert_eq!(first["body"], "# one");
        for version in [json!(3), json!("1")] {
            let got = ask(
                &mut f,
                "artifact:readSource",
                json!({ "artifactId": id, "version": version }),
            );
            assert_eq!(got, Reply::Value(Value::Null));
        }
        std::fs::remove_file(dir.join("1.md")).unwrap();
        let gone = ask(
            &mut f,
            "artifact:readSource",
            json!({ "artifactId": id, "version": 1 }),
        );
        assert_eq!(gone, Reply::Value(Value::Null));
        assert_eq!(
            ask(
                &mut f,
                "artifact:readSource",
                json!({ "sessionId": "s", "artifactId": id })
            ),
            Reply::Server
        );
    }

    #[test]
    fn encodes_as_encode_uri_component() {
        assert_eq!(
            encode_uri_component("a-b_c.d!e~f*g'h(i)j"),
            "a-b_c.d!e~f*g'h(i)j"
        );
        assert_eq!(encode_uri_component("a b/c?é"), "a%20b%2Fc%3F%C3%A9");
        assert_eq!(segment("a/b é.c😀"), "a_b__.c__");
    }
}
