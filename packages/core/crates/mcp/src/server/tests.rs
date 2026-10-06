//! The protocol over a fake Vorn server: what each request is answered with.

use serde_json::{json, Value};

use super::*;
use crate::tools::fake::{block_on, caller, sample_config, FakeRpc};

fn ask(message: Value) -> Option<Value> {
    let rpc = FakeRpc::new(sample_config());
    block_on(Server::new("1.2.3").handle(&rpc, &caller(), &message))
}

fn result_of(message: Value) -> Value {
    let answer = ask(message).expect("a request is answered");
    assert!(answer.get("error").is_none(), "{answer}");
    answer["result"].clone()
}

fn error_of(message: Value) -> Value {
    let answer = ask(message).expect("a request is answered");
    answer["error"].clone()
}

fn initialize(version: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": version, "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }
    })
}

#[test]
fn initialize_agrees_a_version_it_knows() {
    let result = result_of(initialize("2025-03-26"));
    assert_eq!(result["protocolVersion"], "2025-03-26");
    assert_eq!(
        result["serverInfo"],
        json!({ "name": "vorn", "version": "1.2.3" })
    );
    assert_eq!(
        result["capabilities"],
        json!({ "tools": { "listChanged": true } })
    );

    let unknown = result_of(initialize("1999-01-01"));
    assert_eq!(unknown["protocolVersion"], LATEST_PROTOCOL_VERSION);
}

#[test]
fn an_initialize_needs_its_client_info() {
    let bad = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "x" } });
    assert!(!is_initialize_request(&bad));
    assert_eq!(error_of(bad)["code"], INTERNAL_ERROR);

    let mut good = initialize("2025-06-18");
    good.as_object_mut().unwrap().remove("id");
    assert!(
        is_initialize_request(&good),
        "the SDK does not ask for an id"
    );
}

#[test]
fn notifications_and_responses_get_no_answer() {
    assert_eq!(
        ask(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
        None
    );
    assert_eq!(
        ask(json!({ "jsonrpc": "2.0", "id": 4, "result": {} })),
        None
    );
}

#[test]
fn ping_list_and_unknown_methods() {
    assert_eq!(
        result_of(json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" })),
        json!({})
    );
    let listed = result_of(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }));
    assert_eq!(
        listed["tools"].as_array().unwrap().len(),
        crate::tool_count()
    );

    let answer = ask(json!({ "jsonrpc": "2.0", "id": "x", "method": "resources/list" })).unwrap();
    assert_eq!(
        answer,
        json!({ "jsonrpc": "2.0", "id": "x", "error": { "code": METHOD_NOT_FOUND, "message": "Method not found" } })
    );
}

#[test]
fn a_tool_call_without_a_name_is_refused_with_zods_issues() {
    let error =
        error_of(json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {} }));
    assert_eq!(error["code"], INTERNAL_ERROR);
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("\"name\""), "{message}");
}

#[test]
fn unknown_tools_and_bad_arguments_are_tool_errors() {
    let unknown = result_of(
        json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": { "name": "nope" } }),
    );
    assert_eq!(
        unknown,
        json!({ "content": [{ "type": "text", "text": "MCP error -32602: Tool nope not found" }], "isError": true })
    );

    let invalid = result_of(json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "get_task", "arguments": { "id": 5 } }
    }));
    assert_eq!(invalid["isError"], true);
    let text = invalid["content"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with(
            "MCP error -32602: Input validation error: Invalid arguments for tool get_task: "
        ),
        "{text}"
    );
}

#[test]
fn a_result_is_ordered_as_the_sdk_checks_it() {
    let result = result_of(json!({
        "jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": { "name": "get_task", "arguments": { "id": "t1" } }
    }));
    let keys: Vec<&str> = result
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys[0], "content");
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("\"First\""));

    let reordered =
        normalize_result(json!({ "isError": true, "extra": 1, "content": [], "_meta": {} }));
    let keys: Vec<&str> = reordered
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["_meta", "content", "isError", "extra"]);
}
