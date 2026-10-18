//! A card's size reaches its terminal: the cells its box holds are the size
//! vornd records for the session, when the cards open and after the window
//! is resized, on pixels and on cells, in the automatic grid and the
//! Flexible one.

#![cfg(unix)]

mod support;

use serde_json::json;
use support::{add_project, connect, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::screen::menus::MENU;
use vorn_app::state::MenuKind;
use vorn_app::ui::{cells::units, item_id, Ui};
use vorn_app::{ui_config, App};

/// What a card's box holds in whole cells, as the frame measures it.
fn box_cells(app: &App, ui: &Ui, id: &str) -> Option<(u32, u32)> {
    let r = app.term_box(id)?;
    let cell = ui.text.cell();
    Some(((r.w / cell.w).floor() as u32, (r.h / cell.h).floor() as u32))
}

/// What vornd says each session's size is.
fn recorded(app: &App, id: &str) -> Option<(u32, u32)> {
    let info = &app.store.session(id)?.info;
    Some((info.cols? as u32, info.rows? as u32))
}

/// Frames at `size` until every card drawn has its box as its recorded size.
fn until_sized(app: &mut App, ui: &mut Ui, size: (f32, f32), ids: &[String], what: &str) {
    let t = std::time::Instant::now();
    loop {
        app.frame(ui, size);
        app.pump();
        let last: Vec<_> = ids
            .iter()
            .map(|id| (box_cells(app, ui, id), recorded(app, id)))
            .collect();
        // A card the window has no room for is not drawn, and has no box.
        let shown: Vec<_> = last.iter().filter(|(b, _)| b.is_some()).collect();
        if !shown.is_empty() && shown.iter().all(|(b, r)| b == r) {
            return;
        }
        assert!(
            t.elapsed() < support::PATIENCE,
            "{what}: box and recorded size never agreed: {last:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

#[test]
fn a_cards_size_reaches_its_terminal_on_open_and_on_resize() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let dir = add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    let ids: Vec<String> = (0..3)
        .map(|n| {
            // Two shells and an agent's terminal, which start differently.
            let info = if n == 1 {
                let create = json!({
                    "agentType": "acme",
                    "projectName": "demo",
                    "projectPath": dir,
                });
                app.rpc().call("terminal:create", create).expect("agent")
            } else {
                app.rpc().call("shell:create", json!(dir)).expect("shell")
            };
            let id = info["id"].as_str().expect("id").to_owned();
            wait_for("the card", || {
                app.pump();
                app.store.session(&id).map(|_| ())
            });
            id
        })
        .collect();

    // Cells, automatic grid: a terminal's window, then a resized one.
    let mut cells = Ui::cells(&ui_config(1.0));
    for (cols, rows) in [(160, 48), (100, 30)] {
        until_sized(
            &mut app,
            &mut cells,
            units::px(cols, rows),
            &ids,
            &format!("cells, automatic, {cols}x{rows}"),
        );
    }

    // Pixels, automatic grid.
    for size in [(1400.0, 900.0), (1000.0, 640.0)] {
        shot.size = size;
        until_sized(
            &mut app,
            &mut shot.ui,
            size,
            &ids,
            &format!("pixels, automatic, {size:?}"),
        );
    }

    // Pixels, Flexible: chosen from View options, as a person does.
    shot.size = (1400.0, 900.0);
    shot.click(
        &mut app,
        App::control("toolbar", &Act::Open(MenuKind::ViewOptions)),
    );
    let labels = app.menu_labels(&shot.ui);
    let i = labels
        .iter()
        .position(|l| l == "Flexible")
        .unwrap_or_else(|| panic!("Flexible in {labels:?}"));
    shot.click(&mut app, item_id(MENU, i));
    assert_eq!(app.st.columns, -1);
    for size in [(1400.0, 900.0), (1100.0, 700.0)] {
        shot.size = size;
        until_sized(
            &mut app,
            &mut shot.ui,
            size,
            &ids,
            &format!("pixels, flexible, {size:?}"),
        );
    }

    // Cells, Flexible.
    let mut cells = Ui::cells(&ui_config(1.0));
    for (cols, rows) in [(160, 48), (100, 30)] {
        until_sized(
            &mut app,
            &mut cells,
            units::px(cols, rows),
            &ids,
            &format!("cells, flexible, {cols}x{rows}"),
        );
    }
}
