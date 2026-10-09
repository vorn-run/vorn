//! The tools over a fake server: what each sends, and what it answers.

use serde_json::{json, Value};

use super::fake::{block_on, caller, sample_config, FakeRpc};
use super::{call, Args, Cx};
use crate::rpc::Caller;

fn run_as(rpc: &FakeRpc, who: &Caller, tool: &str, args: Value) -> Value {
    let args = match args {
        Value::Null => None,
        other => Some(other),
    };
    let cx = Cx { rpc, caller: who };
    match block_on(call(&cx, tool, Args::new(args))) {
        Ok(result) => result,
        Err(thrown) => json!({ "thrown": thrown }),
    }
}

fn run(rpc: &FakeRpc, tool: &str, args: Value) -> Value {
    run_as(rpc, &caller(), tool, args)
}

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap_or_default()
}

fn parsed(result: &Value) -> Value {
    serde_json::from_str(text(result)).unwrap()
}

#[test]
fn every_listed_tool_is_implemented() {
    let rpc = FakeRpc::new(sample_config());
    for tool in crate::tools_list().as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        let result = run(&rpc, name, json!({}));
        let said = result
            .get("thrown")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            !said.contains(&format!("Tool {name} not found")),
            "{name} is not implemented: {said}"
        );
    }
    assert_eq!(crate::tool_count(), 73);
}

/// The task, workflow and config methods the calls so far wrote through.
fn writes(rpc: &FakeRpc) -> Vec<String> {
    rpc.calls()
        .into_iter()
        .map(|(method, _)| method)
        .filter(|m| m == "config:save" || m.starts_with("task:") || m.starts_with("workflow:"))
        .collect()
}

fn stored_task() -> Value {
    json!({ "id": "t2", "projectName": "app", "title": "Second", "description": "", "status": "done",
            "order": 2, "createdAt": "2026-01-02T00:00:00.000Z", "updatedAt": "2026-01-02T00:00:00.000Z",
            "completedAt": "2026-01-02T00:00:00.000Z" })
}

#[test]
fn creates_a_task_through_task_create() {
    let rpc = FakeRpc::new(sample_config());
    rpc.answer("task:create", json!({ "ok": true, "task": stored_task() }));
    let result = run(
        &rpc,
        "create_task",
        json!({ "project_name": "app", "title": "Second", "status": "done", "branch": "", "use_worktree": false }),
    );
    assert_eq!(parsed(&result), stored_task());
    assert_eq!(
        rpc.last("task:create"),
        Some(
            json!({ "projectName": "app", "title": "Second", "status": "done", "branch": "", "useWorktree": false })
        )
    );
    assert_eq!(writes(&rpc), ["task:create"]);

    rpc.answer("task:create", json!({ "ok": false }));
    let missing = run(
        &rpc,
        "create_task",
        json!({ "project_name": "nope", "title": "x" }),
    );
    assert_eq!(missing["isError"], true);
    assert_eq!(text(&missing), "Error: project \"nope\" not found");
}

#[test]
fn updates_a_task_through_task_update() {
    let rpc = FakeRpc::new(sample_config());
    rpc.answer("task:update", json!({ "ok": true, "task": stored_task() }));
    let result = run(
        &rpc,
        "update_task",
        json!({ "id": "t2", "order": 4, "status": "done", "title": "Second" }),
    );
    assert_eq!(parsed(&result), stored_task());
    assert_eq!(
        rpc.last("task:update"),
        Some(json!({ "id": "t2", "title": "Second", "status": "done", "order": 4 }))
    );
    assert_eq!(writes(&rpc), ["task:update"]);

    rpc.answer("task:update", json!({ "ok": false }));
    let gone = run(&rpc, "update_task", json!({ "id": "t9", "title": "x" }));
    assert_eq!(gone["isError"], true);
    assert_eq!(text(&gone), "Error: task \"t9\" not found");
}

#[test]
fn deletes_a_task_through_task_delete() {
    let rpc = FakeRpc::new(sample_config());
    rpc.answer("task:delete", json!({ "ok": true }));
    let deleted = run(&rpc, "delete_task", json!({ "id": "t1" }));
    assert_eq!(text(&deleted), "Deleted task: First");
    assert_eq!(rpc.last("task:delete"), Some(json!({ "id": "t1" })));

    let missing = run(&rpc, "delete_task", json!({ "id": "t9" }));
    assert_eq!(text(&missing), "Error: task \"t9\" not found");
    assert_eq!(writes(&rpc), ["task:delete"]);

    rpc.answer("task:delete", json!({ "ok": false }));
    let raced = run(&rpc, "delete_task", json!({ "id": "t1" }));
    assert_eq!(raced["isError"], true);
    assert_eq!(text(&raced), "Error: task \"t1\" not found");
}

#[test]
fn archives_and_restores_a_task_through_task_archive() {
    let mut config = sample_config();
    config["tasks"][0]["status"] = json!("done");
    let rpc = FakeRpc::new(config.clone());
    rpc.answer("task:archive", json!({ "ok": true }));
    let archived = run(&rpc, "archive_task", json!({ "id": "t1" }));
    assert_eq!(parsed(&archived), config["tasks"][0]);
    assert_eq!(
        rpc.last("task:archive"),
        Some(json!({ "id": "t1", "archived": true }))
    );
    run(&rpc, "unarchive_task", json!({ "id": "t1" }));
    assert_eq!(
        rpc.last("task:archive"),
        Some(json!({ "id": "t1", "archived": false }))
    );
    assert_eq!(writes(&rpc), ["task:archive", "task:archive"]);

    rpc.answer("task:archive", json!({ "ok": false }));
    let raced = run(&rpc, "unarchive_task", json!({ "id": "t1" }));
    assert_eq!(text(&raced), "Error: task \"t1\" not found");

    let open = FakeRpc::new(sample_config());
    let refused = run(&open, "archive_task", json!({ "id": "t1" }));
    assert_eq!(
        text(&refused),
        "Error: only done or cancelled tasks can be archived (status: in_progress)"
    );
    assert!(writes(&open).is_empty());
}

#[test]
fn context_comes_from_the_callers_directory() {
    let rpc = FakeRpc::new(sample_config());
    let mut who = caller();
    who.cwd = "/work/app/src".into();
    let found = parsed(&run_as(&rpc, &who, "get_my_context", json!({})));
    assert_eq!(found["project"]["name"], "app");
    assert_eq!(found["cwd"], "/work/app/src");
    assert_eq!(found["task"]["id"], "t1");

    let lost = parsed(&run(
        &rpc,
        "get_my_context",
        json!({ "cwd": "/elsewhere/../tmp" }),
    ));
    assert_eq!(lost["cwd"], "/tmp");
    assert_eq!(
        lost["message"],
        "No matching project found for current directory."
    );
}

#[test]
fn projects_and_workspaces_round_trip() {
    let rpc = FakeRpc::new(sample_config());
    let dup = run(
        &rpc,
        "create_project",
        json!({ "name": "app", "path": "/x" }),
    );
    assert_eq!(text(&dup), "Error: project \"app\" already exists");
    let made = parsed(&run(
        &rpc,
        "create_project",
        json!({ "name": "b", "path": "/b", "icon": "" }),
    ));
    assert_eq!(
        made,
        json!({ "name": "b", "path": "/b", "preferredAgents": [] })
    );
    let ws = parsed(&run(&rpc, "create_workspace", json!({ "name": "Team" })));
    assert_eq!(ws["order"], 1);
    let gone = run(&rpc, "delete_workspace", json!({ "id": "personal" }));
    assert_eq!(text(&gone), "Error: cannot delete the default workspace");
    let listed = parsed(&run(
        &rpc,
        "list_projects",
        json!({ "workspace_id": "personal" }),
    ));
    assert_eq!(
        listed.as_array().unwrap().len(),
        2,
        "a project without a workspace is in the default one"
    );
}

#[test]
fn keys_map_to_the_bytes_a_terminal_expects() {
    let rpc = FakeRpc::new(sample_config());
    run(&rpc, "send_key", json!({ "id": "s", "key": " Ctrl+B " }));
    assert_eq!(
        rpc.last("terminal:write"),
        Some(json!({ "id": "s", "data": "\u{2}" }))
    );
    run(
        &rpc,
        "write_to_terminal",
        json!({ "id": "s", "data": "ls\n\n" }),
    );
    assert_eq!(
        rpc.last("terminal:write"),
        Some(json!({ "id": "s", "data": "ls\r" }))
    );
    let unknown = run(&rpc, "send_key", json!({ "id": "s", "key": "hyper" }));
    assert!(text(&unknown).starts_with("Unknown key: \"hyper\". Supported: single chars (1, y, n), named keys (enter, escape, esc,"));
}

#[test]
fn session_tools_need_a_session() {
    let rpc = FakeRpc::new(sample_config());
    let nobody = Caller {
        cwd: "/".into(),
        session: None,
    };
    let browser = run_as(&rpc, &nobody, "read_page", json!({}));
    assert_eq!(browser["isError"], true);
    assert!(text(&browser).contains("Browser tools only work"));
    let device = run_as(&rpc, &nobody, "device_list", json!({}));
    assert!(text(&device)
        .contains("Device tools only work from a terminal session started by the Vorn app."));
    assert!(rpc.calls().is_empty());
}

#[test]
fn page_reads_are_fenced_and_failures_reported() {
    let rpc = FakeRpc::new(sample_config());
    rpc.answer("browser:readPage", json!({ "nodes": [] }));
    let read = run(&rpc, "read_page", json!({ "limit": 5 }));
    assert!(text(&read).starts_with("[BEGIN UNTRUSTED WEB PAGE CONTENT "));
    assert_eq!(
        rpc.last("browser:readPage"),
        Some(json!({ "sessionId": "s1", "limit": 5 }))
    );

    rpc.fail("browser:navigate", "No browser pane");
    let failed = run(&rpc, "browser_navigate", json!({ "url": "https://x" }));
    assert_eq!(
        failed,
        json!({ "content": [{ "type": "text", "text": "Error: No browser pane" }], "isError": true })
    );

    rpc.answer(
        "device:claim",
        json!({ "ok": false, "message": "held by s2" }),
    );
    assert_eq!(
        text(&run(&rpc, "device_claim", json!({ "udid": "u" }))),
        "Error: held by s2"
    );
}

#[test]
fn a_secret_is_never_installed_from_here() {
    let rpc = FakeRpc::new(sample_config());
    rpc.answer("connector:catalog", json!({ "items": [] }));
    rpc.answer(
        "connector:probeSdk",
        json!({ "ok": true, "manifest": {
            "id": "acme", "name": "Acme", "version": "1.0.0",
            "env": [{ "name": "TOKEN", "secret": true, "required": true }, { "name": "HOST", "required": true, "description": "where" }],
            "triggers": [{ "type": "issues", "label": "Issues" }]
        } }),
    );
    let refused = run(&rpc, "install_connector", json!({ "package": "@acme/c" }));
    assert!(text(&refused)
        .starts_with("Error: Acme uses the secret value TOKEN, which this tool cannot accept"));
    assert_eq!(
        rpc.last("connector:probeSdk"),
        Some(json!({ "command": "npx", "args": ["-y", "@acme/c"] }))
    );
    let unknown = run(
        &rpc,
        "install_connector",
        json!({ "package": "@acme/c", "env": { "NOPE": "1" } }),
    );
    assert_eq!(
        text(&unknown),
        "Error: Acme does not use NOPE. It accepts: TOKEN, HOST."
    );
}

#[test]
fn installs_a_connection_with_its_filters() {
    let rpc = FakeRpc::new(sample_config());
    rpc.answer("connector:catalog", json!({ "items": [] }));
    rpc.answer(
        "connector:probeSdk",
        json!({ "ok": true, "manifest": { "id": "acme", "name": "Acme", "version": "1.0.0", "env": [], "triggers": [{ "type": "issues", "label": "Issues" }] } }),
    );
    rpc.answer("connection:create", json!({ "id": "c9" }));
    let done = parsed(&run(
        &rpc,
        "install_connector",
        json!({ "package": "node /x/index.js", "project": "app" }),
    ));
    assert_eq!(done["connectionId"], "c9");
    assert_eq!(done["trigger"], "issues");
    assert_eq!(
        rpc.last("connection:create"),
        Some(json!({
            "connectorId": "sdk",
            "name": "Acme: Issues",
            "filters": { "command": "node", "args": "[\"/x/index.js\"]", "env": "{}", "sdkConnectorId": "acme", "sdkVersion": "1.0.0", "sdkTrigger": "issues" },
            "syncIntervalMinutes": 5,
            "statusMapping": {},
            "executionProject": "app"
        }))
    );
}

#[test]
fn lists_connectors_with_their_connections() {
    let rpc = FakeRpc::new(sample_config());
    rpc.answer("connector:list", json!([{ "id": "github", "name": "GitHub", "capabilities": ["poll"] }, { "id": "hidden", "addable": false }]));
    rpc.answer("connector:catalog", json!({ "items": [{ "id": "lin", "name": "Linear", "packageName": "@x/lin", "capabilities": [], "env": [{ "name": "K" }] }] }));
    rpc.answer(
        "connection:list",
        json!([{ "id": "a", "connectorId": "github", "filters": {} }]),
    );
    rpc.answer(
        "connector:status",
        json!([{ "connectorId": "github", "authed": true }]),
    );
    rpc.answer("connector:listPacks", json!([]));
    let listed = parsed(&run(&rpc, "list_connectors", json!({})));
    assert_eq!(
        listed,
        json!([
            { "id": "github", "name": "GitHub", "source": "built-in", "kind": "connector", "capabilities": ["poll"], "connections": 1, "authenticated": true },
            { "id": "lin", "name": "Linear", "source": "package", "kind": "connector", "package": "@x/lin", "capabilities": [], "connections": 0, "env": ["K"] }
        ])
    );
    let open = parsed(&run(
        &rpc,
        "list_connectors",
        json!({ "installable_only": true }),
    ));
    assert_eq!(open.as_array().unwrap().len(), 1);
}

#[test]
fn workflows_are_checked_stored_and_run() {
    let rpc = FakeRpc::new(sample_config());
    let made = parsed(&run(
        &rpc,
        "create_workflow",
        json!({ "name": "Flat", "actions": [{ "agentType": "claude", "projectName": "app", "projectPath": "/work/app" }] }),
    ));
    assert_eq!(made["nodes"][0]["label"], "Manual Trigger");
    assert_eq!(made["nodes"][1]["label"], "Launch claude");
    assert_eq!(made["nodes"][1]["position"], json!({ "x": 0, "y": 140 }));
    assert_eq!(made["edges"][0]["source"], made["nodes"][0]["id"]);
    let id = made["id"].as_str().unwrap().to_owned();

    rpc.answer("workflow:runManual", json!(null));
    let queued = run(
        &rpc,
        "execute_workflow",
        json!({ "workflow_id": id, "inputs": { "x": 1 } }),
    );
    assert_eq!(
        text(&queued),
        format!("Queued \"Flat\"\ninputs: {{\n  \"x\": 1\n}}\n\nRun history: list_workflow_runs with workflow_id {id}")
    );

    let bad = run(
        &rpc,
        "update_workflow",
        json!({ "id": id, "nodes": [{ "id": "l", "type": "loop", "label": "L", "config": { "mode": "repeat" }, "position": { "x": 0, "y": 0 } }] }),
    );
    assert_eq!(bad["isError"], true);
    assert!(
        text(&bad).starts_with("Error: node \"L\" config.bodyNodeIds: "),
        "{}",
        text(&bad)
    );

    let renamed = run(
        &rpc,
        "update_workflow",
        json!({ "workflow_id": id, "name": "Renamed" }),
    );
    assert_eq!(parsed(&renamed)["name"], "Renamed");
    assert_eq!(
        text(&run(&rpc, "delete_workflow", json!({ "id": id }))),
        "Deleted workflow: Renamed"
    );
}

#[test]
fn workflows_are_written_through_their_methods() {
    let mut config = sample_config();
    config["workflows"] = json!([{ "id": "w", "name": "Nightly", "icon": "Zap", "iconColor": "#000",
                                   "enabled": true, "nodes": [], "edges": [] }]);
    let rpc = FakeRpc::new(config);
    let made = parsed(&run(&rpc, "create_workflow", json!({ "name": "Morning" })));
    assert_eq!(
        rpc.last("workflow:create"),
        Some(json!({ "workflow": made }))
    );

    let renamed = run(
        &rpc,
        "update_workflow",
        json!({ "workflow_id": "w", "name": "Nightly 2", "enabled": false }),
    );
    assert_eq!(parsed(&renamed)["name"], "Nightly 2");
    assert_eq!(
        rpc.last("workflow:update"),
        Some(json!({ "id": "w", "updates": { "name": "Nightly 2", "enabled": false } }))
    );

    let deleted = run(&rpc, "delete_workflow", json!({ "workflow_id": "w" }));
    assert_eq!(text(&deleted), "Deleted workflow: Nightly 2");
    assert_eq!(rpc.last("workflow:delete"), Some(json!({ "id": "w" })));
    assert_eq!(
        writes(&rpc),
        ["workflow:create", "workflow:update", "workflow:delete"]
    );
}

#[test]
fn a_workflow_gone_by_the_time_it_is_written_fails() {
    let mut config = sample_config();
    config["workflows"] = json!([{ "id": "w", "name": "Nightly", "nodes": [], "edges": [] }]);
    let rpc = FakeRpc::new(config);
    rpc.answer("workflow:update", json!({ "ok": false }));
    rpc.answer("workflow:delete", json!({ "ok": false }));
    let updated = run(
        &rpc,
        "update_workflow",
        json!({ "workflow_id": "w", "name": "x" }),
    );
    assert_eq!(updated, json!({ "thrown": "workflow \"w\" not found" }));
    let deleted = run(&rpc, "delete_workflow", json!({ "workflow_id": "w" }));
    assert_eq!(deleted, json!({ "thrown": "workflow \"w\" not found" }));
}

#[test]
fn a_gate_is_answered_on_the_run_that_waits() {
    let rpc = FakeRpc::new(sample_config());
    rpc.answer(
        "workflowRun:listAll",
        json!([{ "runId": "r1", "workflowId": "w", "workflowName": "Ship", "status": "running",
        "nodeStates": [{ "nodeId": "g", "status": "waiting", "message": "Ship it?" }] }]),
    );
    rpc.answer("workflowRun:listWaiting", json!([]));
    rpc.answer("workflow:resolveGate", json!({ "accepted": true }));
    let done = run(
        &rpc,
        "resolve_gate",
        json!({ "run_id": "r1", "decision": "approve", "comment": "  fine " }),
    );
    assert_eq!(
        text(&done),
        "Approved \"g\" on run r1 of \"Ship\".\n\nWhat it asked: Ship it?\n\nThe decision went out; the instance holding the run acts on it, so a desktop has to be open. Confirm with list_workflow_runs."
    );
    assert_eq!(
        rpc.last("workflow:resolveGate"),
        Some(json!({ "runId": "r1", "nodeId": "g", "decision": "approve", "comment": "fine" }))
    );
    let changes = run(
        &rpc,
        "resolve_gate",
        json!({ "run_id": "r1", "decision": "changes" }),
    );
    assert_eq!(
        text(&changes),
        "Error: changes needs a comment saying what to change."
    );
}

#[test]
fn a_workflow_travels_by_file() {
    let mut config = sample_config();
    config["workflows"] = json!([{
        "id": "w", "name": "Ship", "icon": "Zap", "iconColor": "#000", "enabled": true, "edges": [],
        "nodes": [{ "id": "s", "type": "script", "label": "S", "position": { "x": 0, "y": 0 },
                    "config": { "scriptType": "bash", "scriptContent": "make", "projectName": "app", "cwd": "/work/app/sub" } }]
    }]);
    let rpc = FakeRpc::new(config);
    rpc.answer("connection:list", json!([]));
    let exported = text(&run(&rpc, "export_workflow", json!({ "workflow_id": "w" }))).to_owned();
    let file: Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(file["nodes"][0]["config"]["cwd"], "{{project.path}}/sub");

    let imported = run(
        &rpc,
        "import_workflow",
        json!({ "workflow": exported, "project_name": "app" }),
    );
    assert_eq!(
        text(&imported),
        "Imported \"Ship\" as import:app:ship, resolved against /work/app. It is disabled; enable it when ready"
    );
    let again = run(
        &rpc,
        "import_workflow",
        json!({ "workflow": exported, "project_name": "app" }),
    );
    assert!(text(&again).starts_with("Updated \"Ship\" as import:app:ship"));
    assert_eq!(rpc.config()["workflows"].as_array().unwrap().len(), 2);
    let created = rpc.last("workflow:create").unwrap();
    assert_eq!(created["workflow"]["id"], "import:app:ship");
    assert_eq!(created["workflow"]["enabled"], false);
    let updated = rpc.last("workflow:update").unwrap();
    assert_eq!(updated["id"], "import:app:ship");
    assert_eq!(updated["updates"], created["workflow"]);
    assert_eq!(writes(&rpc), ["workflow:create", "workflow:update"]);

    let broken = run(
        &rpc,
        "import_workflow",
        json!({ "workflow": "{", "project_name": "app" }),
    );
    assert!(text(&broken).starts_with("Error: workflow is not valid JSON — SyntaxError: "));
    let old = run(
        &rpc,
        "import_workflow",
        json!({ "workflow": "{\"version\":2}", "project_name": "app" }),
    );
    assert_eq!(
        text(&old),
        "Error: unsupported format version 2; this build reads version 1"
    );
}

#[test]
fn describes_only_the_node_types_asked_for() {
    let rpc = FakeRpc::new(sample_config());
    let some = parsed(&run(
        &rpc,
        "describe_workflow_nodes",
        json!({ "types": ["loop"] }),
    ));
    assert_eq!(
        some["nodeTypes"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["loop"]
    );
    let all = parsed(&run(&rpc, "describe_workflow_nodes", json!({})));
    assert_eq!(all, *crate::workflow_nodes());
}
