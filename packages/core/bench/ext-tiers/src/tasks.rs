//! Tasks as a built-in, a program and a Wasm component, over one board of
//! `n` tasks in a real store: the writes the board makes (create, update, a
//! move across statuses), the MCP tool that lists the board a page at a
//! time, the badge rows a move publishes, memory, and a restart. The
//! built-in is E6's own crate; the other two run `ext-tiers-tasks`, its
//! code reaching the tables over the bridge (`task.store`).
//!
//! Each case prints one line in the shape of E6's `raw.jsonl`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use vorn_extensions::api::types::RowOp;
use vorn_store::{Store, StoreOptions};
use vorn_tasks::{Change, Host, Pending, TableWork, Tasks};

use crate::tier::{Kind, Paths, Tier, TASKS};
use crate::world::Fake;
use crate::{held, me, ms, stats, sys};

/// The project every task is in.
const PROJECT: &str = "app";
/// Sessions with a task each, which a move puts on their card and takes off.
const SESSIONS: usize = 32;
/// Each write, this many times.
const WRITES: usize = 60;
/// The board listed, this many times.
const LISTS: usize = 20;
const RESTARTS: usize = 10;
/// The store calls `task.store` answers: those the built-in's code makes.
const STORE_CALLS: &[&str] = &[
    "dbGetTask",
    "dbInsertTask",
    "dbUpdateTask",
    "dbDeleteTask",
    "dbListTasks",
    "dbGetProject",
    "dbGetMaxTaskOrder",
];

/// A description the length a card's usually is.
fn description(i: usize) -> String {
    format!("Task {i}. ") + &"Make the board read the way the agents left it. ".repeat(12)
}

/// The board, in a store of its own: what vornd's store holds and the bridge reaches.
pub struct Board {
    store: Mutex<Store>,
    /// The size of each `task:rows` note clients were sent.
    pub notes: Mutex<Vec<f64>>,
    /// The triggers fired, by event name.
    pub emitted: Mutex<BTreeMap<String, u64>>,
    /// Calls on the tables over the bridge.
    pub store_calls: AtomicU64,
    _dir: tempfile::TempDir,
}

impl Board {
    /// `n` tasks in one project: most done, some to do, a few in progress,
    /// and one to do for each of [`SESSIONS`] sessions.
    pub fn seeded(n: usize) -> (Board, Duration) {
        let dir = tempfile::tempdir().expect("temp dir");
        let options = StoreOptions {
            default_shell: "/bin/sh".into(),
            default_agent_commands: Map::new(),
            default_workspace: serde_json::from_value(json!({ "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#000", "order": 0 })).expect("workspace"),
            owner_name: "bench".into(),
            seed_workflows: Vec::new(),
        };
        let (mut store, _) = Store::open(&dir.path().join("vorn.db"), options).expect("store");
        let t = Instant::now();
        let project = json!({ "name": PROJECT, "path": "/tmp/app", "preferredAgents": [], "icon": "Folder", "hostIds": ["local"], "workspaceId": "personal" });
        store
            .call("dbInsertProject", json!([project]))
            .expect("project");
        for i in 0..n {
            let mut task = json!({
                "id": format!("t{i}"), "projectName": PROJECT, "title": format!("Task {i}"),
                "description": description(i), "order": i,
                "createdAt": "2026-07-01T00:00:00.000Z", "updatedAt": "2026-07-01T00:00:00.000Z",
            });
            task["status"] = json!(if i < SESSIONS || (1..=3).contains(&(i % 10)) {
                "todo"
            } else if i % 50 == 0 {
                "in_progress"
            } else {
                "done"
            });
            if i < SESSIONS {
                task["assignedSessionId"] = json!(format!("s{i}"));
            }
            store.call("dbInsertTask", json!([task])).expect("task");
        }
        let took = t.elapsed();
        let board = Board {
            store: Mutex::new(store),
            notes: Mutex::new(Vec::new()),
            emitted: Mutex::new(BTreeMap::new()),
            store_calls: AtomicU64::new(0),
            _dir: dir,
        };
        (board, took)
    }

    fn with<T>(&self, f: impl FnOnce(&mut Store) -> T) -> T {
        f(&mut self.store.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Whether [`Board::answer`] answers `method`.
    pub fn answers(method: &str) -> bool {
        matches!(
            method,
            "task.list"
                | "task.get"
                | "task.store"
                | "task.told"
                | "task.emit"
                | "agents.list"
                | "project.list"
        )
    }

    /// A core call on the board, as vornd answers it; blocking.
    pub fn answer(&self, method: &str, args: &Value) -> Result<Value, String> {
        let failed = |e: vorn_store::Error| e.to_string();
        match method {
            "task.list" => self.with(|s| page(s, args)).map_err(failed),
            "task.get" => self
                .with(|s| s.call("dbGetTask", json!([args.get("id")])))
                .map_err(failed),
            "task.store" => {
                let call = args.get("call").and_then(Value::as_str).unwrap_or_default();
                if !STORE_CALLS.contains(&call) {
                    return Err(format!("task.store does not make {call}"));
                }
                self.store_calls.fetch_add(1, Ordering::Relaxed);
                let call_args = args.get("args").cloned().unwrap_or(Value::Null);
                self.with(|s| s.call(call, call_args)).map_err(failed)
            }
            "task.told" => self.told(args).map(|()| Value::Null),
            "task.emit" => {
                let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
                self.emit(name);
                Ok(Value::Null)
            }
            "agents.list" => Ok(json!([])),
            "project.list" => self
                .with(|s| s.call("dbListProjects", json!([])))
                .map_err(failed),
            other => Err(format!("the board has no {other}")),
        }
    }

    pub fn emit(&self, name: &str) {
        *self
            .emitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(name.to_owned())
            .or_default() += 1;
    }

    /// What vornd's `told` does: the configuration read again, each changed
    /// row read again, and both serialised for the clients.
    pub fn told(&self, note: &Value) -> Result<(), String> {
        let (config, note) = self
            .with(|s| -> vorn_store::Result<(Value, Value)> {
                let config = s.load_config()?;
                let Some(rows) = note.get("upsert").and_then(Value::as_array) else {
                    return Ok((config, note.clone()));
                };
                let mut now = Vec::with_capacity(rows.len());
                for row in rows {
                    let read = s.call("dbGetTask", json!([row.get("id")]))?;
                    if !read.is_null() {
                        now.push(read);
                    }
                }
                Ok((config, json!({ "upsert": now })))
            })
            .map_err(|e| e.to_string())?;
        std::hint::black_box(config.to_string());
        #[allow(clippy::cast_precision_loss)]
        let bytes = note.to_string().len() as f64;
        self.notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(bytes);
        Ok(())
    }
}

/// vornd's `task_list::list`: the board, descriptions only when asked for.
fn list(store: &mut Store, params: &Value) -> vorn_store::Result<Value> {
    let get = |k: &str| params.get(k).cloned().unwrap_or(Value::Null);
    let page = |k: &str| get(k).as_f64().filter(|n| n.is_finite() && *n >= 0.0);
    let mut tasks = store.call(
        "dbListTasks",
        json!([
            get("projectName"),
            get("status"),
            page("offset"),
            page("limit")
        ]),
    )?;
    if params
        .get("includeDescription")
        .is_some_and(|v| v.as_bool() == Some(true))
    {
        return Ok(tasks);
    }
    for task in tasks.as_array_mut().into_iter().flatten() {
        task["description"] = json!("");
    }
    Ok(tasks)
}

/// vornd's `task_list::page`: `task.list` for an extension, `{ tasks, next }`.
fn page(store: &mut Store, args: &Value) -> vorn_store::Result<Value> {
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .clamp(1, 1000);
    let mut params = json!({ "offset": offset, "limit": limit });
    for key in ["projectName", "status", "includeDescription"] {
        if let Some(v) = args.get(key) {
            params[key] = v.clone();
        }
    }
    let tasks = list(store, &params)?;
    let n = tasks.as_array().map_or(0, Vec::len) as u64;
    let next = (n == limit).then_some(offset + n);
    Ok(json!({ "tasks": tasks, "next": next }))
}

/// What the built-in reaches Vorn through, as vornd hands it over: the
/// board's tables off the runtime, and the same board and world the other
/// tiers reach over the bridge.
struct Bench {
    board: Arc<Board>,
    world: Arc<Fake>,
    images: PathBuf,
    rows: Mutex<BTreeMap<String, Value>>,
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

impl Host for Bench {
    fn tables(&self, work: TableWork) -> Pending<Result<(), String>> {
        let board = Arc::clone(&self.board);
        Box::pin(blocking(move || {
            board.with(|s| work(s)).map_err(|e| e.to_string())
        }))
    }

    fn data_dir(&self) -> Option<PathBuf> {
        Some(self.images.clone())
    }

    fn emit(&self, name: &str, _data: Value) {
        self.board.emit(name);
    }

    fn changed(&self, change: Change) -> Pending<()> {
        let note = match change {
            Change::Rows(rows) => json!({ "upsert": rows }),
            Change::Removed(ids) => json!({ "remove": ids }),
        };
        let board = Arc::clone(&self.board);
        Box::pin(async move {
            let _ = blocking(move || board.told(&note)).await;
        })
    }

    fn call(&self, method: &str, params: Value) -> Pending<Result<Value, String>> {
        let (board, method) = (Arc::clone(&self.board), method.to_owned());
        Box::pin(blocking(move || board.answer(&method, &params)))
    }

    fn publish(&self, _collection: &str, ops: Vec<RowOp>) {
        {
            let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            for op in ops {
                match op {
                    RowOp::Upsert(row) => {
                        let id = row["id"].as_str().unwrap_or_default().to_owned();
                        rows.insert(id, row);
                    }
                    RowOp::Delete(id) => {
                        rows.remove(&id);
                    }
                    RowOp::Clear => rows.clear(),
                }
            }
        }
        self.world.saw_publish();
    }
}

/// Tasks under one tier.
enum Under {
    Builtin { tasks: Arc<Tasks>, host: Arc<Bench> },
    Package(Tier),
}

impl Under {
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Under::Builtin { tasks, .. } => {
                if let Some(tool) = method.strip_prefix("mcp.") {
                    let result = vorn_tasks::tools::call(tasks, tool, params)
                        .await
                        .ok_or("no such tool")?;
                    if result.get("isError") == Some(&Value::Bool(true)) {
                        return Err(result.to_string());
                    }
                    return Ok(result["content"][0]["text"].clone());
                }
                match tasks.answer(method, params).await {
                    vorn_tasks::Answer::Result(v) => Ok(v),
                    vorn_tasks::Answer::Void => Ok(Value::Null),
                    vorn_tasks::Answer::Error(e) => Err(e),
                    vorn_tasks::Answer::Unanswered => Err(format!("unanswered {method}")),
                }
            }
            Under::Package(tier) => tier.call(method, params).await.map_err(|e| e.message),
        }
    }

    /// Stopped and started again, as a crash or a switch would: until it answers.
    async fn restart(&mut self) -> Duration {
        match self {
            Under::Builtin { tasks, host } => {
                let t = Instant::now();
                *tasks = Tasks::new(Arc::clone(host) as Arc<dyn Host>);
                tasks.badges().await;
                t.elapsed()
            }
            Under::Package(tier) => {
                tier.stop().await;
                let t = Instant::now();
                tier.start().await.expect("started");
                tier.call("bench.echo", json!({})).await.expect("echo");
                t.elapsed()
            }
        }
    }

    /// The badge rows it published, by session.
    fn badges(&self) -> Value {
        let mut rows: Vec<Value> = match self {
            Under::Builtin { host, .. } => host
                .rows
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .values()
                .cloned()
                .collect(),
            Under::Package(tier) => tier.rows("working"),
        };
        rows.sort_by(|a, b| a["id"].to_string().cmp(&b["id"].to_string()));
        Value::Array(rows)
    }

    fn tier(&self) -> Option<&Tier> {
        match self {
            Under::Package(tier) => Some(tier),
            Under::Builtin { .. } => None,
        }
    }
}

/// Times `count` calls of `method`, each with the params `make` gives:
/// `{answer_ms, note_bytes}`, and the bridge calls each made.
async fn writes(
    under: &Under,
    board: &Board,
    world: &Fake,
    method: &str,
    make: impl Fn(usize) -> Value,
) -> Value {
    let notes0 = board
        .notes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    let (calls0, store0) = (
        world.calls.load(Ordering::Relaxed),
        board.store_calls.load(Ordering::Relaxed),
    );
    let mut answer = Vec::with_capacity(WRITES);
    for k in 0..WRITES {
        let t = Instant::now();
        under.call(method, make(k)).await.expect(method);
        answer.push(ms(t.elapsed()));
    }
    let notes = board.notes.lock().unwrap_or_else(PoisonError::into_inner)[notes0..].to_vec();
    #[allow(clippy::cast_precision_loss)]
    let per = |n: u64| stats::round(n as f64 / WRITES as f64);
    json!({
        "answer_ms": stats::summary(&answer),
        "note_bytes": stats::summary(&notes),
        "bridge_calls": under.tier().map(|_| per(world.calls.load(Ordering::Relaxed) - calls0)),
        "store_calls": under.tier().map(|_| per(board.store_calls.load(Ordering::Relaxed) - store0)),
    })
}

/// One case: Tasks under `kind` over `n` tasks.
pub async fn case(kind: Kind, n: usize, paths: &Paths) -> Value {
    let (board, seed) = Board::seeded(n);
    let board = Arc::new(board);
    let world = Arc::new(Fake::with_board(Arc::clone(&board)));
    let images = tempfile::tempdir().expect("images");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let base = sys::memory(me()).unwrap_or(0.0);
    let t = Instant::now();
    let mut under = match kind {
        Kind::Builtin => {
            let host = Arc::new(Bench {
                board: Arc::clone(&board),
                world: Arc::clone(&world),
                images: images.path().to_owned(),
                rows: Mutex::new(BTreeMap::new()),
            });
            let tasks = Tasks::new(Arc::clone(&host) as Arc<dyn Host>);
            tasks.badges().await;
            Under::Builtin { tasks, host }
        }
        Kind::Program | Kind::Wasm => {
            let tier = Tier::on(
                kind,
                TASKS,
                paths,
                Duration::from_secs(60),
                Arc::clone(&world),
            );
            tier.start().await.expect("started");
            Under::Package(tier)
        }
    };
    let first = ms(t.elapsed());

    let mut list = Vec::with_capacity(LISTS);
    let mut listed = 0;
    for _ in 0..LISTS {
        let t = Instant::now();
        let text = under
            .call("mcp.list_tasks", json!({ "project_name": PROJECT }))
            .await
            .expect("list_tasks");
        list.push(ms(t.elapsed()));
        listed = serde_json::from_str::<Value>(text.as_str().unwrap_or("[]"))
            .ok()
            .and_then(|v| v.as_array().map(Vec::len))
            .unwrap_or(0);
    }

    let create = writes(&under, &board, &world, "task:create", |k| {
        json!({ "projectName": PROJECT, "title": format!("New {k}"), "description": description(k) })
    })
    .await;
    let others = n.saturating_sub(SESSIONS).max(1);
    let update = writes(&under, &board, &world, "task:update", |k| {
        json!({ "id": format!("t{}", SESSIONS + (k * 97) % others), "title": format!("Renamed {k}") })
    })
    .await;

    // A move puts a session's task in progress, or takes it out: its card's badge moves each time.
    let mut moved = Vec::with_capacity(WRITES);
    let mut to_badge = Vec::with_capacity(WRITES);
    let notes0 = board
        .notes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    let (calls0, store0) = (
        world.calls.load(Ordering::Relaxed),
        board.store_calls.load(Ordering::Relaxed),
    );
    for k in 0..WRITES {
        let status = if (k / SESSIONS).is_multiple_of(2) {
            "in_progress"
        } else {
            "todo"
        };
        let params = json!({ "id": format!("t{}", k % SESSIONS), "status": status });
        let before = world.published.load(Ordering::SeqCst);
        let t = Instant::now();
        under.call("task:setStatus", params).await.expect("move");
        moved.push(ms(t.elapsed()));
        let published = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let woke = world.moved.notified();
                if world.published.load(Ordering::SeqCst) > before {
                    return;
                }
                woke.await;
            }
        })
        .await;
        if published.is_ok() {
            to_badge.push(ms(t.elapsed()));
        }
    }
    let notes = board.notes.lock().unwrap_or_else(PoisonError::into_inner)[notes0..].to_vec();
    #[allow(clippy::cast_precision_loss)]
    let per = |d: u64| stats::round(d as f64 / WRITES as f64);
    let movement = json!({
        "answer_ms": stats::summary(&moved),
        "note_bytes": stats::summary(&notes),
        "bridge_calls": under.tier().map(|_| per(world.calls.load(Ordering::Relaxed) - calls0)),
        "store_calls": under.tier().map(|_| per(board.store_calls.load(Ordering::Relaxed) - store0)),
    });

    let image = match under
        .call(
            "task:imageUpload",
            json!({ "taskId": "t0", "base64": "aGk=", "filename": "a.png" }),
        )
        .await
    {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "err": e }),
    };

    tokio::time::sleep(Duration::from_millis(300)).await;
    let memory = match &under {
        Under::Package(tier) => held(tier, base),
        Under::Builtin { .. } => {
            let own = sys::memory(me()).unwrap_or(0.0) - base;
            json!({ "host_delta_mb": stats::round(own), "program_mb": null, "linear_mb": null, "total_mb": stats::round(own.max(0.0)) })
        }
    };
    let badges = under.badges();
    let mut restart = Vec::with_capacity(RESTARTS);
    for _ in 0..RESTARTS {
        restart.push(ms(under.restart().await));
    }
    if let Under::Package(tier) = &under {
        tier.stop().await;
    }
    let emitted = board
        .emitted
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!({
        "kind": "tasks",
        "variant": kind.name(),
        "tier": n,
        "seed_ms": stats::round(ms(seed)),
        "first_answer_ms": stats::round(first),
        "list_ms": stats::summary(&list),
        "listed": listed,
        "create": create,
        "update": update,
        "move": movement,
        "badge_ms": stats::summary(&to_badge),
        "restart_ms": stats::summary(&restart),
        "memory": memory,
        "triggers": emitted,
        "badges": badges,
        "image_upload": image,
    })
}

/// The median of `xs`, with the lowest and highest.
fn spread(mut xs: Vec<f64>) -> Option<(f64, f64, f64)> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(f64::total_cmp);
    Some((xs[xs.len() / 2], xs[0], xs[xs.len() - 1]))
}

fn fmt(x: f64) -> String {
    if x >= 100.0 {
        format!("{x:.0}")
    } else if x >= 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.2}")
    }
}

/// E6's `summary.md`: one table per board size, a column per tier, each
/// cell the median of the runs (lowest–highest).
pub fn summary(machine: &Value, rows: &[Value], tiers: &[usize]) -> String {
    let kinds = ["builtin", "program", "wasm"];
    let metrics: &[(&str, &str)] = &[
        (
            "list_tasks (MCP, every page of task.list) ms p50",
            "/list_ms/p50",
        ),
        (
            "list_tasks (MCP, every page of task.list) ms p99",
            "/list_ms/p99",
        ),
        ("create answer ms p50", "/create/answer_ms/p50"),
        ("create answer ms p99", "/create/answer_ms/p99"),
        ("update answer ms p50", "/update/answer_ms/p50"),
        ("update answer ms p99", "/update/answer_ms/p99"),
        ("move answer ms p50", "/move/answer_ms/p50"),
        ("move answer ms p99", "/move/answer_ms/p99"),
        ("move to badge row published ms p50", "/badge_ms/p50"),
        ("move to badge row published ms p99", "/badge_ms/p99"),
        ("change broadcast bytes p50", "/move/note_bytes/p50"),
        ("bridge calls per create", "/create/bridge_calls"),
        ("bridge calls per move", "/move/bridge_calls"),
        ("restart ms p50", "/restart_ms/p50"),
        ("memory MB at end (tier's own)", "/memory/total_mb"),
        ("Wasm linear memory MB", "/memory/linear_mb"),
    ];
    let mut out = String::from("# Tasks as a built-in, a program and a Wasm component\n\n");
    out += &format!(
        "Machine: {machine}. Cells: median of the runs (lowest–highest). One board in a real store per case; the program and the component reach it over the bridge.\n"
    );
    for &n in tiers {
        out += &format!("\n**{n} tasks**\n\n| Metric | builtin | program | wasm |\n| --- | ---: | ---: | ---: |\n");
        for (label, pointer) in metrics {
            let cells: Vec<String> = kinds
                .iter()
                .map(|k| {
                    let xs: Vec<f64> = rows
                        .iter()
                        .filter(|r| r["variant"] == *k && r["tier"] == n)
                        .filter_map(|r| r.pointer(pointer).and_then(Value::as_f64))
                        .collect();
                    spread(xs).map_or("—".to_owned(), |(m, lo, hi)| {
                        format!("{} ({}–{})", fmt(m), fmt(lo), fmt(hi))
                    })
                })
                .collect();
            out += &format!("| {label} | {} |\n", cells.join(" | "));
        }
    }
    out += "\n**Same board under each tier** (last run of each)\n\n| Tier | triggers fired | badge rows | image upload |\n| --- | --- | ---: | --- |\n";
    for k in kinds {
        if let Some(r) = rows.iter().rev().find(|r| r["variant"] == k) {
            let image = r["image_upload"]
                .get("err")
                .map_or("stored".to_owned(), |e| {
                    format!("refused: {}", e.as_str().unwrap_or_default())
                });
            out += &format!(
                "| {k} | {} | {} | {image} |\n",
                r["triggers"],
                r["badges"].as_array().map_or(0, Vec::len)
            );
        }
    }
    out
}
