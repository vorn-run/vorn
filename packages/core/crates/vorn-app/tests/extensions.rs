//! An extension's panes on a card, on a vornd with its own HOME: the card
//! menu offers them, a page pane opens beside the terminal and closes, and a
//! program pane draws its terminal and takes the keys.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use std::fs;
use std::path::Path;

use serde_json::json;
use support::{add_project, connect, key, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::extensions::ExtAct;
use vorn_app::screen::menus::MENU;
use vorn_app::screen::term_id;
use vorn_app::state::MenuKind;
use vorn_app::ui::{item_id, mods};
use vorn_app::App;

const FIXTURE: &str = include_str!("../../extensions/tests/fixtures/extension.mjs");

/// Puts an extension pack where vornd keeps them.
fn install(home: &Path) {
    let dir = home.join("connectors/rep/1.0.0");
    fs::create_dir_all(dir.join("web")).expect("pack dir");
    fs::write(dir.join("index.js"), FIXTURE).expect("index");
    fs::write(dir.join("package.json"), r#"{"type":"module"}"#).expect("package");
    fs::write(dir.join("web/index.html"), "<p>pane</p>").expect("page");
    let manifest = r#"id = "rep"
name = "Reporter"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "index.js"]

[connector]

[[contributes.panes]]
id = "page"
title = "Report"
web = "web/index.html"

[[contributes.panes]]
id = "prog"
title = "Tail"
command = ["sh", "-c", "printf 'from the program'; sleep 60"]
"#;
    fs::write(dir.join("vorn-extension.toml"), manifest).expect("manifest");
    fs::write(
        home.join("connectors/rep/current.json"),
        json!({ "version": "1.0.0", "installedAt": 1 }).to_string(),
    )
    .expect("current");
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

/// The card's More menu, where its panes are offered.
fn open_menu(shot: &mut Shot, app: &mut App, id: &str) {
    shot.hover(app, term_id(id));
    shot.click(
        app,
        App::control("card", &Act::Open(MenuKind::Card(id.to_owned()))),
    );
}

#[test]
fn an_extension_pane_opens_from_the_card_menu() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipped: no node for the extension host");
        return;
    }
    install(&v.home);
    let mut app = connect(&v);
    add_project(&mut app, &v, "api", json!({}));
    let mut shot = Shot::new();
    shot.send(&mut app, key("Backquote", mods::CTRL, Some("`")));
    let id = shot.until(&mut app, "a shell", |app, _| app.st.selected.clone());
    shot.until(&mut app, "the extension active on it", |app, _| {
        (app.st.ext.loaded && !app.st.ext.items(&id).is_empty()).then_some(())
    });

    open_menu(&mut shot, &mut app, &id);
    let labels = app.menu_labels(&shot.ui);
    // Past the buttons the card shows, its panes come first, in their order.
    let rename = labels.iter().position(|l| l == "Rename\u{2026}");
    let report = labels.iter().position(|l| l == "Report");
    assert!(report.is_some() && report < rename, "{labels:?}");
    let panes = report.map(|r| &labels[r..r + 2]);
    assert_eq!(
        panes,
        Some(&["Report".to_owned(), "Tail".to_owned()][..]),
        "{labels:?}"
    );
    shot.save(&mut app, "ext-menu");
    choose(&mut shot, &mut app, "Report");
    let url = shot.until(&mut app, "the page pane", |app, _| {
        app.st.ext.panes.get(&id).and_then(|p| p.url.clone())
    });
    assert!(url.contains("/extensions/rep/pane/page/"), "{url}");
    shot.save(&mut app, "ext-page");

    // The menu now closes it.
    open_menu(&mut shot, &mut app, &id);
    choose(&mut shot, &mut app, "Close Report");
    assert!(!app.st.ext.panes.contains_key(&id));

    // A program pane draws its terminal and takes the keys once clicked.
    open_menu(&mut shot, &mut app, &id);
    choose(&mut shot, &mut app, "Tail");
    let terminal = shot.until(&mut app, "the program pane", |app, _| {
        app.st
            .ext
            .panes
            .get(&id)
            .and_then(|p| p.terminal_id.clone())
    });
    shot.until(&mut app, "its screen", |app, _| {
        app.pane(&terminal).map(|_| ())
    });
    shot.click(
        &mut app,
        App::control("ext-focus", &Act::Ext(ExtAct::Focus(id.clone()))),
    );
    assert_eq!(app.st.ext.focus.as_deref(), Some(id.as_str()));
    shot.save(&mut app, "ext-program");

    shot.click(
        &mut app,
        App::control(
            "pane",
            &Act::Pane(vorn_app::panes::PaneAct::Close(
                id.clone(),
                vorn_app::panes::PaneKind::Extension,
            )),
        ),
    );
    shot.frame(&mut app);
    assert!(!app.st.ext.panes.contains_key(&id));
}
