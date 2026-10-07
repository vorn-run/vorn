//! Every case the TypeScript engine was recorded on
//! (`tests/fixtures/js-reference/workflow-engine.json`), replayed here: the
//! same workflow, configuration and host replies, the same steps taken, and
//! the runs, calls and answers compared after times, generated ids and the
//! timeline's seconds are normalized as the recording normalized them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Map, Value};
use tokio::sync::broadcast;
use vorn_protocol::WorkflowExecution;
use vorn_work::model::{Context, Workflow};
use vorn_workflow::{Answer, Completion, Decision, Engine, Host, Note, Options, Source, TaskMove};

#[derive(Default)]
struct World {
    config: Value,
    calls: Vec<Value>,
    saved: HashMap<String, WorkflowExecution>,
    order: Vec<String>,
    replies: Map<String, Value>,
    used: HashMap<String, Vec<bool>>,
    sessions: u32,
    pages: HashMap<(String, String, u32), String>,
}

#[derive(Clone)]
struct Fake {
    world: Arc<Mutex<World>>,
    notes: broadcast::Sender<Note>,
    data_dir: PathBuf,
}

impl Fake {
    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    fn record(&self, method: &str, params: Value) {
        self.world()
            .calls
            .push(json!({ "method": method, "params": params }));
    }

    /// The first unused reply for `method` whose `when` matches.
    fn take(&self, method: &str, params: &Value) -> Option<Value> {
        let mut w = self.world();
        let list = w.replies.get(method)?.as_array()?.clone();
        let used = w
            .used
            .entry(method.to_owned())
            .or_insert_with(|| vec![false; list.len()]);
        for (i, reply) in list.iter().enumerate() {
            if used[i] {
                continue;
            }
            if let Some(Value::Object(when)) = reply.get("when") {
                if !when.iter().all(|(k, v)| params.get(k) == Some(v)) {
                    continue;
                }
            }
            used[i] = true;
            return Some(reply.clone());
        }
        None
    }

    /// Says each note after its pause, in order.
    fn later(&self, notes: Vec<(u64, Note)>) {
        let sender = self.notes.clone();
        tokio::spawn(async move {
            for (ms, note) in notes {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                let _ = sender.send(note);
            }
        });
    }
}

fn failed(reply: &Value) -> Option<String> {
    reply
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl Host for Fake {
    async fn config(&self) -> Option<Value> {
        Some(self.world().config.clone())
    }

    async fn save_run(&self, run: WorkflowExecution) {
        let mut w = self.world();
        if !w.saved.contains_key(&run.run_id) {
            w.order.push(run.run_id.clone());
        }
        w.saved.insert(run.run_id.clone(), run);
    }

    async fn load_run(&self, run_id: &str) -> Option<WorkflowExecution> {
        self.world().saved.get(run_id).cloned()
    }

    async fn runs_of(&self, workflow_id: &str) -> Vec<WorkflowExecution> {
        self.world()
            .saved
            .values()
            .filter(|r| r.workflow_id == workflow_id)
            .cloned()
            .collect()
    }

    fn publish(&self, _run: &WorkflowExecution) {}

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.record(method, params.clone());
        // An answer comes back on a later turn, as a promise's does.
        tokio::task::yield_now().await;
        match method {
            "headless:create" => {
                let reply = self
                    .take(method, &params)
                    .unwrap_or(json!({ "exitCode": 0 }));
                if let Some(e) = failed(&reply) {
                    return Err(e);
                }
                let id = {
                    let mut w = self.world();
                    w.sessions += 1;
                    format!("sess-{}", w.sessions)
                };
                let mut session = json!({ "id": id, "pid": 4242, "launchCommand": "agent -p" });
                for (k, v) in reply
                    .get("session")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                {
                    session[k] = v.clone();
                }
                let code = reply.get("exitCode").and_then(Value::as_i64);
                if reply.get("exitBeforeId") == Some(&Value::Bool(true)) {
                    let _ = self.notes.send(Note::HeadlessExit {
                        id,
                        code: code.unwrap_or(0),
                    });
                } else {
                    let mut said: Vec<(u64, Note)> = reply
                        .get("output")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|data| {
                            (
                                1,
                                Note::HeadlessData {
                                    id: id.clone(),
                                    data: data.as_str().unwrap_or("").into(),
                                },
                            )
                        })
                        .collect();
                    if let Some(code) = code {
                        said.push((9, Note::HeadlessExit { id, code }));
                    }
                    self.later(said);
                }
                Ok(session)
            }
            "terminal:create" => {
                let reply = self.take(method, &params).unwrap_or(json!({}));
                if let Some(e) = failed(&reply) {
                    return Err(e);
                }
                let id = {
                    let mut w = self.world();
                    w.sessions += 1;
                    format!("term-{}", w.sessions)
                };
                let mut session = json!({ "id": id });
                for (k, v) in reply
                    .get("session")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                {
                    session[k] = v.clone();
                }
                Ok(session)
            }
            "script:execute" => {
                let reply = self
                    .take(method, &params)
                    .unwrap_or(json!({ "result": { "success": true, "output": "" } }));
                let run_id = params
                    .get("runId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                for data in reply
                    .get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let _ = self.notes.send(Note::ScriptData {
                        run_id: run_id.clone(),
                        data: data.as_str().unwrap_or("").into(),
                    });
                }
                if let Some(e) = failed(&reply) {
                    return Err(e);
                }
                Ok(reply["result"].clone())
            }
            "connection:executeAction" | "http:request" | "connection:upsertFromItem" => {
                let default = match method {
                    "connection:executeAction" => json!({ "result": { "success": true } }),
                    "http:request" => {
                        json!({ "result": { "success": true, "output": { "status": 200 } } })
                    }
                    _ => json!({ "result": { "taskId": "t", "created": true } }),
                };
                let reply = self.take(method, &params).unwrap_or(default);
                if let Some(e) = failed(&reply) {
                    return Err(e);
                }
                Ok(reply["result"].clone())
            }
            "sessionEvent:listBySession" => {
                let key = json!({ "sessionId": params["sessionId"] });
                Ok(self
                    .take(method, &key)
                    .map_or(json!([]), |r| r["result"].clone()))
            }
            "worktree:activeSessions" => Ok(self
                .take(method, &json!({}))
                .map_or(json!({ "count": 0, "sessionIds": [] }), |r| {
                    r["result"].clone()
                })),
            "git:worktreeDirty" => Ok(self
                .take(method, &json!({}))
                .map_or(json!(false), |r| r["result"].clone())),
            "git:removeWorktree" => Ok(json!(true)),
            _ => Ok(Value::Null),
        }
    }

    fn notes(&self) -> broadcast::Receiver<Note> {
        self.notes.subscribe()
    }

    async fn take_task(&self, task_id: &str, session_id: &str, agent: &str) -> Option<TaskMove> {
        self.record(
            "task:start",
            json!({ "id": task_id, "sessionId": session_id, "agentType": agent }),
        );
        None
    }

    async fn reopen_task(&self, task_id: &str) -> Option<TaskMove> {
        self.record("task:reopen", json!({ "id": task_id }));
        None
    }

    fn gate_view(
        &self,
        run_id: &str,
        node_id: &str,
        round: u32,
        view: &str,
    ) -> Result<String, String> {
        let token = vorn_work::gates::publish(&self.data_dir, run_id, node_id, round, view)?;
        let html = std::fs::read_to_string(vorn_work::gates::file(
            &self.data_dir,
            run_id,
            node_id,
            round,
        ))
        .unwrap_or_default();
        self.world()
            .pages
            .insert((run_id.into(), node_id.into(), round), html);
        Ok(token)
    }

    fn keep_gate_page(&self, run_id: &str, node_id: &str, title: &str, round: u32) {
        let html = self
            .world()
            .pages
            .get(&(run_id.into(), node_id.into(), round))
            .cloned()
            .unwrap_or_default();
        self.record(
            "gate:keepPage",
            json!({ "runId": run_id, "nodeId": node_id, "title": title, "html": html }),
        );
    }

    async fn report_complete(&self, c: Completion) {
        let mut p = json!({
            "workflowId": c.workflow_id, "workflowName": c.workflow_name, "completedAt": c.completed_at,
            "status": c.status, "sessionsLaunched": c.sessions_launched,
        });
        if let Some(source) = c.source {
            p["source"] = json!(source.as_str());
        }
        self.record("workflow:executionComplete", p);
    }

    async fn complete_inbox(&self, id: i64, lease: &str, disposition: &str, error: Option<String>) {
        let mut p = json!({ "id": id, "leaseToken": lease, "disposition": disposition });
        if let Some(e) = error {
            p["error"] = json!(e);
        }
        self.record("connector:inboxComplete", p);
    }

    async fn renew_inbox(&self, id: i64, lease: &str) -> bool {
        self.record(
            "connector:inboxRenew",
            json!({ "id": id, "leaseToken": lease }),
        );
        true
    }

    async fn agent_sessions(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

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
    let dir = tempfile_dir();
    let fake = Fake {
        world: Arc::new(Mutex::new(World {
            config,
            replies: input
                .get("replies")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            ..World::default()
        })),
        notes: broadcast::channel(1024).0,
        data_dir: dir.clone(),
    };
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

fn tempfile_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vorn-workflow-ref-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
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
