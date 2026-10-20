use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use vorn_extension_sdk::testing::FakeHost;
use vorn_extension_sdk::tree::{
    self, Code, CodeState, Event, EventKind, Kind, LineKind, Node, Tabs,
};
use vorn_extension_sdk::{dispatch, json, Ctx, Extension, Init, Value};

use super::*;

/// A folder on a fake disk: path to text, `None` a folder.
type Disk = Rc<RefCell<BTreeMap<String, Option<String>>>>;

fn host(disk: &Disk) -> FakeHost {
    let (l, r, s, w) = (disk.clone(), disk.clone(), disk.clone(), disk.clone());
    FakeHost::default()
        .answer("sessions.get", |_| Ok(json!({ "projectPath": "/p" })))
        .answer("files.list", move |a| {
            let dir = a["path"].as_str().unwrap_or_default().to_owned();
            let d = l.borrow();
            let mut out: Vec<Value> = d
                .iter()
                .filter(|(p, _)| p.rsplit_once('/').map(|(d, _)| d) == Some(dir.as_str()))
                .map(|(p, t)| json!({ "name": view::base(p), "path": p, "isDirectory": t.is_none() }))
                .collect();
            out.sort_by_key(|e| (!e["isDirectory"].as_bool().unwrap_or(false), e["name"].to_string()));
            Ok(Value::Array(out))
        })
        .answer("files.read", move |a| {
            Ok(json!(r.borrow().get(a["path"].as_str().unwrap_or_default()).cloned().flatten()))
        })
        .answer("files.stat", move |a| {
            let d = s.borrow();
            Ok(match d.get(a["path"].as_str().unwrap_or_default()) {
                Some(Some(t)) => json!({ "size": t.len(), "mtimeMs": 1 }),
                _ => Value::Null,
            })
        })
        .answer("files.write", move |a| {
            let p = a["path"].as_str().unwrap_or_default().to_owned();
            w.borrow_mut().insert(p, a["content"].as_str().map(str::to_owned));
            Ok(json!({ "success": true }))
        })
}

fn disk() -> Disk {
    let mut d = BTreeMap::new();
    d.insert("/p/src".to_owned(), None);
    d.insert(
        "/p/src/app.ts".to_owned(),
        Some("const names = 1\n".to_owned()),
    );
    d.insert("/p/README.md".to_owned(), Some("# Acme\n".to_owned()));
    Rc::new(RefCell::new(d))
}

fn render(f: &mut Files, h: &mut FakeHost, point: &str, scope: &str, width: f32) -> Node {
    let args = json!({ "point": point, "surface": { "scope": scope, "width": width } });
    let v = dispatch(f, h, tree::RENDER, args, &Ctx::default()).unwrap();
    tree::read(&v).unwrap().tree
}

fn tell(
    f: &mut Files,
    h: &mut FakeHost,
    point: &str,
    scope: &str,
    id: &str,
    kind: EventKind,
    value: &str,
) -> tree::Answer {
    let mut e = Event::new(id, kind, value);
    e.scope = Some(scope.to_owned());
    e.width = Some(900);
    let v = dispatch(
        f,
        h,
        tree::EVENT,
        json!({ "point": point, "event": e }),
        &Ctx::default(),
    )
    .unwrap();
    tree::Answer::read(&v)
}

fn find<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
    find_by(n, &|i| i == id)
}

fn find_by<'a>(n: &'a Node, hit: &dyn Fn(&str) -> bool) -> Option<&'a Node> {
    if n.id.as_deref().is_some_and(hit) {
        return Some(n);
    }
    n.children.iter().find_map(|c| find_by(c, hit))
}

fn tabs_of(n: &Node) -> Tabs {
    match &find(n, "tabs").expect("tabs").kind {
        Kind::Tabs(t) => (**t).clone(),
        _ => panic!("tabs"),
    }
}

fn code_of(n: &Node) -> Option<Code> {
    match &find_by(n, &|i| i.starts_with("code:"))?.kind {
        Kind::Code(c) => Some((**c).clone()),
        _ => None,
    }
}

#[test]
fn the_pane_browses_previews_edits_and_saves() {
    let d = disk();
    let mut h = host(&d);
    let mut f = Files::init(&Init::default(), &mut h).unwrap();
    let ctx = Ctx::default();
    dispatch(&mut f, &mut h, "toggle", json!({ "session": "s" }), &ctx).unwrap();
    assert_eq!(h.rows(OPEN).unwrap()[0]["id"], "s");

    let n = render(&mut f, &mut h, PANE, "s", 900.0);
    assert_eq!(tabs_of(&n).empty.as_deref(), Some("Files"));
    let Kind::Outline(o) = &find(&n, "tree").unwrap().kind else {
        panic!("outline")
    };
    let labels: Vec<_> = o.rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["src", "README.md"]);

    tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "tree",
        EventKind::Select,
        "/p/src",
    );
    let n = render(&mut f, &mut h, PANE, "s", 900.0);
    let Kind::Outline(o) = &find(&n, "tree").unwrap().kind else {
        panic!("outline")
    };
    assert_eq!(
        o.rows[0].kind,
        LineKind::Folder {
            open: true,
            busy: false
        }
    );
    assert_eq!(o.rows[1].label, "app.ts");

    tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "tree",
        EventKind::Select,
        "/p/src/app.ts",
    );
    let n = render(&mut f, &mut h, PANE, "s", 900.0);
    let t = tabs_of(&n);
    assert!(t.tabs[0].preview && t.tabs[0].active && !t.tabs[0].dirty);
    let c = code_of(&n).expect("code");
    assert_eq!(
        (c.state, c.text.as_str(), c.root.as_deref()),
        (CodeState::Edit, "const names = 1\n", Some("/p"))
    );

    // An edit is drafted and pins the preview; the same text again changes nothing drawn.
    let a = tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "code:/p/src/app.ts",
        EventKind::Change,
        "edited\n",
    );
    assert!(!a.same);
    let a = tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "code:/p/src/app.ts",
        EventKind::Change,
        "edited more\n",
    );
    assert!(a.same);
    let t = tabs_of(&render(&mut f, &mut h, PANE, "s", 900.0));
    assert!(t.tabs[0].dirty && !t.tabs[0].preview);

    tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "code:/p/src/app.ts",
        EventKind::Submit,
        "saved\n",
    );
    assert_eq!(d.borrow()["/p/src/app.ts"].as_deref(), Some("saved\n"));
    let n = render(&mut f, &mut h, PANE, "s", 900.0);
    assert!(!tabs_of(&n).tabs[0].dirty);
    assert_eq!(code_of(&n).unwrap().draft, None);
}

#[test]
fn edits_ask_before_they_go_and_outlive_the_instance() {
    let d = disk();
    let mut h = host(&d);
    let mut f = Files::init(&Init::default(), &mut h).unwrap();
    dispatch(
        &mut f,
        &mut h,
        "toggle",
        json!({ "session": "s" }),
        &Ctx::default(),
    )
    .unwrap();
    tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "tree",
        EventKind::Open,
        "/p/README.md",
    );
    render(&mut f, &mut h, PANE, "s", 900.0);
    tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "code:/p/README.md",
        EventKind::Change,
        "# Mine\n",
    );
    let a = tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "tabs",
        EventKind::Close,
        "/p/README.md",
    );
    let ask = a.ask.expect("asks first");
    assert_eq!(ask.title, DISCARD);
    assert_eq!(
        tabs_of(&render(&mut f, &mut h, PANE, "s", 900.0))
            .tabs
            .len(),
        1
    );

    // A new instance draws the draft over the file, with the same rev.
    let rev = code_of(&render(&mut f, &mut h, PANE, "s", 900.0))
        .unwrap()
        .rev;
    let mut again = Files::init(&Init::default(), &mut h).unwrap();
    let c = code_of(&render(&mut again, &mut h, PANE, "s", 900.0)).unwrap();
    assert_eq!((c.draft.as_deref(), c.rev), (Some("# Mine\n"), rev));

    // Discard asks too; OK takes the file back and the client takes it afresh.
    let a = tell(
        &mut again,
        &mut h,
        PANE,
        "s",
        "code:/p/README.md",
        EventKind::Click,
        "discard",
    );
    let ok = a.ask.expect("asks").event;
    tell(&mut again, &mut h, PANE, "s", &ok.id, ok.kind, &ok.value);
    let c = code_of(&render(&mut again, &mut h, PANE, "s", 900.0)).unwrap();
    assert_eq!(c.draft, None);
    assert!(c.rev > rev);
    let a = tell(
        &mut again,
        &mut h,
        PANE,
        "s",
        "tabs",
        EventKind::Close,
        "/p/README.md",
    );
    assert!(a.ask.is_none());
    assert!(tabs_of(&render(&mut again, &mut h, PANE, "s", 900.0))
        .tabs
        .is_empty());
}

#[test]
fn a_file_pops_out_into_a_card_and_back() {
    let d = disk();
    let mut h = host(&d);
    let mut f = Files::init(&Init::default(), &mut h).unwrap();
    dispatch(
        &mut f,
        &mut h,
        "toggle",
        json!({ "session": "s" }),
        &Ctx::default(),
    )
    .unwrap();
    render(&mut f, &mut h, PANE, "s", 900.0);
    let a = tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "pop",
        EventKind::Click,
        "/p/README.md",
    );
    let cards = h.rows(CARDS).unwrap();
    assert_eq!(
        a.focus.as_ref(),
        cards[0]["id"].as_str().map(str::to_owned).as_ref()
    );
    assert_eq!(cards.len(), 1);
    assert_eq!(
        (cards[0]["session"].clone(), cards[0]["title"].clone()),
        (json!("s"), json!("README.md"))
    );
    let id = cards[0]["id"].as_str().unwrap().to_owned();
    let card = render(&mut f, &mut h, CARD, &id, 600.0);
    let Kind::Code(c) = &card.kind else {
        panic!("code")
    };
    assert_eq!(c.text, "# Acme\n");
    let labels: Vec<_> = card
        .children
        .iter()
        .filter_map(|b| match &b.kind {
            Kind::Button { label, .. } => Some(label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["Put back in its session card", "Close"]);
    let a = tell(&mut f, &mut h, CARD, &id, "return", EventKind::Click, "");
    assert_eq!(a.focus.as_deref(), Some("s"));
    assert!(h.rows(CARDS).unwrap().is_empty());
    let t = tabs_of(&render(&mut f, &mut h, PANE, "s", 900.0));
    assert_eq!(t.tabs[0].label, "README.md");
}

#[test]
fn a_narrow_pane_hides_the_tree_to_show_the_file() {
    let d = disk();
    let mut h = host(&d);
    let mut f = Files::init(&Init::default(), &mut h).unwrap();
    dispatch(
        &mut f,
        &mut h,
        "toggle",
        json!({ "session": "s" }),
        &Ctx::default(),
    )
    .unwrap();
    render(&mut f, &mut h, PANE, "s", 300.0);
    let mut e = Event::new("tree", EventKind::Select, "/p/README.md");
    e.scope = Some("s".into());
    e.width = Some(300);
    dispatch(
        &mut f,
        &mut h,
        tree::EVENT,
        json!({ "point": PANE, "event": e }),
        &Ctx::default(),
    )
    .unwrap();
    let n = render(&mut f, &mut h, PANE, "s", 300.0);
    assert!(find(&n, "tree").is_none());
    assert!(code_of(&n).is_some());
    let t = tabs_of(&n);
    assert_eq!(
        (t.lead[0].label.as_str(), t.lead[0].on),
        ("Show file tree", false)
    );
}

#[test]
fn the_desktops_own_panes_and_drafts_come_in_once() {
    let d = disk();
    let mut h = host(&d);
    let mut f = Files::init(&Init::default(), &mut h).unwrap();
    let view = json!({
        "files": { "s": { "tabs": ["/p/README.md"], "active": "/p/README.md", "treeVisible": true } },
        "cards": { "card:s:1": { "sessionId": "s", "filePath": "/p/src/app.ts" } },
    });
    let drafts = json!({
        "files:s:/p/README.md": { "filePath": "/p/README.md", "text": "# Kept\n", "base": null },
    });
    let n = dispatch(
        &mut f,
        &mut h,
        "import",
        json!({ "view": view, "drafts": drafts }),
        &Ctx::default(),
    )
    .unwrap();
    assert_eq!(
        (n["imported"].clone(), n["skipped"].clone()),
        (json!(3), json!(0))
    );
    let pane = render(&mut f, &mut h, PANE, "s", 900.0);
    assert_eq!(code_of(&pane).unwrap().draft.as_deref(), Some("# Kept\n"));
    assert_eq!(h.rows(CARDS).unwrap().len(), 1);
    dispatch(
        &mut f,
        &mut h,
        "prune",
        json!({ "live": [] }),
        &Ctx::default(),
    )
    .unwrap();
    assert!(h.rows(OPEN).unwrap().is_empty() && h.rows(CARDS).unwrap().is_empty());
}

#[test]
fn an_edit_told_late_reaches_its_own_file_not_the_tab_in_front() {
    let d = disk();
    let mut h = host(&d);
    let mut f = Files::init(&Init::default(), &mut h).unwrap();
    let ctx = Ctx::default();
    dispatch(&mut f, &mut h, "toggle", json!({ "session": "s" }), &ctx).unwrap();
    for p in ["/p/README.md", "/p/src/app.ts"] {
        tell(&mut f, &mut h, PANE, "s", "tree", EventKind::Open, p);
    }
    render(&mut f, &mut h, PANE, "s", 900.0);
    // app.ts is in front; README.md's typing arrives after the switch.
    tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "code:/p/README.md",
        EventKind::Change,
        "# Late\n",
    );
    let t = tabs_of(&render(&mut f, &mut h, PANE, "s", 900.0));
    let dirty: Vec<_> = t.tabs.iter().map(|t| (t.label.as_str(), t.dirty)).collect();
    assert_eq!(dirty, [("README.md", true), ("app.ts", false)]);
    // A closed tab's late edit leaves nothing behind.
    tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "tabs",
        EventKind::Close,
        "/p/src/app.ts",
    );
    tell(
        &mut f,
        &mut h,
        PANE,
        "s",
        "code:/p/src/app.ts",
        EventKind::Change,
        "x",
    );
    assert!(buffer::read_draft(&mut h, &tab_key("s", "/p/src/app.ts"))
        .unwrap()
        .is_none());
}

#[test]
fn each_sessions_card_takes_its_own_draft_and_a_homeless_one_is_counted() {
    let d = disk();
    let mut h = host(&d);
    let mut f = Files::init(&Init::default(), &mut h).unwrap();
    let draft = |t: &str| json!({ "filePath": "/p/README.md", "text": t, "base": null });
    let view = json!({
        "cards": {
            "card:a": { "sessionId": "s", "filePath": "/p/README.md" },
            "card:b": { "sessionId": "t", "filePath": "/p/README.md" },
        },
    });
    let drafts = json!({
        "card:a": draft("from s"),
        "card:b": draft("from t"),
        "card:gone": draft("nowhere"),
    });
    let n = dispatch(
        &mut f,
        &mut h,
        "import",
        json!({ "view": view, "drafts": drafts }),
        &Ctx::default(),
    )
    .unwrap();
    assert_eq!(n["skipped"], 1);
    let mut cards = f.cards(&mut h).unwrap();
    assert_eq!(cards.all.len(), 2);
    for (session, text) in [("s", "from s"), ("t", "from t")] {
        let id = cards.promote(session, "/p/README.md");
        let c = code_of(&render(&mut f, &mut h, CARD, &id, 600.0)).unwrap();
        assert_eq!(c.draft.as_deref(), Some(text));
    }
}
