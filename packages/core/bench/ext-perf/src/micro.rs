//! In one process, no socket: Tasks' calls and the step loop built in, through the JSON bridge natively, and in the component.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Map, Value};
use vorn_ext_perf_host::{Bridge, TasksComponent, WorkflowsComponent};
use vorn_protocol::WorkflowExecution;
use vorn_store::{Store, StoreOptions};

use crate::stats::summary;

type Shared = Arc<Mutex<Store>>;

fn options() -> StoreOptions {
    StoreOptions {
        default_shell: "/bin/sh".into(),
        default_agent_commands: Map::new(),
        default_workspace: serde_json::from_value(json!({ "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#000", "order": 0 }))
            .expect("a valid workspace"),
        owner_name: "bench".into(),
        seed_workflows: Vec::new(),
    }
}

/// A file store with a `bench` project and `n` tasks.
fn seeded(dir: &Path, n: usize) -> Result<Shared, String> {
    let (mut store, _) = Store::open(&dir.join("vorn.db"), options()).map_err(|e| e.to_string())?;
    store
        .call(
            "dbInsertProject",
            json!([{ "name": "bench", "path": "/bench", "preferredAgents": [] }]),
        )
        .map_err(|e| e.to_string())?;
    for i in 0..n {
        store
            .call("dbInsertTask", json!([crate::tasks::task(i)]))
            .map_err(|e| e.to_string())?;
    }
    Ok(Arc::new(Mutex::new(store)))
}

fn lock(s: &Shared) -> std::sync::MutexGuard<'_, Store> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// The bridge vornd gives the component: named calls on the store, JSON both ways.
struct StoreBridge(Shared);

impl Bridge for StoreBridge {
    fn db(&mut self, op: &str, args: &str) -> Result<String, String> {
        let args: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
        lock(&self.0)
            .call(op, args)
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    }
    fn run_changed(&mut self, _run: &str) {}
    fn log(&mut self, _line: &str) {}
}

/// One way to answer a Tasks call.
enum Path3 {
    Builtin(Shared),
    Bridged(Shared),
    Component(Shared, Box<vorn_ext_perf_host::Tasks>),
}

impl Path3 {
    fn name(&self) -> &'static str {
        match self {
            Path3::Builtin(_) => "builtin",
            Path3::Bridged(_) => "native-json",
            Path3::Component(..) => "component",
        }
    }

    fn call(
        &mut self,
        method: &str,
        params: &Value,
        now: &str,
        id: &str,
    ) -> Result<Option<Value>, String> {
        match self {
            Path3::Builtin(s) => {
                let mut st = lock(s);
                let st = &mut *st;
                let p = params;
                let out = match method {
                    "task:list" => vorn_work::tasks::list(st, p).map(Some),
                    "task:create" => {
                        vorn_work::tasks::create(st, p, id.to_owned(), now).map(|r| Some(r.0))
                    }
                    "task:update" => vorn_work::tasks::update(st, p, now).map(|r| Some(r.0)),
                    "task:setStatus" => vorn_work::tasks::set_status(st, p, now).map(|r| Some(r.0)),
                    "task:reorder" => vorn_work::tasks::reorder(st, p, now).map(|r| Some(r.0)),
                    "task:archive" => vorn_work::tasks::archive(st, p, now).map(|r| Some(r.0)),
                    "task:delete" => vorn_work::tasks::delete(st, p).map(|r| Some(r.0)),
                    "task:get" => vorn_work::tasks::get(st, p).map(Some),
                    _ => Ok(None),
                };
                out.map_err(|e| e.to_string())
            }
            Path3::Bridged(s) => {
                let s = Arc::clone(s);
                let mut store = vorn_store_shim::Store::new(move |op, args| {
                    let args: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
                    lock(&s)
                        .call(op, args)
                        .map(|v| v.to_string())
                        .map_err(|e| e.to_string())
                });
                vorn_ext_perf_guest::methods::call(&mut store, method, params, now, id.to_owned())
                    .map(|(v, _)| v)
                    .map_err(|e| e.to_string())
            }
            Path3::Component(s, tasks) => {
                let bridge = Box::new(StoreBridge(Arc::clone(s)));
                let (answer, _) = tasks
                    .call(bridge, method, &params.to_string(), now, id)
                    .map_err(|e| e.to_string())??;
                answer
                    .map(|a| serde_json::from_str(&a))
                    .transpose()
                    .map_err(|e| e.to_string())
            }
        }
    }
}

fn us(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e6
}

/// The same calls through all three, answers compared field by field.
fn parity(tasks: &TasksComponent, dir: &Path) -> Result<usize, String> {
    let mut paths = Vec::new();
    for (i, kind) in ["builtin", "bridged", "component"].iter().enumerate() {
        let d = dir.join(format!("parity-{i}"));
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let s = seeded(&d, 40)?;
        paths.push(match *kind {
            "builtin" => Path3::Builtin(s),
            "bridged" => Path3::Bridged(s),
            _ => Path3::Component(s, Box::new(tasks.instantiate().map_err(|e| e.to_string())?)),
        });
    }
    let calls = [
        (
            "task:create",
            json!({ "projectName": "bench", "title": "A", "status": "done", "branch": "b", "useWorktree": true }),
        ),
        (
            "task:create",
            json!({ "projectName": "nope", "title": "B" }),
        ),
        (
            "task:update",
            json!({ "id": "t-000003", "title": "Renamed", "status": "done" }),
        ),
        (
            "task:setStatus",
            json!({ "id": "t-000003", "status": "todo" }),
        ),
        (
            "task:archive",
            json!({ "id": "t-000015", "archived": true }),
        ),
        (
            "task:archive",
            json!({ "id": "t-000001", "archived": true }),
        ),
        (
            "task:reorder",
            json!({ "ids": ["t-000009", "t-000000", "t-000009", "ghost"] }),
        ),
        (
            "task:update",
            json!({ "id": "t-000004", "projectName": "nope" }),
        ),
        ("task:delete", json!({ "id": "t-000005" })),
        ("task:delete", json!({ "id": "t-000005" })),
        ("task:get", json!({ "id": "t-000003" })),
        (
            "task:list",
            json!({ "projectName": "bench", "includeDescription": true }),
        ),
        ("task:list", json!({ "status": "done" })),
    ];
    for (k, (method, params)) in calls.iter().enumerate() {
        let answers: Vec<Option<Value>> = paths
            .iter_mut()
            .map(|p| {
                p.call(
                    method,
                    params,
                    "2026-10-14T08:00:00.000Z",
                    &format!("new-{k}"),
                )
            })
            .collect::<Result<_, _>>()?;
        if answers[0] != answers[1] || answers[0] != answers[2] {
            return Err(format!("{method} #{k} answered differently: {answers:?}"));
        }
    }
    Ok(calls.len())
}

/// Times each Tasks call `n` times through one path.
fn time_tasks(path: &mut Path3, tier: usize, round: usize) -> Result<Value, String> {
    let now = "2026-10-14T08:00:00.000Z";
    let (mut list, mut create, mut update, mut moved) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for k in 0..10 {
        let t = Instant::now();
        path.call("task:list", &json!({ "projectName": "bench" }), now, "")?;
        list.push(us(t));
        let _ = k;
    }
    let order = ["in_progress", "in_review", "done", "todo"];
    for k in 0..100 {
        let id = format!("m-{round}-{k}");
        let t = Instant::now();
        path.call(
            "task:create",
            &json!({ "projectName": "bench", "title": format!("New {k}"), "status": "todo" }),
            now,
            &id,
        )?;
        create.push(us(t));
        let t = Instant::now();
        path.call(
            "task:update",
            &json!({ "id": id, "title": format!("Renamed {k}") }),
            now,
            "",
        )?;
        update.push(us(t));
        let t = Instant::now();
        path.call(
            "task:setStatus",
            &json!({ "id": format!("t-{:06}", (k * 37) % tier), "status": order[k % 4] }),
            now,
            "",
        )?;
        moved.push(us(t));
    }
    Ok(json!({
        "path": path.name(),
        "list_us": summary(&list),
        "create_us": summary(&create),
        "update_us": summary(&update),
        "move_us": summary(&moved),
    }))
}

/// The run bridge vornd gives the step loop, without its fan-out: the JSON read back as a run.
struct RunBridge;

impl Bridge for RunBridge {
    fn db(&mut self, op: &str, _args: &str) -> Result<String, String> {
        Err(op.to_owned())
    }
    fn run_changed(&mut self, run: &str) {
        let _ = std::hint::black_box(serde_json::from_str::<WorkflowExecution>(run));
    }
    fn log(&mut self, line: &str) {
        std::hint::black_box(line);
    }
}

struct Kept;

impl vorn_ext_perf_guest::workflows::Host for Kept {
    fn persist(&mut self, run: &WorkflowExecution) {
        std::hint::black_box(run.clone());
    }
    fn log(&mut self, line: &str) {
        std::hint::black_box(line);
    }
}

fn time_walks(flows: &WorkflowsComponent, steps: usize) -> Result<Value, String> {
    let wf = crate::flows::chain("w", steps, false, json!({ "triggerType": "manual" }));
    let text = wf.to_string();
    let (mut native, mut component, mut instantiate, mut memory) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for k in 0..30 {
        let t = Instant::now();
        let w = vorn_ext_perf_guest::workflows::start(&wf, format!("n{k}"), &mut Kept)?;
        native.push(us(t) / steps as f64);
        if w.steps as usize != steps {
            return Err("the native walk stopped short".into());
        }
        let t = Instant::now();
        let mut inst = flows
            .instantiate(Box::new(RunBridge))
            .map_err(|e| e.to_string())?;
        instantiate.push(us(t));
        let t = Instant::now();
        let out = inst
            .start(&text, &format!("c{k}"))
            .map_err(|e| e.to_string())??;
        component.push(us(t) / steps as f64);
        if out.steps as usize != steps {
            return Err("the component's walk stopped short".into());
        }
        memory.push(inst.memory() as f64 / 1024.0);
    }
    Ok(json!({
        "steps": steps,
        "native_us_per_step": summary(&native),
        "component_us_per_step": summary(&component),
        "instantiate_us": summary(&instantiate),
        "instance_kb": summary(&memory),
    }))
}

/// `case micro <tier> <cwasm dir>`.
pub fn case(tier: usize, cwasm: &Path) -> Result<Value, String> {
    let read = |n: &str| std::fs::read(cwasm.join(n)).map_err(|e| format!("{n}: {e}"));
    let t = Instant::now();
    let tasks = TasksComponent::load(&read("tasks.cwasm")?).map_err(|e| e.to_string())?;
    let flows = WorkflowsComponent::load(&read("workflows.cwasm")?).map_err(|e| e.to_string())?;
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    let dir = std::env::temp_dir().join(format!("ext-perf-micro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let checked = parity(&tasks, &dir)?;
        let mut paths = Vec::new();
        for (i, kind) in ["builtin", "bridged", "component"].iter().enumerate() {
            let d = dir.join(format!("timed-{i}"));
            std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            let s = seeded(&d, tier)?;
            paths.push(match *kind {
                "builtin" => Path3::Builtin(s),
                "bridged" => Path3::Bridged(s),
                _ => Path3::Component(s, Box::new(tasks.instantiate().map_err(|e| e.to_string())?)),
            });
        }
        let mut rounds = Vec::new();
        for round in 0..3 {
            for k in 0..paths.len() {
                let i = (k + round) % paths.len();
                rounds.push(time_tasks(&mut paths[i], tier, round)?);
            }
        }
        let walks = [10, 100]
            .iter()
            .map(|&s| time_walks(&flows, s))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(
            json!({ "kind": "micro", "tier": tier, "load_ms": load_ms, "parity_calls": checked, "tasks": rounds, "walks": walks }),
        )
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}
