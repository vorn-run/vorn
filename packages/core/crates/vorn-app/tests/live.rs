//! The app against a real vornd on a temporary HOME: the main screen's data
//! loads, a shell opens from the keyboard and takes typing, and the
//! composer starts a session in a project. Frames are drawn offscreen when
//! the machine has a GPU; without one the terminal is attached directly.

mod support;

use std::sync::Arc;

use support::{wait_for, TestVornd};
use vorn_app::client::Endpoint;
use vorn_app::ui::{mods, Gpu, Input, Offscreen, Ui, OFFSCREEN_FORMAT};
use vorn_app::{look, ui_config, App};

fn key(code: &str, m: u16, text: Option<&str>) -> Input {
    Input::Key {
        code: code.into(),
        mods: m,
        text: text.map(Into::into),
    }
}

fn type_text(app: &mut App, s: &str) {
    for ch in s.chars() {
        app.input(Input::char(ch));
    }
    app.input(key("Enter", 0, Some("\r")));
}

fn connect(v: &TestVornd) -> App {
    let ep = Endpoint::find(&v.home).expect("vornd wrote ws-port and local-token");
    assert_eq!(
        ep.grid, v.grid,
        "the grid endpoint is derived as vornd names it"
    );
    let app = App::connect(&ep, Arc::new(|| {})).expect("connects and loads");
    assert_eq!(app.grid_error, None);
    app
}

/// An offscreen 1440x900 frame, when there is a GPU to draw it.
struct Shot {
    ui: Ui,
    target: Offscreen,
}

impl Shot {
    fn new() -> Option<Shot> {
        let gpu = Gpu::headless().map_err(|e| eprintln!("no GPU: {e}")).ok()?;
        let ui = Ui::new(gpu, OFFSCREEN_FORMAT, &ui_config(1.0));
        let px = (look::WINDOW.0 as u32, look::WINDOW.1 as u32);
        let target = Offscreen::new(&ui.gpu, px);
        Some(Shot { ui, target })
    }

    fn draw(&mut self, app: &mut App) {
        app.frame(&mut self.ui, look::WINDOW);
        self.ui.render(&self.target.view, self.target.size);
    }

    /// Saves the frame as `$VORN_APP_SHOT-<name>.png`, when that is set.
    fn save(&self, name: &str) {
        if let Some(base) = std::env::var_os("VORN_APP_SHOT") {
            let path = format!("{}-{name}.png", base.to_string_lossy());
            self.target.save_png(&self.ui.gpu, &path).expect("png");
        }
    }
}

#[test]
fn a_shell_opens_from_the_keyboard_and_takes_typing() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    assert!(app.store.sessions.is_empty());
    assert!(app.composing(), "an empty grid shows the composer");
    let mut shot = Shot::new();
    if let Some(s) = &mut shot {
        s.draw(&mut app);
        s.save("composer");
    }

    app.input(key("Backquote", mods::CTRL, Some("`")));
    let id = wait_for("the shell's card", || {
        app.pump();
        app.selected.clone()
    });
    assert!(app.store.session(&id).is_some_and(|s| s.is_shell()));
    assert!(!app.composing(), "a card replaces the composer");

    match &mut shot {
        // The frame attaches the card's terminal at the size of its box.
        Some(s) => s.draw(&mut app),
        None => app.grid().expect("grid").attach(&id, 80, 24),
    }
    wait_for("the shell's first screen", || {
        app.pump();
        app.pane(&id).map(|_| ())
    });
    // The typed line, then the shell's answer on a line of its own.
    type_text(&mut app, "echo vorn-42");
    let screen = wait_for("the shell's answer", || {
        app.pump();
        app.pane(&id)
            .map(|p| p.text())
            .filter(|t| t.lines().any(|l| l.trim() == "vorn-42"))
    });
    assert!(screen.contains("echo vorn-42"), "{screen}");

    if let Some(s) = &mut shot {
        s.draw(&mut app);
        s.save("shell");
    }
}

#[cfg(unix)]
#[test]
fn the_composer_starts_the_default_agent_in_the_chosen_project() {
    use serde_json::{json, Value};

    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let project = v.home.join("demo");
    std::fs::create_dir_all(&project).expect("project dir");

    // A project, and an agent that is only a shell, so nothing real runs.
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["projects"] = json!([{
        "name": "demo",
        "path": project,
        "preferredAgents": [],
    }]);
    config["agentCommands"]["claude"] = json!({ "command": "/bin/sh", "args": [] });
    config["defaults"]["defaultAgent"] = json!("claude");
    app.rpc().call("config:save", config).expect("saved");
    wait_for("the project to arrive", || {
        app.pump();
        (!app.store.projects.is_empty()).then_some(())
    });

    // Enter without a project does nothing, as in the renderer.
    app.input(key("Enter", 0, Some("\r")));
    app.pump();
    assert!(!app.composer.launching);

    app.input(key("Tab", 0, Some("\t")));
    assert_eq!(app.composer.project.as_deref(), Some("demo"));
    type_text(&mut app, "hello");
    let id = wait_for("the agent's card", || {
        app.pump();
        app.selected.clone()
    });
    let session = app.store.session(&id).expect("card");
    assert_eq!(session.info.agent_type.as_str(), "claude");
    assert_eq!(session.info.project_name, "demo");
    assert!(app.composer.text.is_empty(), "a launch clears the prompt");
    assert_eq!(app.composer.error, None);

    // A shell beside it starts in the selected card's project.
    app.input(key("Backquote", mods::CTRL, Some("`")));
    let shell = wait_for("the second card", || {
        app.pump();
        app.selected.clone().filter(|s| *s != id)
    });
    let path = &app.store.session(&shell).expect("card").info.project_path;
    assert_eq!(std::path::Path::new(path), project);
    if let Some(mut s) = Shot::new() {
        s.draw(&mut app);
        wait_for("both screens", || {
            app.pump();
            (app.pane(&id).is_some() && app.pane(&shell).is_some()).then_some(())
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        s.draw(&mut app);
        s.save("grid");
    }
}
