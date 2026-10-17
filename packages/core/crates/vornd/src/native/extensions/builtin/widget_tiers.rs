//! Measures the Widget built-in through `Builtins`, whichever tier it runs in:
//! `cargo test --profile ci -p vornd widget_tiers -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_extensions::api::types::{Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::AGENT_PERMISSION;
use vorn_extensions::gate::{Answer, Caller, World};
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

use super::*;

const W: &str = "vorn.widget";

struct Core;

impl World for Core {
    fn call(&self, _: &Caller, method: &str, _: Value) -> Answer<Value> {
        let answer = match method {
            "sessions.list" => Value::Array(
                (0..32)
                    .map(|i| json!({ "id": format!("t{i}"), "name": format!("s{i}"), "agent": "acme", "status": if i % 3 == 0 { "running" } else { "waiting" } }))
                    .collect(),
            ),
            _ => Value::Null,
        };
        Box::pin(async move { Ok(answer) })
    }
    fn fetch(&self, _: &Caller, _: Request) -> Answer<Response> {
        Box::pin(async { Err(ExtError::new("network", "none")) })
    }
    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }
}

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .unwrap_or(0.0)
        / 1024.0
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    let i = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[i.saturating_sub(1).min(v.len() - 1)]
}

async fn until(mut f: impl FnMut() -> bool) {
    while !f() {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement, run by hand"]
async fn widget_tiers() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = RouterSettings::new(Vec::new(), "bench");
    settings.compiler = vorn_extensions::component::Compiler::InProcess;
    let router = Router::new(
        PackStore::new(dir.path().join("connectors")),
        Lifecycle::new(Policy::default()),
        Arc::new(Core) as Arc<dyn World>,
        settings,
        vorn_extensions::events::Bus::new(),
    );
    let builtins = Builtins::new(
        Arc::clone(&router),
        dir.path().join("builtins.json"),
        Vec::new(),
    );
    let agents = |r: &Router| !r.rows(W, "agents").is_empty();
    // A first call compiles a component into the cache; the measured starts load it.
    builtins.set(W, true).unwrap();
    let _ = builtins
        .call(W, "ui.render", json!({ "point": "compact", "surface": {} }))
        .await;
    builtins.set(W, false).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let base = rss_mb();

    let mut start = Vec::new();
    for _ in 0..15 {
        builtins.set(W, false).unwrap();
        router.stop(W).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let t = Instant::now();
        builtins.set(W, true).unwrap();
        // A component starts on its first use: a window asking for its tree.
        let _ = builtins
            .call(W, "ui.render", json!({ "point": "compact", "surface": {} }))
            .await;
        until(|| agents(&router)).await;
        start.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let idle = rss_mb() - base;

    let mut to_publish = Vec::new();
    for i in 0..200 {
        let id = format!("r{i}");
        let t = Instant::now();
        router.bus().emit(AGENT_PERMISSION, || {
            json!({ "requestId": id, "terminalId": "t1", "toolName": "Bash", "toolInput": { "command": "npm test" } })
        });
        until(|| {
            router
                .rows(W, "permissions")
                .iter()
                .any(|r| r["id"] == id.as_str())
        })
        .await;
        to_publish.push(t.elapsed().as_secs_f64() * 1e6);
    }

    let mut call = Vec::new();
    for i in 0..2000 {
        let t = Instant::now();
        builtins
            .call(W, "deny", json!({ "id": format!("r{}", i % 200) }))
            .await
            .unwrap()
            .unwrap();
        call.push(t.elapsed().as_secs_f64() * 1e6);
    }
    let mut render = Vec::new();
    for _ in 0..500 {
        let t = Instant::now();
        let answer = builtins
            .call(W, "ui.render", json!({ "point": "widget", "surface": {} }))
            .await;
        if !matches!(answer, Some(Ok(_))) {
            break;
        }
        render.push(t.elapsed().as_secs_f64() * 1e6);
    }
    let busy = rss_mb() - base;
    let r = json!({
        "start_ms_p50": pct(&mut start, 50.0),
        "event_to_publish_us_p50": pct(&mut to_publish, 50.0),
        "event_to_publish_us_p99": pct(&mut to_publish, 99.0),
        "call_us_p50": pct(&mut call, 50.0),
        "call_us_p99": pct(&mut call, 99.0),
        "render_us_p50": if render.is_empty() { Value::Null } else { json!(pct(&mut render, 50.0)) },
        "memory_idle_mb": idle,
        "memory_busy_mb": busy,
        "linear_mb": router.running(W).and_then(|l| l.component_memory()).map(|b| b as f64 / 1048576.0),
    });
    println!("WIDGET_TIERS {r}");
}
