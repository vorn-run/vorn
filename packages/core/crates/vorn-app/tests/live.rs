//! The app against a real vornd on a temporary HOME: the main screen's data
//! loads, a shell opens from the keyboard and takes typing, and the
//! composer starts a session in a project. Frames are drawn offscreen, on
//! the GPU when there is one.

mod support;

use serde_json::json;
use support::{add_project, connect, key, wait_for, Shot, TestVornd};
use vorn_app::app::MOD;
use vorn_app::ui::mods;

#[test]
fn a_shell_opens_from_the_keyboard_and_takes_typing() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    assert!(app.store.sessions.is_empty());
    assert!(app.composing(), "an empty grid shows the composer");
    let mut shot = Shot::new();
    shot.save(&mut app, "composer");

    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let id = wait_for("the shell's card", || {
        app.pump();
        app.st.selected.clone()
    });
    assert!(app.store.session(&id).is_some_and(|s| s.is_shell()));
    assert!(!app.composing(), "a card replaces the composer");

    // The frame attaches the card's terminal at the size of its box.
    shot.until(&mut app, "the shell's first screen", |app, _| {
        app.pane(&id).map(|_| ())
    });
    // The typed line, then the shell's answer on a line of its own.
    shot.type_line(&mut app, "echo vorn-42");
    let screen = shot.until(&mut app, "the shell's answer", |app, _| {
        app.pane(&id)
            .map(|p| p.text())
            .filter(|t| t.lines().any(|l| l.trim() == "vorn-42"))
    });
    assert!(screen.contains("echo vorn-42"), "{screen}");
    shot.save(&mut app, "shell");
}

#[cfg(unix)]
#[test]
fn tab_and_its_variants_reach_a_focused_terminal() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let id = wait_for("the shell's card", || {
        app.pump();
        app.st.selected.clone()
    });
    shot.until(&mut app, "the shell's first screen", |app, _| {
        app.pane(&id).map(|_| ())
    });
    shot.type_line(&mut app, "stty -echo; cat -vt; stty echo");
    // cat has the terminal once the shell no longer echoes.
    std::thread::sleep(std::time::Duration::from_millis(500));
    for m in [0, mods::SHIFT, mods::CTRL, mods::CTRL | mods::SHIFT] {
        shot.send(&mut app, key("Tab", m, Some("\t")));
        assert_eq!(shot.ui.focus(), None, "Tab with {m:#b} moved focus");
    }
    shot.send(&mut app, key("Escape", 0, None));
    shot.type_line(&mut app, "|end");
    let screen = shot.until(&mut app, "cat's line", |app, _| {
        app.pane(&id)
            .map(|p| p.text())
            .filter(|t| t.contains("|end"))
    });
    assert!(screen.contains("^I^[[Z^I^[[Z^[|end"), "{screen}");
    assert!(app.terminal_has_keys(&shot.ui));

    // Settings hold the keyboard: Tab moves between their controls.
    shot.send(&mut app, key("Comma", MOD, Some(",")));
    assert!(app.st.settings.open);
    shot.send(&mut app, key("Tab", 0, Some("\t")));
    assert!(shot.ui.focus().is_some(), "Tab focuses a settings control");
}

/// The rows of `id`'s screen, trailing blanks dropped.
fn screen(app: &vorn_app::App, id: &str) -> Vec<String> {
    app.pane(id)
        .map(|p| p.text().lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The middle of cell `(col, row)` in `id`'s terminal, in window pixels.
fn cell_xy(app: &vorn_app::App, shot: &Shot, id: &str, col: u16, row: usize) -> (f32, f32) {
    let r = app.term_box(id).expect("the terminal is painted");
    let cell = shot.ui.text.cell();
    let s = shot.ui.scale();
    (
        (r.x.round() + (f32::from(col) + 0.5) * cell.w) / s,
        (r.y.round() + (row as f32 + 0.5) * cell.h) / s,
    )
}

/// The first row whose text is `line`.
fn row_of(app: &vorn_app::App, id: &str, line: &str) -> usize {
    screen(app, id)
        .iter()
        .position(|l| l.trim() == line)
        .unwrap_or_else(|| panic!("{line:?} on {:#?}", screen(app, id)))
}

#[cfg(unix)]
#[test]
fn the_clipboard_pastes_into_a_terminal_and_copies_what_is_selected() {
    use vorn_app::screen::term_id;
    use vorn_app::state::Dialog;
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mac = cfg!(target_os = "macos");
    let (copy, paste) = if mac {
        (
            key("KeyC", mods::SUPER, Some("c")),
            key("KeyV", mods::SUPER, Some("v")),
        )
    } else {
        (
            key("KeyC", mods::CTRL | mods::SHIFT, Some("C")),
            key("KeyV", mods::CTRL | mods::SHIFT, Some("V")),
        )
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let id = wait_for("the shell's card", || {
        app.pump();
        app.st.selected.clone()
    });
    // The card's fitted size renumbers the lines; select after it lands.
    shot.until(&mut app, "the shell's fitted screen", |app, _| {
        app.pane(&id).filter(|p| p.sb_epoch > 0).map(|_| ())
    });
    // Each command prints what its own text does not contain, so an answer
    // is found even after a prompt the shell printed ahead of it.
    let shows = |line: &'static str| {
        let id = id.clone();
        move |app: &mut vorn_app::App, _: &vorn_app::ui::Ui| {
            screen(app, &id)
                .iter()
                .any(|l| l.trim_end().ends_with(line))
                .then_some(())
        }
    };

    // One line goes straight in, as typing would.
    shot.ui.clipboard().set("echo pasted-$((0+1))");
    shot.send(&mut app, paste.clone());
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    shot.until(&mut app, "the pasted command's answer", shows("pasted-1"));

    // The shell would run each line of this as it arrives: the app asks first.
    shot.ui
        .clipboard()
        .set("echo two-$((0+1))\necho two-$((0+2))\n");
    shot.send(&mut app, paste.clone());
    shot.until(&mut app, "the paste confirm", |app, _| {
        (app.st.dialog == Some(Dialog::Paste)).then_some(())
    });
    shot.save(&mut app, "paste-confirm");
    assert!(
        !app.terminal_has_keys(&shot.ui),
        "the dialog holds the keys"
    );
    shot.send(&mut app, key("Escape", 0, None));
    assert_eq!(app.st.dialog, None);
    shot.send(&mut app, key("Insert", mods::SHIFT, None));
    shot.until(&mut app, "the paste confirm again", |app, _| {
        (app.st.dialog == Some(Dialog::Paste)).then_some(())
    });
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    shot.until(&mut app, "the confirmed paste's answer", shows("two-2"));
    let ran = screen(&app, &id)
        .iter()
        .filter(|l| l.trim_end().ends_with("two-1"))
        .count();
    assert_eq!(ran, 1, "only the confirmed paste ran");
    assert_eq!(app.st.dialog, None);

    // A drag across a row selects it; the copy key copies it.
    let row = row_of(&app, &id, "pasted-1");
    let (x0, y) = cell_xy(&app, &shot, &id, 0, row);
    let (x1, _) = cell_xy(&app, &shot, &id, 20, row);
    shot.ui.pointer_move(x0, y);
    shot.ui.pointer_down();
    shot.ui.pointer_move(x1, y);
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    assert!(app.st.sel.is_some(), "the drag selected");
    shot.save(&mut app, "selection");
    shot.ui.clipboard().set("");
    shot.send(&mut app, copy.clone());
    assert!(app.st.sel.is_none(), "copying clears the selection");
    assert_eq!(copied(&mut shot, &mut app, "the copied row"), "pasted-1");

    // A double-click selects the word under it; the menu's Copy copies it.
    // The drag's press is a click too: wait until it no longer pairs.
    std::thread::sleep(std::time::Duration::from_millis(600));
    shot.frame(&mut app);
    let row = row_of(&app, &id, "pasted-1");
    let (x, y) = cell_xy(&app, &shot, &id, 1, row);
    shot.ui.pointer_move(x, y);
    shot.ui.pointer_down();
    app.events(&mut shot.ui);
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    shot.ui.pointer_down();
    app.events(&mut shot.ui);
    // vornd answers before the button comes up, as it does under a hand.
    shot.until(&mut app, "the word's selection", |app, _| {
        app.st.sel.as_ref().map(|_| ())
    });
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    assert!(app.st.sel.is_some(), "the release keeps the word");
    shot.right_click(&mut app, term_id(&id));
    assert_eq!(app.menu_labels(&shot.ui)[0], "Copy");
    shot.ui.clipboard().set("");
    choose(&mut shot, &mut app, "Copy");
    assert_eq!(copied(&mut shot, &mut app, "the copied word"), "pasted-1");

    // The menu's Paste pastes; Ctrl-C with nothing selected still interrupts.
    shot.ui.clipboard().set("sleep 30");
    shot.right_click(&mut app, term_id(&id));
    choose(&mut shot, &mut app, "Paste");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    std::thread::sleep(std::time::Duration::from_millis(300));
    shot.send(&mut app, key("KeyC", mods::CTRL, Some("c")));
    shot.type_line(&mut app, "echo after-$((0+9))");
    shot.until(&mut app, "the shell back from sleep", shows("after-9"));
}

/// What the copy put on the clipboard, once it has; the caller empties it
/// before the copy.
fn copied(shot: &mut Shot, app: &mut vorn_app::App, what: &str) -> String {
    wait_for(what, || {
        shot.frame(app);
        shot.ui.clipboard().get().filter(|t| !t.is_empty())
    })
}

fn choose(shot: &mut Shot, app: &mut vorn_app::App, label: &str) {
    let labels = app.menu_labels(&shot.ui);
    let i = labels
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("{label:?} in {labels:?}"));
    shot.click(app, vorn_app::ui::item_id(vorn_app::screen::menus::MENU, i));
}

#[cfg(unix)]
#[test]
fn the_composer_starts_the_default_agent_in_the_chosen_project() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let project = add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);

    // Enter without a project does nothing, as in the renderer.
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    app.pump();
    assert!(!app.st.composer.launching);

    shot.send(&mut app, key("Tab", 0, Some("\t")));
    assert_eq!(app.st.composer.project.as_deref(), Some("demo"));
    shot.type_line(&mut app, "hello");
    let id = wait_for("the agent's card", || {
        app.pump();
        app.st.selected.clone()
    });
    let session = app.store.session(&id).expect("card");
    assert_eq!(session.info.agent_type.as_str(), "claude");
    assert_eq!(session.info.project_name, "demo");
    assert!(
        app.st.composer.text.is_empty(),
        "a launch clears the prompt"
    );
    assert_eq!(app.st.composer.error, None);

    // A shell beside it starts in the selected card's project.
    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let shell = wait_for("the second card", || {
        app.pump();
        app.st.selected.clone().filter(|s| *s != id)
    });
    let path = &app.store.session(&shell).expect("card").info.project_path;
    assert_eq!(std::path::Path::new(path), project);
    shot.until(&mut app, "both screens", |app, _| {
        (app.pane(&id).is_some() && app.pane(&shell).is_some()).then_some(())
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    shot.save(&mut app, "grid");
}

#[test]
fn an_attach_before_the_session_exists_is_asked_again() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let app = connect(&v);
    let grid = app.grid().expect("grid");
    grid.attach("not-yet", 80, 24);
    assert_eq!(grid.attached(), ["not-yet"]);
    wait_for("the refused attach to be forgotten", || {
        grid.attached().is_empty().then_some(())
    });
    assert!(grid.errors().is_empty(), "{:?}", grid.errors());
}
