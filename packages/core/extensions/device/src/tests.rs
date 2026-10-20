use vorn_extension_sdk::testing::FakeHost;
use vorn_extension_sdk::tree::{self, Event, EventKind, Kind, Node};
use vorn_extension_sdk::{dispatch, json, Ctx, Value};

use super::*;

const S: &str = "s1";
const T: &str = "s2";

/// Vorn with sessions `s1` and `s2`, and a client with two simulators:
/// `u1` running, `u2` down. `boot` of anything else fails as `simctl` does.
fn host() -> FakeHost {
    FakeHost::default()
        .answer("sessions.get", |a| {
            Ok(match a["sessionId"].as_str() {
                Some(s @ (S | T)) => json!({ "id": s }),
                _ => Value::Null,
            })
        })
        .answer("video.call", |a| {
            let udid = a["params"]["udid"].as_str().unwrap_or_default();
            match a["op"].as_str().unwrap_or_default() {
                "list" => Ok(json!([
                    { "udid": "u1", "name": "iPhone 16", "runtime": "iOS 18.0", "booted": true },
                    { "udid": "u2", "name": "iPad", "runtime": "iOS 18.0", "booted": false },
                ])),
                "boot" => match udid {
                    "u1" => Ok(json!({ "name": "iPhone 16", "bootedHere": false })),
                    "u2" => Ok(json!({ "name": "iPad", "bootedHere": true })),
                    _ => Err(Error::new(
                        "failed",
                        format!("No simulator with udid {udid}. Call device_list to see what is available."),
                    )),
                },
                "launch" => Err(Error::new("failed", "Simulator: app not found")),
                "screenshot" => Ok(json!({ "data": "iVBO", "width": 2, "height": 3 })),
                "save" => Ok(json!({ "path": "/tmp/shot.png" })),
                _ => Ok(json!({})),
            }
        })
}

fn call(d: &mut DeviceExt, h: &mut FakeHost, method: &str, args: Value) -> Result<Value, Error> {
    dispatch(d, h, method, args, &Ctx::default())
}

fn agent(h: &mut FakeHost, session: &str, method: &str, mut args: Value) -> Result<Value, Error> {
    args["sessionId"] = json!(session);
    call(&mut DeviceExt, h, &format!("device:{method}"), args)
}

fn render(h: &mut FakeHost, session: &str) -> Node {
    let args = json!({ "point": PANE, "surface": { "scope": session, "width": 400 } });
    let v = call(&mut DeviceExt, h, tree::RENDER, args).expect("renders");
    tree::read(&v).expect("a tree").tree
}

/// The pane's video part, if the pane is open.
fn video(h: &mut FakeHost, session: &str) -> Option<tree::Video> {
    match render(h, session).kind {
        Kind::Video(v) => Some(*v),
        _ => None,
    }
}

fn tell(h: &mut FakeHost, kind: EventKind, value: &str) -> Value {
    let mut e = Event::new(SCREEN, kind, value);
    e.scope = Some(S.to_owned());
    call(
        &mut DeviceExt,
        h,
        tree::EVENT,
        json!({ "point": PANE, "event": e }),
    )
    .expect("told")
}

fn page(h: &mut FakeHost, said: Value) -> Value {
    tell(h, EventKind::Page, &said.to_string())
}

fn button(h: &mut FakeHost) {
    call(&mut DeviceExt, h, "button", json!({ "session": S })).unwrap();
}

fn open_rows(h: &mut FakeHost) -> Vec<String> {
    h.rows(OPEN)
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .collect()
}

fn shutdowns(h: &FakeHost) -> Vec<String> {
    h.asked("video.call")
        .iter()
        .filter(|a| a["op"] == "shutdown")
        .map(|a| a["params"]["udid"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn the_button_opens_the_picker_and_a_pick_boots_into_the_pane() {
    let mut h = host();
    assert!(video(&mut h, S).is_none(), "no pane before the button");
    button(&mut h);
    assert_eq!(open_rows(&mut h), [S]);
    let v = video(&mut h, S).expect("the pane");
    assert_eq!(
        (v.view.as_str(), v.device, v.title.as_str()),
        (S, None, "Simulator")
    );
    let picker = v.picker.expect("the picker is open");
    let ids: Vec<&str> = picker.choices.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["u1", "u2"], "running first");
    assert_eq!(picker.choices[0].badge.as_ref().unwrap().text, "booted");

    tell(&mut h, EventKind::Select, "u2");
    let v = video(&mut h, S).unwrap();
    assert_eq!(v.device.as_deref(), Some("u2"));
    assert_eq!(v.title, "iPad");
    assert!(v.boot && v.busy && v.picker.is_none(), "starting: {v:?}");

    page(&mut h, json!({ "type": "booted", "bootedHere": true }));
    let v = video(&mut h, S).unwrap();
    assert!(!v.boot && !v.busy);
    // Let go, Vorn shuts down what it started.
    assert_eq!(
        agent(&mut h, S, "release", json!({})).unwrap(),
        json!({ "released": true })
    );
    assert_eq!(shutdowns(&h), ["u2"]);
    assert!(video(&mut h, S).is_none() && open_rows(&mut h).is_empty());
}

#[test]
fn a_failed_start_brings_back_the_simulator_it_replaced() {
    let mut h = host();
    agent(&mut h, S, "claim", json!({ "udid": "u1" })).unwrap();
    button(&mut h);
    tell(&mut h, EventKind::Select, "u2");
    page(
        &mut h,
        json!({ "type": "failed", "message": "Unable to boot" }),
    );
    let v = video(&mut h, S).unwrap();
    assert_eq!(v.device.as_deref(), Some("u1"));
    assert_eq!(v.notice.as_deref(), Some("Unable to boot"));
    page(&mut h, json!({ "type": "dismiss" }));
    assert_eq!(video(&mut h, S).unwrap().notice, None);
    assert!(shutdowns(&h).is_empty(), "u1 was the person's");
}

#[test]
fn switching_shuts_down_only_what_vorn_started() {
    let mut h = host();
    agent(&mut h, S, "claim", json!({ "udid": "u2" })).unwrap();
    let said = agent(&mut h, S, "claim", json!({ "udid": "u1" })).unwrap();
    assert_eq!(
        said,
        json!({ "ok": true, "udid": "u1", "name": "iPhone 16", "booted": true })
    );
    assert_eq!(shutdowns(&h), ["u2"]);
}

#[test]
fn closing_the_picker_without_a_simulator_closes_the_pane() {
    let mut h = host();
    button(&mut h);
    page(&mut h, json!({ "type": "switch" }));
    assert!(video(&mut h, S).is_none());

    agent(&mut h, S, "claim", json!({ "udid": "u1" })).unwrap();
    button(&mut h);
    call(
        &mut DeviceExt,
        &mut h,
        "closePicker",
        json!({ "session": S }),
    )
    .unwrap();
    let v = video(&mut h, S).expect("the pane stays with its simulator");
    assert!(v.picker.is_none());
    // The button closes the pane; the claim stays.
    button(&mut h);
    assert!(video(&mut h, S).is_none());
    assert_eq!(
        agent(&mut h, S, "openPane", json!({})).unwrap(),
        json!({ "udid": "u1" })
    );
    assert!(video(&mut h, S).unwrap().picker.is_none());
}

#[test]
fn a_simulator_held_by_another_session_is_refused() {
    let mut h = host();
    agent(&mut h, T, "claim", json!({ "udid": "u1" })).unwrap();
    let said = agent(&mut h, S, "claim", json!({ "udid": "u1" })).unwrap();
    assert_eq!(said["ok"], false);
    assert_eq!(said["reason"], "held-by-session");
    assert_eq!(said["holder"], T);
    let e = agent(&mut h, S, "openPane", json!({ "udid": "u1" })).unwrap_err();
    assert_eq!(e.message, "u1 is in use by session s2.");
    let listed = agent(&mut h, S, "list", json!({})).unwrap();
    assert_eq!(listed[0]["claimedBy"], T);
    assert!(listed[1].get("claimedBy").is_none());

    button(&mut h);
    let picker = video(&mut h, S).unwrap().picker.unwrap();
    let held = picker.choices.iter().find(|c| c.id == "u1").unwrap();
    assert!(held.disabled);
    assert_eq!(held.badge.as_ref().unwrap().text, "in use");
}

#[test]
fn agent_calls_keep_their_answers() {
    let mut h = host();
    let e = agent(&mut h, S, "screenshot", json!({})).unwrap_err();
    assert_eq!(e.message, NO_CLAIM);
    let e = agent(&mut h, S, "openPane", json!({})).unwrap_err();
    assert!(e.message.starts_with("No device is claimed"), "{e:?}");
    let gone = agent(&mut h, S, "claim", json!({ "udid": "nope" })).unwrap();
    assert_eq!(gone["reason"], "gone");

    agent(&mut h, S, "claim", json!({ "udid": "u1" })).unwrap();
    let shot = agent(&mut h, S, "screenshot", json!({})).unwrap();
    assert_eq!(shot["screen"], json!({ "width": 2, "height": 3 }));
    assert_eq!(shot["orientation"], "portrait");
    let e = agent(&mut h, S, "launch", json!({ "bundleId": "com.x" })).unwrap_err();
    assert_eq!(
        e.message,
        "com.x is not installed on this device. Install it first."
    );
    assert_eq!(
        agent(&mut h, S, "openUrl", json!({ "url": "x://y" })).unwrap(),
        json!({ "ok": true })
    );
    let asked = h.asked("video.call");
    let open = asked.iter().find(|a| a["op"] == "openUrl").unwrap();
    assert_eq!(
        (open["view"].as_str(), open["params"]["udid"].as_str()),
        (Some(S), Some("u1"))
    );
    for m in ["readScreen", "find", "interact", "logs"] {
        assert_eq!(
            agent(&mut h, S, m, json!({})).unwrap_err().message,
            NO_COMPANION
        );
    }
    assert_eq!(
        agent(&mut h, S, "release", json!({})).unwrap(),
        json!({ "released": true })
    );
    assert_eq!(
        agent(&mut h, S, "release", json!({})).unwrap(),
        json!({ "released": false })
    );
    let e = agent(&mut h, "s9", "list", json!({})).unwrap_err();
    assert_eq!(e.message, "No session s9 in this app.");
    // The chrome is asked without a session.
    assert_eq!(
        call(&mut DeviceExt, &mut h, "device:chrome", json!({})).unwrap(),
        Value::Null
    );
}

#[test]
fn the_pane_keeps_its_zoom_and_says_what_needs_the_companion() {
    let mut h = host();
    agent(&mut h, S, "openPane", json!({ "udid": "u1" })).unwrap();
    page(&mut h, json!({ "type": "zoom", "zoom": 1.0 }));
    assert_eq!(video(&mut h, S).unwrap().zoom, Some(1.0));
    assert_eq!(
        page(&mut h, json!({ "type": "zoom", "zoom": 1.0 }))["same"],
        true
    );
    page(&mut h, json!({ "type": "zoom" }));
    assert_eq!(video(&mut h, S).unwrap().zoom, None, "fit");

    tell(&mut h, EventKind::Click, "home");
    assert_eq!(
        video(&mut h, S).unwrap().notice.as_deref(),
        Some(NO_COMPANION)
    );
    let saved = tell(&mut h, EventKind::Click, "save");
    assert_eq!(saved["toast"], "Screenshot saved");
    let save = h
        .asked("video.call")
        .into_iter()
        .find(|a| a["op"] == "save")
        .unwrap();
    assert_eq!(save["params"], json!({ "udid": "u1", "name": "iPhone 16" }));
}

#[test]
fn pruning_lets_the_simulators_of_gone_sessions_go() {
    let mut h = host();
    agent(&mut h, S, "openPane", json!({ "udid": "u2" })).unwrap();
    agent(&mut h, T, "claim", json!({ "udid": "u1" })).unwrap();
    call(&mut DeviceExt, &mut h, "prune", json!({ "live": [T] })).unwrap();
    assert!(open_rows(&mut h).is_empty());
    assert_eq!(shutdowns(&h), ["u2"]);
    let listed = agent(&mut h, T, "list", json!({})).unwrap();
    assert_eq!(listed[0]["claimedBy"], T, "the live session keeps its own");
}

#[test]
fn the_panes_close_keeps_the_claim() {
    let mut h = host();
    button(&mut h);
    tell(&mut h, EventKind::Select, "u1");
    page(&mut h, json!({ "type": "booted", "bootedHere": false }));
    call(&mut DeviceExt, &mut h, "close", json!({ "session": S })).unwrap();
    assert!(open_rows(&mut h).is_empty(), "the pane went");
    assert!(video(&mut h, S).is_none());
    let claim = call(
        &mut DeviceExt,
        &mut h,
        "device:list",
        json!({ "sessionId": S }),
    )
    .unwrap();
    let held: Vec<&Value> = claim
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["claimedBy"] == S)
        .collect();
    assert_eq!(held.len(), 1, "still held: {claim}");
    assert!(shutdowns(&h).is_empty(), "nothing shut down");
}
