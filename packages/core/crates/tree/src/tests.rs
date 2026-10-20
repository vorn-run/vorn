use super::sdk::*;
use super::*;
use serde_json::json;

fn page() -> Node {
    col([
        toolbar([button("scan", "Scan").icon("refresh-cw")]),
        card([
            stat("3", "worktrees"),
            stat("1.2 GB", "build output").tone(Tone::Accent).end(),
        ]),
        meter(
            10.0,
            [
                segment(4.0, "Build", Tone::Accent),
                segment(2.0, "Code", Tone::Neutral),
            ],
        )
        .node(),
        list(
            "rows",
            [group(
                "vorn",
                [entry("a", "feat-x")
                    .code("feat-x")
                    .detail("idle 3 days")
                    .badge("Removable", Tone::Accent)
                    .aside("120 MB")
                    .chips(["base: main".to_owned()])
                    .actions([button("a:open", "Open").icon("folder")])
                    .more([button("a:rm", "Remove").danger().confirm("Remove feat-x?")])],
            )
            .note("2 worktrees")],
        )
        .select([button("rm-many", "Remove")]),
        field("token", "").placeholder("Token").secret(),
        choice("kind", [("http", "HTTP"), ("mcp", "MCP")], "mcp"),
        shape(
            "<svg viewBox='0 0 4 4'><path d='M0 0h4v4z'/></svg>",
            '◆',
            16.0,
            16.0,
        ),
    ])
}

#[test]
fn a_tree_travels_as_json_and_back() {
    let r = Rendered::new(page());
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["tokens"], 1);
    assert_eq!(v["tree"]["children"][0]["kind"], "toolbar");
    assert_eq!(read(&v).unwrap(), r);
}

#[test]
fn a_program_writes_the_same_json_by_hand() {
    let v = json!({"tokens": 1, "tree": {"kind": "list", "id": "l", "children": [
        {"kind": "entry", "id": "x", "title": "One", "status": "good", "props": {"p": 4}},
        {"kind": "group", "heading": "G", "children": [{"kind": "entry", "id": "y", "title": "Two"}]}
    ]}});
    let r = read(&v).unwrap();
    let Kind::Entry(e) = &r.tree.children[0].kind else {
        panic!()
    };
    assert_eq!(e.status, Some(Tone::Good));
    assert!(e.selectable);
    assert_eq!(r.tree.children[0].props.p, Some(4.0));
}

#[test]
fn a_later_token_reads_as_unknown_and_a_later_set_is_refused() {
    let p: Paint = serde_json::from_value(json!({"token": "sparkle", "alpha": 0.5})).unwrap();
    assert_eq!(
        p,
        Paint::Tint {
            token: Token::Unknown,
            alpha: 0.5
        }
    );
    let e = read(&json!({"tokens": 9, "tree": {"kind": "box"}})).unwrap_err();
    assert!(e[0].starts_with("$.tokens"), "{e:?}");
}

#[test]
fn check_names_every_mistake_with_its_place() {
    let bad = col([
        button("x", "One"),
        button("x", "Two"),
        node_without_id(),
        list("l", [text("not a row")]),
        text("no").child(text("children")),
        shape("<div/>", '◆', 8.0, 8.0),
    ]);
    let e = check(&bad).unwrap_err();
    let has = |s: &str| e.iter().any(|m| m.contains(s));
    assert!(has("$.children[1].id: 'x' names two nodes"), "{e:?}");
    assert!(has("$.children[2]: a field needs an id"), "{e:?}");
    assert!(
        has("$.children[3].children[0]: a list holds entries"),
        "{e:?}"
    );
    assert!(has("$.children[4]: holds no children"), "{e:?}");
    assert!(has("$.children[5].svg: is not an <svg>"), "{e:?}");
}

fn node_without_id() -> Node {
    let mut f = field("f", "");
    f.id = None;
    f
}

#[test]
fn check_bounds_a_tree() {
    let deep = (0..MAX_DEPTH + 2).fold(text("x"), |n, _| col([n]));
    assert!(check(&deep).unwrap_err()[0].contains("nests deeper"));
    let wide = col((0..MAX_NODES).map(|_| text("x")));
    assert!(check(&wide).unwrap_err()[0].contains("nodes; a tree has"));
    let long = text("x".repeat(MAX_TEXT + 1));
    assert!(check(&long).unwrap_err()[0].contains("text is"));
}

#[test]
fn bulk_needs_a_list_that_selects() {
    let mut l = list("l", []).select([button("go", "Go")]);
    if let Kind::List { select, .. } = &mut l.kind {
        *select = false;
    }
    assert!(check(&l).unwrap_err()[0].contains("set select"));
}

#[test]
fn group_by_keeps_first_seen_order() {
    let g = group_by(["b1", "a1", "b2"], |s| s.as_bytes()[0]);
    assert_eq!(g, vec![(b'b', vec!["b1", "b2"]), (b'a', vec!["a1"])]);
}

#[test]
fn the_page_checks() {
    check(&page()).unwrap();
}

#[test]
fn a_patch_replaces_or_removes_a_node_by_id() {
    let mut tree = col([
        text("Head").id("head"),
        list("rows", [entry("a", "One"), entry("b", "Two")]),
    ]);
    let ok = patch(
        &mut tree,
        &[
            Patch {
                id: "head".into(),
                node: Some(text("New head").id("head")),
            },
            Patch {
                id: "a".into(),
                node: None,
            },
        ],
    );
    assert_eq!(ok, Ok(()));
    let json = serde_json::to_string(&tree).unwrap();
    assert!(json.contains("New head") && !json.contains("One"), "{json}");
    let was = tree.clone();
    let bad = patch(
        &mut tree,
        &[Patch {
            id: "b".into(),
            node: Some(entry("", "Nameless")),
        }],
    );
    assert!(bad.is_err(), "a patch that breaks the tree is refused");
    assert_eq!(tree, was, "and leaves it as it was");
    assert!(patch(
        &mut tree,
        &[Patch {
            id: "zz".into(),
            node: None
        }]
    )
    .is_err());
}

#[test]
fn chained_patches_cannot_nest_past_the_limit() {
    let mut tree = col([text("Leaf").id("p0")]);
    let patches: Vec<Patch> = (0..MAX_DEPTH)
        .map(|i| Patch {
            id: format!("p{i}"),
            node: Some(col([text("Leaf").id(format!("p{}", i + 1))]).id(format!("p{i}"))),
        })
        .collect();
    let err = patch(&mut tree, &patches).unwrap_err();
    assert!(err[0].contains("deeper"), "{err:?}");
    assert_eq!(tree, col([text("Leaf").id("p0")]));
}

#[test]
fn a_per_session_tree_finds_each_sessions_child() {
    let t = per_session([("t1", badge("1 done", Tone::Neutral)), ("t2", text("x"))]);
    check(&t).unwrap();
    assert!(matches!(
        for_session(&t, "t1").map(|n| &n.kind),
        Some(Kind::Badge(_))
    ));
    assert!(for_session(&t, "t3").is_none());
    assert!(is_blank(&per_session(Vec::<(String, Node)>::new())));
    assert!(is_blank(&col([col([])])));
    assert!(is_blank(&col([text(" ")])));
    assert!(!is_blank(&col([text("3 idle")])));
    assert!(!is_blank(&t));
}

#[test]
fn a_toggle_needs_an_id_and_a_dot_takes_a_tone() {
    let mut t = toggle("pref", "Count shells", true);
    check(&t).unwrap();
    t.id = None;
    assert_eq!(check(&t).unwrap_err(), ["$: a toggle needs an id"]);
    let d = dot(Tone::Neutral).tone(Tone::Busy).label("running");
    let json = serde_json::to_value(&d).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "kind": "dot", "tone": "busy", "props": { "label": "running" } })
    );
    let e: Event = serde_json::from_value(
        serde_json::json!({ "id": "pref", "kind": "toggle", "value": "false" }),
    )
    .unwrap();
    assert_eq!(e.kind, EventKind::Toggle);
}

#[test]
fn an_answer_reads_what_it_says_and_nothing_else() {
    let a = Answer::read(&serde_json::json!({
        "toast": "Done", "close": true, "focus": "t1", "window": "compact",
        "patch": [{ "id": "a" }], "later": 1
    }));
    assert_eq!(a.toast.as_deref(), Some("Done"));
    assert!(a.close && !a.show);
    assert_eq!(
        (a.focus.as_deref(), a.window.as_deref()),
        (Some("t1"), Some("compact"))
    );
    assert_eq!(a.patch.len(), 1);
    assert_eq!(Answer::read(&serde_json::Value::Null), Answer::default());
    assert_eq!(
        Answer::read(&serde_json::json!({ "close": "yes" })),
        Answer::default()
    );
}

#[test]
fn the_parts_a_pane_draws_travel_and_check() {
    let tree = col([
        tabs(
            "tabs",
            Tabs {
                tabs: vec![Tab {
                    id: "/a.rs".into(),
                    label: "a.rs".into(),
                    file: true,
                    active: true,
                    preview: true,
                    ..Tab::default()
                }],
                lead: vec![Tool {
                    id: "tree".into(),
                    label: "Hide file tree".into(),
                    icon: "list-tree".into(),
                    on: true,
                }],
                ..Tabs::default()
            },
        ),
        split(
            "split",
            0.38,
            "Resize file tree",
            [
                outline(
                    "tree",
                    Outline {
                        rows: vec![
                            Line {
                                id: "/src".into(),
                                label: "src".into(),
                                depth: 0,
                                kind: LineKind::Folder {
                                    open: true,
                                    busy: false,
                                },
                            },
                            Line {
                                id: String::new(),
                                label: "empty".into(),
                                depth: 1,
                                kind: LineKind::Note,
                            },
                        ],
                        filter: Some(Filter::default()),
                        files: true,
                        ..Outline::default()
                    },
                ),
                code(
                    "code",
                    Code {
                        path: "/a.rs".into(),
                        text: "fn main() {}\n".into(),
                        rev: 1,
                        state: CodeState::Edit,
                        ..Code::default()
                    },
                    [button("back", "Put back")],
                ),
            ],
        ),
        log(Log {
            commands: vec![Command {
                command: Some("ls".into()),
                started: true,
                ..Command::default()
            }],
        }),
        field("q", "").live(),
    ]);
    let v = serde_json::to_value(Rendered::new(tree.clone())).unwrap();
    assert_eq!(
        v["tree"]["children"][1]["children"][0]["rows"][0]["line"],
        "folder"
    );
    assert_eq!(read(&v).unwrap().tree, tree);
    let Kind::Log(l) = &tree.children[2].kind else {
        panic!("a log")
    };
    assert_eq!(l.running().and_then(|c| c.command.as_deref()), Some("ls"));
}

#[test]
fn a_part_that_cannot_be_drawn_says_why() {
    let bad = col([
        split("s", 2.0, "", [divider(), divider(), divider()]),
        code("c", Code::default(), [text("not a button")]),
        tabs(
            "t",
            Tabs {
                tabs: vec![Tab::default(), Tab::default()],
                ..Tabs::default()
            },
        ),
    ]);
    let errs = check(&bad).unwrap_err().join("\n");
    assert!(
        errs.contains("$.children[0].ratio: goes from 0 to 1"),
        "{errs}"
    );
    assert!(errs.contains("a split holds one child or two"), "{errs}");
    assert!(errs.contains("a code's strip holds buttons only"), "{errs}");
    assert!(errs.contains("a tab needs an id"), "{errs}");
}

#[test]
fn an_event_carries_its_scope_and_an_answer_can_ask_first() {
    let e: Event = serde_json::from_value(
        json!({"id": "tabs", "kind": "close", "value": "/a", "scope": "s1", "width": 400}),
    )
    .unwrap();
    assert_eq!(
        (e.kind, e.scope.as_deref(), e.width),
        (EventKind::Close, Some("s1"), Some(400))
    );
    let a = Answer::read(&json!({"ask": {"title": "Discard unsaved changes?", "event": e}}));
    assert_eq!(a.ask.map(|q| q.event.value), Some("/a".to_owned()));
    let s: Surface = serde_json::from_value(json!({"cells": true, "scope": "s1"})).unwrap();
    assert_eq!(s.scope.as_deref(), Some("s1"));
}

#[test]
fn a_web_page_travels_and_names_its_view() {
    let tree = col([
        tabs(
            "tabs",
            Tabs {
                tabs: vec![Tab {
                    id: "0".into(),
                    label: "Plan".into(),
                    active: true,
                    icon: Some("file-text".into()),
                    ..Tab::default()
                }],
                ..Tabs::default()
            },
        ),
        web("page", "s1", "https://example.com/"),
    ]);
    let v = serde_json::to_value(Rendered::new(tree.clone())).unwrap();
    assert_eq!(v["tree"]["children"][1]["kind"], "web");
    assert_eq!(v["tree"]["children"][1]["view"], "s1");
    assert_eq!(v["tree"]["children"][0]["tabs"][0]["icon"], "file-text");
    let back: Rendered = serde_json::from_value(v).unwrap();
    assert_eq!(back.tree, tree);
    assert!(check(&tree).is_ok());

    let e = check(&col([web("page", " ", "about:blank")])).unwrap_err();
    assert!(
        e.iter().any(|m| m.contains(".view: names the web view")),
        "{e:?}"
    );
    let e = check(&col([web("page", "s1", "").child(text("no"))])).unwrap_err();
    assert!(e.iter().any(|m| m.contains("holds no children")), "{e:?}");

    let page = Event::new("page", EventKind::Page, r#"{"type":"submit","text":"a"}"#);
    assert_eq!(serde_json::to_value(&page).unwrap()["kind"], "page");
}
