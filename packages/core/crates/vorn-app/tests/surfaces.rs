//! Each Wave 1 surface driven as a user would, on a vornd with its own
//! HOME: the sidebar and the add-project dialog, the cards with their menus,
//! rename, close, the intent bar, the focused stage and the tabs layout, the
//! shortcuts panel and filters, the ended strip, and what a restart leaves:
//! the restored banner, recent sessions and the headless pill; a card's
//! minimize, expand and collapse with the dock's pills, and the view pills.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use support::{add_project, connect, key, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::screen::menus::MENU;
use vorn_app::screen::{
    intent_id, term_id, MainView, PROJECT_NAME, PROJECT_PATH, VIEWS, VIEW_ORDER,
};
use vorn_app::state::{Dialog, Filter, LayoutMode, MenuKind};
use vorn_app::system::Pick;
use vorn_app::ui::{item_id, mods, Id};
use vorn_app::App;

fn control(place: &str, act: Act) -> Id {
    App::control(place, &act)
}

/// Picks the open menu's entry `label`.
#[track_caller]
fn choose(shot: &mut Shot, app: &mut App, label: &str) {
    let labels = app.menu_labels(&shot.ui);
    let i = labels
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("{label:?} in {labels:?}"));
    shot.click(app, item_id(MENU, i));
}

fn menu(app: &App) -> Option<MenuKind> {
    app.st.menu.as_ref().map(|m| m.kind.clone())
}

fn escape(shot: &mut Shot, app: &mut App) {
    shot.send(app, key("Escape", 0, None));
    shot.frame(app);
}

/// A new shell from Ctrl+`, once its screen is there.
fn new_shell(shot: &mut Shot, app: &mut App) -> String {
    let before: Vec<String> = app
        .store
        .sessions
        .iter()
        .map(|s| s.info.id.clone())
        .collect();
    shot.send(app, key("Backquote", mods::CTRL, Some("`")));
    let id = shot.until(app, "a new shell", |app, _| {
        app.st.selected.clone().filter(|s| !before.contains(s))
    });
    shot.until(app, "its screen", |app, _| app.pane(&id).map(|_| ()));
    id
}

#[test]
fn the_sidebar_adds_and_edits_a_project_through_the_dialog() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.frame(&mut app);
    assert!(app.st.sidebar, "the sidebar starts open");
    shot.save(&mut app, "sidebar-empty");

    let dir = v.home.join("my-app");
    std::fs::create_dir_all(&dir).expect("dir");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let (seen, answer) = (Arc::clone(&asked), dir.clone());
    app.set_picker(Arc::new(move |p| {
        seen.lock().expect("picks").push(p);
        Some(answer.clone())
    }));
    shot.click(&mut app, control("sidebar", Act::AddProject));
    assert_eq!(app.st.dialog, Some(Dialog::Project(None)));
    shot.save(&mut app, "add-project-empty");
    shot.click(&mut app, control("project-dialog", Act::BrowseFolder));
    shot.until(&mut app, "the folder picked", |_, ui| {
        (ui.edit_text(PROJECT_PATH) == dir.to_string_lossy()).then_some(())
    });
    assert_eq!(
        *asked.lock().expect("picks"),
        [Pick::Folder("Select Project Folder".into())]
    );
    assert_eq!(
        shot.ui.edit_text(PROJECT_NAME),
        "my-app",
        "the folder names it"
    );
    shot.click(
        &mut app,
        control("project-icon", Act::ProjectIcon("Rocket".into())),
    );
    shot.click(
        &mut app,
        control("project-color", Act::ProjectColor("#22c55e".into())),
    );
    shot.save(&mut app, "add-project");

    shot.click(&mut app, control("project-dialog", Act::SaveProject));
    assert_eq!(app.st.dialog, None);
    shot.until(&mut app, "the project saved", |app, _| {
        let saved = app.rpc().call("config:load", Value::Null).ok()?;
        (saved["projects"][0]["name"] == "my-app").then_some(())
    });
    assert_eq!(app.store.projects[0].path, dir.to_string_lossy());
    let saved = &app.store.projects[0];
    assert_eq!(saved.icon.as_deref(), Some("Rocket"));
    assert_eq!(saved.icon_color.as_deref(), Some("#22c55e"));
    assert_eq!(saved.host_ids, Some(json!(["local"])));
    assert_eq!(saved.workspace_id.as_deref(), Some("personal"));
    shot.until(&mut app, "the toast", |app, _| {
        app.st
            .toasts
            .iter()
            .any(|t| t.text == "Project \"my-app\" added")
            .then_some(())
    });
    shot.save(&mut app, "sidebar");

    // Its row filters the grid; its context menu edits it.
    let row = control("sidebar-project", Act::Project(Some("my-app".into())));
    shot.click(&mut app, row);
    assert_eq!(app.st.project.as_deref(), Some("my-app"));
    shot.right_click(&mut app, row);
    assert_eq!(menu(&app), Some(MenuKind::Project("my-app".into())));
    assert_eq!(
        app.menu_labels(&shot.ui),
        ["Edit Project", "Delete Project"]
    );
    shot.save(&mut app, "project-menu");
    choose(&mut shot, &mut app, "Edit Project");
    assert_eq!(app.st.dialog, Some(Dialog::Project(Some("my-app".into()))));
    shot.send(&mut app, key("Backspace", 0, None));
    shot.type_line(&mut app, "P");
    shot.until(&mut app, "the rename saved", |app, _| {
        (app.store.projects.first()?.name == "my-apP").then_some(())
    });
    assert_eq!(
        app.st.project.as_deref(),
        Some("my-apP"),
        "the filter follows"
    );

    // Delete asks first, in the same menu.
    shot.right_click(
        &mut app,
        control("sidebar-project", Act::Project(Some("my-apP".into()))),
    );
    choose(&mut shot, &mut app, "Delete Project");
    assert!(app
        .menu_labels(&shot.ui)
        .contains(&"Confirm delete?".to_owned()));
    choose(&mut shot, &mut app, "Confirm delete?");
    shot.until(&mut app, "the project deleted", |app, _| {
        app.store.projects.is_empty().then_some(())
    });
    assert_eq!(app.st.project, None);

    // The view modes and the sidebar toggle.
    shot.click(
        &mut app,
        control("sidebar-mode", Act::View(MainView::Workflows)),
    );
    assert_eq!(app.st.view, MainView::Workflows);
    shot.save(&mut app, "sidebar-workflows");
    shot.send(&mut app, key("KeyB", MOD, None));
    assert!(!app.st.sidebar);
}

#[test]
fn cards_take_their_menus_rename_close_and_the_intent_bar() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    shot.click(
        &mut app,
        control("sidebar-project", Act::Project(Some("demo".into()))),
    );
    let a = new_shell(&mut shot, &mut app);
    let b = new_shell(&mut shot, &mut app);
    let c = new_shell(&mut shot, &mut app);
    std::thread::sleep(std::time::Duration::from_millis(300));
    shot.save(&mut app, "cards");

    // The toolbar's plus.
    shot.click(&mut app, control("toolbar", Act::Open(MenuKind::Grid)));
    assert_eq!(menu(&app), Some(MenuKind::Grid));
    let labels = app.menu_labels(&shot.ui);
    assert!(labels.starts_with(&[
        "New session in\u{2026}".into(),
        "New terminal in\u{2026}".into()
    ]));
    shot.save(&mut app, "grid-menu");
    escape(&mut shot, &mut app);
    assert_eq!(menu(&app), None);

    // A right click on the grid's empty cell opens the same menu there.
    app.st.columns = 2;
    shot.frame(&mut app);
    let r = shot.ui.rect_of(Id::new("grid")).expect("grid");
    shot.ui.pointer_move(r.x + r.w * 0.75, r.y + r.h * 0.75);
    shot.ui.pointer_down_secondary();
    app.events(&mut shot.ui);
    shot.frame(&mut app);
    assert_eq!(menu(&app), Some(MenuKind::Grid));
    shot.save(&mut app, "grid-context-menu");
    escape(&mut shot, &mut app);
    app.st.columns = 0;

    // A card's More menu starts a terminal beside it.
    shot.hover(&mut app, term_id(&a));
    shot.click(
        &mut app,
        control("card", Act::Open(MenuKind::Card(a.clone()))),
    );
    assert_eq!(menu(&app), Some(MenuKind::Card(a.clone())));
    shot.save(&mut app, "card-menu");
    choose(&mut shot, &mut app, "New terminal");
    let d = shot.until(&mut app, "the terminal beside it", |app, _| {
        app.st
            .selected
            .clone()
            .filter(|s| ![&a, &b, &c].contains(&s))
    });

    // The terminal's context menu.
    shot.right_click(&mut app, term_id(&b));
    assert_eq!(menu(&app), Some(MenuKind::Terminal(b.clone())));
    assert_eq!(
        app.menu_labels(&shot.ui)[..4],
        ["Copy", "Paste", "Fit to this device", "Lock size"]
    );
    shot.save(&mut app, "terminal-menu");
    choose(&mut shot, &mut app, "Lock size");
    assert!(app.st.locked.contains(&b));

    // F2 renames the selected card in place.
    shot.click(&mut app, control("card-box", Act::Select(b.clone())));
    shot.send(&mut app, key("F2", 0, None));
    assert_eq!(app.st.renaming.as_deref(), Some(b.as_str()));
    shot.send(&mut app, key("KeyA", MOD, Some("a")));
    shot.type_text(&mut app, "build");
    shot.save(&mut app, "rename");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    shot.until(&mut app, "the new name from vornd", |app, _| {
        let list = app.rpc().call("terminal:listActive", Value::Null).ok()?;
        list.as_array()?
            .iter()
            .any(|s| s["id"] == b.as_str() && s["displayName"] == "build")
            .then_some(())
    });

    // The intent bar runs a line in its card's terminal.
    shot.click(&mut app, intent_id(&c));
    shot.type_text(&mut app, "echo intent-7");
    shot.save(&mut app, "intent-bar");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    shot.until(&mut app, "the intent's answer", |app, _| {
        app.pane(&c)?
            .text()
            .lines()
            .any(|l| l.trim() == "intent-7")
            .then_some(())
    });

    // Close asks first; Escape keeps the card.
    shot.hover(&mut app, term_id(&d));
    shot.click(&mut app, control("card", Act::AskClose(d.clone())));
    assert_eq!(app.st.confirm_close.as_deref(), Some(d.as_str()));
    shot.save(&mut app, "close-confirm");
    escape(&mut shot, &mut app);
    assert_eq!(app.st.confirm_close, None);
    shot.hover(&mut app, term_id(&d));
    shot.click(&mut app, control("card", Act::AskClose(d.clone())));
    shot.click(&mut app, control("confirm", Act::CloseSession(d.clone())));
    assert!(app.store.session(&d).is_none());

    // A shell that exits leaves its ended strip.
    shot.click(&mut app, control("card-box", Act::Select(a.clone())));
    shot.type_line(&mut app, "exit");
    shot.until(&mut app, "the ended strip", |app, _| {
        app.store.session(&a).filter(|s| s.ended()).map(|_| ())
    });
    shot.save(&mut app, "ended");
    shot.click(&mut app, control("ended", Act::CloseSession(a.clone())));
    assert!(app.store.session(&a).is_none());
}

#[test]
fn the_focused_stage_tabs_shortcuts_and_filters() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    let a = new_shell(&mut shot, &mut app);
    let b = new_shell(&mut shot, &mut app);

    // Mod+1 and Mod+] move between cards; Mod+O puts one on the stage.
    shot.send(&mut app, key("Digit1", MOD, Some("1")));
    assert_eq!(app.st.selected.as_deref(), Some(a.as_str()));
    shot.send(&mut app, key("BracketRight", MOD, Some("]")));
    assert_eq!(app.st.selected.as_deref(), Some(b.as_str()));
    shot.send(&mut app, key("KeyO", MOD, Some("o")));
    assert_eq!(app.st.focused.as_deref(), Some(b.as_str()));
    shot.until(&mut app, "the stage's screen", |app, _| {
        app.pane(&b).map(|_| ())
    });
    shot.save(&mut app, "focused");
    shot.send(&mut app, key("BracketLeft", MOD, Some("[")));
    assert_eq!(
        app.st.focused.as_deref(),
        Some(a.as_str()),
        "the stage follows"
    );
    shot.send(&mut app, key("KeyW", MOD, Some("w")));
    assert_eq!(app.st.focused, None);

    // Mod+J opens the view options; the layout is saved to the config.
    shot.send(&mut app, key("KeyJ", MOD, Some("j")));
    assert_eq!(menu(&app), Some(MenuKind::ViewOptions));
    shot.frame(&mut app);
    shot.save(&mut app, "view-options");
    shot.click(&mut app, control("layout", Act::Layout(LayoutMode::Tabs)));
    assert_eq!(app.st.layout, LayoutMode::Tabs);
    assert_eq!(
        menu(&app),
        Some(MenuKind::ViewOptions),
        "the menu stays open"
    );
    escape(&mut shot, &mut app);
    shot.until(&mut app, "the layout saved", |app, _| {
        let saved = app.rpc().call("config:load", Value::Null).ok()?;
        (saved["defaults"]["layoutMode"] == "tabs").then_some(())
    });
    shot.click(&mut app, control("tab", Act::Select(b.clone())));
    assert_eq!(app.st.selected.as_deref(), Some(b.as_str()));
    shot.save(&mut app, "tabs");

    // Alt+digit filters; nothing waits for input, so Idle shows nothing.
    shot.send(&mut app, key("Digit1", mods::ALT, None));
    assert_eq!(app.st.filter, Filter::ALL[0]);
    shot.send(&mut app, key("Digit5", mods::ALT, None));
    assert_eq!(app.st.filter, Filter::ALL[4]);
    shot.send(&mut app, key("Digit1", mods::ALT, None));

    // Mod+/ toggles the shortcuts panel; Escape closes it.
    shot.send(&mut app, key("Slash", MOD, Some("/")));
    assert_eq!(app.st.dialog, Some(Dialog::Shortcuts));
    shot.save(&mut app, "shortcuts");
    escape(&mut shot, &mut app);
    assert_eq!(app.st.dialog, None);

    // Mod+N opens the launcher over the cards, with its pickers.
    shot.send(&mut app, key("KeyN", MOD, Some("n")));
    assert!(app.st.composer_open);
    shot.frame(&mut app);
    shot.save(&mut app, "launcher");
    shot.click(
        &mut app,
        control("composer", Act::Open(MenuKind::LaunchAgent)),
    );
    assert_eq!(menu(&app), Some(MenuKind::LaunchAgent));
    shot.save(&mut app, "launcher-agents");
    escape(&mut shot, &mut app);
    escape(&mut shot, &mut app);
    assert!(!app.st.composer_open);
}

#[test]
fn a_restart_offers_the_sessions_back_with_recent_and_headless() {
    let Some(mut v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let project = add_project(&mut app, &v, "demo", json!({ "reopenSessions": false }));
    // A conversation the agent's own history lists.
    let claude = v.home.join(".claude");
    std::fs::create_dir_all(&claude).expect("dir");
    let line = json!({
        "sessionId": "c0ffee",
        "display": "Fix the flaky test",
        "project": project,
        "timestamp": vorn_app::state::now_ms(),
    });
    std::fs::write(claude.join("history.jsonl"), format!("{line}\n")).expect("history");

    let mut shot = Shot::new();
    shot.frame(&mut app);
    let id = new_shell(&mut shot, &mut app);
    drop(app);
    v.restart();

    // The banner offers the shell; Restore shows it ended, with why.
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.until(&mut app, "the banner", |app, _| {
        (!app.st.banner.is_empty()).then_some(())
    });
    assert_eq!(app.st.banner[0].id, id);
    shot.save(&mut app, "restored-banner");
    shot.click(&mut app, control("banner", Act::Restore));
    assert!(app.st.banner.is_empty());
    let s = app.store.session(&id).expect("the ended pane");
    assert!(s.ended());
    // vornd does not tell a clean stop from a crash, as the renderer reads it.
    assert_eq!(s.cause, Some("the server stopped unexpectedly"));
    shot.save(&mut app, "restored-ended");
    // A shell's strip offers a new shell in its place, under its id.
    shot.click(&mut app, control("ended", Act::Resume(id.clone())));
    shot.until(&mut app, "the session started again", |app, _| {
        app.store.session(&id).filter(|s| !s.ended()).map(|_| ())
    });

    // Recent sessions lists the agent's conversation in this project.
    shot.click(
        &mut app,
        control("sidebar-project", Act::Project(Some("demo".into()))),
    );
    shot.click(&mut app, control("toolbar", Act::Open(MenuKind::Recent)));
    shot.until(&mut app, "the recent list", |app, _| {
        app.st.recent.first().map(|r| r.session_id.clone())
    });
    assert_eq!(app.st.recent[0].display, "Fix the flaky test");
    shot.frame(&mut app);
    shot.save(&mut app, "recent");
    escape(&mut shot, &mut app);

    // A headless agent shows the pill, and its menu lists it.
    let params = json!({
        "agentType": "claude",
        "projectName": "demo",
        "projectPath": project,
        "initialPrompt": "sleep 30",
        "headless": true,
    });
    let h = app.rpc().call("headless:create", params).expect("headless");
    let hid = h["id"].as_str().expect("id").to_owned();
    let mut app2 = connect(&v);
    shot.until(&mut app2, "the pill", |app, _| {
        app.st.headless.iter().any(|x| x.id == hid).then_some(())
    });
    shot.save(&mut app2, "headless-pill");
    shot.click(&mut app2, control("toolbar", Act::Open(MenuKind::Headless)));
    assert_eq!(menu(&app2), Some(MenuKind::Headless));
    shot.save(&mut app2, "headless-menu");
    // A click unfolds the pill's log; the menu stays open.
    let row = control("headless", Act::ToggleHeadless(hid.clone()));
    shot.click(&mut app2, row);
    assert!(app2.st.headless.expanded(&hid));
    assert_eq!(menu(&app2), Some(MenuKind::Headless));
    shot.save(&mut app2, "headless-expanded");
    let r = shot.ui.rect_of(row).expect("its row");
    shot.ui.pointer_move(r.x + r.w / 2.0, r.y + r.h / 2.0);
    shot.frame(&mut app2);
    // It exited (its prompt is not a command), so it can only be hidden.
    shot.click(
        &mut app2,
        control("headless", Act::HideHeadless(hid.clone())),
    );
    assert!(app2.st.headless.iter().all(|x| x.id != hid));
    // With its last pill hidden, the badge and its popover go.
    shot.frame(&mut app2);
    assert_eq!(menu(&app2), None);

    // Dismiss declines what is offered; with reopen on, panes come back.
    drop(app);
    drop(app2);
    let mut config = connect(&v)
        .rpc()
        .call("config:load", Value::Null)
        .expect("config");
    config["defaults"]["reopenSessions"] = json!(true);
    connect(&v)
        .rpc()
        .call("config:save", config)
        .expect("saved");
    v.restart();
    let mut app = connect(&v);
    shot.until(&mut app, "the session reopened", |app, _| {
        app.store.session(&id).filter(|s| !s.ended()).map(|_| ())
    });
    assert!(app.st.banner.is_empty());
    shot.until(&mut app, "its screen", |app, _| app.pane(&id).map(|_| ()));
    shot.save(&mut app, "reopened");
}

#[test]
fn cards_minimize_to_the_dock_expand_and_switch_views() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    shot.click(
        &mut app,
        control("sidebar-project", Act::Project(Some("demo".into()))),
    );
    let a = new_shell(&mut shot, &mut app);
    let b = new_shell(&mut shot, &mut app);
    let pill = control("dock", Act::ToggleMinimized(a.clone()));

    // Minimize leaves the grid for a pill in the top bar.
    shot.hover(&mut app, term_id(&a));
    shot.click(&mut app, control("card", Act::ToggleMinimized(a.clone())));
    assert!(app.st.minimized.contains(&a));
    assert!(shot.ui.rect_of(term_id(&a)).is_none());
    assert!(shot.ui.rect_of(pill).is_some());
    shot.save(&mut app, "minimized");

    // The dock folds to its count and back.
    let fold = control("dock", Act::ToggleDock);
    shot.click(&mut app, fold);
    assert!(app.st.dock_collapsed);
    assert!(shot.ui.rect_of(pill).is_none());
    shot.save(&mut app, "dock-collapsed");
    // Its count chip opens the popover that expands it again.
    shot.click(&mut app, control("dock", Act::DockPopover));
    shot.save(&mut app, "dock-popover");
    shot.click(&mut app, fold);
    assert!(!app.st.dock_collapsed);

    // The pill restores its card.
    shot.click(&mut app, pill);
    assert!(app.st.minimized.is_empty());
    assert_eq!(app.st.selected.as_deref(), Some(a.as_str()));
    assert!(shot.ui.rect_of(term_id(&a)).is_some());

    // Expand puts a card on the focused stage; its toggle collapses it.
    shot.hover(&mut app, term_id(&b));
    shot.click(&mut app, control("card", Act::Expand(b.clone())));
    assert_eq!(app.st.focused.as_deref(), Some(b.as_str()));
    shot.save(&mut app, "expanded");
    shot.hover(&mut app, term_id(&b));
    shot.click(&mut app, control("card", Act::Collapse));
    assert_eq!(app.st.focused, None);

    // With the sidebar closed, the top bar's view pills switch the view.
    shot.click(&mut app, control("sidebar", Act::ToggleSidebar));
    assert!(!app.st.sidebar);
    for (i, view) in VIEW_ORDER.iter().enumerate().rev() {
        shot.click(&mut app, item_id(VIEWS, i));
        assert_eq!(app.st.view, *view);
    }
    assert_eq!(app.st.view, MainView::Sessions);
}
