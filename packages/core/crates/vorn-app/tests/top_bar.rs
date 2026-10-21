//! The top bar's buttons as extensions contribute them, on a vornd with its
//! own HOME: Tasks' button and key follow its switch, and an installed
//! extension's button comes after the built-ins', opens its view and goes
//! when it is removed.

#![cfg(unix)]

mod support;

use std::fs;

use serde_json::json;
use support::{connect, key, Shot, TestVornd};
use vorn_app::app::MOD;
use vorn_app::points::kind;
use vorn_app::screen::MainView;
use vorn_app::ui::mods;
use vorn_app::App;

const TASKS: &str = "vorn.tasks";

fn titles(app: &App) -> Vec<String> {
    let bar = app.st.points.top_bar();
    bar.iter().map(|p| p.title.clone()).collect()
}

#[test]
fn the_tasks_button_follows_its_switch() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.until(&mut app, "the built-ins' buttons", |app, _| {
        (titles(app) == ["Tasks", "Workflows"]).then_some(())
    });
    shot.send(&mut app, key("KeyT", MOD(), None));
    assert_eq!(app.st.view, MainView::Tasks);

    app.rpc()
        .call("extension:setBuiltin", json!({ "id": TASKS, "on": false }))
        .expect("switched off");
    app.st.points.stale = true;
    shot.until(&mut app, "its button gone", |app, _| {
        (titles(app) == ["Workflows"] && app.st.view == MainView::Sessions).then_some(())
    });
    shot.send(&mut app, key("KeyT", MOD(), None));
    assert_eq!(app.st.view, MainView::Sessions);

    app.rpc()
        .call("extension:setBuiltin", json!({ "id": TASKS, "on": true }))
        .expect("switched on");
    app.st.points.stale = true;
    shot.until(&mut app, "its button back", |app, _| {
        (titles(app) == ["Tasks", "Workflows"]).then_some(())
    });
    shot.send(&mut app, key("KeyT", MOD(), None));
    assert_eq!(app.st.view, MainView::Tasks);
}

#[test]
fn an_installed_extension_adds_its_own_button_after_the_built_ins() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    let dir = v.home.join("board-ext");
    fs::create_dir_all(&dir).expect("folder");
    fs::write(
        dir.join("vorn-extension.toml"),
        r#"id = "acme.board"
name = "Board"
version = "0.1.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "main.js"]

[[contributes.views]]
id = "board"
title = "Board"
icon = "layout-grid"
key = "Mod+Shift+B"
tree = true
"#,
    )
    .expect("manifest");
    fs::write(dir.join("main.js"), "").expect("main");
    let linked = app
        .rpc()
        .call("extension:link", json!({ "path": dir }))
        .expect("linked");
    assert_eq!(linked["id"], "acme.board", "{linked}");
    app.st.points.stale = true;
    shot.until(&mut app, "its button", |app, _| {
        (titles(app) == ["Tasks", "Workflows", "Board"]).then_some(())
    });
    let board = app
        .st
        .points
        .find("acme.board", kind::VIEWS, "board")
        .expect("its view")
        .clone();
    assert!(!board.builtin);

    // The install sheet lists the button with its key.
    let groups = vorn_app::plugins::place_groups(Some(&json!({
        "views": [{ "id": "board", "title": "Board", "key": "Mod+Shift+B" }]
    })));
    assert_eq!(
        groups,
        [("Top bar", vec!["Board button \u{21e7}\u{2318}B".to_owned()])]
    );

    shot.send(&mut app, key("KeyB", MOD() | mods::SHIFT, None));
    assert_eq!(app.st.view, MainView::Extension);
    assert_eq!(app.st.ext_view.as_deref(), Some("acme.board/views/board"));
    shot.frame(&mut app);

    app.rpc()
        .call("extension:remove", json!({ "id": "acme.board" }))
        .expect("removed");
    app.st.points.stale = true;
    shot.until(&mut app, "its button gone", |app, _| {
        (titles(app) == ["Tasks", "Workflows"] && app.st.view == MainView::Sessions).then_some(())
    });
}
