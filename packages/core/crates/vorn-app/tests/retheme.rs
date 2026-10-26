//! Picking a theme in Settings › Appearance recolours the window and the
//! terminals already running.

#![cfg(unix)]

mod support;

use serde_json::json;
use support::{wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::screen::settings::theme_tile_id;
use vorn_app::state::MenuKind;
use vorn_app::ui::palette;
use vorn_app::App;

#[test]
fn picking_a_theme_recolours_the_running_terminals() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let dir = v.home.join("proj");
    std::fs::create_dir_all(&dir).expect("dir");
    let info = app.rpc().call("shell:create", json!(dir)).expect("shell");
    let sid = info["id"].as_str().expect("id").to_owned();
    let mut shot = Shot::new();
    wait_for("the shell's screen", || {
        shot.frame(&mut app);
        app.pane(&sid).map(|_| ())
    });
    shot.click(&mut app, App::control("sidebar-footer", &Act::Settings));
    let picker = Act::Open(MenuKind::ThemePicker(vorn_theme::Appearance::Dark));
    for id in ["nord", "dracula"] {
        shot.click(&mut app, App::control("theme-chip", &picker));
        shot.click(&mut app, theme_tile_id(id));
        let theme = vorn_theme::find_bundled(id).expect("bundled");
        let bg = theme.terminal.background.to_u32();
        let fg = theme.terminal.foreground.to_u32();
        wait_for(&format!("{id} on the terminal"), || {
            shot.frame(&mut app);
            app.pump();
            let p = app.pane(&sid)?;
            (palette::current().theme.id == id && p.bg == bg && p.fg == fg).then_some(())
        });
    }
}
