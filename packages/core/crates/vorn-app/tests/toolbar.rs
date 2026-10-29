//! The toolbar driven as a person does, on a vornd with its own HOME: its
//! items arranged and hidden in Customize Toolbar and kept for the next
//! launch, + starting a session at once, and a card split right and down
//! on the Flexible grid. `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{add_project, connect, key, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::arrange::List;
use vorn_app::flex::{Dir, Node};
use vorn_app::screen::menus::MENU;
use vorn_app::settings::{Category, SetAct};
use vorn_app::state::{Dialog, MenuKind};
use vorn_app::toolbar::{NEW, SPLIT, VIEW};
use vorn_app::ui::{item_id, mods, Id};
use vorn_app::App;

const RECENT: &str = "vorn.recent/toolbar/recent";

fn control(place: &str, act: Act) -> Id {
    App::control(place, &act)
}

fn new_shell(shot: &mut Shot, app: &mut App) -> String {
    let before: Vec<String> = app
        .store
        .sessions
        .iter()
        .map(|s| s.info.id.clone())
        .collect();
    shot.send(app, key("Backquote", mods::CTRL, Some("`")));
    let id = shot.until(app, "a new shell", |app, _| {
        app.st.selected.clone().filter(|s| !before.contains(s))
    });
    shot.until(app, "its screen", |app, _| app.pane(&id).map(|_| ()));
    id
}

#[test]
fn the_toolbar_is_arranged_in_customize_and_kept() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.until(&mut app, "Recent's toolbar item", |app, _| {
        app.st.points.get(RECENT).map(|_| ())
    });
    let shown = |app: &App| {
        let ids = vorn_app::toolbar::available(&app.st.points);
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let out: Vec<String> = app
            .st
            .arrange
            .toolbar
            .shown(&refs)
            .into_iter()
            .map(str::to_owned)
            .collect();
        out
    };
    assert_eq!(shown(&app), [VIEW, RECENT, NEW], "Split waits in the sheet");

    // A right click on the toolbar offers the sheet.
    shot.right_click(&mut app, Id::new("toolbar-items"));
    assert_eq!(
        app.st.menu.as_ref().map(|m| m.kind.clone()),
        Some(MenuKind::ToolbarMenu)
    );
    shot.click(&mut app, item_id(MENU, 0));
    assert_eq!(app.st.dialog, Some(Dialog::Customize));
    shot.save(&mut app, "customize");

    // Split into the bar, New first, Recent hidden.
    let show = |id: &str, shown: bool| Act::ArrangeShow {
        list: List::Toolbar,
        id: id.to_owned(),
        shown,
    };
    shot.click(&mut app, control("arrange-show", show(SPLIT, true)));
    loop {
        let ids = vorn_app::toolbar::available(&app.st.points);
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let at = app
            .st
            .arrange
            .toolbar
            .display(&refs)
            .iter()
            .position(|i| *i == NEW);
        let Some(at) = at.filter(|at| *at > 0) else {
            break;
        };
        let up = Act::ArrangeMove {
            list: List::Toolbar,
            id: NEW.to_owned(),
            to: at - 1,
        };
        shot.click(&mut app, control("arrange-up", up));
    }
    shot.click(&mut app, control("arrange-show", show(RECENT, false)));
    assert_eq!(shown(&app), [NEW, VIEW, SPLIT]);
    shot.save(&mut app, "customize-arranged");

    // Kept beside vornd, and read again by the next launch.
    let saved = std::fs::read_to_string(v.home.join("desktop/arrange.json")).expect("saved");
    let saved: Value = serde_json::from_str(&saved).expect("json");
    assert_eq!(saved["toolbar"]["hidden"], json!([RECENT]));
    let mut again = connect(&v);
    shot.until(&mut again, "Recent's item", |app, _| {
        app.st.points.get(RECENT).map(|_| ())
    });
    assert_eq!(shown(&again), [NEW, VIEW, SPLIT]);
}

#[test]
fn plus_starts_at_once_and_a_card_splits_right_then_down() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    shot.click(
        &mut app,
        control("sidebar-project", Act::Project(Some("demo".into()))),
    );
    let a = new_shell(&mut shot, &mut app);

    // General's New session starts a shell, so + starts one at once.
    shot.click(&mut app, control("sidebar-footer", Act::Settings));
    shot.click(
        &mut app,
        control("settings-nav", Act::Set(SetAct::Open(Category::General))),
    );
    let shell = SetAct::Default("newSessionStarts".into(), "\"shell\"".into());
    shot.click(&mut app, control("newSessionStarts", Act::Set(shell)));
    shot.save(&mut app, "general-new-session-starts");
    shot.until(&mut app, "the setting", |app, _| {
        app.store
            .config
            .pointer("/defaults/newSessionStarts")
            .filter(|v| *v == "shell")
            .map(|_| ())
    });
    shot.send(&mut app, key("Escape", 0, None));
    shot.frame(&mut app);
    if app.st.settings.open {
        shot.click(&mut app, control("sidebar-footer", Act::Settings));
    }
    let before = app.store.sessions.len();
    shot.click(&mut app, control("toolbar", Act::QuickStart));
    let b = shot.until(&mut app, "the session + started", |app, _| {
        (app.store.sessions.len() > before)
            .then(|| app.st.selected.clone())
            .flatten()
    });
    assert_eq!(app.st.dialog, None, "no composer, no menu");
    assert!(app.st.menu.is_none());
    assert!(app.store.session(&b).is_some_and(|s| s.is_shell()));

    // The card's header: Split right turns the grid flexible and puts a
    // shell right of the card; the first five buttons show there.
    shot.hover(&mut app, vorn_app::screen::term_id(&a));
    let split = Act::Split {
        session: Some(a.clone()),
        down: false,
    };
    shot.save(&mut app, "card-header-split");
    shot.click(&mut app, control("card", split));
    assert_eq!(app.st.columns, -1);
    let right = shot.until(&mut app, "the shell beside", |app, _| {
        let tree = app.st.grids.values().next()?.tree.clone()?;
        let id = app.st.selected.clone().filter(|s| *s != a && *s != b)?;
        tree.contains(&id).then_some((tree, id))
    });
    let (tree, c_id) = right;
    assert!(
        split_of(&tree, &a, &c_id) == Some(Dir::Row),
        "{c_id} right of {a}: {tree:?}"
    );

    // Split Down from the keys, on the new shell.
    let (code, m) = if mods::mac() {
        ("KeyD", vorn_app::app::MOD() | mods::SHIFT)
    } else {
        ("Minus", mods::CTRL | mods::SHIFT)
    };
    shot.send(&mut app, key(code, m, None));
    let down = shot.until(&mut app, "the shell under", |app, _| {
        let tree = app.st.grids.values().next()?.tree.clone()?;
        let id = app
            .st
            .selected
            .clone()
            .filter(|s| ![&a, &b, &c_id].contains(&s))?;
        tree.contains(&id).then_some((tree, id))
    });
    assert_eq!(
        split_of(&down.0, &c_id, &down.1),
        Some(Dir::Col),
        "{down:?}"
    );
    shot.save(&mut app, "split-down");

    // With two on the header, the rest open from More in the same order.
    app.st.arrange.card_shown = 2;
    shot.hover(&mut app, vorn_app::screen::term_id(&a));
    shot.click(
        &mut app,
        control("card", Act::Open(MenuKind::Card(a.clone()))),
    );
    let labels = app.menu_labels(&shot.ui);
    let expand = labels.iter().position(|l| l == "Expand");
    let close = labels.iter().position(|l| l == "Close session");
    let rename = labels.iter().position(|l| l == "Rename\u{2026}");
    assert!(
        expand.is_some() && expand < close && close < rename,
        "{labels:?}"
    );
    assert!(!labels.iter().any(|l| l == "Split right"), "{labels:?}");
    shot.save(&mut app, "card-more-overflow");

    // Close from More asks on the header, and closes.
    shot.click(&mut app, item_id(MENU, close.expect("listed")));
    assert_eq!(app.st.confirm_close.as_deref(), Some(a.as_str()));
    shot.save(&mut app, "card-more-close-asks");
    shot.click(&mut app, control("confirm", Act::CloseSession(a.clone())));
    shot.until(&mut app, "the card closed", |app, _| {
        app.store.session(&a).is_none().then_some(())
    });

    // A card whose Close is hidden still closes from More.
    (app.st.arrange.card).set_shown(vorn_app::screen::card_buttons::CLOSE, false);
    shot.card_menu(&mut app, &b, "Close session");
    assert_eq!(app.st.confirm_close.as_deref(), Some(b.as_str()));
    shot.click(&mut app, control("confirm", Act::CloseSession(b.clone())));
    shot.until(&mut app, "the other card closed", |app, _| {
        app.store.session(&b).is_none().then_some(())
    });
}

/// The direction of the split whose two children are exactly `first` and `second`.
fn split_of(node: &Node, first: &str, second: &str) -> Option<Dir> {
    match node {
        Node::Leaf(_) => None,
        Node::Split { dir, a, b, .. } => {
            if matches!((&**a, &**b), (Node::Leaf(x), Node::Leaf(y)) if x == first && y == second) {
                return Some(*dir);
            }
            split_of(a, first, second).or_else(|| split_of(b, first, second))
        }
    }
}

#[test]
fn every_entry_in_a_cards_more_menu_runs_under_the_pointer() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    shot.click(
        &mut app,
        control("sidebar-project", Act::Project(Some("demo".into()))),
    );
    let a = new_shell(&mut shot, &mut app);
    let _b = new_shell(&mut shot, &mut app);
    app.st.arrange.card_shown = 0;
    shot.hover(&mut app, vorn_app::screen::term_id(&a));
    shot.click(
        &mut app,
        control("card", Act::Open(MenuKind::Card(a.clone()))),
    );
    let labels = app.menu_labels(&shot.ui);
    let at = labels
        .iter()
        .position(|l| l == "Expand")
        .expect("{labels:?}");
    // The pointer travels from More to the entry over a few frames, as a hand does.
    let to = shot
        .ui
        .rect_of(item_id(MENU, at))
        .expect("the entry is laid out");
    let from = shot
        .ui
        .rect_of(control("card", Act::Open(MenuKind::Card(a.clone()))))
        .expect("More");
    let mut now = shot.ui.now();
    for step in 1..=12 {
        let t = step as f32 / 12.0;
        shot.ui.pointer_move(
            from.x + (to.x + to.w / 2.0 - from.x) * t,
            from.y + (to.y + to.h / 2.0 - from.y) * t,
        );
        now += std::time::Duration::from_millis(16);
        shot.ui.set_now(now);
        shot.frame(&mut app);
    }
    assert!(app.st.menu.is_some(), "the menu stays open on the way");
    shot.ui.pointer_down();
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    shot.frame(&mut app);
    assert_eq!(app.st.focused.as_deref(), Some(a.as_str()), "Expand ran");
}
