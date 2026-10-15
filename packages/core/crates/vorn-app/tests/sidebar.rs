//! The sidebar's view options, the workspace switcher, the new-session
//! menu and each agent's icon, driven against a test vornd and drawn as
//! `VORN_APP_SHOT=<prefix>` frames to compare with today's screens.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{connect, key, seed_acme, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::WS_NAME;
use vorn_app::screen::menus::{MENU, SUB};
use vorn_app::sidebar_view::{ProjectSort, WorktreeFilter};
use vorn_app::state::{MenuKind, SidebarMode};
use vorn_app::ui::{self, item_id, Id};
use vorn_app::App;

fn control(place: &str, act: Act) -> Id {
    App::control(place, &act)
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

fn menu(app: &App) -> Option<MenuKind> {
    app.st.menu.as_ref().map(|m| m.kind.clone())
}

fn escape(shot: &mut Shot, app: &mut App) {
    shot.send(app, key("Escape", 0, None));
    shot.frame(app);
}

fn park(shot: &mut Shot, app: &mut App) {
    shot.ui.pointer_move(130.0, 600.0);
    shot.frame(app);
}

/// A session of `agent` (a plain shell underneath) in `project`.
fn session(shot: &mut Shot, app: &mut App, agent: &str, project: &str, path: &str) -> String {
    let made = if agent == "shell" {
        app.rpc().call("shell:create", json!(path))
    } else {
        let params = json!({ "agentType": agent, "projectName": project, "projectPath": path });
        app.rpc().call("terminal:create", params)
    }
    .expect("created");
    let id = made["id"].as_str().expect("an id").to_owned();
    shot.until(app, "the session", |app, _| {
        app.store.session(&id).map(|_| ())
    });
    id
}

fn active_workspace(app: &App) -> String {
    let c = app.rpc().call("config:load", Value::Null).expect("config");
    c["defaults"]["activeWorkspace"]
        .as_str()
        .unwrap_or("personal")
        .to_owned()
}

#[test]
fn the_sidebar_options_persist_and_rearrange_it() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    let repo = seed_acme(&mut app, &v);
    let path = repo.to_string_lossy().into_owned();
    shot.until(&mut app, "the worktrees", |app, _| {
        app.st
            .worktrees
            .get(&path)
            .filter(|w| w.len() > 1)
            .map(|_| ())
    });
    let agent = session(&mut shot, &mut app, "acme", "acme", &path);
    session(&mut shot, &mut app, "shell", "acme", &path);
    park(&mut shot, &mut app);
    shot.save(&mut app, "sidebar-worktrees-sessions");
    assert!(app.store.session(&agent).is_some());

    let options = control("sidebar", Act::Open(MenuKind::SidebarOptions));
    shot.click(&mut app, options);
    assert_eq!(menu(&app), Some(MenuKind::SidebarOptions));
    let labels = app.menu_labels(&shot.ui);
    for want in [
        "Projects > Worktrees",
        "Projects > Worktrees > Sessions",
        "Projects > Sessions",
        "Groups > Sessions",
        "Sessions (Flat)",
        "With active sessions",
        "Recent activity",
    ] {
        assert!(labels.iter().any(|l| l == want), "{want:?} in {labels:?}");
    }
    shot.save(&mut app, "sidebar-options");

    choose(&mut shot, &mut app, "With active sessions");
    assert_eq!(app.st.sidebar_view.worktree_filter, WorktreeFilter::Active);
    if menu(&app).is_none() {
        shot.click(&mut app, options);
    }
    choose(&mut shot, &mut app, "Name");
    assert_eq!(app.st.sidebar_view.project_sort, ProjectSort::Name);
    if menu(&app).is_none() {
        shot.click(&mut app, options);
    }
    choose(&mut shot, &mut app, "Sessions (Flat)");
    assert_eq!(app.st.sidebar_view.view_mode, SidebarMode::Flat);
    escape(&mut shot, &mut app);
    park(&mut shot, &mut app);
    shot.save(&mut app, "sidebar-flat");

    let saved = std::fs::read_to_string(v.home.join("desktop/sidebar.json")).expect("saved");
    let saved: Value = serde_json::from_str(&saved).expect("json");
    assert_eq!(saved["viewMode"], "sessions-flat");
    assert_eq!(saved["worktreeFilter"], "active");
    assert_eq!(saved["projectSort"], "name");

    let again = connect(&v);
    assert_eq!(
        again.st.sidebar_view, app.st.sidebar_view,
        "reopened, it keeps them"
    );
}

#[test]
fn workspaces_switch_create_rename_and_delete() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    seed_acme(&mut app, &v);
    assert_eq!(app.store.workspace(), "personal");

    let open = control("workspace", Act::Open(MenuKind::Workspace));
    if menu(&app) != Some(MenuKind::Workspace) {
        shot.click(&mut app, open);
    }
    assert_eq!(menu(&app), Some(MenuKind::Workspace));
    shot.save(&mut app, "workspace-menu");

    shot.click(&mut app, control("workspace", Act::EditWorkspace(None)));
    assert!(shot.ui.focused(WS_NAME));
    shot.type_text(&mut app, "Work");
    shot.save(&mut app, "workspace-new");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    let work = shot.until(&mut app, "the new workspace", |app, _| {
        let ws = app.store.current_workspace();
        (ws.name == "Work").then(|| ws.id.clone())
    });
    shot.until(&mut app, "vornd to hold the switch", |app, _| {
        (active_workspace(app) == work).then_some(())
    });
    assert!(
        app.store.workspace_projects().next().is_none(),
        "a new one starts empty"
    );
    park(&mut shot, &mut app);
    shot.save(&mut app, "workspace-work");

    if menu(&app) != Some(MenuKind::Workspace) {
        shot.click(&mut app, open);
    }
    let row = control("workspace", Act::SwitchWorkspace(work.clone()));
    shot.hover(&mut app, row);
    shot.save(&mut app, "workspace-row-hover");
    shot.click(
        &mut app,
        control("workspace", Act::EditWorkspace(Some(work.clone()))),
    );
    shot.type_text(&mut app, " team");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    shot.until(&mut app, "the rename", |app, _| {
        (app.store.current_workspace().name == "Work team").then_some(())
    });

    // Another client switching moves this one with it.
    let mut c = app.rpc().call("config:load", Value::Null).expect("config");
    c["defaults"]["activeWorkspace"] = json!("personal");
    app.rpc().call("config:save", c).expect("saved");
    shot.until(&mut app, "the outside switch", |app, _| {
        (app.store.workspace() == "personal").then_some(())
    });
    assert!(app.store.workspace_projects().any(|p| p.name == "acme"));

    if menu(&app) != Some(MenuKind::Workspace) {
        shot.click(&mut app, open);
    }
    shot.click(
        &mut app,
        control("workspace", Act::SwitchWorkspace(work.clone())),
    );
    shot.until(&mut app, "the switch", |app, _| {
        (app.store.workspace() == work).then_some(())
    });
    shot.until(&mut app, "vornd to follow", |app, _| {
        (active_workspace(app) == work).then_some(())
    });

    if menu(&app) != Some(MenuKind::Workspace) {
        shot.click(&mut app, open);
    }
    shot.hover(&mut app, row);
    shot.click(
        &mut app,
        control("workspace", Act::AskDeleteWorkspace(work.clone())),
    );
    shot.save(&mut app, "workspace-delete");
    shot.click(
        &mut app,
        control("workspace", Act::DeleteWorkspace(work.clone())),
    );
    shot.until(&mut app, "the delete", |app, _| {
        app.store
            .workspaces()
            .iter()
            .all(|w| w.id != work)
            .then_some(())
    });
    assert_eq!(app.store.workspace(), "personal");
}

#[test]
fn the_new_session_menu_lists_every_project_and_worktree() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    let repo = seed_acme(&mut app, &v);
    let path = repo.to_string_lossy().into_owned();
    session(&mut shot, &mut app, "acme", "acme", &path);
    shot.until(&mut app, "the worktrees", |app, _| {
        app.st
            .worktrees
            .get(&path)
            .filter(|w| w.len() > 1)
            .map(|_| ())
    });

    shot.click(&mut app, control("toolbar", Act::Open(MenuKind::Grid)));
    assert_eq!(menu(&app), Some(MenuKind::Grid));
    let labels = app.menu_labels(&shot.ui);
    for want in ["New session...", "New session in\u{2026}"] {
        assert!(labels.iter().any(|l| l == want), "{want:?} in {labels:?}");
    }
    shot.save(&mut app, "plus-menu");
    let i = labels
        .iter()
        .position(|l| l == "New session in\u{2026}")
        .expect("the submenu");
    shot.hover(&mut app, item_id(MENU, i));
    assert_eq!(open_sub(&app), Some(i), "hovering opens the submenu");
    shot.save(&mut app, "plus-menu-projects");

    // Beside its entry, level with it, covering none of the menu.
    let row = shot.ui.rect_of(item_id(MENU, i)).expect("the entry");
    let sub = shot
        .ui
        .rect_of(ui::menu::sub_panel_id(SUB))
        .expect("the submenu");
    let slack = ui::menu::OVERLAP + 1.5;
    assert!(
        sub.x >= row.right() - slack || sub.right() <= row.x + slack,
        "{sub:?} beside {row:?}"
    );
    assert!(
        sub.y <= row.y && sub.y >= row.y - 8.0,
        "{sub:?} level with {row:?}"
    );

    // Heading for it across the next entry keeps it open.
    if let Some(next) = shot.ui.rect_of(item_id(MENU, i + 1)) {
        let toward = if sub.x > row.x { 1.0 } else { -1.0 };
        let (x0, y0) = (row.x + row.w / 2.0, row.y + row.h / 2.0);
        let (x1, y1) = (x0 + toward * 40.0, next.y + next.h / 2.0);
        for k in 1..=4 {
            let t = k as f32 / 4.0;
            shot.ui.pointer_move(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            shot.frame(&mut app);
        }
        assert_eq!(open_sub(&app), Some(i), "held while heading for it");
        let near = if toward > 0.0 {
            sub.x + 20.0
        } else {
            sub.right() - 20.0
        };
        let (x2, y2) = (near, (y1 + 10.0).min(sub.bottom() - 10.0));
        for k in 1..=3 {
            let t = k as f32 / 3.0;
            shot.ui.pointer_move(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
            shot.frame(&mut app);
        }
        assert_eq!(open_sub(&app), Some(i), "and reached");
        shot.save(&mut app, "plus-menu-heading");
        assert_eq!(open_sub(&app), Some(i), "resting in it keeps it");
    }

    // A click on the entry toggles its submenu; the menu stays.
    shot.click(&mut app, item_id(MENU, i));
    assert_eq!(open_sub(&app), None);
    assert_eq!(menu(&app), Some(MenuKind::Grid));
    shot.click(&mut app, item_id(MENU, i));
    assert_eq!(open_sub(&app), Some(i));

    // Keys: Left closes one level, Right opens into it, Escape closes one level.
    shot.send(&mut app, key("Escape", 0, None));
    assert_eq!(open_sub(&app), None);
    assert_eq!(menu(&app), Some(MenuKind::Grid), "Escape closed one level");
    for _ in 0..labels.len() {
        if shot.ui.focused(item_id(MENU, i)) {
            break;
        }
        shot.send(&mut app, key("ArrowDown", 0, None));
    }
    assert!(shot.ui.focused(item_id(MENU, i)), "Down reaches the entry");
    shot.send(&mut app, key("ArrowRight", 0, None));
    shot.frame(&mut app);
    shot.frame(&mut app);
    assert_eq!(open_sub(&app), Some(i));
    assert!(
        shot.ui.focused(item_id(SUB, 1)),
        "focus goes into the submenu, past its header"
    );
    shot.save(&mut app, "plus-menu-keys");
    shot.send(&mut app, key("ArrowLeft", 0, None));
    assert_eq!(open_sub(&app), None);
    assert!(shot.ui.focused(item_id(MENU, i)), "focus back on the entry");
    escape(&mut shot, &mut app);
    assert_eq!(menu(&app), None);
}

fn open_sub(app: &App) -> Option<usize> {
    app.st.menu.as_ref().and_then(|m| m.sub.open)
}

#[test]
fn the_sidebar_resizes_from_its_edge_within_its_bounds() {
    use vorn_app::screen::sidebar::SIDEBAR_HANDLE;
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    let handle = shot.until(&mut app, "the handle", |_, ui| ui.rect_of(SIDEBAR_HANDLE));
    // The sidebar starts 256 wide at the window's left edge.
    assert_eq!(handle.x + handle.w, 256.0);
    let y = handle.y + handle.h / 2.0;
    let drag = |shot: &mut Shot, app: &mut App, to: f32| {
        let from = shot.ui.rect_of(SIDEBAR_HANDLE).expect("its handle");
        shot.ui.pointer_move(from.x + 3.0, y);
        shot.ui.pointer_down();
        shot.ui.pointer_move(to, y);
        app.events(&mut shot.ui);
        shot.ui.pointer_up();
        app.events(&mut shot.ui);
        shot.frame(app);
        let after = shot.ui.rect_of(SIDEBAR_HANDLE).expect("its handle");
        after.x + after.w
    };
    assert_eq!(drag(&mut shot, &mut app, 320.0), 320.0);
    shot.save(&mut app, "sidebar-wide");
    assert_eq!(drag(&mut shot, &mut app, 900.0), 400.0);
    assert_eq!(drag(&mut shot, &mut app, 150.0), 180.0);
}
