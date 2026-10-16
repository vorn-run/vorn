//! The native screens over a filled store, at a phone's and a tablet's size.

use std::sync::Arc;

use serde_json::{json, Value};
use vorn_phone::model::Link;
use vorn_phone::pages::{self, Do};
use vorn_phone::{config, Model, Phone};
use vornui::backend::Backend;
use vornui::native::{Native, Node};
use vornui::{Rgba, Ui};

const PHONE: (f32, f32) = (390.0, 844.0);
const TABLET: (f32, f32) = (1180.0, 820.0);

fn de<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).expect("fixture")
}

fn phone(size: (f32, f32)) -> Phone {
    let mut model = Model::new(Arc::new(|| {}));
    model.link = Link::Online;
    let s = &mut model.store;
    s.sessions_known = true;
    s.projects = vec![
        de(json!({ "name": "vorn", "path": "/src/vorn", "preferredAgents": [] })),
        de(json!({ "name": "site", "path": "/src/site", "preferredAgents": [] })),
    ];
    for (id, project, status, at) in [
        ("s1", "vorn", "running", 1_700_000_000_000_i64),
        ("s2", "vorn", "waiting", 1_700_000_100_000),
        ("s3", "site", "idle", 1_700_000_200_000),
    ] {
        s.upsert(de(json!({
            "id": id, "agentType": "claude", "projectName": project,
            "projectPath": format!("/src/{project}"), "status": status,
            "createdAt": at, "pid": 1, "branch": "main",
        })));
    }
    let tasks = json!([
        { "id": "t1", "projectName": "vorn", "title": "Fix the build", "description": "It fails on CI.",
          "status": "todo", "order": 0, "createdAt": "2024-01-01T00:00:00Z", "updatedAt": "2024-01-02T00:00:00Z" },
        { "id": "t2", "projectName": "site", "title": "", "description": "",
          "status": "in_progress", "order": 1, "createdAt": "2024-01-01T00:00:00Z", "updatedAt": "2024-01-02T00:00:00Z" },
    ]);
    s.tasks = tasks.as_array().unwrap().iter().cloned().map(de).collect();
    s.config["tasks"] = tasks;
    let nodes = json!([
        { "id": "trig", "type": "trigger", "label": "Start", "position": { "x": 0, "y": 0 },
          "config": { "triggerType": "manual", "inputs": [
            { "key": "topic", "label": "Topic", "type": "text", "required": true },
            { "key": "deep", "label": "Go deep", "type": "boolean" } ] } },
        { "id": "write", "type": "launchAgent", "label": "Write", "position": { "x": 0, "y": 0 },
          "config": { "agentType": "claude", "prompt": "Write" } },
        { "id": "gate", "type": "approval", "label": "Check", "position": { "x": 0, "y": 0 },
          "config": { "message": "Ship it?", "feedback": { "from": "write", "maxRounds": 3 } } },
    ]);
    s.workflows = vec![de(json!({
        "id": "w1", "name": "Draft notes", "icon": "zap", "iconColor": "#fff",
        "nodes": nodes, "edges": [], "enabled": true,
    }))];
    s.runs.insert(
        "r1".into(),
        json!({ "runId": "r1", "workflowId": "w1", "status": "running",
                "startedAt": "2024-01-02T00:00:00Z",
                "nodeStates": [
                  { "nodeId": "trig", "status": "success" },
                  { "nodeId": "write", "status": "success", "logs": "wrote it", "diagnostics": "[+0.1s] started" },
                  { "nodeId": "gate", "status": "waiting", "editableText": "Draft text", "round": 1 } ] }),
    );
    model.permissions.push(json!({
        "requestId": "q1", "terminalId": "s2", "toolName": "Bash", "description": "rm -rf build",
    }));
    let mut p = Phone::new(model);
    p.size = size;
    p
}

fn tree(p: &mut Phone, ui: &mut Ui, route: &str, size: (f32, f32)) -> Node {
    let el = pages::screen(p, ui, route);
    ui.begin(Rgba::TRANSPARENT);
    ui.layout(el, size);
    let mut out = Native::default();
    out.present(ui).expect("native");
    ui.native_tree().expect("tree").clone()
}

fn kinds(n: &Node) -> Vec<&'static str> {
    n.walk().iter().map(|n| n.kind).collect()
}

fn find<'a>(n: &'a Node, label: &str) -> &'a Node {
    n.walk()
        .into_iter()
        .find(|n| n.label == label)
        .unwrap_or_else(|| panic!("no {label:?}"))
}

fn has(n: &Node, label: &str) -> bool {
    n.walk()
        .iter()
        .any(|n| n.label == label || n.value == label)
}

const ROUTES: &[(&str, &[&str])] = &[
    ("sessions", &["bar", "section", "link", "status"]),
    ("sessions-scope", &["bar", "field", "option"]),
    ("launch", &["choice", "option", "field", "button"]),
    ("session:s2", &["bar", "more", "button", "drawn"]),
    ("rename:s1", &["bar", "field"]),
    ("tasks", &["bar", "choice", "option", "link"]),
    ("tasks-scope", &["field", "option"]),
    ("task:t1", &["choice", "option", "section"]),
    ("task-new", &["field", "choice", "button"]),
    ("task-edit:t1", &["field", "button"]),
    ("workflows", &["choice", "link"]),
    ("workflows-trigger", &["option"]),
    ("workflow:w1", &["section", "link"]),
    ("workflow-run:w1", &["field", "toggle"]),
    ("run:r1", &["section", "button", "link", "status"]),
    ("run-log:r1:write", &["copy", "text"]),
    ("review:r1:gate", &["button"]),
    ("gate-changes:r1:gate", &["field", "button"]),
    ("gate-edit:r1:gate", &["field", "button"]),
];

#[test]
fn every_screen_reads_at_phone_and_tablet_sizes() {
    for size in [PHONE, TABLET] {
        let mut p = phone(size);
        for (route, want) in ROUTES {
            let mut ui = Ui::native(&config(3.0));
            let t = tree(&mut p, &mut ui, route, size);
            let k = kinds(&t);
            for w in *want {
                assert!(k.contains(w), "{route} at {size:?} has no {w}: {k:?}");
            }
        }
    }
}

#[test]
fn sessions_group_by_project_waiting_first_and_show_requests() {
    let mut p = phone(PHONE);
    let mut ui = Ui::native(&config(3.0));
    let t = tree(&mut p, &mut ui, "sessions", PHONE);
    let sections: Vec<_> = t
        .walk()
        .into_iter()
        .filter(|n| n.kind == "section")
        .map(|n| n.label.clone())
        .collect();
    assert_eq!(sections, ["vorn", "site"]);
    let t = tree(&mut p, &mut ui, "session:s2", PHONE);
    assert!(has(&t, "Allow") && has(&t, "Deny"));
    let deny = find(&t, "Deny").id.expect("id");
    let d = p.view.act(deny).expect("bound");
    pages::run(&mut p, &mut ui, d);
    assert!(p.model.permissions.is_empty(), "an answered request goes");
}

#[test]
fn tabs_scopes_and_task_moves_act_in_place() {
    let mut p = phone(TABLET);
    let mut ui = Ui::native(&config(3.0));
    let t = tree(&mut p, &mut ui, "tasks", TABLET);
    assert!(has(&t, "Fix the build"));
    let doing = t
        .walk()
        .into_iter()
        .find(|n| n.kind == "option" && n.label.starts_with("In Progress"))
        .and_then(|n| n.id)
        .expect("column tab");
    let d = p.view.act(doing).expect("bound");
    assert_eq!(d, Do::Column("in_progress"));
    pages::run(&mut p, &mut ui, d);
    let t = tree(&mut p, &mut ui, "tasks", TABLET);
    assert!(has(&t, "Untitled") && !has(&t, "Fix the build"));

    let t = tree(&mut p, &mut ui, "task:t1", TABLET);
    let done = t
        .walk()
        .into_iter()
        .find(|n| n.kind == "option" && n.label == "Done")
        .and_then(|n| n.id)
        .expect("status option");
    let d = p.view.act(done).expect("bound");
    pages::run(&mut p, &mut ui, d);
    let t1 = p.model.store.tasks.iter().find(|t| t.id == "t1").unwrap();
    assert_eq!(t1.status.to_string(), "done");

    let t = tree(&mut p, &mut ui, "tasks-scope", TABLET);
    let site = t
        .walk()
        .into_iter()
        .find(|n| n.kind == "option" && n.label.starts_with("site"))
        .and_then(|n| n.id)
        .expect("scope");
    let d = p.view.act(site).expect("bound");
    pages::run(&mut p, &mut ui, d);
    assert_eq!(p.view.tasks_scope.as_deref(), Some("site"));
    assert_eq!(
        p.model.go.take().as_deref(),
        Some("back"),
        "a sheet closes once picked"
    );
}

#[test]
fn a_gate_offers_its_answers_and_the_review_tab_counts_it() {
    let mut p = phone(PHONE);
    let mut ui = Ui::native(&config(3.0));
    let t = tree(&mut p, &mut ui, "workflows", PHONE);
    assert!(has(&t, "Review 1"));
    let t = tree(&mut p, &mut ui, "run:r1", PHONE);
    for l in [
        "Approve",
        "Open review",
        "Edit the text",
        "Request changes",
        "Reject",
        "Draft text",
        "round 1 of 3",
    ] {
        assert!(has(&t, l), "{l} on the run");
    }
    let t = tree(&mut p, &mut ui, "gate-edit:r1:gate", PHONE);
    let save = find(&t, "Save").id.expect("id");
    let field = t
        .walk()
        .into_iter()
        .find(|n| n.kind == "field")
        .and_then(|n| n.id)
        .expect("field");
    assert_eq!(
        ui.edit_text(field),
        "Draft text",
        "the editor starts with the gate's text"
    );
    ui.set_edit_text(field, "Better text");
    let d = p.view.act(save).expect("bound");
    pages::run(&mut p, &mut ui, d);
    let t = tree(&mut p, &mut ui, "run:r1", PHONE);
    assert!(has(&t, "Better text") && has(&t, "Approving sends your edited text."));
}
