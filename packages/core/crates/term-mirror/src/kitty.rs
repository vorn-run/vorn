//! Kitty unicode placeholders: cells whose text is U+10EEEE draw a piece of a virtual placement,
//! the image id in their foreground colour, the placement id in their underline colour, and the
//! row, column and high id byte in up to three diacritics. A client lays the pieces out itself.

use vorn_term_proto::screen::{Color, Virtual};

/// The placeholder character.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// Whether a cell's text is a placeholder.
pub fn is_placeholder(text: &str) -> bool {
    text.starts_with(PLACEHOLDER)
}

/// What one placeholder cell says; anything left out continues the run before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    low: u32,
    high: Option<u8>,
    placement: u32,
    row: Option<u32>,
    col: Option<u32>,
}

impl Cell {
    /// The placeholder in a cell with this text and colours, if it is one.
    pub fn parse(text: &str, fg: Color, underline: Color) -> Option<Cell> {
        let mut marks = text.strip_prefix(PLACEHOLDER)?.chars().map(index);
        let row = marks.next().flatten();
        let col = marks.next().flatten();
        let high = marks.next().flatten().and_then(|h| u8::try_from(h).ok());
        Some(Cell {
            low: color_id(fg),
            high,
            placement: color_id(underline),
            row,
            col,
        })
    }
}

/// A run of placeholder cells on one row: one row of cells' worth of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// The screen column of its first cell.
    pub x: u16,
    pub width: u32,
    pub image: u32,
    /// Zero when the cells name none.
    pub placement: u32,
    /// The cell of the placement's grid its first cell shows.
    pub row: u32,
    pub col: u32,
}

/// Joins a row's placeholder cells into runs, cell by cell.
#[derive(Debug, Default)]
pub struct Runs {
    cur: Option<(u16, u32, Cell)>,
}

impl Runs {
    /// The cell at column `x`, `None` when it is no placeholder; a run it ends goes to `out`.
    pub fn push(&mut self, x: u16, cell: Option<Cell>, out: &mut Vec<Run>) {
        let Some(c) = cell else {
            return self.end(out);
        };
        if let Some((start, width, run)) = &mut self.cur {
            let joins = u32::from(x) == u32::from(*start) + *width
                && c.low == run.low
                && c.placement == run.placement
                && c.row.is_none_or(|r| Some(r) == run.row)
                && c.col.is_none_or(|col| Some(col) == run.col.map(|rc| rc + *width))
                && c.high.is_none_or(|h| Some(h) == run.high);
            if joins {
                *width += 1;
                return;
            }
            self.end(out);
        }
        let started = Cell {
            row: Some(c.row.unwrap_or(0)),
            col: Some(c.col.unwrap_or(0)),
            ..c
        };
        self.cur = Some((x, 1, started));
    }

    /// Ends the row: its last run goes to `out`.
    pub fn end(&mut self, out: &mut Vec<Run>) {
        if let Some((x, width, c)) = self.cur.take() {
            out.push(Run {
                x,
                width,
                image: c.low | u32::from(c.high.unwrap_or(0)) << 24,
                placement: c.placement,
                row: c.row.unwrap_or(0),
                col: c.col.unwrap_or(0),
            });
        }
    }
}

/// The virtual placement a run draws: the one it names, else the image's lowest placement id.
pub fn target(virtuals: &[Virtual], run: &Run) -> Option<Virtual> {
    let of_image = virtuals.iter().filter(|v| v.image == run.image);
    if run.placement != 0 {
        of_image.copied().find(|v| v.placement == run.placement)
    } else {
        of_image.copied().min_by_key(|v| v.placement)
    }
}

/// Where a run draws, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Piece {
    /// The part of the image: x, y, width, height in image pixels.
    pub src: [f32; 4],
    /// Where it goes: x, y, width, height from the top left of the run's first cell, in the
    /// server's cell pixels.
    pub dst: [f32; 4],
}

/// The piece of `image` (width, height) a run of placement `v` shows, with cells `cell` pixels:
/// the image fit into the placement's grid keeping its aspect ratio, centred, and cut to the
/// run's cells. `None` when the run shows only the padding around it.
pub fn piece(run: &Run, v: &Virtual, image: (u32, u32), cell: (u32, u32)) -> Option<Piece> {
    if v.cols == 0 || v.rows == 0 || cell.0 == 0 || cell.1 == 0 {
        return None;
    }
    let (img_w, img_h) = (f64::from(image.0), f64::from(image.1));
    let (cols, rows) = (f64::from(v.cols), f64::from(v.rows));
    let grid_w = cols * f64::from(cell.0);
    let grid_h = rows * f64::from(cell.1);
    // The scale fitting the image into the grid, and the padding that centres it.
    let (scale, pad_x, pad_y) = if img_w * grid_h > img_h * grid_w {
        let s = grid_w / img_w.max(1.0);
        (s, 0.0, (grid_h - img_h * s) / 2.0)
    } else {
        let s = grid_h / img_h.max(1.0);
        (s, (grid_w - img_w * s) / 2.0, 0.0)
    };
    // The padded image in image pixels, which the grid cuts evenly.
    let (off_x, off_y) = (pad_x / scale, pad_y / scale);
    let (full_w, full_h) = (img_w + off_x * 2.0, img_h + off_y * 2.0);
    let (mut sx, mut sw) = (
        full_w * f64::from(run.col) / cols,
        full_w * f64::from(run.width) / cols,
    );
    let (mut sy, mut sh) = (full_h * f64::from(run.row) / rows, full_h / rows);
    let (dx, dw) = clip(&mut sx, &mut sw, off_x, full_w, img_w, scale, f64::from(run.width * cell.0));
    let (dy, dh) = clip(&mut sy, &mut sh, off_y, full_h, img_h, scale, f64::from(cell.1));
    if sw <= 0.0 || sh <= 0.0 {
        return None;
    }
    let r = |v: f64| v.round() as f32;
    Some(Piece {
        src: [r(sx), r(sy), r(sw), r(sh)],
        dst: [r(dx * scale), r(dy * scale), r(dw), r(dh)],
    })
}

/// One axis of [`piece`]: takes the padding off the source span `s`..`s + len` and answers the
/// destination offset (in image pixels) and length (in grid pixels).
fn clip(
    s: &mut f64,
    len: &mut f64,
    off: f64,
    full: f64,
    img: f64,
    scale: f64,
    cells: f64,
) -> (f64, f64) {
    let mut d_off = 0.0;
    let mut d_len = cells;
    if *s < off {
        let cut = off - *s;
        *len -= cut;
        d_off = cut;
        d_len -= cut * scale;
        *s = 0.0;
        if *len > img {
            *len = img;
            d_len = img * scale;
        }
    } else if *s + *len > full - off {
        *s -= off;
        *len = full - off - *s - off;
        d_len = *len * scale;
    } else {
        *s -= off;
    }
    (d_off, d_len)
}

/// A diacritic's value: its place in the table.
fn index(c: char) -> Option<u32> {
    DIACRITICS.binary_search(&u32::from(c)).ok().map(|i| i as u32)
}

/// The 24 bits of an id a colour carries.
fn color_id(c: Color) -> u32 {
    match c {
        Color::Default => 0,
        Color::Palette(i) => u32::from(i),
        Color::Rgb(r, g, b) => u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW0: char = '\u{0305}';
    const ROW1: char = '\u{030D}';
    const ROW2: char = '\u{030E}';

    fn cell(marks: &str, fg: Color, underline: Color) -> Option<Cell> {
        Cell::parse(&format!("{PLACEHOLDER}{marks}"), fg, underline)
    }

    fn runs(cells: &[Option<Cell>]) -> Vec<Run> {
        let (mut runs, mut out) = (Runs::default(), Vec::new());
        for (x, c) in cells.iter().enumerate() {
            runs.push(x as u16, *c, &mut out);
        }
        runs.end(&mut out);
        out
    }

    #[test]
    fn diacritics_are_their_place_in_the_table() {
        assert_eq!(DIACRITICS.len(), 297);
        assert!(DIACRITICS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!((index(ROW0), index(ROW2), index('\u{1D244}')), (Some(0), Some(2), Some(296)));
        assert_eq!(index('a'), None);
        assert_eq!(Cell::parse("x", Color::Palette(1), Color::Default), None);
    }

    #[test]
    fn cells_with_no_marks_continue_the_run_before_them() {
        let id = Color::Rgb(0, 1, 2);
        let first = cell(&format!("{ROW1}{ROW2}"), id, Color::Default);
        let bare = cell("", id, Color::Default);
        let got = runs(&[None, first, bare, bare, None, bare]);
        let run = |x, width, row, col| Run { x, width, image: 258, placement: 0, row, col };
        assert_eq!(got, [run(1, 3, 1, 2), run(5, 1, 0, 0)]);
    }

    #[test]
    fn a_cell_that_does_not_follow_starts_a_new_run() {
        let a = Color::Palette(7);
        let at = |row: char, col: char| cell(&format!("{row}{col}"), a, Color::Palette(3));
        let other_image = cell("", Color::Palette(8), Color::Palette(3));
        let got = runs(&[at(ROW0, ROW0), at(ROW0, ROW1), at(ROW0, ROW0), at(ROW1, ROW1), other_image]);
        let widths: Vec<_> = got.iter().map(|r| (r.width, r.row, r.col, r.image)).collect();
        assert_eq!(widths, [(2, 0, 0, 7), (1, 0, 0, 7), (1, 1, 1, 7), (1, 0, 0, 8)]);
        assert!(got.iter().all(|r| r.placement == 3));
    }

    #[test]
    fn the_third_mark_is_the_high_byte_of_the_image_id() {
        let c = cell(&format!("{ROW0}{ROW0}{ROW2}"), Color::Palette(5), Color::Default);
        assert_eq!(runs(&[c])[0].image, 2 << 24 | 5);
    }

    #[test]
    fn a_run_draws_the_placement_it_names_or_the_lowest() {
        let v = |placement| Virtual { image: 4, gen: 1, placement, cols: 2, rows: 2 };
        let virtuals = [v(9), v(3), Virtual { image: 5, ..v(1) }];
        let run = |placement| Run { x: 0, width: 1, image: 4, placement, row: 0, col: 0 };
        assert_eq!(target(&virtuals, &run(9)).map(|v| v.placement), Some(9));
        assert_eq!(target(&virtuals, &run(0)).map(|v| v.placement), Some(3));
        assert_eq!(target(&virtuals, &run(2)), None);
    }

    #[test]
    fn an_image_the_grid_fits_is_cut_evenly() {
        let v = Virtual { image: 1, gen: 1, placement: 0, cols: 4, rows: 2 };
        let run = Run { x: 0, width: 2, image: 1, placement: 0, row: 1, col: 1 };
        let p = piece(&run, &v, (20, 10), (10, 10)).unwrap();
        assert_eq!((p.src, p.dst), ([5.0, 5.0, 10.0, 5.0], [0.0, 0.0, 20.0, 10.0]));
    }

    #[test]
    fn a_narrow_image_is_centred_and_its_padding_draws_nothing() {
        let v = Virtual { image: 1, gen: 1, placement: 0, cols: 4, rows: 2 };
        let run = |col, width| Run { x: 0, width, image: 1, placement: 0, row: 0, col };
        assert_eq!(piece(&run(0, 1), &v, (10, 10), (10, 10)), None);
        assert_eq!(piece(&run(3, 1), &v, (10, 10), (10, 10)), None);
        let inner = piece(&run(1, 1), &v, (10, 10), (10, 10)).unwrap();
        assert_eq!((inner.src, inner.dst), ([0.0, 0.0, 5.0, 5.0], [0.0, 0.0, 10.0, 10.0]));
        let whole = piece(&run(0, 4), &v, (10, 10), (10, 10)).unwrap();
        assert_eq!((whole.src, whole.dst), ([0.0, 0.0, 10.0, 5.0], [10.0, 0.0, 20.0, 10.0]));
    }
}

/// The row and column diacritics, in order of the value they stand for.
#[rustfmt::skip]
const DIACRITICS: [u32; 297] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B, 0x034C,
    0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369,
    0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592,
    0x0593, 0x0594, 0x0595, 0x0597, 0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1,
    0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
    0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6, 0x06D7, 0x06D8,
    0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2, 0x06E4, 0x06E7, 0x06E8, 0x06EB,
    0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743,
    0x0745, 0x0747, 0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3,
    0x0816, 0x0817, 0x0818, 0x0819, 0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
    0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C, 0x082D, 0x0951, 0x0953, 0x0954,
    0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
    0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71,
    0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5,
    0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
    0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7,
    0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2,
    0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE,
    0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1,
    0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED,
    0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1,
    0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186,
    0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];
