//! The vt100 crate: a parser and screen meant for terminal multiplexers and
//! tests. It answers no queries and keeps no hyperlinks; its title arrives
//! through a callback.

use vt100::{Callbacks, Parser};
use vt_api::{Attrs, Cell, Color, Cursor, Engine, Grid, Underline, Width};

#[derive(Default)]
struct Titles(Option<String>);

impl Callbacks for Titles {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.0 = Some(String::from_utf8_lossy(title).into_owned());
    }
}

pub struct Vt100 {
    parser: Parser<Titles>,
}

impl Engine for Vt100 {
    const NAME: &'static str = "vt100";

    fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        Vt100 {
            parser: Parser::new_with_callbacks(rows, cols, scrollback, Titles::default()),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    fn grid(&self) -> Grid {
        let s = self.parser.screen();
        let (rows, cols) = s.size();
        let mut g = Grid::blank(cols, rows);
        for y in 0..rows {
            for x in 0..cols {
                if let Some(c) = s.cell(y, x) {
                    *g.cell_mut(x, y) = cell(c);
                }
            }
        }
        g
    }

    fn cursor(&self) -> Cursor {
        let s = self.parser.screen();
        let (y, x) = s.cursor_position();
        Cursor {
            x,
            y,
            visible: !s.hide_cursor(),
        }
    }

    fn scrollback_lines(&mut self) -> usize {
        // vt100 reports history only as how far its viewport can scroll.
        let s = self.parser.screen_mut();
        s.set_scrollback(usize::MAX);
        let n = s.scrollback();
        s.set_scrollback(0);
        n
    }

    fn alt_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    fn title(&self) -> Option<String> {
        self.parser.callbacks().0.clone().filter(|t| !t.is_empty())
    }

    fn take_replies(&mut self) -> Vec<u8> {
        Vec::new()
    }

    fn history_line(&mut self, n: usize) -> Option<String> {
        let h = self.scrollback_lines();
        if n >= h {
            return None;
        }
        // Only reachable by scrolling the viewport until the line is on top.
        let s = self.parser.screen_mut();
        s.set_scrollback(h - n);
        let cols = s.size().1;
        let line = s.rows(0, cols).next();
        s.set_scrollback(0);
        line.map(|l| l.trim_end().to_owned())
    }

    fn rebuild(&mut self) -> Result<(Self, usize), &'static str> {
        let screen = self.parser.screen();
        let snap = screen.state_formatted();
        let (rows, cols) = screen.size();
        let mut rebuilt = Vt100::new(cols, rows, 0);
        rebuilt.feed(&snap);
        Ok((rebuilt, snap.len()))
    }
}

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn cell(c: &vt100::Cell) -> Cell {
    let mut attrs = Attrs::empty();
    attrs.set(Attrs::BOLD, c.bold());
    attrs.set(Attrs::FAINT, c.dim());
    attrs.set(Attrs::ITALIC, c.italic());
    attrs.set(Attrs::INVERSE, c.inverse());
    let width = if c.is_wide() {
        Width::Wide
    } else if c.is_wide_continuation() {
        Width::Spacer
    } else {
        Width::Narrow
    };
    let text = match c.contents() {
        " " => String::new(),
        t => t.to_owned(),
    };
    Cell {
        text,
        width,
        fg: color(c.fgcolor()),
        bg: color(c.bgcolor()),
        attrs,
        underline: if c.underline() {
            Underline::Single
        } else {
            Underline::None
        },
        link: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_colour_and_wide_cells() {
        let mut e = Vt100::new(10, 3, 0);
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
    fn counts_history() {
        let mut e = Vt100::new(10, 3, 100);
        e.feed(b"1\r\n2\r\n3\r\n4\r\n5");
        assert_eq!(e.scrollback_lines(), 2);
        assert_eq!(e.grid().row_text(0), "3");
    }
}
