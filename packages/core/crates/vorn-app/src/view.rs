//! What a terminal pane looks like to the painter: rows cut into runs of
//! one resolved style, colours already resolved against the app's terminal
//! theme and whatever palette the program set.

use std::sync::Arc;

use vorn_grid_client::Shown;
use vorn_term_mirror::{kitty, Mirror};
use vorn_term_proto::msg::GridPoint;
use vorn_term_proto::row;
use vorn_term_proto::screen::{
    self, attrs, row_flags, Color, CursorStyle, Image, MouseMode, Row, Screen, StyleDef, Underline,
};

use crate::look::{self, Rgb, Term};

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

/// A kitty image drawn on a pane, over or under its text.
#[derive(Clone)]
pub struct Picture {
    /// Names these pixels for the renderer's texture cache.
    pub key: u64,
    pub image: Arc<Image>,
    /// The part of the image drawn: x, y, width, height in image pixels.
    pub src: [f32; 4],
    /// Where: x, y, width, height from the top left of row 0, in [`PaneView::cell_px`] pixels.
    pub dst: [f32; 4],
    /// Below text when negative.
    pub z: i32,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picture")
            .field("key", &self.key)
            .field("image", &self.image.id)
            .field("src", &self.src)
            .field("dst", &self.dst)
            .field("z", &self.z)
            .finish()
    }
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
    /// Kitty images to draw, lowest z first; see [`pictures`].
    pub pictures: Vec<Picture>,
    /// The server's cell size in pixels, which picture pixels are measured in.
    pub cell_px: (u32, u32),
    /// Runs of unicode placeholder cells, by row; drawn blank, their pictures placed by
    /// [`pictures`].
    pub holders: Vec<(u16, kitty::Run)>,
}

/// The xterm 256-colour palette, its first sixteen the theme's.
pub fn xterm(i: u8, ansi: &[Rgb; 16]) -> Rgb {
    match i {
        0..=15 => ansi[i as usize],
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

/// `c` when a program set it: vornd reports its start colours too, and the
/// theme is drawn in their place, so picking a theme recolours running terminals.
fn program_set(c: Option<(u8, u8, u8)>, start: [u8; 3]) -> Option<(u8, u8, u8)> {
    c.filter(|&(r, g, b)| [r, g, b] != start)
}

struct Palette {
    fg: Rgb,
    bg: Rgb,
    theme: Term,
    over: Vec<(u8, Rgb)>,
}

impl Palette {
    fn of(m: &Mirror, theme: Term) -> Palette {
        let c = &m.term().colors;
        Palette {
            fg: program_set(c.fg, screen::START_FG).map_or(theme.fg, rgb),
            bg: program_set(c.bg, screen::START_BG).map_or(theme.bg, rgb),
            theme,
            over: c
                .palette
                .iter()
                .filter(|(i, v)| program_set(Some(*v), screen::start_entry(*i)).is_some())
                .map(|(i, v)| (*i, rgb(*v)))
                .collect(),
        }
    }

    fn color(&self, c: Color, default: Rgb) -> Rgb {
        match c {
            Color::Default => default,
            Color::Palette(i) => self
                .over
                .iter()
                .find(|(j, _)| *j == i)
                .map_or_else(|| xterm(i, &self.theme.ansi), |(_, v)| *v),
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
    v.cursor_color = program_set(t.colors.cursor, screen::START_FG).map_or(v.cursor_color, rgb);
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
    let pal = Palette::of(m, look::term());
    let mut v = PaneView {
        rev: m.rev(),
        cols: t.cols,
        fg: pal.fg,
        bg: pal.bg,
        cursor_color: pal.theme.cursor,
        ..PaneView::default()
    };
    // Placeholders draw nothing without a virtual placement, so are left as text unparsed.
    let holders = !m.placements().virtuals.is_empty();
    let mut held = kitty::Runs::default();
    let mut row_held = Vec::new();
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
            let style = m.style(run.style);
            let (fg, bg, f) = pal.resolve(style);
            for cell in run.cells {
                let w = if cell.wide { 2 } else { 1 };
                let held_cell = holders
                    .then(|| {
                        let (fg, ul) = style.map_or((Color::Default, Color::Default), |s| {
                            (s.fg, s.underline_color)
                        });
                        kitty::Cell::parse(&cell.text, fg, ul)
                    })
                    .flatten();
                if holders {
                    held.push(col, held_cell, &mut row_held);
                }
                let ascii = held_cell.is_some() || (!cell.wide && cell.text.len() <= 1);
                let text = if cell.text.is_empty() || held_cell.is_some() {
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
        if holders {
            held.end(&mut row_held);
            v.holders.extend(row_held.drain(..).map(|r| (y, r)));
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

/// Places the kitty images `m` shows on `v`, a view of it [`build`] made, their pixels looked up
/// by id in `image`: placements a frame carried, and the pieces of virtual ones its placeholder
/// runs show. Those wholly off the view are left out.
pub fn pictures<'a>(v: &mut PaneView, m: &Mirror, image: impl Fn(u32) -> Option<&'a Shown>) {
    let p = m.placements();
    v.pictures.clear();
    if p.is_empty() || p.cell_w == 0 || p.cell_h == 0 {
        return;
    }
    v.cell_px = (p.cell_w, p.cell_h);
    let (cw, ch) = (p.cell_w as f32, p.cell_h as f32);
    let shown = |id: u32, gen: u64| image(id).filter(|s| s.image.gen == gen);
    // A scrolled view draws one row more, below its last.
    let height = (f32::from(v.rows) + 1.0) * ch;
    let width = f32::from(v.cols) * cw;
    let visible =
        |d: [f32; 4]| d[1] < height && d[1] + d[3] > 0.0 && d[0] < width && d[0] + d[2] > 0.0;
    let top = if v.primary { v.line0 as i64 } else { 0 };
    for pl in &p.list {
        let Some(s) = shown(pl.image, pl.gen) else {
            continue;
        };
        let row = pl.line.saturating_sub(top) as f32;
        let dst = [
            pl.col as f32 * cw + pl.x_off as f32,
            row * ch + pl.y_off as f32,
            pl.width as f32,
            pl.height as f32,
        ];
        if !visible(dst) {
            continue;
        }
        v.pictures.push(Picture {
            key: s.key,
            image: Arc::clone(&s.image),
            src: sent_px(&s.image, pl.src.map(|n| n as f32)),
            dst,
            z: pl.z,
        });
    }
    for (row, run) in &v.holders {
        let Some(t) = kitty::target(&p.virtuals, run) else {
            continue;
        };
        let Some(s) = shown(t.image, t.gen) else {
            continue;
        };
        let size = (s.image.source_width, s.image.source_height);
        let Some(piece) = kitty::piece(run, &t, size, (p.cell_w, p.cell_h)) else {
            continue;
        };
        let (x, y) = (f32::from(run.x) * cw, f32::from(*row) * ch);
        let d = piece.dst;
        v.pictures.push(Picture {
            key: s.key,
            image: Arc::clone(&s.image),
            src: sent_px(&s.image, piece.src),
            dst: [x + d[0], y + d[1], d[2], d[3]],
            z: 0,
        });
    }
    v.pictures.sort_by_key(|p| p.z);
}

/// `src` in an image's source pixels as pixels of the copy it was sent, which may be shrunk.
fn sent_px(image: &Image, src: [f32; 4]) -> [f32; 4] {
    let kx = image.width as f32 / image.source_width.max(1) as f32;
    let ky = image.height as f32 / image.source_height.max(1) as f32;
    [src[0] * kx, src[1] * ky, src[2] * kx, src[3] * ky]
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

    /// The lowest row anything is drawn on, the cursor's included: what a clipped card keeps in view.
    pub fn last_row(&self) -> u16 {
        self.runs
            .iter()
            .filter(|r| !self.run_text(r).trim().is_empty())
            .map(|r| r.row)
            .fold(self.cursor_y, u16::max)
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
        assert_eq!(xterm(1, &look::term().ansi), 0xef4444);
        assert_eq!(xterm(16, &look::term().ansi), 0x000000);
        assert_eq!(xterm(231, &look::term().ansi), 0xffffff);
        assert_eq!(xterm(232, &look::term().ansi), 0x080808);
        assert_eq!(xterm(196, &look::term().ansi), 0xff0000);
    }

    #[test]
    fn start_colours_draw_in_the_theme_and_a_programs_own_stay() {
        let mut snap = scrolled_snapshot();
        let t = |[r, g, b]: [u8; 3]| (r, g, b);
        let c = &mut snap.term.colors;
        c.fg = Some(t(screen::START_FG));
        c.bg = Some((1, 2, 3));
        c.cursor = Some(t(screen::START_FG));
        c.palette = (0..=255).map(|i| (i, t(screen::start_entry(i)))).collect();
        c.palette[2].1 = (9, 9, 9);
        let m = Mirror::from_snapshot(snap).expect("valid snapshot");
        let theme = Term {
            fg: 0xaaaaaa,
            bg: 0x111111,
            cursor: 0xbbbbbb,
            selection: 0,
            ansi: [0x010101; 16],
        };
        let p = Palette::of(&m, theme);
        assert_eq!((p.fg, p.bg), (0xaaaaaa, 0x010203));
        assert_eq!(p.color(Color::Palette(1), 0), 0x010101);
        assert_eq!(p.color(Color::Palette(2), 0), 0x090909);
        assert_eq!(p.color(Color::Palette(196), 0), 0xff0000);
        assert_eq!(p.over.len(), 1, "only the entry a program set overrides");
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
        Mirror::from_snapshot(scrolled_snapshot()).expect("valid snapshot")
    }

    fn scrolled_snapshot() -> vorn_term_proto::screen::Snapshot {
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
        Snapshot {
            state_gen: 1,
            rev: 1,
            row_fmt: ROW_FMT,
            term,
            styles: vec![StyleDef::default()],
            links: vec![LinkDef::default()],
            rows: vec![at(0, 2, "two"), at(1, 3, "three"), at(2, 4, "four")],
            history: vec![at(0, 0, "zero"), at(0, 1, "one")],
            ..Snapshot::default()
        }
    }

    fn shown(id: u32, gen: u64, (width, height): (u32, u32)) -> Shown {
        Shown {
            key: u64::from(id) << 8 | gen,
            image: Arc::new(Image {
                id,
                gen,
                width,
                height,
                source_width: width,
                source_height: height,
                rgba: vec![255; (width * height * 4) as usize],
            }),
        }
    }

    #[test]
    fn crops_of_a_shrunk_image_scale_to_the_copy_sent() {
        let mut s = shown(1, 1, (1024, 683));
        let img = Arc::get_mut(&mut s.image).unwrap();
        (img.source_width, img.source_height) = (6000, 4000);
        let src = sent_px(&s.image, [3000.0, 0.0, 3000.0, 4000.0]);
        assert_eq!(src, [512.0, 0.0, 512.0, 683.0]);
    }

    fn placed(image: u32, line: i64, z: i32) -> vorn_term_proto::screen::Placement {
        vorn_term_proto::screen::Placement {
            image,
            gen: 1,
            line,
            col: 1,
            x_off: 2,
            y_off: 3,
            width: 20,
            height: 40,
            src: [0, 0, 4, 8],
            z,
        }
    }

    #[test]
    fn pictures_sit_on_their_lines_as_the_view_scrolls() {
        use vorn_term_proto::screen::Placements;
        let mut snap = scrolled_snapshot();
        snap.placements = Some(Placements {
            cell_w: 10,
            cell_h: 20,
            // Line 0 is two rows above the live screen: off it but for a part of this one.
            list: vec![
                placed(1, 3, 5),
                placed(2, 3, -1),
                vorn_term_proto::screen::Placement {
                    y_off: 0,
                    ..placed(1, 0, 0)
                },
                placed(1, 0, 0),
                placed(9, 2, 0),
            ],
            virtuals: Vec::new(),
        });
        let m = Mirror::from_snapshot(snap).expect("valid snapshot");
        let images = [shown(1, 1, (4, 8)), shown(2, 1, (4, 8))];
        let look = |id: u32| images.iter().find(|s| s.image.id == id);
        let mut live = build(&m, None);
        pictures(&mut live, &m, look);
        assert_eq!(live.cell_px, (10, 20));
        let at: Vec<_> = live
            .pictures
            .iter()
            .map(|p| (p.image.id, p.z, p.dst))
            .collect();
        // Lowest z first; the unknown image 9 and the one wholly scrolled off are left out.
        assert_eq!(
            at,
            [
                (2, -1, [12.0, 23.0, 20.0, 40.0]),
                (1, 0, [12.0, -37.0, 20.0, 40.0]),
                (1, 5, [12.0, 23.0, 20.0, 40.0])
            ]
        );
        assert_eq!(live.pictures[0].src, [0.0, 0.0, 4.0, 8.0]);
        let mut back = build(&m, Some(0));
        pictures(&mut back, &m, look);
        let ys: Vec<_> = back
            .pictures
            .iter()
            .map(|p| (p.image.id, p.dst[1]))
            .collect();
        assert_eq!(ys, [(2, 63.0), (1, 0.0), (1, 3.0), (1, 63.0)]);
        // A newer generation than the pixels held is not drawn with the old ones.
        let stale = [shown(1, 2, (4, 8))];
        pictures(&mut live, &m, |id| stale.iter().find(|s| s.image.id == id));
        assert!(live.pictures.is_empty());
    }

    #[test]
    fn placeholder_cells_draw_blank_with_their_piece_of_the_image() {
        use vorn_term_proto::row::{Cell, Run as Cells};
        use vorn_term_proto::screen::{Placements, Virtual};
        let mut snap = scrolled_snapshot();
        snap.styles.push(StyleDef {
            id: 1,
            fg: Color::Palette(7),
            ..StyleDef::default()
        });
        let holder = |marks: &str| Cell::new(format!("{}{marks}", kitty::PLACEHOLDER));
        snap.rows[0].cells = row::encode(&[
            Cells {
                style: 0,
                link: 0,
                cells: vec![Cell::new("$".to_string())],
            },
            Cells {
                style: 1,
                link: 0,
                cells: vec![holder("\u{305}\u{305}"), holder("")],
            },
        ]);
        let m = Mirror::from_snapshot(snap.clone()).expect("valid snapshot");
        let mut v = build(&m, None);
        assert_eq!(
            v.text().lines().next(),
            Some("$\u{10EEEE}\u{305}\u{305}\u{10EEEE}")
        );
        assert!(v.holders.is_empty(), "no virtual placement, nothing parsed");
        snap.placements = Some(Placements {
            cell_w: 10,
            cell_h: 20,
            list: Vec::new(),
            virtuals: vec![Virtual {
                image: 7,
                gen: 1,
                placement: 0,
                cols: 2,
                rows: 1,
            }],
        });
        let m = Mirror::from_snapshot(snap).expect("valid snapshot");
        v = build(&m, None);
        assert_eq!(v.text().lines().next(), Some("$"));
        assert_eq!(v.holders.len(), 1);
        assert_eq!(
            (v.holders[0].0, v.holders[0].1.x, v.holders[0].1.width),
            (0, 1, 2)
        );
        let images = [shown(7, 1, (20, 20))];
        pictures(&mut v, &m, |id| images.iter().find(|s| s.image.id == id));
        let at: Vec<_> = v.pictures.iter().map(|p| (p.src, p.dst)).collect();
        assert_eq!(at, [([0.0, 0.0, 20.0, 20.0], [10.0, 0.0, 20.0, 20.0])]);
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
