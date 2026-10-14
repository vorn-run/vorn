//! The command palette, the Workflows pill's waiting badge, the approve and
//! deny keys and the crash screen, driven on a vornd with its own HOME.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{add_project, connect, key, Shot, TestVornd};
use vorn_app::app::MOD;
use vorn_app::screen::{MainView, PALETTE_INPUT};
use vorn_app::state::Dialog;
use vorn_app::ui::{mods, Ui, UiConfig};
use vorn_app::{crash, look, ui_config};

fn palette_open(app: &vorn_app::App) -> bool {
    app.st.dialog == Some(Dialog::Palette)
}

#[test]
fn the_palette_filters_moves_and_runs_a_command() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "api", json!({}));
    let mut shot = Shot::new();

    shot.send(&mut app, key("KeyK", MOD(), None));
    assert!(palette_open(&app));
    assert_eq!(shot.ui.focus(), Some(PALETTE_INPUT));
    shot.save(&mut app, "palette");

    // Mod+K again closes it, as the renderer's toggle does.
    shot.send(&mut app, key("KeyK", MOD(), None));
    assert!(!palette_open(&app));
    shot.send(&mut app, key("KeyK", MOD(), None));

    shot.send(&mut app, key("ArrowDown", 0, None));
    assert_eq!(app.st.palette.active, 1);
    shot.send(&mut app, key("ArrowUp", 0, None));
    shot.send(&mut app, key("ArrowUp", 0, None));
    assert!(
        app.st.palette.active > 1,
        "up from the first wraps to the last"
    );

    shot.type_text(&mut app, "tasks view");
    shot.frame(&mut app);
    assert_eq!(app.st.palette.query, "tasks view");
    assert_eq!(app.st.palette.active, 0, "typing starts at the best match");
    shot.save(&mut app, "palette-query");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    assert!(!palette_open(&app));
    assert_eq!(app.st.view, MainView::Tasks);

    shot.send(&mut app, key("KeyK", MOD(), None));
    shot.type_text(&mut app, "zzzz-nothing");
    shot.save(&mut app, "palette-empty");
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert!(!palette_open(&app));
}

/// A workflow that stops at an approval gate.
fn gated() -> Value {
    json!({
        "id": "wf-gate", "name": "Ship it", "icon": "Rocket", "iconColor": "#22c55e",
        "enabled": true, "workspaceId": "personal",
        "nodes": [
            { "id": "t", "type": "trigger", "label": "Manual", "position": { "x": 0, "y": 0 },
              "config": { "triggerType": "manual" } },
            { "id": "g", "type": "approval", "label": "Sign off", "position": { "x": 0, "y": 100 },
              "config": { "message": "Ship it?" } }
        ],
        "edges": [{ "id": "e", "source": "t", "target": "g" }]
    })
}

#[test]
fn a_waiting_gate_badges_the_workflows_pill() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["workflows"] = json!([gated()]);
    app.rpc().call("config:save", config).expect("saved");
    app.rpc()
        .call("workflow:runManual", json!({ "workflowId": "wf-gate" }))
        .expect("runs");
    shot.until(&mut app, "the gate waiting", |app, _| {
        app.store
            .runs
            .values()
            .any(|r| {
                r["nodeStates"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|n| n["status"] == "waiting"))
            })
            .then_some(())
    });
    shot.frame(&mut app);
    // The pills live in the top bar while the sidebar is closed.
    shot.click(
        &mut app,
        vorn_app::App::control("sidebar", &vorn_app::act::Act::ToggleSidebar),
    );
    // The badge's count is part of the pill's tooltip.
    let pill = vorn_app::ui::item_id(vorn_app::screen::VIEWS, 2);
    shot.hover(&mut app, pill);
    std::thread::sleep(std::time::Duration::from_millis(450));
    shot.save(&mut app, "badge");
    assert_eq!(app.st.view, MainView::Sessions);
    shot.send(&mut app, key("KeyW", MOD() | mods::SHIFT, None));
    assert_eq!(app.st.view, MainView::Workflows);
}

#[test]
fn the_approve_and_deny_keys_wait_for_a_request() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    // Before any agent has asked, the keys are the terminal's.
    shot.frame(&mut app);
    assert!(!app.intercept(&mut shot.ui, &key("KeyA", MOD() | mods::SHIFT, None)));
    app.st.permission_keys = true;
    assert!(app.intercept(&mut shot.ui, &key("KeyA", MOD() | mods::SHIFT, None)));
    assert!(app.intercept(&mut shot.ui, &key("KeyD", MOD() | mods::SHIFT, None)));
}

#[test]
fn the_crash_screen_shows_the_error_and_copies_it() {
    let cfg: UiConfig = ui_config(1.0);
    let mut ui = Ui::headless(vorn_app::ui::RenderMode::Auto, &cfg);
    let error =
        crash::guard::<()>(|| panic!("index out of bounds: the len is 3")).expect_err("caught");
    assert!(error.starts_with("index out of bounds"));
    let lay = |ui: &mut Ui| {
        ui.begin(vorn_app::ui::Rgba::hex(look::SURFACE_BASE));
        ui.layout(crash::view(ui, &error), look::WINDOW)
    };
    lay(&mut ui);
    let copy = ui.rect_of(crash::COPY).expect("Copy error on screen");
    assert!(ui.rect_of(crash::RELOAD).is_some());
    ui.pointer_move(copy.x + 4.0, copy.y + 4.0);
    ui.pointer_down();
    ui.pointer_up();
    let events = ui.take_events();
    let mut reload = None;
    for e in &events {
        reload = reload.or(crash::event(&mut ui, e, &error));
    }
    assert_eq!(reload, None);
    assert_eq!(ui.clipboard().get().as_deref(), Some(error.as_str()));
    if let Some(base) = std::env::var_os("VORN_APP_SHOT") {
        let target = ui.offscreen((look::WINDOW.0 as u32, look::WINDOW.1 as u32));
        lay(&mut ui);
        if !ui.render_offscreen(&target) {
            lay(&mut ui);
            ui.render_offscreen(&target);
        }
        let path = format!("{}-crash.png", base.to_string_lossy());
        ui.save_png(&target, &path).expect("png");
    }
}
