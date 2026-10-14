//! The Cells surface: the same element tree laid out in terminal cells
//! (one column by one row) and drawn as characters.
//!
//! [`Ui::layout`](crate::Ui::layout) on a cells [`Ui`](crate::Ui) converts
//! every length to cells ([`units`]), measures text by its display width,
//! and records a [`PaintList`] instead of shaping glyphs. [`Buffer::draw`]
//! rasterizes that list into a grid of [`Cell`]s; [`Buffer::diff`] writes
//! what changed since the last frame (through [`term`]).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::scene::{Rect, Rgba};

#[cfg(not(target_arch = "wasm32"))]
pub mod term;

/// How a layout length becomes cells. Spacing collapses to 0 or 1 cell,
/// sizes keep their proportion, borders become one box-drawing cell.
pub mod units {
    /// Logical pixels per column, the width of a terminal cell at 13 px.
    pub const COL: f32 = 8.0;
    /// Logical pixels per row for fixed heights: a control of 28 to 40 px is one row.
    pub const ROW: f32 = 28.0;
    /// Vertical spacing under this many pixels is no row at all.
    pub const ROW_GAP: f32 = 20.0;

    fn signed(px: f32, f: impl Fn(f32) -> f32) -> f32 {
        if px < 0.0 {
            -f(-px)
        } else {
            f(px)
        }
    }

    /// A width: proportional, and never a nonzero box squeezed to nothing.
    pub fn width(px: f32) -> f32 {
        signed(px, |p| {
            if p <= 0.0 {
                0.0
            } else {
                (p / COL).round().max(1.0)
            }
        })
    }

    /// A height: one row per control (28 to 55 px), more for tall boxes.
    pub fn height(px: f32) -> f32 {
        signed(px, |p| {
            if p <= 0.0 {
                0.0
            } else {
                (p / ROW).floor().max(1.0)
            }
        })
    }

    /// Horizontal padding, margin or gap: 4 to 8 px is a column, 2 px is none.
    pub fn space_x(px: f32) -> f32 {
        signed(px, |p| (p / COL + 0.5).floor())
    }

    /// Vertical padding, margin or gap: rows are scarce, so most is none.
    pub fn space_y(px: f32) -> f32 {
        signed(px, |p| (p / ROW_GAP).floor())
    }

    /// A border: one cell of box drawing.
    pub fn border(px: f32) -> f32 {
        if px > 0.0 {
            1.0
        } else {
            0.0
        }
    }

    /// A `cols`×`rows` terminal as the window size views lay out by, in logical pixels.
    pub fn px(cols: u16, rows: u16) -> (f32, f32) {
        (f32::from(cols) * COL, f32::from(rows) * ROW)
    }
}

/// Display width of `s` in columns.
pub fn width(s: &str) -> usize {
    s.graphemes(true).map(grapheme_width).sum()
}

fn grapheme_width(g: &str) -> usize {
    match g {
        "\t" => 1,
        _ => UnicodeWidthStr::width(g).min(2),
    }
}

/// Lines `s` takes when wrapped at `max` columns (words kept whole when they fit).
pub fn wrap(s: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        let mut w = 0;
        for word in para.split_word_bounds() {
            let ww = width(word);
            if w + ww > max && w > 0 {
                lines.push(std::mem::take(&mut line).trim_end().to_owned());
                w = 0;
                if word.trim().is_empty() {
                    continue;
                }
            }
            if ww > max {
                for g in word.graphemes(true) {
                    let gw = grapheme_width(g);
                    if w + gw > max && w > 0 {
                        lines.push(std::mem::take(&mut line));
                        w = 0;
                    }
                    line.push_str(g);
                    w += gw;
                }
            } else {
                line.push_str(word);
                w += ww;
            }
        }
        lines.push(line.trim_end().to_owned());
    }
    lines
}

/// `s` with the Mac's modifier symbols spelled the way a terminal names
/// them: the command key is Alt there.
pub fn keys(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.contains(['⌘', '⌥', '⇧', '⌃']) {
        return s.into();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '⌘' | '⌥' => out.push_str("Alt+"),
            '⇧' => out.push_str("Shift+"),
            '⌃' => out.push_str("Ctrl+"),
            c => out.push(c),
        }
    }
    out.into()
}

/// `s` cut to `max` columns, ending in an ellipsis when it was cut.
pub fn ellipsize(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for g in s.graphemes(true) {
        let gw = grapheme_width(g);
        if w + gw > max - 1 {
            break;
        }
        out.push_str(g);
        w += gw;
    }
    out.push('…');
    out
}

/// A cell's text: one grapheme of up to 15 bytes, stored inline so a cell
/// is `Copy` and a frame allocates nothing per cell.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Sym {
    len: u8,
    bytes: [u8; 15],
}

impl Sym {
    pub const SPACE: Sym = Sym {
        len: 1,
        bytes: [b' ', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    };
    /// The second column of a wide character: nothing of its own.
    pub const TAIL: Sym = Sym {
        len: 0,
        bytes: [0; 15],
    };

    pub fn new(g: &str) -> Sym {
        let mut s = Sym::TAIL;
        // Longer clusters keep their first scalar.
        let g = if g.len() > 15 {
            g.chars().next().map_or(" ", |c| &g[..c.len_utf8()])
        } else {
            g
        };
        s.bytes[..g.len()].copy_from_slice(g.as_bytes());
        s.len = g.len() as u8;
        s
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or(" ")
    }
}

impl std::fmt::Debug for Sym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

/// Cell attributes.
pub mod attr {
    pub const BOLD: u8 = 1;
    pub const DIM: u8 = 2;
    pub const ITALIC: u8 = 4;
    pub const UNDERLINE: u8 = 8;
    pub const STRIKE: u8 = 16;
    pub const INVERSE: u8 = 32;
}

/// The icons cells frames refer to: each SVG's key, and the SVG of each key.
#[derive(Default)]
struct Icons {
    keys: std::collections::HashMap<String, u32>,
    svgs: Vec<String>,
}

static ICONS: std::sync::LazyLock<std::sync::Mutex<Icons>> =
    std::sync::LazyLock::new(Default::default);

/// A key for icon `svg`, the same for the same SVG for the life of the process.
pub fn icon_key(svg: &str) -> u32 {
    let mut icons = ICONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(k) = icons.keys.get(svg) {
        return *k;
    }
    icons.svgs.push(svg.to_owned());
    let key = u32::try_from(icons.svgs.len()).unwrap_or(0);
    icons.keys.insert(svg.to_owned(), key);
    key
}

/// The SVG icon `key` stands for.
pub fn icon_svg(key: u32) -> Option<String> {
    let icons = ICONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    icons
        .svgs
        .get(usize::try_from(key).ok()?.checked_sub(1)?)
        .cloned()
}

/// One terminal cell: its grapheme, colours (sRGB) and attributes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub sym: Sym,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub attr: u8,
    /// The icon this cell stands for ([`icon_key`]), 0 for none: a terminal
    /// that shows images draws the icon itself, the rest the glyph in `sym`.
    pub icon: u32,
}

impl Cell {
    pub fn blank(bg: [u8; 3]) -> Cell {
        Cell {
            sym: Sym::SPACE,
            fg: [255; 3],
            bg,
            attr: 0,
            icon: 0,
        }
    }
}

/// Where and how the terminal's own cursor shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CursorAt {
    pub col: u16,
    pub row: u16,
    pub shape: CursorShape,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

/// What a cells frame draws, in the order it was recorded.
#[derive(Debug, Clone)]
pub enum Op {
    /// A box's background.
    Fill { color: Rgba },
    /// A box's border as box drawing, rounded where the pixel box is;
    /// `open` draws only its top and left edges, its neighbours the rest.
    Frame {
        color: Rgba,
        round: bool,
        open: bool,
    },
    /// One side's rule: 0 top, 1 right, 2 bottom, 3 left.
    Rule { side: u8, color: Rgba },
    /// A coloured accent bar down a box's left edge, a selection's marker.
    Bar { color: Rgba },
    /// A scrollbar: a faint track the box's height, the thumb over rows
    /// `thumb.0..thumb.0 + thumb.1`.
    Scroll { thumb: (i32, i32), color: Rgba },
    /// A run of text: bold, dim and italic come from its role.
    Text {
        text: String,
        color: Rgba,
        attr: u8,
        wrap: bool,
    },
    /// An icon's glyph, centred in its box.
    /// `icon` is the drawn icon's [`icon_key`], 0 for a plain glyph.
    Glyph {
        glyph: String,
        color: Rgba,
        icon: u32,
    },
    /// Cells copied as they are (a terminal pane), `cols` wide, row by row.
    Cells { cols: u16, cells: Vec<Cell> },
}

/// One recorded op: what, where, under which clip and layer, at what opacity.
#[derive(Debug, Clone)]
pub struct Item {
    pub op: Op,
    pub rect: Rect,
    pub clip: Rect,
    pub layer: u32,
    pub alpha: f32,
}

/// The frame as a cells backend sees it.
#[derive(Debug, Default)]
pub struct PaintList {
    pub items: Vec<Item>,
    pub cursor: Option<CursorAt>,
    pub clear: Rgba,
    alpha: Vec<f32>,
    pub(crate) clip: Option<Rect>,
    pub(crate) layer: u32,
}

impl PaintList {
    pub fn begin(&mut self, clear: Rgba) {
        self.items.clear();
        self.cursor = None;
        self.clear = clear;
        self.alpha.clear();
        self.clip = None;
        self.layer = 0;
    }

    pub(crate) fn push_alpha(&mut self, a: f32) {
        let a = self.opacity() * a;
        self.alpha.push(a);
    }

    pub(crate) fn pop_alpha(&mut self) {
        self.alpha.pop();
    }

    fn opacity(&self) -> f32 {
        self.alpha.last().copied().unwrap_or(1.0)
    }

    /// Records `op` over `rect` under the current clip, unless it is invisible.
    pub fn push(&mut self, op: Op, rect: Rect) {
        let alpha = self.opacity();
        if alpha < 0.05 {
            return;
        }
        self.items.push(Item {
            op,
            rect,
            clip: self.clip.unwrap_or(Rect::EVERYTHING),
            layer: self.layer,
            alpha,
        });
    }

    /// Records `op` over `rect` in the window's own layer, under `clip`:
    /// what the app paints into its custom boxes after layout.
    pub fn push_app(&mut self, op: Op, rect: Rect, clip: Rect) {
        self.items.push(Item {
            op,
            rect,
            clip,
            layer: 0,
            alpha: 1.0,
        });
    }
}

/// How many colours the terminal shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Depth {
    True,
    Ansi256,
    Ansi16,
}

impl Depth {
    /// From `COLORTERM` and `TERM`, as terminals advertise it.
    pub fn from_env() -> Depth {
        Depth::detect(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    }

    pub fn detect(colorterm: Option<&str>, term: Option<&str>) -> Depth {
        let ct = colorterm.unwrap_or_default().to_ascii_lowercase();
        if ct.contains("truecolor") || ct.contains("24bit") {
            Depth::True
        } else if term.unwrap_or_default().contains("256color") {
            Depth::Ansi256
        } else {
            Depth::Ansi16
        }
    }
}

fn to8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// `c` at alpha `a` over the opaque `under`.
fn over(under: [u8; 3], c: Rgba, a: f32) -> [u8; 3] {
    let a = (c.0[3] * a).clamp(0.0, 1.0);
    let mut out = [0; 3];
    for i in 0..3 {
        let u = f32::from(under[i]) / 255.0;
        out[i] = to8(u + (c.0[i] - u) * a);
    }
    out
}

/// A translucent white over a surface is a hover or selection tint, too
/// faint to read in cells; this many times stronger reads as one step brighter.
const TINT_BOOST: f32 = 2.0;
const LINE_BOOST: f32 = 2.5;

pub(crate) fn is_tint(c: Rgba) -> bool {
    c.0[3] < 0.2 && c.0[0] > 0.9 && c.0[1] > 0.9 && c.0[2] > 0.9
}

/// A frame of cells.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Buffer {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
    pub cursor: Option<CursorAt>,
}

/// Ranges of cells in `clip ∩ r`, in whole cells.
fn span(r: Rect, clip: Rect, cols: u16, rows: u16) -> (i32, i32, i32, i32) {
    let v = r.intersect(&clip);
    let x0 = (v.x.round() as i32).max(0);
    let y0 = (v.y.round() as i32).max(0);
    let x1 = (v.right().round() as i32).min(i32::from(cols));
    let y1 = (v.bottom().round() as i32).min(i32::from(rows));
    (x0, y0, x1, y1)
}

impl Buffer {
    pub fn new(cols: u16, rows: u16) -> Buffer {
        Buffer {
            cols,
            rows,
            cells: vec![Cell::blank([0; 3]); usize::from(cols) * usize::from(rows)],
            cursor: None,
        }
    }

    pub fn get(&self, col: u16, row: u16) -> Option<&Cell> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.cells
            .get(usize::from(row) * usize::from(self.cols) + usize::from(col))
    }

    /// The text of row `row`, for tests and logs.
    pub fn row_text(&self, row: u16) -> String {
        (0..self.cols)
            .filter_map(|c| self.get(c, row))
            .map(|c| c.sym.as_str())
            .collect()
    }

    /// Rasterizes `list` into this buffer, replacing what was there.
    pub fn draw(&mut self, list: &PaintList) {
        let clear = over([0; 3], list.clear, 1.0);
        self.cells.fill(Cell::blank(clear));
        self.cursor = list.cursor;
        let mut order: Vec<&Item> = list.items.iter().collect();
        order.sort_by_key(|i| i.layer);
        let n = self.cells.len();
        let mut r = Raster {
            buf: self,
            line: vec![None; n],
            gap: vec![false; n],
        };
        for item in order {
            r.item(item);
        }
        r.join();
    }

    /// Cells that differ from `prev`, row-major: every cell when the size changed.
    pub fn diff<'a>(
        &'a self,
        prev: Option<&'a Buffer>,
    ) -> impl Iterator<Item = (u16, u16, &'a Cell)> + 'a {
        let same_size = prev.is_some_and(|p| p.cols == self.cols && p.rows == self.rows);
        let cols = self.cols;
        self.cells.iter().enumerate().filter_map(move |(i, c)| {
            if same_size && prev.and_then(|p| p.cells.get(i)) == Some(c) {
                return None;
            }
            Some((
                (i % usize::from(cols)) as u16,
                (i / usize::from(cols)) as u16,
                c,
            ))
        })
    }

    /// The frame as a standalone HTML page, for snapshots next to the pixel render.
    pub fn to_html(&self, title: &str) -> String {
        let mut out = String::new();
        out.push_str("<!doctype html><meta charset=utf-8><title>");
        out.push_str(&escape(title));
        out.push_str("</title><style>body{margin:0;background:#000}pre{margin:0;font:13px/16px Menlo,'DejaVu Sans Mono',monospace;display:inline-block}span{white-space:pre}</style><pre>");
        for row in 0..self.rows {
            let mut run: Option<(Cell, String)> = None;
            for col in 0..self.cols {
                let Some(c) = self.get(col, row) else {
                    continue;
                };
                if c.sym == Sym::TAIL {
                    continue;
                }
                let mut style = *c;
                style.sym = Sym::SPACE;
                if let Some((s, text)) = &mut run {
                    if *s == style {
                        text.push_str(c.sym.as_str());
                        continue;
                    }
                }
                if let Some((s, text)) = run.take() {
                    span_html(&mut out, &s, &text);
                }
                run = Some((style, c.sym.as_str().to_owned()));
            }
            if let Some((s, text)) = run.take() {
                span_html(&mut out, &s, &text);
            }
            out.push('\n');
        }
        out.push_str("</pre>");
        out
    }
}

/// Box-drawing arms a line cell reaches out with.
mod arm {
    pub const U: u8 = 1;
    pub const R: u8 = 2;
    pub const D: u8 = 4;
    pub const L: u8 = 8;
    pub const H: u8 = L | R;
    pub const V: u8 = U | D;

    /// The arm pointing back from the neighbour in direction `a`.
    pub fn back(a: u8) -> u8 {
        match a {
            U => D,
            D => U,
            L => R,
            _ => L,
        }
    }

    pub fn of(s: &str) -> u8 {
        match s {
            "─" => H,
            "│" => V,
            "┌" | "╭" => R | D,
            "┐" | "╮" => L | D,
            "└" | "╰" => U | R,
            "┘" | "╯" => U | L,
            "├" => V | R,
            "┤" => V | L,
            "┬" => H | D,
            "┴" => H | U,
            "┼" => H | V,
            _ => 0,
        }
    }

    pub fn glyph(a: u8, round: bool) -> &'static str {
        match (a, round) {
            (0b1111, _) => "┼",
            (x, _) if x == V | R => "├",
            (x, _) if x == V | L => "┤",
            (x, _) if x == H | D => "┬",
            (x, _) if x == H | U => "┴",
            (x, true) if x == R | D => "╭",
            (x, true) if x == L | D => "╮",
            (x, true) if x == U | R => "╰",
            (x, true) if x == U | L => "╯",
            (x, false) if x == R | D => "┌",
            (x, false) if x == L | D => "┐",
            (x, false) if x == U | R => "└",
            (x, false) if x == U | L => "┘",
            (x, _) if x & V != 0 && x & H == 0 => "│",
            _ => "─",
        }
    }

    pub fn round(s: &str) -> bool {
        matches!(s, "╭" | "╮" | "╰" | "╯")
    }
}

/// [`Buffer::draw`]'s state: which cells hold box drawing and on which
/// layer, and which a collapsed edge left empty.
struct Raster<'a> {
    buf: &'a mut Buffer,
    line: Vec<Option<u32>>,
    gap: Vec<bool>,
}

/// The 4-neighbour offsets, with the arm that reaches each.
const AROUND: [(i32, i32, u8); 4] = [
    (0, -1, arm::U),
    (1, 0, arm::R),
    (0, 1, arm::D),
    (-1, 0, arm::L),
];

impl Raster<'_> {
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (cols, rows) = (i32::from(self.buf.cols), i32::from(self.buf.rows));
        (x >= 0 && y >= 0 && x < cols && y < rows).then(|| (y * cols + x) as usize)
    }

    /// The arms of the box drawing at `(x, y)` on `layer`, 0 for anything else.
    fn arms(&self, x: i32, y: i32, layer: u32) -> u8 {
        match self.index(x, y) {
            Some(i) if self.line[i] == Some(layer) => arm::of(self.buf.cells[i].sym.as_str()),
            _ => 0,
        }
    }

    /// Whether a line runs through `(x, y)` along `axis` on `layer`.
    fn runs(&self, x: i32, y: i32, axis: u8, layer: u32) -> bool {
        self.arms(x, y, layer) & axis == axis
    }

    /// Marks a cell the box no longer draws on, its edge moved onto a neighbour's.
    fn vacate(&mut self, x: i32, y: i32) {
        if let Some(i) = self.index(x, y) {
            self.gap[i] = true;
        }
    }

    /// Draws `arms` at `(x, y)`, merged with the line already there on the same layer.
    #[allow(clippy::too_many_arguments)]
    fn line(&mut self, x: i32, y: i32, arms: u8, round: bool, color: Rgba, alpha: f32, layer: u32) {
        let Some(i) = self.index(x, y) else { return };
        // Hairlines a pixel surface draws at 6-8% would vanish as glyph strokes.
        let alpha = if is_tint(color) {
            alpha * LINE_BOOST
        } else {
            alpha
        };
        let c = &mut self.buf.cells[i];
        let (arms, round) = if self.line[i] == Some(layer) {
            let old = c.sym.as_str();
            (arms | arm::of(old), round || arm::round(old))
        } else {
            (arms, round)
        };
        c.sym = Sym::new(arm::glyph(arms, round));
        c.fg = over(c.bg, color, alpha);
        c.attr = 0;
        self.line[i] = Some(layer);
        self.gap[i] = false;
    }

    /// A box's edges: the left and top only when `open` (its neighbours
    /// draw the others), and an edge that meets a parallel line on the
    /// same layer collapses onto it.
    fn frame(&mut self, item: &Item, round: bool, open: bool, color: Rgba) {
        let (cols, rows) = (self.buf.cols, self.buf.rows);
        let (cx0, cy0, cx1, cy1) = span(item.rect, item.clip, cols, rows);
        let r = item.rect;
        let (rx0, ry0) = (r.x.round() as i32, r.y.round() as i32);
        let (rx1, ry1) = (r.right().round() as i32, r.bottom().round() as i32);
        let (w, h) = (rx1 - rx0, ry1 - ry0);
        let (layer, alpha) = (item.layer, item.alpha);
        if w <= 0 || h <= 0 {
            return;
        }
        if w == 1 || h == 1 {
            let a = if h == 1 { arm::H } else { arm::V };
            for y in cy0..cy1 {
                for x in cx0..cx1 {
                    self.line(x, y, a, false, color, alpha, layer);
                }
            }
            return;
        }
        let (xm, ym) = (rx0 + w / 2, ry0 + h / 2);
        let left = self.runs(rx0 - 1, ym, arm::V, layer);
        let top = self.runs(xm, ry0 - 1, arm::H, layer);
        let right = !open && self.runs(rx1, ym, arm::V, layer);
        let bottom = !open && self.runs(xm, ry1, arm::H, layer);
        let x0 = if left { rx0 - 1 } else { rx0 };
        let y0 = if top { ry0 - 1 } else { ry0 };
        let x1 = if right { rx1 } else { rx1 - 1 };
        let y1 = if bottom { ry1 } else { ry1 - 1 };
        let inside = |x: i32, y: i32| x >= cx0 && x < cx1 && y >= cy0 && y < cy1;
        for (gone, along_x, at) in [
            (left, false, rx0),
            (top, true, ry0),
            (right, false, rx1 - 1),
            (bottom, true, ry1 - 1),
        ] {
            if !gone {
                continue;
            }
            if along_x {
                for x in rx0..rx1 {
                    if inside(x, at) {
                        self.vacate(x, at);
                    }
                }
            } else {
                for y in ry0..ry1 {
                    if inside(at, y) {
                        self.vacate(at, y);
                    }
                }
            }
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (t, b) = (y == y0, y == y1 && !open);
                let (l, rt) = (x == x0, x == x1 && !open);
                // An open box's edges run on into whatever box draws next to it.
                let a = match (t, b, l, rt) {
                    (true, _, true, _) => arm::R | arm::D,
                    (true, _, _, true) => arm::L | arm::D,
                    (_, true, true, _) => arm::U | arm::R,
                    (_, true, _, true) => arm::U | arm::L,
                    (true, ..) | (_, true, ..) => arm::H,
                    (.., true, _) | (.., true) => arm::V,
                    _ => 0,
                };
                if a == 0 {
                    continue;
                }
                // A collapsed edge only joins the line it met, outside its own clip.
                if !inside(x, y) && self.arms(x, y, layer) == 0 {
                    continue;
                }
                self.line(x, y, a, round, color, alpha, layer);
            }
        }
    }

    /// One side's rule, collapsed onto a parallel line it touches.
    fn rule(&mut self, item: &Item, side: u8, color: Rgba) {
        let (cols, rows) = (self.buf.cols, self.buf.rows);
        let (cx0, cy0, cx1, cy1) = span(item.rect, item.clip, cols, rows);
        let r = item.rect;
        let (rx0, ry0) = (r.x.round() as i32, r.y.round() as i32);
        let (rx1, ry1) = (r.right().round() as i32, r.bottom().round() as i32);
        let layer = item.layer;
        let inside = |x: i32, y: i32| x >= cx0 && x < cx1 && y >= cy0 && y < cy1;
        let (horizontal, at, step) = match side {
            0 => (true, ry0, -1),
            2 => (true, ry1 - 1, 1),
            1 => (false, rx1 - 1, 1),
            _ => (false, rx0, -1),
        };
        let cells: Vec<(i32, i32)> = if horizontal {
            (rx0..rx1).map(|x| (x, at)).collect()
        } else {
            (ry0..ry1).map(|y| (at, y)).collect()
        };
        let Some(&(mx, my)) = cells.get(cells.len() / 2) else {
            return;
        };
        let (axis, (dx, dy)) = if horizontal {
            (arm::H, (0, step))
        } else {
            (arm::V, (step, 0))
        };
        let moved = cells.len() > 1 && self.runs(mx + dx, my + dy, axis, layer);
        for (x, y) in cells {
            if !inside(x, y) {
                continue;
            }
            if moved {
                self.vacate(x, y);
                if self.arms(x + dx, y + dy, layer) != 0 {
                    self.line(x + dx, y + dy, axis, false, color, item.alpha, layer);
                }
            } else {
                self.line(x, y, axis, false, color, item.alpha, layer);
            }
        }
    }

    /// Joins lines that meet: an end touching a line gets its junction, and
    /// a cell an edge collapsed out of carries a line across to the one it met.
    fn join(&mut self) {
        let (cols, rows) = (i32::from(self.buf.cols), i32::from(self.buf.rows));
        for y in 0..rows {
            for x in 0..cols {
                let Some(i) = self.index(x, y) else { continue };
                if !self.gap[i] || self.buf.cells[i].sym != Sym::SPACE {
                    continue;
                }
                for (dx, dy, a) in AROUND {
                    let Some(Some(layer)) = self.index(x + dx, y + dy).map(|j| self.line[j]) else {
                        continue;
                    };
                    let far = self.arms(x + dx, y + dy, layer) & arm::back(a) != 0;
                    let near = self.arms(x - dx, y - dy, layer) != 0;
                    if far && near {
                        let c = self.buf.cells[self.index(x + dx, y + dy).unwrap_or(i)];
                        let fg = [c.fg[0], c.fg[1], c.fg[2]];
                        let cell = &mut self.buf.cells[i];
                        cell.sym = Sym::new(arm::glyph(a | arm::back(a), false));
                        cell.fg = fg;
                        self.line[i] = Some(layer);
                        break;
                    }
                }
            }
        }
        let mut add = Vec::new();
        for y in 0..rows {
            for x in 0..cols {
                let Some(i) = self.index(x, y) else { continue };
                let Some(layer) = self.line[i] else { continue };
                let own = self.arms(x, y, layer);
                let mut more = 0;
                for (dx, dy, a) in AROUND {
                    if own & a == 0 && self.arms(x + dx, y + dy, layer) & arm::back(a) != 0 {
                        more |= a;
                    }
                }
                if more != 0 {
                    let round = arm::round(self.buf.cells[i].sym.as_str());
                    add.push((i, arm::glyph(own | more, round)));
                }
            }
        }
        for (i, g) in add {
            self.buf.cells[i].sym = Sym::new(g);
        }
    }

    fn item(&mut self, item: &Item) {
        let (cols, rows) = (self.buf.cols, self.buf.rows);
        let (x0, y0, x1, y1) = span(item.rect, item.clip, cols, rows);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let r = item.rect;
        let (rx0, ry0) = (r.x.round() as i32, r.y.round() as i32);
        let (rx1, ry1) = (r.right().round() as i32, r.bottom().round() as i32);
        let inside = |x: i32, y: i32| x >= x0 && x < x1 && y >= y0 && y < y1;
        match &item.op {
            Op::Fill { color } => {
                let mut a = item.alpha;
                if is_tint(*color) {
                    a *= TINT_BOOST;
                }
                let opaque = color.0[3] * a >= 0.99;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let Some(i) = self.index(x, y) else { continue };
                        let c = &mut self.buf.cells[i];
                        c.bg = over(c.bg, *color, a);
                        if opaque {
                            c.sym = Sym::SPACE;
                            c.attr = 0;
                            self.line[i] = None;
                            self.gap[i] = false;
                        }
                    }
                }
            }
            Op::Frame { color, round, open } => self.frame(item, *round, *open, *color),
            Op::Rule { side, color } => self.rule(item, *side, *color),
            Op::Bar { color } => {
                for y in y0..y1 {
                    self.text(rx0, y, "▎", *color, 0, item.alpha, (x0, y0, x1, y1));
                }
            }
            Op::Scroll { thumb, color } => {
                for y in y0..y1 {
                    let on = y >= thumb.0 && y < thumb.0 + thumb.1;
                    let (g, a) = if on { ("┃", 1.0) } else { ("│", 0.35) };
                    self.text(rx0, y, g, *color, 0, item.alpha * a, (x0, y0, x1, y1));
                }
            }
            Op::Text {
                text,
                color,
                attr,
                wrap: wraps,
            } => {
                let room = (rx1 - rx0).max(0) as usize;
                let lines = if *wraps {
                    wrap(text, room)
                } else {
                    vec![text.lines().next().unwrap_or_default().to_owned()]
                };
                for (i, line) in lines.iter().enumerate() {
                    let y = ry0 + i as i32;
                    if y >= ry1.max(ry0 + 1) {
                        break;
                    }
                    let line = ellipsize(line, room);
                    self.text(rx0, y, &line, *color, *attr, item.alpha, (x0, y0, x1, y1));
                }
            }
            Op::Glyph { glyph, color, icon } => {
                let gw = width(glyph).max(1) as i32;
                let x = rx0 + ((rx1 - rx0 - gw) / 2).max(0);
                let y = ry0 + ((ry1 - ry0 - 1) / 2).max(0);
                self.text(x, y, glyph, *color, 0, item.alpha, (x0, y0, x1, y1));
                // One cell of image stands for a one-cell glyph.
                if *icon != 0 && gw == 1 && inside(x, y) {
                    if let Some(j) = self.index(x, y) {
                        self.buf.cells[j].icon = *icon;
                    }
                }
            }
            Op::Cells { cols: w, cells } => {
                let w = i32::from(*w);
                if w == 0 {
                    return;
                }
                for (i, src) in cells.iter().enumerate() {
                    let (x, y) = (rx0 + i as i32 % w, ry0 + i as i32 / w);
                    if inside(x, y) {
                        if let Some(j) = self.index(x, y) {
                            self.buf.cells[j] = *src;
                            self.line[j] = None;
                            self.gap[j] = false;
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        x: i32,
        y: i32,
        s: &str,
        color: Rgba,
        attr: u8,
        alpha: f32,
        clip: (i32, i32, i32, i32),
    ) {
        let (x0, y0, x1, y1) = clip;
        if y < y0 || y >= y1 {
            return;
        }
        let mut cx = x;
        for g in s.graphemes(true) {
            let gw = grapheme_width(g) as i32;
            if gw == 0 {
                continue;
            }
            if cx >= x1 {
                break;
            }
            if cx >= x0 && cx + gw <= x1 {
                if let Some(i) = self.index(cx, y) {
                    let c = &mut self.buf.cells[i];
                    c.sym = Sym::new(g);
                    c.fg = over(c.bg, color, alpha);
                    c.attr = attr;
                    let (fg, bg) = (c.fg, c.bg);
                    self.line[i] = None;
                    self.gap[i] = false;
                    if gw == 2 {
                        if let Some(j) = self.index(cx + 1, y) {
                            self.buf.cells[j] = Cell {
                                sym: Sym::TAIL,
                                fg,
                                bg,
                                attr,
                                icon: 0,
                            };
                            self.line[j] = None;
                        }
                    }
                }
            }
            cx += gw;
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn span_html(out: &mut String, c: &Cell, text: &str) {
    let hex = |c: [u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let (mut fg, mut bg) = (c.fg, c.bg);
    if c.attr & attr::INVERSE != 0 {
        std::mem::swap(&mut fg, &mut bg);
    }
    out.push_str(&format!(
        "<span style=\"color:{};background:{}{}{}{}\">{}</span>",
        hex(fg),
        hex(bg),
        if c.attr & attr::BOLD != 0 {
            ";font-weight:bold"
        } else {
            ""
        },
        if c.attr & attr::DIM != 0 {
            ";opacity:.6"
        } else {
            ""
        },
        if c.attr & attr::UNDERLINE != 0 {
            ";text-decoration:underline"
        } else {
            ""
        },
        escape(text)
    ));
}

/// The nearest of the xterm 256 colours to `c`.
pub fn ansi256(c: [u8; 3]) -> u8 {
    // The surfaces are a few levels apart, under the ramp's step of ten: chosen
    // a step each, in order, so panels and wells stay apart on Terminal.app.
    const SURFACES: [([u8; 3], u8); 4] = [
        ([0x0d, 0x0d, 0x0f], 232),
        ([0x10, 0x10, 0x12], 233),
        ([0x14, 0x14, 0x16], 234),
        ([0x1c, 0x1c, 0x20], 235),
    ];
    if let Some((_, i)) = SURFACES.iter().find(|(rgb, _)| *rgb == c) {
        return *i;
    }
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let near = |v: u8| -> usize {
        STEPS
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| (i32::from(**s) - i32::from(v)).abs())
            .map_or(0, |(i, _)| i)
    };
    let (r, g, b) = (near(c[0]), near(c[1]), near(c[2]));
    let cube = [STEPS[r], STEPS[g], STEPS[b]];
    let avg = (u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2])) / 3;
    let gray_i = ((avg.saturating_sub(8) + 5) / 10).min(23) as u8;
    let gray = 8 + gray_i * 10;
    let dist = |a: [u8; 3]| -> i32 {
        (0..3)
            .map(|i| (i32::from(a[i]) - i32::from(c[i])).pow(2))
            .sum()
    };
    if dist([gray; 3]) < dist(cube) {
        232 + gray_i
    } else {
        16 + 36 * r as u8 + 6 * g as u8 + b as u8
    }
}

/// The nearest of the 16 ANSI colours, by the xterm defaults.
pub fn ansi16(c: [u8; 3]) -> u8 {
    const PALETTE: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    PALETTE
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| {
            (0..3)
                .map(|i| (i32::from(p[i]) - i32::from(c[i])).pow(2))
                .sum::<i32>()
        })
        .map_or(0, |(i, _)| i as u8)
}

/// A stretch of an edit field's text on one cell row, in one attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRun {
    pub text: String,
    /// Column and row inside the field.
    pub col: u16,
    pub row: u16,
    pub attr: u8,
}

/// An edit field laid out in cells: what shows, and where the caret is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditCells {
    pub runs: Vec<EditRun>,
    /// The caret's column and row inside the field, when it is in view.
    pub caret: Option<(u16, u16)>,
}

/// Lays out `text` in a (columns, rows) field, scrolled to keep `caret` in view, `marks` in their attributes.
pub fn edit_cells(
    text: &str,
    caret: usize,
    marks: &[(std::ops::Range<usize>, u8)],
    (w, h): (usize, usize),
    wrap: bool,
) -> EditCells {
    let (w, h) = (w.max(1), h.max(1));
    // Each visual line as a byte range of `text`.
    let mut lines: Vec<std::ops::Range<usize>> = Vec::new();
    let mut start = 0;
    for para in text.split('\n') {
        let end = start + para.len();
        let mut line = start;
        let mut col = 0;
        if wrap {
            for (i, g) in para.grapheme_indices(true) {
                let gw = grapheme_width(g);
                if col + gw > w && col > 0 {
                    lines.push(line..start + i);
                    line = start + i;
                    col = 0;
                }
                col += gw;
            }
        }
        lines.push(line..end);
        start = end + 1;
    }
    let row = lines
        .iter()
        .rposition(|l| l.start <= caret)
        .unwrap_or_default();
    let col = width(&text[lines[row].start..caret.clamp(lines[row].start, lines[row].end)]);
    let top = row.saturating_sub(h - 1);
    let left = if wrap { 0 } else { col.saturating_sub(w - 1) };
    let attr_at = |i: usize| {
        marks
            .iter()
            .filter(|(r, _)| r.contains(&i))
            .fold(0, |a, (_, m)| a | m)
    };
    let mut runs: Vec<EditRun> = Vec::new();
    for (r, line) in lines.iter().enumerate().skip(top).take(h) {
        let mut x = 0;
        for (i, g) in text[line.clone()].grapheme_indices(true) {
            let gw = grapheme_width(g);
            if x < left {
                x += gw;
                continue;
            }
            if x - left + gw > w {
                break;
            }
            let attr = attr_at(line.start + i);
            let (c, rr) = ((x - left) as u16, (r - top) as u16);
            match runs.last_mut() {
                Some(run) if run.row == rr && run.attr == attr => run.text.push_str(g),
                _ => runs.push(EditRun {
                    text: g.to_owned(),
                    col: c,
                    row: rr,
                    attr,
                }),
            }
            x += gw;
        }
    }
    let caret = (col - left < w).then_some(((col - left) as u16, (row - top) as u16));
    EditCells { runs, caret }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_collapses_and_sizes_keep_proportion() {
        assert_eq!(units::space_x(2.0), 0.0);
        assert_eq!(units::space_x(4.0), 1.0);
        assert_eq!(units::space_x(8.0), 1.0);
        assert_eq!(units::space_x(16.0), 2.0);
        assert_eq!(units::space_y(8.0), 0.0);
        assert_eq!(units::width(256.0), 32.0);
        assert_eq!(units::height(36.0), 1.0);
        assert_eq!(units::height(43.0), 1.0);
        assert_eq!(units::height(56.0), 2.0);
        assert_eq!(units::height(2.0), 1.0);
        assert_eq!(units::border(0.5), 1.0);
    }

    #[test]
    fn truncates_with_an_ellipsis_and_wraps_words() {
        assert_eq!(ellipsize("worktrees", 5), "work…");
        assert_eq!(ellipsize("ok", 5), "ok");
        assert_eq!(width("日本"), 4);
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
    }

    #[test]
    fn draws_rounded_frames_text_and_diffs() {
        let mut list = PaintList::default();
        list.begin(Rgba::hex(0x0d0d0f));
        list.push(
            Op::Frame {
                color: Rgba::white(0.5),
                round: true,
                open: false,
            },
            Rect::new(0.0, 0.0, 6.0, 3.0),
        );
        list.push(
            Op::Text {
                text: "Sessions".into(),
                color: Rgba::white(1.0),
                attr: attr::BOLD,
                wrap: false,
            },
            Rect::new(1.0, 1.0, 4.0, 1.0),
        );
        let mut b = Buffer::new(6, 3);
        b.draw(&list);
        assert_eq!(b.row_text(0), "╭────╮");
        assert_eq!(b.row_text(1), "│Ses…│");
        assert_eq!(b.row_text(2), "╰────╯");
        assert_eq!(b.get(1, 1).map(|c| c.attr), Some(attr::BOLD));
        let prev = b.clone();
        assert_eq!(b.diff(Some(&prev)).count(), 0);
        assert_eq!(b.diff(None).count(), 18);
    }

    #[test]
    fn neighbouring_boxes_share_their_edges() {
        let mut list = PaintList::default();
        list.begin(Rgba::hex(0x0d0d0f));
        let frame = Op::Frame {
            color: Rgba::white(0.5),
            round: false,
            open: true,
        };
        list.push(frame.clone(), Rect::new(0.0, 0.0, 4.0, 3.0));
        list.push(frame.clone(), Rect::new(4.0, 0.0, 4.0, 3.0));
        list.push(frame, Rect::new(5.0, 1.0, 3.0, 2.0));
        let mut b = Buffer::new(8, 3);
        b.draw(&list);
        assert_eq!(b.row_text(0), "┌───┬───");
        assert_eq!(b.row_text(1), "│   │   ");
        assert_eq!(b.row_text(2), "│   │   ");
    }

    #[test]
    fn spells_modifier_symbols_as_a_terminal_names_them() {
        assert_eq!(keys("⌘K"), "Alt+K");
        assert_eq!(keys("⌘⇧W"), "Alt+Shift+W");
        assert_eq!(keys("Esc"), "Esc");
    }

    #[test]
    fn picks_colour_depth_from_the_environment() {
        assert_eq!(Depth::detect(Some("truecolor"), None), Depth::True);
        assert_eq!(Depth::detect(None, Some("xterm-256color")), Depth::Ansi256);
        assert_eq!(Depth::detect(None, Some("xterm")), Depth::Ansi16);
        assert_eq!(ansi256([255, 255, 255]), 231);
        assert_eq!(ansi16([250, 0, 0]), 9);
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    fn texts(e: &EditCells) -> Vec<(&str, u16, u16, u8)> {
        e.runs
            .iter()
            .map(|r| (r.text.as_str(), r.col, r.row, r.attr))
            .collect()
    }

    #[test]
    fn the_caret_sits_where_the_next_character_goes() {
        let e = edit_cells("hello", 2, &[], (10, 1), false);
        assert_eq!(e.caret, Some((2, 0)));
        assert_eq!(texts(&e), [("hello", 0, 0, 0)]);
        let wide = edit_cells("日本語", 3, &[], (10, 1), false);
        assert_eq!(
            wide.caret,
            Some((2, 0)),
            "a wide character takes two columns"
        );
    }

    #[test]
    fn a_long_line_scrolls_to_keep_the_caret_in_view() {
        let e = edit_cells("abcdefghij", 10, &[], (4, 1), false);
        assert_eq!(e.caret, Some((3, 0)));
        assert_eq!(texts(&e), [("hij", 0, 0, 0)]);
    }

    #[test]
    fn marks_split_runs_by_attribute() {
        let e = edit_cells(
            "abcdef",
            4,
            &[(1..3, attr::INVERSE), (4..6, attr::UNDERLINE)],
            (10, 1),
            false,
        );
        assert_eq!(
            texts(&e),
            [
                ("a", 0, 0, 0),
                ("bc", 1, 0, attr::INVERSE),
                ("d", 3, 0, 0),
                ("ef", 4, 0, attr::UNDERLINE)
            ]
        );
    }

    #[test]
    fn wrapped_lines_put_the_caret_on_its_row_and_scroll_down() {
        let e = edit_cells("one\ntwo three", 13, &[], (5, 2), true);
        assert_eq!(texts(&e), [("two t", 0, 0, 0), ("hree", 0, 1, 0)]);
        assert_eq!(e.caret, Some((4, 1)));
        let start = edit_cells("one\ntwo", 4, &[], (5, 3), true);
        assert_eq!(
            start.caret,
            Some((0, 1)),
            "just after a newline is the next row"
        );
        let empty = edit_cells("", 0, &[], (5, 1), false);
        assert_eq!((empty.runs.len(), empty.caret), (0, Some((0, 0))));
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    #[test]
    fn the_surfaces_keep_their_order_a_step_apart_in_256_colours() {
        let idx: Vec<u8> = [0x0d0d0f, 0x101012, 0x141416, 0x1c1c20]
            .map(|h: u32| ansi256([(h >> 16) as u8, (h >> 8) as u8, h as u8]))
            .to_vec();
        assert_eq!(idx, [232, 233, 234, 235]);
        // A hover tint over the base is a visible step up from every surface.
        let tint = over([0x0d, 0x0d, 0x0f], Rgba::white(0.14), 1.0);
        assert!(ansi256(tint) > 235, "{tint:?} -> {}", ansi256(tint));
        assert_eq!(ansi256([255, 255, 255]), 231);
        assert_eq!(ansi256([0x6c, 0x6c, 0x6c]), 242);
    }
}
