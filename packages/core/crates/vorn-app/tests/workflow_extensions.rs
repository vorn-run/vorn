//! Workflows as extensions draw them, on a vornd with its own HOME: a gate
//! waiting as an attention item the dock shows and the approve key answers,
//! Run workflow as Workflows' contributed menu rows, a step type
//! an installed extension declares in the library and its form, the editor
//! on the kit's graph with a 200-step workflow, and Workflows' button and
//! menus following its switch.

#![cfg(unix)]

mod support;

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{json, Value};
use support::{add_project, connect, key, seed_workflows, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::forms::field_of;
use vorn_app::app::MOD;
use vorn_app::screen::menus::{MENU, SUB};
use vorn_app::screen::MainView;
use vorn_app::state::MenuKind;
use vorn_app::ui::{item_id, mods};
use vorn_app::workflows::{self as wf, WfAct};
use vorn_app::App;

fn runs_of(app: &App, id: &str) -> Vec<Value> {
    app.rpc()
        .call("workflowRun:list", json!({ "workflowId": id, "limit": 20 }))
        .ok()
        .and_then(|r| r.as_array().cloned())
        .unwrap_or_default()
}

fn step(id: &str, kind: &str, config: Value) -> Value {
    json!({ "id": id, "type": kind, "label": id, "slug": id, "position": { "x": 0, "y": 0 }, "config": config })
}

fn workflow(id: &str, name: &str, nodes: Vec<Value>) -> Value {
    let edges: Vec<Value> = nodes
        .windows(2)
        .map(|w| json!({ "id": format!("{}-{}", w[0]["id"].as_str().unwrap(), w[1]["id"].as_str().unwrap()), "source": w[0]["id"], "target": w[1]["id"] }))
        .collect();
    json!({
        "id": id, "name": name, "icon": "Zap", "iconColor": "#eab308",
        "enabled": true, "workspaceId": "personal", "nodes": nodes, "edges": edges
    })
}

fn manual() -> Value {
    step("t", "trigger", json!({ "triggerType": "manual" }))
}

/// Seeds `w` and waits until the app lists it.
fn seed(shot: &mut Shot, app: &mut App, w: Value) {
    let id = w["id"].as_str().unwrap_or("").to_owned();
    seed_workflows(app.rpc(), &[w]);
    shot.until(app, "the workflow listed", |app, _| {
        app.store.workflows.iter().any(|w| w.id == id).then_some(())
    });
}

#[test]
fn a_gate_waits_as_an_attention_item_and_the_approve_key_answers_it() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    seed(
        &mut shot,
        &mut app,
        workflow(
            "wf-sign",
            "Sign off",
            vec![
                manual(),
                step("g", "approval", json!({ "message": "Ready to ship?" })),
            ],
        ),
    );
    app.rpc()
        .call("workflow:runManual", json!({ "workflowId": "wf-sign" }))
        .expect("the run starts");
    let waiting = shot.until(&mut app, "the gate as an attention item", |app, _| {
        let items = app.st.points.attention("personal");
        items
            .first()
            .map(|(p, r)| (p.extension.clone(), r.text("message")))
    });
    assert_eq!(waiting, (wf::ID.to_owned(), "Ready to ship?".to_owned()));
    shot.save(&mut app, "x-attention");

    // Mod+Shift+A, with no agent asking, approves the oldest gate.
    shot.send(&mut app, key("KeyA", MOD() | mods::SHIFT, None));
    let status = shot.until(&mut app, "the run to end", |app, _| {
        app.pump();
        let runs = runs_of(app, "wf-sign");
        let run = runs.first()?;
        (run["status"] != "running").then(|| run["status"].as_str().unwrap_or("").to_owned())
    });
    assert_eq!(status, "success");
    shot.until(&mut app, "the item to go", |app, _| {
        app.st.points.attention("personal").is_empty().then_some(())
    });
}

#[test]
fn run_workflow_comes_from_the_editors_menu_rows() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    seed(
        &mut shot,
        &mut app,
        workflow(
            "wf-nightly",
            "Nightly",
            vec![
                manual(),
                step(
                    "s",
                    "script",
                    json!({ "scriptType": "bash", "scriptContent": "echo hi" }),
                ),
            ],
        ),
    );
    shot.until(&mut app, "the menu's rows", |app, _| {
        let p = app
            .st
            .points
            .find(wf::ID, vorn_app::points::kind::MENUS, "run")?;
        (!app.st.points.listed_rows(p, "personal").is_empty()).then_some(())
    });
    app.st.view = MainView::Sessions;
    shot.click(
        &mut app,
        App::control("toolbar", &Act::Open(MenuKind::NewMenu)),
    );
    let labels = app.menu_labels(&shot.ui);
    let at = labels
        .iter()
        .position(|l| l == "Run workflow")
        .unwrap_or_else(|| panic!("Run workflow in {labels:?}"));
    shot.hover(&mut app, item_id(MENU, at));
    shot.save(&mut app, "x-run-menu");
    let p = app
        .st
        .points
        .find(wf::ID, vorn_app::points::kind::MENUS, "run")
        .expect("the menu");
    let titles: Vec<String> = app
        .st
        .points
        .listed_rows(p, "personal")
        .iter()
        .map(|r| r.text("title"))
        .collect();
    // The seeded Default Task Workflow is offered first.
    assert_eq!(titles, ["Default Task Workflow", "Nightly"]);
    shot.click(&mut app, item_id(SUB, 1));
    let runs = shot.until(&mut app, "the run it started", |app, _| {
        let runs = runs_of(app, "wf-nightly");
        (!runs.is_empty()).then_some(runs)
    });
    assert_eq!(runs.len(), 1);
}

#[test]
fn a_step_type_an_extension_declares_is_in_the_library_with_its_form() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    let shout = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../examples/extensions/shout");
    app.rpc()
        .call("extension:link", json!({ "path": shout }))
        .expect("the example extension links");
    // The list is read again once its contributions change.
    app.st.points.stale = true;
    shot.until(&mut app, "its node type", |app, _| {
        app.st
            .points
            .node_types
            .iter()
            .any(|t| t["id"] == "shout")
            .then_some(())
    });
    let src = vorn_app::library::Sources {
        node_types: &app.st.points.node_types,
        ..vorn_app::library::Sources::default()
    };
    let lines = vorn_app::library::entries(
        &src,
        vorn_app::library::Scope::default(),
        "",
        &[],
        &HashSet::new(),
    );
    let row = lines
        .iter()
        .find(|e| e.key() == "node:shout")
        .expect("Shout in the library");
    assert!(
        matches!(row, vorn_app::library::Entry::Row { label, owner, .. }
        if label == "Shout" && owner.as_deref() == Some("Shout"))
    );

    seed(
        &mut shot,
        &mut app,
        workflow(
            "wf-shout",
            "Shout it",
            vec![
                manual(),
                step("loud", "shout", json!({ "text": "hello", "times": 2 })),
            ],
        ),
    );
    support::views_listed(&mut shot, &mut app);
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    shot.click(
        &mut app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit("wf-shout".into()))),
    );
    shot.click(&mut app, vorn_app::app::canvas::card_id("loud"));
    assert_eq!(app.st.wf.node.as_deref(), Some("loud"));
    let text = field_of(&wf::Bind {
        node: Some("loud".into()),
        path: "config.text".into(),
        conv: wf::Conv::Keep,
        vars: true,
    });
    shot.frame(&mut app);
    assert!(shot.ui.rect_of(text).is_some(), "the schema's text field");
    shot.save(&mut app, "x-shout-form");
}

/// Opens workflow `id`, fitted when `fit`, and drags its step `s000` a little further each frame; the frames' p50 and p99.
fn drag_frames(shot: &mut Shot, app: &mut App, id: &str, fit: bool) -> (Duration, Duration) {
    shot.click(
        app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit(id.into()))),
    );
    if fit {
        shot.click(app, App::control("wf-controls", &Act::Workflow(WfAct::Fit)));
    }
    shot.frame(app);
    let card = vorn_app::app::canvas::card_id("s000");
    let r = shot.ui.rect_of(card).expect("the first step");
    let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    shot.ui.pointer_move(x, y);
    shot.ui.pointer_down();
    app.events(&mut shot.ui);
    let mut frames: Vec<Duration> = Vec::new();
    for i in 1..=40 {
        shot.ui.pointer_move(x + i as f32 * 3.0, y + i as f32);
        let at = thread_time();
        app.events(&mut shot.ui);
        shot.frame(app);
        frames.push(thread_time().saturating_sub(at));
    }
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    let d = app.st.wf.editor.as_ref().expect("editor");
    assert_ne!(
        d.node("s000").unwrap()["position"]["x"],
        0,
        "the drag moved it"
    );
    frames.sort();
    (frames[frames.len() / 2], frames[frames.len() * 99 / 100])
}

/// CPU time this thread has used: the frame's own work, not the turns a
/// busy test runner gives the other tests.
fn thread_time() -> Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `t` is a valid, writable timespec for the call's duration.
    let r = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    assert_eq!(r, 0, "clock_gettime");
    Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

fn steps(count: usize) -> Vec<Value> {
    let mut nodes = vec![manual()];
    for i in 0..count {
        nodes.push(step(
            &format!("s{i:03}"),
            "script",
            json!({ "scriptType": "bash", "scriptContent": format!("echo {i}") }),
        ));
    }
    nodes
}

#[test]
fn the_editor_draws_a_200_step_workflow_within_a_frame() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    seed(&mut shot, &mut app, workflow("wf-small", "Small", steps(1)));
    seed(&mut shot, &mut app, workflow("wf-big", "Big", steps(199)));
    support::views_listed(&mut shot, &mut app);
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    let (small, _) = drag_frames(&mut shot, &mut app, "wf-small", true);
    let (p50, p99) = drag_frames(&mut shot, &mut app, "wf-big", false);
    let (all50, all99) = drag_frames(&mut shot, &mut app, "wf-big", true);
    eprintln!(
        "editor drag frame: 2 steps p50 {small:?}; 200 steps p50 {p50:?}, p99 {p99:?}; all 200 in view p50 {all50:?}, p99 {all99:?}"
    );
    // What 198 more steps add to an edit's frame, so a slow renderer does not count against it.
    for (seen, took) in [("in part", p50), ("whole", all50)] {
        assert!(
            took.saturating_sub(small) < Duration::from_millis(16),
            "200 steps seen {seen} add {:?} to a drag frame",
            took.saturating_sub(small)
        );
    }
}

#[test]
fn uninstalling_workflows_takes_its_button_and_run_workflow_away() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    shot.until(&mut app, "Workflows' menus", |app, _| {
        app.st
            .points
            .find(wf::ID, vorn_app::points::kind::MENUS, "run")
            .map(|_| ())
    });
    support::views_listed(&mut shot, &mut app);
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    assert_eq!(app.st.view, MainView::Workflows);
    app.rpc()
        .call("extension:remove", json!({ "id": wf::ID }))
        .expect("uninstalled");
    app.st.points.stale = true;
    shot.until(&mut app, "its button and menus gone", |app, _| {
        let left = app.st.points.all.iter().any(|p| p.extension == wf::ID);
        (!left && app.st.view == MainView::Sessions).then_some(())
    });
    // Its key goes with its button.
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    assert_eq!(app.st.view, MainView::Sessions);
    shot.frame(&mut app);
    shot.save(&mut app, "x-workflows-off");
}

#[test]
fn save_to_project_writes_the_workflow_into_its_projects_folder() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let root = add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    let mut w = workflow(
        "wf-keep",
        "Keep me",
        vec![
            manual(),
            step(
                "s",
                "script",
                json!({ "scriptType": "bash", "scriptContent": "echo hi" }),
            ),
        ],
    );
    w["nodes"][1]["config"]["projectName"] = json!("web");
    seed(&mut shot, &mut app, w);
    support::views_listed(&mut shot, &mut app);
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    shot.right_click(
        &mut app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit("wf-keep".into()))),
    );
    let labels = app.menu_labels(&shot.ui);
    let at = labels
        .iter()
        .position(|l| l == "Save to project")
        .unwrap_or_else(|| panic!("Save to project in {labels:?}"));
    shot.click(&mut app, item_id(MENU, at));
    let file = root.join(".vorn/workflows/keep-me.toml");
    // The toast follows the write, so the file is whole once it shows.
    shot.until(&mut app, "the toast", |app, _| {
        app.st
            .toasts
            .iter()
            .any(|t| t.text.starts_with("Saved \"Keep me\" to "))
            .then_some(())
    });
    let text = shot.until(&mut app, "the file", |_, _| {
        std::fs::read_to_string(&file)
            .ok()
            .filter(|t| t.starts_with("name = \"Keep me\""))
    });
    assert!(text.contains("echo hi"), "{text}");
}
