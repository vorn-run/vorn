//! `task_triggers::for_change` against what the TypeScript it replaced
//! answered for the same seeded saves (`tests/fixtures/js-reference/task-triggers.json`).

use serde_json::Value;

#[test]
fn every_recorded_save_fires_what_the_server_fired() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/js-reference/task-triggers.json");
    let text = std::fs::read_to_string(&path).expect("the recorded saves");
    let recorded: Value = serde_json::from_str(&text).expect("JSON");
    let cases = recorded["cases"].as_array().expect("cases");
    assert!(cases.len() > 50);
    for (i, case) in cases.iter().enumerate() {
        let fired = vorn_work::task_triggers::for_change(&case["before"], &case["after"]);
        assert_eq!(Value::Array(fired), case["triggers"], "case {i}");
    }
}
