//! The one surface every engine in the spike is driven and read through, so
//! the harness measures and compares engines and not adapters.
//!
//! Cells are read into plain owned data in a single shape: colours keep the
//! form the program asked for (default, palette index or RGB) rather than a
//! rendered RGB, since a renderer resolves palettes later and engines differ
//! only in when they would.

use std::fmt;

/// A colour as the program set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// SGR attributes, one bit each. Underline style is a separate field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Attrs(u16);

impl Attrs {
    pub const BOLD: Attrs = Attrs(1);
    pub const FAINT: Attrs = Attrs(1 << 1);
    pub const ITALIC: Attrs = Attrs(1 << 2);
    pub const BLINK: Attrs = Attrs(1 << 3);
    pub const INVERSE: Attrs = Attrs(1 << 4);
    pub const INVISIBLE: Attrs = Attrs(1 << 5);
    pub const STRIKE: Attrs = Attrs(1 << 6);
    pub const OVERLINE: Attrs = Attrs(1 << 7);

    pub const fn empty() -> Attrs {
        Attrs(0)
    }

    pub fn set(&mut self, flag: Attrs, on: bool) {
        if on {
            self.0 |= flag.0;
        } else {
            self.0 &= !flag.0;
        }
    }

    pub const fn contains(self, flag: Attrs) -> bool {
        self.0 & flag.0 == flag.0
    }
}

/// Underline style, as SGR 4:n names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// How many columns a cell's text takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Width {
    #[default]
    Narrow,
    /// The first column of a two-column character.
    Wide,
    /// The column a wide character to its left covers.
    Spacer,
}

/// One visible cell.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    /// The grapheme, base codepoint first; empty for a blank cell. A space
    /// written by the program and a cell never written read the same, since
    /// engines do not agree on keeping the difference.
    pub text: String,
    pub width: Width,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attrs,
    pub underline: Underline,
    /// The OSC 8 URI the cell links to.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
}

/// The visible screen, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
}

impl Grid {
    pub fn blank(cols: u16, rows: u16) -> Grid {
        Grid {
            cols,
            rows,
            cells: vec![Cell::default(); usize::from(cols) * usize::from(rows)],
        }
    }

    pub fn row(&self, y: u16) -> &[Cell] {
        let w = usize::from(self.cols);
        let start = usize::from(y) * w;
        &self.cells[start..start + w]
    }

    pub fn cell_mut(&mut self, x: u16, y: u16) -> &mut Cell {
        let i = usize::from(y) * usize::from(self.cols) + usize::from(x);
        &mut self.cells[i]
    }

    /// A row's text with trailing blanks dropped, for diffs and row matching.
    pub fn row_text(&self, y: u16) -> String {
        line_text(self.row(y))
    }
}

/// A line's text with spacers skipped and trailing blanks dropped.
pub fn line_text(cells: &[Cell]) -> String {
    let mut s: String = cells
        .iter()
        .filter(|c| c.width != Width::Spacer)
        .map(|c| if c.text.is_empty() { " " } else { &c.text })
        .collect();
    s.truncate(s.trim_end().len());
    s
}

impl fmt::Display for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for y in 0..self.rows {
            writeln!(f, "{}", self.row_text(y))?;
        }
        Ok(())
    }
}

/// [`Engine::rebuild`]'s answer for an engine that cannot write a snapshot.
pub const NO_SNAPSHOT: &str = "no snapshot";

/// What a terminal engine has to do for vornd's screen model. Engines are
/// built for one session each; nothing here is shared between instances.
pub trait Engine {
    /// The engine's name in reports.
    const NAME: &'static str;

    /// A terminal of `cols` x `rows` keeping at least `scrollback` lines of
    /// history above the screen.
    fn new(cols: u16, rows: u16, scrollback: usize) -> Self;

    /// One read of program output.
    fn feed(&mut self, bytes: &[u8]);

    fn resize(&mut self, cols: u16, rows: u16);

    /// The visible screen (the active one, primary or alternate).
    fn grid(&self) -> Grid;

    fn cursor(&self) -> Cursor;

    /// Lines of history above the screen now retained. Mutable because one
    /// engine (vt100) only gives the count up by scrolling its viewport.
    fn scrollback_lines(&mut self) -> usize;

    fn alt_screen(&self) -> bool;

    /// The last OSC 0/2 title, if the engine keeps one.
    fn title(&self) -> Option<String>;

    /// Bytes the terminal wrote back to the program (query replies), drained.
    fn take_replies(&mut self) -> Vec<u8>;

    /// The text of history line `n`, 0 being the oldest retained; `None`
    /// past the end. Absolute numbering across evictions is the caller's.
    fn history_line(&mut self, n: usize) -> Option<String>;

    /// A fresh engine rebuilt from this one's snapshot, with the snapshot's
    /// size in bytes, or why there is none. vornd's checkpoints need this.
    fn rebuild(&mut self) -> Result<(Self, usize), &'static str>
    where
        Self: Sized,
    {
        Err(NO_SNAPSHOT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_text_skips_spacers_and_trailing_blanks() {
        let mut g = Grid::blank(4, 1);
        g.cell_mut(0, 0).text = "界".into();
        g.cell_mut(0, 0).width = Width::Wide;
        g.cell_mut(1, 0).width = Width::Spacer;
        g.cell_mut(2, 0).text = "a".into();
        assert_eq!(g.row_text(0), "界a");
    }

    #[test]
    fn attrs_set_and_clear() {
        let mut a = Attrs::empty();
        a.set(Attrs::BOLD, true);
        a.set(Attrs::ITALIC, true);
        a.set(Attrs::BOLD, false);
        assert!(a.contains(Attrs::ITALIC) && !a.contains(Attrs::BOLD));
    }
}
