//! The terminal UI drawn as pixels, as the TUI draws it on a terminal that
//! shows images: the desktop's screens at the terminal's pixel size, a
//! keystroke sending a patch rather than the frame, and Settings offering
//! the drawing mode. `VORN_APP_SHOT` saves each frame as a PNG.

#![cfg(unix)]

mod support;

use std::time::Duration;

use serde_json::json;
use support::{connect, wait_for, TestVornd};
use vorn_app::act::Act;
use vorn_app::screen::TermDrawing;
use vorn_app::settings::SetAct;
use vorn_app::ui::tty::kitty::Wrap;
use vorn_app::ui::tty::present::{Placing, Sent, Transport};
use vorn_app::ui::tty::PixelFrames;
use vorn_app::ui::{Input, Ui};
use vorn_app::App;

/// 120x40 cells of 16x32 pixels, drawn at scale 2.
const CELL: (u32, u32) = (16, 32);
const PX: (u32, u32) = (120 * CELL.0, 40 * CELL.1);
const SCALE: f32 = 2.0;
const SIZE: (f32, f32) = (PX.0 as f32 / SCALE, PX.1 as f32 / SCALE);

struct Tui {
    ui: Ui,
    frames: PixelFrames,
}

impl Tui {
    fn new(app: &mut App) -> Tui {
        // As the TUI starts: a cells Ui switched to pixels.
        let mut ui = Ui::cells(&vorn_app::ui_config(1.0));
        ui.set_surface(false, SCALE);
        (app.chrome, app.traffic) = (false, false);
        app.term_drawing = Some(TermDrawing {
            supported: true,
            pixels: true,
        });
        let transport = Transport {
            file_dir: None,
            zlib: true,
        };
        Tui {
            ui,
            frames: PixelFrames::new(Wrap::Direct, Placing::Cursor, CELL, transport),
        }
    }

    fn draw(&mut self, app: &mut App, name: &str) -> Sent {
        let mut out = Vec::new();
        app.frame(&mut self.ui, SIZE);
        let sent = match self.frames.draw(&mut self.ui, PX, &mut out) {
            Some(s) => s,
            None => {
                app.frame(&mut self.ui, SIZE);
                self.frames
                    .draw(&mut self.ui, PX, &mut out)
                    .expect("atlas room")
            }
        };
        assert_eq!(sent.bytes, out.len());
        if let Some(base) = std::env::var_os("VORN_APP_SHOT") {
            let (rgba, size) = self.frames.pixels();
            let path = format!("{}-{name}.png", base.to_string_lossy());
            vornui::write_png(&path, size, rgba).expect("png");
        }
        sent
    }

    fn click(&mut self, app: &mut App, id: vorn_app::ui::Id) {
        app.frame(&mut self.ui, SIZE);
        assert!(app.click(&mut self.ui, id), "{id:?} is on screen");
        app.frame(&mut self.ui, SIZE);
    }
}

#[test]
fn the_tui_draws_the_desktop_screens_as_pixels() {
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
    let mut t = Tui::new(&mut app);
    assert!(!t.ui.is_cells(), "laid out as the desktop lays out");
    wait_for("the card's screen", || {
        app.frame(&mut t.ui, SIZE);
        app.pane(&sid).map(|_| ())
    });
    std::thread::sleep(Duration::from_millis(500));
    let first = t.draw(&mut app, "pixels-sessions");
    assert!(first.whole && first.images == 1, "{first:?}");

    // A keystroke into the shell is a patch, not the frame.
    app.focus_session(&sid);
    t.draw(&mut app, "pixels-focused");
    std::thread::sleep(Duration::from_millis(300));
    t.draw(&mut app, "pixels-focused");
    for ch in "echo pixels".chars() {
        app.send(&mut t.ui, Input::char(ch));
    }
    std::thread::sleep(Duration::from_millis(300));
    app.pump();
    let typed = t.draw(&mut app, "pixels-typed");
    assert!(!typed.whole, "typing sends what changed: {typed:?}");
    assert!(typed.images >= 1, "{typed:?}");
    assert!(
        typed.bytes * 10 < first.bytes,
        "{typed:?} against {first:?}"
    );

    // Settings › Appearance offers the drawing mode first.
    app.st.sidebar = true;
    t.click(&mut app, App::control("sidebar-footer", &Act::Settings));
    assert!(app.st.settings.open);
    let to_text = Act::Set(SetAct::Default("tuiDrawing".into(), "\"text\"".into()));
    let text = App::control("tui-drawing", &to_text);
    t.draw(&mut app, "pixels-settings");
    t.click(&mut app, text);
    wait_for("the setting saved", || {
        app.pump();
        let c = app
            .rpc()
            .call("config:load", serde_json::Value::Null)
            .ok()?;
        (c.pointer("/defaults/tuiDrawing")? == "text").then_some(())
    });
}
