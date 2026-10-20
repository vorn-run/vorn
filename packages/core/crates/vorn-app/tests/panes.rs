//! The wave 4 panes driven as a user would, on a vornd with its own HOME:
//! the Browser's pane from the card's globe with its address bar and tabs
//! (the `vorn.browser` built-in's tree), switched off, an
//! artifact the agent publishes with its bar, versions, doc editor and
//! comments rail, a design's canvas and tweaks, and the Device's pane (the
//! `vorn.device` built-in's tree) with its picker and controls, switched
//! off. No web view is seated offscreen, so the page itself never loads, and
//! no simulator is started; what a page would report (a design's manifest)
//! is set on the app's state. `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{add_project, connect, key, wait_for, Shot, TestVornd};
use vorn_app::act::{Act, Acts};
use vorn_app::app::MOD;
use vorn_app::panes::{
    browser_key, device_key, note_id, video_part, PaneAct, PaneKind, BROWSER, DEVICE,
};
use vorn_app::screen::term_id;
use vorn_app::ui::Id;
use vorn_app::App;
use vorn_app::artifact;
use vorn_tree::Kind;

fn pane(act: PaneAct) -> Id {
    App::control("pane", &Act::Pane(act))
}

fn card(act: PaneAct) -> Id {
    App::control("card", &Act::Pane(act))
}

/// An agent session in project `demo`, started from the composer, its
/// card's actions showing.
fn agent(shot: &mut Shot, app: &mut App) -> String {
    app.answer_agents();
    shot.frame(app);
    shot.send(app, key("Tab", 0, Some("\t")));
    shot.type_line(app, "hello");
    let id = wait_for("the agent's card", || {
        app.pump();
        app.st.selected.clone()
    });
    shot.until(app, "its screen", |app, _| app.pane(&id).map(|_| ()));
    shot.hover(app, term_id(&id));
    id
}

/// Asks vornd as an agent's tools do, pumping the app that answers.
fn ask(shot: &mut Shot, app: &mut App, method: &str, params: Value) -> Value {
    let pending = app.rpc().request(method, params);
    shot.until(app, method, |app, _| {
        app.pump();
        pending.poll()
    })
    .unwrap_or_else(|e| panic!("{method}: {e:?}"))
}

/// The page the session's pane shows, once the app has vetted it.
fn shown(app: &App, sid: &str) -> Option<String> {
    app.st
        .bench
        .pages
        .get(sid)
        .filter(|u| !u.is_empty())
        .cloned()
}

/// The labels of the Browser's tabs, as its tree draws them.
fn tabs(app: &App, sid: &str) -> Vec<String> {
    let Some(tree) = browser_key(&app.st.points, sid).and_then(|k| app.st.points.tree(&k)) else {
        return Vec::new();
    };
    let mut stack = vec![tree];
    while let Some(n) = stack.pop() {
        if let Kind::Tabs(t) = &n.kind {
            return t.tabs.iter().map(|t| t.label.clone()).collect();
        }
        stack.extend(&n.children);
    }
    Vec::new()
}

/// Frames until the pane shows `url`.
fn until_shown(shot: &mut Shot, app: &mut App, sid: &str, url: &str) {
    shot.until(app, url, |app, _| {
        app.pump();
        (shown(app, sid).as_deref() == Some(url)).then_some(())
    });
}

/// The page's view sits in its pane, under the address bar, in physical
/// pixels at the window's scale: never scaled twice off the pane.
fn page_under_its_address_bar(shot: &mut Shot, app: &mut App, address: Id) {
    for scale in [1.0, 2.0] {
        let mut at = Shot::at(shot.size, scale);
        at.frame(app);
        let bar = at.ui.rect_of(address).expect("address bar");
        let placed: Vec<_> = app.placed_pages().values().copied().collect();
        assert_eq!(placed.len(), 1, "one page shown");
        let page = placed[0];
        let (x, y) = (page.x / scale, page.y / scale);
        let (right, bottom) = ((page.x + page.w) / scale, (page.y + page.h) / scale);
        assert!(
            y >= bar.y + bar.h - 1.0,
            "below the bar at {scale}x: {page:?} {bar:?}"
        );
        assert!(
            x <= bar.x && right >= bar.x + bar.w,
            "as wide as the pane at {scale}x"
        );
        assert!(
            right <= at.size.0 + 0.5 && bottom <= at.size.1 + 0.5,
            "inside the window at {scale}x"
        );
        assert!(
            y - (bar.y + bar.h) < 40.0,
            "right under the bar at {scale}x"
        );
    }
    shot.frame(app);
}

#[test]
fn the_browser_pane_opens_from_the_card_and_follows_the_agent() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    let sid = agent(&mut shot, &mut app);

    // The globe opens an empty pane beside the terminal.
    shot.click(&mut app, card(PaneAct::ToggleBrowser(sid.clone())));
    until_shown(&mut shot, &mut app, &sid, "about:blank");
    assert_eq!(app.st.bench.kinds(&sid), [PaneKind::Browser]);
    assert_eq!(tabs(&app, &sid), ["New tab"]);
    shot.save(&mut app, "browser-blank");

    // The address bar loads what was typed.
    let address = Acts::id("address", &Act::Pane(PaneAct::Address(sid.clone())));
    shot.click(&mut app, address);
    shot.type_line(&mut app, "localhost:3000");
    until_shown(&mut shot, &mut app, &sid, "http://localhost:3000/");
    shot.save(&mut app, "browser");
    page_under_its_address_bar(&mut shot, &mut app, address);

    // Escape puts the address back to the page actually loaded.
    shot.click(&mut app, address);
    shot.type_text(&mut app, "/elsewhere");
    assert!(shot.ui.edit_text(address).contains("/elsewhere"));
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    assert_eq!(shot.ui.edit_text(address), "http://localhost:3000/");
    assert!(shot.ui.focused(address));
    assert_eq!(shown(&app, &sid).as_deref(), Some("http://localhost:3000/"));

    // A second tab from the strip, then the agent's view of them.
    let new_tab = shot
        .labelled(&mut app, "Button", "New tab")
        .expect("the strip's new tab");
    shot.click(&mut app, new_tab);
    shot.until(&mut app, "a second tab", |app, _| {
        app.pump();
        (tabs(app, &sid).len() == 2).then_some(())
    });
    shot.save(&mut app, "browser-tabs");
    let listed = ask(
        &mut shot,
        &mut app,
        "browser:listTabs",
        json!({ "sessionId": sid }),
    );
    assert_eq!(listed["tabs"].as_array().map(Vec::len), Some(2), "{listed}");

    let went = ask(
        &mut shot,
        &mut app,
        "browser:navigate",
        json!({ "sessionId": sid, "url": "example.com" }),
    );
    assert!(went.is_object(), "{went}");
    until_shown(&mut shot, &mut app, &sid, "https://example.com/");

    // The globe closes it again.
    shot.click(&mut app, card(PaneAct::ToggleBrowser(sid.clone())));
    shot.until(&mut app, "the pane closed", |app, _| {
        app.pump();
        app.st.bench.kinds(&sid).is_empty().then_some(())
    });
}

#[test]
fn switched_off_the_browser_leaves_no_globe_and_refuses_its_calls() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    let sid = agent(&mut shot, &mut app);
    shot.click(&mut app, card(PaneAct::ToggleBrowser(sid.clone())));
    until_shown(&mut shot, &mut app, &sid, "about:blank");

    app.rpc()
        .call(
            "extension:setBuiltin",
            json!({ "id": BROWSER, "on": false }),
        )
        .expect("switched off");
    shot.until(&mut app, "the pane gone", |app, _| {
        app.pump();
        (app.st.points.loaded && app.st.bench.kinds(&sid).is_empty()).then_some(())
    });
    shot.hover(&mut app, term_id(&sid));
    assert!(
        shot.labelled(&mut app, "Button", "Open browser").is_none(),
        "no globe while it is off"
    );
    let refused = app
        .rpc()
        .call("browser:listTabs", json!({ "sessionId": sid }))
        .expect_err("refused");
    let refused = format!("{refused:?}");
    assert!(
        refused.contains("Browser is off. Turn it on in Settings \u{2192} Extensions."),
        "{refused}"
    );
}

const DOC: &str = "# Plan\n\nShip the panes first.\n\nThen the device.";

#[test]
fn a_published_artifact_shows_its_bar_editor_and_rail() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    let sid = agent(&mut shot, &mut app);

    // Publishing opens the artifact in the session's pane.
    let published = ask(
        &mut shot,
        &mut app,
        "artifact:publish",
        json!({ "sessionId": sid, "title": "Plan", "kind": "doc", "content": DOC }),
    );
    let art = published["artifact"]["id"].as_str().expect("id").to_owned();
    shot.until(&mut app, "the artifact's state", |app, _| {
        app.st.bench.artifact(&sid).and_then(|(_, s)| s).map(|_| ())
    });
    let (tab, _) = app.st.bench.artifact(&sid).expect("artifact tab");
    assert_eq!((tab.id.as_str(), tab.version), (art.as_str(), 1));
    shot.save(&mut app, "artifact-doc");

    // The versions menu, after a second version.
    ask(
        &mut shot,
        &mut app,
        "artifact:publish",
        json!({ "sessionId": sid, "artifactId": art, "title": "Plan", "kind": "doc",
                "content": format!("{DOC}\n\nAnd the canvas.") }),
    );
    shot.until(&mut app, "version 2", |app, _| {
        let (tab, state) = app.st.bench.artifact(&sid)?;
        (tab.version == 2 && state?.artifact.latest_version == 2).then_some(())
    });
    shot.click(&mut app, pane(PaneAct::Versions(sid.clone())));
    assert!(app.st.bench.live(&sid).is_some_and(|l| l.versions));
    shot.save(&mut app, "artifact-versions");
    shot.send(&mut app, key("Escape", 0, None));
    if app.st.bench.live(&sid).is_some_and(|l| l.versions) {
        shot.click(&mut app, pane(PaneAct::Versions(sid.clone())));
    }

    // Edit opens the doc editor on the latest source.
    shot.click(&mut app, pane(PaneAct::Edit(sid.clone())));
    let draft = shot.until(&mut app, "the doc's source", |app, _| {
        app.st.bench.live(&sid)?.editing.clone()
    });
    assert!(draft.original.ends_with("And the canvas."), "{draft:?}");
    shot.save(&mut app, "doc-editor");
    shot.click(&mut app, pane(PaneAct::Discard(sid.clone())));
    assert!(app.st.bench.live(&sid).is_some_and(|l| l.editing.is_none()));

    // Comments: the rail, and a note saved from it.
    shot.click(&mut app, pane(PaneAct::Comments(sid.clone())));
    assert!(app.st.bench.live(&sid).is_some_and(|l| l.commenting));
    shot.click(&mut app, note_id(&sid));
    shot.type_text(&mut app, "Tighten the intro");
    shot.send(&mut app, key("Enter", MOD(), Some("\r")));
    shot.until(&mut app, "the note saved", |app, _| {
        let (drafts, _) = app.st.bench.comments(&sid);
        drafts
            .iter()
            .any(|c| c.body == "Tighten the intro")
            .then_some(())
    });
    shot.save(&mut app, "artifact-rail");
}

const DESIGN: &str = r##"{"kind":"design","title":"Pricing","tweaks":{
  "accent":{"type":"color","default":"#6366f1","label":"Accent"},
  "dense":{"type":"boolean","default":false,"label":"Dense"},
  "plan":{"type":"select","default":"pro","options":["free","pro","team"],"label":"Plan"},
  "radius":{"type":"number","default":12,"min":0,"max":32,"step":2,"unit":"px","label":"Radius"}},
  "artboards":[{"id":"mobile","label":"Mobile","width":390,"height":844},
               {"id":"desktop","label":"Desktop","width":1280,"height":800}]}"##;

#[test]
fn a_design_lays_its_artboards_on_the_canvas_with_its_tweaks() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    let sid = agent(&mut shot, &mut app);
    let page = format!(
        "<!doctype html><script id=\"artifact\" type=\"application/json\">{DESIGN}</script><h1>Pricing</h1>"
    );
    ask(
        &mut shot,
        &mut app,
        "artifact:publish",
        json!({ "sessionId": sid, "title": "Pricing", "kind": "design", "content": page }),
    );
    shot.until(&mut app, "the design's tab", |app, _| {
        app.pump();
        app.st.bench.artifact(&sid).map(|_| ())
    });

    // What the page's manifest says, as it would report it once loaded.
    let manifest = artifact::parse_manifest(DESIGN).expect("manifest");
    app.st.bench.live_mut(&sid).manifest = Some(manifest);
    shot.frame(&mut app);
    assert_eq!(
        app.st.bench.board(&sid).map(|b| b.id.as_str()),
        Some("mobile")
    );
    shot.save(&mut app, "design-canvas");

    shot.click(
        &mut app,
        App::control(
            "pane-board",
            &Act::Pane(PaneAct::Board(sid.clone(), "desktop".into())),
        ),
    );
    assert_eq!(
        app.st.bench.board(&sid).map(|b| b.id.as_str()),
        Some("desktop")
    );
    shot.click(&mut app, pane(PaneAct::Zoom(sid.clone(), 1)));
    assert!(app.st.bench.live(&sid).and_then(|l| l.zoom).is_some());

    let dense = App::control(
        "pane-tweak",
        &Act::Pane(PaneAct::Tweak {
            session: sid.clone(),
            key: "dense".into(),
            value: "true".into(),
        }),
    );
    shot.click(&mut app, dense);
    assert_eq!(
        app.st.bench.live(&sid).map(|l| l.values["dense"].clone()),
        Some(Value::Bool(true))
    );
    shot.save(&mut app, "design-tweaks");
}

/// The Device's video part for the session, as its tree last drew it.
fn video(app: &App, sid: &str) -> Option<vorn_tree::Video> {
    let key = device_key(&app.st.points, sid)?;
    let tree = app.st.points.tree(&key)?;
    video_part(tree, sid).map(|(_, v)| v.clone())
}

/// Frames until the Device's pane shows what `ready` looks for.
fn until_video(
    shot: &mut Shot,
    app: &mut App,
    sid: &str,
    what: &str,
    ready: impl Fn(&vorn_tree::Video) -> bool,
) -> vorn_tree::Video {
    shot.until(app, what, |app, _| {
        app.pump();
        video(app, sid).filter(|v| ready(v))
    })
}

/// An agent session in a React Native project, which offers the phone.
fn mobile_agent(shot: &mut Shot, app: &mut App, v: &TestVornd) -> String {
    let project = add_project(app, v, "demo", json!({}));
    std::fs::write(
        project.join("package.json"),
        r#"{ "dependencies": { "react-native": "0.80.0" } }"#,
    )
    .expect("package.json");
    let sid = agent(shot, app);
    let path = project.to_string_lossy().into_owned();
    shot.until(app, "the project seen as mobile", |app, _| {
        app.st.bench.mobile.get(&path).copied()?.then_some(())
    });
    sid
}

#[test]
fn the_device_button_opens_the_picker_and_the_pane() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    let sid = mobile_agent(&mut shot, &mut app, &v);

    // The phone opens the pane with its picker listing the simulators.
    shot.hover(&mut app, term_id(&sid));
    let phone = shot
        .labelled(&mut app, "Button", "Open device")
        .expect("the card's phone");
    shot.click(&mut app, phone);
    let open = until_video(&mut shot, &mut app, &sid, "the picker", |v| {
        v.picker.as_ref().is_some_and(|p| !p.loading)
    });
    assert_eq!(app.st.bench.kinds(&sid), [PaneKind::Device]);
    assert_eq!((open.title.as_str(), open.device.as_deref()), ("Simulator", None));
    let picker = open.picker.expect("picker");
    assert_eq!(picker.heading, "Simulators");
    assert!(
        !picker.choices.is_empty() || picker.empty.as_deref() == Some("No simulators found"),
        "{picker:?}"
    );
    assert!(
        shot.labelled(&mut app, "ListBox", "Simulators").is_some(),
        "the picker is drawn"
    );
    assert_eq!(app.st.bench.video_picker.as_deref(), Some(sid.as_str()));
    shot.save(&mut app, "device-picker");

    // Escape with nothing picked shuts the picker and the pane with it.
    shot.send(&mut app, key("Escape", 0, None));
    shot.until(&mut app, "the picker shut", |app, _| {
        app.pump();
        app.st.bench.kinds(&sid).is_empty().then_some(())
    });
    assert!(app.st.bench.video_picker.is_none());
    shot.click(&mut app, card(PaneAct::DeviceButton(sid.clone())));
    until_video(&mut shot, &mut app, &sid, "the picker again", |v| {
        v.picker.as_ref().is_some_and(|p| !p.loading)
    });

    // A simulator that is not there: the app's start fails into the pane's notice.
    let key = device_key(&app.st.points, &sid).expect("the pane's tree");
    let mut pick = vorn_tree::Event::new("screen", vorn_tree::EventKind::Select, "TEST-UDID");
    pick.scope = Some(sid.clone());
    let params = app.st.points.tell(&key, &pick).expect("told");
    let pending = app.rpc().request("extension:call", params);
    shot.until(&mut app, "the pick", |app, _| {
        app.pump();
        pending.poll()
    })
    .expect("picked");
    let failed = until_video(&mut shot, &mut app, &sid, "the failed start", |v| {
        v.notice.is_some()
    });
    assert!(
        failed
            .notice
            .as_deref()
            .is_some_and(|n| n.starts_with("No simulator with udid TEST-UDID")),
        "{failed:?}"
    );
    assert!(!failed.boot && failed.device.is_none(), "{failed:?}");
    shot.save(&mut app, "device-pane");
    assert!(
        shot.labelled(&mut app, "Toolbar", "Device controls").is_some(),
        "the control bar"
    );
    let dismiss = shot
        .labelled(&mut app, "Button", "Dismiss error")
        .expect("the notice's ✕");
    shot.click(&mut app, dismiss);
    until_video(&mut shot, &mut app, &sid, "the notice gone", |v| {
        v.notice.is_none()
    });

    // The zoom is the pane's, kept by the Device.
    let zoom_in = shot
        .labelled(&mut app, "Button", "Zoom in")
        .expect("zoom in");
    shot.click(&mut app, zoom_in);
    until_video(&mut shot, &mut app, &sid, "zoomed", |v| v.zoom.is_some());
    let fit = shot
        .labelled(&mut app, "Button", "Fit the device to the pane")
        .expect("fit");
    shot.click(&mut app, fit);
    until_video(&mut shot, &mut app, &sid, "fitted", |v| v.zoom.is_none());

    // The pane's ✕ closes it.
    let close = shot
        .labelled(&mut app, "Button", "Close Simulator")
        .expect("the pane's close");
    shot.click(&mut app, close);
    shot.until(&mut app, "the pane closed", |app, _| {
        app.pump();
        app.st.bench.kinds(&sid).is_empty().then_some(())
    });
}

#[test]
fn switched_off_the_device_leaves_no_phone_and_refuses_its_calls() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    let sid = mobile_agent(&mut shot, &mut app, &v);
    shot.click(&mut app, card(PaneAct::DeviceButton(sid.clone())));
    until_video(&mut shot, &mut app, &sid, "the pane", |_| true);

    app.rpc()
        .call("extension:setBuiltin", json!({ "id": DEVICE, "on": false }))
        .expect("switched off");
    shot.until(&mut app, "the pane gone", |app, _| {
        app.pump();
        (app.st.points.loaded && app.st.bench.kinds(&sid).is_empty()).then_some(())
    });
    shot.hover(&mut app, term_id(&sid));
    assert!(
        shot.labelled(&mut app, "Button", "Open device").is_none(),
        "no phone while it is off"
    );
    let refused = app
        .rpc()
        .call("device:list", json!({ "sessionId": sid }))
        .expect_err("refused");
    let refused = format!("{refused:?}");
    assert!(
        refused.contains("Device is off. Turn it on in Settings \u{2192} Extensions."),
        "{refused}"
    );
}
