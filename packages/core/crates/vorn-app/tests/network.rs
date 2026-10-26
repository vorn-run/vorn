//! The Remote Access page driven as a user would, on a vornd with its own
//! HOME and ports: share this machine on and off, pair a phone by code (the
//! phone's half played over HTTP on loopback) in the Add a device sheet,
//! mint a token for another computer under Advanced, and remove a device
//! from its menu.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use std::io::{Read, Write};
use std::net::TcpStream;

use serde_json::{json, Value};
use support::{connect, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::client::Endpoint;
use vorn_app::network::{NetAct, DEVICE_NAME};
use vorn_app::screen::settings::CONTENT;
use vorn_app::settings::Category;
use vorn_app::state::{Dialog, MenuKind};
use vorn_app::ui::Id;
use vorn_app::App;

fn net(place: &str, act: NetAct) -> Id {
    App::control(place, &Act::Net(act))
}

/// A JSON POST to vornd's HTTP port on loopback, as the phone app sends.
fn post(port: u16, path: &str, body: &Value) -> Value {
    let body = body.to_string();
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("vornd's http port");
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).expect("request");
    let mut res = String::new();
    s.read_to_string(&mut res).expect("response");
    let (_, json) = res.split_once("\r\n\r\n").expect("a head and a body");
    serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}: {res}"))
}

fn sharing(app: &App) -> bool {
    app.store
        .config
        .pointer("/defaults/networkAccessEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Waits until the page's addresses say `remote`, reopening the page when
/// its one recheck came before vornd finished rebinding.
fn until_remote(shot: &mut Shot, app: &mut App, remote: bool) {
    shot.until(app, "the addresses after the rebind", |app, _| {
        let n = &app.st.network;
        let now = n.reachable.as_ref().map(|r| r.remote);
        if now == Some(remote) {
            return Some(());
        }
        if n.recheck_at.is_none() && now.is_some() {
            app.st.settings.open = false;
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        show_remote(app);
        None
    });
}

fn show_remote(app: &mut App) {
    app.st.settings.open = true;
    app.st.settings.category = Category::Network;
}

#[test]
fn remote_access_page() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let port = Endpoint::find(&v.home).expect("endpoint").port;
    let mut app = connect(&v);
    let mut shot = Shot::new();

    // Opened as a settings shell would: the page loads itself.
    show_remote(&mut app);
    shot.until(&mut app, "the status and devices", |app, _| {
        let n = &app.st.network;
        (n.open && !n.loading && n.tokens.is_some() && n.reachable.is_some()).then_some(())
    });
    assert!(!sharing(&app), "a fresh HOME shares nothing");
    assert!(!app.st.network.can_scan(), "loopback only: no code to scan");
    assert_eq!(app.st.network.active().len(), 0);
    // This computer's web client, before sharing anything: its address to copy or open.
    assert_eq!(app.web_url, format!("http://127.0.0.1:{port}/app/"));
    shot.frame(&mut app);
    let copy = net("net-copy", NetAct::Copy(app.web_url.clone()));
    assert!(
        shot.ui.rect_of(copy).is_some(),
        "Open in a browser offers it to copy"
    );
    shot.save(&mut app, "page");

    // Share this machine. Off a tailnet it asks first.
    shot.click(&mut app, net("net-share", NetAct::Share(true)));
    if app.st.network.confirming {
        shot.save(&mut app, "confirm");
        shot.click(&mut app, net("net-share", NetAct::EnableAnyway));
    }
    shot.until(&mut app, "the switch saved", |app, _| {
        sharing(app).then_some(())
    });
    until_remote(&mut shot, &mut app, true);
    assert!(app.st.network.can_scan());
    shot.save(&mut app, "shared");

    // Add a device by code; the phone redeems it over HTTP.
    shot.click(&mut app, net("net-add", NetAct::AddDevice));
    assert_eq!(app.st.dialog, Some(Dialog::AddDevice));
    let code = shot.until(&mut app, "a pairing code", |app, _| {
        match &app.st.network.adding {
            Some(Some(c)) => Some(c.code.clone()),
            _ => None,
        }
    });
    assert!(app.st.network.pair_url().is_some());
    shot.until(&mut app, "the QR code", |app, _| {
        app.st.network.qr.as_ref().map(|_| ())
    });
    let link = app.st.network.qr.as_ref().map(|(l, _)| l.clone()).unwrap();
    assert!(link.starts_with("vorn://pair?url=") && link.contains(&code));
    shot.save(&mut app, "code");
    let redeemed = post(
        port,
        "/api/pair/redeem",
        &json!({ "code": code, "deviceName": "Test phone" }),
    );
    let request = redeemed["requestId"]
        .as_str()
        .expect("a request id")
        .to_owned();
    shot.until(&mut app, "the approval prompt", |app, _| {
        app.st.network.asking.as_ref().map(|_| ())
    });
    assert_eq!(
        app.st
            .network
            .asking
            .as_ref()
            .map(|a| a.device_name.as_str()),
        Some("Test phone")
    );
    shot.save(&mut app, "asking");
    shot.click(&mut app, net("net-decide", NetAct::Decide(true)));
    let collected = wait_for("the phone to collect its token", || {
        let polled = post(port, "/api/pair/poll", &json!({ "requestId": request }));
        (polled["status"] == "approved").then_some(polled)
    });
    assert!(collected["token"].as_str().is_some_and(|t| !t.is_empty()));
    shot.until(&mut app, "the phone in the list", |app, _| {
        let n = &app.st.network;
        (n.active().iter().any(|t| t.name == "Test phone") && n.adding.is_none()).then_some(())
    });
    assert_eq!(app.st.dialog, None, "approving closes the sheet");
    shot.save(&mut app, "devices");

    // Escape closes a sheet and retires its code.
    shot.click(&mut app, net("net-add", NetAct::AddDevice));
    shot.until(&mut app, "a second code", |app, _| {
        matches!(app.st.network.adding, Some(Some(_))).then_some(())
    });
    shot.send(&mut app, support::key("Escape", 0, None));
    shot.frame(&mut app);
    assert_eq!(app.st.dialog, None);
    assert!(app.st.network.adding.is_none());
    assert!(app.st.settings.open, "Escape closes only the sheet");

    // A token for another computer, under Advanced, shown once.
    let id = net("net-advanced", NetAct::ToggleAdvanced);
    shot.reveal(&mut app, CONTENT, id);
    shot.click(&mut app, id);
    let id = net("net-name-token", NetAct::NameToken);
    shot.reveal(&mut app, CONTENT, id);
    shot.click(&mut app, id);
    assert_eq!(shot.ui.focus(), Some(DEVICE_NAME));
    shot.save(&mut app, "token-form");
    shot.type_line(&mut app, "Laptop");
    let plaintext = shot.until(&mut app, "the minted token", |app, _| {
        app.st.network.minted.as_ref().map(|m| m.plaintext.clone())
    });
    assert!(!plaintext.is_empty());
    assert_eq!(
        app.st.network.minted.as_ref().map(|m| m.name.as_str()),
        Some("Laptop")
    );
    shot.until(&mut app, "the laptop in the list", |app, _| {
        (app.st.network.active().len() == 2).then_some(())
    });
    shot.save(&mut app, "minted");
    let id = net("net-minted", NetAct::DoneMinted);
    shot.reveal(&mut app, CONTENT, id);
    shot.click(&mut app, id);
    assert!(app.st.network.minted.is_none());

    // Revoke it: Remove asks, Revoke does.
    let id = app
        .st
        .network
        .active()
        .iter()
        .find(|t| t.name == "Laptop")
        .map(|t| t.id.clone())
        .expect("the laptop's token");
    let menu = App::control("net-device", &Act::Open(MenuKind::Device(id.clone())));
    shot.reveal(&mut app, CONTENT, menu);
    shot.click(&mut app, menu);
    let remove = shot
        .labelled(&mut app, "MenuItem", "Remove\u{2026}")
        .expect("Remove in the device's menu");
    shot.click(&mut app, remove);
    assert_eq!(app.st.network.confirm_revoke.as_deref(), Some(id.as_str()));
    shot.save(&mut app, "revoke");
    shot.click(&mut app, net("net-revoke", NetAct::Revoke(id.clone())));
    shot.until(&mut app, "the laptop revoked", |app, _| {
        let n = &app.st.network;
        let gone = n
            .tokens
            .iter()
            .flatten()
            .any(|t| t.id == id && t.revoked_at.is_some());
        (gone && n.active().len() == 1).then_some(())
    });
    let listed = app.rpc().call("token:list", Value::Null).expect("tokens");
    let revoked = listed
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["id"] == id.as_str())
        .map(|t| !t["revokedAt"].is_null());
    assert_eq!(revoked, Some(true), "vornd holds it revoked");

    // Stop sharing: back to loopback.
    let off = net("net-share", NetAct::Share(false));
    shot.reveal(&mut app, CONTENT, off);
    shot.click(&mut app, off);
    shot.until(&mut app, "the switch off", |app, _| {
        (!sharing(app)).then_some(())
    });
    until_remote(&mut shot, &mut app, false);

    // Closing the page forgets it.
    app.st.settings.open = false;
    shot.frame(&mut app);
    assert!(!app.st.network.open);
}
