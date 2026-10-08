//! The baseline: libghostty-vt, driven the way vornd drives it.
//!
//! [`Ghostty`] goes through vorn-screen's [`Emulator`], which runs its own
//! model of Ghostty's parser beside the terminal so a checkpoint can be cut;
//! that is what a session costs today. [`GhosttyRaw`] writes straight to the
//! terminal, to separate libghostty-vt's own speed from vorn-screen's.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::screen::{CellContentTag, CellWide, Screen as Which};
use libghostty_vt::style::{StyleColor, Underline as GUnderline};
use libghostty_vt::terminal::{Options, Point, PointCoordinate, Terminal};
use libghostty_vt::Error as GError;
use vorn_screen::{Effect, Emulator};
use vt_api::{line_text, Attrs, Cell, Color, Cursor, Engine, Grid, Underline, Width};

/// Ghostty bounds history in bytes of page memory, not lines; this finds the
/// smallest budget that keeps `lines` lines at `cols` columns, once per width.
pub fn scrollback_bytes(cols: u16, lines: usize) -> usize {
    static CACHE: OnceLock<Mutex<HashMap<(u16, usize), usize>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(&b) = cache.lock().expect("calibration lock").get(&(cols, lines)) {
        return b;
    }
    let bytes = if lines == 0 {
        0
    } else {
        calibrate(cols, lines)
    };
    cache
        .lock()
        .expect("calibration lock")
        .insert((cols, lines), bytes);
    bytes
}

fn retained(cols: u16, lines: usize, bytes: usize) -> usize {
    let mut t = Terminal::new(Options {
        cols,
        rows: 24,
        max_scrollback: bytes,
    })
    .expect("scratch terminal");
    let mut line = vec![b'x'; usize::from(cols) - 1];
    line.extend_from_slice(b"\r\n");
    for _ in 0..lines + lines / 2 + 64 {
        t.vt_write(&line);
    }
    t.scrollback_rows().expect("scrollback rows")
}

fn calibrate(cols: u16, lines: usize) -> usize {
    let mut hi = 64 << 10;
    while retained(cols, lines, hi) < lines {
        hi *= 2;
    }
    let mut lo = hi / 2;
    while hi - lo > 4096 {
        let mid = lo + (hi - lo) / 2;
        if retained(cols, lines, mid) >= lines {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// libghostty-vt behind vorn-screen's [`Emulator`], as vornd runs it.
pub struct Ghostty {
    em: Emulator,
    effects: Vec<Effect>,
    replies: Vec<u8>,
}

impl Engine for Ghostty {
    const NAME: &'static str = "libghostty-vt";

    fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let bytes = scrollback_bytes(cols, scrollback);
        Ghostty {
            em: Emulator::with_scrollback(cols.into(), rows.into(), bytes).expect("emulator"),
            effects: Vec::new(),
            replies: Vec::new(),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.em.feed(bytes, &mut self.effects);
        for e in self.effects.drain(..) {
            if let Effect::Reply(r) = e {
                self.replies.extend_from_slice(&r);
            }
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.em
            .resize(cols.into(), rows.into(), &mut self.effects)
            .expect("resize");
        self.effects.clear();
    }

    fn grid(&self) -> Grid {
        read_grid(self.em.terminal())
    }

    fn cursor(&self) -> Cursor {
        read_cursor(self.em.terminal())
    }

    fn scrollback_lines(&mut self) -> usize {
        self.em.terminal().scrollback_rows().unwrap_or(0)
    }

    fn alt_screen(&self) -> bool {
        is_alt(self.em.terminal())
    }

    fn title(&self) -> Option<String> {
        read_title(self.em.terminal())
    }

    fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }

    fn history_line(&mut self, n: usize) -> Option<String> {
        history_text(self.em.terminal(), n)
    }

    fn rebuild(&mut self) -> Result<(Self, usize), &'static str> {
        let cp = self.em.checkpoint()?;
        let em = Emulator::restore(&cp).map_err(|_| "restore failed")?;
        let rebuilt = Ghostty {
            em,
            effects: Vec::new(),
            replies: Vec::new(),
        };
        Ok((rebuilt, cp.encode().len()))
    }
}

/// libghostty-vt with nothing in front of it. Queries go unanswered: the
/// replies need callbacks this adapter does not install.
pub struct GhosttyRaw {
    term: Terminal<'static, 'static>,
}

impl Engine for GhosttyRaw {
    const NAME: &'static str = "libghostty-vt (raw)";

    fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let term = Terminal::new(Options {
            cols,
            rows,
            max_scrollback: scrollback_bytes(cols, scrollback),
        })
        .expect("terminal");
        GhosttyRaw { term }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.term.vt_write(bytes);
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(cols, rows, 1, 1).expect("resize");
    }

    fn grid(&self) -> Grid {
        read_grid(&self.term)
    }

    fn cursor(&self) -> Cursor {
        read_cursor(&self.term)
    }

    fn scrollback_lines(&mut self) -> usize {
        self.term.scrollback_rows().unwrap_or(0)
    }

    fn alt_screen(&self) -> bool {
        is_alt(&self.term)
    }

    fn title(&self) -> Option<String> {
        read_title(&self.term)
    }

    fn take_replies(&mut self) -> Vec<u8> {
        Vec::new()
    }

    fn history_line(&mut self, n: usize) -> Option<String> {
        history_text(&self.term, n)
    }

    /// libghostty-vt's formatter alone, which vorn-screen's checkpoints
    /// exist to complete; measured to show the gap.
    fn rebuild(&mut self) -> Result<(Self, usize), &'static str> {
        let snap = format_vt(&self.term).ok_or("formatter failed")?;
        let (cols, rows) = dims(&self.term);
        let mut rebuilt = GhosttyRaw::new(cols, rows, 0);
        rebuilt.feed(&snap);
        Ok((rebuilt, snap.len()))
    }
}

fn color(c: StyleColor) -> Color {
    match c {
        StyleColor::None => Color::Default,
        StyleColor::Palette(i) => Color::Indexed(i.0),
        StyleColor::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn underline(u: GUnderline) -> Underline {
    match u {
        GUnderline::Single => Underline::Single,
        GUnderline::Double => Underline::Double,
        GUnderline::Curly => Underline::Curly,
        GUnderline::Dotted => Underline::Dotted,
        GUnderline::Dashed => Underline::Dashed,
        _ => Underline::None,
    }
}

fn read_cell(term: &Terminal<'static, 'static>, at: Point) -> Result<Cell, GError> {
    let r = term.grid_ref(at)?;
    let raw = r.cell()?;
    let style = r.style()?;
    let mut out = Cell {
        fg: color(style.fg_color),
        bg: color(style.bg_color),
        underline: underline(style.underline),
        width: match raw.wide()? {
            CellWide::Wide => Width::Wide,
            CellWide::SpacerTail => Width::Spacer,
            CellWide::Narrow | CellWide::SpacerHead => Width::Narrow,
        },
        ..Cell::default()
    };
    // A cell with no text can carry a background of its own (erased with a
    // coloured pen); its style is then the default.
    match raw.content_tag()? {
        CellContentTag::BgColorPalette => out.bg = Color::Indexed(raw.bg_color_palette()?.0),
        CellContentTag::BgColorRgb => {
            let c = raw.bg_color_rgb()?;
            out.bg = Color::Rgb(c.r, c.g, c.b);
        }
        CellContentTag::Codepoint | CellContentTag::CodepointGrapheme => {}
    }
    let a = &mut out.attrs;
    a.set(Attrs::BOLD, style.bold);
    a.set(Attrs::FAINT, style.faint);
    a.set(Attrs::ITALIC, style.italic);
    a.set(Attrs::BLINK, style.blink);
    a.set(Attrs::INVERSE, style.inverse);
    a.set(Attrs::INVISIBLE, style.invisible);
    a.set(Attrs::STRIKE, style.strikethrough);
    a.set(Attrs::OVERLINE, style.overline);
    if raw.has_text()? {
        let mut buf = ['\0'; 16];
        let n = match r.graphemes(&mut buf) {
            Ok(n) => n,
            Err(GError::OutOfSpace { .. }) => buf.len(),
            Err(e) => return Err(e),
        };
        out.text = buf[..n].iter().collect();
        if out.text == " " {
            out.text.clear();
        }
    }
    if raw.has_hyperlink()? {
        let mut buf = vec![0u8; 4096];
        if let Ok(n) = r.hyperlink_uri(&mut buf) {
            out.link = Some(String::from_utf8_lossy(&buf[..n]).into_owned());
        }
    }
    Ok(out)
}

fn read_grid(term: &Terminal<'static, 'static>) -> Grid {
    let (cols, rows) = dims(term);
    let mut g = Grid::blank(cols, rows);
    for y in 0..rows {
        for x in 0..cols {
            // An unreadable cell stays blank and shows up as a difference.
            let at = Point::Active(PointCoordinate { x, y: y.into() });
            if let Ok(c) = read_cell(term, at) {
                *g.cell_mut(x, y) = c;
            }
        }
    }
    g
}

fn history_text(term: &Terminal<'static, 'static>, n: usize) -> Option<String> {
    if n >= term.scrollback_rows().ok()? {
        return None;
    }
    let y = u32::try_from(n).ok()?;
    let cells: Vec<Cell> = (0..term.cols().ok()?)
        .map(|x| read_cell(term, Point::History(PointCoordinate { x, y })).unwrap_or_default())
        .collect();
    Some(line_text(&cells))
}

/// VT with the formatter options vorn-screen's checkpoints use.
fn format_vt(term: &Terminal<'static, 'static>) -> Option<Vec<u8>> {
    let opts = FormatterOptions::new()
        .with_format(Format::Vt)
        .with_modes(true)
        .with_scrolling_region(true)
        .with_cursor(true)
        .with_style(true)
        .with_hyperlink(true)
        .with_charsets(true);
    let mut f = Formatter::new(term, opts).ok()?;
    f.format_alloc(None).ok().map(|b| b.to_vec())
}

fn dims(term: &Terminal<'static, 'static>) -> (u16, u16) {
    (term.cols().unwrap_or(0), term.rows().unwrap_or(0))
}

fn read_cursor(term: &Terminal<'static, 'static>) -> Cursor {
    Cursor {
        x: term.cursor_x().unwrap_or(0),
        y: term.cursor_y().unwrap_or(0),
        visible: term.is_cursor_visible().unwrap_or(true),
    }
}

fn is_alt(term: &Terminal<'static, 'static>) -> bool {
    matches!(term.active_screen(), Ok(Which::Alternate))
}

fn read_title(term: &Terminal<'static, 'static>) -> Option<String> {
    term.title()
        .ok()
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_colour_and_wide_cells() {
        let mut g = Ghostty::new(10, 3, 0);
        g.feed("\x1b[1;31mab\x1b[0m界".as_bytes());
        let grid = g.grid();
        let row = grid.row(0);
        assert_eq!(row[0].text, "a");
        assert_eq!(row[0].fg, Color::Indexed(1));
        assert!(row[0].attrs.contains(Attrs::BOLD));
        assert_eq!(row[2].width, Width::Wide);
        assert_eq!(row[3].width, Width::Spacer);
        assert_eq!(g.cursor().x, 4);
    }

    #[test]
    fn answers_a_cursor_position_query() {
        let mut g = Ghostty::new(10, 3, 0);
        g.feed(b"ab\x1b[6n");
        assert_eq!(g.take_replies(), b"\x1b[1;3R");
    }

    #[test]
    fn calibrated_scrollback_keeps_the_lines_asked_for() {
        let bytes = scrollback_bytes(80, 1000);
        assert!(retained(80, 1000, bytes) >= 1000);
    }
}
