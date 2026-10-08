//! wezterm-term: wezterm's terminal model, with termwiz's types, from
//! upstream at a pinned commit (it is not published on crates.io).

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use termwiz::cell::{Blink, Intensity, Underline as WUnderline};
use termwiz::color::ColorAttribute;
use termwiz::surface::CursorVisibility;
use vt_api::{line_text, Attrs, Cell, Color, Cursor, Engine, Grid, Underline, Width};
use wezterm_term::color::ColorPalette;
use wezterm_term::{Terminal, TerminalConfiguration, TerminalSize};

/// The title wezterm-term starts with, reported here as no title.
const DEFAULT_TITLE: &str = "wezterm";

#[derive(Debug)]
struct Config {
    scrollback: usize,
}

impl TerminalConfiguration for Config {
    fn scrollback_size(&self) -> usize {
        self.scrollback
    }

    fn color_palette(&self) -> ColorPalette {
        ColorPalette::default()
    }
}

/// Where the terminal writes its answers to queries.
#[derive(Clone, Default)]
struct Replies(Arc<Mutex<Vec<u8>>>);

impl Write for Replies {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct Wezterm {
    term: Terminal,
    replies: Replies,
}

fn size(cols: u16, rows: u16) -> TerminalSize {
    TerminalSize {
        rows: rows.into(),
        cols: cols.into(),
        pixel_width: usize::from(cols) * 8,
        pixel_height: usize::from(rows) * 16,
        dpi: 0,
    }
}

impl Engine for Wezterm {
    const NAME: &'static str = "wezterm-term";

    fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let replies = Replies::default();
        let term = Terminal::new(
            size(cols, rows),
            Arc::new(Config { scrollback }),
            "vorn",
            "0",
            Box::new(replies.clone()),
        );
        Wezterm { term, replies }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.term.advance_bytes(bytes);
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(size(cols, rows));
    }

    fn grid(&self) -> Grid {
        let screen = self.term.screen();
        let (cols, rows) = (screen.physical_cols, screen.physical_rows);
        let mut g = Grid::blank(cols as u16, rows as u16);
        let range = screen.phys_range(&(0..rows as i64));
        // `with_phys_lines` would avoid the clone, but it indexes the second
        // half of its ring buffer by absolute row and panics once it wraps.
        for (y, line) in screen.lines_in_phys_range(range).iter().enumerate() {
            let start = y * cols;
            read_line(line, &mut g.cells[start..start + cols]);
        }
        g
    }

    fn cursor(&self) -> Cursor {
        let p = self.term.cursor_pos();
        Cursor {
            x: p.x as u16,
            y: p.y.max(0) as u16,
            visible: p.visibility == CursorVisibility::Visible,
        }
    }

    fn scrollback_lines(&mut self) -> usize {
        let s = self.term.screen();
        s.scrollback_rows().saturating_sub(s.physical_rows)
    }

    fn alt_screen(&self) -> bool {
        self.term.is_alt_screen_active()
    }

    fn title(&self) -> Option<String> {
        let t = self.term.get_title();
        (!t.is_empty() && t != DEFAULT_TITLE).then(|| t.to_owned())
    }

    fn take_replies(&mut self) -> Vec<u8> {
        // wezterm-term hands replies to a writer thread of its own (one per
        // terminal), so they land a moment after the feed that asked.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        loop {
            let got = self
                .replies
                .0
                .lock()
                .map(|mut r| std::mem::take(&mut *r))
                .unwrap_or_default();
            if !got.is_empty() || std::time::Instant::now() >= deadline {
                return got;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn history_line(&mut self, n: usize) -> Option<String> {
        if n >= self.scrollback_lines() {
            return None;
        }
        let screen = self.term.screen();
        let mut cells = vec![Cell::default(); screen.physical_cols];
        // Physical line 0 is the oldest history line.
        let line = screen.lines_in_phys_range(n..n + 1).pop()?;
        read_line(&line, &mut cells);
        Some(line_text(&cells))
    }
}

fn read_line(line: &wezterm_term::Line, out: &mut [Cell]) {
    let cols = out.len();
    for c in line.visible_cells() {
        let x = c.cell_index();
        if x >= cols {
            break;
        }
        let wide = c.width() > 1;
        out[x] = cell(c.str(), wide, c.attrs());
        if wide && x + 1 < cols {
            out[x + 1] = cell("", false, c.attrs());
            out[x + 1].width = Width::Spacer;
        }
    }
}

fn color(c: ColorAttribute) -> Color {
    match c {
        ColorAttribute::Default => Color::Default,
        ColorAttribute::PaletteIndex(i) => Color::Indexed(i),
        ColorAttribute::TrueColorWithPaletteFallback(rgb, _)
        | ColorAttribute::TrueColorWithDefaultFallback(rgb) => {
            // termwiz keeps RGB as floats; round rather than truncate so 8-bit
            // values survive the trip.
            let (r, g, b, _) = rgb.to_tuple_rgba();
            let u = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
            Color::Rgb(u(r), u(g), u(b))
        }
    }
}

fn cell(text: &str, wide: bool, a: &termwiz::cell::CellAttributes) -> Cell {
    let mut attrs = Attrs::empty();
    attrs.set(Attrs::BOLD, a.intensity() == Intensity::Bold);
    attrs.set(Attrs::FAINT, a.intensity() == Intensity::Half);
    attrs.set(Attrs::ITALIC, a.italic());
    attrs.set(Attrs::BLINK, a.blink() != Blink::None);
    attrs.set(Attrs::INVERSE, a.reverse());
    attrs.set(Attrs::INVISIBLE, a.invisible());
    attrs.set(Attrs::STRIKE, a.strikethrough());
    attrs.set(Attrs::OVERLINE, a.overline());
    let underline = match a.underline() {
        WUnderline::None => Underline::None,
        WUnderline::Single => Underline::Single,
        WUnderline::Double => Underline::Double,
        WUnderline::Curly => Underline::Curly,
        WUnderline::Dotted => Underline::Dotted,
        WUnderline::Dashed => Underline::Dashed,
    };
    Cell {
        text: if text == " " {
            String::new()
        } else {
            text.to_owned()
        },
        width: if wide { Width::Wide } else { Width::Narrow },
        fg: color(a.foreground()),
        bg: color(a.background()),
        attrs,
        underline,
        link: a.hyperlink().map(|h| h.uri().to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_colour_and_wide_cells() {
        let mut e = Wezterm::new(10, 3, 0);
        e.feed("\x1b[1;31mab\x1b[0m界".as_bytes());
        let grid = e.grid();
        let row = grid.row(0);
        assert_eq!(row[0].text, "a");
        assert_eq!(row[0].fg, Color::Indexed(1));
        assert!(row[0].attrs.contains(Attrs::BOLD));
        assert_eq!(row[2].width, Width::Wide);
        assert_eq!(row[3].width, Width::Spacer);
        assert_eq!(e.cursor().x, 4);
    }

    #[test]
    fn round_trips_truecolor() {
        let mut e = Wezterm::new(10, 3, 0);
        e.feed(b"\x1b[38;2;1;128;254mx");
        assert_eq!(e.grid().row(0)[0].fg, Color::Rgb(1, 128, 254));
    }

    #[test]
    fn reads_the_screen_after_history_wraps_its_ring() {
        let mut e = Wezterm::new(10, 3, 5);
        let text: Vec<String> = (0..100).map(|i| i.to_string()).collect();
        e.feed(text.join("\r\n").as_bytes());
        assert_eq!(e.grid().row_text(2), "99");
        assert_eq!(e.history_line(4).as_deref(), Some("96"));
    }

    #[test]
    fn keeps_title_and_replies() {
        let mut e = Wezterm::new(10, 3, 0);
        assert_eq!(e.title(), None);
        e.feed(b"\x1b]2;t\x07ab\x1b[6n");
        assert_eq!(e.title().as_deref(), Some("t"));
        assert_eq!(e.take_replies(), b"\x1b[1;3R");
    }
}
