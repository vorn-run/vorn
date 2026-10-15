//! A real extension (examples/extensions/turns) with a template for every
//! place that takes one, drawn by the one renderer on each surface: the
//! window (Pixels) and the terminal (Cells) at 120x36 and 80x24. Its views
//! also redraw live as their files change.
//! `VORN_APP_SHOT=<prefix>` saves each state: a PNG, and the terminal's
//! cells as HTML and text.

#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;
use support::{add_project, connect, key, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::screen::menus::MENU;
use vorn_app::screen::term_id;
use vorn_app::state::MenuKind;
use vorn_app::ui::backend::{Backend, Cells};
use vorn_app::ui::{cells::units, item_id, mods, Ui};
use vorn_app::{ui_config, App};

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../examples/extensions/turns"
);
const ID: &str = "example.turns";
const SIZES: [(u16, u16); 2] = [(120, 36), (80, 24)];

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("dir");
    for e in fs::read_dir(from).expect("example") {
        let e = e.expect("entry");
        let at = to.join(e.file_name());
        if e.file_type().expect("type").is_dir() {
            copy_dir(&e.path(), &at);
        } else {
            fs::copy(e.path(), at).expect("copy");
        }
    }
}

fn has_node() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Links a copy of the example, so the test may edit its views.
fn link(app: &App, home: &Path) -> PathBuf {
    let dir = home.join("turns");
    copy_dir(Path::new(EXAMPLE), &dir);
    let linked = app
        .rpc()
        .call("extension:link", json!({ "path": dir }))
        .expect("linked");
    assert_eq!(linked["id"], ID, "{linked}");
    dir
}

fn point(kind: &str, id: &str) -> String {
    format!("{ID}/{kind}/{id}")
}

/// This state on a terminal at each size, as text, saved when asked.
fn cells(app: &mut App, name: &str) -> [String; 2] {
    let (chrome, traffic) = (app.chrome, app.traffic);
    (app.chrome, app.traffic) = (false, false);
    let out = SIZES.map(|(cols, rows)| {
        let mut ui = Ui::cells(&ui_config(1.0));
        let size = units::px(cols, rows);
        app.frame(&mut ui, size);
        app.frame(&mut ui, size);
        let mut target = Cells::offscreen(cols, rows);
        target.present(&mut ui).expect("present");
        let text: Vec<String> = (0..rows).map(|r| target.buffer.row_text(r)).collect();
        let text = text.join("\n");
        if let Some(base) = std::env::var_os("VORN_APP_SHOT") {
            let path = format!("{}-cells-{name}-{cols}x{rows}", base.to_string_lossy());
            let html = target.buffer.to_html(&format!("{name} {cols}x{rows}"));
            fs::write(format!("{path}.html"), html).expect("html");
            fs::write(format!("{path}.txt"), &text).expect("txt");
        }
        text
    });
    (app.chrome, app.traffic) = (chrome, traffic);
    out
}

/// Draws this state on every surface and checks each terminal size shows
/// `wide` at 120x36 and `narrow` at 80x24.
#[track_caller]
fn every_surface(shot: &mut Shot, app: &mut App, name: &str, wide: &[&str], narrow: &[&str]) {
    shot.save(app, name);
    let [w, n] = cells(app, name);
    for t in wide {
        assert!(w.contains(t), "{name} 120x36 shows {t:?}:\n{w}");
    }
    for t in narrow {
        assert!(n.contains(t), "{name} 80x24 shows {t:?}:\n{n}");
    }
    shot.frame(app);
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

fn open_card_menu(shot: &mut Shot, app: &mut App, id: &str) {
    shot.hover(app, term_id(id));
    shot.click(
        app,
        App::control("card", &Act::Open(MenuKind::Card(id.to_owned()))),
    );
}

#[test]
fn every_template_kind_draws_on_every_surface() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    if !has_node() {
        eprintln!("skipped: no node for the example's program");
        return;
    }
    let mut app = connect(&v);
    let project = add_project(&mut app, &v, "api", json!({}));
    let dir = link(&app, &v.home);
    let mut shot = Shot::new();
    let all = [
        point("sidebar", "finished"),
        point("badges", "count"),
        point("cardFooters", "last"),
        point("decorations", "line"),
        point("panes", "all"),
        point("composer", "summary"),
        point("settings", "prefs"),
        point("windows", "turns"),
        point("dialogs", "clear"),
        point("commands", "toggle"),
        point("menus", "reset"),
        point("launchers", "window"),
    ];
    shot.until(&mut app, "every contribution", |a, _| {
        all.iter()
            .all(|k| a.st.points.get(k).is_some_and(|p| p.template.is_ok()))
            .then_some(())
    });

    // No session yet: the empty state's launcher.
    every_surface(
        &mut shot,
        &mut app,
        "t-launcher",
        &["Watch turns"],
        &["Watch turns"],
    );

    // An agent (a shell standing in) goes quiet: a turn finished.
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": "claude", "projectName": "api", "projectPath": project }),
        )
        .expect("an agent");
    let sid = info["id"].as_str().expect("id").to_owned();
    shot.until(&mut app, "a finished turn", |a, _| {
        let rows = a.st.points.rows_of(ID);
        let counted = rows
            .get("sessions")?
            .iter()
            .any(|r| r.id == sid && r.fields.get("turns").and_then(|t| t.as_f64()) >= Some(1.0));
        let listed = rows.get("finished").is_some_and(|f| !f.is_empty());
        (counted && listed).then_some(())
    });
    app.st.selected = Some(sid.clone());
    // First seen without sessions, the project starts folded.
    app.st.collapsed.remove("api");
    every_surface(
        &mut shot,
        &mut app,
        "t-card",
        &[" done", "finished at", "Finished turns", "Turns: "],
        &[" done", "finished at"],
    );

    // The new-session composer: its strip over the prompt, its launcher under it.
    shot.send(&mut app, key("KeyN", MOD(), Some("n")));
    assert!(app.st.composer_open);
    every_surface(
        &mut shot,
        &mut app,
        "t-composer",
        &["Finished turns: ", "Watch turns"],
        &["Finished turns: ", "Watch turns"],
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert!(!app.st.composer_open);

    // The card menu: its template pane, and the extension's item.
    open_card_menu(&mut shot, &mut app, &sid);
    let labels = app.menu_labels(&shot.ui);
    assert!(labels.iter().any(|l| l == "Reset Turn Count"), "{labels:?}");
    choose(&mut shot, &mut app, "Turns");
    shot.until(&mut app, "the template pane", |a, _| {
        a.st.ext.panes.get(&sid).map(|_| ())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-pane",
        &["At work: 0 of 1"],
        &["At work: 0 of 1"],
    );
    open_card_menu(&mut shot, &mut app, &sid);
    choose(&mut shot, &mut app, "Close Turns");
    assert!(!app.st.ext.panes.contains_key(&sid));

    // Its settings page, among Vorn's own.
    shot.send(&mut app, key("Comma", MOD(), None));
    assert!(app.st.settings.open);
    let page = point("settings", "prefs");
    shot.click(
        &mut app,
        App::control(
            "settings-nav-ext",
            &Act::Point(vorn_app::points::PointAct::Settings(page.clone())),
        ),
    );
    shot.until(&mut app, "the prefs", |a, _| {
        a.st.points
            .rows_of(ID)
            .get("prefs")
            .is_some_and(|p| !p.is_empty())
            .then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-settings",
        &["Count shells too", "the latest first"],
        &["Count shells too"],
    );
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert!(!app.st.settings.open);

    // A palette command that opens its dialog.
    shot.send(&mut app, key("KeyK", MOD(), None));
    shot.type_text(&mut app, "clear finished turns");
    shot.frame(&mut app);
    every_surface(
        &mut shot,
        &mut app,
        "t-palette",
        &["Clear Finished Turns…"],
        &["Clear Finished Turns…"],
    );
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    let dialog = point("dialogs", "clear");
    shot.until(&mut app, "the dialog", |a, _| {
        (a.st.points.dialog.as_deref() == Some(dialog.as_str())).then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-dialog",
        &["Clear finished turns?"],
        &["Clear finished turns?"],
    );
    let clear = shot.button(&mut app, "Clear").expect("the dialog's Clear");
    shot.click(&mut app, clear);
    shot.until(&mut app, "the list cleared", |a, _| {
        a.st.points
            .rows_of(ID)
            .get("finished")?
            .is_empty()
            .then_some(())
    });
    assert_eq!(app.st.points.dialog, None);

    // Its key opens its window, which floats in a window without its own.
    let window = point("windows", "turns");
    shot.send(&mut app, key("KeyT", MOD() | mods::ALT, None));
    shot.until(&mut app, "the window", |a, _| {
        a.st.points.windows.contains(&window).then_some(())
    });
    every_surface(
        &mut shot,
        &mut app,
        "t-window",
        &["At work: 0 of 1"],
        &["At work: 0 of 1"],
    );
    shot.send(&mut app, key("KeyT", MOD() | mods::ALT, None));
    shot.frame(&mut app);
    assert!(!app.st.points.windows.contains(&window));

    // The card menu's item calls the program with the card's session.
    open_card_menu(&mut shot, &mut app, &sid);
    choose(&mut shot, &mut app, "Reset Turn Count");
    shot.until(&mut app, "the count reset", |a, _| {
        let rows = a.st.points.rows_of(ID).get("sessions")?;
        rows.iter()
            .any(|r| r.id == sid && r.fields.get("turns").and_then(|t| t.as_f64()) == Some(0.0))
            .then_some(())
    });

    live_preview(&mut shot, &mut app, &dir);
}

/// A saved view redraws at once; a broken one shows where and why instead.
fn live_preview(shot: &mut Shot, app: &mut App, dir: &Path) {
    let view = dir.join("views/settings.kdl");
    let was = fs::read_to_string(&view).expect("view");
    shot.send(app, key("Comma", MOD(), None));
    let page = point("settings", "prefs");
    shot.click(
        app,
        App::control(
            "settings-nav-ext",
            &Act::Point(vorn_app::points::PointAct::Settings(page.clone())),
        ),
    );
    // Timed to the frame that has the new template; the terminal's text then shows it.
    let redraw = |shot: &mut Shot, app: &mut App, text: &str, what: &str| {
        let t = Instant::now();
        wait_for(what, || {
            shot.frame(app);
            let p = app.st.points.get(&page)?;
            let now = match &p.template {
                Ok(t) => format!("{t:?}"),
                Err(e) => e.join("\n"),
            };
            now.contains(text).then_some(())
        });
        let took = t.elapsed();
        let [w, _] = cells(app, "t-live");
        assert!(w.contains(text), "{what} shows {text:?}:\n{w}");
        took
    };
    fs::write(&view, was.replace("Counting", "What counts")).expect("edit");
    let took = redraw(shot, app, "What counts", "the edited view");
    eprintln!("live preview: {took:?}");
    assert!(took < Duration::from_millis(250), "redrawn in {took:?}");

    fs::write(
        &view,
        was.replace("section \"Counting\"", "sectoin \"Counting\""),
    )
    .expect("break");
    redraw(shot, app, "views/settings.kdl:", "the error box");
    let [w, _] = cells(app, "t-error");
    assert!(w.contains("sectoin"), "the error names what is wrong:\n{w}");
    shot.save(app, "t-error");

    fs::write(&view, &was).expect("fix");
    redraw(shot, app, "Counting", "the fixed view");
}
