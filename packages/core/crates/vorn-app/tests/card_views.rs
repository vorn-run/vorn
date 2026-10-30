//! Card views from a manifest alone (`fixtures/kanban`), on a vornd with its own HOME: the switcher's chevron lists them, a view puts the sessions in columns at their sizes without resizing a mini card's session, permission requests show over every size, a drag moves a session only where its columns can, and a view that goes falls back to Grid. `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use std::path::Path;

use serde_json::{json, Value};
use support::{add_project, connect, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::card_view::Ask;
use vorn_app::screen::menus::MENU;
use vorn_app::screen::views::{card_id, column_id};
use vorn_app::sidebar_view::GroupBy;
use vorn_app::state::{LayoutMode, MenuKind};
use vorn_app::ui::{item_id, Event as UiEvent, Ui};
use vorn_app::App;

const KANBAN: &str = "test.kanban/views/kanban";
const GROUPS: &str = "test.kanban/views/groups";

/// Links the fixture's card views, as a person links a folder, and waits for the app to list them.
fn link_kanban(app: &mut App, shot: &mut Shot) {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kanban");
    let linked = app
        .rpc()
        .call("extension:link", json!({ "path": folder }))
        .expect("the views link");
    assert_eq!(linked["id"], "test.kanban", "{linked}");
    app.st.points.stale = true;
    shot.until(app, "the card views", |app, _| {
        (app.st.points.card_views().len() == 2).then_some(())
    });
}

/// A shell in `dir`, once the app has its card.
fn shell(app: &mut App, dir: &Path) -> String {
    let info = app.rpc().call("shell:create", json!(dir)).expect("shell");
    let id = info["id"].as_str().expect("id").to_owned();
    wait_for("the card", || {
        app.pump();
        app.store.session(&id).map(|_| ())
    });
    id
}

/// What vornd says a session's size is.
fn recorded(app: &App, id: &str) -> Option<(u32, u32)> {
    let info = &app.store.session(id)?.info;
    Some((info.cols? as u32, info.rows? as u32))
}

fn menu_index(app: &App, shot: &Shot, label: &str) -> usize {
    let labels = app.menu_labels(&shot.ui);
    labels
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("{label} in {labels:?}"))
}

#[test]
fn the_chevron_lists_card_views_and_kanban_draws_columns_without_resizing_a_session() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let dir = add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    let quiet = shell(&mut app, &dir);
    let busy = shell(&mut app, &dir);
    shot.until(&mut app, "both cards drawn at their size", |app, _| {
        (recorded(app, &quiet).is_some() && recorded(app, &busy).is_some()).then_some(())
    });
    shot.save(&mut app, "views-grid");

    // Without a card view the switcher is Grid and Tabs alone.
    let options = App::control("toolbar", &Act::Open(MenuKind::ViewOptions));
    let chevron = App::control("layout", &Act::ToggleCardViews);
    shot.click(&mut app, options);
    assert!(shot.ui.rect_of(chevron).is_none());
    shot.save(&mut app, "views-switcher-before");
    shot.click(&mut app, options);

    link_kanban(&mut app, &mut shot);
    shot.click(&mut app, options);
    assert!(shot.ui.rect_of(chevron).is_some(), "the chevron shows");
    shot.click(&mut app, chevron);
    assert_eq!(
        app.st.menu.as_ref().map(|m| &m.kind),
        Some(&MenuKind::ViewOptions)
    );
    let labels = app.menu_labels(&shot.ui);
    assert!(
        labels.iter().any(|l| l == "Kanban") && labels.iter().any(|l| l == "Group board"),
        "{labels:?}"
    );
    shot.save(&mut app, "views-switcher-chevron");
    let i = menu_index(&app, &shot, "Kanban");
    shot.click(&mut app, item_id(MENU, i));
    assert_eq!(app.st.layout, LayoutMode::View(KANBAN.into()));
    wait_for("the view saved as the layout", || {
        app.pump();
        let mode = app.store.config.pointer("/defaults/layoutMode").cloned();
        (mode == Some(json!(format!("view:{KANBAN}")))).then_some(())
    });
    app.st.menu = None;

    // The palette offers it too.
    let palette = vorn_app::palette::commands(&app.store, &app.st, "\u{2318}", "\u{2325}");
    assert!(palette.iter().any(|e| e.label == "Show Sessions as Kanban"));

    // One prints and stays a mini card; the other goes quiet and finishes as a line.
    let before = recorded(&app, &busy).expect("its size");
    let quiet_size = recorded(&app, &quiet).expect("its size");
    app.rpc()
        .call(
            "terminal:write",
            json!({ "id": quiet, "data": "echo done\r" }),
        )
        .ok();
    app.rpc()
        .call(
            "terminal:write",
            json!({ "id": busy, "data": "while true; do echo tick; sleep 0.2; done\r" }),
        )
        .ok();
    shot.until(
        &mut app,
        "the busy shell in Running, the quiet one done",
        |app, ui| {
            app.pump();
            let running = ui.rect_of(column_id("printing"))?;
            let mini = ui.rect_of(card_id(&busy))?;
            let line = ui.rect_of(card_id(&quiet))?;
            (mini.x >= running.x && mini.x < running.x + running.w && line.h < mini.h).then_some(())
        },
    );
    shot.until(&mut app, "the quiet shell's last line", |app, _| {
        app.pump();
        let line = app.st.cards.lines.get(&quiet)?.text.as_deref()?;
        (line == "done" || line.ends_with('$') || line.ends_with('#')).then_some(())
    });
    for _ in 0..20 {
        shot.frame(&mut app);
        app.pump();
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(
        recorded(&app, &busy),
        Some(before),
        "a mini card never resizes its session"
    );
    // Only the mini card's terminal stays attached; the line card's lets go until it is selected, for typing.
    app.st.selected = Some(busy.clone());
    shot.frame(&mut app);
    let attached = app.grid().expect("grid").attached();
    assert!(attached.contains(&busy), "{attached:?}");
    assert!(!attached.contains(&quiet), "{attached:?}");
    shot.save(&mut app, "views-kanban");
    shot.click(&mut app, card_id(&quiet));
    assert_eq!(app.st.selected.as_deref(), Some(quiet.as_str()));
    let attached = app.grid().expect("grid").attached();
    assert!(
        attached.contains(&quiet),
        "a selected line card takes the keys: {attached:?}"
    );
    assert_eq!(
        recorded(&app, &quiet),
        Some(quiet_size),
        "typing into it sizes nothing"
    );

    // A request to use a tool shows over the mini card and over the line card.
    for (n, sid) in [&busy, &quiet].into_iter().enumerate() {
        app.st.asks.push(Ask {
            request: format!("r{n}"),
            session: sid.clone(),
            tool: "Bash".into(),
            description: Some("git push origin main".into()),
        });
    }
    shot.frame(&mut app);
    for n in 0..2 {
        let allow = App::control(
            "view-ask",
            &Act::Permission {
                request: format!("r{n}"),
                allow: true,
            },
        );
        assert!(shot.ui.rect_of(allow).is_some(), "request r{n} is drawn");
    }
    shot.save(&mut app, "views-kanban-asking");
    let deny = App::control(
        "view-ask",
        &Act::Permission {
            request: "r1".into(),
            allow: false,
        },
    );
    shot.click(&mut app, deny);
    assert_eq!(app.st.asks.len(), 1, "an answered request goes");
}

#[test]
fn a_drag_moves_a_session_only_between_your_groups() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let dir = add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    let id = shell(&mut app, &dir);
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["sessionGroups"] =
        json!([{ "id": "g1", "name": "Review", "order": 0.0, "workspaceId": "personal" }]);
    app.rpc().call("config:save", config).expect("saved");
    wait_for("the group", || {
        app.pump();
        (!app.store.groups.is_empty()).then_some(())
    });
    link_kanban(&mut app, &mut shot);

    let drag = |app: &mut App, shot: &mut Shot, to: &str| {
        shot.frame(app);
        let from = shot.ui.rect_of(card_id(&id)).expect("the card");
        let over = shot.ui.rect_of(column_id(to)).expect("the column");
        let (x, y) = (over.x + over.w / 2.0, over.y + over.h / 2.0);
        let (dx, dy) = (x - from.x, y - from.y);
        let id = card_id(&id);
        app.event(&mut shot.ui, UiEvent::Move { id, dx, dy, x, y });
        shot.frame(app);
        app.event(&mut shot.ui, UiEvent::Drop { id, dx, dy, x, y });
        shot.frame(app);
    };

    // By status nothing moves: the columns are what each session does.
    app.st.layout = LayoutMode::View(KANBAN.into());
    drag(&mut app, &mut shot, "waiting");
    assert_eq!(
        app.store.session(&id).and_then(|s| s.info.group_id.clone()),
        None
    );

    // By your groups a drop moves the session into the group.
    app.st.layout = LayoutMode::View(GROUPS.into());
    drag(&mut app, &mut shot, "g1");
    wait_for("the session in its new group", || {
        app.pump();
        (app.store.session(&id)?.info.group_id.as_deref() == Some("g1")).then_some(())
    });
    shot.save(&mut app, "views-groups");
}

#[test]
fn a_view_that_goes_falls_back_to_grid() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();

    // Stored for an extension that is not there: Grid, without a word.
    app.st.layout = LayoutMode::from_key("view:gone.ext/views/board");
    shot.until(&mut app, "the grid back", |app, _| {
        (app.st.layout == LayoutMode::Grid).then_some(())
    });
    assert!(app.st.toasts.is_empty());
    app.st.layout = LayoutMode::from_key("list");
    assert_eq!(app.st.layout, LayoutMode::Grid, "the list layout is gone");

    // Its extension removed while it shows: Grid, and a toast says why.
    link_kanban(&mut app, &mut shot);
    app.st.layout = LayoutMode::View(KANBAN.into());
    shot.frame(&mut app);
    app.rpc()
        .call("extension:remove", json!({ "id": "test.kanban" }))
        .expect("removed");
    app.st.points.stale = true;
    shot.until(&mut app, "the grid back with a toast", |app, _| {
        (app.st.layout == LayoutMode::Grid && !app.st.toasts.is_empty()).then_some(())
    });
    assert!(app.st.toasts[0].text.contains("Kanban is off"));
}

#[test]
fn a_terminal_shows_grid_with_a_note_and_a_grouping_heading_selects_and_folds() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let dir = add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    let id = shell(&mut app, &dir);
    link_kanban(&mut app, &mut shot);

    // A terminal's cells cannot draw mini terminals: the grid, and a line saying why.
    app.st.layout = LayoutMode::View(KANBAN.into());
    let mut cells = Ui::cells(&vorn_app::ui_config(1.0));
    let tree = app.frame(&mut cells, vorn_app::ui::cells::units::px(160, 48));
    let note = "Card views draw in the desktop window";
    assert!(tree.nodes.iter().any(|(_, n)| n.label() == Some(note)));
    assert!(
        cells.rect_of(card_id(&id)).is_none(),
        "no view card is laid out"
    );
    assert_eq!(
        app.st.layout,
        LayoutMode::View(KANBAN.into()),
        "the choice is kept"
    );

    // Grouped by Status: a heading's click selects it, and its chevron folds it.
    let status = "vorn.status/groupings/status".to_owned();
    app.st.sidebar_view.group_by = GroupBy::Point(status.clone());
    let key = format!("{status}#Running");
    let select = App::control("sidebar-heading", &Act::SideGroup(Some(key.clone())));
    let fold = App::control("sidebar-heading", &Act::ToggleGroup(key.clone()));
    shot.until(&mut app, "the Running heading", |app, ui| {
        app.pump();
        ui.rect_of(select).map(|_| ())
    });
    shot.click(&mut app, select);
    assert_eq!(
        app.st.side_selection,
        Some(vorn_tree::view::Selection::Group(key.clone()))
    );
    assert_eq!(
        app.st.visible(&app.store).len(),
        1,
        "the view follows the selected heading"
    );
    shot.save(&mut app, "views-heading-selected");
    shot.click(&mut app, fold);
    assert!(app.st.groups_folded.contains(&key));
    assert_eq!(
        app.st.side_selection,
        Some(vorn_tree::view::Selection::Group(key)),
        "folding keeps the selection"
    );
    shot.click(&mut app, select);
    assert_eq!(app.st.side_selection, None, "a second click clears it");
}
