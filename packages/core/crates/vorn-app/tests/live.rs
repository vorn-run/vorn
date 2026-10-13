//! The app against a real vornd on a temporary HOME: the main screen's data
//! loads, a shell opens from the keyboard and takes typing, and the
//! composer starts a session in a project. Frames are drawn offscreen, on
//! the GPU when there is one.

mod support;

use serde_json::json;
use support::{add_project, connect, key, wait_for, Shot, TestVornd};
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
    grid.attach("not-yet", 80, 24, 0);
    assert_eq!(grid.attached(), ["not-yet"]);
    wait_for("the refused attach to be forgotten", || {
        grid.attached().is_empty().then_some(())
    });
    assert!(grid.errors().is_empty(), "{:?}", grid.errors());
}
