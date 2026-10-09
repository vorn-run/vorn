//! Every store call replayed against the recorded reference (`tests/fixtures/js-reference/store.json`).

mod common;

use std::collections::HashMap;

use common::{call, options_with, save_config, update_connection, update_task};
use serde_json::{json, Map, Value};
use vorn_store::Store;

const REFERENCE: &str = include_str!("../../../../../tests/fixtures/js-reference/store.json");

const HOUR_MS: i64 = 60 * 60 * 1000;

fn now_ms() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is past 1970");
    i64::try_from(since.as_millis()).expect("milliseconds fit")
}

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` in lowercase hex.
fn is_uuid(s: &[u8]) -> bool {
    s.len() == 36
        && s.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => matches!(b, b'0'..=b'9' | b'a'..=b'f'),
        })
}

fn is_token(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Milliseconds since the epoch of `YYYY-MM-DDTHH:MM:SS.mmmZ`, or `None` for anything else.
fn iso_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let shape = b.len() == 24
        && b.iter().enumerate().all(|(i, &c)| match i {
            4 | 7 => c == b'-',
            10 => c == b'T',
            13 | 16 => c == b':',
            19 => c == b'.',
            23 => c == b'Z',
            _ => c.is_ascii_digit(),
        });
    if !shape {
        return None;
    }
    let n = |range: std::ops::Range<usize>| -> i64 { s[range].parse().expect("digits") };
    let (y, m, d) = (n(0..4), n(5..7), n(8..10));
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + n(11..13)) * 60 + n(14..16)) * 60_000 + n(17..19) * 1000 + n(20..23))
}

/// `normalizeStoreOutput`: a copy of `value` with what the store made up replaced by placeholders.
fn normalize(value: &Value, now: i64) -> Value {
    struct Names(HashMap<String, String>);
    impl Names {
        fn name(&mut self, kind: &str, raw: &str) -> String {
            let next = self.0.len() + 1;
            self.0
                .entry(raw.to_owned())
                .or_insert_with(|| format!("<{kind} {next}>"))
                .clone()
        }
    }
    fn walk(v: &Value, names: &mut Names, now: i64) -> Value {
        let recent = |ms: i64| (ms - now).abs() < HOUR_MS;
        match v {
            Value::String(s) => {
                if is_uuid(s.as_bytes()) {
                    return Value::String(names.name("uuid", s));
                }
                if is_token(s) {
                    return Value::String(names.name("token", s));
                }
                if iso_ms(s).is_some_and(recent) {
                    return json!("<now>");
                }
                // A batch id or run key embedding one.
                let mut out = String::with_capacity(s.len());
                let mut i = 0;
                while i < s.len() {
                    match s.get(i..i + 36) {
                        Some(part) if is_uuid(part.as_bytes()) => {
                            out.push_str(&names.name("uuid", part));
                            i += 36;
                        }
                        _ => {
                            let c = s[i..].chars().next().expect("in bounds");
                            out.push(c);
                            i += c.len_utf8();
                        }
                    }
                }
                Value::String(out)
            }
            Value::Number(n) => match n.as_f64() {
                Some(f) if f.fract() == 0.0 && recent(f as i64) => json!("<now ms>"),
                _ => v.clone(),
            },
            Value::Array(items) => {
                Value::Array(items.iter().map(|i| walk(i, names, now)).collect())
            }
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for key in keys {
                    out.insert(key.clone(), walk(&map[key], names, now));
                }
                Value::Object(out)
            }
            Value::Null | Value::Bool(_) => v.clone(),
        }
    }
    walk(value, &mut Names(HashMap::new()), now)
}

/// The scenario's calls, each with the label the reference records it under.
struct Scenario {
    store: Store,
    out: Vec<(String, Value)>,
}

impl Scenario {
    fn step(&mut self, label: &str, value: Value) {
        self.out.push((label.to_owned(), value));
    }

    fn call(&mut self, name: &str, args: Value) -> Value {
        call(&mut self.store, name, args)
    }

    /// Calls `name` and records what it returned under `label`.
    fn record(&mut self, label: &str, name: &str, args: Value) -> Value {
        let value = self.call(name, args);
        self.step(label, value.clone());
        value
    }
}

fn project() -> Value {
    json!({
        "name": "proj", "path": "/tmp/proj", "preferredAgents": ["claude"],
        "icon": "Folder", "hostIds": ["local"], "workspaceId": "personal"
    })
}

fn task(id: &str, order: Value) -> Value {
    json!({
        "id": id, "projectName": "proj", "title": format!("Task {id}"), "description": "body",
        "status": "todo", "order": order,
        "createdAt": "2026-10-01T00:00:00.000Z", "updatedAt": "2026-10-01T00:00:00.000Z"
    })
}

fn workflow(id: &str) -> Value {
    json!({
        "id": id, "name": id, "icon": "Plug", "iconColor": "#fff",
        "enabled": true, "nodes": [], "edges": []
    })
}

fn item(external: &str) -> Value {
    json!({
        "connectionId": "conn-1", "connectorId": "github", "externalId": external,
        "title": format!("Item {external}"), "raw": { "externalId": external }
    })
}

fn run(id: &str, status: &str) -> Value {
    json!({
        "workflowId": "wf-1", "runId": id, "startedAt": "2026-10-01T10:00:00.000Z",
        "status": status, "triggerTaskId": "t1",
        "nodeStates": [
            { "nodeId": "n1", "status": "success", "output": "ok", "agentType": "claude" },
            { "nodeId": "gate", "status": "waiting", "waitingFor": "signIn" }
        ]
    })
}

fn with(mut value: Value, key: &str, field: Value) -> Value {
    value[key] = field;
    value
}

fn scenario(store: Store) -> Vec<(String, Value)> {
    let mut s = Scenario {
        store,
        out: Vec::new(),
    };

    // Config
    let loaded = s.record("loadConfig fresh", "loadConfig", json!([]));
    let mut config = loaded.clone();
    config["defaults"]["fontSize"] = json!(15);
    config["projects"] = json!([project()]);
    save_config(&mut s.store, config, &[]);
    s.record("loadConfig saved", "loadConfig", json!([]));

    // Projects
    s.record("dbListProjects", "dbListProjects", json!([]));
    let other = with(
        with(project(), "name", json!("other")),
        "path",
        json!("/tmp/other"),
    );
    s.call("dbInsertProject", json!([other]));
    s.call(
        "dbUpdateProject",
        json!(["other", { "icon": "Star", "hostIds": ["local", "h1"] }]),
    );
    s.record("dbGetProject", "dbGetProject", json!(["other"]));
    s.call("dbDeleteProject", json!(["other"]));
    s.record("dbGetProject deleted", "dbGetProject", json!(["other"]));

    // Tasks
    s.call("dbInsertTask", json!([task("t1", json!(0))]));
    let t2 = with(
        with(task("t2", json!(1.5)), "branch", json!("b")),
        "useWorktree",
        json!(true),
    );
    s.call("dbInsertTask", json!([t2]));
    update_task(
        &mut s.store,
        "t1",
        json!({ "status": "done", "completedAt": "2026-10-02T00:00:00.000Z" }),
        &["status", "completedAt"],
    );
    // `{ completedAt: undefined }`: JSON drops the key, the wrapper names it.
    update_task(&mut s.store, "t1", json!({}), &["completedAt"]);
    s.record("dbListTasks", "dbListTasks", json!(["proj", null]));
    s.record(
        "dbListTasks by status",
        "dbListTasks",
        json!([null, "done"]),
    );
    s.record("dbGetTask", "dbGetTask", json!(["t2"]));
    s.record("dbGetMaxTaskOrder", "dbGetMaxTaskOrder", json!(["proj"]));
    s.record(
        "dbGetMaxTaskOrder none",
        "dbGetMaxTaskOrder",
        json!(["nope"]),
    );

    // Workflows
    s.call("dbInsertWorkflow", json!([workflow("wf-1")]));
    s.call("dbInsertWorkflow", json!([workflow("wf-2")]));
    s.record(
        "dbUpdateWorkflow",
        "dbUpdateWorkflow",
        json!(["wf-2", { "name": "Two", "enabled": false }]),
    );
    s.record(
        "dbUpdateWorkflow none",
        "dbUpdateWorkflow",
        json!(["missing", { "name": "x" }]),
    );
    s.call(
        "updateWorkflowRunStatus",
        json!(["wf-1", "2026-10-01T10:00:00.000Z", "success"]),
    );
    s.record("dbGetWorkflow", "dbGetWorkflow", json!(["wf-1"]));
    s.call("dbDeleteWorkflow", json!(["wf-2"]));
    s.record("dbListWorkflows", "dbListWorkflows", json!([]));

    // Identity
    let owner = s.record("dbGetOwnerUser", "dbGetOwnerUser", json!([]));
    s.record("dbHasDeviceTokens empty", "dbHasDeviceTokens", json!([]));
    let user = owner["id"].as_str().unwrap_or("u").to_owned();
    s.call(
        "dbInsertDeviceToken",
        json!([{
            "id": "dt1", "userId": user, "name": "phone", "tokenHash": "hash",
            "createdAt": "2026-10-01T00:00:00.000Z"
        }]),
    );
    s.call(
        "dbTouchDeviceToken",
        json!(["dt1", "2026-10-01T01:00:00.000Z"]),
    );
    s.record(
        "dbGetDeviceTokenSecret",
        "dbGetDeviceTokenSecret",
        json!(["dt1"]),
    );
    s.record(
        "dbRevokeDeviceToken",
        "dbRevokeDeviceToken",
        json!(["dt1", "2026-10-01T02:00:00.000Z"]),
    );
    s.record(
        "dbRevokeDeviceToken again",
        "dbRevokeDeviceToken",
        json!(["dt1", "2026-10-01T03:00:00.000Z"]),
    );
    s.record("dbListDeviceTokens", "dbListDeviceTokens", json!([]));

    // Workspaces and groups
    s.call(
        "dbInsertWorkspace",
        json!([{ "id": "ws", "name": "Work", "order": 1 }]),
    );
    s.call(
        "dbUpdateWorkspace",
        json!(["ws", { "name": "Work 2", "iconColor": "#000" }]),
    );
    s.call(
        "dbInsertSessionGroup",
        json!([{ "id": "g1", "name": "G", "order": 0, "workspaceId": "ws" }]),
    );
    s.call("dbUpdateSessionGroup", json!(["g1", { "name": "G2" }]));
    s.record("dbListSessionGroups", "dbListSessionGroups", json!([]));
    s.call(
        "dbInsertSessionGroup",
        json!([{ "id": "g2", "name": "H", "order": 1, "workspaceId": "personal" }]),
    );
    s.call("dbDeleteSessionGroup", json!(["g2"]));
    s.call("dbDeleteWorkspace", json!(["ws"]));
    s.record("dbListWorkspaces", "dbListWorkspaces", json!([]));
    s.record(
        "dbListSessionGroups after",
        "dbListSessionGroups",
        json!([]),
    );

    // SSH keys
    s.call(
        "dbSaveSSHKey",
        json!([{
            "id": "k1", "label": "key", "encryptedPrivateKey": "enc", "publicKey": "pub",
            "createdAt": "2026-10-01T00:00:00.000Z"
        }]),
    );
    s.record("dbListSSHKeys", "dbListSSHKeys", json!([]));
    s.record("dbGetSSHKey", "dbGetSSHKey", json!(["k1"]));
    s.call("dbDeleteSSHKey", json!(["k1"]));
    s.record("dbGetSSHKey deleted", "dbGetSSHKey", json!(["k1"]));

    // Source connections and the connector inbox
    s.call(
        "dbInsertSourceConnection",
        json!([{
            "id": "conn-1", "connectorId": "github", "name": "owner/repo",
            "filters": { "owner": "owner", "repo": "repo" }, "syncIntervalMinutes": 5,
            "statusMapping": {}, "createdAt": "2026-10-01T00:00:00.000Z"
        }]),
    );
    update_connection(
        &mut s.store,
        "conn-1",
        json!({ "name": "renamed" }),
        &["name", "lastSyncAt"],
    );
    s.call(
        "dbSetConnectionSignIn",
        json!(["conn-1", "me", "2026-10-01T00:00:00.000Z"]),
    );
    s.record(
        "dbListSourceConnections",
        "dbListSourceConnections",
        json!(["github"]),
    );
    s.record(
        "dbGetSourceConnection",
        "dbGetSourceConnection",
        json!(["conn-1"]),
    );
    s.record(
        "dbGetConnectorPollCursor none",
        "dbGetConnectorPollCursor",
        json!(["wf-1", "conn-1"]),
    );
    let events: Vec<Value> = ["a", "b", "c"]
        .iter()
        .map(|id| {
            json!({
                "eventId": id, "eventType": "issueCreated",
                "eventTimestamp": "2026-10-01T00:30:00.000Z", "connectorItem": item(id)
            })
        })
        .collect();
    s.record(
        "dbRecordConnectorPollPage",
        "dbRecordConnectorPollPage",
        json!([{
            "workflowId": "wf-1", "connectionId": "conn-1", "connectorId": "github",
            "cursor": "c1", "polledAt": "2026-10-01T01:00:00.000Z", "events": events
        }]),
    );
    s.call(
        "dbRecordConnectorPollError",
        json!([{
            "workflowId": "wf-1", "connectionId": "conn-1", "error": "boom",
            "polledAt": "2026-10-01T01:30:00.000Z"
        }]),
    );
    s.record(
        "dbGetConnectorPollCursor",
        "dbGetConnectorPollCursor",
        json!(["wf-1", "conn-1"]),
    );
    s.call(
        "dbEnqueueWebhookEvent",
        json!([{
            "workflowId": "wf-1", "eventId": "hook",
            "receivedAt": "2026-10-01T01:40:00.000Z", "item": item("d")
        }]),
    );
    let claimed = s.record(
        "dbClaimConnectorInbox",
        "dbClaimConnectorInbox",
        json!([{
            "now": "2026-10-01T02:00:00.000Z", "leaseUntil": "2026-10-01T02:05:00.000Z",
            "limit": 3
        }]),
    );
    s.record(
        "dbCountActiveConnectorInboxLeases",
        "dbCountActiveConnectorInboxLeases",
        json!(["2026-10-01T02:01:00.000Z"]),
    );
    let (first, second, third) = (&claimed[0], &claimed[1], &claimed[2]);
    s.record(
        "dbCompleteConnectorInbox",
        "dbCompleteConnectorInbox",
        json!([first["id"], first["leaseToken"], "2026-10-01T02:02:00.000Z"]),
    );
    s.record(
        "dbCompleteConnectorInbox stale",
        "dbCompleteConnectorInbox",
        json!([first["id"], "wrong", "2026-10-01T02:02:00.000Z"]),
    );
    s.record(
        "dbRetryConnectorInbox",
        "dbRetryConnectorInbox",
        json!([{
            "id": second["id"], "leaseToken": second["leaseToken"], "error": "later",
            "now": "2026-10-01T02:03:00.000Z"
        }]),
    );
    s.record(
        "dbRenewConnectorInboxLease",
        "dbRenewConnectorInboxLease",
        json!([third["id"], third["leaseToken"], "2026-10-01T02:10:00.000Z"]),
    );
    s.record(
        "dbDeferConnectorInbox",
        "dbDeferConnectorInbox",
        json!([third["id"], third["leaseToken"], "2026-10-01T03:00:00.000Z"]),
    );
    s.call(
        "dbReleaseConnectorInboxLeases",
        json!(["2026-10-01T04:00:00.000Z"]),
    );
    s.record(
        "dbCountActiveConnectorInboxLeases after",
        "dbCountActiveConnectorInboxLeases",
        json!(["2026-10-01T04:00:00.000Z"]),
    );

    // Task source links
    s.call(
        "dbInsertTaskSourceLink",
        json!([{
            "taskId": "t1", "connectionId": "conn-1", "connectorId": "github",
            "externalId": "a", "externalUrl": "https://example.com/a",
            "sourceStatusRaw": "open", "sourceUpdatedAt": "2026-10-01T00:00:00.000Z",
            "lastSyncedAt": "2026-10-01T00:00:00.000Z", "conflictState": "none"
        }]),
    );
    s.call(
        "dbUpdateTaskSourceLink",
        json!(["t1", { "conflictState": "upstream_changed" }]),
    );
    s.record("dbGetTaskSourceLink", "dbGetTaskSourceLink", json!(["t1"]));
    s.record(
        "dbGetTaskSourceLinkByExternalId",
        "dbGetTaskSourceLinkByExternalId",
        json!(["conn-1", "a"]),
    );
    s.record(
        "dbListTaskSourceLinks",
        "dbListTaskSourceLinks",
        json!(["conn-1"]),
    );
    update_task(
        &mut s.store,
        "t2",
        json!({ "sourceConnectorId": "github", "sourceExternalId": "z" }),
        &["sourceConnectorId", "sourceExternalId"],
    );
    s.record(
        "dbFindTaskByConnectorExternalId",
        "dbFindTaskByConnectorExternalId",
        json!(["github", "z"]),
    );
    s.call("dbDeleteTaskSourceLink", json!(["t1"]));
    s.record(
        "dbGetTaskSourceLink deleted",
        "dbGetTaskSourceLink",
        json!(["t1"]),
    );

    // Workflow runs
    s.call("saveWorkflowRun", json!([run("wf-1:r1", "running")]));
    s.call(
        "saveWorkflowRun",
        json!([with(
            run("wf-1:r2", "success"),
            "completedAt",
            json!("2026-10-01T11:00:00.000Z")
        )]),
    );
    s.call(
        "saveWorkflowRun",
        json!([with(
            run("wf-1:r3", "running"),
            "connectorInboxId",
            first["id"].clone()
        )]),
    );
    s.record("listWorkflowRunIds", "listWorkflowRunIds", json!([]));
    s.record("getWorkflowRun", "getWorkflowRun", json!(["wf-1:r2"]));
    s.record("listWorkflowRuns", "listWorkflowRuns", json!(["wf-1", 2]));
    s.record(
        "listWorkflowRunsByTask",
        "listWorkflowRunsByTask",
        json!(["t1", 20]),
    );
    s.record("listRunningRuns", "listRunningRuns", json!([]));
    s.record(
        "listRunsWithWaitingGates",
        "listRunsWithWaitingGates",
        json!([null]),
    );
    s.record(
        "listRunsWithWaitingGates signIn",
        "listRunsWithWaitingGates",
        json!(["signIn"]),
    );
    s.record(
        "listAllWorkflowRuns",
        "listAllWorkflowRuns",
        json!(["personal", 10]),
    );
    s.record(
        "dbGetWorkflowRunByConnectorInboxId",
        "dbGetWorkflowRunByConnectorInboxId",
        json!([first["id"]]),
    );

    // Sessions, schedule log, effects, events
    s.call(
        "saveSessions",
        json!([[{
            "id": "s1", "agentType": "claude", "projectName": "proj",
            "projectPath": "/tmp/proj", "status": "running", "createdAt": 1_700_000_000_000_i64,
            "pid": 42, "branch": "main", "isWorktree": false
        }]]),
    );
    s.record("getPreviousSessions", "getPreviousSessions", json!([]));
    s.call("clearSessions", json!([]));
    s.record(
        "getPreviousSessions cleared",
        "getPreviousSessions",
        json!([]),
    );
    s.call(
        "addScheduleLogEntry",
        json!([{
            "workflowId": "wf-1", "workflowName": "wf-1",
            "executedAt": "2026-10-01T00:00:00.000Z", "status": "error",
            "sessionsLaunched": 0, "error": "nope"
        }]),
    );
    s.record(
        "getScheduleLogEntries",
        "getScheduleLogEntries",
        json!(["wf-1"]),
    );
    s.call("clearScheduleLog", json!([]));
    s.record(
        "getScheduleLogEntries cleared",
        "getScheduleLogEntries",
        json!([null]),
    );
    s.record("claimEffect", "claimEffect", json!(["e1", "notify", 1000]));
    s.record(
        "claimEffect again",
        "claimEffect",
        json!(["e1", "notify", 2000]),
    );
    s.record(
        "pruneEffectReceipts",
        "pruneEffectReceipts",
        json!(["notify", 1500]),
    );
    s.call(
        "insertSessionEvent",
        json!([{
            "sessionId": "s1", "eventType": "created",
            "timestamp": "2026-10-01T00:00:00.000Z", "metadata": { "a": 1 }
        }]),
    );
    s.record(
        "listSessionEvents",
        "listSessionEvents",
        json!(["created", 5]),
    );
    s.record(
        "listSessionEventsBySession",
        "listSessionEventsBySession",
        json!(["s1", 100]),
    );

    // Artifacts
    let inserted = s.call(
        "insertArtifact",
        json!([{
            "kind": "page", "title": "Page", "sessionId": "s1", "projectName": "proj",
            "gateRunId": "wf-1:r1", "gateNodeId": "gate"
        }]),
    );
    let artifact = inserted["artifact"]["id"]
        .as_str()
        .expect("an artifact id")
        .to_owned();
    s.record("getArtifact", "getArtifact", json!([artifact]));
    let token = s.call("getArtifactToken", json!([artifact]));
    s.step(
        "getArtifactToken",
        json!(if token.is_string() {
            "string"
        } else {
            "object"
        }),
    );
    s.record(
        "findGateArtifact",
        "findGateArtifact",
        json!(["wf-1:r1", "gate"]),
    );
    s.call("renameArtifact", json!([artifact, "Renamed"]));
    s.record(
        "listArtifacts",
        "listArtifacts",
        json!([{ "projectName": "proj" }, 5]),
    );
    s.record(
        "addArtifactVersion",
        "addArtifactVersion",
        json!([artifact, "agent", null]),
    );
    let comment = s.record(
        "insertArtifactComment",
        "insertArtifactComment",
        json!([{ "artifactId": artifact, "version": 1, "anchor": null, "body": "first" }]),
    );
    s.record(
        "updateArtifactComment",
        "updateArtifactComment",
        json!([comment["id"], { "body": "edited" }]),
    );
    s.record(
        "getArtifactComment",
        "getArtifactComment",
        json!([comment["id"]]),
    );
    s.record(
        "sendArtifactDrafts",
        "sendArtifactDrafts",
        json!([artifact]),
    );
    s.record("unansweredBatchId", "unansweredBatchId", json!([artifact]));
    s.record(
        "listArtifactComments",
        "listArtifactComments",
        json!([artifact, { "state": "sent" }]),
    );
    s.record(
        "listArtifactVersions",
        "listArtifactVersions",
        json!([artifact]),
    );
    let draft = s.call(
        "insertArtifactComment",
        json!([{ "artifactId": artifact, "version": 1, "anchor": null, "body": "second" }]),
    );
    s.record(
        "deleteArtifactComment",
        "deleteArtifactComment",
        json!([draft["id"]]),
    );
    s.record("listArtifactIds", "listArtifactIds", json!([]));
    s.record(
        "deleteArtifactsUpdatedBefore",
        "deleteArtifactsUpdatedBefore",
        json!(["2000-01-01T00:00:00.000Z"]),
    );

    // A final snapshot of everything, and a second save of it
    let last = s.record("loadConfig final", "loadConfig", json!([]));
    save_config(&mut s.store, last, &[]);
    s.record("loadConfig resaved", "loadConfig", json!([]));
    s.out
}

#[test]
fn every_call_is_answered_as_the_typescript_store_answered_it() {
    let store = Store::open_in_memory(options_with("<default shell>", "<owner name>"))
        .expect("an in-memory store opens");
    let now = now_ms();
    let actual: Vec<(String, Value)> = scenario(store)
        .into_iter()
        .map(|(label, value)| (label, normalize(&value, now)))
        .collect();
    let reference: Vec<(String, Value)> =
        serde_json::from_str(REFERENCE).expect("the reference parses");

    let labels = |steps: &[(String, Value)]| -> Vec<String> {
        steps.iter().map(|(label, _)| label.clone()).collect()
    };
    assert_eq!(labels(&actual), labels(&reference));
    for ((label, got), (_, want)) in actual.iter().zip(&reference) {
        assert_eq!(got, want, "{label}\n got: {got}\nwant: {want}");
    }
}

#[test]
fn the_normalizer_names_what_the_store_made_up() {
    let now = iso_ms("2026-10-09T12:00:00.000Z").expect("an ISO time");
    let id = "0b9c2c4e-6f5a-4c1e-9d7b-2a3f4e5d6c7b";
    let value = json!({
        "b": id,
        "a": format!("run:{id}"),
        "token": "0123456789abcdef0123456789abcdef",
        "at": "2026-10-09T11:30:00.000Z",
        "old": "2026-10-01T00:00:00.000Z",
        "ms": now - 1000,
        "order": 1.5
    });
    assert_eq!(
        normalize(&value, now),
        json!({
            "a": "run:<uuid 1>",
            "at": "<now>",
            "b": "<uuid 1>",
            "ms": "<now ms>",
            "old": "2026-10-01T00:00:00.000Z",
            "order": 1.5,
            "token": "<token 2>"
        })
    );
    assert_eq!(iso_ms("1970-01-01T00:00:01.500Z"), Some(1500));
}
