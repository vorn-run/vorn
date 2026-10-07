//! Every case the TypeScript engine was recorded on
//! (`tests/fixtures/js-reference/workflow-engine.json`), replayed here: the
//! same workflow, configuration and host replies, the same steps taken, and
//! the runs, calls and answers compared after times, generated ids and the
//! timeline's seconds are normalized as the recording normalized them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Map, Value};
use vorn_protocol::WorkflowExecution;
use vorn_work::model::{Context, Workflow};
use vorn_workflow::{Answer, Decision, Engine, Host, Options, Source};

mod common;
use common::Fake;

fn normalize(
    value: &Value,
    runs: &HashMap<String, String>,
    times: &Regex,
    seconds: &Regex,
) -> Value {
    match value {
        Value::String(s) if times.is_match(s) => json!("<time>"),
        Value::String(s) if runs.contains_key(s) => json!(runs[s]),
        Value::String(s) => json!(seconds.replace_all(s, "[+s]")),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| normalize(v, runs, times, seconds))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                match (k.as_str(), v) {
                    ("definition", _) => {}
                    ("viewToken", Value::String(_)) => {
                        out.insert(k.clone(), json!("<token>"));
                    }
                    ("runId", Value::String(s)) if !runs.contains_key(s) => {
                        out.insert(k.clone(), json!("<id>"));
                    }
                    _ => {
                        out.insert(k.clone(), normalize(v, runs, times, seconds));
                    }
                }
            }
            Value::Object(out)
        }
        other => vorn_work::js_numbers(other.clone()),
    }
}

async fn play(case: &Value) -> Value {
    let input = &case["input"];
    let workflow_json = input["workflow"].clone();
    let workflow = Workflow::from_json(&workflow_json).unwrap();
    let mut config = input["config"].clone();
    config["workflows"] = json!([workflow_json]);
    let dir = common::temp_dir();
    let replies = input
        .get("replies")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let fake = Fake::new(config, replies, dir.clone());
    let engine = Engine::new(fake.clone());
    let mut answers: Vec<Value> = Vec::new();
    let mut background: Vec<tokio::task::JoinHandle<Option<Value>>> = Vec::new();
    let run_at = |i: usize| fake.world().order[i].clone();
    for step in input["steps"].as_array().unwrap() {
        let bg = step.get("background") == Some(&Value::Bool(true));
        if let Some(run) = step.get("run") {
            let context = run.get("context").and_then(Context::from_json);
            let options = Options {
                source: Some(
                    if run.get("source").and_then(Value::as_str) == Some("scheduler") {
                        Source::Scheduler
                    } else {
                        Source::Manual
                    },
                ),
                target: run
                    .get("targetNodeId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            let e = engine.clone();
            let w = workflow.clone();
            let task = tokio::spawn(async move {
                Some(match e.execute(&w, context, options).await {
                    Ok(started) => {
                        let id = started.run.run_id.clone();
                        started.finish().await;
                        json!({ "run": id })
                    }
                    Err(err) => json!({ "error": err }),
                })
            });
            if bg {
                background.push(task);
            } else if let Some(a) = task.await.unwrap() {
                answers.push(a);
            }
            continue;
        }
        if let Some(r) = step.get("retry") {
            engine.flush().await;
            let id = run_at(r["run"].as_u64().unwrap() as usize);
            let failed = fake.world().saved[&id].clone();
            engine
                .retry(&workflow, &failed)
                .await
                .unwrap()
                .finish()
                .await;
        } else if let Some(r) = step.get("rerun") {
            engine.flush().await;
            let id = run_at(r["run"].as_u64().unwrap() as usize);
            let earlier = fake.world().saved[&id].clone();
            engine
                .rerun(&workflow, &earlier)
                .await
                .unwrap()
                .finish()
                .await;
        } else if let Some(g) = step.get("gate") {
            engine.flush().await;
            let run_id = run_at(g["run"].as_u64().unwrap() as usize);
            let node = g["nodeId"].as_str().unwrap();
            let decision = Decision::parse(g["decision"].as_str().unwrap()).unwrap();
            let comment = g.get("comment").and_then(Value::as_str).map(str::to_owned);
            let edited = g.get("edited").and_then(Value::as_str).map(str::to_owned);
            let comments = g.get("comments").cloned();
            let answer = Answer {
                decision,
                comment,
                edited,
                comments,
            };
            let answer = match engine.check_gate(&run_id, node, &answer).await {
                Err(None) => json!({ "accepted": false }),
                Err(Some(reason)) => json!({ "accepted": false, "reason": reason }),
                Ok(()) => {
                    engine.apply_gate_decision(&run_id, node, answer).await;
                    json!({ "accepted": true })
                }
            };
            answers.push(answer);
        } else if let Some(s) = step.get("stop") {
            engine.flush().await;
            engine
                .stop(&run_at(s["run"].as_u64().unwrap() as usize))
                .await;
        } else if let Some(s) = step.get("signIn") {
            engine.flush().await;
            let parked: Vec<WorkflowExecution> = {
                let w = fake.world();
                w.order
                    .iter()
                    .map(|id| w.saved[id].clone())
                    .filter(|r| r.node_states.iter().any(|n| n.status.0 == "waiting"))
                    .collect()
            };
            engine
                .resume_sign_in_waits(s["connectionId"].as_str().unwrap(), parked)
                .await;
        } else if let Some(ms) = step.get("sleep") {
            tokio::time::sleep(Duration::from_millis(ms.as_u64().unwrap())).await;
        } else if let Some(stored) = step.get("reconcile") {
            let run: WorkflowExecution = serde_json::from_value(stored.clone()).unwrap();
            fake.save_run(run.clone()).await;
            engine
                .reconcile(vec![run], std::slice::from_ref(&workflow))
                .await;
        }
    }
    for task in background {
        if let Some(a) = task.await.unwrap() {
            answers.insert(0, a);
        }
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    engine.flush().await;
    let w = fake.world();
    let runs_map: HashMap<String, String> = w
        .order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), format!("run-{}", i + 1)))
        .collect();
    let runs: Vec<Value> = w
        .order
        .iter()
        .map(|id| serde_json::to_value(&w.saved[id]).unwrap())
        .collect();
    let out = json!({ "runs": runs, "calls": w.calls, "answers": answers });
    let times = Regex::new(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$").unwrap();
    let seconds = Regex::new(r"\[\+\d+(\.\d+)?s\]").unwrap();
    let _ = std::fs::remove_dir_all(dir);
    normalize(&out, &runs_map, &times, &seconds)
}

/// Accepted difference: a run id inside a longer string (a retry's
/// fingerprint names the run it retries) is a fresh id on each side.
fn ids_inside_text(value: &Value) -> Value {
    let uuid = Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    match value {
        Value::String(s) => json!(uuid.replace_all(s, "<uuid>")),
        Value::Array(items) => Value::Array(items.iter().map(ids_inside_text).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), ids_inside_text(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn cases() -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/js-reference/workflow-engine.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Where two values first differ, for the failure message.
fn first_difference(a: &Value, b: &Value, at: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().find_map(|k| match (x.get(k), y.get(k)) {
                (Some(p), Some(q)) => first_difference(p, q, &format!("{at}/{k}")),
                (p, q) => Some(format!("{at}/{k}: {p:?} vs {q:?}")),
            })
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!(
                    "{at}: {} items vs {}\n  got {a}\n  want {b}",
                    x.len(),
                    y.len()
                ));
            }
            x.iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (p, q))| first_difference(p, q, &format!("{at}/{i}")))
        }
        _ if a == b => None,
        _ => Some(format!("{at}: {a} vs {b}")),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_recorded_case_runs_as_the_typescript_engine_ran_it() {
    let only = std::env::var("CASE").ok();
    let mut failures = Vec::new();
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let Ok(got) = tokio::time::timeout(Duration::from_secs(10), play(&case)).await else {
            failures.push(format!("{name}: did not finish"));
            continue;
        };
        if let Some(diff) = first_difference(
            &ids_inside_text(&got),
            &ids_inside_text(&case["expected"]),
            "",
        ) {
            failures.push(format!("{name}: {diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} case(s) differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
