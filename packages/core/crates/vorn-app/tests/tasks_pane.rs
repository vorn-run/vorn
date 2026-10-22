//! The Tasks pane beside a card, on a vornd with its own HOME: the card menu
//! offers it, it lists only the card's project's tasks, follows a task moved
//! elsewhere, stays while the top bar hides Tasks' button, and Tasks turned
//! off takes it and its menu row away.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{add_project, connect, key, seed_tasks, tree_says, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::points::{kind, scoped};
use vorn_app::screen::menus::MENU;
use vorn_app::screen::term_id;
use vorn_app::state::MenuKind;
use vorn_app::ui::item_id;
use vorn_app::App;

const TASKS: &str = "vorn.tasks";

fn open_menu(shot: &mut Shot, app: &mut App, id: &str) -> Vec<String> {
    shot.hover(app, term_id(id));
    shot.click(
        app,
        App::control("card", &Act::Open(MenuKind::Card(id.to_owned()))),
    );
    app.menu_labels(&shot.ui)
}

#[test]
fn the_tasks_pane_lists_the_cards_project_while_tasks_is_installed() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    shot.send(&mut app, key("Tab", 0, Some("\t")));
    shot.type_line(&mut app, "hello");
    let id = shot.until(&mut app, "the agent's card", |app, _| {
        app.st.selected.clone()
    });
    let project = "web";
    let other = "api";
    let api = v.home.join(other);
    std::fs::create_dir_all(&api).expect("project dir");
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["projects"]
        .as_array_mut()
        .expect("projects")
        .push(json!({ "name": other, "path": api, "preferredAgents": [] }));
    app.rpc().call("config:save", config).expect("saved");
    shot.until(&mut app, "both projects", |app, _| {
        (app.store.projects.len() == 2).then_some(())
    });
    seed_tasks(
        app.rpc(),
        &[
            json!({ "id": "mine-1", "projectName": project, "title": "Mine to do" }),
            json!({ "id": "theirs-1", "projectName": other, "title": "Not on this card" }),
        ],
    );
    shot.until(&mut app, "the pane listed", |app, _| {
        app.st.points.find(TASKS, kind::PANES, "tasks").map(|_| ())
    });

    let labels = open_menu(&mut shot, &mut app, &id);
    let i = labels
        .iter()
        .position(|l| l == "Tasks")
        .unwrap_or_else(|| panic!("Tasks in {labels:?}"));
    shot.click(&mut app, item_id(MENU, i));
    let pane = app
        .st
        .points
        .find(TASKS, kind::PANES, "tasks")
        .expect("pane");
    let tree = scoped(&pane.key(), &id);
    shot.until(&mut app, "its project's task", |app, _| {
        tree_says(app, &tree, "Mine to do").then_some(())
    });
    assert!(!tree_says(&app, &tree, "Not on this card"));
    shot.save(&mut app, "tasks-pane");

    // A task moved anywhere draws the pane again.
    app.rpc()
        .call(
            "task:setStatus",
            json!({ "id": "mine-1", "status": "in_review" }),
        )
        .expect("moved");
    shot.until(&mut app, "the task under In Review", |app, _| {
        tree_says(app, &tree, "In Review").then_some(())
    });

    // Hiding Tasks' button keeps the open pane in the card menu.
    app.rpc()
        .call(
            "extension:setPreference",
            json!({ "id": TASKS, "key": "top_bar", "value": false }),
        )
        .expect("button hidden");
    shot.until(&mut app, "the button hidden", |app, _| {
        (!app.st.points.top_bar().iter().any(|p| p.extension == TASKS)).then_some(())
    });
    let labels = open_menu(&mut shot, &mut app, &id);
    assert!(labels.iter().any(|l| l == "Close Tasks"), "{labels:?}");
    shot.send(&mut app, key("Escape", 0, None));

    app.rpc()
        .call("extension:remove", json!({ "id": TASKS }))
        .expect("Tasks uninstalled");
    app.st.points.stale = true;
    shot.until(&mut app, "the pane gone", |app, _| {
        let listed = app.st.points.find(TASKS, kind::PANES, "tasks").is_some();
        let offered = app
            .st
            .ext
            .items(&id)
            .iter()
            .any(|p| p.extension_id == TASKS);
        (!listed && !offered && !app.st.ext.panes.contains_key(&id)).then_some(())
    });
    let labels = open_menu(&mut shot, &mut app, &id);
    assert!(!labels.iter().any(|l| l.ends_with("Tasks")), "{labels:?}");
}
