//! The engine's paths the recorded cases do not take: what a task or a
//! session coming back starts, schedules and connector items, which agent a
//! step runs, gates left waiting by an earlier process, and a lease taken over.

use std::time::Duration;

use serde_json::{json, Map, Value};
use vorn_protocol::WorkflowExecution;
use vorn_work::model::{Context, Workflow};
use vorn_workflow::{Host, Options, Source};

mod common;
use common::Fake;

fn workflow(id: &str, trigger: Value, step: Value) -> Value {
    json!({
        "id": id, "name": id, "icon": "x", "iconColor": "#000", "enabled": true,
        "nodes": [
            { "id": "t", "type": "trigger", "label": "T", "config": trigger },
            step
        ],
        "edges": [{ "id": "e", "source": "t", "target": "s" }]
    })
}

fn script(content: &str) -> Value {
    json!({ "id": "s", "type": "script", "label": "S", "slug": "s", "config": { "scriptType": "bash", "scriptContent": content } })
}

fn agent(config: Value) -> Value {
    let mut c =
        json!({ "projectName": "p", "projectPath": "/p", "headless": true, "prompt": "go" });
    for (k, v) in config.as_object().unwrap() {
        c[k] = v.clone();
    }
    json!({ "id": "s", "type": "launchAgent", "label": "S", "slug": "s", "config": c })
}

fn host(workflows: Vec<Value>, replies: Value) -> Fake {
    let config = json!({
        "defaults": { "defaultAgent": "beta" },
        "projects": [{ "name": "p", "path": "/p" }],
        "tasks": [
            { "id": "t1", "projectName": "p", "title": "Mine", "description": "", "status": "todo", "order": 1, "assignedAgent": "gamma" }
        ],
        "workflows": workflows,
    });
    Fake::new(
        config,
        replies.as_object().cloned().unwrap_or_default(),
        common::temp_dir(),
    )
}

/// Waits for `count` runs to have finished.
async fn finished(fake: &Fake, count: usize) -> Vec<WorkflowExecution> {
    for _ in 0..400 {
        let runs = fake.runs();
        if runs.len() >= count && runs.iter().all(|r| r.status != "running") {
            return runs;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no {count} finished run(s): {:?}", fake.runs());
}

/// Gives anything that would start a run the time to.
async fn quiet() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_starts_the_workflows_watching_for_it_and_no_others() {
    let fake = host(
        vec![
            workflow(
                "created",
                json!({ "triggerType": "taskCreated" }),
                script("echo {{task.title}}"),
            ),
            workflow(
                "elsewhere",
                json!({ "triggerType": "taskCreated", "projectFilter": "q" }),
                script("x"),
            ),
            workflow(
                "to-done",
                json!({ "triggerType": "taskStatusChanged", "toStatus": "done" }),
                script("x"),
            ),
            workflow(
                "moved",
                json!({ "triggerType": "taskStatusChanged", "fromStatus": "todo", "toStatus": "in_progress", "projectFilter": "p" }),
                script("echo {{trigger.fromStatus}}>{{trigger.toStatus}}"),
            ),
            {
                let mut off = workflow("off", json!({ "triggerType": "taskCreated" }), script("x"));
                off["enabled"] = json!(false);
                off
            },
        ],
        json!({}),
    );
    let engine = common::engine(fake.clone());
    let task = json!({ "id": "t1", "title": "Mine", "projectName": "p" });
    let fire = |trigger: serde_json::Value| {
        let engine = engine.clone();
        async move {
            // Tasks fires its triggers through `workflow.fire`, with these arguments.
            let Some(args) = vorn_tasks::triggers::fire_args(&trigger) else {
                return;
            };
            let context = vorn_work::model::Context::from_json(&args["context"]).unwrap();
            let fields = args["fields"].as_object().unwrap().clone();
            engine
                .fire(args["trigger"].as_str().unwrap(), &fields, context)
                .await;
        }
    };
    fire(json!({ "kind": "taskCreated", "task": task })).await;
    fire(json!({ "kind": "taskStatusChanged", "task": task, "from": "todo", "to": "in_progress" }))
        .await;
    fire(json!({ "kind": "taskStatusChanged", "task": task, "from": "todo", "to": "todo" })).await;
    let runs = finished(&fake, 2).await;
    quiet().await;
    let mut started: Vec<String> = fake.runs().iter().map(|r| r.workflow_id.clone()).collect();
    started.sort();
    assert_eq!(started, ["created", "moved"]);
    assert!(runs
        .iter()
        .all(|r| r.trigger_task_id.as_deref() == Some("t1")));
    let contents: Vec<String> = fake
        .calls_of("script:execute")
        .iter()
        .map(|c| c["scriptContent"].as_str().unwrap().to_owned())
        .collect();
    assert!(contents.contains(&"echo Mine".to_owned()));
    assert!(contents.contains(&"echo todo>in_progress".to_owned()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_coming_back_starts_what_asked_for_its_kind_of_return() {
    let fake = host(
        vec![
            workflow(
                "cold",
                json!({ "triggerType": "sessionRestored" }),
                script("echo {{context.projectPath}} {{trigger.restore}}"),
            ),
            workflow(
                "any",
                json!({ "triggerType": "sessionRestored", "restore": "any", "concurrency": "unbounded" }),
                script("echo any"),
            ),
            workflow(
                "other",
                json!({ "triggerType": "sessionRestored", "projectFilter": "q" }),
                script("x"),
            ),
        ],
        json!({}),
    );
    let engine = common::engine(fake.clone());
    let session =
        json!({ "id": "sess-9", "projectName": "p", "projectPath": "/p", "displayName": "Card" });
    vorn_workflows::triggers::session_restored(&engine, &session, "warm", None).await;
    let warm = finished(&fake, 1).await;
    assert_eq!(warm[0].workflow_id, "any");
    vorn_workflows::triggers::session_restored(
        &engine,
        &session,
        "cold",
        Some(json!({ "rebooted": true })),
    )
    .await;
    let runs = finished(&fake, 3).await;
    let cold = runs.iter().find(|r| r.workflow_id == "cold").unwrap();
    assert_eq!(
        cold.trigger_session,
        Some(json!({ "id": "sess-9", "label": "Card", "restore": "cold" }))
    );
    assert!(fake
        .calls_of("script:execute")
        .iter()
        .any(|c| c["scriptContent"] == "echo /p cold"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_schedule_runs_with_its_inputs_and_says_so_in_the_log() {
    let fake = host(
        vec![workflow(
            "nightly",
            json!({ "triggerType": "recurring", "cron": "* * * * *" }),
            script("echo {{inputs.who}}"),
        )],
        json!({}),
    );
    let engine = common::engine(fake.clone());
    let mut inputs = Map::new();
    inputs.insert("who".into(), json!("me"));
    engine.run_scheduled("nightly", Some(inputs)).await;
    engine.run_scheduled("gone", None).await;
    finished(&fake, 1).await;
    quiet().await;
    assert_eq!(fake.runs().len(), 1);
    assert_eq!(
        fake.calls_of("script:execute")[0]["scriptContent"],
        "echo me"
    );
    assert_eq!(
        fake.calls_of("workflow:executionComplete")[0]["source"],
        "scheduler"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connector_item_runs_once_and_one_whose_workflow_is_gone_waits() {
    let fake = host(
        vec![workflow(
            "poll",
            json!({ "triggerType": "connectorPoll", "cron": "* * * * *" }),
            script("echo {{connectorItem.title}}"),
        )],
        json!({}),
    );
    let engine = common::engine(fake.clone());
    let item = json!({ "inboxId": 3, "inboxLeaseToken": "lease-3", "connectionId": "c", "connectorId": "x", "externalId": "e", "title": "Item", "raw": {} });
    engine.run_connector_item("poll", item.clone(), None).await;
    let runs = finished(&fake, 1).await;
    assert_eq!(
        runs[0].connector_inbox_disposition.as_deref(),
        Some("processed")
    );
    assert_eq!(
        fake.calls_of("connector:inboxComplete"),
        [json!({ "id": 3, "leaseToken": "lease-3", "disposition": "processed" })]
    );

    engine
        .run_connector_item(
            "deleted",
            json!({ "inboxId": 4, "inboxLeaseToken": "lease-4" }),
            None,
        )
        .await;
    assert_eq!(
        fake.calls_of("connector:inboxComplete")[1],
        json!({ "id": 4, "leaseToken": "lease-4", "disposition": "defer" })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_item_hands_its_new_lease_to_the_run_already_going() {
    let fake = host(
        vec![workflow(
            "poll",
            json!({ "triggerType": "connectorPoll", "cron": "* * * * *" }),
            script("x"),
        )],
        json!({}),
    );
    let engine = common::engine(fake.clone());
    let running: WorkflowExecution = serde_json::from_value(json!({
        "runId": "r1", "workflowId": "poll", "startedAt": "2026-01-01T00:00:00.000Z", "status": "running",
        "connectorInboxId": 5, "connectorInboxLeaseToken": "old",
        "nodeStates": [{ "nodeId": "t", "status": "success" }, { "nodeId": "s", "status": "running", "sessionId": "live" }]
    }))
    .unwrap();
    let item = json!({ "inboxId": 5, "inboxLeaseToken": "new", "connectionId": "c", "connectorId": "x", "externalId": "e", "title": "I", "raw": {} });
    engine.run_connector_item("poll", item, Some(running)).await;
    let stored = engine.snapshot("r1").await.unwrap();
    assert_eq!(stored.connector_inbox_lease_token.as_deref(), Some("new"));
    assert_eq!(stored.status, "running");
    // The agent was asked after, and still runs: nothing is settled.
    assert!(fake.calls_of("connector:inboxComplete").is_empty());
    assert_eq!(fake.calls_of("sessionEvent:listBySession").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn which_agent_a_step_runs() {
    let cases = [
        (json!({ "agentType": "acme" }), None, "acme"),
        (
            json!({ "agentType": "fromTask" }),
            Some(json!({ "id": "x", "assignedAgent": "delta" })),
            "delta",
        ),
        (
            json!({ "agentType": "fromTask", "taskId": "t1" }),
            None,
            "gamma",
        ),
        (json!({ "agentType": "fromTask" }), None, "beta"),
    ];
    for (config, task, want) in cases {
        let fake = host(
            vec![workflow(
                "a",
                json!({ "triggerType": "manual" }),
                agent(config.clone()),
            )],
            json!({}),
        );
        let engine = common::engine(fake.clone());
        let wf = Workflow::from_json(&fake.world().config["workflows"][0]).unwrap();
        let context = task.map(|t| Context {
            task: Some(t),
            ..Context::default()
        });
        engine
            .execute(&wf, context, Options::default())
            .await
            .unwrap()
            .finish()
            .await;
        let created = fake.calls_of("headless:create");
        assert_eq!(created[0]["agentType"], want, "{config}");
    }

    let fake = host(
        vec![workflow(
            "a",
            json!({ "triggerType": "manual" }),
            agent(json!({ "agentType": "fromTask" })),
        )],
        json!({}),
    );
    fake.world().config["defaults"] = json!({});
    let engine = common::engine(fake.clone());
    let wf = Workflow::from_json(&fake.world().config["workflows"][0]).unwrap();
    engine
        .execute(&wf, None, Options::default())
        .await
        .unwrap()
        .finish()
        .await;
    assert_eq!(fake.calls_of("headless:create")[0]["agentType"], "acme");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gate_left_waiting_by_an_earlier_process_still_times_out() {
    let gate = json!({ "id": "s", "type": "approval", "label": "Gate", "slug": "s", "config": { "timeoutMs": 60_000 } });
    let fake = host(
        vec![workflow("g", json!({ "triggerType": "manual" }), gate)],
        json!({}),
    );
    let engine = common::engine(fake.clone());
    let wf = Workflow::from_json(&fake.world().config["workflows"][0]).unwrap();
    let parked: WorkflowExecution = serde_json::from_value(json!({
        "runId": "r1", "workflowId": "g", "startedAt": "2026-01-01T00:00:00.000Z", "status": "running",
        "nodeStates": [{ "nodeId": "t", "status": "success" }, { "nodeId": "s", "status": "waiting", "startedAt": "2026-01-01T00:00:00.000Z", "round": 1 }]
    }))
    .unwrap();
    fake.save_run(parked.clone()).await;
    engine
        .rearm_gates(vec![parked], std::slice::from_ref(&wf))
        .await;
    let runs = finished(&fake, 1).await;
    assert_eq!(runs[0].status, "error");
    assert_eq!(
        runs[0].node_states[1].error.as_deref(),
        Some("Approval timed out after 60000ms")
    );
    assert_eq!(runs[0].node_states[1].rejected_at, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rerun_and_a_retry_carry_the_context_without_the_lease() {
    let fake = host(
        vec![workflow(
            "w",
            json!({ "triggerType": "webhook" }),
            script("echo {{trigger.body.n}} {{connectorItem.title}}"),
        )],
        json!({}),
    );
    let engine = common::engine(fake.clone());
    let earlier: WorkflowExecution = serde_json::from_value(json!({
        "runId": "r0", "workflowId": "w", "startedAt": "2026-01-01T00:00:00.000Z", "status": "error",
        "connectorItem": { "inboxId": 9, "inboxLeaseToken": "l", "connectorId": "webhook", "connectionId": "webhook:w", "externalId": "e", "title": "Hook", "raw": { "body": { "n": 4 } } },
        "nodeStates": []
    }))
    .unwrap();
    let context = engine.context_from_run(&earlier).await.unwrap();
    let item = context.connector_item.as_ref().unwrap();
    assert!(item.get("inboxId").is_none() && item.get("inboxLeaseToken").is_none());
    assert_eq!(
        context.trigger,
        Some(json!({ "type": "webhook", "body": { "n": 4 } }))
    );
    let wf = Workflow::from_json(&fake.world().config["workflows"][0]).unwrap();
    engine.rerun(&wf, &earlier).await.unwrap().finish().await;
    assert_eq!(
        fake.calls_of("script:execute")[0]["scriptContent"],
        "echo 4 Hook"
    );
    assert!(fake.calls_of("connector:inboxComplete").is_empty());
    assert_eq!(
        fake.calls_of("workflow:executionComplete")[0]["source"],
        json!(Source::Manual.as_str())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claims_and_releases_through_the_engine() {
    let engine = common::engine(host(vec![], json!({})));
    let first = engine.claim("w", Some("p"), None);
    assert!(first.granted);
    assert_eq!(engine.claim("w", Some("p"), None).run_id, first.run_id);
    engine.release("w", Some("p"), &first.run_id);
    assert!(engine.claim("w", Some("p"), None).granted);
    assert_eq!(engine.active_count(), 0);
    let unknown: Value = serde_json::to_value(engine.snapshot("nope").await).unwrap();
    assert_eq!(unknown, Value::Null);
}

#[tokio::test]
async fn a_type_stays_with_the_extension_that_declared_it_first() {
    use vorn_workflow::{NodeType, Resolved, Runner, TriggerType};
    let dir = common::temp_dir();
    let engine = common::engine(Fake::new(json!({}), Map::new(), dir));
    let r = engine.registry();
    let node = NodeType {
        id: "script".into(),
        provider: "someone.else".into(),
        provider_name: "Someone".into(),
        doc: json!({}),
    };
    assert!(!r.add_node(node, Some(Runner::Call)));
    let trigger = TriggerType {
        id: "taskStatusChanged".into(),
        provider: "someone.else".into(),
        provider_name: "Someone".into(),
        timing: None,
        fans_out: false,
        doc: json!({}),
    };
    assert!(!r.add_trigger(trigger, true));
    match r.resolve("script") {
        Resolved::Run(t, Runner::Local(_)) => assert_eq!(t.provider, vorn_workflows::ID),
        _ => panic!("script is no longer the built-in's"),
    }
    assert_eq!(
        r.trigger("taskStatusChanged").map(|t| t.provider),
        Some("vorn.tasks".to_owned())
    );
}

/// A script that prints `items` as JSON, a loop over them run in
/// parallel, its body, and a step after it.
fn fan_out(loop_config: Value) -> Value {
    let mut config = json!({ "mode": "forEach", "items": "{{steps.fetch.json.items}}", "bodyNodeIds": ["body"], "parallel": true });
    for (k, v) in loop_config.as_object().unwrap() {
        config[k] = v.clone();
    }
    json!({
        "id": "fan", "name": "fan", "icon": "x", "iconColor": "#000", "enabled": true,
        "nodes": [
            { "id": "t", "type": "trigger", "label": "T", "config": { "triggerType": "manual" } },
            { "id": "fetch", "type": "script", "label": "Fetch", "slug": "fetch", "config": { "scriptType": "bash", "scriptContent": "fetch" } },
            { "id": "each", "type": "loop", "label": "Each", "slug": "each", "config": config },
            { "id": "body", "type": "script", "label": "Body", "slug": "body", "config": { "scriptType": "bash", "scriptContent": "echo {{loop.item.n}} of {{loop.count}}" } },
            { "id": "after", "type": "script", "label": "After", "slug": "after", "config": { "scriptType": "bash", "scriptContent": "after {{steps.body.output}}" } }
        ],
        "edges": [
            { "id": "e1", "source": "t", "target": "fetch" },
            { "id": "e2", "source": "fetch", "target": "each" },
            { "id": "e3", "source": "each", "target": "body" },
            { "id": "e4", "source": "body", "target": "after" }
        ]
    })
}

/// Replies for the fetch and for each item, item 1 the slowest, so the items finish in reverse.
fn fan_replies(count: u64, fail: Option<u64>) -> Value {
    let items: Vec<Value> = (1..=count).map(|n| json!({ "n": n })).collect();
    let mut scripts = vec![json!({
        "when": { "scriptContent": "fetch" },
        "result": { "success": true, "output": format!("\u{1b}[0m{}\r\n", json!({ "items": items })) }
    })];
    for n in 1..=count {
        let ok = fail != Some(n);
        scripts.push(json!({
            "when": { "scriptContent": format!("echo {n} of {count}") },
            "delayMs": 20 + (count - n) * 15,
            "result": { "success": ok, "output": format!("item {n}"), "error": if ok { Value::Null } else { json!("broke") } }
        }));
    }
    json!({ "script:execute": scripts })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loop_in_parallel_runs_each_item_as_steps_of_its_own_at_most_its_limit_at_once() {
    let fake = host(
        vec![fan_out(json!({ "concurrency": 2 }))],
        fan_replies(5, None),
    );
    let engine = common::engine(fake.clone());
    engine.run_scheduled("fan", None).await;
    let run = finished(&fake, 1).await.remove(0);
    assert_eq!(run.status, "success", "{run:#?}");
    assert_eq!(fake.world().most_scripts, 2);
    let state = |id: &str| {
        run.node_states
            .iter()
            .find(|s| s.node_id == id)
            .unwrap_or_else(|| panic!("no state for {id}"))
    };
    for n in 1..=5u32 {
        let item = state(&format!("body#{n}"));
        assert_eq!(item.status.0, "success");
        assert_eq!(item.output.as_deref(), Some(format!("item {n}").as_str()));
        assert_eq!(item.iteration, Some(f64::from(n)));
    }
    // Kept in the list's order, though item 5 finished first.
    let each = state("each").structured_output.clone().unwrap();
    let items: Vec<Value> = each["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["item"]["n"].clone())
        .collect();
    assert_eq!(items, [1, 2, 3, 4, 5].map(|n| json!(n)));
    assert_eq!(each["outputs"][2], "item 3");
    assert_eq!(each["results"][0]["steps"]["body"]["output"], "item 1");
    assert_eq!(state("each").output.as_deref(), Some("5"));
    // The body step itself holds what the last item left, for the steps after.
    assert_eq!(state("body").output.as_deref(), Some("item 5"));
    assert!(fake
        .calls_of("script:execute")
        .iter()
        .any(|c| c["scriptContent"] == "after item 5"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_item_stops_the_items_not_yet_started() {
    let fake = host(
        vec![fan_out(json!({ "concurrency": 1 }))],
        fan_replies(4, Some(2)),
    );
    let engine = common::engine(fake.clone());
    engine.run_scheduled("fan", None).await;
    let run = finished(&fake, 1).await.remove(0);
    assert_eq!(run.status, "error");
    let state = |id: &str| run.node_states.iter().find(|s| s.node_id == id).unwrap();
    assert_eq!(state("body#1").status.0, "success");
    assert_eq!(state("body#2").status.0, "error");
    for id in ["body#3", "body#4"] {
        assert_eq!(state(id).status.0, "skipped");
        assert_eq!(
            state(id).error.as_deref(),
            Some("Skipped: an earlier item failed")
        );
    }
    assert_eq!(state("each").status.0, "error");
    assert_eq!(
        state("each").error.as_deref(),
        Some("\"Body\" failed on item 2")
    );
    assert_eq!(fake.calls_of("script:execute").len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loop_in_parallel_cannot_stop_on_a_condition() {
    let fake = host(
        vec![fan_out(
            json!({ "until": { "variable": "{{steps.body.output}}", "operator": "equals", "value": "x" } }),
        )],
        fan_replies(2, None),
    );
    let engine = common::engine(fake.clone());
    engine.run_scheduled("fan", None).await;
    let run = finished(&fake, 1).await.remove(0);
    let each = run.node_states.iter().find(|s| s.node_id == "each").unwrap();
    assert_eq!(each.status.0, "error");
    assert!(each
        .error
        .as_deref()
        .unwrap()
        .starts_with("A loop that runs in parallel goes through every item"));
}
