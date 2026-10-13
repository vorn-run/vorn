//! What a browser key event means to a terminal. Keys reach the app through
//! a hidden textarea, so the browser's IME and a phone's keyboard work; the
//! split below is what lets both coexist with a terminal's own keys:
//!
//! - `keydown` sends named keys (Enter, arrows, F-keys), chords (Ctrl+C,
//!   Alt+x) and plain characters typed on a hardware keyboard, and the page
//!   suppresses the browser's default for them.
//! - Everything else is left to the textarea: an IME's composition and a
//!   phone keyboard's inserts arrive as `composition*` and `input` events
//!   (on Android `keydown` names no key at all: "Unidentified", 229).
//! - The platform's paste chord is left to the browser so its `paste` event
//!   brings the clipboard without a permission prompt; copy and cut reach
//!   the app and also the browser, whose `copy` event takes the app's text.
//! - The browser keeps the chords it never gives a page (new tab, close tab).

use crate::input::mods;

/// A `keydown` as the page reads it.
#[derive(Debug, Clone, Copy)]
pub struct KeyDown<'a> {
    pub key: &'a str,
    pub code: &'a str,
    pub mods: u16,
    pub composing: bool,
    pub mac: bool,
}

/// What the page does with a `keydown`.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Send this key to the terminal and suppress the browser's default.
    Send {
        code: String,
        mods: u16,
        text: Option<String>,
    },
    /// Send it, and let the browser act on it too (copy, cut).
    Share { code: String, mods: u16 },
    /// Leave it to the textarea and the browser.
    Pass,
}

/// Decides what a `keydown` does.
pub fn on_keydown(k: KeyDown<'_>) -> KeyAction {
    if k.composing || k.key == "Unidentified" || k.key == "Process" || k.key == "Dead" {
        return KeyAction::Pass;
    }
    let primary = if k.mac {
        mods::SUPER
    } else {
        mods::CTRL | mods::SHIFT
    };
    // Cmd+C/V on a Mac, Ctrl+Shift+C/V elsewhere: the browser's own copy and
    // paste, whose events the page handles.
    if k.mods & primary == primary {
        match k.code {
            "KeyV" => return KeyAction::Pass,
            "KeyC" | "KeyX" => {
                return KeyAction::Share {
                    code: k.code.to_owned(),
                    mods: k.mods,
                }
            }
            _ => {}
        }
    }
    if k.mac
        && k.mods & mods::SUPER != 0
        && matches!(k.code, "KeyT" | "KeyW" | "KeyN" | "KeyQ" | "KeyR" | "KeyL")
    {
        return KeyAction::Pass;
    }
    let one_char = k.key.chars().nth(1).is_none() && !k.key.is_empty();
    if one_char {
        let chord = k.mods & (mods::CTRL | mods::ALT | mods::SUPER) != 0;
        let text = (!chord).then(|| k.key.to_owned());
        // Some layouts and virtual keyboards report no `code`.
        let code = if k.code.is_empty() {
            code_of_char(k.key)
        } else {
            k.code.to_owned()
        };
        return KeyAction::Send {
            code,
            mods: k.mods,
            text,
        };
    }
    if is_named(k.key) {
        let code = if k.code.is_empty() { k.key } else { k.code };
        return KeyAction::Send {
            code: code.to_owned(),
            mods: k.mods,
            text: None,
        };
    }
    KeyAction::Pass
}

fn is_named(key: &str) -> bool {
    matches!(
        key,
        "Enter"
            | "Tab"
            | "Backspace"
            | "Escape"
            | "Delete"
            | "Insert"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
            | "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
    ) || key
        .strip_prefix('F')
        .is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)))
}

fn code_of_char(key: &str) -> String {
    match key.chars().next() {
        Some(c @ 'a'..='z') | Some(c @ 'A'..='Z') => format!("Key{}", c.to_ascii_uppercase()),
        Some(c @ '0'..='9') => format!("Digit{c}"),
        Some(' ') => "Space".into(),
        _ => String::new(),
    }
}

/// What an `input` event the textarea saw means, by its `inputType`: the
/// keys a phone keyboard sends this way.
#[derive(Debug, PartialEq, Eq)]
pub enum Inserted {
    Text(String),
    Key(&'static str),
    Nothing,
}

pub fn on_input(input_type: &str, data: Option<&str>, composing: bool) -> Inserted {
    if composing {
        return Inserted::Nothing;
    }
    match input_type {
        "insertText" | "insertReplacementText" => data
            .filter(|d| !d.is_empty())
            .map_or(Inserted::Nothing, |d| Inserted::Text(d.to_owned())),
        "insertLineBreak" | "insertParagraph" => Inserted::Key("Enter"),
        "deleteContentBackward" => Inserted::Key("Backspace"),
        "deleteContentForward" => Inserted::Key("Delete"),
        // insertCompositionText and insertFromComposition: the composition
        // events carry them.
        _ => Inserted::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kd(key: &str, code: &str, mods: u16) -> KeyAction {
        on_keydown(KeyDown {
            key,
            code,
            mods,
            composing: false,
            mac: true,
        })
    }

    fn send(code: &str, mods: u16, text: Option<&str>) -> KeyAction {
        KeyAction::Send {
            code: code.into(),
            mods,
            text: text.map(Into::into),
        }
    }

    #[test]
    fn hardware_keys() {
        assert_eq!(kd("a", "KeyA", 0), send("KeyA", 0, Some("a")));
        assert_eq!(
            kd("A", "KeyA", mods::SHIFT),
            send("KeyA", mods::SHIFT, Some("A"))
        );
        assert_eq!(kd("c", "KeyC", mods::CTRL), send("KeyC", mods::CTRL, None));
        assert_eq!(kd("Enter", "Enter", 0), send("Enter", 0, None));
        assert_eq!(kd("F5", "F5", 0), send("F5", 0, None));
        assert_eq!(kd("é", "", 0), send("", 0, Some("é")));
    }

    #[test]
    fn left_to_the_browser() {
        assert_eq!(kd("v", "KeyV", mods::SUPER), KeyAction::Pass);
        assert_eq!(kd("t", "KeyT", mods::SUPER), KeyAction::Pass);
        assert_eq!(
            kd("c", "KeyC", mods::SUPER),
            KeyAction::Share {
                code: "KeyC".into(),
                mods: mods::SUPER
            }
        );
        assert_eq!(
            kd("k", "KeyK", mods::SUPER),
            send("KeyK", mods::SUPER, None)
        );
        assert_eq!(kd("Shift", "ShiftLeft", mods::SHIFT), KeyAction::Pass);
        assert_eq!(kd("Unidentified", "", 0), KeyAction::Pass);
        let composing = KeyDown {
            key: "n",
            code: "KeyN",
            mods: 0,
            composing: true,
            mac: false,
        };
        assert_eq!(on_keydown(composing), KeyAction::Pass);
        let paste = KeyDown {
            key: "V",
            code: "KeyV",
            mods: mods::CTRL | mods::SHIFT,
            composing: false,
            mac: false,
        };
        assert_eq!(on_keydown(paste), KeyAction::Pass);
    }

    #[test]
    fn phone_keyboard_inserts() {
        assert_eq!(
            on_input("insertText", Some("hi"), false),
            Inserted::Text("hi".into())
        );
        assert_eq!(
            on_input("deleteContentBackward", None, false),
            Inserted::Key("Backspace")
        );
        assert_eq!(
            on_input("insertLineBreak", None, false),
            Inserted::Key("Enter")
        );
        assert_eq!(on_input("insertText", Some("に"), true), Inserted::Nothing);
        assert_eq!(
            on_input("insertFromComposition", Some("日本"), false),
            Inserted::Nothing
        );
    }
}
