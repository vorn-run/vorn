//! Input as the app sees it: key presses with the W3C `code` name and the
//! text they type, and IME composition. A window translates winit's events
//! into these; benches and tests inject them directly, so both go through
//! the same app handler.

use winit::event::{ElementState, Ime, KeyEvent, Modifiers, WindowEvent};
use winit::keyboard::{ModifiersState, PhysicalKey};

/// Modifier bits, the terminal wire's layout.
pub mod mods {
    pub const SHIFT: u16 = 1;
    pub const ALT: u16 = 2;
    pub const CTRL: u16 = 4;
    pub const SUPER: u16 = 8;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Key {
        code: String,
        mods: u16,
        text: Option<String>,
    },
    /// IME composition in progress; empty when it ends without a commit.
    Preedit(String),
    /// Text the IME committed.
    Commit(String),
}

impl Input {
    /// A key that types `ch`, as a US layout would send it.
    pub fn char(ch: char) -> Input {
        let code = match ch {
            'a'..='z' | 'A'..='Z' => format!("Key{}", ch.to_ascii_uppercase()),
            '0'..='9' => format!("Digit{ch}"),
            ' ' => "Space".to_owned(),
            _ => String::new(),
        };
        Input::Key {
            code,
            mods: if ch.is_ascii_uppercase() {
                mods::SHIFT
            } else {
                0
            },
            text: Some(ch.to_string()),
        }
    }

    pub fn named(code: &str) -> Input {
        Input::Key {
            code: code.to_owned(),
            mods: 0,
            text: None,
        }
    }
}

pub fn mods_of(m: &Modifiers) -> u16 {
    let s: ModifiersState = m.state();
    let mut out = 0;
    if s.shift_key() {
        out |= mods::SHIFT;
    }
    if s.alt_key() {
        out |= mods::ALT;
    }
    if s.control_key() {
        out |= mods::CTRL;
    }
    if s.super_key() {
        out |= mods::SUPER;
    }
    out
}

/// The app-level input for a winit window event, if it is one.
pub fn from_winit(ev: &WindowEvent, mods: u16) -> Option<Input> {
    match ev {
        WindowEvent::KeyboardInput {
            event:
                KeyEvent {
                    physical_key,
                    state: ElementState::Pressed,
                    text,
                    ..
                },
            is_synthetic: false,
            ..
        } => {
            let code = match physical_key {
                PhysicalKey::Code(c) => format!("{c:?}"),
                PhysicalKey::Unidentified(_) => String::new(),
            };
            Some(Input::Key {
                code,
                mods,
                text: text.as_ref().map(|t| t.to_string()),
            })
        }
        WindowEvent::Ime(ime) => from_ime(ime),
        _ => None,
    }
}

pub fn from_ime(ime: &Ime) -> Option<Input> {
    match ime {
        Ime::Preedit(s, _) => Some(Input::Preedit(s.clone())),
        Ime::Commit(s) => Some(Input::Commit(s.clone())),
        Ime::Disabled => Some(Input::Preedit(String::new())),
        Ime::Enabled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chars_carry_code_and_shift() {
        assert_eq!(
            Input::char('Q'),
            Input::Key {
                code: "KeyQ".into(),
                mods: mods::SHIFT,
                text: Some("Q".into())
            }
        );
    }

    #[test]
    fn ime_events_map_to_composition() {
        let pre = Ime::Preedit("にほんご".into(), Some((0, 12)));
        assert_eq!(from_ime(&pre), Some(Input::Preedit("にほんご".into())));
        let commit = Ime::Commit("日本語".into());
        assert_eq!(from_ime(&commit), Some(Input::Commit("日本語".into())));
    }
}
