//! The app's screens in a terminal: the same [`App`] laid out in cells and
//! drawn through vornui's Cells backend, input read from the terminal.
//!
//! Keys the terminal cannot send as the window's shortcuts: Alt stands in
//! for the command key (Alt+1..9 jump to a session, Alt+[ and Alt+] cycle,
//! Alt+B shows or hides the sidebar, Alt+K opens the palette, Alt+T and
//! Alt+S switch to Tasks and Sessions, Alt+Shift+W to Workflows), and
//! Ctrl+\ detaches (the sessions keep running in vornd).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::MOD;
use crate::client::Endpoint;
use crate::ui::backend::{Backend, Cells};
use crate::ui::cells::term::{TermEvent, Terminal};
use crate::ui::{mods, Input, Ui};
use crate::App;

/// Where the TUI starts.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The session to open full window, its sidebar hidden.
    pub focus: Option<String>,
}

/// What the TUI does with a key before the app sees it.
#[derive(Debug, PartialEq)]
enum Key {
    Detach,
    App(Input),
}

/// The window's `MOD` chords from the Alt chords a terminal can send.
fn route(input: Input) -> Key {
    let Input::Key {
        code,
        mods: m,
        text,
    } = input
    else {
        return Key::App(input);
    };
    if code == "Backslash" && m == mods::CTRL {
        return Key::Detach;
    }
    const OURS: [&str; 12] = [
        "KeyB",
        "KeyK",
        "KeyT",
        "KeyS",
        "KeyN",
        "KeyJ",
        "KeyO",
        "KeyW",
        "Slash",
        "Comma",
        "BracketLeft",
        "BracketRight",
    ];
    let ours =
        OURS.contains(&code.as_str()) || code.strip_prefix("Digit").is_some_and(|d| d != "0");
    let shifted = matches!(code.as_str(), "KeyW" | "KeyA" | "KeyD");
    let pane = code.starts_with("Arrow") || code == "Equal";
    let chord = match m {
        mods::ALT if ours => MOD(),
        // Ctrl+Alt reaches a terminal where the window's modifier cannot.
        _ if pane && m & !mods::SHIFT == mods::CTRL | mods::ALT => {
            MOD() | mods::ALT | (m & mods::SHIFT)
        }
        _ if m == mods::ALT | mods::SHIFT && shifted => MOD() | mods::SHIFT,
        _ => m,
    };
    if chord != m {
        return Key::App(Input::Key {
            code,
            mods: chord,
            text: None,
        });
    }
    Key::App(Input::Key {
        code,
        mods: m,
        text,
    })
}

/// Runs the TUI against the vornd serving `dir` until it is detached.
pub fn run(dir: &Path, opts: &Options) -> Result<(), String> {
    let woken = Arc::new(AtomicBool::new(true));
    let wake = {
        let woken = woken.clone();
        Arc::new(move || woken.store(true, Ordering::Release))
    };
    let ep = Endpoint::find(dir).map_err(|e| e.to_string())?;
    let mut app = App::connect(&ep, wake).map_err(|e| e.to_string())?;
    (app.chrome, app.traffic) = (false, false);
    app.load_panes(dir);
    app.set_data_dir(dir.to_owned());
    let mut ui = Ui::cells(&crate::ui_config(1.0));
    let until = Instant::now() + Duration::from_secs(5);
    while app.busy() && Instant::now() < until {
        app.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
    let term = Terminal::enter().map_err(|e| e.to_string())?;
    let mut out = Cells::terminal(term);
    let size = |c: &Cells| crate::ui::cells::units::px(c.buffer.cols, c.buffer.rows);
    app.frame(&mut ui, size(&out));
    if let Some(id) = &opts.focus {
        app.st.sidebar = false;
        app.focus_session(id);
    }
    let mut dirty = true;
    loop {
        let now = Instant::now();
        let due = ui.next_wake().is_some_and(|t| t <= now);
        if dirty || due || woken.swap(false, Ordering::AcqRel) || ui.animating() {
            app.frame(&mut ui, size(&out));
            out.present(&mut ui)?;
            dirty = false;
        }
        let Some(e) = Terminal::poll(Duration::from_millis(16)).map_err(|e| e.to_string())? else {
            continue;
        };
        dirty = true;
        match e {
            TermEvent::Input(input) => match route(input) {
                Key::Detach => break,
                Key::App(input) => app.send(&mut ui, input),
            },
            TermEvent::Paste(s) => app.send(&mut ui, Input::Commit(s)),
            TermEvent::Move(x, y) => ui.pointer_move(x, y),
            TermEvent::Down(x, y) => {
                ui.pointer_move(x, y);
                ui.pointer_down();
                app.events(&mut ui);
            }
            TermEvent::Up(x, y) => {
                ui.pointer_move(x, y);
                ui.pointer_up();
                app.events(&mut ui);
            }
            TermEvent::SecondaryDown(x, y) => {
                ui.pointer_move(x, y);
                ui.pointer_down_secondary();
                app.events(&mut ui);
            }
            TermEvent::Wheel(x, y, dy) => {
                ui.pointer_move(x, y);
                ui.wheel(dy);
                app.events(&mut ui);
            }
            TermEvent::Resize(cols, rows) => out.resize(cols, rows),
            TermEvent::Focus(f) => ui.set_window_focused(f),
        }
    }
    drop(out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &str, m: u16) -> Input {
        Input::Key {
            code: code.into(),
            mods: m,
            text: None,
        }
    }

    #[test]
    fn alt_chords_stand_in_for_the_window_shortcuts() {
        assert_eq!(route(key("Backslash", mods::CTRL)), Key::Detach);
        assert_eq!(
            route(key("Digit2", mods::ALT)),
            Key::App(key("Digit2", MOD()))
        );
        assert_eq!(route(key("KeyB", mods::ALT)), Key::App(key("KeyB", MOD())));
        assert_eq!(
            route(key("KeyB", mods::CTRL)),
            Key::App(key("KeyB", mods::CTRL))
        );
        assert_eq!(
            route(key("Digit0", mods::ALT)),
            Key::App(key("Digit0", mods::ALT))
        );
        assert_eq!(route(key("KeyK", mods::ALT)), Key::App(key("KeyK", MOD())));
        assert_eq!(
            route(key("KeyW", mods::ALT | mods::SHIFT)),
            Key::App(key("KeyW", MOD() | mods::SHIFT))
        );
        assert_eq!(
            route(key("KeyK", mods::ALT | mods::SHIFT)),
            Key::App(key("KeyK", mods::ALT | mods::SHIFT))
        );
        let pane = mods::CTRL | mods::ALT;
        assert_eq!(
            route(key("ArrowLeft", pane)),
            Key::App(key("ArrowLeft", MOD() | mods::ALT))
        );
        assert_eq!(
            route(key("ArrowUp", pane | mods::SHIFT)),
            Key::App(key("ArrowUp", MOD() | mods::ALT | mods::SHIFT))
        );
        assert_eq!(
            route(key("ArrowUp", mods::ALT)),
            Key::App(key("ArrowUp", mods::ALT))
        );
    }
}
