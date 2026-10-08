//! alacritty_terminal: vte's parser feeding alacritty's `Term` and grid.
//!
//! Synchronized updates (mode 2026) are applied as they arrive: vte's
//! processor would otherwise hold the bytes back until the update ends or a
//! wall-clock timeout passes, which a headless model has no use for and
//! which would make every read depend on timing.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AColor, Processor, Timeout};
use vt_api::{line_text, Attrs, Cell, Color, Cursor, Engine, Grid, Underline, Width};

#[derive(Default)]
struct Sink {
    replies: Vec<u8>,
    title: Option<String>,
}

#[derive(Clone, Default)]
struct Listener(Rc<RefCell<Sink>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let mut s = self.0.borrow_mut();
        match event {
            Event::PtyWrite(text) => s.replies.extend_from_slice(text.as_bytes()),
            Event::Title(t) => s.title = Some(t),
            Event::ResetTitle => s.title = None,
            _ => {}
        }
    }
}

/// Never holds a synchronized update back (see the module docs).
#[derive(Default)]
struct NoSync;

impl Timeout for NoSync {
    fn set_timeout(&mut self, _: Duration) {}
    fn clear_timeout(&mut self) {}
    fn pending_timeout(&self) -> bool {
        false
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Alacritty {
    term: Term<Listener>,
    parser: Processor<NoSync>,
    sink: Listener,
}

impl Engine for Alacritty {
    const NAME: &'static str = "alacritty_terminal";

    fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let sink = Listener::default();
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let size = Size {
            cols: cols.into(),
            rows: rows.into(),
        };
        Alacritty {
            term: Term::new(config, &size, sink.clone()),
            parser: Processor::new(),
            sink,
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size {
            cols: cols.into(),
            rows: rows.into(),
        });
    }

    fn grid(&self) -> Grid {
        let grid = self.term.grid();
        let (cols, rows) = (grid.columns(), grid.screen_lines());
        let mut g = Grid::blank(cols as u16, rows as u16);
        for y in 0..rows {
            let row = &grid[Line(y as i32)];
            for x in 0..cols {
                *g.cell_mut(x as u16, y as u16) = cell(&row[Column(x)]);
            }
        }
        g
    }

    fn cursor(&self) -> Cursor {
        let p = self.term.grid().cursor.point;
        Cursor {
            x: p.column.0 as u16,
            y: p.line.0.max(0) as u16,
            visible: self.term.mode().contains(TermMode::SHOW_CURSOR),
        }
    }

    fn scrollback_lines(&mut self) -> usize {
        self.term.grid().history_size()
    }

    fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    fn title(&self) -> Option<String> {
        self.sink.0.borrow().title.clone().filter(|t| !t.is_empty())
    }

    fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.sink.0.borrow_mut().replies)
    }

    fn history_line(&mut self, n: usize) -> Option<String> {
        let grid = self.term.grid();
        let h = grid.history_size();
        if n >= h {
            return None;
        }
        // History runs from `Line(-h)` (oldest) to `Line(-1)` (newest).
        let row = &grid[Line(n as i32 - h as i32)];
        let cells: Vec<Cell> = (0..grid.columns()).map(|x| cell(&row[Column(x)])).collect();
        Some(line_text(&cells))
    }
}

fn color(c: AColor) -> Color {
    match c {
        AColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AColor::Indexed(i) => Color::Indexed(i),
        AColor::Named(n) if (n as usize) < 16 => Color::Indexed(n as u8),
        // Foreground, Background, Cursor and the dim variants are a
        // renderer's business; SGR only reaches them through a reset.
        AColor::Named(_) => Color::Default,
    }
}

fn cell(c: &alacritty_terminal::term::cell::Cell) -> Cell {
    let f = c.flags;
    let mut attrs = Attrs::empty();
    attrs.set(Attrs::BOLD, f.contains(Flags::BOLD));
    attrs.set(Attrs::FAINT, f.contains(Flags::DIM));
    attrs.set(Attrs::ITALIC, f.contains(Flags::ITALIC));
    attrs.set(Attrs::INVERSE, f.contains(Flags::INVERSE));
    attrs.set(Attrs::INVISIBLE, f.contains(Flags::HIDDEN));
    attrs.set(Attrs::STRIKE, f.contains(Flags::STRIKEOUT));
    let underline = if f.contains(Flags::UNDERCURL) {
        Underline::Curly
    } else if f.contains(Flags::DOUBLE_UNDERLINE) {
        Underline::Double
    } else if f.contains(Flags::DOTTED_UNDERLINE) {
        Underline::Dotted
    } else if f.contains(Flags::DASHED_UNDERLINE) {
        Underline::Dashed
    } else if f.contains(Flags::UNDERLINE) {
        Underline::Single
    } else {
        Underline::None
    };
    let width = if f.contains(Flags::WIDE_CHAR) {
        Width::Wide
    } else if f.contains(Flags::WIDE_CHAR_SPACER) {
        Width::Spacer
    } else {
        Width::Narrow
    };
    let mut text = String::new();
    // Alacritty keeps a tab in the cell it started on, for copying; it is
    // blank on screen.
    if !matches!(c.c, ' ' | '\t') || c.zerowidth().is_some() {
        text.push(c.c);
        text.extend(c.zerowidth().unwrap_or_default());
    }
    if width == Width::Spacer {
        text.clear();
    }
    Cell {
        text,
        width,
        fg: color(c.fg),
        bg: color(c.bg),
        attrs,
        underline,
        link: c.hyperlink().map(|h| h.uri().to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_colour_and_wide_cells() {
        let mut e = Alacritty::new(10, 3, 0);
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
    fn applies_a_synchronized_update_at_once() {
        let mut e = Alacritty::new(10, 3, 0);
        e.feed(b"\x1b[?2026hhi");
        assert_eq!(e.grid().row_text(0), "hi");
    }

    #[test]
    fn keeps_title_and_replies() {
        let mut e = Alacritty::new(10, 3, 0);
        e.feed(b"\x1b]2;t\x07ab\x1b[6n");
        assert_eq!(e.title().as_deref(), Some("t"));
        assert_eq!(e.take_replies(), b"\x1b[1;3R");
    }
}
