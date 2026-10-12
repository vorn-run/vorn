//! The Workflows view driven as a user would, on a vornd with its own HOME:
//! the editor from a blank canvas to a saved workflow, its run, history and
//! export and import, the runs landing with an approval gate, the run
//! prompt's inputs and the missed schedules. `VORN_APP_SHOT=<prefix>` saves
//! a frame of each.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{add_project, connect, key, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::gates::{self, GateOp, Mode, Place};
use vorn_app::screen::menus::MENU;
use vorn_app::screen::MainView;
use vorn_app::state::{Dialog, MenuKind};
use vorn_app::ui::{item_id, mods, Id};
use vorn_app::workflows::{self as wf, Anchor, Missed, Panel, WfAct};
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
        .call("config:load", Value::Null)
        .ok()
        .and_then(|c| c["workflows"].as_array().cloned())
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
    shot.send(&mut app, key("KeyW", MOD | mods::SHIFT, None));
    assert_eq!(app.st.view, MainView::Workflows);
    (app, shot)
}

/// A library pick at the open anchor; the new node's id.
fn pick(shot: &mut Shot, app: &mut App, kind: &str) -> String {
    shot.click(app, control("wf-library", WfAct::Pick(kind.into())));
    assert_eq!(app.st.wf.panel, Panel::Node);
    app.st.wf.node.clone().expect("the new node open")
}

/// Puts `config` straight into vornd as a workflow.
fn seed(app: &mut App, shot: &mut Shot, w: Value) {
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    let id = w["id"].as_str().unwrap_or("").to_owned();
    let mut list = config["workflows"].as_array().cloned().unwrap_or_default();
    list.push(w);
    config["workflows"] = Value::Array(list);
    app.rpc().call("config:save", config).expect("saved");
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
    shot.save(&mut app, "w-start-from");
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
    let trigger = pick(&mut shot, &mut app, "manual");
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
        wf::field_id(Some(&script), "config.scriptContent"),
    );
    shot.send(&mut app, key("KeyA", MOD, None));
    shot.type_text(&mut app, "echo hello");
    shot.click(
        &mut app,
        App::control(
            "wf-select",
            &Act::Open(MenuKind::WfChoice("onError".into())),
        ),
    );
    choose(&mut shot, &mut app, "Carry on anyway");
    shot.save(&mut app, "w-node-script");

    shot.click(&mut app, wf::field_id(None, "name"));
    shot.send(&mut app, key("KeyA", MOD, None));
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
    choose(&mut shot, &mut app, "Export as file\u{2026}");
    assert_eq!(app.st.dialog, Some(Dialog::WfFile));
    let path = v.home.join("hello.vornflow.json");
    shot.ui
        .set_edit_text(wf::file_id(), &path.to_string_lossy());
    shot.save(&mut app, "w-export");
    shot.click(
        &mut app,
        App::control("wf-dialog-foot", &Act::Workflow(WfAct::File)),
    );
    shot.until(&mut app, "the file written", |_, _| {
        path.is_file().then_some(())
    });
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
    assert_eq!(file["name"], "Hello flow");

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
    assert_eq!(app.st.dialog, Some(Dialog::WfFile));
    shot.type_line(&mut app, &path.to_string_lossy());
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
        ["Edit Workflow", "Export as file\u{2026}", "Delete Workflow"]
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
