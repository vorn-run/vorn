//! A cells frame on a real terminal through crossterm: raw mode, the
//! alternate screen, mouse reports and the kitty keyboard protocol where the
//! terminal has it; frames written as a diff inside synchronized output.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{
    self, Event as CtEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, MouseButton, MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::style::{
    Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::{cursor, execute, queue, terminal};

use super::{ansi16, ansi256, attr, Buffer, Cell, CursorShape, Depth, Sym};
use crate::input::{mods, Input};

/// What the terminal reported, in vornui's terms. Positions are cell centres.
#[derive(Debug, Clone, PartialEq)]
pub enum TermEvent {
    Input(Input),
    Move(f32, f32),
    Down(f32, f32),
    Up(f32, f32),
    SecondaryDown(f32, f32),
    /// Wheel in rows; positive scrolls down.
    Wheel(f32, f32, f32),
    Paste(String),
    Resize(u16, u16),
    Focus(bool),
}

/// The terminal, owned while the TUI runs; dropping it gives it back as it was.
pub struct Terminal {
    out: io::Stdout,
    depth: Depth,
    prev: Option<Buffer>,
    kitty: bool,
    active: bool,
    /// The terminal takes OSC 22 pointer shapes.
    shapes: bool,
    /// The pointer shape last asked for.
    pointer: &'static str,
}

impl Terminal {
    /// Takes over the terminal.
    pub fn enter() -> io::Result<Terminal> {
        let mut out = io::stdout();
        terminal::enable_raw_mode()?;
        let kitty = terminal::supports_keyboard_enhancement().unwrap_or(false);
        execute!(
            out,
            terminal::EnterAlternateScreen,
            event::EnableMouseCapture,
            event::EnableBracketedPaste,
            event::EnableFocusChange,
            cursor::Hide,
            terminal::Clear(terminal::ClearType::All)
        )?;
        if kitty {
            execute!(
                out,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                )
            )?;
        }
        Ok(Terminal {
            out,
            depth: Depth::from_env(),
            prev: None,
            kitty,
            active: true,
            shapes: takes_pointer_shapes(
                std::env::var("TERM_PROGRAM").ok().as_deref(),
                std::env::var("TERM").ok().as_deref(),
            ),
            pointer: "default",
        })
    }

    pub fn size() -> io::Result<(u16, u16)> {
        terminal::size()
    }

    pub fn depth(&self) -> Depth {
        self.depth
    }

    pub fn kitty(&self) -> bool {
        self.kitty
    }

    /// Forgets the last frame, so the next one is written whole.
    pub fn invalidate(&mut self) {
        self.prev = None;
    }

    /// Writes the cells of `frame` that changed since the last one, then
    /// places the cursor, all in one synchronized update. Answers how many
    /// cells it wrote.
    pub fn draw(&mut self, frame: &Buffer) -> io::Result<usize> {
        let mut w = io::BufWriter::with_capacity(64 * 1024, &mut self.out);
        queue!(w, terminal::BeginSynchronizedUpdate, cursor::Hide)?;
        let mut written = 0;
        let mut pen: Option<(u16, u16)> = None;
        let mut style: Option<Cell> = None;
        for (x, y, c) in frame.diff(self.prev.as_ref()) {
            if c.sym == Sym::TAIL {
                continue;
            }
            if pen != Some((x, y)) {
                queue!(w, cursor::MoveTo(x, y))?;
            }
            let mut s = *c;
            s.sym = Sym::SPACE;
            if style != Some(s) {
                write_style(&mut w, c, self.depth)?;
                style = Some(s);
            }
            queue!(w, Print(c.sym.as_str()))?;
            let wide = super::width(c.sym.as_str()).max(1) as u16;
            pen = Some((x + wide, y));
            written += 1;
        }
        queue!(w, SetAttribute(Attribute::Reset))?;
        if let Some(cur) = frame.cursor {
            let shape = match cur.shape {
                CursorShape::Block => cursor::SetCursorStyle::SteadyBlock,
                CursorShape::Bar => cursor::SetCursorStyle::SteadyBar,
                CursorShape::Underline => cursor::SetCursorStyle::SteadyUnderScore,
            };
            queue!(w, cursor::MoveTo(cur.col, cur.row), shape, cursor::Show)?;
        }
        queue!(w, terminal::EndSynchronizedUpdate)?;
        w.flush()?;
        drop(w);
        self.prev = Some(frame.clone());
        Ok(written)
    }

    /// Shows `cursor` as the mouse pointer, where the terminal takes OSC 22;
    /// elsewhere controls show they can be clicked by their fill instead.
    pub fn set_pointer(&mut self, cursor: crate::Cursor) -> io::Result<()> {
        let name = match cursor {
            crate::Cursor::Default => "default",
            crate::Cursor::Pointer => "pointer",
            crate::Cursor::Text => "text",
            crate::Cursor::ColResize => "ew-resize",
            crate::Cursor::RowResize => "ns-resize",
        };
        if !self.shapes || name == self.pointer {
            return Ok(());
        }
        self.pointer = name;
        write!(self.out, "\x1b]22;{name}\x1b\\")?;
        self.out.flush()
    }

    /// The next event within `timeout`, if any.
    pub fn poll(timeout: Duration) -> io::Result<Option<TermEvent>> {
        if !event::poll(timeout)? {
            return Ok(None);
        }
        Ok(map_event(event::read()?))
    }

    /// Gives the terminal back.
    pub fn leave(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        let _ = self.set_pointer(crate::Cursor::Default);
        if self.kitty {
            execute!(self.out, PopKeyboardEnhancementFlags)?;
        }
        execute!(
            self.out,
            SetAttribute(Attribute::Reset),
            cursor::SetCursorStyle::DefaultUserShape,
            cursor::Show,
            event::DisableFocusChange,
            event::DisableBracketedPaste,
            event::DisableMouseCapture,
            terminal::LeaveAlternateScreen
        )?;
        terminal::disable_raw_mode()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

/// Whether the terminal named by `TERM_PROGRAM` and `TERM` changes the mouse
/// pointer on OSC 22. Terminal.app and iTerm2 do not; the rest are left alone too,
/// since an unknown OSC can print.
fn takes_pointer_shapes(program: Option<&str>, term: Option<&str>) -> bool {
    let program = program.unwrap_or_default().to_ascii_lowercase();
    let term = term.unwrap_or_default();
    matches!(program.as_str(), "ghostty" | "wezterm" | "kitty")
        || term.starts_with("xterm-kitty")
        || term.starts_with("xterm-ghostty")
        || term.starts_with("foot")
        || term == "wezterm"
}

fn color(c: [u8; 3], depth: Depth) -> Color {
    match depth {
        Depth::True => Color::Rgb {
            r: c[0],
            g: c[1],
            b: c[2],
        },
        Depth::Ansi256 => Color::AnsiValue(ansi256(c)),
        Depth::Ansi16 => Color::AnsiValue(ansi16(c)),
    }
}

fn write_style(w: &mut impl Write, c: &Cell, depth: Depth) -> io::Result<()> {
    queue!(w, SetAttribute(Attribute::Reset))?;
    for (bit, at) in [
        (attr::BOLD, Attribute::Bold),
        (attr::DIM, Attribute::Dim),
        (attr::ITALIC, Attribute::Italic),
        (attr::UNDERLINE, Attribute::Underlined),
        (attr::STRIKE, Attribute::CrossedOut),
        (attr::INVERSE, Attribute::Reverse),
    ] {
        if c.attr & bit != 0 {
            queue!(w, SetAttribute(at))?;
        }
    }
    queue!(
        w,
        SetForegroundColor(color(c.fg, depth)),
        SetBackgroundColor(color(c.bg, depth))
    )
}

fn mods_of(m: KeyModifiers) -> u16 {
    let mut out = 0;
    if m.contains(KeyModifiers::SHIFT) {
        out |= mods::SHIFT;
    }
    if m.contains(KeyModifiers::ALT) {
        out |= mods::ALT;
    }
    if m.contains(KeyModifiers::CONTROL) {
        out |= mods::CTRL;
    }
    if m.contains(KeyModifiers::SUPER) || m.contains(KeyModifiers::META) {
        out |= mods::SUPER;
    }
    out
}

/// The W3C `code` a US layout gives `ch`, and whether Shift typed it.
fn code_of_char(ch: char) -> (String, bool) {
    const SHIFTED: &str = "!@#$%^&*()_+{}|:\"<>?~";
    const SHIFTED_CODES: [&str; 21] = [
        "Digit1",
        "Digit2",
        "Digit3",
        "Digit4",
        "Digit5",
        "Digit6",
        "Digit7",
        "Digit8",
        "Digit9",
        "Digit0",
        "Minus",
        "Equal",
        "BracketLeft",
        "BracketRight",
        "Backslash",
        "Semicolon",
        "Quote",
        "Comma",
        "Period",
        "Slash",
        "Backquote",
    ];
    const PLAIN: &str = "-=[]\\;',./` ";
    const PLAIN_CODES: [&str; 12] = [
        "Minus",
        "Equal",
        "BracketLeft",
        "BracketRight",
        "Backslash",
        "Semicolon",
        "Quote",
        "Comma",
        "Period",
        "Slash",
        "Backquote",
        "Space",
    ];
    match ch {
        'a'..='z' => (format!("Key{}", ch.to_ascii_uppercase()), false),
        'A'..='Z' => (format!("Key{ch}"), true),
        '0'..='9' => (format!("Digit{ch}"), false),
        _ => {
            if let Some(i) = SHIFTED.chars().position(|c| c == ch) {
                return (SHIFTED_CODES[i].to_owned(), true);
            }
            if let Some(i) = PLAIN.chars().position(|c| c == ch) {
                return (PLAIN_CODES[i].to_owned(), false);
            }
            (String::new(), false)
        }
    }
}

/// A crossterm key as vornui's [`Input::Key`].
pub fn map_key(k: &KeyEvent) -> Option<Input> {
    if k.kind == KeyEventKind::Release {
        return None;
    }
    let mut m = mods_of(k.modifiers);
    let (code, text) = match k.code {
        // Legacy terminals send Ctrl+\ and Ctrl+] as bytes crossterm reads as Ctrl+4 and Ctrl+5.
        KeyCode::Char('4') if m == mods::CTRL => ("Backslash".to_owned(), None),
        KeyCode::Char('5') if m == mods::CTRL => ("BracketRight".to_owned(), None),
        KeyCode::Char(ch) => {
            let (code, shifted) = code_of_char(ch);
            if shifted || ch.is_ascii_uppercase() {
                m |= mods::SHIFT;
            }
            // Control and Alt chords type nothing; the terminal encodes them from the code.
            let typed = m & (mods::CTRL | mods::ALT | mods::SUPER) == 0;
            let ch = if m & mods::SHIFT != 0 && ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            (code, typed.then(|| ch.to_string()))
        }
        KeyCode::BackTab => {
            m |= mods::SHIFT;
            ("Tab".to_owned(), None)
        }
        KeyCode::F(n) => (format!("F{n}"), None),
        other => {
            let name = match other {
                KeyCode::Enter => "Enter",
                KeyCode::Tab => "Tab",
                KeyCode::Backspace => "Backspace",
                KeyCode::Esc => "Escape",
                KeyCode::Left => "ArrowLeft",
                KeyCode::Right => "ArrowRight",
                KeyCode::Up => "ArrowUp",
                KeyCode::Down => "ArrowDown",
                KeyCode::Home => "Home",
                KeyCode::End => "End",
                KeyCode::PageUp => "PageUp",
                KeyCode::PageDown => "PageDown",
                KeyCode::Delete => "Delete",
                KeyCode::Insert => "Insert",
                _ => return None,
            };
            (name.to_owned(), None)
        }
    };
    Some(Input::Key {
        code,
        mods: m,
        text,
    })
}

fn mouse(m: &MouseEvent) -> Option<TermEvent> {
    let (x, y) = (f32::from(m.column) + 0.5, f32::from(m.row) + 0.5);
    Some(match m.kind {
        MouseEventKind::Down(MouseButton::Left) => TermEvent::Down(x, y),
        MouseEventKind::Down(MouseButton::Right) => TermEvent::SecondaryDown(x, y),
        MouseEventKind::Up(MouseButton::Left) => TermEvent::Up(x, y),
        MouseEventKind::Moved | MouseEventKind::Drag(_) => TermEvent::Move(x, y),
        MouseEventKind::ScrollDown => TermEvent::Wheel(x, y, 3.0),
        MouseEventKind::ScrollUp => TermEvent::Wheel(x, y, -3.0),
        _ => return None,
    })
}

/// A crossterm event as a [`TermEvent`].
pub fn map_event(e: CtEvent) -> Option<TermEvent> {
    match e {
        CtEvent::Key(k) => map_key(&k).map(TermEvent::Input),
        CtEvent::Mouse(m) => mouse(&m),
        CtEvent::Paste(s) => Some(TermEvent::Paste(s)),
        CtEvent::Resize(c, r) => Some(TermEvent::Resize(c, r)),
        CtEvent::FocusGained => Some(TermEvent::Focus(true)),
        CtEvent::FocusLost => Some(TermEvent::Focus(false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, m: KeyModifiers) -> Option<Input> {
        map_key(&KeyEvent::new(code, m))
    }

    #[test]
    fn maps_keys_to_w3c_codes() {
        assert_eq!(
            key(KeyCode::Char('a'), KeyModifiers::NONE),
            Some(Input::char('a'))
        );
        assert_eq!(
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(Input::Key {
                code: "KeyC".into(),
                mods: mods::CTRL,
                text: None
            })
        );
        assert_eq!(
            key(KeyCode::Char('?'), KeyModifiers::NONE),
            Some(Input::Key {
                code: "Slash".into(),
                mods: mods::SHIFT,
                text: Some("?".into())
            })
        );
        assert_eq!(
            key(KeyCode::Char('4'), KeyModifiers::CONTROL),
            Some(Input::Key {
                code: "Backslash".into(),
                mods: mods::CTRL,
                text: None
            })
        );
        assert_eq!(
            key(KeyCode::Up, KeyModifiers::NONE),
            Some(Input::named("ArrowUp"))
        );
        assert_eq!(
            key(KeyCode::BackTab, KeyModifiers::SHIFT),
            Some(Input::Key {
                code: "Tab".into(),
                mods: mods::SHIFT,
                text: None
            })
        );
    }

    #[test]
    fn pointer_shapes_go_only_to_terminals_that_take_them() {
        assert!(takes_pointer_shapes(Some("ghostty"), Some("xterm-ghostty")));
        assert!(takes_pointer_shapes(Some("WezTerm"), None));
        assert!(takes_pointer_shapes(None, Some("xterm-kitty")));
        assert!(takes_pointer_shapes(None, Some("foot-extra")));
        assert!(!takes_pointer_shapes(
            Some("Apple_Terminal"),
            Some("xterm-256color")
        ));
        assert!(!takes_pointer_shapes(
            Some("iTerm.app"),
            Some("xterm-256color")
        ));
        assert!(!takes_pointer_shapes(None, None));
    }
}
