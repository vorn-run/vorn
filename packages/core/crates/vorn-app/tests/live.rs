//! The app against a real vornd on a temporary HOME: the main screen's data
//! loads, a shell opens from the keyboard and takes typing, and the
//! composer starts a session in a project. Frames are drawn offscreen, on
//! the GPU when there is one.

mod support;

use serde_json::json;
use std::time::{Duration, Instant};
use support::{add_project, connect, key, wait_for, Shot, TestVornd};
use vorn_app::app::MOD;
use vorn_app::screen::term_id;
use vorn_app::ui::{mods, Input, WheelPhase};
use vorn_app::App;

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
    shot.send(&mut app, key("Comma", MOD(), Some(",")));
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

    // Enter without a project does nothing.
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
    assert_eq!(session.info.agent_type.as_str(), "acme");
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

/// One trackpad event, then a frame 8 ms on, as a display draws them.
fn pad(shot: &mut Shot, app: &mut App, dy: f32, phase: WheelPhase) {
    shot.ui.wheel_phase(dy, phase);
    app.events(&mut shot.ui);
    shot.frame(app);
    std::thread::sleep(Duration::from_millis(8));
}

/// Frames until every spring and ease has come to rest.
fn settle(shot: &mut Shot, app: &mut App) {
    for _ in 0..80 {
        shot.frame(app);
        std::thread::sleep(Duration::from_millis(8));
    }
}

#[cfg(unix)]
#[test]
fn a_terminal_scrolls_its_history_by_the_pixel_and_typing_returns_live() {
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
    let typed = Instant::now();
    shot.type_line(&mut app, "seq 1 5000; sleep 15; seq 7001 7040");
    shot.until(&mut app, "5000 lines, some in history", |app, _| {
        app.pane(&id)
            .filter(|p| p.text().lines().any(|l| l.trim() == "5000") && p.history > 200)
            .map(|_| ())
    });
    shot.hover(&mut app, term_id(&id));
    let row = shot.ui.text.cell().h;

    // Fingers up the trackpad: the view follows by the pixel.
    pad(&mut shot, &mut app, 0.0, WheelPhase::Began);
    for _ in 0..10 {
        pad(&mut shot, &mut app, -9.0, WheelPhase::Moved);
    }
    assert_eq!(app.scroll_position(&id), 90.0);
    assert!(app.pane(&id).expect("pane").back() > 0);
    shot.save(&mut app, "scroll-mid-gesture");
    // Lifted with momentum, it carries on and rests on a whole row.
    pad(&mut shot, &mut app, -12.0, WheelPhase::Moved);
    pad(&mut shot, &mut app, 0.0, WheelPhase::Ended);
    pad(&mut shot, &mut app, 0.0, WheelPhase::Began);
    for i in 0..20 {
        pad(&mut shot, &mut app, -(20.0 - i as f32), WheelPhase::Moved);
    }
    pad(&mut shot, &mut app, 0.0, WheelPhase::Ended);
    settle(&mut shot, &mut app);
    let pos = app.scroll_position(&id);
    assert!(pos > 250.0, "{pos}");
    let back = app.pane(&id).map(|p| p.back()).expect("pane");
    assert!(
        (back as f32 * row - pos).abs() < 0.01,
        "rests on a row: {pos} by {row}"
    );
    let top = shot.until(&mut app, "history rows in view", |app, _| {
        let t = app.pane(&id)?.text();
        let first = t.lines().next()?.trim().parse::<u32>().ok()?;
        (first < 5000).then_some(first)
    });
    shot.save(&mut app, "scroll-rested");

    // New output does not move a view scrolled back.
    while typed.elapsed() < Duration::from_secs(18) {
        app.pump();
        shot.frame(&mut app);
        std::thread::sleep(Duration::from_millis(50));
    }
    let t = app.pane(&id).expect("pane").text();
    assert_eq!(
        t.lines().next().map(str::trim),
        Some(top.to_string().as_str())
    );
    assert!(app.scrolled_back(&id));

    // Pulled past the oldest line it stretches, then springs back to it.
    pad(&mut shot, &mut app, 0.0, WheelPhase::Began);
    for _ in 0..40 {
        pad(&mut shot, &mut app, -3000.0, WheelPhase::Moved);
    }
    let max = app.pane(&id).expect("pane").history as f32 * row;
    assert!(app.scroll_position(&id) > max, "stretched past the top");
    shot.until(&mut app, "the oldest rows", |app, _| {
        app.pane(&id).filter(|p| p.back() == p.history).map(|_| ())
    });
    shot.save(&mut app, "scroll-stretched-at-top");
    pad(&mut shot, &mut app, 0.0, WheelPhase::Ended);
    settle(&mut shot, &mut app);
    assert!((app.scroll_position(&id) - max).abs() < 0.01);

    // Typing returns to the live screen.
    shot.send(&mut app, Input::char('x'));
    assert!(!app.scrolled_back(&id));
    assert_eq!(app.pane(&id).expect("pane").back(), 0);
    shot.send(&mut app, key("Backspace", 0, None));
    shot.until(&mut app, "the late output on the live screen", |app, _| {
        app.pane(&id)
            .filter(|p| p.text().lines().any(|l| l.trim() == "7040"))
            .map(|_| ())
    });

    // Pulled down at the live screen it stretches and springs back.
    pad(&mut shot, &mut app, 0.0, WheelPhase::Began);
    for _ in 0..3 {
        pad(&mut shot, &mut app, 60.0, WheelPhase::Moved);
    }
    assert!(app.scroll_position(&id) < 0.0);
    shot.save(&mut app, "scroll-stretched-at-bottom");
    pad(&mut shot, &mut app, 0.0, WheelPhase::Ended);
    settle(&mut shot, &mut app);
    assert_eq!(app.scroll_position(&id), 0.0);

    // A wheel notch eases three rows.
    shot.ui.wheel(-vorn_app::ui::WHEEL_LINE);
    app.events(&mut shot.ui);
    settle(&mut shot, &mut app);
    assert!((app.scroll_position(&id) - 3.0 * row).abs() < 0.01);
    shot.save(&mut app, "scroll-notch");
    shot.send(&mut app, Input::char('x'));
    shot.send(&mut app, key("Backspace", 0, None));

    // On the alternate screen without mouse reporting, a notch is arrow keys.
    shot.type_line(&mut app, "seq 1 300 | less");
    shot.until(&mut app, "less", |app, _| {
        app.pane(&id)
            .filter(|p| p.text().lines().next().map(str::trim) == Some("1"))
            .map(|_| ())
    });
    shot.ui.wheel(vorn_app::ui::WHEEL_LINE);
    app.events(&mut shot.ui);
    shot.until(&mut app, "less three lines on", |app, _| {
        app.pane(&id)
            .filter(|p| p.text().lines().next().map(str::trim) == Some("4"))
            .map(|_| ())
    });
    assert!(!app.scrolled_back(&id));
    shot.send(&mut app, Input::char('q'));
}

/// A program that turns bracketed paste on and prints what each read
/// brings, control bytes as `^X`, as an agent CLI sees its input.
#[cfg(unix)]
const SHOW_BYTES: &str = r#"python3 -c "import os,sys,tty;tty.setraw(0);sys.stdout.write('\x1b[?2004h'+'got-'+'ready\r\n');sys.stdout.flush();[(sys.stdout.write('got '+''.join(c if ' '<=c<='~' else '^'+chr(ord(c)^64) for c in os.read(0,4096).decode())+'.\r\n'),sys.stdout.flush()) for _ in iter(int,1)]""#;

#[test]
#[cfg(unix)]
fn images_reach_an_agent_cli_as_ctrl_v_and_drops_as_quoted_paths() {
    use vorn_app::ui::{Dropped, MemoryClipboard};
    let Some(v) = TestVornd::start() else {
        return;
    };
    let paste = if cfg!(target_os = "macos") {
        key("KeyV", mods::SUPER, Some("v"))
    } else {
        key("KeyV", mods::CTRL | mods::SHIFT, Some("V"))
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let id = wait_for("the shell's card", || {
        app.pump();
        app.st.selected.clone()
    });
    shot.until(&mut app, "the shell's fitted screen", |app, _| {
        app.pane(&id).filter(|p| p.sb_epoch > 0).map(|_| ())
    });
    shot.type_line(&mut app, SHOW_BYTES);
    // What the program printed, its wrapped rows joined.
    let got = |app: &App| -> String { screen(app, &id).iter().map(|l| l.trim_end()).collect() };
    let shows = |what: String, n: usize| {
        move |app: &mut App, _: &vorn_app::ui::Ui| {
            (got(app).matches(&what).count() >= n).then_some(())
        }
    };
    shot.until(&mut app, "the byte printer", shows("got-ready".into(), 1));

    // Ctrl-V is the program's own key.
    shot.send(&mut app, key("KeyV", mods::CTRL, Some("v")));
    shot.until(&mut app, "Ctrl-V's byte", shows("got ^V.".into(), 1));

    // An image and no text: the paste key sends Ctrl-V, for the CLI to read it.
    shot.ui.set_clipboard(Box::new(MemoryClipboard {
        text: None,
        image: true,
    }));
    shot.send(&mut app, paste.clone());
    shot.until(
        &mut app,
        "the image paste's Ctrl-V",
        shows("got ^V.".into(), 2),
    );

    // Text and an image: the text, bracketed.
    shot.ui.set_clipboard(Box::new(MemoryClipboard {
        text: Some("hi there".into()),
        image: true,
    }));
    shot.send(&mut app, paste.clone());
    shot.until(
        &mut app,
        "the text paste",
        shows("got ^[[200~hi there^[[201~.".into(), 1),
    );

    // Dropped files: their quoted paths, space separated, bracketed.
    let r = app.term_box(&id).expect("the terminal is painted");
    let k = shot.ui.scale();
    let mid = ((r.x + r.w / 2.0) / k, (r.y + r.h / 2.0) / k);
    let files = vec!["/tmp/a.png".into(), "/tmp/my shot.png".into()];
    app.dropped(&mut shot.ui, Dropped::Files(files), Some(mid));
    shot.until(
        &mut app,
        "the dropped paths",
        shows(
            "got ^[[200~/tmp/a.png '/tmp/my shot.png' ^[[201~.".into(),
            1,
        ),
    );

    // A dropped image with no file: saved, and its path pasted.
    let png = b"\x89PNG\r\n\x1a\nnot really".to_vec();
    app.dropped(
        &mut shot.ui,
        Dropped::Image {
            name: "image.png".into(),
            bytes: png.clone(),
        },
        None,
    );
    let dir = std::env::temp_dir().join("vorn-images");
    let start = format!("got ^[[200~{}", dir.join("image-").display());
    let path = shot.until(&mut app, "the saved image's path", |app, _| {
        let text = got(app);
        let at = text.find(&start)?;
        let rest = &text[at + "got ^[[200~".len()..];
        rest.find(" ^[[201~.").map(|end| rest[..end].to_owned())
    });
    assert!(path.ends_with(".png"), "{path}");
    assert_eq!(std::fs::read(&path).expect("the image was saved"), png);
    let _ = std::fs::remove_file(&path);
}

#[cfg(unix)]
#[test]
fn a_terminal_shows_a_scrollbar_that_drags_pages_and_fades() {
    use vorn_app::screen::{scrollbar_id, thumb_id};
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
    shot.type_line(&mut app, "seq 1 3000");
    shot.until(&mut app, "3000 lines, most in history", |app, _| {
        app.pane(&id)
            .filter(|p| p.text().contains("3000") && p.history > 200)
            .map(|_| ())
    });
    // At rest the bar only listens for the pointer.
    shot.frame(&mut app);
    assert!(shot.ui.rect_of(scrollbar_id(&id)).is_some());
    assert_eq!(shot.ui.rect_of(thumb_id(&id)), None);

    // A wheel notch shows it, its thumb up from the bottom.
    shot.hover(&mut app, term_id(&id));
    pad(
        &mut shot,
        &mut app,
        -vorn_app::ui::WHEEL_LINE,
        WheelPhase::Line,
    );
    // Read while it shows: on a slow machine settling would outlast its fade.
    let thumb = shot.until(&mut app, "the thumb shows", |_, ui| {
        ui.rect_of(thumb_id(&id))
    });
    let bar = shot.ui.rect_of(scrollbar_id(&id)).expect("bar");
    assert!(
        thumb.y + thumb.h > bar.y + bar.h - 8.0,
        "near the bottom: {thumb:?} {bar:?}"
    );
    shot.save(&mut app, "scrollbar-shown");

    // Dragged to the top, it shows the oldest line.
    shot.ui
        .pointer_move(thumb.x + thumb.w / 2.0, thumb.y + thumb.h / 2.0);
    shot.ui.pointer_down();
    app.events(&mut shot.ui);
    for step in 1..=10 {
        shot.ui
            .pointer_move(thumb.x + thumb.w / 2.0, thumb.y - step as f32 * 200.0);
        app.events(&mut shot.ui);
        shot.frame(&mut app);
    }
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    let thumb = shot.until(&mut app, "the oldest line", |app, ui| {
        let p = app.pane(&id)?;
        (p.back() == p.history).then_some(())?;
        ui.rect_of(thumb_id(&id))
    });
    shot.save(&mut app, "scrollbar-top");

    // A click in the track below the thumb pages down.
    let before = app.pane(&id).expect("pane").back();
    let rows = app.pane(&id).expect("pane").rows;
    shot.ui
        .pointer_move(thumb.x + thumb.w / 2.0, thumb.y + thumb.h + 40.0);
    shot.ui.pointer_down();
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    let paged = before - u64::from(rows - 1);
    shot.until(&mut app, "a page down", |app, _| {
        (app.pane(&id)?.back() == paged).then_some(())
    });

    // Away from it and left alone, it fades.
    shot.ui.pointer_move(1.0, 1.0);
    let still = Instant::now();
    while still.elapsed() < Duration::from_millis(1400) {
        shot.frame(&mut app);
        std::thread::sleep(Duration::from_millis(16));
    }
    assert_eq!(shot.ui.rect_of(thumb_id(&id)), None, "faded");
    shot.save(&mut app, "scrollbar-faded");
}
