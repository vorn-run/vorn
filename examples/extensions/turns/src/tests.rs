use vorn_extension_sdk::testing::FakeHost;
use vorn_extension_sdk::{dispatch, Ctx};

use super::*;

fn host() -> FakeHost {
    FakeHost::default().answer("sessions.list", |_| {
        Ok(json!([{ "id": "t1", "name": "api" }, { "id": "t2", "name": "web" }]))
    })
}

fn event(name: &str, data: Value) -> Event {
    Event {
        name: name.into(),
        data,
    }
}

/// `t1` starts and finishes a turn; `t2` starts.
fn one_turn(t: &mut Turns, h: &mut FakeHost) {
    t.on_events(
        h,
        vec![
            event("session.created", json!({ "sessionId": "t1", "kind": "agent", "agent": "acme" })),
            event("session.created", json!({ "sessionId": "t2", "kind": "shell" })),
            event("session.status", json!({ "sessionId": "t1", "status": "idle" })),
        ],
    );
}

fn drawn(t: &mut Turns, h: &mut FakeHost, point: &str) -> tree::Node {
    let args = json!({ "point": point, "surface": {} });
    let answer = dispatch(t, h, tree::RENDER, args, &Ctx::default()).unwrap();
    tree::read(&answer).expect("a tree the host draws").tree
}

fn told(t: &mut Turns, h: &mut FakeHost, id: &str, kind: &str, value: &str) -> Value {
    let args = json!({ "point": "p", "event": { "id": id, "kind": kind, "value": value } });
    dispatch(t, h, tree::EVENT, args, &Ctx::default()).unwrap()
}

fn says(n: &tree::Node, what: &str) -> bool {
    serde_json_text(n).contains(what)
}

fn serde_json_text(n: &tree::Node) -> String {
    vorn_extension_sdk::serde_json::to_string(n).unwrap()
}

#[test]
fn a_finished_turn_counts_on_its_card_and_lists_in_the_sidebar() {
    let mut h = host();
    let mut t = Turns::init(&Init::default(), &mut h).unwrap();
    one_turn(&mut t, &mut h);
    let rows = h.rows(SESSIONS).unwrap();
    let api = rows.iter().find(|r| r["id"] == "t1").unwrap();
    assert_eq!((api["name"].clone(), api["turns"].clone()), (json!("api"), json!(1)));
    let badges = drawn(&mut t, &mut h, "count");
    assert_eq!(tree::for_session(&badges, "t1").map(|b| says(b, "1 done")), Some(true));
    assert!(tree::for_session(&badges, "t2").is_none(), "no turn yet");
    let footer = drawn(&mut t, &mut h, "last");
    assert!(tree::for_session(&footer, "t1").is_some_and(|f| says(f, "reset:t1")));
    assert!(says(&drawn(&mut t, &mut h, "finished"), "show:t1"));
    assert!(says(&drawn(&mut t, &mut h, "summary"), "Finished turns: 1"));
    assert!(says(&drawn(&mut t, &mut h, "all"), "At work: 1 of 2"));
    assert!(says(&drawn(&mut t, &mut h, "turns"), "At work: 1 of 2"));
}

#[test]
fn reset_clear_and_a_session_ending_change_what_it_draws() {
    let mut h = host();
    let mut t = Turns::init(&Init::default(), &mut h).unwrap();
    one_turn(&mut t, &mut h);
    assert_eq!(told(&mut t, &mut h, "reset:t1", "click", ""), Value::Null);
    assert!(tree::for_session(&drawn(&mut t, &mut h, "count"), "t1").is_none());
    assert_eq!(told(&mut t, &mut h, "ask-clear", "click", ""), json!({ "open": "clear" }));
    assert_eq!(told(&mut t, &mut h, "clear", "click", ""), json!({ "close": true }));
    assert!(h.rows(FINISHED).unwrap().is_empty());
    assert!(drawn(&mut t, &mut h, "summary").children.is_empty());
    t.on_events(
        &mut h,
        vec![event("session.exited", json!({ "sessionId": "t1" }))],
    );
    assert!(h.rows(SESSIONS).unwrap().iter().all(|r| r["id"] != "t1"));
    let menu = Turns::act(&mut h, "reset", &json!({ "sessionId": "t1" })).unwrap();
    assert_eq!(menu, json!({ "toast": "That session is gone" }));
}

#[test]
fn the_settings_page_switches_picks_signs_in_and_forgets() {
    let mut h = host();
    let mut t = Turns::init(&Init::default(), &mut h).unwrap();
    let page = drawn(&mut t, &mut h, "prefs");
    for shown in ["Count shells too", "Team plan", "Paste a key", "signin:turns-account", "Forget Turns account?"] {
        assert!(says(&page, shown), "{shown}");
    }
    told(&mut t, &mut h, "pref:shells", "toggle", "false");
    assert_eq!(h.rows(PREFS).unwrap()[0]["on"], json!(false));
    t.on_events(
        &mut h,
        vec![event("session.created", json!({ "sessionId": "t3", "kind": "shell" }))],
    );
    assert!(h.rows(SESSIONS).unwrap().is_empty(), "shells are not counted now");
    told(&mut t, &mut h, "plan", "select", "team");
    let team = h.rows(PLANS).unwrap().into_iter().find(|p| p["id"] == "team").unwrap();
    assert_eq!(team["selected"], json!(true));
    let signin = told(&mut t, &mut h, "signin:turns-account", "click", "");
    assert_eq!(signin["signin"]["id"], "turns-account");
    assert!(signin["signin"]["browser"]["signInUrl"].is_string());
    assert_eq!(
        told(&mut t, &mut h, "forget:turns-account", "click", ""),
        json!({ "toast": "Forgot Turns account" })
    );
}

#[test]
fn a_restarted_instance_keeps_what_it_kept() {
    let mut h = host();
    let mut t = Turns::init(&Init::default(), &mut h).unwrap();
    told(&mut t, &mut h, "plan", "select", "team");
    one_turn(&mut t, &mut h);
    let mut again = Turns::init(&Init::default(), &mut h).unwrap();
    let team = h.rows(PLANS).unwrap().into_iter().find(|p| p["id"] == "team").unwrap();
    assert_eq!(team["selected"], json!(true));
    assert!(says(&drawn(&mut again, &mut h, "count"), "1 done"));
}

#[test]
fn every_place_draws_a_tree_the_host_accepts() {
    let mut h = host();
    let mut t = Turns::init(&Init::default(), &mut h).unwrap();
    one_turn(&mut t, &mut h);
    for point in ["finished", "count", "last", "line", "all", "summary", "prefs", "turns", "clear"] {
        drawn(&mut t, &mut h, point);
    }
    let args = json!({ "point": "nope", "surface": {} });
    assert!(dispatch(&mut t, &mut h, tree::RENDER, args, &Ctx::default()).is_err());
}
