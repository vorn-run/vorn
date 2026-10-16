use super::sdk::*;
use super::*;
use serde_json::json;

fn page() -> Node {
    col([
        toolbar([button("scan", "Scan").icon("refresh-cw")]),
        summary([
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
    assert!(WIT.contains("export render"));
}
