//! Agents from their adapters, end to end against a real vornd: the example
//! card footer (examples/extensions/agent-footer) shows an adapter's agent's
//! model and tool calls, from the agent's hooks, on the window (Pixels) and
//! the terminal (Cells); and with every adapter switched off the launcher
//! opens a plain shell.
//! `VORN_APP_SHOT=<prefix>` saves each state.

#![cfg(unix)]

mod support;

use std::io::{Read, Write};
use std::path::Path;

use serde_json::{json, Value};
use support::{add_project, connect, wait_for, Shot, TestVornd};
use vorn_app::ui::backend::{Backend, Cells};
use vorn_app::ui::{cells::units, Ui};
use vorn_app::{ui_config, App};

const SIZES: [(u16, u16); 2] = [(120, 36), (80, 24)];

/// This state as a terminal shows it, at each size.
fn cells(app: &mut App) -> [String; 2] {
    let (chrome, traffic) = (app.chrome, app.traffic);
    (app.chrome, app.traffic) = (false, false);
    let out = SIZES.map(|(cols, rows)| {
        let mut ui = Ui::cells(&ui_config(1.0));
        let size = units::px(cols, rows);
        app.frame(&mut ui, size);
        app.frame(&mut ui, size);
        let mut target = Cells::offscreen(cols, rows);
        target.present(&mut ui).expect("present");
        (0..rows)
            .map(|r| target.buffer.row_text(r))
            .collect::<Vec<_>>()
            .join("\n")
    });
    (app.chrome, app.traffic) = (chrome, traffic);
    out
}

/// Posts a hook event to the endpoint vornd registered in the test HOME, as
/// an agent's hook does.
fn post_hook(home: &Path, body: &Value) {
    let dir = home.join(".vorn");
    let (port, token) = wait_for("the hook endpoint", || {
        let port = std::fs::read_to_string(dir.join("port")).ok()?;
        let token = std::fs::read_to_string(dir.join("token")).ok()?;
        Some((port.trim().to_owned(), token.trim().to_owned()))
    });
    let body = body.to_string();
    let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("hooks");
    write!(
        stream,
        "POST /hooks HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("sent");
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
}

const FOOTER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../examples/extensions/agent-footer"
);
const FOOTER_ID: &str = "example.agent-footer";

fn has_node() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_card_footer_draws_an_adapters_agents_tool_calls_on_every_surface() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    if !has_node() {
        eprintln!("skipped: no node for the example's program");
        return;
    }
    let mut app = connect(&v);
    let project = add_project(&mut app, &v, "api", json!({}));
    let linked = app
        .rpc()
        .call("extension:link", json!({ "path": FOOTER }))
        .expect("linked");
    assert_eq!(linked["id"], FOOTER_ID, "{linked}");
    // Its program starts on its first call or event; this call starts it.
    app.rpc()
        .call(
            "extension:call",
            json!({ "id": FOOTER_ID, "action": "start", "args": {} }),
        )
        .expect("started");
    let mut shot = Shot::new();
    shot.until(&mut app, "the example's card footer", |a, _| {
        a.st.points
            .at("cardFooters")
            .any(|p| p.extension == FOOTER_ID)
            .then_some(())
    });
    // The linked test adapter's agent, run by a shell.
    let agent = "acme";
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": agent, "projectName": "api", "projectPath": project, "model": "mini" }),
        )
        .expect("a session");
    let sid = info["id"].as_str().expect("id").to_owned();
    post_hook(
        &v.home,
        &json!({ "hook_event_name": "PreToolUse", "session_id": "conv", "cwd": project,
                 "tool_name": "edit", "vorn_terminal_id": sid }),
    );
    shot.until(&mut app, "the session's row", |a, _| {
        a.st.points
            .rows_of(FOOTER_ID)
            .get("sessions")?
            .iter()
            .any(|r| r.id == sid && r.fields.get("tools").and_then(Value::as_f64) == Some(1.0))
            .then_some(())
    });
    app.st.selected = Some(sid.clone());
    app.st.collapsed.remove("api");
    shot.save(&mut app, "adapter-footer");
    let [wide, narrow] = cells(&mut app);
    for (size, text) in [("120x36", &wide), ("80x24", &narrow)] {
        assert!(
            text.contains("1 tool calls"),
            "{size} shows the tool calls:\n{text}"
        );
        assert!(
            text.contains("last edit"),
            "{size} shows the last tool:\n{text}"
        );
    }
    assert!(
        wide.contains("mini"),
        "the model it was started with:\n{wide}"
    );
}

#[test]
fn with_every_adapter_off_the_launcher_opens_a_plain_shell() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "web", json!({}));
    // The linked test agent goes, then every shipped adapter is switched off.
    app.rpc()
        .call("extension:remove", json!({ "id": "test.acme" }))
        .expect("removed");
    let builtins = app
        .rpc()
        .call("extension:builtins", Value::Null)
        .expect("builtins");
    let adapters: Vec<String> = app
        .store
        .agents
        .iter()
        .filter(|a| a.builtin)
        .map(|a| a.extension.clone())
        .collect();
    assert!(!adapters.is_empty());
    for b in builtins.as_array().into_iter().flatten() {
        let id = b["id"].as_str().unwrap_or_default();
        if adapters.iter().any(|a| a == id) {
            app.rpc()
                .call("extension:setBuiltin", json!({ "id": id, "on": false }))
                .expect("switched off");
        }
    }
    let mut shot = Shot::new();
    shot.until(&mut app, "no agent on", |a, _| {
        a.store.agents.is_empty().then_some(())
    });
    assert_eq!(app.store.default_agent(), "shell");
    // The launcher starts a shell in the chosen project.
    shot.frame(&mut app);
    shot.send(&mut app, support::key("Tab", 0, Some("\t")));
    assert_eq!(app.st.composer.project.as_deref(), Some("web"));
    shot.type_line(&mut app, "ls");
    let id = wait_for("the session", || {
        app.pump();
        app.st.selected.clone()
    });
    let session = app.store.session(&id).expect("its card");
    assert!(session.is_shell(), "{:?}", session.info.agent_type);
    // Switched on again, its agents come back.
    app.rpc()
        .call(
            "extension:setBuiltin",
            json!({ "id": adapters[0], "on": true }),
        )
        .expect("switched on");
    shot.until(&mut app, "an agent on", |a, _| {
        (!a.store.agents.is_empty()).then_some(())
    });
}
