//! Whole screens on the seed today's renderer is captured with (a git
//! project `demo`, three tasks, a group and a workflow), each state drawn
//! as `VORN_APP_SHOT=<prefix>` frames for a side-by-side comparison, and the
//! session groups driven through their menus against vornd.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};
use support::{connect, key, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::{GROUP_NAME, MOD, MODEL_ID};
use vorn_app::git::GitAct;
use vorn_app::screen::menus::MENU;
use vorn_app::screen::tasks::card_id;
use vorn_app::screen::MainView;
use vorn_app::state::{LayoutMode, MenuKind, SidebarMode};
use vorn_app::tasks::{Mode, TaskAct};
use vorn_app::ui::{item_id, mods, Id};
use vorn_app::workflows::{Anchor, WfAct};
use vorn_app::App;

const T0: &str = "2026-01-05T10:00:00.000Z";

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

/// The pointer where today's captures park it, over the empty sidebar.
fn park(shot: &mut Shot, app: &mut App) {
    shot.ui.pointer_move(130.0, 600.0);
    shot.frame(app);
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "git {args:?}");
}

fn task(id: &str, title: &str, status: &str) -> Value {
    json!({
        "id": id, "title": title, "status": status, "order": 0, "projectName": "demo",
        "description": "## Description\nThe login form drops the session on refresh.",
        "createdAt": T0, "updatedAt": T0
    })
}

/// The capture seed: `demo` a git repo on `main` with one commit, its tasks,
/// the Backend group and the Hello flow workflow.
fn seed(v: &TestVornd, shot: &mut Shot, app: &mut App) {
    let dir = v.home.join("demo");
    std::fs::create_dir_all(&dir).expect("project dir");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    git(&dir, &["config", "user.name", "Test"]);
    std::fs::write(dir.join("README.md"), "demo\n").expect("readme");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "first"]);
    let path = dir.to_string_lossy();
    let mut c = app.rpc().call("config:load", Value::Null).expect("config");
    c["projects"] = json!([{ "name": "demo", "path": path, "preferredAgents": [] }]);
    c["agentCommands"]["claude"] = json!({ "command": "/bin/sh", "args": [] });
    c["defaults"]["defaultAgent"] = json!("claude");
    c["tasks"] = json!([
        task("t1", "Fix login", "todo"),
        task("t2", "Add dark mode", "in_progress"),
        task("t3", "Write release notes", "done"),
    ]);
    c["sessionGroups"] =
        json!([{ "id": "g1", "name": "Backend", "order": 0, "workspaceId": "personal" }]);
    c["workflows"] = json!([{
        "id": "wf-hello", "name": "Hello flow", "icon": "Rocket", "iconColor": "#22c55e",
        "enabled": true, "workspaceId": "personal",
        "nodes": [
            { "id": "t", "type": "trigger", "label": "Manual Trigger", "position": { "x": 0, "y": 0 },
              "config": { "triggerType": "manual" } },
            { "id": "s", "type": "script", "label": "Execute Script", "position": { "x": 0, "y": 120 },
              "config": { "scriptType": "bash", "scriptContent": "echo hello",
                          "projectName": "demo", "projectPath": path } }
        ],
        "edges": [{ "id": "e", "source": "t", "target": "s" }]
    }]);
    app.rpc().call("config:save", c).expect("saved");
    shot.until(app, "the seed loaded", |app, _| {
        (!app.store.projects.is_empty()
            && !app.store.groups.is_empty()
            && app.store.tasks.len() == 3)
            .then_some(())
    });
    shot.until(app, "the worktrees listed", |app, _| {
        app.st
            .worktrees
            .get(path.as_ref())
            .filter(|w| !w.is_empty())
            .map(|_| ())
    });
}

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
fn the_sessions_screens() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    seed(&v, &mut shot, &mut app);
    assert_eq!(app.st.sidebar_mode, SidebarMode::WorktreesSessions);
    park(&mut shot, &mut app);
    shot.save(&mut app, "sidebar");

    shot.click(
        &mut app,
        control("sidebar-project", Act::Project(Some("demo".into()))),
    );
    let ids: Vec<String> = (0..3).map(|_| new_shell(&mut shot, &mut app)).collect();
    shot.send(&mut app, key("Digit1", MOD, Some("1")));
    assert_eq!(app.st.selected.as_deref(), Some(ids[0].as_str()));
    // The project keeps the fold it took while it had no sessions.
    assert!(app.st.collapsed.contains("demo"));
    shot.until(&mut app, "the branch chips", |app, _| {
        app.store
            .sessions
            .iter()
            .all(|s| s.info.branch.as_deref() == Some("main"))
            .then_some(())
    });
    park(&mut shot, &mut app);
    std::thread::sleep(Duration::from_millis(1500));
    shot.save(&mut app, "grid");

    // The status bar's Open lists the editors found here, when there are any.
    if app.st.git.ides.as_ref().is_some_and(|l| !l.is_empty()) {
        let open_in = Act::Git(GitAct::OpenInMenu(ids[0].clone()));
        shot.click(&mut app, control("open-in-more", open_in));
        assert_eq!(menu(&app), Some(MenuKind::OpenIn(ids[0].clone())));
        shot.save(&mut app, "open-in");
        escape(&mut shot, &mut app);
    }

    shot.send(&mut app, key("KeyB", MOD, Some("b")));
    assert!(!app.st.sidebar);
    park(&mut shot, &mut app);
    shot.save(&mut app, "sidebar-closed");
    shot.send(&mut app, key("KeyB", MOD, Some("b")));
    assert!(!app.st.collapsed.contains("demo"), "reopened, it unfolds");

    shot.send(&mut app, key("KeyO", MOD, Some("o")));
    assert_eq!(app.st.focused.as_deref(), Some(ids[0].as_str()));
    park(&mut shot, &mut app);
    shot.save(&mut app, "focused");
    shot.send(&mut app, key("KeyW", MOD, Some("w")));
    assert_eq!(app.st.focused, None);

    shot.send(&mut app, key("KeyJ", MOD, Some("j")));
    assert_eq!(menu(&app), Some(MenuKind::ViewOptions));
    shot.save(&mut app, "view-options");
    shot.click(&mut app, control("layout", Act::Layout(LayoutMode::Tabs)));
    escape(&mut shot, &mut app);
    park(&mut shot, &mut app);
    shot.save(&mut app, "tabs");
    shot.send(&mut app, key("KeyJ", MOD, Some("j")));
    shot.click(&mut app, control("layout", Act::Layout(LayoutMode::Grid)));
    escape(&mut shot, &mut app);

    shot.send(&mut app, key("KeyN", MOD, Some("n")));
    assert!(app.st.composer_open);
    park(&mut shot, &mut app);
    shot.save(&mut app, "launcher");
    for (kind, name) in [
        (MenuKind::LaunchProject, "launcher-project"),
        (MenuKind::LaunchAgent, "launcher-agent"),
        (MenuKind::LaunchWorktree, "launcher-worktree"),
        (MenuKind::LaunchBranch, "launcher-branch"),
    ] {
        shot.click(&mut app, control("composer", Act::Open(kind.clone())));
        assert_eq!(menu(&app), Some(kind));
        std::thread::sleep(Duration::from_millis(300));
        park(&mut shot, &mut app);
        shot.save(&mut app, name);
        escape(&mut shot, &mut app);
        assert!(app.st.composer_open, "Escape closes the picker first");
    }

    // The model picker: the project's listing is asked for, a bad id is
    // refused, a typed id is taken.
    let path = app.store.projects[0].path.clone();
    let models = ("claude".to_owned(), path);
    shot.until(&mut app, "the models answered", |app, _| {
        app.st
            .models
            .lists
            .get(&models)
            .filter(|l| !l.loading && l.catalog.is_some())
            .map(|_| ())
    });
    let open_model = control("composer", Act::Open(MenuKind::LaunchModel));
    shot.click(&mut app, open_model);
    assert_eq!(menu(&app), Some(MenuKind::LaunchModel));
    assert!(shot.ui.focused(MODEL_ID), "the id field takes the keys");
    shot.type_line(&mut app, "-bad");
    assert_eq!(app.st.models.invalid, Some(vorn_app::models::INVALID_ID));
    assert_eq!(menu(&app), Some(MenuKind::LaunchModel));
    shot.send(&mut app, key("KeyA", MOD, None));
    shot.type_line(&mut app, "my-model");
    assert_eq!(menu(&app), None);
    assert_eq!(app.st.models.picked.get("claude").map(String::as_str), Some("my-model"));
    app.st.models.lists.insert(
        models,
        vorn_app::models::Listing {
            catalog: Some(vorn_app::models::Catalog::from_json(&json!({
                "status": "ready", "fetchedAt": vorn_app::state::now_ms(),
                "choices": [
                    { "id": "sonnet", "label": "Sonnet 4.5" },
                    { "id": "opus", "label": "Opus 4.1" },
                    { "id": "haiku", "label": "Haiku 4.5" }
                ]
            }))),
            loading: false,
        },
    );
    shot.click(&mut app, open_model);
    std::thread::sleep(Duration::from_millis(300));
    park(&mut shot, &mut app);
    shot.save(&mut app, "launcher-model");
    shot.click(&mut app, control("model-pick", Act::PickModel(None)));
    assert_eq!(menu(&app), None);
    assert!(!app.st.models.picked.contains_key("claude"));
}

#[test]
fn session_groups_work_through_their_menus() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    seed(&v, &mut shot, &mut app);
    let shell = new_shell(&mut shot, &mut app);

    shot.click(
        &mut app,
        control("sidebar", Act::Open(MenuKind::SidebarOptions)),
    );
    choose(&mut shot, &mut app, "Groups > Sessions");
    assert_eq!(app.st.sidebar_mode, SidebarMode::Groups);
    escape(&mut shot, &mut app);
    park(&mut shot, &mut app);
    shot.save(&mut app, "groups");

    // A session's More moves it into a group, through vornd.
    let more = control(
        "sidebar-session",
        Act::Open(MenuKind::SessionGroups(shell.clone())),
    );
    shot.hover(
        &mut app,
        control("sidebar-session", Act::Expand(shell.clone())),
    );
    shot.click(&mut app, more);
    assert_eq!(
        app.menu_labels(&shot.ui),
        ["Move to group", "Backend", "New group\u{2026}"]
    );
    shot.save(&mut app, "session-groups-menu");
    choose(&mut shot, &mut app, "Backend");
    shot.until(&mut app, "the session in Backend", |app, _| {
        let list = app.rpc().call("terminal:listActive", Value::Null).ok()?;
        list.as_array()?
            .iter()
            .any(|s| s["id"] == shell.as_str() && s["groupId"] == "g1")
            .then_some(())
    });

    // The group row's hover, its More tooltip and menu.
    let row = control("sidebar-group", Act::Group(Some("g1".into())));
    shot.hover(&mut app, row);
    shot.save(&mut app, "group-hover");
    let more = control("sidebar-group", Act::Open(MenuKind::Group("g1".into())));
    shot.hover(&mut app, more);
    std::thread::sleep(Duration::from_millis(400) + Duration::from_millis(50));
    shot.frame(&mut app);
    assert!(shot.ui.tooltip_shown(more));
    shot.save(&mut app, "group-tooltip");
    shot.click(&mut app, more);
    assert_eq!(
        app.menu_labels(&shot.ui),
        [
            "Rename group",
            "Change icon",
            "Remove all sessions",
            "Delete group"
        ]
    );
    shot.save(&mut app, "group-menu");

    // Rename through the inline field.
    choose(&mut shot, &mut app, "Rename group");
    assert_eq!(shot.ui.edit_text(GROUP_NAME), "Backend");
    shot.send(&mut app, key("KeyA", MOD, None));
    shot.type_line(&mut app, "API");
    shot.until(&mut app, "the rename saved", |app, _| {
        let c = app.rpc().call("config:load", Value::Null).ok()?;
        (c["sessionGroups"][0]["name"] == "API").then_some(())
    });

    // Change icon keeps the picker open across picks.
    shot.hover(&mut app, row);
    shot.click(&mut app, more);
    choose(&mut shot, &mut app, "Change icon");
    assert_eq!(menu(&app), Some(MenuKind::GroupIcon("g1".into())));
    shot.save(&mut app, "group-icon");
    shot.click(
        &mut app,
        App::control(
            "group-icon",
            &Act::GroupIconName("g1".into(), "Server".into()),
        ),
    );
    shot.until(&mut app, "the icon saved", |app, _| {
        let c = app.rpc().call("config:load", Value::Null).ok()?;
        (c["sessionGroups"][0]["icon"] == "Server").then_some(())
    });
    assert_eq!(
        menu(&app),
        Some(MenuKind::GroupIcon("g1".into())),
        "stays open"
    );
    escape(&mut shot, &mut app);

    // Delete asks in place, then the group goes with a toast.
    shot.hover(&mut app, row);
    shot.click(&mut app, more);
    choose(&mut shot, &mut app, "Delete group");
    assert!(app
        .menu_labels(&shot.ui)
        .contains(&"Confirm delete?".to_owned()));
    shot.save(&mut app, "group-menu-confirm");
    choose(&mut shot, &mut app, "Confirm delete?");
    shot.until(&mut app, "the group deleted", |app, _| {
        app.store.groups.is_empty().then_some(())
    });
    shot.until(&mut app, "the toast", |app, _| {
        app.st
            .toasts
            .iter()
            .any(|t| t.text == "Group \"API\" deleted")
            .then_some(())
    });

    // New group from a session's menu adds one with it inside.
    shot.hover(
        &mut app,
        control("sidebar-session", Act::Expand(shell.clone())),
    );
    shot.click(
        &mut app,
        control(
            "sidebar-session",
            Act::Open(MenuKind::SessionGroups(shell.clone())),
        ),
    );
    choose(&mut shot, &mut app, "New group\u{2026}");
    shot.until(&mut app, "the new group", |app, _| {
        let g = app.store.groups.first()?;
        app.store
            .session(&shell)
            .filter(|s| s.info.group_id.as_deref() == Some(g.id.as_str()))
            .map(|_| ())
    });
}

#[test]
fn the_tasks_screens() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    seed(&v, &mut shot, &mut app);
    shot.send(&mut app, key("KeyT", MOD, None));
    assert_eq!(app.st.view, MainView::Tasks);
    park(&mut shot, &mut app);
    shot.save(&mut app, "tasks-list");
    shot.click(&mut app, card_id("t1"));
    assert_eq!(app.st.tasks.selected.as_deref(), Some("t1"));
    park(&mut shot, &mut app);
    shot.save(&mut app, "task-detail");
    shot.click(
        &mut app,
        control("task-panel", Act::Task(TaskAct::CloseDetail)),
    );
    shot.click(
        &mut app,
        control("toolbar", Act::Open(MenuKind::TaskOptions)),
    );
    shot.click(
        &mut app,
        control("task-mode", Act::Task(TaskAct::Mode(Mode::Board))),
    );
    escape(&mut shot, &mut app);
    park(&mut shot, &mut app);
    shot.save(&mut app, "tasks-board");
}

#[test]
fn the_workflows_screens() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    seed(&v, &mut shot, &mut app);
    shot.send(&mut app, key("KeyW", MOD | mods::SHIFT, None));
    assert_eq!(app.st.view, MainView::Workflows);
    park(&mut shot, &mut app);
    shot.save(&mut app, "w-landing-empty");

    shot.hover(
        &mut app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit("wf-hello".into()))),
    );
    shot.click(&mut app, Id::new("wf-side-run").child("wf-hello"));
    let run = shot.until(&mut app, "the run done", |app, _| {
        let r = app
            .rpc()
            .call(
                "workflowRun:list",
                json!({ "workflowId": "wf-hello", "limit": 5 }),
            )
            .ok()?;
        let r = r.as_array()?.first()?.clone();
        (r["status"] == "success").then_some(r)
    });
    shot.click(
        &mut app,
        App::control("sidebar", &Act::Workflow(WfAct::AllRuns)),
    );
    shot.until(&mut app, "the run listed", |app, _| {
        (!app.st.wf.runs.is_empty()).then_some(())
    });
    park(&mut shot, &mut app);
    shot.save(&mut app, "w-landing");
    let id = run["runId"].as_str().expect("run id").to_owned();
    shot.click(
        &mut app,
        App::control("wf-run", &Act::Workflow(WfAct::SelectRun(id))),
    );
    park(&mut shot, &mut app);
    shot.save(&mut app, "w-run-detail");

    shot.click(
        &mut app,
        App::control("wf-side", &Act::Workflow(WfAct::Edit("wf-hello".into()))),
    );
    park(&mut shot, &mut app);
    shot.save(&mut app, "w-editor");
    shot.click(
        &mut app,
        App::control("wf-card", &Act::Workflow(WfAct::SelectNode("s".into()))),
    );
    park(&mut shot, &mut app);
    shot.save(&mut app, "w-editor-node");
    let plus = Anchor {
        after: Some("s".into()),
        ..Anchor::default()
    };
    shot.click(
        &mut app,
        App::control("wf-plus", &Act::Workflow(WfAct::Library(plus))),
    );
    // The published catalog arrives as `connector:catalogChanged` when the
    // network allows; the seed has no actions, so the shot waits a while.
    let t = std::time::Instant::now();
    while t.elapsed() < std::time::Duration::from_secs(10)
        && !app
            .st
            .wf
            .lib
            .catalog
            .iter()
            .any(|c| c.get("actions").is_some())
    {
        shot.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    park(&mut shot, &mut app);
    shot.save(&mut app, "w-editor-library");
}
