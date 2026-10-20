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

/// What the terminal reported, in vornui's terms. Positions are in the
/// layout's units: cell centres in cells, logical pixels when drawn as pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum TermEvent {
    Input(Input),
    Move(f32, f32),
    Down(f32, f32),
    Up(f32, f32),
    SecondaryDown(f32, f32),
    /// Wheel in the layout's units (rows in cells, logical pixels when
    /// drawn as pixels); positive scrolls down.
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
    /// Icons drawn as images, where the terminal shows kitty graphics.
    graphics: Option<Graphics>,
}

/// The icons sent to a terminal that shows kitty graphics, drawn in place of
/// Unicode placeholders so the diff and the terminal's own scrolling keep them.
struct Graphics {
    /// A cell's size in pixels, which each icon is rasterized to.
    cell: (u32, u32),
    /// Each icon in each colour sent, and its image id.
    sent: std::collections::HashMap<(u32, [u8; 3]), u32>,
    next: u32,
}

/// The placeholder for the image's top-left (and only) cell: U+10EEEE, row 0, column 0.
const PLACEHOLDER: &str = "\u{10EEEE}\u{0305}\u{0305}";

impl Graphics {
    /// The cell size the terminal reports, when it reports one.
    fn new() -> Option<Graphics> {
        let ws = terminal::window_size().ok()?;
        if ws.width == 0 || ws.height == 0 || ws.columns == 0 || ws.rows == 0 {
            return None;
        }
        let cell = (
            u32::from(ws.width / ws.columns),
            u32::from(ws.height / ws.rows),
        );
        (cell.0 >= 4 && cell.1 >= 4).then(|| Graphics {
            cell,
            sent: std::collections::HashMap::new(),
            next: 1,
        })
    }

    /// The image id for icon `key` in `fg`, sent first if it has not been;
    /// `None` when the icon cannot be drawn and its glyph stands in.
    fn image(&mut self, w: &mut impl Write, key: u32, fg: [u8; 3]) -> io::Result<Option<u32>> {
        if let Some(id) = self.sent.get(&(key, fg)) {
            return Ok(Some(*id));
        }
        let Some(rgba) = icon_rgba(key, fg, self.cell) else {
            return Ok(None);
        };
        let id = self.next;
        // Ids ride in the 24-bit foreground of the placeholder.
        if id > 0x00ff_ffff {
            return Ok(None);
        }
        self.next += 1;
        let data = base64(&rgba);
        let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let more = u8::from(i + 1 < chunks.len());
            if i == 0 {
                let (cw, ch) = self.cell;
                write!(w, "\x1b_Gq=2,a=t,f=32,s={cw},v={ch},i={id},m={more};")?;
            } else {
                write!(w, "\x1b_Gq=2,m={more};")?;
            }
            w.write_all(chunk)?;
            w.write_all(b"\x1b\\")?;
        }
        write!(w, "\x1b_Gq=2,a=p,U=1,i={id},c=1,r=1\x1b\\")?;
        self.sent.insert((key, fg), id);
        Ok(Some(id))
    }
}

/// Icon `key` in `fg` over a transparent `cell`, centred and as large as fits.
fn icon_rgba(key: u32, fg: [u8; 3], (cw, ch): (u32, u32)) -> Option<Vec<u8>> {
    let svg = super::icon_svg(key)?;
    let side = cw.min(ch);
    let mask = crate::rasterize_svg(&svg, side)?;
    let (ox, oy) = ((cw - side) / 2, (ch - side) / 2);
    let mut out = vec![0u8; (cw * ch * 4) as usize];
    for y in 0..side {
        for x in 0..side {
            let a = mask[(y * side + x) as usize];
            let at = (((y + oy) * cw + x + ox) * 4) as usize;
            out[at..at + 4].copy_from_slice(&[fg[0], fg[1], fg[2], a]);
        }
    }
    Some(out)
}

/// Standard base64, as the graphics protocol carries pixels.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
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
            shapes: pointer_shapes_from_env(),
            pointer: "default",
            graphics: images_wanted().then(Graphics::new).flatten(),
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

    /// Forgets the last frame, so the next one is written whole; icons are
    /// sent again at the cell size the terminal now has.
    pub fn invalidate(&mut self) {
        self.prev = None;
        if self.graphics.is_some() {
            let _ = write!(self.out, "\x1b_Ga=d,d=A,q=2\x1b\\");
            self.graphics = Graphics::new();
        }
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
            let image = match (&mut self.graphics, c.icon) {
                (Some(g), key) if key != 0 => g.image(&mut w, key, c.fg)?,
                _ => None,
            };
            if let Some(id) = image {
                let [_, r, g, b] = id.to_be_bytes();
                queue!(
                    w,
                    SetBackgroundColor(color(c.bg, self.depth)),
                    SetForegroundColor(Color::Rgb { r, g, b }),
                    Print(PLACEHOLDER)
                )?;
                style = None;
                pen = Some((x + 1, y));
                written += 1;
                continue;
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
        let name = pointer_name(cursor);
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
        if self.graphics.is_some() {
            let _ = write!(self.out, "\x1b_Ga=d,d=A,q=2\x1b\\");
        }
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

/// Whether this process's terminal changes the mouse pointer on OSC 22.
pub(crate) fn pointer_shapes_from_env() -> bool {
    takes_pointer_shapes(
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    )
}

/// `cursor`'s name in OSC 22.
pub(crate) fn pointer_name(cursor: crate::Cursor) -> &'static str {
    match cursor {
        crate::Cursor::Default => "default",
        crate::Cursor::Pointer => "pointer",
        crate::Cursor::Text => "text",
        crate::Cursor::ColResize => "ew-resize",
        crate::Cursor::RowResize => "ns-resize",
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

/// Whether to draw icons as images: on a terminal that shows kitty graphics
/// in true colour, unless `VORN_TUI_IMAGES=0` says not to.
fn images_wanted() -> bool {
    let env = |k: &str| std::env::var(k).ok();
    env("VORN_TUI_IMAGES").as_deref() != Some("0")
        && Depth::from_env() == Depth::True
        && takes_images(env("TERM_PROGRAM").as_deref(), env("TERM").as_deref())
}

/// Terminals that place kitty graphics at Unicode placeholders.
fn takes_images(program: Option<&str>, term: Option<&str>) -> bool {
    let program = program.unwrap_or_default().to_ascii_lowercase();
    let term = term.unwrap_or_default();
    matches!(program.as_str(), "ghostty" | "kitty")
        || term.starts_with("xterm-kitty")
        || term.starts_with("xterm-ghostty")
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

    #[test]
    fn base64_is_the_standard_alphabet_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd, 0x00]), "//79AA==");
    }

    #[test]
    fn icons_are_images_only_where_placeholders_work() {
        assert!(takes_images(Some("ghostty"), None));
        assert!(takes_images(None, Some("xterm-kitty")));
        assert!(!takes_images(
            Some("Apple_Terminal"),
            Some("xterm-256color")
        ));
        assert!(
            !takes_images(Some("WezTerm"), None),
            "no placeholder support there yet"
        );
    }

    #[test]
    fn an_icon_fills_its_cell_in_its_colour() {
        let key = super::super::icon_key(crate::icons::PLUS);
        let px = icon_rgba(key, [10, 20, 30], (8, 16)).expect("pixels");
        assert_eq!(px.len(), 8 * 16 * 4);
        assert!(px
            .chunks(4)
            .any(|p| p == [10, 20, 30, 255] || (p[..3] == [10, 20, 30] && p[3] > 0)));
        assert!(
            px.chunks(4).take(8).all(|p| p[3] == 0),
            "the top row of a tall cell is clear"
        );
        assert_eq!(icon_rgba(0, [0; 3], (8, 16)), None);
    }
}
