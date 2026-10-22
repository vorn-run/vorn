//! The Workflows view driven as a user would, on a vornd with its own HOME:
//! the editor from a blank canvas to a saved workflow, its run, history and
//! export and import, the runs landing with an approval gate, the run
//! prompt's inputs and the missed schedules. `VORN_APP_SHOT=<prefix>` saves
//! a frame of each.

#![cfg(unix)]

mod support;

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use support::{
    add_project, connect, key, replace_workflows, seed_runs, seed_workflows, wait_for, Shot,
    TestVornd,
};
use vorn_app::act::Act;
use vorn_app::app::forms::field_of;
use vorn_app::app::MOD;
use vorn_app::gates::{self, GateOp, Mode, Place};
use vorn_app::notify::{self, Recorded};
use vorn_app::screen::menus::MENU;
use vorn_app::screen::MainView;
use vorn_app::state::{Dialog, MenuKind};
use vorn_app::system::Pick as FilePick;
use vorn_app::tasks::{Form, TaskAct};
use vorn_app::ui::{item_id, mods, Id};
use vorn_app::workflows::{self as wf, Anchor, Missed, Panel, Pick, WfAct};
use vorn_app::App;

fn control(place: &str, a: WfAct) -> Id {
    App::control(place, &Act::Workflow(a))
}

#[track_caller]
fn choose(shot: &mut Shot, app: &mut App, label: &str) {
    let labels = app.menu_labels(&shot.ui);
    let i = labels
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("{label:?} in {labels:?}"));
    shot.click(app, item_id(MENU, i));
}

/// The saved workflows, as vornd has them.
fn saved(app: &App) -> Vec<Value> {
    app.rpc()
        .call("workflow:list", Value::Null)
        .ok()
        .and_then(|w| w.as_array().cloned())
        .unwrap_or_default()
}

fn runs_of(app: &App, id: &str) -> Vec<Value> {
    app.rpc()
        .call("workflowRun:list", json!({ "workflowId": id, "limit": 20 }))
        .ok()
        .and_then(|r| r.as_array().cloned())
        .unwrap_or_default()
}

fn workflows_view(v: &TestVornd) -> (App, Shot) {
    let mut app = connect(v);
    add_project(&mut app, v, "web", json!({}));
    let mut shot = Shot::new();
    support::views_listed(&mut shot, &mut app);
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    assert_eq!(app.st.view, MainView::Workflows);
    // The view is Workflows' tree around the canvas, once its component answers.
    shot.until(&mut app, "the editor's tree", |app, _| {
        match app.st.points.trees.get("vorn.workflows/views/workflows") {
            Some(Ok(_)) => Some(()),
            Some(Err(e)) => panic!("the editor's tree cannot be drawn: {e:?}"),
            None => None,
        }
    });
    (app, shot)
}

/// A library pick at the open anchor; the new node's id.
fn pick(shot: &mut Shot, app: &mut App, p: Pick) -> String {
    shot.click(app, control("wf-library", WfAct::Pick(p)));
    assert_eq!(app.st.wf.panel, Panel::Node);
    app.st.wf.node.clone().expect("the new node open")
}

/// Puts `config` straight into vornd as a workflow.
fn seed(app: &mut App, shot: &mut Shot, w: Value) {
    let id = w["id"].as_str().unwrap_or("").to_owned();
    seed_workflows(app.rpc(), &[w]);
    shot.until(app, "the workflow listed", |app, _| {
        app.store.config["workflows"]
            .as_array()?
            .iter()
            .any(|w| w["id"] == id)
            .then_some(())
    });
}

#[test]
fn a_workflow_is_built_saved_run_exported_and_imported() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = workflows_view(&v);
    shot.save(&mut app, "w-landing-empty");

    shot.click(
        &mut app,
        App::control("sidebar", &Act::Workflow(WfAct::New)),
    );
    assert_eq!(app.st.wf.panel, Panel::StartFrom);
    shot.save(&mut app, "w-start-from-first");
    shot.click(&mut app, control("wf-start-blank", WfAct::StartBlank));
    assert_eq!(app.st.wf.panel, Panel::Properties);
    shot.save(&mut app, "w-editor-blank");

    // A trigger, then a script after it.
    let at = Anchor {
        trigger: true,
        ..Anchor::default()
    };
    shot.click(&mut app, control("wf-canvas", WfAct::Library(at)));
    shot.save(&mut app, "w-library-triggers");
    let trigger = pick(&mut shot, &mut app, Pick::TriggerType("manual".into()));
    shot.save(&mut app, "w-node-trigger");
    let plus = Anchor {
        after: Some(trigger.clone()),
        ..Anchor::default()
    };
    shot.click(&mut app, control("wf-plus", WfAct::Library(plus)));
    shot.type_text(&mut app, "scr");
    assert_eq!(app.st.wf.search, "scr");
    shot.save(&mut app, "w-library-search");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    let script = app.st.wf.node.clone().expect("the script open");
    assert_ne!(script, trigger);
    shot.click(
        &mut app,
        field_of(&wf::Bind {
            node: Some(script.clone()),
            path: "config.scriptContent".into(),
            conv: wf::Conv::Keep,
            vars: true,
        }),
    );
    shot.send(&mut app, key("KeyA", MOD(), None));
    shot.type_text(&mut app, "echo hello");
    shot.click(
        &mut app,
        App::control(
            "wf-select",
            &Act::Open(MenuKind::WfChoice(format!("380|{script}|onError"))),
        ),
    );
    choose(&mut shot, &mut app, "Carry on anyway");
    shot.save(&mut app, "w-node-script");

    // The icon picker: a glyph and a colour, kept with the workflow.
    shot.click(
        &mut app,
        App::control("wf-editor", &Act::Open(MenuKind::WfIcon)),
    );
    shot.save(&mut app, "w-icon-picker");
    shot.click(&mut app, control("wf-icon", WfAct::Icon("Rocket".into())));
    shot.click(
        &mut app,
        control("wf-icon", WfAct::IconColor("#22c55e".into())),
    );
    assert_eq!(
        app.st.menu.as_ref().map(|m| &m.kind),
        Some(&MenuKind::WfIcon),
        "stays open"
    );
    shot.send(&mut app, key("Escape", 0, None));

    shot.click(&mut app, wf::field_id(None, "name"));
    shot.send(&mut app, key("KeyA", MOD(), None));
    shot.type_text(&mut app, "Hello flow");
    shot.click(&mut app, control("wf-editor", WfAct::Save));
    let w = shot.until(&mut app, "the workflow saved", |app, _| {
        saved(app).into_iter().find(|w| w["name"] == "Hello flow")
    });
    let id = w["id"].as_str().expect("an id").to_owned();
    let nodes = w["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 2);
    let step = nodes
        .iter()
        .find(|n| n["id"] == script.as_str())
        .expect("script");
    assert_eq!(step["config"]["scriptContent"], "echo hello");
    assert_eq!(step["onError"], "continue");
    assert_eq!(
        (w["icon"].as_str(), w["iconColor"].as_str()),
        (Some("Rocket"), Some("#22c55e"))
    );
    assert_eq!(w["edges"][0]["source"], trigger.as_str());
    assert_eq!(w["edges"][0]["target"], script.as_str());

    // Run from the editor: it lands in the history panel and completes.
    shot.click(&mut app, control("wf-editor", WfAct::RunDraft));
    assert_eq!(app.st.wf.panel, Panel::History);
    let done = shot.until(&mut app, "the run done", |app, _| {
        let r = runs_of(app, &id).into_iter().next()?;
        (r["status"] != "running").then_some(r)
    });
    assert_eq!(done["status"], "success", "{done}");
    shot.until(&mut app, "the history listed", |app, _| {
        (!app.st.wf.history.is_empty()).then_some(())
    });
    shot.save(&mut app, "w-history");

    // The launched run opened at its step, whose full output opens over all.
    let run_key = vorn_app::runs::key(&done);
    shot.click(
        &mut app,
        App::control(
            "run-output",
            &Act::Run(vorn_app::runs::RunAct::Output(run_key, script.clone())),
        ),
    );
    assert_eq!(app.st.dialog, Some(Dialog::Output));
    assert_eq!(app.st.tasks.output.trim(), "hello");
    shot.save(&mut app, "w-full-output");
    shot.send(&mut app, key("Escape", 0, None));
    assert_eq!(app.st.dialog, None);

    // The card's hover toolbar and the canvas after the run.
    shot.click(&mut app, control("wf-editor", WfAct::History));
    shot.hover(
        &mut app,
        App::control("wf-card", &Act::Workflow(WfAct::SelectNode(script.clone()))),
    );
    shot.save(&mut app, "w-card-hover");

    // Export, delete, import: the same workflow comes back.
    shot.click(
        &mut app,
        App::control("wf-editor", &Act::Open(MenuKind::EditorMore)),
    );
    shot.save(&mut app, "w-editor-more");
    let path = v.home.join("hello.vornflow.json");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let (seen, answer) = (Arc::clone(&asked), path.clone());
    app.set_picker(Arc::new(move |p| {
        seen.lock().expect("picks").push(p);
        Some(answer.clone())
    }));
    choose(&mut shot, &mut app, "Export as file\u{2026}");
    assert_eq!(app.st.dialog, None, "the system's save dialog asks");
    shot.until(&mut app, "the file written", |_, _| {
        path.is_file().then_some(())
    });
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
    assert_eq!(file["name"], "Hello flow");
    match asked.lock().expect("picks").as_slice() {
        [FilePick::Save { title, at }] => {
            assert_eq!(title, "Export workflow");
            assert!(at.ends_with("hello-flow.vorn-workflow.json"), "{at:?}");
        }
        other => panic!("{other:?}"),
    }

    shot.click(
        &mut app,
        App::control("wf-editor", &Act::Open(MenuKind::EditorMore)),
    );
    choose(&mut shot, &mut app, "Delete workflow");
    shot.until(&mut app, "deleted", |app, _| {
        (!saved(app).iter().any(|w| w["id"] == id.as_str())).then_some(())
    });
    assert!(app.st.wf.editor.is_none());
    shot.click(
        &mut app,
        App::control("sidebar", &Act::Workflow(WfAct::Import)),
    );
    assert_eq!(
        asked.lock().expect("picks").last(),
        Some(&FilePick::Workflow("Import workflow file".into()))
    );
    let back = shot.until(&mut app, "imported", |app, _| {
        saved(app).into_iter().find(|w| w["name"] == "Hello flow")
    });
    assert_eq!(back["nodes"].as_array().map(Vec::len), Some(2));
    shot.save(&mut app, "w-imported");
}

#[test]
fn the_landing_answers_a_gate_and_the_prompt_asks_for_inputs() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = workflows_view(&v);
    seed(
        &mut app,
        &mut shot,
        json!({
            "id": "wf-gate",
            "name": "Ship it",
            "icon": "Rocket",
            "iconColor": "#22c55e",
            "enabled": true,
            "workspaceId": "personal",
            "nodes": [
                { "id": "t", "type": "trigger", "label": "Manual", "position": { "x": 0, "y": 0 },
                  "config": { "triggerType": "manual", "inputs": [
                      { "key": "who", "label": "Who", "type": "text", "required": true },
                      { "key": "loud", "label": "Loud", "type": "boolean" }
                  ] } },
                { "id": "g", "type": "approval", "label": "Sign off", "position": { "x": 0, "y": 100 },
                  "config": { "message": "Ship to {{inputs.who}}?" } }
            ],
            "edges": [{ "id": "e", "source": "t", "target": "g" }]
        }),
    );
    shot.save(&mut app, "w-sidebar");

    // The run asks for its inputs first.
    shot.hover(
        &mut app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit("wf-gate".into()))),
    );
    shot.click(&mut app, Id::new("wf-side-run").child("wf-gate"));
    assert_eq!(app.st.dialog, Some(Dialog::RunPrompt));
    shot.save(&mut app, "w-run-prompt");
    let go = App::control("wf-dialog-foot", &Act::Workflow(WfAct::PromptRun));
    shot.frame(&mut app);
    assert!(!app.click(&mut shot.ui, go), "Who is required");
    shot.click(&mut app, wf::prompt_id("who"));
    shot.type_text(&mut app, "prod");
    shot.click(
        &mut app,
        App::control(
            "wf-dialog-check",
            &Act::Workflow(WfAct::PromptSet("loud".into(), "true".into())),
        ),
    );
    shot.click(&mut app, go);
    assert_eq!(app.st.dialog, None);

    // The landing shows it waiting at the gate; approving finishes it.
    let r = shot.until(&mut app, "the gate open", |app, _| {
        let r = runs_of(app, "wf-gate").into_iter().next()?;
        r["nodeStates"]
            .as_array()?
            .iter()
            .any(|n| n["status"] == "waiting")
            .then_some(r)
    });
    assert_eq!(r["inputs"]["who"], "prod");
    assert_eq!(r["inputs"]["loud"], true);
    let run = r["runId"].as_str().expect("run id").to_owned();
    shot.click(
        &mut app,
        App::control("sidebar", &Act::Workflow(WfAct::AllRuns)),
    );
    shot.until(&mut app, "the run listed", |app, _| {
        app.st
            .wf
            .runs
            .iter()
            .any(|r| r["runId"] == run.as_str())
            .then_some(())
    });
    shot.click(&mut app, control("wf-run", WfAct::SelectRun(run.clone())));
    shot.save(&mut app, "w-landing-gate");
    shot.click(
        &mut app,
        App::control(
            "wf-gate",
            &Act::Gate(GateOp::Approve(gates::key(Place::Pane, &run, "g"))),
        ),
    );
    shot.until(&mut app, "the run approved", |app, _| {
        let r = runs_of(app, "wf-gate").into_iter().next()?;
        (r["status"] != "running").then_some(())
    });
    shot.save(&mut app, "w-landing-done");

    // The sidebar's menu and the missed schedules.
    shot.right_click(
        &mut app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit("wf-gate".into()))),
    );
    assert_eq!(
        app.menu_labels(&shot.ui),
        [
            "Edit Workflow",
            "Export as file\u{2026}",
            "Save to project",
            "Delete Workflow"
        ]
    );
    shot.save(&mut app, "w-side-menu");
    shot.send(&mut app, key("Escape", 0, None));

    app.st.wf.missed = vec![Missed {
        workflow: "wf-gate".into(),
        name: "Ship it".into(),
        at: "2030-01-01T09:00:00.000Z".into(),
        on: true,
    }];
    app.st.dialog = Some(Dialog::Missed);
    shot.save(&mut app, "w-missed");
    shot.send(&mut app, key("Escape", 0, None));
    assert_eq!(
        app.st.dialog,
        Some(Dialog::Missed),
        "only its buttons close it"
    );
    shot.click(&mut app, control("wf-missed-skip", WfAct::MissedSkip));
    assert_eq!(app.st.dialog, None);
}

#[test]
fn a_finished_run_is_recorded_as_a_notice_and_never_reaches_the_os() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = workflows_view(&v);
    let seen = Recorded::default();
    app.set_notice_sink(Arc::new(seen.clone()));
    let web = v.home.join("web");
    seed(
        &mut app,
        &mut shot,
        json!({
            "id": "wf-tell", "name": "Tell me", "icon": "Rocket", "iconColor": "#22c55e",
            "enabled": true, "workspaceId": "personal",
            "nodes": [
                { "id": "t", "type": "trigger", "label": "Manual", "position": { "x": 0, "y": 0 },
                  "config": { "triggerType": "manual" } },
                { "id": "s", "type": "script", "label": "Wait", "position": { "x": 0, "y": 100 },
                  "config": { "scriptType": "bash", "scriptContent": "sleep 1",
                              "projectName": "web", "projectPath": web } }
            ],
            "edges": [{ "id": "e", "source": "t", "target": "s" }]
        }),
    );
    // Unfocused, as a test window is, so nothing holds a notice back.
    shot.ui.set_window_focused(false);
    app.rpc()
        .call("workflow:run", json!({ "workflowId": "wf-tell" }))
        .expect("the run starts");
    let notice = shot.until(&mut app, "the run's notice", |_, _| {
        seen.notices()
            .into_iter()
            .find(|n| n.body.starts_with("Workflow \"Tell me\""))
    });
    assert_eq!(notice.title, "Vorn");
    assert!(
        notice.body.starts_with("Workflow \"Tell me\" completed"),
        "{notice:?}"
    );
    assert_eq!(notify::os_shown(), 0, "no notifier process is spawned");
}

fn gate(op: GateOp) -> Id {
    App::control("gate", &Act::Gate(op))
}

fn pane_gate(op: GateOp) -> Id {
    App::control("wf-gate", &Act::Gate(op))
}

/// The gate's state in vornd's latest copy of the run.
fn gate_state(app: &App, id: &str) -> Option<Value> {
    let r = runs_of(app, id).into_iter().next()?;
    r["nodeStates"]
        .as_array()?
        .iter()
        .find(|n| n["nodeId"] == "g")
        .cloned()
}

#[test]
fn a_gate_is_edited_sent_back_reviewed_and_approved() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = workflows_view(&v);
    let web = v.home.join("web");
    let list = r#"[{"id":1,"ok":true,"note":"null deref"},{"id":2,"ok":false,"note":"typo"}]"#;
    seed(
        &mut app,
        &mut shot,
        json!({
            "id": "wf-review", "name": "Review it", "icon": "Shield", "iconColor": "#8b5cf6",
            "enabled": true, "workspaceId": "personal",
            "nodes": [
                { "id": "t", "type": "trigger", "label": "Manual", "position": { "x": 0, "y": 0 },
                  "config": { "triggerType": "manual" } },
                { "id": "s", "type": "script", "label": "Find issues", "slug": "find-issues",
                  "position": { "x": 0, "y": 0 },
                  "config": { "scriptType": "bash", "scriptContent": "echo found",
                              "projectName": "web", "projectPath": web } },
                { "id": "g", "type": "approval", "label": "Check findings", "slug": "check-findings",
                  "position": { "x": 0, "y": 0 },
                  "config": { "message": "Keep the real findings.", "edit": list,
                              "feedback": { "from": "s", "maxRounds": 2 },
                              "view": "<h1>Findings</h1>{{steps.find-issues.output}}" } }
            ],
            "edges": [
                { "id": "e1", "source": "t", "target": "s" },
                { "id": "e2", "source": "s", "target": "g" }
            ]
        }),
    );
    shot.hover(
        &mut app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit("wf-review".into()))),
    );
    shot.click(&mut app, Id::new("wf-side-run").child("wf-review"));
    let st = shot.until(&mut app, "the gate open", |app, _| {
        gate_state(app, "wf-review").filter(|g| g["status"] == "waiting")
    });
    assert!(
        st["viewToken"].is_string(),
        "the review page was kept: {st}"
    );
    let run = runs_of(&app, "wf-review")[0]["runId"]
        .as_str()
        .expect("run id")
        .to_owned();
    shot.click(
        &mut app,
        App::control("sidebar", &Act::Workflow(WfAct::AllRuns)),
    );
    shot.until(&mut app, "the run listed", |app, _| {
        app.st
            .wf
            .runs
            .iter()
            .any(|r| r["runId"] == run.as_str())
            .then_some(())
    });
    shot.click(&mut app, control("wf-run", WfAct::SelectRun(run.clone())));
    shot.save(&mut app, "g-pane");

    // The list opens as a table; a row goes and the rewrite is kept.
    let pane = gates::key(Place::Pane, &run, "g");
    shot.click(&mut app, pane_gate(GateOp::Open(pane.clone(), Mode::Edit)));
    shot.save(&mut app, "g-edit-table");
    shot.click(
        &mut app,
        App::control("gate-tab", &Act::Gate(GateOp::Source(pane.clone(), true))),
    );
    shot.save(&mut app, "g-edit-source");
    shot.click(
        &mut app,
        App::control("gate-tab", &Act::Gate(GateOp::Source(pane.clone(), false))),
    );
    shot.click(
        &mut app,
        App::control(
            "gate-row",
            &Act::Gate(GateOp::Remove(pane.clone(), 1, true)),
        ),
    );
    shot.click(&mut app, gate(GateOp::Save(pane.clone())));
    let kept = app.st.runs.gates[&pane]
        .edited
        .clone()
        .expect("a rewrite kept");
    let kept: Value = serde_json::from_str(&kept).expect("json");
    assert_eq!(kept, json!([{ "id": 1, "ok": true, "note": "null deref" }]));

    // Sending the work back runs it again from the script, at round 2.
    shot.click(
        &mut app,
        pane_gate(GateOp::Open(pane.clone(), Mode::Changes)),
    );
    shot.type_text(&mut app, "Drop the typo");
    shot.save(&mut app, "g-changes");
    shot.click(&mut app, gate(GateOp::Send(pane.clone())));
    let st = shot.until(&mut app, "round two", |app, _| {
        gate_state(app, "wf-review")
            .filter(|g| g["status"] == "waiting" && g["round"].as_f64() == Some(2.0))
    });
    assert_eq!(st["feedback"][0]["comment"], "Drop the typo");
    assert!(!gates::can_request_changes(
        Some(&json!({ "feedback": { "from": "s", "maxRounds": 2 } })),
        &st
    ));

    // The trace's Reject asks for a note; the review page opens over it.
    let trace = gates::key(Place::Trace, &run, "g");
    shot.until(&mut app, "the run updated", |app, _| {
        app.store.runs.get(&run).and_then(|r| {
            r["nodeStates"]
                .as_array()?
                .iter()
                .any(|n| n["nodeId"] == "g" && n["round"].as_f64() == Some(2.0))
                .then_some(())
        })
    });
    shot.click(&mut app, gate(GateOp::Open(trace.clone(), Mode::Reject)));
    shot.save(&mut app, "g-reject-note");
    shot.click(&mut app, gate(GateOp::Cancel(trace)));
    shot.click(
        &mut app,
        pane_gate(GateOp::Review(Some((run.clone(), "g".into())))),
    );
    assert_eq!(app.st.dialog, Some(Dialog::GateReview));
    shot.save(&mut app, "g-review");
    shot.click(
        &mut app,
        App::control("gate-review", &Act::Gate(GateOp::Review(None))),
    );
    assert_eq!(app.st.dialog, None);

    // Approving sends the rewrite made in the editor.
    shot.click(&mut app, pane_gate(GateOp::Open(pane.clone(), Mode::Edit)));
    shot.click(
        &mut app,
        App::control(
            "gate-row",
            &Act::Gate(GateOp::Remove(pane.clone(), 0, true)),
        ),
    );
    shot.click(&mut app, pane_gate(GateOp::Approve(pane)));
    let done = shot.until(&mut app, "the run done", |app, _| {
        let r = runs_of(app, "wf-review").into_iter().next()?;
        (r["status"] != "running").then_some(r)
    });
    assert_eq!(done["status"], "success", "{done}");
    let g = done["nodeStates"]
        .as_array()
        .and_then(|l| l.iter().find(|n| n["nodeId"] == "g").cloned())
        .expect("gate state");
    let sent: Value =
        serde_json::from_str(g["editedText"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
    assert_eq!(
        sent,
        json!([{ "id": 2, "ok": false, "note": "typo" }]),
        "{g}"
    );
}

/// Forks, a loop and a request on one canvas.
fn shapes(web: &std::path::Path) -> Value {
    let sc = |cmd: &str| json!({ "scriptType": "bash", "scriptContent": cmd, "projectName": "web", "projectPath": web });
    let n = |id: &str, kind: &str, label: &str, config: Value| json!({ "id": id, "type": kind, "label": label, "slug": id, "position": { "x": 0, "y": 0 }, "config": config });
    json!({
        "id": "wf-shapes", "name": "Shapes", "icon": "Zap", "iconColor": "#eab308",
        "enabled": true, "workspaceId": "personal",
        "nodes": [
            n("t", "trigger", "Manual Trigger", json!({ "triggerType": "manual" })),
            n("a", "script", "Fetch", sc("echo a")),
            n("c", "condition", "Has x", json!({ "variable": "{{steps.a.output}}", "operator": "contains", "value": "x" })),
            n("tb", "script", "True Branch", sc("echo yes")),
            n("fb", "approval", "Check it", json!({ "message": "Look at it" })),
            n("j", "script", "Join", sc("echo join")),
            n("l", "loop", "Each item", json!({ "nodeType": "loop", "mode": "forEach", "items": "{{steps.a.output}}", "bodyNodeIds": ["b1"], "maxIterations": 3 })),
            n("b1", "script", "Per item", sc("echo item")),
            n("h", "httpRequest", "Notify", json!({ "nodeType": "httpRequest", "method": "POST", "url": "https://example.com/hook", "headers": {}, "body": "" })),
            n("ag", "launchAgent", "Draft", json!({ "agentType": "acme", "projectName": "web", "projectPath": web, "headless": true, "prompt": "Write it up" }))
        ],
        "edges": [
            { "id": "e1", "source": "t", "target": "a" },
            { "id": "e2", "source": "a", "target": "c" },
            { "id": "e3", "source": "c", "target": "tb", "conditionBranch": "true" },
            { "id": "e4", "source": "c", "target": "fb", "conditionBranch": "false" },
            { "id": "e5", "source": "tb", "target": "j" },
            { "id": "e6", "source": "fb", "target": "j" },
            { "id": "e7", "source": "j", "target": "l" },
            { "id": "e8", "source": "l", "target": "b1" },
            { "id": "e9", "source": "b1", "target": "h" },
            { "id": "e10", "source": "h", "target": "ag" }
        ]
    })
}

#[test]
fn the_canvas_lays_out_forks_and_loops_pans_zooms_drags_and_connects() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = workflows_view(&v);
    seed(&mut app, &mut shot, shapes(&v.home.join("web")));
    shot.click(
        &mut app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit("wf-shapes".into()))),
    );
    shot.save(&mut app, "c-shapes");
    shot.click(&mut app, control("wf-controls", WfAct::Fit));
    shot.save(&mut app, "c-shapes-fit");
    shot.hover(&mut app, vorn_app::app::canvas::card_id("a"));
    shot.save(&mut app, "c-card-hover");
    for (id, name) in [
        ("c", "f-condition"),
        ("fb", "f-approval"),
        ("l", "f-loop"),
        ("h", "f-http"),
        ("ag", "f-agent"),
    ] {
        shot.click(&mut app, vorn_app::app::canvas::card_id(id));
        assert_eq!(app.st.wf.node.as_deref(), Some(id));
        shot.save(&mut app, name);
    }

    // `{{` in the request's body offers the steps above it.
    let body = field_of(&wf::Bind {
        node: Some("h".into()),
        path: "config.body".into(),
        conv: wf::Conv::Keep,
        vars: true,
    });
    shot.click(&mut app, vorn_app::app::canvas::card_id("h"));
    shot.click(&mut app, body);
    shot.type_text(&mut app, "{{");
    assert!(app.st.wf.ac.is_some(), "the dropdown opens");
    let items = app.ac_items();
    assert!(
        items.iter().any(|i| i.pattern == "{{steps.a.output}}"),
        "{items:?}"
    );
    shot.save(&mut app, "f-autocomplete");
    shot.type_text(&mut app, "outp");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    assert!(app.st.wf.ac.is_none());
    let d = app.st.wf.editor.as_ref().expect("editor");
    let filled = d.node("h").unwrap()["config"]["body"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(
        filled.starts_with("{{steps.") && filled.ends_with(".output}}"),
        "{filled}"
    );
    shot.click(&mut app, control("wf-controls", WfAct::Fit));
    let fitted = app.st.wf.canvas.views["wf-shapes"];
    assert!(
        fitted.zoom < 1.0,
        "a tall workflow fits below 100%: {fitted:?}"
    );

    // Zoom in about the centre, then undo nothing: views are not edits.
    shot.click(&mut app, control("wf-controls", WfAct::ZoomIn));
    let zoomed = app.st.wf.canvas.views["wf-shapes"];
    assert!((zoomed.zoom / fitted.zoom - 1.2).abs() < 1e-9);

    // Dragging a card writes every top-level position on the lattice.
    let card = vorn_app::app::canvas::card_id("h");
    shot.frame(&mut app);
    let r = shot.ui.rect_of(card).expect("the request's card");
    let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    shot.ui.pointer_move(x, y);
    shot.ui.pointer_down();
    app.events(&mut shot.ui);
    for i in 1..=6 {
        let f = i as f32 / 6.0;
        shot.ui.pointer_move(x + 120.0 * f, y + 40.0 * f);
        app.events(&mut shot.ui);
        shot.frame(&mut app);
    }
    shot.save(&mut app, "c-drag");
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    let d = app.st.wf.editor.as_ref().expect("editor");
    let h = d.node("h").expect("request");
    let hx = h["position"]["x"].as_f64().expect("x");
    assert_ne!(hx, 0.0, "moved off the seed column");
    assert_eq!(hx % 8.0, 0.0, "snapped to the lattice");
    assert!(
        d.node("t").unwrap()["position"]["x"].as_f64() != Some(0.0),
        "the layout materialized"
    );

    // Undo puts the seed positions back.
    shot.send(&mut app, key("KeyZ", MOD(), None));
    let d = app.st.wf.editor.as_ref().expect("editor");
    assert_eq!(d.node("h").unwrap()["position"]["x"], 0);
}

#[test]
fn a_template_starts_the_canvas_and_names_what_it_still_needs() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = workflows_view(&v);
    shot.click(
        &mut app,
        App::control("sidebar", &Act::Workflow(WfAct::New)),
    );
    let needs = |t: &Value| {
        t.pointer("/portable/requires")
            .and_then(Value::as_array)
            .is_some_and(|r| !r.is_empty())
    };
    let id = shot.until(&mut app, "the templates", |app, _| {
        let t = app.st.wf.lib.templates.iter().find(|t| needs(t))?;
        t["id"].as_str().map(str::to_owned)
    });
    shot.save(&mut app, "w-start-from");
    // A refreshed catalog can reorder the list, so the row is found by id on the frame it is clicked.
    let place = format!("wf-template.{id}");
    let name = wait_for("the template's row clicked", || {
        shot.frame(&mut app);
        let at = app
            .st
            .wf
            .lib
            .templates
            .iter()
            .position(|t| t["id"] == id.as_str())?;
        let name = app.st.wf.lib.templates[at]["name"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        app.click(&mut shot.ui, control(&place, WfAct::Template(at)))
            .then_some(name)
    });
    assert_eq!(app.st.wf.panel, Panel::Properties);
    let d = app.st.wf.editor.as_ref().expect("the editor");
    assert_eq!(d.wf["name"], name);
    assert!(!d.nodes().is_empty());
    assert!(app.st.toasts.iter().any(|t| t.text.contains("still needs")));
    // A public request is a finished step, so nothing is listed as still needed.
    assert!(app.st.wf.still_needed().is_empty());
    shot.save(&mut app, "w-template");

    // A connector step with no connection is.
    let home = v.home.to_string_lossy().into_owned();
    seed(
        &mut app,
        &mut shot,
        json!({
            "id": "wf-needs", "name": "Needs a hand", "icon": "Zap", "iconColor": "#6366f1",
            "enabled": true, "workspaceId": "personal",
            "nodes": [
                { "id": "t", "type": "trigger", "label": "Manual", "position": { "x": 0, "y": 0 },
                  "config": { "triggerType": "manual" } },
                { "id": "a", "type": "callConnectorAction", "label": "Open an issue",
                  "position": { "x": 0, "y": 120 },
                  "config": { "connectorId": "github", "connectionId": "", "action": "", "args": {}, "cwd": home } }
            ],
            "edges": [{ "id": "e", "source": "t", "target": "a" }]
        }),
    );
    shot.click(&mut app, control("wf-side", WfAct::Edit("wf-needs".into())));
    let needs = app.st.wf.still_needed();
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].req["connectorId"], "github");
    shot.save(&mut app, "w-still-needs");
}

fn icon_run(id: &str, wf: &str, extra: Value) -> Value {
    let mut run = json!({
        "runId": id,
        "workflowId": wf,
        "workflowName": wf,
        "startedAt": "2030-01-01T00:00:00.000Z",
        "completedAt": "2030-01-01T00:00:05.000Z",
        "status": "success",
        "nodeStates": [{ "nodeId": "t", "status": "success" }]
    });
    if let (Some(r), Value::Object(more)) = (run.as_object_mut(), extra) {
        r.extend(more);
    }
    run
}

#[test]
fn projects_workflows_and_runs_draw_their_configured_icons() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let dir = |n: &str| {
        let p = v.home.join(n);
        std::fs::create_dir_all(&p).expect("project dir");
        p
    };
    let trigger = |kind: &str| {
        json!([{ "id": "t", "type": "trigger", "label": "Start", "position": { "x": 0, "y": 0 },
                 "config": { "triggerType": kind } }])
    };
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["projects"] = json!([
        { "name": "web", "path": dir("web"), "preferredAgents": [],
          "icon": "Rocket", "iconColor": "#22c55e" },
        { "name": "api", "path": dir("api"), "preferredAgents": [], "iconColor": "#ef4444" },
        { "name": "docs", "path": dir("docs"), "preferredAgents": [] }
    ]);
    let workflows = json!([
        { "id": "wf-bug", "name": "Triage", "icon": "Rocket", "iconColor": "#f97316",
          "enabled": true, "workspaceId": "personal", "nodes": trigger("manual"), "edges": [] },
        { "id": "wf-plain", "name": "Nightly", "icon": "", "iconColor": "", "enabled": true, "workspaceId": "personal",
          "nodes": trigger("recurring"), "edges": [] },
        { "id": "vorn.connectors:c1:issue.opened", "name": "On issue", "icon": "", "iconColor": "", "enabled": true,
          "workspaceId": "personal", "nodes": trigger("connectorPoll"), "edges": [] }
    ]);
    app.rpc().call("config:save", config).expect("saved");
    replace_workflows(app.rpc(), workflows.as_array().expect("a list"));
    let mut shot = Shot::new();
    shot.until(&mut app, "the projects and workflows", |app, _| {
        let wfs = app.store.config["workflows"].as_array()?;
        let seeded = ["wf-bug", "wf-plain", "vorn.connectors:c1:issue.opened"]
            .iter()
            .all(|id| wfs.iter().any(|w| w["id"] == *id));
        (app.store.projects.len() == 3 && seeded).then_some(())
    });
    let web = &app.store.projects[0];
    assert_eq!(
        (web.icon.as_deref(), web.icon_color.as_deref()),
        (Some("Rocket"), Some("#22c55e"))
    );

    shot.save(&mut app, "icons-projects");

    // The task dialog's project picker draws `ProjectPicker`'s own icons.
    shot.send(&mut app, key("KeyT", MOD(), None));
    shot.click(
        &mut app,
        App::control("toolbar", &Act::Task(TaskAct::New("todo".into()))),
    );
    assert_eq!(app.st.dialog, Some(Dialog::AddTask));
    shot.click(
        &mut app,
        App::control(
            "task-project-picker",
            &Act::Open(MenuKind::TaskProject(Form::Dialog)),
        ),
    );
    assert_eq!(app.menu_labels(&shot.ui), ["web", "api", "docs"]);
    shot.save(&mut app, "icons-task-picker");
    shot.send(&mut app, key("Escape", 0, None));
    shot.send(&mut app, key("Escape", 0, None));
    seed_runs(
        &v,
        &[
            icon_run("run-bug", "wf-bug", json!({})),
            icon_run("run-sched", "wf-plain", json!({})),
            icon_run(
                "run-pr",
                "vorn.connectors:c1:issue.opened",
                json!({ "connectorItem": {
                "connectionId": "c1", "connectorId": "github", "externalId": "42",
                "externalUrl": "https://example.test/o/r/pull/42", "title": "Fix the build"
            } }),
            ),
        ],
    );
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    assert_eq!(app.st.view, MainView::Workflows);
    shot.click(
        &mut app,
        App::control("sidebar", &Act::Workflow(WfAct::AllRuns)),
    );
    shot.until(&mut app, "the runs listed", |app, _| {
        (app.st.wf.runs.len() >= 3).then_some(())
    });
    shot.save(&mut app, "icons-landing");
    shot.click(
        &mut app,
        control("wf-run", WfAct::SelectRun("run-pr".into())),
    );
    shot.save(&mut app, "icons-run-detail");
}
