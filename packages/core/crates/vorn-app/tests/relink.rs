//! A running app outlives a vornd restart: once the new vornd answers, the
//! app's links open again by themselves, the cards it showed attach again,
//! their terminals move and resize as before, and what the app took from
//! the old endpoint follows the new one, with no app restart.

#![cfg(unix)]

mod support;

use std::path::Path;

use serde_json::{json, Value};
use support::{add_project, connect, TestVornd};
use vorn_app::App;

/// The stand-in the card runs: it repaints on every SIGWINCH.
const AGENT: &str = "acme";

/// The stand-in's repaint serial on the card's first row, and the size it names.
fn painted(app: &App, id: &str) -> Option<(String, u64)> {
    let text = app.pane(id)?.text();
    let first = text.lines().next()?;
    let mut parts = first.split(' ');
    let _row = parts.next()?;
    let size = parts.next()?.to_owned();
    let serial = parts.next()?.strip_prefix('#')?.parse().ok()?;
    Some((size, serial))
}

#[test]
fn the_grid_link_opens_again_after_vornd_restarts() {
    let Some(mut v) = TestVornd::start() else {
        return;
    };
    let python = ["/usr/bin/python3", "/opt/homebrew/bin/python3"]
        .iter()
        .find(|p| Path::new(p).is_file());
    let Some(python) = python else {
        eprintln!("skipped: no python3 for the stand-in");
        return;
    };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tui-standin.py");
    let mut app = connect(&v);
    let dir = add_project(&mut app, &v, "demo", json!({}));
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["agentCommands"][AGENT] = json!({ "command": python, "args": [script] });
    app.rpc().call("config:save", config).expect("saved");
    let mut shot = support::Shot::at((1200.0, 800.0), 1.0);
    shot.frame(&mut app);
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": AGENT, "projectName": "demo", "projectPath": dir }),
        )
        .expect("a card");
    let id = info["id"].as_str().expect("id").to_owned();
    let before = shot.until(&mut app, "the card's first paint", |app, _| {
        painted(app, &id)
    });

    v.restart_server();

    // A new size reaches the program through the new link and its repaint
    // reaches the card: the link is open again, both ways.
    shot.size = (1000.0, 700.0);
    let after = shot.until(
        &mut app,
        "the card to follow after the restart",
        |app, _| painted(app, &id).filter(|(size, serial)| *serial > before.1 && *size != before.0),
    );
    assert!(after.1 > before.1, "{before:?} then {after:?}");
    // And calls reach the new vornd, whose web address the app now offers.
    app.rpc()
        .call("config:load", Value::Null)
        .expect("calls reach the new vornd");
    assert_eq!(app.web_url, format!("http://127.0.0.1:{}/app/", v.port()));
}
