//! Both tiers of each extension behave the same: the same scripted session
//! gives the same answers, rows, trees and calls to Vorn whether the
//! extension runs as a Wasm component or as a program. The RSS port is
//! also checked against the Node connector it was ported from.

use std::path::PathBuf;

use serde_json::{json, Map, Value};
use vorn_bench_ext_tiers::node::{self, NodeRss};
use vorn_bench_ext_tiers::rig::{Ext, Rig, Tier, CRASH, RSS, WIDGET, WORKTREES};
use vorn_bench_ext_tiers::world::Bench;
use vorn_bench_ext_tiers::{fixtures, scenario};

fn program(ext: Ext) -> PathBuf {
    let bin = match ext.bin {
        "tier-widget" => env!("CARGO_BIN_EXE_tier-widget"),
        "tier-worktrees" => env!("CARGO_BIN_EXE_tier-worktrees"),
        "tier-rss" => env!("CARGO_BIN_EXE_tier-rss"),
        _ => env!("CARGO_BIN_EXE_tier-crash"),
    };
    PathBuf::from(bin)
}

/// The scripted session's transcript on each tier.
async fn both(ext: Ext) -> (Value, Value) {
    let wasm = vorn_guest_build::component(ext.package).unwrap_or_else(|e| panic!("{e}"));
    let feeds = fixtures::serve().await.expect("the fixture server");
    let mut out = Vec::new();
    for tier in [Tier::Wasm(wasm), Tier::Program(program(ext))] {
        let rig = Rig::new(ext, tier, Bench::logged(Some(feeds)), None).expect("a rig");
        out.push(scenario::run(&rig).await);
        rig.stop().await;
    }
    let program = out.pop().expect("two runs");
    (out.pop().expect("two runs"), program)
}

/// The first step at which `a` and `b` differ, so a failure names it; both are kept to compare.
fn same(name: &str, a: &Value, b: &Value) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    for (tier, t) in [("wasm", a), ("program", b)] {
        let _ = std::fs::write(dir.join(format!("{name}-{tier}.json")), format!("{t:#}"));
    }
    let (Some(a), Some(b)) = (a.as_array(), b.as_array()) else {
        panic!("transcripts are lists");
    };
    for (x, y) in a.iter().zip(b) {
        assert_eq!(x, y, "the tiers differ at step {}", x["step"]);
    }
    assert_eq!(a.len(), b.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_widget_behaves_the_same_in_both_tiers() {
    let (wasm, program) = both(WIDGET).await;
    same("widget", &wasm, &program);
    let text = wasm.to_string();
    assert!(text.contains("cargo test"), "the ask was drawn");
    assert!(text.contains("permissions.resolve"), "a click answered the agent");
}

#[tokio::test(flavor = "multi_thread")]
async fn worktree_cleanup_behaves_the_same_in_both_tiers() {
    let (wasm, program) = both(WORKTREES).await;
    same("worktrees", &wasm, &program);
    assert!(wasm.to_string().contains("worktrees.remove"), "a worktree was removed");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_feed_reader_behaves_the_same_in_both_tiers() {
    let (wasm, program) = both(RSS).await;
    same("rss", &wasm, &program);
    assert!(wasm.to_string().contains("\"items\""), "feeds were read");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_crash_probe_answers_in_both_tiers() {
    let (wasm, program) = both(CRASH).await;
    same("crash", &wasm, &program);
}

/// A poll as the connector protocol carries it: the Node SDK lifts each
/// item's `data` into the item and fills in what the item leaves out.
fn wire(poll: &Value) -> Value {
    const RESERVED: [&str; 8] = ["externalId", "title", "url", "description", "status", "labels", "assignee", "updatedAt"];
    let mut poll = scenario::poll_shape(poll.clone());
    if let Some(items) = poll["items"].as_array_mut() {
        for item in items {
            let mut framed = Map::new();
            if let Some(data) = item["data"].as_object() {
                framed.extend(data.iter().filter(|(k, _)| !RESERVED.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())));
            }
            framed.insert("externalId".into(), item["externalId"].clone());
            framed.insert("title".into(), item["title"].clone());
            framed.insert("url".into(), item.get("url").cloned().unwrap_or(json!("")));
            framed.insert("description".into(), item.get("description").cloned().unwrap_or(json!("")));
            framed.insert("status".into(), json!("open"));
            framed.insert("labels".into(), item.get("labels").cloned().unwrap_or(json!([])));
            if let Some(a) = item.get("assignee") {
                framed.insert("assignee".into(), a.clone());
            }
            framed.insert("updatedAt".into(), item["updatedAt"].clone());
            *item = Value::Object(framed);
        }
    }
    poll
}

/// The Rust port answers each action as the Node connector does.
#[tokio::test(flavor = "multi_thread")]
async fn the_rust_feed_reader_answers_as_the_node_connector() {
    let Some(node) = node::node() else {
        eprintln!("skipped: no node on PATH");
        return;
    };
    let feeds = fixtures::serve().await.expect("the fixture server");
    let connector = NodeRss::open(&node, feeds, &scenario::feeds()).await.expect("the connector");
    let rig = Rig::new(RSS, Tier::Program(program(RSS)), Bench::new(Some(feeds)), None).expect("a rig");
    for (action, args) in scenario::rss_calls() {
        let ported = rig.call(action, args.clone()).await.map_err(|e| e.message);
        let shipped = connector.action(action, &args).await;
        match (&ported, &shipped) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{action} {args}"),
            (Err(a), Err(b)) => assert!(b.contains(a.as_str()), "{action} {args}: {a} / {b}"),
            _ => panic!("{action} {args}: {ported:?} / {shipped:?}"),
        }
    }
    let ported = rig.call("poll", scenario::poll_args(&Value::Null)).await.expect("a poll");
    let shipped = connector.poll(None).await.expect("a poll");
    assert_eq!(wire(&ported), scenario::poll_shape(shipped.clone()));
    let again = rig.call("poll", scenario::poll_args(&ported["nextCursor"])).await.expect("a poll");
    let shipped = connector.poll(shipped["nextCursor"].as_str()).await.expect("a poll");
    assert_eq!(wire(&again), scenario::poll_shape(shipped));
    connector.close().await;
    rig.stop().await;
}
