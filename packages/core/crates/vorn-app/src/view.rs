//! What a terminal pane looks like to the painter: rows cut into runs of
//! one resolved style, colours already resolved against the app's terminal
//! theme and whatever palette the program set.

use vorn_term_mirror::Mirror;
use vorn_term_proto::msg::GridPoint;
use vorn_term_proto::row;
use vorn_term_proto::screen::{
    attrs, row_flags, Color, CursorStyle, MouseMode, Row, Screen, StyleDef, Underline,
};

use crate::look::{term, Rgb};

/// Bits of [`Run::flags`].
pub mod flags {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 2;
    pub const UNDERLINE: u16 = 4;
    pub const STRIKE: u16 = 8;
    /// One cell two columns wide (CJK, emoji).
    pub const WIDE: u16 = 16;
    pub const FAINT: u16 = 32;
    /// Not plain ASCII: draw at exactly `col`, alone.
    pub const CLUSTER: u16 = 64;
}

/// `ncols` columns from `col` on `row` in one style; its text is
/// `text[text_off..text_off + text_len]` of the view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Run {
    pub row: u16,
    pub col: u16,
    pub ncols: u16,
    pub flags: u16,
    pub fg: Rgb,
    pub bg: Rgb,
    pub text_off: u32,
    pub text_len: u32,
}

/// The cursor's shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Block,
    Hollow,
    Bar,
    Underline,
}

/// One pane's screen, ready to paint.
#[derive(Debug, Clone, Default)]
pub struct PaneView {
    pub rev: u64,
    pub cols: u16,
    pub rows: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    pub cursor: Cursor,
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor_color: Rgb,
    pub runs: Vec<Run>,
    /// Every run's text, back to back: one allocation per view, not per run.
    pub text: Vec<u8>,
    /// Rows where a shell prompt starts (OSC 133): one command block each.
    pub prompts: Vec<u16>,
    /// The absolute line of row 0, and of the live screen's row 0; on the
    /// alternate screen, 0.
    pub line0: u64,
    pub live_line: u64,
    /// The oldest line vornd still holds, history included.
    pub oldest: u64,
    /// Lines of history above the live screen.
    pub history: u64,
    pub sb_epoch: u32,
    pub primary: bool,
    /// The program tracks the mouse.
    pub mouse: bool,
}

/// The xterm 256-colour palette, its first sixteen the app's.
pub fn xterm(i: u8) -> Rgb {
    match i {
        0..=15 => term::ANSI16[i as usize],
        16..=231 => {
            let i = i - 16;
            let lv = |v: u8| if v == 0 { 0 } else { 55 + 40 * v as u32 };
            (lv(i / 36) << 16) | (lv((i / 6) % 6) << 8) | lv(i % 6)
        }
        _ => {
            let v = 8 + 10 * (i as u32 - 232);
            (v << 16) | (v << 8) | v
        }
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> Rgb {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

struct Palette {
    fg: Rgb,
    bg: Rgb,
    over: Vec<(u8, Rgb)>,
}

impl Palette {
    fn of(m: &Mirror) -> Palette {
        let c = &m.term().colors;
        Palette {
            fg: c.fg.map_or(term::FG, rgb),
            bg: c.bg.map_or(term::BG, rgb),
            over: c.palette.iter().map(|(i, v)| (*i, rgb(*v))).collect(),
        }
    }

    fn color(&self, c: Color, default: Rgb) -> Rgb {
        match c {
            Color::Default => default,
            Color::Palette(i) => self
                .over
                .iter()
                .find(|(j, _)| *j == i)
                .map_or_else(|| xterm(i), |(_, v)| *v),
            Color::Rgb(r, g, b) => rgb((r, g, b)),
        }
    }

    /// (fg, bg, flags) of a style.
    fn resolve(&self, s: Option<&StyleDef>) -> (Rgb, Rgb, u16) {
        let Some(s) = s else {
            return (self.fg, self.bg, 0);
        };
        let mut fg = self.color(s.fg, self.fg);
        let mut bg = self.color(s.bg, self.bg);
        // Bold brightens the first eight colours, as most terminals do.
        if s.attrs & attrs::BOLD != 0 {
            if let Color::Palette(i @ 0..=7) = s.fg {
                fg = self.color(Color::Palette(i + 8), self.fg);
            }
        }
        if s.attrs & attrs::INVERSE != 0 {
            std::mem::swap(&mut fg, &mut bg);
        }
        if s.attrs & attrs::INVISIBLE != 0 {
            fg = bg;
        }
        let mut f = 0;
        for (attr, flag) in [
            (attrs::BOLD, flags::BOLD),
            (attrs::ITALIC, flags::ITALIC),
            (attrs::FAINT, flags::FAINT),
            (attrs::STRIKE, flags::STRIKE),
        ] {
            if s.attrs & attr != 0 {
                f |= flag;
            }
        }
        if s.underline != Underline::None {
            f |= flags::UNDERLINE;
        }
        (fg, bg, f)
    }
}

/// The view of a mirror as it is now, starting at absolute line `top`
/// when scrolled back. A scrolled view has one row more, below its last,
/// for drawing between rows.
pub fn build(m: &Mirror, top: Option<u64>) -> PaneView {
    let t = m.term();
    let primary = t.screen == Screen::Primary;
    let oldest = t.top_line.saturating_sub(t.history_lines);
    let first = match top {
        Some(top) if primary => top.clamp(oldest, t.top_line),
        _ => t.top_line,
    };
    let back = t.top_line - first;
    let mut v = if back == 0 {
        rows(m, m.rows())
    } else {
        // Rows the cache does not hold yet draw blank.
        let blank = Row::default();
        let shown = u64::from(t.rows) + 1;
        let mut v = rows(
            m,
            (first..first + shown).map(|l| m.row_at(l).unwrap_or(&blank)),
        );
        v.prompts.retain(|y| *y < t.rows);
        v
    };
    let cursor_y = u64::from(t.cursor.y) + back;
    v.rows = t.rows;
    v.cursor_x = t.cursor.x;
    v.cursor_y = u16::try_from(cursor_y).unwrap_or(u16::MAX);
    v.cursor_visible = t.cursor.visible && cursor_y <= u64::from(t.rows);
    v.cursor = match t.cursor.style {
        CursorStyle::BlockHollow => Cursor::Hollow,
        CursorStyle::Bar => Cursor::Bar,
        CursorStyle::Underline => Cursor::Underline,
        _ => Cursor::Block,
    };
    v.cursor_color = t.colors.cursor.map_or(term::CURSOR, rgb);
    v.sb_epoch = t.sb_epoch;
    v.primary = primary;
    v.mouse = t.mouse != MouseMode::None;
    if primary {
        v.line0 = first;
        v.live_line = t.top_line;
        v.oldest = oldest;
        v.history = t.history_lines;
    }
    v
}

/// A view of `rows`, one a line from the top, without a cursor: a finished
/// command's lines, cut from wherever the mirror holds them.
pub fn rows<'a>(m: &Mirror, rows: impl IntoIterator<Item = &'a Row>) -> PaneView {
    let t = m.term();
    let pal = Palette::of(m);
    let mut v = PaneView {
        rev: m.rev(),
        cols: t.cols,
        fg: pal.fg,
        bg: pal.bg,
        cursor_color: term::CURSOR,
        ..PaneView::default()
    };
    let mut n: u16 = 0;
    for (y, r) in rows.into_iter().enumerate() {
        n = y as u16 + 1;
        if r.flags & row_flags::PROMPT != 0 && r.flags & row_flags::PROMPT_CONTINUATION == 0 {
            v.prompts.push(y as u16);
        }
        let Ok(runs) = row::decode(&r.cells) else {
            continue;
        };
        let y = y as u16;
        let mut col: u16 = 0;
        for run in runs {
            let (fg, bg, f) = pal.resolve(m.style(run.style));
            for cell in run.cells {
                let w = if cell.wide { 2 } else { 1 };
                let ascii = !cell.wide && cell.text.len() <= 1;
                let text = if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                };
                match v.runs.last_mut() {
                    Some(l)
                        if ascii
                            && l.flags == f
                            && l.row == y
                            && l.col + l.ncols == col
                            && l.fg == fg
                            && l.bg == bg =>
                    {
                        l.ncols += 1;
                        l.text_len += 1;
                        v.text.push(text.as_bytes()[0]);
                    }
                    _ => {
                        let mut flags = f;
                        if !ascii {
                            flags |= flags::CLUSTER;
                        }
                        if cell.wide {
                            flags |= flags::WIDE;
                        }
                        v.runs.push(Run {
                            row: y,
                            col,
                            ncols: w,
                            flags,
                            fg,
                            bg,
                            text_off: v.text.len() as u32,
                            text_len: text.len() as u32,
                        });
                        v.text.extend_from_slice(text.as_bytes());
                    }
                }
                col += w;
            }
        }
    }
    v.rows = n;
    // Runs that are only blanks on the default background draw nothing.
    let (text, bg) = (&v.text, v.bg);
    v.runs.retain(|r| {
        r.bg != bg
            || r.flags & (flags::UNDERLINE | flags::STRIKE) != 0
            || text[r.text_off as usize..(r.text_off + r.text_len) as usize]
                .iter()
                .any(|b| *b != b' ')
    });
    v
}

impl PaneView {
    /// The cell at `(col, row)` on screen, as vornd names it.
    pub fn point(&self, col: u16, row: u16) -> GridPoint {
        GridPoint {
            line: self.line0 + u64::from(row),
            col,
            sb_epoch: self.sb_epoch,
        }
    }

    /// Rows the view starts above the live screen.
    pub fn back(&self) -> u64 {
        self.live_line - self.line0
    }

    /// The text of a run.
    pub fn run_text(&self, r: &Run) -> &str {
        std::str::from_utf8(&self.text[r.text_off as usize..(r.text_off + r.text_len) as usize])
            .unwrap_or("?")
    }

    /// The screen as text, one line per row, trailing blanks dropped: for
    /// checks and assistive technology, not for drawing.
    pub fn text(&self) -> String {
        let mut rows = vec![(String::new(), 0u16); self.rows as usize];
        for r in &self.runs {
            let Some((line, at)) = rows.get_mut(r.row as usize) else {
                continue;
            };
            // Runs skip blank cells; pad to the run's column.
            line.extend(std::iter::repeat_n(' ', r.col.saturating_sub(*at) as usize));
            line.push_str(self.run_text(r));
            *at = r.col + r.ncols;
        }
        let mut rows: Vec<String> = rows.into_iter().map(|(line, _)| line).collect();
        for line in &mut rows {
            line.truncate(line.trim_end().len());
        }
        rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_corners() {
        assert_eq!(xterm(1), 0xef4444);
        assert_eq!(xterm(16), 0x000000);
        assert_eq!(xterm(231), 0xffffff);
        assert_eq!(xterm(232), 0x080808);
        assert_eq!(xterm(196), 0xff0000);
    }

    #[test]
    fn text_pads_to_each_run() {
        let mut v = PaneView {
            rows: 2,
            cols: 10,
            text: b"ab$ x".to_vec(),
            ..PaneView::default()
        };
        let run = |row, col, off, len| Run {
            row,
            col,
            ncols: len as u16,
            text_off: off,
            text_len: len,
            ..Run::default()
        };
        v.runs = vec![run(0, 2, 0, 2), run(1, 0, 2, 2), run(1, 4, 4, 1)];
        assert_eq!(v.text(), "  ab\n$   x");
    }

    /// Three screen rows at lines 2..5 under two lines of history.
    fn scrolled_mirror() -> Mirror {
        use vorn_term_proto::row::{Cell, Run as Cells, ROW_FMT};
        use vorn_term_proto::screen::{LinkDef, Row, Snapshot, TermState};
        let at = |y: u16, line: u64, text: &str| Row {
            y,
            line,
            flags: 0,
            cells: row::encode(&[Cells {
                style: 0,
                link: 0,
                cells: text.chars().map(|c| Cell::new(c.to_string())).collect(),
            }]),
        };
        let mut term = TermState {
            cols: 10,
            rows: 3,
            top_line: 2,
            history_lines: 2,
            ..TermState::default()
        };
        term.cursor.y = 2;
        term.cursor.visible = true;
        Mirror::from_snapshot(Snapshot {
            state_gen: 1,
            rev: 1,
            row_fmt: ROW_FMT,
            term,
            styles: vec![StyleDef::default()],
            links: vec![LinkDef::default()],
            rows: vec![at(0, 2, "two"), at(1, 3, "three"), at(2, 4, "four")],
            history: vec![at(0, 0, "zero"), at(0, 1, "one")],
            ..Snapshot::default()
        })
        .expect("valid snapshot")
    }

    #[test]
    fn a_scrolled_view_starts_in_history_with_a_row_to_spare() {
        let m = scrolled_mirror();
        let live = build(&m, None);
        assert_eq!((live.line0, live.back(), live.history), (2, 0, 2));
        assert_eq!(live.text(), "two\nthree\nfour");
        let back = build(&m, Some(1));
        assert_eq!(back.back(), 1);
        assert_eq!(back.text(), "one\ntwo\nthree");
        // The spare row below is there to draw between rows.
        assert!(back
            .runs
            .iter()
            .any(|r| r.row == 3 && back.run_text(r) == "four"));
        assert_eq!((back.cursor_y, back.cursor_visible), (3, true));
        assert_eq!(build(&m, Some(0)).back(), 2);
        assert_eq!(build(&m, Some(9)).back(), 0);
    }
}
