//! Tasks against a test vornd: the board's reads and each change's round trip, built in or answered by the component.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::live::Live;
use crate::stats::summary;
use crate::support::TestVornd;
use crate::{sys, Variant};

/// The statuses seeded tasks hold, about as a busy board holds them.
const MIX: [&str; 20] = [
    "todo",
    "todo",
    "todo",
    "todo",
    "todo",
    "todo",
    "todo",
    "todo",
    "todo",
    "in_progress",
    "in_progress",
    "in_progress",
    "in_progress",
    "in_review",
    "in_review",
    "done",
    "done",
    "done",
    "done",
    "cancelled",
];
const WORDS: [&str; 8] = [
    "parser", "session", "sidebar", "gate", "schedule", "palette", "grid", "worktree",
];
/// Calls timed per kind, after the warm-up.
const SAMPLES: usize = 60;

/// One seeded task.
pub fn task(i: usize) -> Value {
    let status = MIX[i % MIX.len()];
    let mut t = json!({
        "id": format!("t-{i:06}"),
        "projectName": "bench",
        "title": format!("Task {i}: tidy the {} module", WORDS[i % WORDS.len()]),
        "description": "",
        "status": status,
        "order": i,
        "createdAt": "2026-10-01T10:00:00.000Z",
        "updatedAt": "2026-10-01T10:00:00.000Z",
    });
    if i.is_multiple_of(3) {
        t["assignedAgent"] = json!("claude");
    }
    if matches!(status, "done" | "cancelled") {
        t["completedAt"] = json!("2026-10-02T10:00:00.000Z");
    }
    t
}

/// Writes `n` tasks straight into vornd's database: a save of the whole configuration takes minutes at 10,000.
pub fn insert(home: &std::path::Path, n: usize) -> Result<(), String> {
    let mut store = vorn_store::Store::open_beside(&home.join("vorn.db"))
        .map_err(|e| e.to_string())?
        .ok_or("vornd has no database yet")?;
    for i in 0..n {
        store
            .call("dbInsertTask", json!([task(i)]))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The configuration with a `bench` project and the welcome seen, saved before any task exists.
pub fn bench_config(mut config: Value, home: &std::path::Path) -> Result<Value, String> {
    let path = home.join("bench");
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    config["projects"] = json!([{ "name": "bench", "path": path, "preferredAgents": [], "workspaceId": "personal" }]);
    config["defaults"]["hasSeenOnboarding"] = json!(2);
    Ok(config)
}

/// A `bench` project with `n` tasks.
pub async fn seed(live: &Live, home: &std::path::Path, n: usize) -> Result<(), String> {
    let (config, _) = live.call("config:load", Value::Null).await?;
    live.call("config:save", bench_config(config, home)?)
        .await?;
    let h = home.to_path_buf();
    tokio::task::spawn_blocking(move || insert(&h, n))
        .await
        .map_err(|e| e.to_string())??;
    let (list, _) = live
        .call("task:list", json!({ "projectName": "bench" }))
        .await?;
    let got = list.as_array().map_or(0, Vec::len);
    if got != n {
        return Err(format!("seeded {n} tasks, the board lists {got}"));
    }
    Ok(())
}

/// The broadcast that tells a viewer the board changed, by variant.
fn change_note(variant: Variant) -> &'static str {
    match variant {
        Variant::Ext => "ext:taskRows",
        _ => "config:changed",
    }
}

struct Timed {
    answer: Vec<f64>,
    settle: Vec<f64>,
    bytes: Vec<f64>,
}

impl Timed {
    fn new() -> Timed {
        Timed {
            answer: Vec::new(),
            settle: Vec::new(),
            bytes: Vec::new(),
        }
    }

    fn json(&self) -> Value {
        json!({ "answer_ms": summary(&self.answer), "settle_ms": summary(&self.settle), "note_bytes": summary(&self.bytes) })
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// One change: its answer, then the broadcast viewers redraw from.
async fn change(
    live: &mut Live,
    note: &str,
    method: &str,
    params: Value,
    into: &mut Timed,
) -> Result<Value, String> {
    live.drain();
    let (sent, rx) = live.send(method, params);
    let (at, answer) = tokio::time::timeout(Duration::from_secs(60), rx)
        .await
        .map_err(|_| format!("{method} took too long"))?
        .map_err(|_| "closed".to_owned())?;
    let answer = answer?;
    let heard = live
        .wait_for(note, Duration::from_secs(10))
        .await
        .ok_or(format!("no {note} after {method}"))?;
    into.answer.push(ms(at - sent));
    into.settle.push(ms(heard.at.max(at) - sent));
    into.bytes.push(heard.bytes as f64);
    Ok(answer)
}

async fn read(live: &Live, method: &str, params: Value, n: usize) -> Result<Vec<f64>, String> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let (_, rtt) = live.call(method, params.clone()).await?;
        out.push(ms(rtt));
    }
    Ok(out)
}

/// `case tasks <variant> <tier>`.
pub async fn case(variant: Variant, tier: usize) -> Result<Value, String> {
    variant.apply("tasks");
    let v = tokio::task::spawn_blocking(TestVornd::start)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no vornd built for the bench")?;
    let pid = v.pids()[0];
    let mut live = Live::connect(&v.home).await?;
    let t = Instant::now();
    seed(&live, &v.home, tier).await?;
    let seeded_ms = ms(t.elapsed());
    let mem_seeded = sys::memory(pid);
    let note = change_note(variant);

    for i in 0..10 {
        live.call("task:get", json!({ "id": format!("t-{i:06}") }))
            .await?;
        live.call("task:list", json!({ "projectName": "bench" }))
            .await?;
    }
    let list = read(
        &live,
        "task:list",
        json!({ "projectName": "bench" }),
        SAMPLES / 3,
    )
    .await?;
    let config = read(&live, "config:load", Value::Null, SAMPLES / 3).await?;

    let (mut create, mut update, mut moved) = (Timed::new(), Timed::new(), Timed::new());
    let mut made = Vec::new();
    for k in 0..SAMPLES {
        let a = change(
            &mut live,
            note,
            "task:create",
            json!({ "projectName": "bench", "title": format!("New task {k}"), "status": "todo" }),
            &mut create,
        )
        .await?;
        made.push(a["task"]["id"].as_str().unwrap_or_default().to_owned());
    }
    for (k, id) in made.iter().enumerate() {
        change(
            &mut live,
            note,
            "task:update",
            json!({ "id": id, "title": format!("Renamed task {k}") }),
            &mut update,
        )
        .await?;
    }
    let order = ["in_progress", "in_review", "done", "todo"];
    for k in 0..SAMPLES {
        let id = format!("t-{:06}", (k * 37) % tier);
        let status = order[k % order.len()];
        change(
            &mut live,
            note,
            "task:setStatus",
            json!({ "id": id, "status": status }),
            &mut moved,
        )
        .await?;
    }
    let mut board_save = Timed::new();
    if variant == Variant::Builtin {
        let saves = SAMPLES / 3;
        for k in 0..saves {
            let (mut config, _) = live.call("config:load", Value::Null).await?;
            let i = (k * 53) % tier;
            if let Some(t) = config["tasks"]
                .as_array_mut()
                .and_then(|ts| ts.iter_mut().find(|t| t["id"] == format!("t-{i:06}")))
            {
                t["status"] = json!(order[k % order.len()]);
            }
            change(
                &mut live,
                "config:changed",
                "config:save",
                config,
                &mut board_save,
            )
            .await?;
        }
    }
    let mem_end = sys::memory(pid);
    let mb = |m: Option<(f64, f64)>| {
        m.map_or(
            Value::Null,
            |(f, r)| json!({ "footprint": f, "resident": r }),
        )
    };
    Ok(json!({
        "kind": "tasks",
        "variant": variant.name(),
        "tier": tier,
        "seed_ms": seeded_ms,
        "list_ms": summary(&list),
        "config_load_ms": summary(&config),
        "create": create.json(),
        "update": update.json(),
        "move": moved.json(),
        "board_save": if board_save.answer.is_empty() { Value::Null } else { board_save.json() },
        "vornd_mb": { "seeded": mb(mem_seeded), "end": mb(mem_end) },
    }))
}
