//! `task_triggers::for_task` against what the TypeScript it replaced
//! answered for the same seeded saves (`tests/fixtures/js-reference/task-triggers.json`):
//! each recorded save, taken a task at a time as vornd now writes them,
//! fires the same triggers in the same order.

use std::collections::HashMap;

use serde_json::Value;

fn tasks(config: &Value) -> impl Iterator<Item = &Value> {
    config
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// A task id as a `Map` key: its JSON, so `1` and `"1"` stay apart.
fn key(id: Option<&Value>) -> String {
    id.map_or_else(|| "undefined".to_owned(), Value::to_string)
}

#[test]
fn every_recorded_save_fires_what_the_server_fired() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/js-reference/task-triggers.json");
    let text = std::fs::read_to_string(&path).expect("the recorded saves");
    let recorded: Value = serde_json::from_str(&text).expect("JSON");
    let cases = recorded["cases"].as_array().expect("cases");
    assert!(cases.len() > 50);
    for (i, case) in cases.iter().enumerate() {
        let before: HashMap<String, &Value> =
            tasks(&case["before"]).map(|t| (key(t.get("id")), t)).collect();
        let fired: Vec<Value> = tasks(&case["after"])
            .filter_map(|t| vorn_work::task_triggers::for_task(before.get(&key(t.get("id"))).copied(), t))
            .collect();
        assert_eq!(Value::Array(fired), case["triggers"], "case {i}");
    }
}
