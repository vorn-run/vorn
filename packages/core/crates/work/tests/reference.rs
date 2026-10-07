//! The workflow helpers against what the TypeScript ones answered for the
//! same seeded inputs (`tests/fixtures/js-reference/workflow-functions.json`,
//! recorded from the last release that had them): templates, typed agent
//! output, prompts, item lists, gate rewrites, conditions, fingerprints,
//! script configs, log bounds and doc pages.

use std::path::PathBuf;

use serde_json::{json, Value};
use vorn_work::graph::{
    append_bounded_log, dedupe_fingerprint, evaluate_condition, gate_edit_refusal,
    loop_should_stop, resolve_script_config,
};
use vorn_work::items::to_item_list;
use vorn_work::js_numbers;
use vorn_work::model::Context;
use vorn_work::template::{resolve, resolve_value, StepOutputs};

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/js-reference/workflow-functions.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the fixture is there"))
        .expect("the fixture is JSON")
}

/// Long text as the recording kept it: its length and end.
fn short(v: Value) -> Value {
    match v {
        Value::String(s) if s.encode_utf16().count() > 1000 => {
            let units: Vec<u16> = s.encode_utf16().collect();
            json!({ "length": units.len(), "end": String::from_utf16_lossy(&units[units.len() - 10..]) })
        }
        other => js_numbers(other),
    }
}

#[test]
fn templates_resolve_as_they_did() {
    let f = fixture();
    let contexts: Vec<Option<Context>> = f["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .map(Context::from_json)
        .collect();
    let outputs: StepOutputs = f["outputs"].as_object().unwrap().clone();
    let mut differ = Vec::new();
    for case in f["templates"].as_array().unwrap() {
        let template = case["template"].as_str().unwrap();
        let ctx = contexts[case["context"].as_u64().unwrap() as usize].as_ref();
        let out = case["outputs"].as_bool().unwrap().then_some(&outputs);
        let text = short(Value::String(resolve(template, ctx, out)));
        let value = short(resolve_value(template, ctx, out));
        if text != js_numbers(case["text"].clone()) || value != js_numbers(case["value"].clone()) {
            differ.push(format!(
                "{template:?}: {text} / {value} vs {} / {}",
                case["text"], case["value"]
            ));
        }
    }
    assert!(
        differ.is_empty(),
        "{} differ:\n{}",
        differ.len(),
        differ.join("\n")
    );
}

#[test]
fn typed_output_is_extracted_as_it_was() {
    let f = fixture();
    for case in f["structured"].as_array().unwrap() {
        let got =
            match vorn_work::structured::extract(case["logs"].as_str().unwrap(), &case["schema"]) {
                Ok(o) => json!({ "output": o }),
                Err(e) => json!({ "error": e }),
            };
        assert_eq!(
            js_numbers(got),
            js_numbers(case["result"].clone()),
            "{:?}",
            case["logs"]
        );
    }
}

#[test]
fn prompts_read_as_they_did() {
    let f = fixture();
    for case in f["prompts"].as_array().unwrap() {
        let siblings = case["siblings"].as_array().unwrap();
        assert_eq!(
            vorn_work::prompt::task_prompt(&case["task"], &case["project"], siblings),
            case["taskPrompt"].as_str().unwrap()
        );
        let schema = Some(&case["outputSchema"]).filter(|s| !s.is_null());
        assert_eq!(
            vorn_work::prompt::workflow_prompt(
                case["workflow"]["id"].as_str().unwrap(),
                case["workflow"]["name"].as_str().unwrap(),
                case["step"].as_str().unwrap(),
                case["prompt"].as_str().unwrap(),
                schema
            ),
            case["workflowPrompt"].as_str().unwrap()
        );
    }
}

#[test]
fn graph_helpers_answer_as_they_did() {
    let f = fixture();
    let g = &f["graph"];
    for case in g["itemLists"].as_array().unwrap() {
        let got = match to_item_list(&case["value"]) {
            Ok(list) => match list.wrapper_key {
                Some(k) => json!({ "items": list.items, "wrapperKey": k }),
                None => json!({ "items": list.items }),
            },
            Err(e) => json!({ "error": e }),
        };
        assert_eq!(got, case["result"], "{}", case["value"]);
    }
    for case in g["gateEdits"].as_array().unwrap() {
        let got = gate_edit_refusal(case["editable"].as_str(), case["edited"].as_str());
        assert_eq!(
            got.map_or(Value::Null, Value::String),
            case["result"],
            "{} / {}",
            case["editable"],
            case["edited"]
        );
    }
    for case in g["conditions"].as_array().unwrap() {
        let got = evaluate_condition(
            case["op"].as_str().unwrap(),
            case["a"].as_str().unwrap(),
            case["b"].as_str().unwrap(),
        );
        assert_eq!(Value::Bool(got), case["result"], "{case}");
    }
    for case in g["loopStops"].as_array().unwrap() {
        let got = loop_should_stop(
            Some(&case["until"]),
            case["a"].as_str().unwrap(),
            case["b"].as_str().unwrap(),
        );
        assert_eq!(Value::Bool(got), case["result"], "{case}");
    }
    for case in g["fingerprints"].as_array().unwrap() {
        let ctx = Context::from_json(&case["context"]);
        assert_eq!(
            dedupe_fingerprint(ctx.as_ref()),
            case["result"].as_str().unwrap(),
            "{case}"
        );
    }
    let ctx = Context::from_json(&f["contexts"][1]);
    let outputs: StepOutputs = f["outputs"].as_object().unwrap().clone();
    for case in g["scripts"].as_array().unwrap() {
        let got = resolve_script_config(&case["config"], ctx.as_ref(), Some(&outputs));
        assert_eq!(
            js_numbers(got),
            js_numbers(case["result"].clone()),
            "{case}"
        );
    }
    for case in g["logs"].as_array().unwrap() {
        let mut log = "a".repeat(case["buffer"].as_u64().unwrap() as usize);
        append_bounded_log(
            &mut log,
            &"b".repeat(case["chunk"].as_u64().unwrap() as usize),
        );
        assert_eq!(log.len() as u64, case["length"].as_u64().unwrap());
        assert_eq!(
            &log[log.len().saturating_sub(3)..],
            case["tail"].as_str().unwrap()
        );
    }
}

#[test]
fn doc_markdown_renders_as_it_did() {
    let f = fixture();
    for case in f["markdown"].as_array().unwrap() {
        assert_eq!(
            vorn_work::markdown::to_html(case["md"].as_str().unwrap()),
            case["html"].as_str().unwrap(),
            "{:?}",
            case["md"]
        );
    }
}
