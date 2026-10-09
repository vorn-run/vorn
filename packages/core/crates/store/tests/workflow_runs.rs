//! Workflow runs: what a run and its steps keep across a reload, which are trimmed, and their listings.

mod common;

use common::{call, ids, lacks, memory, save_config};
use serde_json::{json, Value};
use vorn_store::Store;

fn save(store: &mut Store, run: Value) {
    call(store, "saveWorkflowRun", json!([run]));
}

/// `listWorkflowRuns(workflowId, limit = 20)`.
fn runs(store: &mut Store, workflow: &str, limit: i64) -> Vec<Value> {
    call(store, "listWorkflowRuns", json!([workflow, limit]))
        .as_array()
        .cloned()
        .expect("a list")
}

fn first_run(store: &mut Store, workflow: &str) -> Value {
    runs(store, workflow, 20).into_iter().next().expect("a run")
}

mod persistence {
    use super::*;

    #[test]
    fn agent_and_project_fields_of_a_step_round_trip() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "workflowId": "wf-1", "runId": "wf-1:2026-04-20T10:00:00Z",
                "startedAt": "2026-04-20T10:00:00Z", "completedAt": "2026-04-20T10:00:05Z",
                "status": "success",
                "nodeStates": [{
                    "nodeId": "node-1", "status": "success", "agentSessionId": "agent-xyz",
                    "agentType": "claude", "projectName": "proj", "projectPath": "/abs/proj",
                    "approvedAt": "2026-04-20T10:00:04Z"
                }]
            }),
        );
        let listed = runs(&mut store, "wf-1", 20);
        assert_eq!(listed.len(), 1);
        let state = &listed[0]["nodeStates"][0];
        assert_eq!(state["agentType"], "claude");
        assert_eq!(state["projectName"], "proj");
        assert_eq!(state["projectPath"], "/abs/proj");
        assert_eq!(state["agentSessionId"], "agent-xyz");
        assert_eq!(state["approvedAt"], "2026-04-20T10:00:04Z");
    }

    #[test]
    fn the_worktree_a_step_made_round_trips() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "workflowId": "wf-1", "runId": "wf-1:2026-09-10T10:00:00Z",
                "startedAt": "2026-09-10T10:00:00Z", "status": "running",
                "nodeStates": [
                    {
                        "nodeId": "research", "status": "success",
                        "worktreePath": "/repos/.vorn-worktrees/app/silver-madrigal-d4dc9209",
                        "worktreeName": "silver-madrigal", "worktreeOrigin": "created"
                    },
                    { "nodeId": "approve", "status": "waiting" }
                ]
            }),
        );
        let run = first_run(&mut store, "wf-1");
        let made = &run["nodeStates"][0];
        assert_eq!(
            made["worktreePath"],
            "/repos/.vorn-worktrees/app/silver-madrigal-d4dc9209"
        );
        assert_eq!(made["worktreeName"], "silver-madrigal");
        assert_eq!(made["worktreeOrigin"], "created");
        assert!(lacks(&run["nodeStates"][1], "worktreePath"));
    }

    #[test]
    fn a_gates_text_and_the_reviewers_rewrite_round_trip() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "workflowId": "wf-1", "runId": "wf-1:2026-09-16T10:00:00Z",
                "startedAt": "2026-09-16T10:00:00Z", "status": "running",
                "nodeStates": [
                    {
                        "nodeId": "approve", "status": "waiting",
                        "editableText": "A tidy draft.", "editedText": "My words.",
                        "feedback": [{
                            "round": 1, "decision": "changes", "comment": "Too neat",
                            "at": "", "edited": "My words."
                        }]
                    },
                    { "nodeId": "draft", "status": "success" }
                ]
            }),
        );
        let run = first_run(&mut store, "wf-1");
        let gate = &run["nodeStates"][0];
        assert_eq!(gate["editableText"], "A tidy draft.");
        assert_eq!(gate["editedText"], "My words.");
        assert_eq!(gate["feedback"][0]["edited"], "My words.");
        assert!(lacks(&run["nodeStates"][1], "editableText"));
    }

    #[test]
    fn step_diagnostics_round_trip() {
        let mut store = memory();
        let timeline = "[+0.0s] Launching claude in /abs/proj\n\
            [+0.4s] Session sess-1 started (pid 4242): claude --dangerously-skip-permissions -p\n\
            [+3600.0s] Step timed out. The agent was started but never produced any output.";
        save(
            &mut store,
            json!({
                "workflowId": "wf-diag", "runId": "wf-diag:2026-04-20T10:00:00Z",
                "startedAt": "2026-04-20T10:00:00Z", "status": "error",
                "nodeStates": [{ "nodeId": "node-1", "status": "error", "diagnostics": timeline }]
            }),
        );
        assert_eq!(
            first_run(&mut store, "wf-diag")["nodeStates"][0]["diagnostics"],
            timeline
        );
    }

    #[test]
    fn the_connector_inbox_row_round_trips_across_gate_resumes() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "workflowId": "wf-connector", "runId": "wf-connector:2026-04-20T10:00:00Z",
                "startedAt": "2026-04-20T10:00:00Z", "status": "running",
                "connectorInboxId": 73, "connectorInboxLeaseToken": "lease-73",
                "connectorItem": {
                    "inboxId": 73, "inboxLeaseToken": "lease-73", "connectionId": "conn-1",
                    "connectorId": "github", "externalId": "issue-73", "title": "Persist me",
                    "raw": { "number": 73 }
                },
                "nodeStates": [{ "nodeId": "approval", "status": "waiting" }]
            }),
        );
        let run = first_run(&mut store, "wf-connector");
        assert_eq!(run["connectorInboxId"], 73);
        assert_eq!(run["connectorInboxLeaseToken"], "lease-73");
        assert_eq!(run["connectorItem"]["externalId"], "issue-73");
        assert_eq!(run["connectorItem"]["title"], "Persist me");
        assert_eq!(run["connectorItem"]["raw"], json!({ "number": 73 }));
    }

    #[test]
    fn fields_that_were_not_set_are_left_out() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "workflowId": "wf-2", "runId": "wf-2:2026-04-20T11:00:00Z",
                "startedAt": "2026-04-20T11:00:00Z", "status": "success",
                "nodeStates": [{ "nodeId": "node-1", "status": "success" }]
            }),
        );
        let state = &first_run(&mut store, "wf-2")["nodeStates"][0];
        for key in ["agentType", "projectName", "projectPath"] {
            assert!(lacks(state, key), "{key}");
        }
    }

    #[test]
    fn a_running_connector_run_needed_for_recovery_is_never_trimmed() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "runId": "active-connector-run", "workflowId": "wf-trim",
                "startedAt": "2026-04-20T00:00:00Z", "status": "running",
                "connectorInboxId": 500, "connectorInboxLeaseToken": "lease-500",
                "nodeStates": [{ "nodeId": "agent", "status": "running" }]
            }),
        );
        for index in 0..50 {
            save(
                &mut store,
                json!({
                    "runId": format!("completed-{index}"), "workflowId": "wf-trim",
                    "startedAt": format!("2026-04-21T00:{index:02}:00Z"),
                    "completedAt": format!("2026-04-21T00:{index:02}:30Z"),
                    "status": "success", "nodeStates": []
                }),
            );
        }
        assert!(runs(&mut store, "wf-trim", 100)
            .iter()
            .any(|r| r["runId"] == "active-connector-run"));
    }

    #[test]
    fn a_finished_connector_run_whose_inbox_row_is_gone_is_trimmed() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "runId": "orphan-connector-run", "workflowId": "wf-orphan",
                "startedAt": "2026-04-20T00:00:00Z", "completedAt": "2026-04-20T00:00:30Z",
                "status": "success", "connectorInboxId": 900, "nodeStates": []
            }),
        );
        for index in 0..60 {
            save(
                &mut store,
                json!({
                    "runId": format!("later-{index}"), "workflowId": "wf-orphan",
                    "startedAt": format!("2026-04-21T00:{index:02}:00Z"),
                    "completedAt": format!("2026-04-21T00:{index:02}:30Z"),
                    "status": "success", "nodeStates": []
                }),
            );
        }
        let listed = runs(&mut store, "wf-orphan", 200);
        assert!(!listed.iter().any(|r| r["runId"] == "orphan-connector-run"));
        assert!(listed.len() <= 51, "{}", listed.len());
    }
}

mod all_runs {
    use super::*;

    /// `listAllWorkflowRuns(workspaceId, limit = 50)`.
    fn all(store: &mut Store, workspace: Value, limit: i64) -> Vec<Value> {
        call(store, "listAllWorkflowRuns", json!([workspace, limit]))
            .as_array()
            .cloned()
            .expect("a list")
    }

    fn workflow_ids(runs: &[Value]) -> Vec<String> {
        ids(&Value::Array(runs.to_vec()), "workflowId")
    }

    fn config_with_workflows(workflows: &[(&str, &str, Option<&str>)]) -> Value {
        let workflows: Vec<Value> = workflows
            .iter()
            .map(|(id, name, workspace)| {
                let mut w = json!({
                    "id": id, "name": name, "icon": "Zap", "iconColor": "#fff",
                    "nodes": [{
                        "id": "t", "type": "trigger", "label": "T",
                        "config": { "triggerType": "manual" }, "position": { "x": 0, "y": 0 }
                    }],
                    "edges": [], "enabled": true
                });
                if let Some(workspace) = workspace {
                    w["workspaceId"] = json!(workspace);
                }
                w
            })
            .collect();
        json!({
            "version": 1,
            "defaults": { "shell": "bash", "fontSize": 14, "theme": "dark" },
            "projects": [], "workflows": workflows
        })
    }

    fn empty_config() -> Value {
        json!({
            "version": 1,
            "defaults": { "shell": "bash", "fontSize": 14, "theme": "dark" },
            "projects": []
        })
    }

    fn bare_run(workflow: &str, started: &str) -> Value {
        json!({
            "workflowId": workflow, "runId": format!("{workflow}:{started}"),
            "startedAt": started, "status": "success", "nodeStates": []
        })
    }

    #[test]
    fn runs_of_every_workflow_come_newest_first_with_their_names() {
        let mut store = memory();
        save_config(
            &mut store,
            config_with_workflows(&[("wf-a", "Alpha", None), ("wf-b", "Beta", None)]),
            &[],
        );
        save(
            &mut store,
            json!({
                "workflowId": "wf-a", "runId": "wf-a:2026-04-20T10:00:00Z",
                "startedAt": "2026-04-20T10:00:00Z", "completedAt": "2026-04-20T10:00:05Z",
                "status": "success", "nodeStates": [{ "nodeId": "n", "status": "success" }]
            }),
        );
        save(
            &mut store,
            json!({
                "workflowId": "wf-b", "runId": "wf-b:2026-04-20T10:01:00Z",
                "startedAt": "2026-04-20T10:01:00Z", "completedAt": "2026-04-20T10:01:09Z",
                "status": "error", "nodeStates": [{ "nodeId": "n", "status": "error" }]
            }),
        );
        let listed = all(&mut store, Value::Null, 50);
        assert_eq!(workflow_ids(&listed), ["wf-b", "wf-a"]);
        assert_eq!(
            ids(&Value::Array(listed), "workflowName"),
            ["Beta", "Alpha"]
        );
    }

    #[test]
    fn a_workspace_narrows_to_its_workflows() {
        let mut store = memory();
        save_config(
            &mut store,
            config_with_workflows(&[
                ("wf-personal", "P", Some("personal")),
                ("wf-team", "T", Some("team")),
            ]),
            &[],
        );
        save(&mut store, bare_run("wf-personal", "2026-04-20T10:00:00Z"));
        save(&mut store, bare_run("wf-team", "2026-04-20T10:00:30Z"));
        assert_eq!(
            workflow_ids(&all(&mut store, json!("personal"), 50)),
            ["wf-personal"]
        );
        assert_eq!(
            workflow_ids(&all(&mut store, json!("team"), 50)),
            ["wf-team"]
        );
    }

    #[test]
    fn the_limit_is_honoured_and_clamped_to_500() {
        let mut store = memory();
        save_config(
            &mut store,
            config_with_workflows(&[("wf-x", "X", None)]),
            &[],
        );
        for i in 0..5 {
            save(
                &mut store,
                bare_run("wf-x", &format!("2026-04-20T10:0{i}:00Z")),
            );
        }
        assert_eq!(all(&mut store, Value::Null, 2).len(), 2);
        assert_eq!(all(&mut store, Value::Null, 99999).len(), 5);
    }

    #[test]
    fn the_trigger_task_comes_back_and_an_orphaned_run_survives() {
        let mut store = memory();
        save_config(
            &mut store,
            config_with_workflows(&[("wf-orphan", "O", None)]),
            &[],
        );
        let mut run = bare_run("wf-orphan", "2026-04-20T10:00:00Z");
        run["triggerTaskId"] = json!("task-7");
        save(&mut store, run);
        // The workflow goes from the config; its run row stays.
        save_config(&mut store, empty_config(), &[]);
        let listed = all(&mut store, Value::Null, 50);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["triggerTaskId"], "task-7");
        assert!(lacks(&listed[0], "workflowName"));
    }

    #[test]
    fn orphaned_runs_are_left_out_of_a_workspace_listing() {
        let mut store = memory();
        save_config(
            &mut store,
            config_with_workflows(&[("wf-orphan", "O", Some("team"))]),
            &[],
        );
        save(&mut store, bare_run("wf-orphan", "2026-04-20T10:00:00Z"));
        save_config(&mut store, empty_config(), &[]);
        assert!(all(&mut store, json!("personal"), 50).is_empty());
        assert!(all(&mut store, json!("team"), 50).is_empty());
        assert_eq!(all(&mut store, Value::Null, 50).len(), 1);
    }
}

mod inputs {
    use super::*;

    fn run_with(id: &str, workflow: &str, inputs: Option<Value>) -> Value {
        let mut run = json!({
            "runId": id, "workflowId": workflow, "startedAt": "2026-04-20T10:00:00Z",
            "status": "success", "nodeStates": [{ "nodeId": "n1", "status": "success" }]
        });
        if let Some(inputs) = inputs {
            run["inputs"] = inputs;
        }
        run
    }

    #[test]
    fn the_values_a_manual_run_started_with_round_trip() {
        let mut store = memory();
        let inputs = json!({ "issue": "gh-42", "count": 3, "item": { "number": 7 } });
        save(
            &mut store,
            run_with("run-inputs-1", "wf-inputs", Some(inputs.clone())),
        );
        assert_eq!(first_run(&mut store, "wf-inputs")["inputs"], inputs);
    }

    #[test]
    fn a_non_object_inputs_blob_is_dropped_rather_than_read_as_numeric_keys() {
        let mut store = memory();
        save(
            &mut store,
            run_with("run-inputs-3", "wf-bad", Some(json!(["a", "b"]))),
        );
        assert!(lacks(&first_run(&mut store, "wf-bad"), "inputs"));
    }

    #[test]
    fn a_run_without_inputs_has_none() {
        let mut store = memory();
        save(&mut store, run_with("run-inputs-2", "wf-none", None));
        assert!(lacks(&first_run(&mut store, "wf-none"), "inputs"));
    }
}

mod step_results {
    use super::*;

    fn with_result(overrides: Value) -> Value {
        let mut node = json!({
            "nodeId": "review-1", "status": "success", "output": "2",
            "structuredOutput": { "approved": true, "blocking": [], "main_story": "Meta open-weights" },
            "iteration": 2
        });
        for (key, value) in overrides.as_object().expect("an object") {
            node[key] = value.clone();
        }
        json!({
            "workflowId": "wf-results", "runId": "wf-results:2026-08-10T10:00:00Z",
            "startedAt": "2026-08-10T10:00:00Z", "status": "success", "nodeStates": [node]
        })
    }

    fn node(store: &mut Store, workflow: &str, index: usize) -> Value {
        runs(store, workflow, 10)[0]["nodeStates"][index].clone()
    }

    #[test]
    fn output_structured_output_and_iteration_round_trip() {
        let mut store = memory();
        save(&mut store, with_result(json!({})));
        let node = node(&mut store, "wf-results", 0);
        assert_eq!(node["output"], "2");
        assert_eq!(
            node["structuredOutput"],
            json!({ "approved": true, "blocking": [], "main_story": "Meta open-weights" })
        );
        assert_eq!(node["iteration"], 2);
    }

    #[test]
    fn a_nested_typed_value_stays_walkable() {
        let mut store = memory();
        save(
            &mut store,
            with_result(json!({
                "structuredOutput": { "issue": { "number": 42, "url": "https://example/42" } }
            })),
        );
        assert_eq!(
            node(&mut store, "wf-results", 0)["structuredOutput"]["issue"]["number"],
            42
        );
    }

    #[test]
    fn a_step_that_produced_none_has_none() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "workflowId": "wf-bare", "runId": "wf-bare:2026-08-10T10:00:00Z",
                "startedAt": "2026-08-10T10:00:00Z", "status": "success",
                "nodeStates": [{ "nodeId": "plain", "status": "success" }]
            }),
        );
        let node = node(&mut store, "wf-bare", 0);
        assert!(lacks(&node, "structuredOutput"));
        assert!(lacks(&node, "iteration"));
    }

    #[test]
    fn an_array_typed_output_degrades_to_none_rather_than_failing_the_history() {
        let mut store = memory();
        save(&mut store, with_result(json!({ "structuredOutput": [] })));
        let run = &runs(&mut store, "wf-results", 10)[0];
        assert_eq!(run["nodeStates"].as_array().map(Vec::len), Some(1));
        assert!(lacks(&run["nodeStates"][0], "structuredOutput"));
    }

    #[test]
    fn one_unreadable_typed_output_leaves_the_rest_of_the_run() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "workflowId": "wf-mixed", "runId": "wf-mixed:2026-08-10T10:00:00Z",
                "startedAt": "2026-08-10T10:00:00Z", "status": "success",
                "nodeStates": [
                    { "nodeId": "bad", "status": "success", "structuredOutput": "nope" },
                    { "nodeId": "good", "status": "success", "structuredOutput": { "ok": true } }
                ]
            }),
        );
        let run = &runs(&mut store, "wf-mixed", 10)[0];
        assert_eq!(run["nodeStates"].as_array().map(Vec::len), Some(2));
        assert!(lacks(&run["nodeStates"][0], "structuredOutput"));
        assert_eq!(
            run["nodeStates"][1]["structuredOutput"],
            json!({ "ok": true })
        );
    }
}

mod waiting_gates {
    use super::*;

    /// A run of `workflow` with one step, `gate`, in `node_status`.
    fn run(workflow: &str, status: &str, node_status: &str) -> Value {
        let digit = &workflow[workflow.len() - 1..];
        json!({
            "runId": format!("{workflow}:2026-04-20T10:00:0{digit}Z"),
            "workflowId": workflow,
            "startedAt": format!("2026-04-20T10:00:0{digit}Z"),
            "status": status,
            "nodeStates": [{ "nodeId": "gate", "status": node_status }]
        })
    }

    fn waiting(store: &mut Store) -> Vec<Value> {
        call(store, "listRunsWithWaitingGates", json!([null]))
            .as_array()
            .cloned()
            .expect("a list")
    }

    #[test]
    fn none_when_no_run_has_a_waiting_step() {
        let mut store = memory();
        save(&mut store, run("wf-1", "success", "success"));
        assert!(waiting(&mut store).is_empty());
    }

    #[test]
    fn a_run_with_a_waiting_step_is_listed() {
        let mut store = memory();
        save(&mut store, run("wf-1", "running", "waiting"));
        save(&mut store, run("wf-2", "success", "success"));
        let listed = waiting(&mut store);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["workflowId"], "wf-1");
        assert_eq!(listed[0]["nodeStates"][0]["status"], "waiting");
    }

    #[test]
    fn several_waiting_runs_each_come_with_their_steps() {
        let mut store = memory();
        for wf in ["wf-1", "wf-2", "wf-3"] {
            save(&mut store, run(wf, "running", "waiting"));
        }
        let listed = waiting(&mut store);
        let mut workflows = ids(&Value::Array(listed.clone()), "workflowId");
        workflows.sort();
        assert_eq!(workflows, ["wf-1", "wf-2", "wf-3"]);
        assert!(listed
            .iter()
            .all(|r| r["nodeStates"][0]["status"] == "waiting"));
    }

    #[test]
    fn approval_and_agent_metadata_survive_the_round_trip() {
        let mut store = memory();
        save(
            &mut store,
            json!({
                "runId": "wf-9:2026-04-20T10:00:00Z", "workflowId": "wf-9",
                "startedAt": "2026-04-20T10:00:00Z", "status": "running",
                "nodeStates": [{
                    "nodeId": "gate", "status": "waiting", "startedAt": "2026-04-20T10:00:00Z",
                    "completedAt": "2026-04-20T10:00:05Z", "sessionId": "s1", "error": "none",
                    "logs": "log line", "taskId": "t1", "agentSessionId": "as1",
                    "agentType": "claude", "projectName": "p", "projectPath": "/p",
                    "approvedAt": "2026-04-20T10:00:10Z"
                }],
                "triggerTaskId": "trig-1", "completedAt": "2026-04-20T10:00:10Z"
            }),
        );
        let got = &waiting(&mut store)[0];
        let ns = &got["nodeStates"][0];
        for (key, value) in [
            ("status", "waiting"),
            ("agentType", "claude"),
            ("projectName", "p"),
            ("projectPath", "/p"),
            ("approvedAt", "2026-04-20T10:00:10Z"),
            ("agentSessionId", "as1"),
            ("taskId", "t1"),
            ("logs", "log line"),
        ] {
            assert_eq!(ns[key], value, "{key}");
        }
        assert_eq!(got["completedAt"], "2026-04-20T10:00:10Z");
        assert_eq!(got["triggerTaskId"], "trig-1");
    }

    #[test]
    fn a_step_waiting_for_a_sign_in_still_says_so() {
        let mut store = memory();
        let mut r = run("wf-9", "running", "waiting");
        r["nodeStates"] =
            json!([{ "nodeId": "draft", "status": "waiting", "waitingFor": "signIn" }]);
        save(&mut store, r);
        assert_eq!(
            waiting(&mut store)[0]["nodeStates"][0]["waitingFor"],
            "signIn"
        );
    }

    #[test]
    fn the_definition_a_run_started_with_comes_back_with_it() {
        let mut store = memory();
        let definition = json!({
            "id": "wf-9", "name": "Notes",
            "nodes": [{
                "id": "gate", "type": "approval", "label": "Gate", "config": {},
                "position": { "x": 0, "y": 0 }
            }],
            "edges": []
        });
        let mut r = run("wf-9", "running", "waiting");
        r["definition"] = definition.clone();
        save(&mut store, r);
        assert_eq!(waiting(&mut store)[0]["definition"], definition);
    }
}
