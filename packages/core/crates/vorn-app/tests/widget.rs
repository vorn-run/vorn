//! The floating widget on a vornd with its own HOME: its agents, a waiting
//! permission request answered from it, and the folded pill.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use serde_json::json;
use support::{connect, key, wait_for, Shot, TestVornd};
use vorn_app::client::Endpoint;
use vorn_app::ui::{mods, Event, Id, Offscreen, RenderMode, Ui};
use vorn_app::widget::{Request, Widget, COMPACT, FULL};
use vorn_app::{look, ui_config};

/// Draws the widget into `target` at `size` on its window's background.
fn draw(ui: &mut Ui, w: &mut Widget, target: &Offscreen, size: (f32, f32)) {
    for _ in 0..2 {
        ui.begin(vorn_app::ui::Rgba::hex(look::SURFACE_BASE));
        let root = w.view(ui);
        ui.layout(root, size);
        if ui.render_offscreen(target) {
            return;
        }
    }
}

fn save(ui: &mut Ui, w: &mut Widget, size: (f32, f32), name: &str) {
    let Some(base) = std::env::var_os("VORN_APP_SHOT") else {
        let target = ui.offscreen((size.0 as u32, size.1 as u32));
        draw(ui, w, &target, size);
        return;
    };
    let scale = ui.scale();
    let target = ui.offscreen(((size.0 * scale) as u32, (size.1 * scale) as u32));
    draw(ui, w, &target, size);
    let path = format!("{}-{name}.png", base.to_string_lossy());
    ui.save_png(&target, &path).expect("png");
}

/// A press on `id` where the last frame placed it.
fn click(ui: &mut Ui, w: &mut Widget, id: Id) {
    let r = ui
        .rect_of(id)
        .unwrap_or_else(|| panic!("{id:?} is on screen"));
    ui.pointer_move(r.x + r.w / 2.0, r.y + r.h / 2.0);
    ui.pointer_down();
    ui.pointer_up();
    for e in ui.take_events() {
        if let Event::Click(id) = e {
            w.click(ui, id);
        }
    }
}

#[test]
fn the_widget_lists_agents_and_answers_a_request() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let id = shot.until(&mut app, "a shell", |app, _| app.st.selected.clone());

    let ep = Endpoint::find(&v.home).expect("endpoint");
    let mut w = Widget::connect(&ep, std::sync::Arc::new(|| {})).expect("the widget connects");
    wait_for("the shell listed", || {
        w.pump();
        w.agents.iter().any(|a| a.id == id).then_some(())
    });
    let scale = std::env::var("VORN_APP_SHOT_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let mut ui = Ui::headless(RenderMode::Auto, &ui_config(scale));
    save(&mut ui, &mut w, FULL, "widget");

    w.notified(
        "widget:permission-request",
        &json!({ "requestId": "r1", "terminalId": id, "toolName": "Bash",
                 "toolInput": { "command": "npm test" },
                 "permissionSuggestions": [{ "type": "addRules",
                     "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }] }] }),
    );
    save(&mut ui, &mut w, FULL, "widget-request");
    click(&mut ui, &mut w, Id::new("w-allow").child("r1"));
    assert!(w.permissions.is_empty(), "answered and put away");

    click(&mut ui, &mut w, Id::new("w-row").child(id.as_str()));
    assert_eq!(w.out, [Request::Focus(id.clone())]);

    click(&mut ui, &mut w, Id::new("w-collapse"));
    assert!(w.compact);
    save(&mut ui, &mut w, COMPACT, "widget-compact");
    w.notified("widget:permission-cancelled", &json!("r1"));
}
