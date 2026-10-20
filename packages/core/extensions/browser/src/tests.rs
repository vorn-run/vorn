use vorn_extension_sdk::testing::FakeHost;
use vorn_extension_sdk::tree::{self, Event, EventKind, Kind, Node};
use vorn_extension_sdk::{dispatch, json, Ctx, Value};

use super::*;

const S: &str = "s1";

/// Vorn with one session in `/work/proj`, whose client answers every page call with what it was asked.
fn host() -> FakeHost {
    FakeHost::default()
        .answer("sessions.get", |a| {
            Ok(match a["sessionId"].as_str() {
                Some(S) => json!({ "id": S, "projectPath": "/work/proj" }),
                _ => Value::Null,
            })
        })
        .answer("web.call", |a| Ok(json!({ "asked": a })))
}

fn call(b: &mut Browser, h: &mut FakeHost, method: &str, args: Value) -> Result<Value, Error> {
    dispatch(b, h, method, args, &Ctx::default())
}

fn agent(b: &mut Browser, h: &mut FakeHost, method: &str, mut args: Value) -> Result<Value, Error> {
    args["sessionId"] = json!(S);
    call(b, h, &format!("browser:{method}"), args)
}

fn render(b: &mut Browser, h: &mut FakeHost) -> Node {
    let args = json!({ "point": PANE, "surface": { "scope": S, "width": 600 } });
    let v = call(b, h, tree::RENDER, args).expect("renders");
    tree::read(&v).expect("a tree").tree
}

fn tell(b: &mut Browser, h: &mut FakeHost, id: &str, kind: EventKind, value: &str) -> Value {
    let mut e = Event::new(id, kind, value);
    e.scope = Some(S.to_owned());
    call(b, h, tree::EVENT, json!({ "point": PANE, "event": e })).expect("told")
}

fn page(b: &mut Browser, h: &mut FakeHost, said: Value) -> Value {
    tell(b, h, "page", EventKind::Page, &said.to_string())
}

/// The strip's tabs, `(label, active)`, and the page's address.
fn shown(n: &Node) -> (Vec<(String, bool)>, Option<String>) {
    let mut tabs = Vec::new();
    let mut url = None;
    let mut stack = vec![n];
    while let Some(n) = stack.pop() {
        match &n.kind {
            Kind::Tabs(t) => tabs = t.tabs.iter().map(|t| (t.label.clone(), t.active)).collect(),
            Kind::Web(w) => {
                assert_eq!(w.view, S, "the view is the session's");
                url = Some(w.url.clone());
            }
            _ => {}
        }
        stack.extend(&n.children);
    }
    (tabs, url)
}

fn open_rows(h: &mut FakeHost) -> Vec<String> {
    h.rows(OPEN)
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn the_globe_opens_a_blank_tab_and_closing_keeps_the_tabs() {
    let (mut b, mut h) = (Browser, host());
    assert_eq!(shown(&render(&mut b, &mut h)), (vec![], None));
    call(&mut b, &mut h, "toggle", json!({ "session": S })).unwrap();
    assert_eq!(open_rows(&mut h), [S]);
    assert_eq!(
        shown(&render(&mut b, &mut h)),
        (vec![("New tab".into(), true)], Some("about:blank".into()))
    );
    page(
        &mut b,
        &mut h,
        json!({ "type": "submit", "text": "example.com" }),
    );
    call(&mut b, &mut h, "toggle", json!({ "session": S })).unwrap();
    assert!(open_rows(&mut h).is_empty());
    assert_eq!(shown(&render(&mut b, &mut h)).1, None);
    call(&mut b, &mut h, "toggle", json!({ "session": S })).unwrap();
    assert_eq!(
        shown(&render(&mut b, &mut h)).1.as_deref(),
        Some("https://example.com/"),
        "it opens as it was left"
    );
}

#[test]
fn tabs_are_added_selected_and_closed_from_the_strip() {
    let (mut b, mut h) = (Browser, host());
    agent(&mut b, &mut h, "openPane", json!({ "url": "a.test" })).unwrap();
    tell(&mut b, &mut h, "new", EventKind::Click, "");
    assert_eq!(
        shown(&render(&mut b, &mut h)).0,
        [("a.test".into(), false), ("New tab".into(), true)]
    );
    tell(&mut b, &mut h, "tabs", EventKind::Select, "0");
    assert_eq!(
        shown(&render(&mut b, &mut h)).1.as_deref(),
        Some("https://a.test/")
    );
    assert_eq!(
        tell(&mut b, &mut h, "tabs", EventKind::Select, "0"),
        json!({ "same": true })
    );
    tell(&mut b, &mut h, "tabs", EventKind::Close, "0");
    tell(&mut b, &mut h, "tabs", EventKind::Close, "0");
    assert!(open_rows(&mut h).is_empty(), "the last tab takes the pane");
    call(&mut b, &mut h, "toggle", json!({ "session": S })).unwrap();
    assert_eq!(
        shown(&render(&mut b, &mut h)).1.as_deref(),
        Some("about:blank"),
        "and leaves nothing to reopen"
    );
}

#[test]
fn what_the_page_reports_names_its_tab_and_reaches_the_agent() {
    let (mut b, mut h) = (Browser, host());
    agent(
        &mut b,
        &mut h,
        "openPane",
        json!({ "url": "http://localhost:3000" }),
    )
    .unwrap();
    page(
        &mut b,
        &mut h,
        json!({ "type": "state", "url": "http://localhost:3000/x", "title": "App" }),
    );
    assert_eq!(
        agent(&mut b, &mut h, "listTabs", json!({})).unwrap(),
        json!({ "tabs": [{ "index": 0, "url": "http://localhost:3000/x", "active": true, "title": "App" }] })
    );
    assert_eq!(
        shown(&render(&mut b, &mut h)).1.as_deref(),
        Some("http://localhost:3000/"),
        "where the page wandered is not loaded again"
    );
    assert_eq!(
        page(
            &mut b,
            &mut h,
            json!({ "type": "state", "url": "http://localhost:3000/x", "title": "App" })
        ),
        json!({ "same": true })
    );
    page(
        &mut b,
        &mut h,
        json!({ "type": "state", "design": "Landing" }),
    );
    assert_eq!(shown(&render(&mut b, &mut h)).0[0].0, "Landing");
}

#[test]
fn a_page_opening_a_window_gets_a_tab_only_for_an_address_it_may_load() {
    let (mut b, mut h) = (Browser, host());
    agent(&mut b, &mut h, "openPane", json!({ "url": "a.test" })).unwrap();
    page(
        &mut b,
        &mut h,
        json!({ "type": "opened", "url": "javascript:alert(1)" }),
    );
    page(
        &mut b,
        &mut h,
        json!({ "type": "opened", "url": "file:///etc/passwd" }),
    );
    assert_eq!(shown(&render(&mut b, &mut h)).0.len(), 1);
    page(
        &mut b,
        &mut h,
        json!({ "type": "opened", "url": "https://b.test/" }),
    );
    assert_eq!(
        shown(&render(&mut b, &mut h)),
        (
            vec![("a.test".into(), false), ("b.test".into(), true)],
            Some("https://b.test/".into())
        )
    );
}

#[test]
fn the_agent_opens_navigates_and_is_refused_what_it_may_not_load() {
    let (mut b, mut h) = (Browser, host());
    assert_eq!(
        agent(&mut b, &mut h, "navigate", json!({ "url": "a.test" }))
            .unwrap_err()
            .message,
        NO_PANE
    );
    assert_eq!(
        agent(&mut b, &mut h, "openPane", json!({ "url": "javascript:1" }))
            .unwrap_err()
            .message,
        "Refusing to open \"javascript:1\" \u{2014} not an allowed web address."
    );
    assert_eq!(
        agent(
            &mut b,
            &mut h,
            "openPane",
            json!({ "url": "file:///work/proj/index.html" })
        )
        .unwrap(),
        json!({ "url": "file:///work/proj/index.html" })
    );
    assert_eq!(
        agent(
            &mut b,
            &mut h,
            "navigate",
            json!({ "url": "file:///work/other.html" })
        )
        .unwrap_err()
        .message,
        "Refusing to navigate to \"file:///work/other.html\" \u{2014} not an allowed web address."
    );
    agent(&mut b, &mut h, "navigate", json!({ "url": "b.test" })).unwrap();
    assert_eq!(
        h.asked("web.call").last(),
        Some(&json!({ "view": S, "op": "load", "params": { "url": "https://b.test/" } })),
        "a navigation loads even an address the tab was asked for before"
    );
    let other = call(
        &mut b,
        &mut h,
        "browser:listTabs",
        json!({ "sessionId": "gone" }),
    );
    assert_eq!(other.unwrap_err().message, "No session gone in this app.");
}

#[test]
fn tab_calls_need_an_index_and_vet_what_they_add() {
    let (mut b, mut h) = (Browser, host());
    agent(&mut b, &mut h, "openPane", json!({})).unwrap();
    assert_eq!(
        agent(&mut b, &mut h, "tabs", json!({ "action": "select" }))
            .unwrap_err()
            .message,
        "A tab index is required to select a tab."
    );
    assert!(agent(
        &mut b,
        &mut h,
        "tabs",
        json!({ "action": "add", "url": "data:x" })
    )
    .is_err());
    agent(
        &mut b,
        &mut h,
        "tabs",
        json!({ "action": "add", "url": "c.test" }),
    )
    .unwrap();
    agent(
        &mut b,
        &mut h,
        "tabs",
        json!({ "action": "select", "index": 0 }),
    )
    .unwrap();
    let tabs = agent(&mut b, &mut h, "listTabs", json!({})).unwrap();
    assert_eq!(tabs["tabs"][1]["url"], "https://c.test/");
    assert_eq!(tabs["tabs"][0]["active"], true);
}

#[test]
fn page_calls_go_to_the_client_showing_the_page() {
    let (mut b, mut h) = (Browser, host());
    assert_eq!(
        agent(&mut b, &mut h, "readPage", json!({}))
            .unwrap_err()
            .message,
        NO_PANE
    );
    assert!(h.asked("web.call").is_empty(), "no pane, no page to ask");
    agent(&mut b, &mut h, "openPane", json!({ "url": "a.test" })).unwrap();
    for (method, op) in [
        ("readPage", "read"),
        ("getText", "text"),
        ("find", "find"),
        ("interact", "act"),
        ("consoleMessages", "console"),
        ("networkRequests", "network"),
        ("history", "history"),
    ] {
        let answer = agent(&mut b, &mut h, method, json!({ "limit": 3 })).unwrap();
        assert_eq!(answer["asked"]["view"], S, "{method}");
        assert_eq!(answer["asked"]["op"], op, "{method}");
        assert_eq!(answer["asked"]["params"]["limit"], 3, "{method}");
    }
    assert!(agent(&mut b, &mut h, "screenshot", json!({}))
        .unwrap_err()
        .message
        .starts_with("Page screenshots are not available"));
    assert_eq!(
        agent(&mut b, &mut h, "dance", json!({})).unwrap_err().code,
        "unknown-action"
    );
}

#[test]
fn an_artifact_gets_its_own_tab_and_keeps_it_across_versions() {
    let (mut b, mut h) = (Browser, host());
    agent(&mut b, &mut h, "openPane", json!({ "url": "a.test" })).unwrap();
    let art = json!({ "id": "art1", "kind": "design", "version": 1, "title": "Landing" });
    let url = "http://127.0.0.1:4000/artifact/art1/1?t=k";
    agent(
        &mut b,
        &mut h,
        "openPane",
        json!({ "url": url, "artifact": art }),
    )
    .unwrap();
    let tree = render(&mut b, &mut h);
    assert_eq!(
        shown(&tree),
        (
            vec![("a.test".into(), false), ("Landing".into(), true)],
            Some(url.into())
        )
    );
    let v2 = "http://127.0.0.1:4000/artifact/art1/2?t=k";
    page(
        &mut b,
        &mut h,
        json!({ "type": "go", "url": v2, "version": 2 }),
    );
    let pane: Pane = serde_json::from_value(h.kv[&format!("pane/{S}")].clone()).unwrap();
    let tab = pane.active().unwrap();
    assert_eq!(
        (tab.url.as_str(), tab.artifact.as_ref().map(|a| a.version)),
        (v2, Some(2))
    );
    let mut art = art;
    art["version"] = json!(3);
    agent(
        &mut b,
        &mut h,
        "openPane",
        json!({ "url": v2.replace("/2?", "/3?"), "artifact": art }),
    )
    .unwrap();
    assert_eq!(
        shown(&render(&mut b, &mut h)).0.len(),
        2,
        "replaced in place"
    );
}

#[test]
fn panes_of_sessions_gone_are_pruned() {
    let (mut b, mut h) = (Browser, host());
    call(&mut b, &mut h, "toggle", json!({ "session": S })).unwrap();
    call(&mut b, &mut h, "toggle", json!({ "session": "s2" })).unwrap();
    call(&mut b, &mut h, "prune", json!({ "live": [S] })).unwrap();
    assert_eq!(open_rows(&mut h), [S]);
    assert!(!h.kv.contains_key("pane/s2"));
}

#[test]
fn the_desktops_panes_come_over_once_with_intent_only() {
    let (mut b, mut h) = (Browser, host());
    call(&mut b, &mut h, "toggle", json!({ "session": "kept" })).unwrap();
    let saved = json!({
        "open": {
            S: { "tabs": [{ "url": "https://a.test/", "liveUrl": "https://a.test/x", "title": "A" }], "activeTab": 3 },
            "kept": { "tabs": [{ "url": "https://lost.test/" }], "activeTab": 0 },
            "empty": { "tabs": [], "activeTab": 0 },
        },
        "memory": { "s3": { "tabs": [{ "url": "https://m.test/" }], "activeTab": 0 } },
        "tweaks": { "/p/d.html": { "x": 1 } },
    });
    assert_eq!(
        call(&mut b, &mut h, "import", saved).unwrap(),
        json!({ "imported": 2 })
    );
    let pane: Pane = serde_json::from_value(h.kv[&format!("pane/{S}")].clone()).unwrap();
    assert_eq!(pane.tabs, [Tab::at("https://a.test/")]);
    assert_eq!(pane.active_tab, 0);
    let kept: Pane = serde_json::from_value(h.kv["pane/kept"].clone()).unwrap();
    assert_eq!(
        kept.tabs,
        [Tab::at(address::DEFAULT_URL)],
        "a pane here stays"
    );
    assert!(!h.kv.contains_key("pane/empty"));
    assert!(h.kv.contains_key("memory/s3"));
    let mut open = open_rows(&mut h);
    open.sort();
    assert_eq!(open, ["kept", S]);
}

#[test]
fn the_address_bar_loads_again_and_refuses_what_is_no_address() {
    let (mut b, mut h) = (Browser, host());
    call(&mut b, &mut h, "toggle", json!({ "session": S })).unwrap();
    page(
        &mut b,
        &mut h,
        json!({ "type": "submit", "text": "example.com" }),
    );
    page(
        &mut b,
        &mut h,
        json!({ "type": "submit", "text": "example.com" }),
    );
    let loads = h
        .calls
        .iter()
        .filter(|(m, a)| m == "web.call" && a["op"] == "load")
        .count();
    assert_eq!(loads, 2, "Enter on the page shown loads it again");
    let said = json!({ "type": "submit", "text": "javascript:alert(1)" }).to_string();
    let mut e = Event::new("page", EventKind::Page, said);
    e.scope = Some(S.to_owned());
    let refused = call(
        &mut b,
        &mut h,
        tree::EVENT,
        json!({ "point": PANE, "event": e }),
    );
    assert_eq!(refused.unwrap_err().message, NOT_AN_ADDRESS);
}
