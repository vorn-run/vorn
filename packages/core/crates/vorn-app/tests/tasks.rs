//! The Tasks view driven as a user would, on a vornd with its own HOME:
//! the list and the board, the add dialog and its editor, the detail panel
//! with its pickers, images, workflow runs and diff review, the commit
//! dialog, the card menu and a card dragged across the board, and a board
//! of thousands that builds only the cards in view on a window and cells.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
use support::{add_project, connect, key, seed_runs, seed_tasks, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::runs::RunAct;
use vorn_app::screen::menus::MENU;
use vorn_app::screen::tasks::{
    card_id, column_id, column_scroll_id, comment_id, commit_id, desc_id, image_id, title_id,
};
use vorn_app::screen::MainView;
use vorn_app::state::{Dialog, MenuKind};
use vorn_app::tasks::{Form, Mode, TaskAct};
use vorn_app::ui::{item_id, mods, Id, RenderMode, Ui};
use vorn_app::{ui_config, App};

fn control(place: &str, act: Act) -> Id {
    App::control(place, &act)
}

fn task(act: TaskAct) -> Act {
    Act::Task(act)
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

#[track_caller]
fn toast(shot: &mut Shot, app: &mut App, text: &str) {
    shot.until(app, text, |app, _| {
        app.st.toasts.iter().any(|t| t.text == text).then_some(())
    });
}

/// The saved task `id`, as vornd has it.
fn saved(app: &App, id: &str) -> Option<Value> {
    let task = app.rpc().call("task:get", json!({ "id": id })).ok()?;
    task.is_object().then_some(task)
}

/// The Tasks view of a vornd with project `web`.
fn tasks_view(v: &TestVornd) -> (App, Shot) {
    let mut app = connect(v);
    add_project(&mut app, v, "web", json!({}));
    let mut shot = Shot::new();
    shot.send(&mut app, key("KeyT", MOD(), None));
    assert_eq!(app.st.view, MainView::Tasks);
    (app, shot)
}

/// A task made through the add dialog, by its id.
fn create(shot: &mut Shot, app: &mut App, title: &str) -> String {
    shot.click(app, control("toolbar", task(TaskAct::New("todo".into()))));
    assert_eq!(app.st.dialog, Some(Dialog::AddTask));
    let id = app.st.tasks.dialog_form.id.clone();
    shot.type_line(app, title);
    shot.until(app, "the task saved", |app, _| {
        app.store.tasks.iter().any(|t| t.id == id).then_some(())
    });
    id
}

#[test]
fn a_task_is_created_edited_moved_and_deleted() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = tasks_view(&v);
    shot.save(&mut app, "tasks-empty");

    shot.click(
        &mut app,
        control("toolbar", task(TaskAct::New("todo".into()))),
    );
    assert_eq!(app.st.dialog, Some(Dialog::AddTask));
    assert_eq!(app.st.tasks.dialog_form.project, "web");
    assert!(shot
        .ui
        .edit_text(desc_id(Form::Dialog))
        .starts_with("## Description"));
    shot.type_text(&mut app, "Fix login");
    shot.save(&mut app, "task-dialog");
    let id = app.st.tasks.dialog_form.id.clone();
    shot.click(
        &mut app,
        control("task-dialog-submit", task(TaskAct::Submit)),
    );
    assert_eq!(app.st.dialog, None);
    toast(&mut shot, &mut app, "Task created");
    let t = shot.until(&mut app, "the task saved", |app, _| saved(app, &id));
    assert_eq!(t["title"], "Fix login");
    assert!(t["description"]
        .as_str()
        .is_some_and(|d| d.starts_with("## Description\n")));
    assert_eq!(t["status"], "todo");
    assert_eq!(t["projectName"], "web");
    shot.until(&mut app, "its row", |app, _| {
        app.store.tasks.iter().any(|t| t.id == id).then_some(())
    });
    shot.save(&mut app, "tasks-list");

    // The row opens the panel; its title autosaves.
    shot.click(&mut app, card_id(&id));
    assert_eq!(app.st.tasks.selected.as_deref(), Some(id.as_str()));
    assert_eq!(shot.ui.edit_text(title_id(Form::Detail)), "Fix login");
    shot.click(&mut app, title_id(Form::Detail));
    shot.send(&mut app, key("End", 0, None));
    shot.type_text(&mut app, " flow");
    shot.until(&mut app, "the edit saved", |app, _| {
        (saved(app, &id)?["title"] == "Fix login flow").then_some(())
    });
    shot.save(&mut app, "task-detail");

    // The status picker moves it; the card menu completes and archives it.
    shot.click(
        &mut app,
        control(
            "task-status-picker",
            Act::Open(MenuKind::TaskStatus(Form::Detail)),
        ),
    );
    assert_eq!(
        app.menu_labels(&shot.ui),
        ["Todo", "In Progress", "In Review", "Done", "Cancelled"]
    );
    shot.save(&mut app, "task-status-menu");
    choose(&mut shot, &mut app, "In Review");
    toast(&mut shot, &mut app, "Task moved to review");
    shot.until(&mut app, "in review", |app, _| {
        (saved(app, &id)?["status"] == "in_review").then_some(())
    });

    shot.click(&mut app, control("task-panel", task(TaskAct::CloseDetail)));
    assert_eq!(app.st.tasks.selected, None);
    // A list row's hover buttons complete and archive it.
    shot.hover(&mut app, card_id(&id));
    shot.save(&mut app, "task-row-actions");
    shot.click(
        &mut app,
        control("task-row-done", task(TaskAct::Complete(id.clone()))),
    );
    toast(&mut shot, &mut app, "Task completed");
    shot.until(&mut app, "done", |app, _| {
        (saved(app, &id)?["status"] == "done").then_some(())
    });
    shot.hover(&mut app, card_id(&id));
    shot.click(
        &mut app,
        control("task-row-archive", task(TaskAct::Archive(id.clone()))),
    );
    toast(&mut shot, &mut app, "Task archived");
    shot.until(&mut app, "archived", |app, _| {
        saved(app, &id)?["archivedAt"].as_str().map(|_| ())
    });
    shot.until(&mut app, "the row hidden", |app, ui| {
        ui.rect_of(card_id(&id))
            .is_none()
            .then_some(())
            .filter(|_| {
                app.store
                    .tasks
                    .iter()
                    .any(|t| t.id == id && t.archived_at.is_some())
            })
    });

    // Archived tasks show with the toolbar's switch; the panel deletes one.
    shot.click(
        &mut app,
        control("toolbar", Act::Open(MenuKind::TaskOptions)),
    );
    shot.save(&mut app, "task-options");
    shot.click(
        &mut app,
        control("task-archived", task(TaskAct::ToggleArchived)),
    );
    assert!(app.st.tasks.archived);
    shot.send(&mut app, key("Escape", 0, None));
    shot.click(&mut app, card_id(&id));
    shot.click(
        &mut app,
        control("task-panel", task(TaskAct::AskDelete(id.clone()))),
    );
    assert_eq!(app.st.tasks.confirm_delete.as_deref(), Some(id.as_str()));
    shot.save(&mut app, "task-delete-confirm");
    shot.click(
        &mut app,
        control("task-delete-confirm", task(TaskAct::Delete(id.clone()))),
    );
    toast(&mut shot, &mut app, "Task deleted");
    shot.until(&mut app, "the task gone", |app, _| {
        saved(app, &id).is_none().then_some(())
    });
    assert_eq!(app.st.tasks.selected, None);
}

#[test]
fn the_board_moves_a_dragged_card_to_the_column_it_is_dropped_on() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = tasks_view(&v);
    let id = create(&mut shot, &mut app, "Ship it");

    shot.click(
        &mut app,
        control("toolbar", Act::Open(MenuKind::TaskOptions)),
    );
    shot.click(
        &mut app,
        control("task-mode", task(TaskAct::Mode(Mode::Board))),
    );
    assert_eq!(app.st.tasks.mode, Mode::Board);
    shot.until(&mut app, "the mode saved", |app, _| {
        let c = app.rpc().call("config:load", Value::Null).ok()?;
        (c["defaults"]["taskViewMode"] == "kanban").then_some(())
    });
    shot.send(&mut app, key("Escape", 0, None));
    shot.save(&mut app, "tasks-board");

    shot.hover(&mut app, card_id(&id));
    let more = shot.button(&mut app, "More").expect("the card's menu");
    shot.click(&mut app, more);
    assert!(app.st.points.more.is_some());
    for label in ["Edit", "Cancel task", "Delete"] {
        assert!(
            shot.labelled(&mut app, "MenuItem", label).is_some(),
            "{label} in the menu"
        );
    }
    for label in ["Archive", "Reopen task", "Review diff"] {
        assert!(
            shot.labelled(&mut app, "MenuItem", label).is_none(),
            "{label} not in the menu"
        );
    }
    shot.save(&mut app, "task-card-menu");
    shot.send(&mut app, key("Escape", 0, None));
    assert!(app.st.points.more.is_none());

    shot.frame(&mut app);
    let from = shot.ui.rect_of(card_id(&id)).expect("the card");
    let to = shot.ui.rect_of(column_id("done")).expect("the Done column");
    let (x0, y0) = (from.x + from.w / 2.0, from.y + from.h / 2.0);
    let (x1, y1) = (to.x + to.w / 2.0, to.y + 80.0);
    shot.ui.pointer_move(x0, y0);
    shot.ui.pointer_down();
    app.events(&mut shot.ui);
    for i in 1..=8 {
        let f = i as f32 / 8.0;
        shot.ui.pointer_move(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
        app.events(&mut shot.ui);
        shot.frame(&mut app);
    }
    assert_eq!(
        app.st.tasks.dragging,
        Some((id.clone(), Some("done".to_owned())))
    );
    shot.save(&mut app, "task-drag");
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    assert_eq!(app.st.tasks.dragging, None);
    toast(&mut shot, &mut app, "Task completed");
    shot.until(&mut app, "done", |app, _| {
        (saved(app, &id)?["status"] == "done").then_some(())
    });
}

#[test]
fn the_editor_runs_slash_commands_carries_lists_and_formats_a_selection() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = tasks_view(&v);
    shot.click(
        &mut app,
        control("toolbar", task(TaskAct::New("todo".into()))),
    );
    let desc = desc_id(Form::Dialog);
    // At rest the description is drawn; a click edits its markdown.
    shot.save(&mut app, "task-dialog-rendered");
    shot.click(
        &mut app,
        control("task-check.4", task(TaskAct::CheckItem(Form::Dialog, 4))),
    );
    assert!(shot
        .ui
        .edit_text(desc)
        .contains("- [x] Criterion 1\n- [ ] Criterion 2"));
    shot.save(&mut app, "task-dialog-checked");
    shot.click(
        &mut app,
        control("task-desc-preview", task(TaskAct::EditDesc(Form::Dialog))),
    );
    assert!(shot.ui.focused(desc));
    shot.ui.set_edit_text(desc, "");
    shot.type_text(&mut app, "/task");
    shot.save(&mut app, "task-slash");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    assert_eq!(shot.ui.edit_text(desc), "- [ ] ");
    shot.type_text(&mut app, "first");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    assert_eq!(shot.ui.edit_text(desc), "- [ ] first\n- [ ] ");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    assert_eq!(
        shot.ui.edit_text(desc),
        "- [ ] first\n\n",
        "an empty item ends it"
    );

    shot.ui.edit_mut(desc).set_selection(6, 11);
    shot.save(&mut app, "task-toolbar");
    shot.send(&mut app, key("KeyB", MOD(), None));
    assert_eq!(shot.ui.edit_text(desc), "- [ ] **first**\n\n");
    assert!(
        app.st.sidebar,
        "Mod+B in the editor is bold, not the sidebar"
    );

    shot.ui.edit_mut(desc).set_selection(8, 13);
    shot.click(
        &mut app,
        control("task-format", task(TaskAct::StartLink(Form::Dialog))),
    );
    assert!(app.st.tasks.linking.is_some());
    shot.type_text(&mut app, "example.com");
    shot.save(&mut app, "task-link");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    assert_eq!(
        shot.ui.edit_text(desc),
        "- [ ] **[first](https://example.com)**\n\n"
    );
}

/// A repository in `dir` with one committed file and an uncommitted change.
fn repo(dir: &Path) {
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("git runs")
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    std::fs::write(dir.join("app.txt"), "one\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);
    std::fs::write(dir.join("app.txt"), "one\ntwo\n").expect("change");
}

#[test]
fn a_review_shows_the_diff_takes_line_comments_and_commits() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let dir = add_project(&mut app, &v, "web", json!({}));
    repo(&dir);
    let mut shot = Shot::new();
    shot.send(&mut app, key("KeyT", MOD(), None));
    let id = create(&mut shot, &mut app, "Add two");
    shot.click(&mut app, card_id(&id));
    shot.click(
        &mut app,
        control(
            "task-status-picker",
            Act::Open(MenuKind::TaskStatus(Form::Detail)),
        ),
    );
    choose(&mut shot, &mut app, "In Review");

    let path = shot.until(&mut app, "the diff", |app, _| {
        Some(app.st.tasks.diff.files.first()?.path.clone())
    });
    assert_eq!(path, "app.txt");
    shot.save(&mut app, "task-diff");
    shot.click(
        &mut app,
        control(
            "task-diff-file",
            task(TaskAct::PickFile(Some(path.clone()))),
        ),
    );
    let diff = app.st.tasks.diff.files[0].diff.clone();
    let line = diff
        .split('\n')
        .position(|l| l == "+two")
        .expect("the added line");
    shot.click(
        &mut app,
        control("task-diff-line", task(TaskAct::Line(path.clone(), line))),
    );
    assert_eq!(shot.ui.focus(), Some(comment_id()));
    shot.type_text(&mut app, "Say three too");
    shot.send(&mut app, key("Enter", MOD(), Some("\r")));
    let c = &app.st.tasks.diff.comments;
    assert_eq!(c.len(), 1);
    assert_eq!(
        (c[0].content.as_str(), c[0].text.as_str()),
        ("+two", "Say three too")
    );
    shot.save(&mut app, "task-comments");
    shot.click(
        &mut app,
        control("task-comments-send", task(TaskAct::SendFeedback)),
    );
    toast(&mut shot, &mut app, "Review feedback copied to clipboard");
    assert!(app.st.tasks.diff.comments.is_empty());

    shot.click(
        &mut app,
        control("task-commit-open", task(TaskAct::OpenCommit)),
    );
    assert_eq!(app.st.dialog, Some(Dialog::Commit));
    assert_eq!(shot.ui.focus(), Some(commit_id()));
    shot.type_text(&mut app, "Add two");
    shot.save(&mut app, "task-commit");
    shot.send(&mut app, key("Enter", MOD(), Some("\r")));
    toast(&mut shot, &mut app, "Changes committed");
    assert_eq!(app.st.dialog, None);
    let log = Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(&dir)
        .output()
        .expect("git log");
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "Add two");
    shot.until(&mut app, "a clean diff", |app, _| {
        (!app.st.tasks.diff.loading && app.st.tasks.diff.files.is_empty()).then_some(())
    });
}

#[test]
fn the_panel_attaches_images_and_lists_the_tasks_workflow_runs() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let (mut app, mut shot) = tasks_view(&v);
    let id = create(&mut shot, &mut app, "With a picture");
    let run = json!({
        "runId": "run-1",
        "workflowId": "wf-1",
        "workflowName": "Nightly checks",
        "startedAt": "2030-01-01T00:00:00.000Z",
        "completedAt": "2030-01-01T00:00:05.000Z",
        "status": "completed",
        "triggerTaskId": id,
        "nodeStates": [{
            "nodeId": "n1",
            "status": "completed",
            "startedAt": "2030-01-01T00:00:00.000Z",
            "completedAt": "2030-01-01T00:00:05.000Z",
            "logs": "all checks passed"
        }]
    });
    seed_runs(&v, &[run]);
    shot.click(&mut app, card_id(&id));
    shot.until(&mut app, "the run", |app, _| {
        (app.st.tasks.runs.first()?["runId"] == "run-1").then_some(())
    });
    shot.click(
        &mut app,
        control("run-toggle", Act::Run(RunAct::Toggle("run-1".into()))),
    );
    shot.click(
        &mut app,
        control(
            "run-step",
            Act::Run(RunAct::ToggleStep("run-1".into(), "n1".into())),
        ),
    );
    shot.save(&mut app, "task-runs");
    shot.click(
        &mut app,
        control(
            "run-output",
            Act::Run(RunAct::Output("run-1".into(), "n1".into())),
        ),
    );
    assert_eq!(app.st.dialog, Some(Dialog::Output));
    assert_eq!(app.st.tasks.output, "all checks passed");
    shot.save(&mut app, "task-output");
    shot.send(&mut app, key("Escape", 0, None));
    assert_eq!(app.st.dialog, None);

    let png = v.home.join("shot.png");
    std::fs::write(&png, vorn_app::look::LOGO_PNG).expect("png");
    shot.click(
        &mut app,
        control("task-image-add", task(TaskAct::AttachImages(Form::Detail))),
    );
    assert_eq!(shot.ui.focus(), Some(image_id(Form::Detail)));
    shot.type_line(&mut app, &png.to_string_lossy());
    // The store keeps no image list (the renderer's saves drop it too), so
    // the file and the panel's form are what show it was attached.
    let name = shot.until(&mut app, "the image saved", |app, _| {
        app.st.tasks.detail_form.images.first().cloned()
    });
    let file = v.home.join("task-images").join(&id).join(&name);
    assert!(file.is_file(), "{} stored", file.display());
    shot.until(&mut app, "its thumbnail", |app, _| {
        app.st.tasks.thumbs.contains_key(&name).then_some(())
    });
    shot.save(&mut app, "task-images");
    shot.click(
        &mut app,
        control(
            "task-image-remove",
            task(TaskAct::RemoveImage(Form::Detail, name.clone())),
        ),
    );
    assert!(app.st.tasks.detail_form.images.is_empty());
    shot.until(&mut app, "the file removed", |_, _| {
        (!file.exists()).then_some(())
    });
}

/// A board of `LONG` tasks, alternately To do and Done, on a window or on
/// cells: only the cards in view are built, yet every card is reached by
/// scrolling, the arrows and a drag that scrolls the column it nears.
fn long_board(cells: bool) {
    const LONG: usize = 400;
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    let id = |i: usize| format!("t-{i:05}");
    let tasks: Vec<Value> = (0..LONG)
        .map(|i| {
            json!({
                "id": id(i), "projectName": "web", "title": format!("Task {i}"),
                "description": "", "status": if i % 2 == 0 { "todo" } else { "done" },
            })
        })
        .collect();
    seed_tasks(app.rpc(), &tasks);
    let (mut ui, size) = if cells {
        let ui = Ui::cells(&ui_config(1.0));
        (ui, vorn_app::ui::cells::units::px(180, 40))
    } else {
        (
            Ui::headless(RenderMode::Auto, &ui_config(1.0)),
            (1400.0, 900.0),
        )
    };
    app.st.view = MainView::Tasks;
    app.st.project = Some("web".into());
    app.st.tasks.mode = Mode::Board;
    let todo: Vec<String> = (0..LONG).step_by(2).map(id).collect();
    let done: Vec<String> = (1..LONG).step_by(2).map(id).collect();
    support::wait_for("the tasks", || {
        app.frame(&mut ui, size);
        ui.rect_of(card_id(&todo[0])).map(|_| ())
    });
    app.frame(&mut ui, size);
    let built = |ui: &Ui| {
        todo.iter()
            .filter(|t| ui.rect_of(card_id(t)).is_some())
            .count()
    };
    assert!(
        built(&ui) < 40,
        "{} of {} To do cards built",
        built(&ui),
        todo.len()
    );
    assert!(ui.rect_of(card_id(&todo[todo.len() - 1])).is_none());

    // Scrolled to the end, the last card shows inside its column.
    let scroll = column_scroll_id("todo");
    for _ in 0..6 {
        ui.set_scroll_offset(scroll, f32::MAX / 4.0);
        app.frame(&mut ui, size);
    }
    let view = ui.rect_of(scroll).expect("the To do column");
    let last = ui
        .rect_of(card_id(&todo[todo.len() - 1]))
        .expect("the last card");
    assert!(
        last.h > 0.0 && last.y + last.h <= view.y + view.h + 1.0,
        "{last:?} in {view:?}"
    );
    assert!(built(&ui) < 40);
    ui.set_scroll_offset(scroll, 0.0);
    app.frame(&mut ui, size);

    // The arrows walk the cards, scrolling them into view.
    let send = |app: &mut App, ui: &mut Ui, code: &str, m: u16| {
        app.frame(ui, size);
        app.send(ui, key(code, m, None));
        app.frame(ui, size);
    };
    ui.set_focus(Some(card_id(&todo[0])));
    send(&mut app, &mut ui, "ArrowDown", 0);
    assert_eq!(ui.focus(), Some(card_id(&todo[1])));
    send(&mut app, &mut ui, "ArrowRight", 0);
    assert_eq!(ui.focus(), Some(card_id(&done[1])));
    for _ in 0..60 {
        send(&mut app, &mut ui, "ArrowDown", 0);
    }
    assert_eq!(ui.focus(), Some(card_id(&done[61])));
    let column = ui
        .rect_of(column_scroll_id("done"))
        .expect("the Done column");
    let card = ui.rect_of(card_id(&done[61])).expect("the focused card");
    assert!(
        card.h > 0.0 && card.y >= column.y - 0.5,
        "{card:?} in {column:?}"
    );
    assert!(
        card.y + card.h <= column.y + column.h + 0.5,
        "{card:?} in {column:?}"
    );
    // Option-Left moves it a column over, still focused.
    send(&mut app, &mut ui, "ArrowLeft", mods::ALT);
    let moved = app.store.tasks.iter().find(|t| t.id == done[61]).unwrap();
    assert_eq!(json!(moved.status), "in_review");
    assert_eq!(ui.focus(), Some(card_id(&done[61])));
    ui.set_focus(None);

    // A card held at the foot of Done scrolls it, and drops there.
    let from = ui.rect_of(card_id(&todo[0])).expect("the first card");
    let target = ui
        .rect_of(column_scroll_id("done"))
        .expect("the Done column");
    let before = ui.scroll_offset(column_scroll_id("done"));
    ui.pointer_move(from.x + from.w / 2.0, from.y + from.h / 2.0);
    ui.pointer_down();
    app.events(&mut ui);
    let (x1, y1) = (target.x + target.w / 2.0, target.y + target.h - 0.5);
    for i in 1..=30 {
        let f = (i as f32 / 8.0).min(1.0);
        let (x0, y0) = (from.x + from.w / 2.0, from.y + from.h / 2.0);
        ui.pointer_move(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
        app.events(&mut ui);
        app.frame(&mut ui, size);
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    assert_eq!(
        app.st.tasks.dragging,
        Some((todo[0].clone(), Some("done".to_owned())))
    );
    let after = ui.scroll_offset(column_scroll_id("done"));
    assert!(after > before, "the column scrolled: {before} to {after}");
    ui.pointer_up();
    app.events(&mut ui);
    app.frame(&mut ui, size);
    let dropped = app.store.tasks.iter().find(|t| t.id == todo[0]).unwrap();
    assert_eq!(json!(dropped.status), "done");
    support::wait_for("the drop saved", || {
        (saved(&app, &todo[0])?["status"] == "done").then_some(())
    });
}

#[test]
fn a_long_board_builds_only_the_cards_in_view() {
    long_board(false);
}

#[test]
fn a_long_board_on_cells_builds_only_the_cards_in_view() {
    long_board(true);
}

/// What a cells frame of the app shows, row after row.
fn cells_text(app: &mut App) -> String {
    use vorn_app::ui::backend::{Backend, Cells};
    let mut ui = Ui::cells(&ui_config(1.0));
    let size = vorn_app::ui::cells::units::px(160, 40);
    app.frame(&mut ui, size);
    app.frame(&mut ui, size);
    let mut target = Cells::offscreen(160, 40);
    target.present(&mut ui).expect("present");
    (0..40)
        .map(|r| target.buffer.row_text(r))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_session_working_on_a_task_shows_it_on_its_card_and_off_tasks_are_kept() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let project = add_project(&mut app, &v, "web", json!({}));
    let info = app
        .rpc()
        .call("shell:create", json!(project))
        .expect("a shell");
    let sid = info["id"].as_str().expect("id").to_owned();
    let made = app
        .rpc()
        .call(
            "task:create",
            json!({ "projectName": "web", "title": "Badge me", "status": "in_progress" }),
        )
        .expect("a task");
    let id = made["task"]["id"].as_str().expect("id").to_owned();
    app.rpc()
        .call(
            "task:patch",
            json!({ "id": id, "set": { "assignedSessionId": sid } }),
        )
        .expect("assigned");
    app.st.view = MainView::Sessions;
    // The badge shows once its row and its template have both come.
    let mut shown = String::new();
    support::wait_for("the badge on the card", || {
        let rows = app.st.points.rows_of("vorn.tasks");
        let row = rows.get("working").is_some_and(|w| {
            w.iter()
                .any(|r| r.id == sid && r.text("title") == "Badge me")
        });
        let point = app.st.points.find("vorn.tasks", "badges", "task").is_some();
        shown = cells_text(&mut app);
        (row && point && shown.contains("Badge me")).then_some(())
    });

    // Switched off, the board says so, the tasks stay, and writes are refused.
    app.rpc()
        .call(
            "extension:setBuiltin",
            json!({ "id": "vorn.tasks", "on": false }),
        )
        .expect("switched off");
    app.st.view = MainView::Tasks;
    support::wait_for("the board gone", || {
        let mut ui = Ui::cells(&ui_config(1.0));
        app.frame(&mut ui, (1200.0, 800.0));
        (app.st.points.loaded && app.st.points.find("vorn.tasks", "views", "board").is_none())
            .then_some(())
    });
    let off = cells_text(&mut app);
    assert!(off.contains("Tasks is off"), "{off}");
    let refused = app
        .rpc()
        .call("task:setStatus", json!({ "id": id, "status": "done" }));
    assert!(refused.is_err());
    let listed = app
        .rpc()
        .call("task:list", json!({}))
        .expect("the core lists");
    assert_eq!(listed[0]["title"], "Badge me");
    app.rpc()
        .call(
            "extension:setBuiltin",
            json!({ "id": "vorn.tasks", "on": true }),
        )
        .expect("switched on");
}
