//! Input events from grid clients, encoded against the terminal's modes at
//! the moment they are dequeued (Terminal State Protocol §10): a key typed
//! after a program switched to application cursor keys is encoded as that
//! program expects once its switch has been parsed.
//!
//! libghostty-vt's encoders take their options from the terminal (cursor
//! and keypad modes, Kitty flags, modifyOtherKeys, mouse tracking and
//! format), so the state kept here is the encoders, the buttons held and the
//! cell last reported. A paste
//! that would run a command without bracketed paste is held for the client
//! to confirm.

use libghostty_vt::focus;
use libghostty_vt::key::{self, Mods};
use libghostty_vt::mouse::{self, EncoderSize, Position};
use libghostty_vt::paste;
use libghostty_vt::terminal::{Mode, Terminal};
use vorn_term_proto::msg::{mods, InputEvent, KeyAction, KeyCode, MouseAction};

use crate::Error;

type Term = Terminal<'static, 'static>;

/// What an event comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Encoded {
    /// Bytes for the program.
    Bytes(Vec<u8>),
    /// A paste that needs the client's confirmation first.
    Confirm,
    /// Nothing to send in the terminal's current modes.
    Nothing,
}

/// Ghostty's key and mouse encoders, kept between events.
pub struct InputEncoder {
    key: key::Encoder<'static>,
    key_event: key::Event<'static>,
    mouse: mouse::Encoder<'static>,
    mouse_event: mouse::Event<'static>,
    /// Buttons pressed and not yet released, one bit each, so drags out of the pane still report.
    held: u16,
    /// The cell of the last mouse report. Kept here because Ghostty's own copy is cleared by
    /// every options or size update, which each event needs.
    last_cell: Option<(u32, u32)>,
}

impl std::fmt::Debug for InputEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InputEncoder")
    }
}

impl InputEncoder {
    pub fn new() -> Result<Self, Error> {
        Ok(InputEncoder {
            key: key::Encoder::new()?,
            key_event: key::Event::new()?,
            mouse: mouse::Encoder::new()?,
            mouse_event: mouse::Event::new()?,
            held: 0,
            last_cell: None,
        })
    }

    /// `event` as the program expects it, given the modes `t` is in now.
    pub fn encode(&mut self, t: &Term, event: &InputEvent) -> Result<Encoded, Error> {
        let mut out = Vec::new();
        match event {
            InputEvent::Key {
                action,
                key,
                mods,
                consumed_mods,
                text,
                unshifted,
                composing,
            } => {
                let mods = tab_mods(t, *key, *mods)?;
                let ev = &mut self.key_event;
                ev.set_action(match action {
                    KeyAction::Press => key::Action::Press,
                    KeyAction::Repeat => key::Action::Repeat,
                    KeyAction::Release => key::Action::Release,
                })
                .set_key(ghostty_key(*key))
                .set_mods(ghostty_mods(mods))
                .set_consumed_mods(ghostty_mods(*consumed_mods))
                .set_composing(*composing)
                .set_utf8(text.clone());
                ev.set_unshifted_codepoint(unshifted.unwrap_or('\0'));
                self.key.set_options_from_terminal(t);
                self.key.encode_to_vec(&self.key_event, &mut out)?;
            }
            InputEvent::Text { utf8 } => out.extend_from_slice(utf8.as_bytes()),
            InputEvent::Paste { utf8, confirmed } => {
                let bracketed = t.mode(Mode::BRACKETED_PASTE)?;
                if !bracketed && !confirmed && !paste::is_safe(utf8) {
                    return Ok(Encoded::Confirm);
                }
                let mut data = utf8.clone().into_bytes();
                // The bracketing adds twelve bytes at most.
                let mut buf = vec![0; data.len() + 16];
                let n = paste::encode(&mut data, bracketed, &mut buf)?;
                buf.truncate(n);
                out = buf;
            }
            InputEvent::Mouse {
                action,
                button,
                mods,
                x,
                y,
            } => {
                let bit = button
                    .filter(|b| !WHEEL.contains(b))
                    .map_or(0, |b| 1u16 << (b & 15));
                let action = match action {
                    MouseAction::Press => {
                        self.held |= bit;
                        mouse::Action::Press
                    }
                    MouseAction::Release => {
                        // A release without a button lets go of all of them.
                        self.held &= if bit == 0 { 0 } else { !bit };
                        mouse::Action::Release
                    }
                    MouseAction::Motion => mouse::Action::Motion,
                };
                self.mouse_at(
                    t,
                    action,
                    button.map(ghostty_button),
                    *mods,
                    *x,
                    *y,
                    &mut out,
                )?;
            }
            InputEvent::Wheel { dy, mods, x, y, .. } => {
                // A program tracking the mouse gets wheel buttons; otherwise
                // the client scrolls its own mirror and nothing is sent.
                if t.is_mouse_tracking()? && *dy != 0.0 {
                    let button = if *dy < 0.0 {
                        mouse::Button::Four
                    } else {
                        mouse::Button::Five
                    };
                    self.mouse_at(
                        t,
                        mouse::Action::Press,
                        Some(button),
                        *mods,
                        *x,
                        *y,
                        &mut out,
                    )?;
                }
            }
            InputEvent::Focus { focused } => {
                if t.mode(Mode::FOCUS_EVENT)? {
                    let ev = if *focused {
                        focus::Event::Gained
                    } else {
                        focus::Event::Lost
                    };
                    let mut buf = [0u8; 8];
                    let n = ev.encode(&mut buf)?;
                    out.extend_from_slice(&buf[..n]);
                }
            }
            InputEvent::Raw { bytes } => out.extend_from_slice(bytes),
        }
        Ok(if out.is_empty() {
            Encoded::Nothing
        } else {
            Encoded::Bytes(out)
        })
    }

    /// A mouse event at cell `(x, y)`: positions arrive in cells and are
    /// scaled by the cell's pixel size, which pixel reports (mode 1016) carry.
    #[allow(clippy::too_many_arguments)]
    fn mouse_at(
        &mut self,
        t: &Term,
        action: mouse::Action,
        button: Option<mouse::Button>,
        mods: u16,
        x: f32,
        y: f32,
        out: &mut Vec<u8>,
    ) -> Result<(), Error> {
        let (cols, rows) = (u32::from(t.cols()?), u32::from(t.rows()?));
        let cell_width = (t.width_px()? / cols.max(1)).max(1);
        let cell_height = (t.height_px()? / rows.max(1)).max(1);
        // Ghostty's rule: motion is reported once per cell unless it counts pixels.
        let cell = (clamp_cell(x, cols), clamp_cell(y, rows));
        if action == mouse::Action::Motion
            && self.last_cell == Some(cell)
            && !t.mode(Mode::SGR_PIXELS_MOUSE)?
        {
            return Ok(());
        }
        self.mouse_event
            .set_action(action)
            .set_button(button)
            .set_mods(ghostty_mods(mods))
            .set_position(Position {
                x: x * cell_width as f32,
                y: y * cell_height as f32,
            });
        self.mouse
            .set_options_from_terminal(t)
            .set_any_button_pressed(self.held != 0)
            .set_size(EncoderSize {
                screen_width: cols * cell_width,
                screen_height: rows * cell_height,
                cell_width,
                cell_height,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            });
        let before = out.len();
        self.mouse.encode_to_vec(&self.mouse_event, out)?;
        if out.len() > before {
            self.last_cell = Some(cell);
        }
        Ok(())
    }
}

/// The cell a position in cells falls in, held inside the pane as Ghostty holds it.
/// Tab's modifiers as xterm.js sends them, Ctrl and Alt dropped (Shift
/// still makes it CSI Z), unless the program asked for Kitty key reports.
fn tab_mods(t: &Term, key: KeyCode, m: u16) -> Result<u16, Error> {
    if key.name() != "Tab" || !t.kitty_keyboard_flags()?.is_empty() {
        return Ok(m);
    }
    Ok(m & !(mods::CTRL | mods::ALT | mods::CTRL_RIGHT | mods::ALT_RIGHT))
}

fn clamp_cell(at: f32, n: u32) -> u32 {
    (at.max(0.0) as u32).min(n.saturating_sub(1))
}

/// The wire's key code as Ghostty's: both follow the W3C UI Events order,
/// which a test checks name by name.
pub fn ghostty_key(code: KeyCode) -> key::Key {
    key::Key::try_from(i32::from(code.0)).unwrap_or(key::Key::Unidentified)
}

/// The wire's wheel buttons, which are never held.
const WHEEL: [u8; 4] = [4, 5, 6, 7];

fn ghostty_mods(m: u16) -> Mods {
    let mut out = Mods::empty();
    for (bit, g) in [
        (mods::SHIFT, Mods::SHIFT),
        (mods::ALT, Mods::ALT),
        (mods::CTRL, Mods::CTRL),
        (mods::SUPER, Mods::SUPER),
        (mods::CAPS_LOCK, Mods::CAPS_LOCK),
        (mods::NUM_LOCK, Mods::NUM_LOCK),
        (mods::SHIFT_RIGHT, Mods::SHIFT_SIDE),
        (mods::ALT_RIGHT, Mods::ALT_SIDE),
        (mods::CTRL_RIGHT, Mods::CTRL_SIDE),
        (mods::SUPER_RIGHT, Mods::SUPER_SIDE),
    ] {
        if m & bit != 0 {
            out |= g;
        }
    }
    out
}

fn ghostty_button(b: u8) -> mouse::Button {
    match b {
        1 => mouse::Button::Left,
        2 => mouse::Button::Middle,
        3 => mouse::Button::Right,
        4 => mouse::Button::Four,
        5 => mouse::Button::Five,
        6 => mouse::Button::Six,
        7 => mouse::Button::Seven,
        8 => mouse::Button::Eight,
        9 => mouse::Button::Nine,
        10 => mouse::Button::Ten,
        11 => mouse::Button::Eleven,
        _ => mouse::Button::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_term_proto::msg::KEY_NAMES;

    #[test]
    fn wire_key_codes_name_the_same_keys_as_ghostty() {
        for (i, name) in KEY_NAMES.iter().enumerate() {
            let k = ghostty_key(KeyCode(i as u16));
            assert_eq!(format!("{k:?}"), *name);
        }
        assert_eq!(ghostty_key(KeyCode(60_000)), key::Key::Unidentified);
    }

    fn key(name: &str, mods: u16, text: Option<&str>) -> InputEvent {
        InputEvent::Key {
            action: KeyAction::Press,
            key: KeyCode::from_name(name).unwrap(),
            mods,
            consumed_mods: 0,
            text: text.map(str::to_owned),
            unshifted: text.and_then(|t| t.chars().next()),
            composing: false,
        }
    }

    fn bytes(e: Encoded) -> Vec<u8> {
        match e {
            Encoded::Bytes(b) => b,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tab_and_its_variants_encode_as_xterm_sends_them() {
        let t = Term::new(20, 5).unwrap();
        let mut enc = InputEncoder::new().unwrap();
        let mut send = |t: &Term, m, text| bytes(enc.encode(t, &key("Tab", m, text)).unwrap());
        assert_eq!(send(&t, 0, Some("\t")), b"\t");
        assert_eq!(send(&t, mods::SHIFT, None), b"\x1b[Z");
        assert_eq!(send(&t, mods::CTRL, None), b"\t");
        assert_eq!(send(&t, mods::CTRL | mods::SHIFT, None), b"\x1b[Z");
        assert_eq!(send(&t, mods::ALT, None), b"\t");
        // A program that asked for Kitty key reports still tells Ctrl+Tab apart.
        let mut t = t;
        t.vt_write(b"\x1b[>1u");
        assert_eq!(send(&t, mods::CTRL, None), b"\x1b[9;5u");
    }

    #[test]
    fn pixel_reports_use_the_cell_size_and_drags_report_outside_the_pane() {
        let mut t = Term::new(20, 5).unwrap();
        t.resize(20, 5, 10, 20).unwrap();
        let mut enc = InputEncoder::new().unwrap();
        let mouse = |action, button, x, y| InputEvent::Mouse {
            action,
            button,
            mods: 0,
            x,
            y,
        };
        // Button-event tracking with SGR pixel coordinates.
        t.vt_write(b"\x1b[?1002h\x1b[?1016h");
        let press = mouse(MouseAction::Press, Some(1), 2.5, 1.0);
        assert_eq!(bytes(enc.encode(&t, &press).unwrap()), b"\x1b[<0;25;20M");
        // A drag past the right edge still reports while the button is held.
        let out = mouse(MouseAction::Motion, Some(1), 25.0, 1.0);
        assert_eq!(bytes(enc.encode(&t, &out).unwrap()), b"\x1b[<32;250;20M");
        let release = mouse(MouseAction::Release, Some(1), 25.0, 1.0);
        assert_eq!(bytes(enc.encode(&t, &release).unwrap()), b"\x1b[<0;250;20m");
        let out = mouse(MouseAction::Motion, None, 26.0, 1.0);
        assert_eq!(enc.encode(&t, &out).unwrap(), Encoded::Nothing);
        // In cell coordinates, motion within the cell last reported is dropped.
        t.vt_write(b"\x1b[?1016l\x1b[?1006h");
        let press = mouse(MouseAction::Press, Some(1), 3.0, 1.0);
        assert_eq!(bytes(enc.encode(&t, &press).unwrap()), b"\x1b[<0;4;2M");
        let same = mouse(MouseAction::Motion, Some(1), 3.5, 1.2);
        assert_eq!(enc.encode(&t, &same).unwrap(), Encoded::Nothing);
        let next = mouse(MouseAction::Motion, Some(1), 4.0, 1.0);
        assert_eq!(bytes(enc.encode(&t, &next).unwrap()), b"\x1b[<32;5;2M");
    }

    #[test]
    fn pastes_that_would_run_a_command_wait_for_confirmation() {
        let mut t = Term::new(20, 5).unwrap();
        let mut enc = InputEncoder::new().unwrap();
        let paste = |confirmed| InputEvent::Paste {
            utf8: "rm -rf x\n".into(),
            confirmed,
        };
        assert_eq!(enc.encode(&t, &paste(false)).unwrap(), Encoded::Confirm);
        assert_eq!(bytes(enc.encode(&t, &paste(true)).unwrap()), b"rm -rf x\r");
        t.vt_write(b"\x1b[?2004h");
        assert_eq!(
            bytes(enc.encode(&t, &paste(false)).unwrap()),
            b"\x1b[200~rm -rf x\n\x1b[201~"
        );
        // The wheel scrolls the cell under the pointer, in SGR coordinates
        // counted from 1.
        t.vt_write(b"\x1b[?1000h\x1b[?1006h");
        let wheel = InputEvent::Wheel {
            dx: 0.0,
            dy: -1.0,
            mods: 0,
            x: 5.0,
            y: 3.0,
        };
        assert_eq!(bytes(enc.encode(&t, &wheel).unwrap()), b"\x1b[<64;6;4M");
        t.vt_write(b"\x1b[?1006l\x1b[?1000l");
        // Focus reports only when the program asked for them.
        let focus = InputEvent::Focus { focused: true };
        assert_eq!(enc.encode(&t, &focus).unwrap(), Encoded::Nothing);
        t.vt_write(b"\x1b[?1004h");
        assert_eq!(bytes(enc.encode(&t, &focus).unwrap()), b"\x1b[I");
        assert_eq!(
            bytes(enc.encode(&t, &key("Enter", 0, Some("\r"))).unwrap()),
            b"\r"
        );
    }
}
