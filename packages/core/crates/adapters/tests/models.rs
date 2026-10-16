//! The shipped adapters' model listings, read from each CLI's recorded answer.

mod support;

use serde_json::{json, Value};
use vorn_agents::models::{parse_choices, ModelChoice};
use vorn_agents::{Agent, Probe, Step};

fn choice(id: &str, label: &str, description: Option<&str>) -> ModelChoice {
    ModelChoice {
        id: id.into(),
        label: label.into(),
        description: description.map(Into::into),
    }
}

fn parse(agent: &Agent, value: &Value) -> Vec<ModelChoice> {
    let list = agent.spec().models.as_ref().unwrap().list.as_ref().unwrap();
    parse_choices(&list.choice, value).unwrap()
}

fn line(v: Value) -> String {
    v.to_string()
}

#[test]
fn normalizes_each_cli_answer_and_drops_unusable_entries() {
    assert_eq!(
        parse(
            &support::agent("claude"),
            &json!([{ "value": "opus[1m]", "resolvedModel": "versioned", "displayName": "Opus" }])
        ),
        [choice("opus[1m]", "Opus", None)]
    );
    assert_eq!(
        parse(
            &support::agent("codex"),
            &json!([
                { "id": "internal", "model": "cli-id", "displayName": "Model" },
                { "id": "fallback", "model": null },
                { "id": "hidden", "hidden": true },
                { "id": "off", "policy": { "state": "disabled" } },
                { "id": "", "name": "empty" },
                "not an object",
                { "id": "cli-id", "name": "Again", "description": "" }
            ])
        ),
        [
            choice("cli-id", "Again", None),
            choice("fallback", "fallback", None)
        ]
    );
    assert_eq!(
        parse(
            &support::agent("copilot"),
            &json!([
                { "modelId": "auto", "name": "Auto", "description": "Let Copilot pick" },
                { "modelId": "opus", "name": "Opus",
                  "_meta": { "copilotUsage": "15x", "copilotEnablement": "enabled" } },
                { "modelId": "blocked", "_meta": { "copilotEnablement": "policy_disabled" } },
                { "modelId": "nulled", "_meta": { "copilotEnablement": null } }
            ])
        ),
        [
            choice("auto", "Auto", Some("Let Copilot pick")),
            choice("opus", "Opus", Some("15x usage"))
        ]
    );
    let opencode = Probe::new(&support::agent("opencode"), "/p").unwrap();
    assert_eq!(
        opencode
            .on_exit(
                true,
                "provider/model\r\nprovider/model\nother/model\nnoise\n a/b c\n/x\n"
            )
            .unwrap(),
        [
            choice("provider/model", "provider/model", None),
            choice("other/model", "other/model", None)
        ]
    );
    assert!(support::agent("gemini").spec().models.is_none());
}

#[test]
fn starts_each_cli_in_its_listing_mode() {
    let configured: Vec<String> = [
        "--profile",
        "work",
        "-c",
        "model=x",
        "--oss",
        "--config=a=b",
        "--yolo",
        "-m",
        "gpt",
        "-p",
    ]
    .map(String::from)
    .to_vec();
    let args = |name: &str| {
        Probe::new(&support::agent(name), "/p")
            .unwrap()
            .arguments(&configured)
    };
    assert_eq!(
        args("codex"),
        [
            "--profile",
            "work",
            "-c",
            "model=x",
            "--oss",
            "--config=a=b",
            "app-server",
            "--stdio"
        ]
    );
    assert_eq!(args("opencode"), ["models"]);
    assert_eq!(
        args("copilot"),
        ["--acp", "--no-remote", "--no-remote-export"]
    );
    assert_eq!(
        args("claude"),
        [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--safe-mode",
            "--strict-mcp-config",
            "--tools",
            ""
        ]
    );
}

#[test]
fn exchanges_each_cli_s_messages_as_recorded() {
    let mut codex = Probe::new(&support::agent("codex"), "/project").unwrap();
    assert_eq!(
        codex.opening().unwrap(),
        [json!({ "id": 1, "method": "initialize",
                 "params": { "clientInfo": { "name": "vorn_model_catalog", "version": "1.0.0" } } })]
    );
    let Step::Send(sent) = codex
        .on_line(&line(json!({ "id": 1, "result": {} })))
        .unwrap()
    else {
        panic!("expected messages");
    };
    assert_eq!(sent[0], json!({ "method": "initialized" }));
    assert_eq!(
        sent[1],
        json!({ "id": 2, "method": "model/list", "params": { "limit": 100, "includeHidden": false } })
    );
    let Step::Send(next) = codex
        .on_line(&line(
            json!({ "id": 2, "result": { "data": [{ "model": "first" }], "nextCursor": "c" } }),
        ))
        .unwrap()
    else {
        panic!("expected the next page");
    };
    assert_eq!(
        next[0],
        json!({ "id": 3, "method": "model/list", "params": { "limit": 100, "includeHidden": false, "cursor": "c" } })
    );

    let mut claude = Probe::new(&support::agent("claude"), "/p").unwrap();
    assert_eq!(
        claude.opening().unwrap(),
        [
            json!({ "type": "control_request", "request_id": "vorn-models", "request": { "subtype": "initialize" } })
        ]
    );
    let done = claude
        .on_line(&line(
            json!({ "type": "control_response", "response": { "subtype": "success",
            "response": { "models": [{ "value": "opus", "displayName": "Opus" }] } } }),
        ))
        .unwrap();
    assert_eq!(done, Step::Done(vec![choice("opus", "Opus", None)]));

    let mut copilot = Probe::new(&support::agent("copilot"), "/proj").unwrap();
    assert_eq!(
        copilot.opening().unwrap()[0]["params"]["protocolVersion"],
        1
    );
    let Step::Send(sent) = copilot
        .on_line(&line(json!({ "id": 1, "result": {} })))
        .unwrap()
    else {
        panic!("expected session/new");
    };
    assert_eq!(
        sent[0],
        json!({ "jsonrpc": "2.0", "id": 2, "method": "session/new",
                "params": { "cwd": "/proj", "mcpServers": [] } })
    );
    let done = copilot
        .on_line(&line(json!({ "id": 2, "result": { "models": { "availableModels": [{ "modelId": "gpt" }] } } })))
        .unwrap();
    assert!(matches!(done, Step::Done(ref m) if m[0].id == "gpt"));
}
