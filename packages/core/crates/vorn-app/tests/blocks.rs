//! A shell's finished commands as blocks, against a real vornd and an
//! integrated shell: each command with its output, exit status and
//! duration; the one running; and the composer's history and completions.

mod support;

use std::path::Path;

use serde_json::json;
use support::{add_project, connect, key, wait_for, Shot, TestVornd};
use vorn_app::screen::intent_id;
use vorn_app::ui::mods;

/// A shell with Vorn's integration on this machine.
fn integrated_shell() -> Option<&'static str> {
    ["/bin/zsh", "/usr/bin/zsh", "/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
}

#[cfg(unix)]
#[test]
fn a_shells_commands_become_blocks_with_status_and_duration() {
    let Some(shell) = integrated_shell() else {
        return;
    };
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({ "shell": shell }));
    let mut shot = Shot::new();
    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let id = wait_for("the shell's card", || {
        app.pump();
        app.st.selected.clone()
    });
    shot.until(&mut app, "the shell's first prompt", |app, _| {
        app.st
            .shells
            .get(&id)
            .is_some_and(|sh| sh.integrated())
            .then_some(())
    });
    shot.click(&mut app, intent_id(&id));

    shot.type_line(&mut app, "echo vorn-42");
    let out = shot.until(&mut app, "the echo's block", |app, _| {
        let sh = app.st.shells.get(&id)?;
        let b = sh.blocks.last()?;
        (b.command.as_deref() == Some("echo vorn-42")).then(|| b.text())
    });
    assert!(out.lines().any(|l| l.trim() == "vorn-42"), "{out}");

    shot.type_line(&mut app, "false");
    let exit = shot.until(&mut app, "the failed block", |app, _| {
        let b = app.st.shells.get(&id)?.blocks.last()?;
        (b.command.as_deref() == Some("false")).then_some(b.exit_code)
    });
    assert_eq!(exit, Some(1));

    shot.type_line(&mut app, "sleep 2");
    let running = shot.until(&mut app, "the running command", |app, _| {
        app.st.shells.get(&id)?.running()
    });
    assert_eq!(running.command.as_deref(), Some("sleep 2"));
    std::thread::sleep(std::time::Duration::from_millis(400));
    shot.save(&mut app, "blocks-running");
    let took = shot.until(&mut app, "the sleep's block", |app, _| {
        let b = app.st.shells.get(&id)?.blocks.last()?;
        (b.command.as_deref() == Some("sleep 2")).then_some(b.duration_ms)
    });
    assert!(took.is_some_and(|ms| ms >= 1500), "{took:?}");
    assert!(app.st.shells[&id].running().is_none());
    shot.save(&mut app, "blocks");

    // ↑ opens the commands run before, newest first.
    shot.click(&mut app, intent_id(&id));
    shot.send(&mut app, key("ArrowUp", 0, None));
    let first = shot.until(&mut app, "the history menu", |app, _| {
        let m = app.st.intent.menus.get(&id)?;
        m.open.then(|| m.history.first().map(|e| e.text.clone()))?
    });
    assert_eq!(first, "sleep 2");
    shot.save(&mut app, "blocks-history");
    shot.send(&mut app, key("Escape", 0, None));

    shot.type_text(&mut app, "git ch");
    let labels = shot.until(&mut app, "git's completions", |app, _| {
        let m = app.st.intent.menus.get(&id)?;
        let l: Vec<String> = m.completions.iter().map(|c| c.label.clone()).collect();
        (!l.is_empty()).then_some(l)
    });
    assert!(labels.iter().any(|l| l == "checkout"), "{labels:?}");
    shot.save(&mut app, "blocks-completions");

    let kept = std::fs::read_to_string(vorn_app::intent::history_file(&v.home)).expect("history");
    assert!(kept.contains("echo vorn-42"), "{kept}");
}
