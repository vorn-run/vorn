//! The terminal UI's surface (Cells), driven as the TUI drives it: menus open
//! against what opened them, at the sizes a terminal has.

#![cfg(unix)]

mod support;

use serde_json::json;
use support::{connect, wait_for, TestVornd};
use vorn_app::act::Act;
use vorn_app::screen::menus::MENU;
use vorn_app::state::MenuKind;
use vorn_app::ui::{cells::units, item_id, Rect, Ui};
use vorn_app::App;

/// A frame of `app` on a `cols`×`rows` terminal.
fn frame(app: &mut App, ui: &mut Ui, (cols, rows): (u16, u16)) {
    app.frame(ui, units::px(cols, rows));
}

/// A click in the middle of `id`'s cells, as a terminal reports one.
fn click(app: &mut App, ui: &mut Ui, size: (u16, u16), r: Rect) {
    ui.pointer_move(
        (r.x + r.w / 2.0).floor() + 0.5,
        (r.y + r.h / 2.0).floor() + 0.5,
    );
    ui.pointer_down();
    app.events(ui);
    ui.pointer_up();
    app.events(ui);
    frame(app, ui, size);
}

#[test]
fn menus_open_beside_their_trigger_on_a_terminal() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = connect(&v);
    let dir = v.home.join("proj");
    std::fs::create_dir_all(&dir).expect("dir");
    let info = app.rpc().call("shell:create", json!(dir)).expect("shell");
    let sid = info["id"].as_str().expect("id").to_owned();
    wait_for("the shell", || {
        app.pump();
        app.store.session(&sid).map(|_| ())
    });
    for size in [(120, 36), (80, 24)] {
        let mut ui = Ui::cells(&vorn_app::ui_config(1.0));
        frame(&mut app, &mut ui, size);
        app.st.menu = None;
        let trigger = App::control("toolbar", &Act::Open(MenuKind::ViewOptions));
        let t = ui.rect_of(trigger).expect("the view options button");
        click(&mut app, &mut ui, size, t);
        assert_eq!(
            app.st.menu.as_ref().map(|m| m.kind.clone()),
            Some(MenuKind::ViewOptions),
            "{size:?}"
        );
        let first = (0..12)
            .find_map(|i| ui.rect_of(item_id(MENU, i)))
            .expect("the menu's first entry");
        // Under the button, its right edge near the button's, inside the terminal.
        assert!(
            first.y > t.y && first.y <= t.y + t.h + 3.0,
            "{size:?}: {first:?} under {t:?}"
        );
        assert!(
            (first.x + first.w - (t.x + t.w)).abs() <= 3.0,
            "{size:?}: {first:?} against {t:?}"
        );
        assert!(
            first.x >= 0.0 && first.x + first.w <= f32::from(size.0),
            "{size:?}"
        );
        app.st.menu = None;
        ui.close_menu();
    }
}
