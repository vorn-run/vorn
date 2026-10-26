use vorn_extension_sdk::testing::FakeHost;
use vorn_extension_sdk::{dispatch, Ctx, Event, Extension, Host, Init};

use super::*;

fn host() -> FakeHost {
    FakeHost::default()
        .answer("sessions.list", |_| {
            Ok(json!([
                { "id": "t1", "name": "api", "agent": "acme", "status": "running" },
                { "id": "t2", "name": "web", "agent": "acme", "status": "waiting" }
            ]))
        })
        .answer("permissions.resolve", |_| Ok(Value::Null))
}

fn started(host: &mut FakeHost) -> Widget {
    Widget::init(&Init::default(), host).unwrap()
}

fn asked_bash() -> Event {
    Event {
        name: AGENT_PERMISSION.into(),
        data: json!({
            "requestId": "r1", "terminalId": "t1", "toolName": "Bash",
            "toolInput": { "command": "npm test" },
            "permissionSuggestions": [
                { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }] }
            ]
        }),
    }
}

fn asked_question() -> Event {
    Event {
        name: AGENT_PERMISSION.into(),
        data: json!({
            "requestId": "r2", "terminalId": "t2", "toolName": "AskUserQuestion",
            "questions": [{ "header": "Colour", "question": "Which theme?",
                            "options": [{ "label": "Dark" }, { "label": "Light" }] }]
        }),
    }
}

/// The tree `point` draws, as JSON text, checked as the host checks it.
fn drawn(w: &mut Widget, h: &mut FakeHost, point: &str) -> String {
    let args = json!({ "point": point, "surface": {} });
    let answer = dispatch(w, h, tree::RENDER, args, &Ctx::default()).unwrap();
    tree::read(&answer).expect("a tree the host draws");
    answer.to_string()
}

fn click(w: &mut Widget, h: &mut FakeHost, point: &str, id: &str) -> Result<Value, Error> {
    let args = json!({ "point": point, "event": { "id": id, "kind": "click" } });
    dispatch(w, h, tree::EVENT, args, &Ctx::default())
}

#[test]
fn a_request_reads_as_its_description_or_its_input() {
    assert_eq!(
        asked(&json!({ "description": " Run the tests " })),
        "Run the tests"
    );
    assert_eq!(
        asked(&json!({ "toolInput": { "command": "cargo test" } })),
        "cargo test"
    );
    let long = "x".repeat(400);
    assert_eq!(
        asked(&json!({ "toolInput": { "command": long } }))
            .chars()
            .count(),
        INPUT_MAX
    );
}

#[test]
fn a_suggestion_reads_as_the_rule_it_keeps() {
    let rule = |r: &str| json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": r }] });
    assert_eq!(
        suggestion_label(&rule("src/**"), "Bash"),
        "Allow Bash in src/"
    );
    assert_eq!(
        suggestion_label(&rule("npm test"), "Bash"),
        "Always allow `npm test`"
    );
    assert_eq!(suggestion_label(&rule(""), "Bash"), "Always allow Bash");
    let mode = json!({ "type": "setMode", "mode": "acceptEdits" });
    assert_eq!(suggestion_label(&mode, "Edit"), "Auto-accept edits");
}

#[test]
fn answers_wait_for_every_question() {
    let o = |id: &str, picked: bool| json!({ "id": id, "request": "r:1", "label": id, "picked": picked });
    let mut opts = vec![o("r:1:0:0", true), o("r:1:0:1", false), o("r:1:1:0", false)];
    assert_eq!(answers(&opts, "r:1"), None);
    opts[2]["picked"] = json!(true);
    assert_eq!(
        answers(&opts, "r:1"),
        Some(json!({ "0": "r:1:0:0", "1": "r:1:1:0" }))
    );
}

#[test]
fn a_fresh_instance_lists_the_agents_and_the_line_counts_them() {
    let mut h = host();
    let mut w = started(&mut h);
    assert_eq!(h.rows(AGENTS).unwrap().len(), 2);
    let line = drawn(&mut w, &mut h, "compact");
    assert!(line.contains(r#""label":"running""#), "{line}");
    assert!(line.contains(r#""label":"waiting""#), "{line}");
    assert!(!line.contains("No active agents"), "{line}");
    let window = drawn(&mut w, &mut h, "widget");
    assert!(window.contains("1/2 running"), "{window}");
    assert!(window.contains("show:t2"), "{window}");
    let mut empty = FakeHost::default().answer("sessions.list", |_| Ok(json!([])));
    let mut w = started(&mut empty);
    assert!(drawn(&mut w, &mut empty, "compact").contains("No active agents"));
    assert!(drawn(&mut w, &mut empty, "widget").contains("No active agents"));
}

#[test]
fn a_request_shows_its_rule_and_allow_keeps_it_then_forgets_the_request() {
    let mut h = host();
    let mut w = started(&mut h);
    w.on_events(&mut h, vec![asked_bash()]);
    let window = drawn(&mut w, &mut h, "widget");
    for shown in ["npm test", "Always allow `npm test`", "allow:r1", "deny:r1"] {
        assert!(window.contains(shown), "{shown} in {window}");
    }
    assert_eq!(
        click(&mut w, &mut h, "widget", "suggest:r1:0"),
        Ok(Value::Null)
    );
    assert_eq!(
        h.asked("permissions.resolve"),
        [json!({ "requestId": "r1", "allow": true, "suggestion": 0 })]
    );
    assert!(h.rows(PERMISSIONS).unwrap().is_empty());
    assert!(h.rows(SUGGESTIONS).unwrap().is_empty());
    assert!(!drawn(&mut w, &mut h, "widget").contains("npm test"));
}

#[test]
fn a_question_is_submitted_once_each_has_a_pick() {
    let mut h = host();
    let mut w = started(&mut h);
    w.on_events(&mut h, vec![asked_question()]);
    let window = drawn(&mut w, &mut h, "widget");
    assert!(window.contains("COLOUR: Which theme?"), "{window}");
    assert!(!window.contains("submit:r2"), "nothing picked yet");
    click(&mut w, &mut h, "widget", "pick:r2:0:1").unwrap();
    assert!(drawn(&mut w, &mut h, "widget").contains("submit:r2"));
    click(&mut w, &mut h, "widget", "submit:r2").unwrap();
    assert_eq!(
        h.asked("permissions.resolve"),
        [json!({ "requestId": "r2", "allow": true, "answers": { "0": "Light" } })]
    );
    assert!(h.rows(OPTIONS).unwrap().is_empty());
}

#[test]
fn deny_and_a_closed_request_forget_it() {
    let mut h = host();
    let mut w = started(&mut h);
    w.on_events(&mut h, vec![asked_bash(), asked_question()]);
    click(&mut w, &mut h, "widget", "deny:r1").unwrap();
    assert_eq!(
        h.asked("permissions.resolve"),
        [json!({ "requestId": "r1", "allow": false })]
    );
    let closed = Event {
        name: AGENT_PERMISSION_CLOSED.into(),
        data: json!({ "requestId": "r2" }),
    };
    w.on_events(&mut h, vec![closed]);
    assert!(h.rows(PERMISSIONS).unwrap().is_empty());
    assert!(h.rows(OPTIONS).unwrap().is_empty());
}

#[test]
fn its_buttons_fold_raise_and_focus_through_the_answer() {
    let mut h = host();
    let mut w = started(&mut h);
    let said = |w: &mut Widget, h: &mut FakeHost, id| click(w, h, "widget", id).unwrap();
    assert_eq!(
        said(&mut w, &mut h, "collapse"),
        json!({ "window": "compact" })
    );
    assert_eq!(
        said(&mut w, &mut h, "expand"),
        json!({ "window": "widget" })
    );
    assert_eq!(said(&mut w, &mut h, "vorn"), json!({ "show": true }));
    assert_eq!(said(&mut w, &mut h, "show:t1"), json!({ "focus": "t1" }));
    let err = click(&mut w, &mut h, "widget", "nope").unwrap_err();
    assert_eq!(err.code, "bad-request");
}
