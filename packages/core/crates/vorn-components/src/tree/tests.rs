//! Every component on pixels (named for assistive technology) and on cells (read as text).

use std::cell::RefCell;

use vorn_tree::sdk::*;
use vorn_tree::{EventKind, Node, Tone};
use vornui::accesskit::{Role, Toggled};
use vornui::backend::{Backend, Cells};
use vornui::theme::color;
use vornui::{div, Id, Laid, RenderMode, Ui, UiConfig};

use super::{page, view, Ev, TreeBind};

const PIXELS: (f32, f32) = (720.0, 900.0);
const CELLS: (u16, u16) = (100, 40);

#[derive(Default)]
struct Rec {
    evs: RefCell<Vec<(Id, Ev)>>,
    checked: Vec<(&'static str, &'static str)>,
    asking: Option<&'static str>,
    menu: Option<&'static str>,
}

impl Rec {
    fn ev(&self, id: Id) -> Option<Ev> {
        let evs = self.evs.borrow();
        evs.iter().find(|(i, _)| *i == id).map(|(_, e)| e.clone())
    }

    fn any(&self, f: impl Fn(&Ev) -> bool) -> bool {
        self.evs.borrow().iter().any(|(_, e)| f(e))
    }
}

impl TreeBind for Rec {
    fn on(&self, ev: Ev) -> Id {
        let id = Id::new("tree").child(format!("{ev:?}"));
        self.evs.borrow_mut().push((id, ev));
        id
    }

    fn confirming(&self, id: Id) -> Option<(Id, Id)> {
        let asking = self.asking?;
        let e = self.ev(id)?;
        matches!(&e, Ev::Send { event, .. } if event.id == asking)
            .then(|| (id.child("ok"), id.child("no")))
    }

    fn menu_open(&self, id: Id) -> bool {
        matches!(self.ev(id), Some(Ev::Menu { node }) if Some(node.as_str()) == self.menu)
    }

    fn checked(&self, list: &str, row: &str) -> bool {
        self.checked.iter().any(|(l, r)| *l == list && *r == row)
    }
}

fn pixels(tree: &Node, bind: &Rec) -> Laid {
    let mut ui = Ui::headless(RenderMode::Cpu, &UiConfig::system(1.0, 13.0));
    let mut laid = None;
    for _ in 0..2 {
        ui.begin(color::SURFACE_BASE);
        let root = div()
            .w(PIXELS.0)
            .h(PIXELS.1)
            .col()
            .p(16.0)
            .child(view(&ui, tree, bind));
        laid = Some(ui.layout(root, PIXELS));
    }
    laid.expect("laid out")
}

fn cells(tree: &Node, bind: &Rec) -> String {
    let mut ui = Ui::cells(&UiConfig::system(1.0, 13.0));
    let size = vornui::cells::units::px(CELLS.0, CELLS.1);
    for _ in 0..2 {
        ui.begin(color::SURFACE_BASE);
        let root = div().w(size.0).h(size.1).col().child(view(&ui, tree, bind));
        ui.layout(root, size);
    }
    let mut target = Cells::offscreen(CELLS.0, CELLS.1);
    target.present(&mut ui).expect("present");
    (0..CELLS.1)
        .map(|r| target.buffer.row_text(r))
        .collect::<Vec<_>>()
        .join("\n")
}

fn named(laid: &Laid, role: Role, name: &str) -> bool {
    laid.tree
        .nodes
        .iter()
        .any(|(_, n)| n.role() == role && n.label().is_some_and(|l| l.contains(name)))
}

fn count(laid: &Laid, role: Role) -> usize {
    laid.tree
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == role)
        .count()
}

/// Component `tree` names itself `role` `name` on pixels and shows `shown` on cells.
#[track_caller]
fn both(what: &str, tree: Node, role: Role, name: &str, shown: &str) {
    vorn_tree::check(&tree).unwrap_or_else(|e| panic!("{what} checks: {e:?}"));
    let bind = Rec::default();
    let laid = pixels(&tree, &bind);
    assert!(
        named(&laid, role, name),
        "{what} on pixels is a {role:?} named {name:?}"
    );
    let text = cells(&tree, &bind);
    assert!(
        text.contains(shown),
        "{what} on cells shows {shown:?}:\n{text}"
    );
}

fn entries() -> Vec<Node> {
    vec![
        entry("a", "fix-login")
            .icon("folder-git-2")
            .code("fix-login")
            .detail("today · uncommitted changes")
            .badge("Review", Tone::Warn)
            .aside("24 MB")
            .meter(meter(24.0, [segment(20.0, "", Tone::Accent)]))
            .actions([button("a:open", "").icon("folder-open").label("Open")])
            .more([button("a:rm", "Remove")
                .danger()
                .confirm("Remove fix-login?")]),
        entry("b", "main").detail("the project").fixed(),
    ]
}

#[test]
fn text_box_and_divider() {
    let t = col([
        text("Plain words"),
        text("A title").style(vorn_tree::TextStyle::Title),
        divider(),
        row([text("left"), text("right")]).label("Pair"),
    ]);
    both("box", t.clone(), Role::Group, "Pair", "left");
    let text = cells(&t, &Rec::default());
    assert!(
        text.contains("Plain words") && text.contains("A title"),
        "{text}"
    );
}

#[test]
fn icon_and_shape() {
    let svg = "<svg viewBox='0 0 4 4'><path d='M0 0h4v4z'/></svg>";
    both(
        "shape",
        row([icon("globe"), shape(svg, '◆', 16.0, 16.0).label("Logo")]),
        Role::Image,
        "Logo",
        "◆",
    );
}

#[test]
fn button_and_its_confirm() {
    let b = button("go", "Reclaim 70 MB")
        .icon("trash-2")
        .confirm("Delete build output?");
    both(
        "button",
        b.clone(),
        Role::Button,
        "Reclaim 70 MB",
        "Reclaim 70 MB",
    );
    let bind = Rec {
        asking: Some("go"),
        ..Rec::default()
    };
    assert!(named(&pixels(&b, &bind), Role::Button, "Reclaim 70 MB"));
    let text = cells(&b, &bind);
    assert!(text.contains("Delete build output?"), "{text}");
}

#[test]
fn field_one_line_and_secret() {
    let f = col([
        field("name", "").placeholder("Name"),
        field("token", "s3cret").placeholder("Token").secret(),
    ]);
    both("field", f.clone(), Role::TextInput, "Name", "Name");
    let bind = Rec::default();
    pixels(&f, &bind);
    assert!(
        bind.any(
            |e| matches!(e, Ev::Field { node, value } if node == "token" && value == "s3cret")
        ),
        "a field binds with what it holds"
    );
}

#[test]
fn choice_chips() {
    let c = choice("kind", [("http", "HTTP"), ("mcp", "MCP")], "mcp");
    both("choice", c.clone(), Role::RadioButton, "MCP", "MCP");
    let laid = pixels(&c, &Rec::default());
    let on = laid
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::RadioButton && n.label() == Some("MCP"));
    assert_eq!(on.and_then(|(_, n)| n.toggled()), Some(Toggled::True));
}

#[test]
fn badge_notice_and_stat() {
    both(
        "badge",
        badge("Removable", Tone::Accent),
        Role::Label,
        "Removable",
        "Removable",
    );
    both(
        "notice",
        notice("The scan failed", Tone::Bad),
        Role::Status,
        "The scan failed",
        "The scan failed",
    );
    both(
        "stat",
        card([row([
            stat("73 MB", "On disk").icon("hard-drive"),
            stat("70 MB", "Build output").end(),
        ])]),
        Role::Label,
        "On disk: 73 MB",
        "73 MB",
    );
}

#[test]
fn meter_and_its_legend() {
    let m = meter(
        10.0,
        [
            segment(6.0, "Build output", Tone::Accent),
            segment(2.0, "Removable", Tone::Accent),
        ],
    )
    .aside("40 KB in merged worktrees")
    .node();
    both(
        "meter",
        m.clone(),
        Role::Meter,
        "Build output 60%",
        "Build output",
    );
    let text = cells(&m, &Rec::default());
    assert!(
        text.contains("████████████████████████▓▓▓▓▓▓▓▓"),
        "a bar of blocks:\n{text}"
    );
    assert!(text.contains("40 KB in merged"), "{text}");
}

#[test]
fn section_and_toolbar() {
    let t = col([
        toolbar([button("scan", "Rescan").icon("refresh-cw")]),
        section("Danger zone", [text("Below")]),
    ]);
    both(
        "section",
        t.clone(),
        Role::Heading,
        "Danger zone",
        "DANGER ZONE",
    );
    both("toolbar", t.clone(), Role::Toolbar, "Actions", "Rescan");
    let ui = Ui::headless(RenderMode::Cpu, &UiConfig::system(1.0, 13.0));
    let (bar, body) = page(&ui, &t, &Rec::default());
    assert!(
        bar.is_some() && body.is_some(),
        "a page lifts its toolbar out"
    );
}

#[test]
fn list_entries_hover_actions_and_more() {
    let l = list("rows", entries());
    both("entry", l.clone(), Role::ListItem, "fix-login", "fix-login");
    let laid = pixels(&l, &Rec::default());
    assert!(named(&laid, Role::Button, "Open"), "a hover action");
    assert!(named(&laid, Role::Meter, "83%"), "the row's bar");
    assert!(named(&laid, Role::Label, "Review"), "its badge");
    let open = Rec {
        menu: Some("a"),
        ..Rec::default()
    };
    assert!(
        named(&pixels(&l, &open), Role::MenuItem, "Remove"),
        "the more menu"
    );
    // A terminal cannot hover: the action joins the menu.
    let text = cells(&l, &open);
    assert!(text.contains("Open") && text.contains("Remove"), "{text}");
    assert!(text.contains("24 MB"), "{text}");
}

#[test]
fn list_groups_by_heading_with_aside() {
    let rows = [
        ("acme", "fix-login"),
        ("web", "redesign"),
        ("acme", "notes"),
    ];
    let groups = group_by(rows, |(p, _)| *p).into_iter().map(|(p, rs)| {
        group(p, rs.into_iter().map(|(_, b)| entry(b, b)))
            .note("vs main")
            .aside("24 MB")
    });
    let l = list("rows", groups);
    both("group", l.clone(), Role::Heading, "acme", "vs main");
    let laid = pixels(&l, &Rec::default());
    assert_eq!(
        count(&laid, Role::Heading),
        2,
        "two projects, notes in acme"
    );
    let iconed = list(
        "c",
        [group(
            "HTTP",
            [entry("x", "Example API").chips(["baseUrl: https://api.example.com".into()])],
        )
        .icon("globe")
        .note("2 connections")
        .actions([button("add", "Add another").icon("plus")])],
    );
    both(
        "group actions",
        iconed.clone(),
        Role::Button,
        "Add another",
        "2 connections",
    );
    both(
        "chips",
        iconed,
        Role::ListItem,
        "Example API",
        "baseUrl: https://api",
    );
}

#[test]
fn selection_checks_rows_and_bulk_acts_on_them() {
    let l = list("rows", entries()).select([button("rm-many", "Remove").danger()]);
    both("selection", l.clone(), Role::CheckBox, "Select all", "[ ]");
    let none = Rec::default();
    let laid = pixels(&l, &none);
    assert_eq!(
        count(&laid, Role::CheckBox),
        2,
        "all, and the one row that can be checked"
    );
    assert!(
        !named(&laid, Role::Button, "Remove"),
        "bulk buttons wait for a check"
    );
    assert!(
        none.any(|e| matches!(e, Ev::Check { rows, on: true, .. } if rows == &["a".to_owned()])),
        "a row's box checks it"
    );
    let some = Rec {
        checked: vec![("rows", "a")],
        ..Rec::default()
    };
    let laid = pixels(&l, &some);
    assert!(named(&laid, Role::Toolbar, "1 selected"));
    let bulk = some.evs.borrow().iter().find_map(|(_, e)| match e {
        Ev::Send { event, .. } if event.id == "rm-many" => Some(event.clone()),
        _ => None,
    });
    let bulk = bulk.expect("a bulk button");
    assert_eq!(
        (bulk.kind, bulk.selected),
        (EventKind::Click, vec!["a".to_owned()])
    );
    let text = cells(&l, &some);
    assert!(
        text.contains("[x]") && text.contains("1 selected"),
        "{text}"
    );
}

#[test]
fn a_lazy_list_builds_the_rows_in_view() {
    let many = (0..500).map(|i| entry(format!("r{i}"), format!("Row {i}")).detail("detail"));
    let l = list("big", many).lazy();
    both("lazy list", l.clone(), Role::ListItem, "Row 0", "Row 0");
    let built = count(&pixels(&l, &Rec::default()), Role::ListItem);
    assert!(built > 3 && built < 40, "{built} rows built of 500");
}

#[test]
fn empty_and_loading_lists_say_so() {
    both(
        "empty",
        list("e", []).empty("No worktrees yet"),
        Role::Status,
        "No worktrees yet",
        "No worktrees yet",
    );
    both(
        "loading",
        list("e", []).loading("Scanning"),
        Role::Status,
        "Scanning",
        "Scanning",
    );
}

#[test]
fn board_columns_and_cards() {
    let b = board([
        column("To do", [text("Write the spec")]).tone(Tone::Neutral),
        column("Done", [text("Ship it")]).tone(Tone::Good),
    ]);
    both("board", b.clone(), Role::Group, "Done", "Ship it");
    let text = cells(&b, &Rec::default());
    assert!(
        text.contains("To do") && text.contains("Write the spec"),
        "{text}"
    );
}

/// `tree` read as a platform's semantic nodes, through the native backend.
fn native(tree: &Node, bind: &Rec) -> vornui::native::Node {
    let mut ui = Ui::native(&UiConfig::system(1.0, 13.0));
    ui.begin(color::SURFACE_BASE);
    ui.layout(view(&ui, tree, bind), (390.0, 844.0));
    let mut out = vornui::native::Native::default();
    out.present(&mut ui).expect("present");
    assert!(out.json.starts_with('{'), "{}", out.json);
    ui.native_tree().expect("a native tree").clone()
}

/// The first node of `kind` labelled `label`, depth first.
fn find<'a>(
    n: &'a vornui::native::Node,
    kind: &str,
    label: &str,
) -> Option<&'a vornui::native::Node> {
    if n.kind == kind && n.label == label {
        return Some(n);
    }
    n.children.iter().find_map(|c| find(c, kind, label))
}

#[track_caller]
fn has<'a>(n: &'a vornui::native::Node, kind: &str, label: &str) -> &'a vornui::native::Node {
    find(n, kind, label).unwrap_or_else(|| panic!("a {kind} {label:?} in {n:#?}"))
}

#[test]
fn native_reads_controls_and_structure() {
    let t = col([
        toolbar([button("scan", "Rescan").icon("refresh-cw")]),
        section(
            "Danger zone",
            [text("Below").style(vorn_tree::TextStyle::Secondary)],
        ),
        field("name", "acme").placeholder("Name"),
        field("token", "s3cret").placeholder("Token").secret(),
        choice("kind", [("http", "HTTP"), ("mcp", "MCP")], "mcp"),
        notice("The scan failed", Tone::Bad),
        card([row([stat("73 MB", "On disk").icon("hard-drive")])]),
        meter(10.0, [segment(6.0, "Build output", Tone::Accent)]).node(),
    ]);
    vorn_tree::check(&t).expect("checks");
    let n = native(&t, &Rec::default());
    let bar = n.children.iter().find(|c| c.kind == "toolbar");
    let scan = has(bar.expect("a toolbar"), "button", "Rescan");
    assert_eq!(scan.value, "refresh-cw");
    let danger = has(&n, "section", "Danger zone");
    assert_eq!(has(danger, "text", "Below").style, "secondary");
    assert!(!has(&n, "field", "Name").secret);
    let token = has(&n, "field", "Token");
    assert!(token.secret && token.value.is_empty(), "a secret stays put");
    let kind = n
        .children
        .iter()
        .find(|c| c.kind == "choice")
        .expect("a choice");
    assert_eq!(has(kind, "option", "MCP").on, Some(true));
    assert_eq!(has(&n, "notice", "The scan failed").style, "bad");
    let disk = has(&n, "entry", "On disk");
    assert_eq!(has(disk, "text", "73 MB").style, "number");
    assert!(find(&n, "text", "Build output 60%").is_some(), "{n:#?}");
}

#[test]
fn native_lists_rows_checks_and_menus() {
    let l = list("rows", entries()).select([button("rm-many", "Remove").danger()]);
    let picked = Rec {
        checked: vec![("rows", "a")],
        ..Rec::default()
    };
    let n = native(&l, &picked);
    let all = has(&n, "select", "Select all");
    assert_eq!(all.on, Some(true), "the one row that can be checked is");
    let row = has(&n, "entry", "fix-login");
    assert_eq!(row.value, "fix-login · today · uncommitted changes");
    let kinds: Vec<&str> = row.children.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, ["select", "icon", "badge", "text", "more"]);
    assert_eq!(has(row, "select", "Select fix-login").on, Some(true));
    let more = &row.children[4];
    let open = has(more, "button", "Open");
    assert_eq!(open.value, "folder-open", "a hover action joins the menu");
    let rm = has(more, "button", "Remove");
    assert_eq!(
        (rm.style, rm.confirm.as_deref()),
        ("danger", Some("Remove fix-login?"))
    );
    let bulk = has(&n, "button", "Remove");
    assert_eq!(
        bulk.style, "danger",
        "a bulk button shows once a row is checked"
    );
    assert!(find(has(&n, "entry", "main"), "select", "Select main").is_none());

    let groups = list(
        "c",
        [group("HTTP", [entry("x", "Example API")])
            .icon("globe")
            .actions([button("add", "Add another").icon("plus")])],
    );
    let n = native(&groups, &Rec::default());
    let http = has(&n, "section", "HTTP");
    assert_eq!(http.children[0].kind, "icon", "a section wears its icon");
    has(http, "button", "Add another");
    has(http, "entry", "Example API");

    let many = (0..500).map(|i| entry(format!("r{i}"), format!("Row {i}")));
    let n = native(&list("big", many).lazy(), &Rec::default());
    has(&n, "entry", "Row 499");
}

#[test]
fn dots_in_every_tone_and_a_growing_line_between_them() {
    let t = row([
        dot(Tone::Busy).label("running"),
        text("api, a long name that is cut short").grow(),
        dot(Tone::Muted).label("idle"),
        button("show", "Show"),
    ]);
    both("dot", t.clone(), Role::Status, "running", "●");
    let laid = pixels(&t, &Rec::default());
    assert!(named(&laid, Role::Status, "idle"));
    assert!(
        named(&laid, Role::Button, "Show"),
        "the growing text leaves its room"
    );
    let text = cells(&t, &Rec::default());
    assert!(text.contains("Show"), "{text}");
}

#[test]
fn a_toggle_reports_the_state_it_switches_to() {
    let t = col([
        toggle("shells", "Count shells too", true),
        toggle("tools", "Count tool calls", false),
    ]);
    both(
        "toggle",
        t.clone(),
        Role::Switch,
        "Count shells too",
        "Count shells too",
    );
    let bind = Rec::default();
    let laid = pixels(&t, &bind);
    let on = laid
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::Switch && n.label() == Some("Count shells too"));
    assert_eq!(on.and_then(|(_, n)| n.toggled()), Some(Toggled::True));
    let sent = |id: &str, to: &str| {
        bind.any(|e| {
            matches!(e, Ev::Send { event, .. }
                if event.id == id && event.kind == EventKind::Toggle && event.value == to)
        })
    };
    assert!(sent("shells", "false") && sent("tools", "true"));
    let n = native(&t, &Rec::default());
    assert_eq!(has(&n, "toggle", "Count shells too").on, Some(true));
}

#[test]
fn a_per_session_tree_draws_one_sessions_child() {
    let t = per_session([
        ("t1", badge("2 done", Tone::Neutral)),
        ("t2", badge("5 done", Tone::Neutral)),
    ]);
    let one = vorn_tree::for_session(&t, "t2").expect("t2's");
    both("badge", one.clone(), Role::Label, "5 done", "5 done");
    let text = cells(one, &Rec::default());
    assert!(!text.contains("2 done"), "{text}");
}
