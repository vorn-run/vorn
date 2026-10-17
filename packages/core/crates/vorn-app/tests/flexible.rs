//! The Flexible grid driven as a person does: chosen from View options, a
//! divider dragged and nudged from the keyboard, the panes evened, and the
//! tree kept for the next launch. `VORN_APP_SHOT=<prefix>` saves a frame of each.

#![cfg(unix)]

mod support;

use serde_json::json;
use support::{add_project, connect, key, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::flex::Node;
use vorn_app::screen::menus::MENU;
use vorn_app::screen::{flex_divider_id, term_id};
use vorn_app::state::MenuKind;
use vorn_app::ui::backend::{Backend, Cells};
use vorn_app::ui::{cells::units, item_id, mods, Ui};
use vorn_app::{ui_config, App};

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

/// This state as a terminal shows it at `cols`×`rows`, saved beside the
/// shots when they are.
fn cells(app: &mut App, cols: u16, rows: u16) -> String {
    let mut ui = Ui::cells(&ui_config(1.0));
    let size = units::px(cols, rows);
    // The terminals take the size their panes give them.
    for _ in 0..6 {
        app.frame(&mut ui, size);
        std::thread::sleep(std::time::Duration::from_millis(100));
        app.pump();
    }
    app.frame(&mut ui, size);
    let mut target = Cells::offscreen(cols, rows);
    target.present(&mut ui).expect("present");
    let text = (0..rows)
        .map(|r| target.buffer.row_text(r))
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(base) = std::env::var_os("VORN_APP_SHOT") {
        let path = format!("{}-cells-flexible-{cols}x{rows}", base.to_string_lossy());
        let html = target.buffer.to_html(&format!("flexible {cols}x{rows}"));
        std::fs::write(format!("{path}.html"), html).expect("html");
        std::fs::write(format!("{path}.txt"), &text).expect("txt");
    }
    text
}

/// The workspace's tree, when the grid is flexible.
fn tree(app: &App) -> Option<Node> {
    let grid = app.st.grids.values().next()?;
    grid.flexible.then(|| grid.tree.clone()).flatten()
}

/// Whether a split in `node` has `ratio`.
fn has_ratio(node: &Node, want: f32) -> bool {
    match node {
        Node::Leaf(_) => false,
        Node::Split { ratio, a, b, .. } => {
            (ratio - want).abs() < 1e-3 || has_ratio(a, want) || has_ratio(b, want)
        }
    }
}

/// The root split's ratio.
fn ratio(app: &App) -> f32 {
    match tree(app) {
        Some(Node::Split { ratio, .. }) => ratio,
        other => panic!("a split at the root, not {other:?}"),
    }
}

/// Drags the root divider `dx` across.
fn drag(shot: &mut Shot, app: &mut App, dx: f32) {
    shot.frame(app);
    let r = shot
        .ui
        .rect_of(flex_divider_id(1))
        .expect("the root divider is on screen");
    let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    shot.ui.pointer_move(x, y);
    shot.ui.pointer_down();
    app.events(&mut shot.ui);
    shot.frame(app);
    for step in 1..=4 {
        shot.ui.pointer_move(x + dx * step as f32 / 4.0, y);
        app.events(&mut shot.ui);
        shot.frame(app);
    }
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    shot.frame(app);
}

#[test]
fn the_flexible_grid_splits_resizes_and_is_kept() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    add_project(&mut app, &v, "demo", json!({}));
    let mut shot = Shot::new();
    shot.frame(&mut app);
    shot.click(
        &mut app,
        App::control("sidebar-project", &Act::Project(Some("demo".into()))),
    );
    let a = new_shell(&mut shot, &mut app);
    let b = new_shell(&mut shot, &mut app);
    let c = new_shell(&mut shot, &mut app);

    // View options › Flexible lays the cards out as a tree.
    shot.click(
        &mut app,
        App::control("toolbar", &Act::Open(MenuKind::ViewOptions)),
    );
    let labels = app.menu_labels(&shot.ui);
    let i = labels
        .iter()
        .position(|l| l == "Flexible")
        .unwrap_or_else(|| panic!("Flexible in {labels:?}"));
    shot.click(&mut app, item_id(MENU, i));
    assert_eq!(app.st.columns, -1);
    assert!(app
        .menu_labels(&shot.ui)
        .contains(&"Equal Pane Sizes".into()));
    shot.save(&mut app, "flexible-menu");
    shot.send(&mut app, key("Escape", 0, None));
    let t = tree(&app).expect("a tree");
    let mut leaves = t.leaves();
    leaves.sort_unstable();
    let mut want = vec![a.as_str(), b.as_str(), c.as_str()];
    want.sort_unstable();
    assert_eq!(leaves, want);
    shot.until(&mut app, "the divider", |_, ui| {
        ui.rect_of(flex_divider_id(1)).map(|_| ())
    });
    shot.save(&mut app, "flexible");

    // Dragging the divider moves it, and stops short of a pane's minimum.
    let before = ratio(&app);
    drag(&mut shot, &mut app, 160.0);
    let dragged = ratio(&app);
    assert!(dragged > before + 0.05, "{before} -> {dragged}");
    drag(&mut shot, &mut app, 4000.0);
    assert!(ratio(&app) < 0.9, "the far pane keeps its minimum");
    shot.save(&mut app, "flexible-dragged");

    // A new card splits the selected one; closing it gives its room back.
    shot.click(&mut app, App::control("card-box", &Act::Select(a.clone())));
    let (held, split) = (tree(&app), ratio(&app));
    let d = new_shell(&mut shot, &mut app);
    shot.until(&mut app, "the new card in the tree", |app, _| {
        tree(app).filter(|t| t.contains(&d)).map(|_| ())
    });
    assert_eq!(ratio(&app), split, "a new card keeps the dragged split");
    shot.save(&mut app, "flexible-four");
    shot.hover(&mut app, term_id(&d));
    shot.click(&mut app, App::control("card", &Act::AskClose(d.clone())));
    shot.click(
        &mut app,
        App::control("confirm", &Act::CloseSession(d.clone())),
    );
    shot.until(&mut app, "its pane gone", |app, _| {
        tree(app).filter(|t| !t.contains(&d)).map(|_| ())
    });
    assert_eq!(tree(&app), held, "closing it gives its room back");

    // The keyboard moves the divider and evens the panes.
    shot.click(&mut app, App::control("card-box", &Act::Select(a.clone())));
    let before = tree(&app);
    // The far pane is at its minimum: only the way back moves.
    shot.send(
        &mut app,
        key("ArrowLeft", MOD() | mods::ALT | mods::SHIFT, None),
    );
    assert_ne!(tree(&app), before, "the keys moved a divider");
    shot.send(&mut app, key("Equal", MOD() | mods::ALT, Some("=")));
    let mut even = before.expect("a tree");
    even.equalize();
    assert_eq!(tree(&app), Some(even));

    // The tree is kept with the viewer's settings for the next launch.
    drag(&mut shot, &mut app, 120.0);
    let kept = tree(&app).expect("a tree");
    shot.until(&mut app, "the tree saved", |app, _| {
        app.pump();
        (!app.busy()).then_some(())
    });
    drop(app);
    let mut app = connect(&v);
    let mut shot = Shot::new();
    shot.frame(&mut app);
    assert_eq!(app.st.columns, -1, "the grid opens flexible");
    let back = tree(&app).expect("the kept tree");
    assert_eq!(back.leaves(), kept.leaves());
    match (&back, &kept) {
        (Node::Split { ratio: r1, .. }, Node::Split { ratio: r2, .. }) => {
            assert!((r1 - r2).abs() < 1e-3, "{r1} kept as {r2}")
        }
        _ => panic!("a split at the root"),
    }
    shot.save(&mut app, "flexible-relaunched");

    // A terminal shows the same panes, each card whole in its cells.
    for (cols, rows) in [(120, 36), (80, 24)] {
        let text = cells(&mut app, cols, rows);
        let cards = text.matches("Open").count();
        assert_eq!(cards, 3, "{cols}x{rows}:\n{text}");
    }

    // A layout opened elsewhere sets the grid it names: columns, then a tree.
    let layouts = v.home.join("layouts");
    std::fs::create_dir_all(&layouts).expect("layouts");
    let project = v.home.join("demo");
    let p = project.to_str().expect("path");
    std::fs::write(
        layouts.join("cols.toml"),
        format!("name = \"cols\"\nproject = \"{p}\"\nview = \"grid\"\ncolumns = 2\nshell = true\n"),
    )
    .expect("layout");
    std::fs::write(
        layouts.join("flex.toml"),
        format!("name = \"flex\"\nproject = \"{p}\"\nsplit = \"vertical\"\nratio = 0.7\n\n[[children]]\nshell = true\n\n[[children]]\nshell = true\n"),
    )
    .expect("layout");
    app.rpc()
        .call("layout:open", json!({ "name": "cols" }))
        .expect("opened");
    shot.until(&mut app, "two columns", |app, _| {
        app.pump();
        (app.st.columns == 2).then_some(())
    });
    app.rpc()
        .call("layout:open", json!({ "name": "flex" }))
        .expect("opened");
    shot.until(&mut app, "the layout's tree", |app, _| {
        app.pump();
        tree(app).filter(|t| has_ratio(t, 0.7)).map(|_| ())
    });
    // The layout's split stays whole; the cards already open sit beside it.
    let Some(Node::Split { a, .. }) = tree(&app) else {
        panic!("a split at the root");
    };
    assert!(matches!(*a, Node::Split { ratio, ref a, ref b, .. }
        if (ratio - 0.7).abs() < 1e-3 && matches!(**a, Node::Leaf(_)) && matches!(**b, Node::Leaf(_))));
    assert_eq!(app.st.columns, -1);
    shot.save(&mut app, "flexible-layout");
}
