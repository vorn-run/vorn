//! The screen mirror's types (Terminal State Protocol §5 and §6): what vornd
//! sends a grid client in place of bytes. libghostty-vt's types never appear
//! here, so a Ghostty upgrade cannot change the wire.
//!
//! Their CBOR form, and the messages that carry them, are in [`crate::msg`].

use crate::position::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Primary,
    Alternate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    BlockHollow,
    Bar,
    Underline,
    /// A style a newer Ghostty added and this build does not know.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    pub blinking: bool,
    pub style: CursorStyle,
    pub wide_tail: bool,
}

/// Unresolved, so a palette change resends no rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Palette(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    None,
    X10,
    Normal,
    Button,
    Any,
}

/// Bits of `TermState::flags`.
pub mod flags {
    pub const BRACKETED_PASTE: u32 = 1 << 0;
    pub const FOCUS_EVENTS: u32 = 1 << 1;
    pub const SYNC_OUTPUT_ACTIVE: u32 = 1 << 2;
    pub const PASSWORD_INPUT: u32 = 1 << 3;
    pub const REVERSE_VIDEO: u32 = 1 << 4;
}

/// Bits of `StyleDef::attrs`.
pub mod attrs {
    pub const BOLD: u16 = 1 << 0;
    pub const ITALIC: u16 = 1 << 1;
    pub const FAINT: u16 = 1 << 2;
    pub const BLINK: u16 = 1 << 3;
    pub const INVERSE: u16 = 1 << 4;
    pub const INVISIBLE: u16 = 1 << 5;
    pub const STRIKE: u16 = 1 << 6;
    pub const OVERLINE: u16 = 1 << 7;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
    Unknown,
}

/// The foreground a session's terminal starts with. vornd answers colour
/// queries with these start colours; a client draws its own theme wherever a
/// colour is still the start one, so only colours a program set override it.
pub const START_FG: [u8; 3] = [0xd4, 0xd4, 0xd8];
/// The background a session's terminal starts with.
pub const START_BG: [u8; 3] = [0x14, 0x14, 0x16];
/// The sixteen ANSI colours a session's terminal starts with.
pub const START_ANSI: [[u8; 3]; 16] = [
    [0x27, 0x27, 0x2a],
    [0xef, 0x44, 0x44],
    [0x22, 0xc5, 0x5e],
    [0xea, 0xb3, 0x08],
    [0x3b, 0x82, 0xf6],
    [0xa8, 0x55, 0xf7],
    [0x06, 0xb6, 0xd4],
    [0xd4, 0xd4, 0xd8],
    [0x52, 0x52, 0x5b],
    [0xf8, 0x71, 0x71],
    [0x4a, 0xde, 0x80],
    [0xfa, 0xcc, 0x15],
    [0x60, 0xa5, 0xfa],
    [0xc0, 0x84, 0xfc],
    [0x22, 0xd3, 0xee],
    [0xfa, 0xfa, 0xfa],
];

/// Palette entry `i` as a session's terminal starts with it: the start ANSI
/// colours, then xterm's colour cube and gray ramp.
pub fn start_entry(i: u8) -> [u8; 3] {
    match i {
        0..=15 => START_ANSI[usize::from(i)],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            [level(i / 36), level((i / 6) % 6), level(i % 6)]
        }
        _ => [8 + 10 * (i - 232); 3],
    }
}

/// The palette's defaults and the entries that changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColorsDelta {
    pub fg: Option<(u8, u8, u8)>,
    pub bg: Option<(u8, u8, u8)>,
    pub cursor: Option<(u8, u8, u8)>,
    pub palette: Vec<(u8, (u8, u8, u8))>,
}

/// Sent whole in a snapshot; a delta carries the same fields, each only when
/// it changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TermState {
    pub cols: u16,
    pub rows: u16,
    pub screen: Screen,
    pub cursor: CursorState,
    pub colors: ColorsDelta,
    pub mouse: MouseMode,
    pub flags: u32,
    pub title: String,
    pub cwd: String,
    /// Raised when absolute line numbers stop being valid.
    pub sb_epoch: u32,
    /// Lines currently held in scrollback.
    pub history_lines: u64,
    /// Absolute line of active row 0, primary screen only.
    pub top_line: u64,
}

/// A delta's changes to `TermState`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TermDelta {
    pub size: Option<(u16, u16)>,
    pub screen: Option<Screen>,
    pub cursor: Option<CursorState>,
    pub colors: Option<ColorsDelta>,
    pub mouse: Option<MouseMode>,
    pub flags: Option<u32>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub sb_epoch: Option<u32>,
    pub history_lines: Option<u64>,
    pub top_line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyleDef {
    pub id: u32,
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Color,
    pub attrs: u16,
    pub underline: Underline,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkDef {
    pub id: u32,
    pub uri: String,
    pub osc8_id: Option<String>,
}

/// Bits of `Row::flags`.
pub mod row_flags {
    pub const WRAPPED: u8 = 1 << 0;
    pub const WRAP_CONTINUATION: u8 = 1 << 1;
    pub const PROMPT: u8 = 1 << 2;
    pub const PROMPT_CONTINUATION: u8 = 1 << 3;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    /// Index in the viewport.
    pub y: u16,
    /// Absolute line on the primary screen; 0 on the alternate screen.
    pub line: u64,
    pub flags: u8,
    /// `row_fmt` 1, see [`crate::row`].
    pub cells: Vec<u8>,
}

/// What a grid client gets on attach or after a resync: complete tables and
/// the whole viewport.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub state_gen: u64,
    pub rev: u64,
    pub resume: Cursor,
    pub table_gen: u32,
    pub row_fmt: u8,
    pub term: TermState,
    pub styles: Vec<StyleDef>,
    pub links: Vec<LinkDef>,
    pub rows: Vec<Row>,
    pub history: Vec<Row>,
    /// Kitty image placements, for a client that asked for images and a session that has some.
    pub placements: Option<Placements>,
}

/// The rows that changed since `base_rev`, and every table definition past
/// the client's marks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    pub state_gen: u64,
    pub table_gen: u32,
    pub base_rev: u64,
    pub rev: u64,
    pub resume: Cursor,
    /// Lines pushed into history since `base_rev`.
    pub scrolled: u32,
    pub term: Option<TermDelta>,
    pub styles: Vec<StyleDef>,
    pub links: Vec<LinkDef>,
    pub rows: Vec<Row>,
    /// Every placement, when any moved, appeared or left since `base_rev`.
    pub placements: Option<Placements>,
}

/// One kitty image's pixels, sent to a client once per generation, before the first placement
/// that shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Image {
    pub id: u32,
    /// Changes whenever the pixels do.
    pub gen: u64,
    pub width: u32,
    pub height: u32,
    /// The size the program sent it at, which placement crops are measured in: larger than
    /// `width` × `height` when it was shrunk to [`IMAGE_MAX_SIDE`].
    pub source_width: u32,
    pub source_height: u32,
    /// Straight-alpha RGBA, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// The longest side an image is sent at, so its frame stays far below the frame limit.
pub const IMAGE_MAX_SIDE: u32 = 1024;

/// Straight-alpha RGBA `width` × `height` shrunk to at most `max` px a side, keeping its aspect,
/// each pixel the alpha-weighted average of those it covers; `None` when it already fits or
/// `rgba` is short.
pub fn shrink(width: u32, height: u32, rgba: &[u8], max: u32) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = (width, height);
    if (w <= max && h <= max) || max == 0 || rgba.len() < w as usize * h as usize * 4 {
        return None;
    }
    let f = f64::from(w.max(h)) / f64::from(max);
    let sw = ((f64::from(w) / f).round() as u32).clamp(1, max);
    let sh = ((f64::from(h) / f).round() as u32).clamp(1, max);
    let mut out = vec![0u8; sw as usize * sh as usize * 4];
    for (y, line) in (0..sh).zip(out.chunks_exact_mut(sw as usize * 4)) {
        let (y0, y1) = (y * h / sh, ((y + 1) * h / sh).max(y * h / sh + 1));
        for (x, o) in (0..sw).zip(line.chunks_exact_mut(4)) {
            let (x0, x1) = (x * w / sw, ((x + 1) * w / sw).max(x * w / sw + 1));
            let mut sum = [0u64; 4];
            for py in y0..y1 {
                let row =
                    &rgba[(py as usize * w as usize + x0 as usize) * 4..][..(x1 - x0) as usize * 4];
                for p in row.chunks_exact(4) {
                    let a = u64::from(p[3]);
                    for c in 0..3 {
                        sum[c] += u64::from(p[c]) * a;
                    }
                    sum[3] += a;
                }
            }
            let n = u64::from((x1 - x0) * (y1 - y0));
            for c in 0..3 {
                o[c] = sum[c].checked_div(sum[3]).unwrap_or(0) as u8;
            }
            o[3] = (sum[3] / n) as u8;
        }
    }
    Some((sw, sh, out))
}

/// An image placed on the grid, in the server's cell pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Placement {
    pub image: u32,
    pub gen: u64,
    /// The line of its top left cell: absolute on the primary screen, a viewport row on the
    /// alternate one.
    pub line: i64,
    pub col: i32,
    /// Pixels from the top left of that cell.
    pub x_off: u32,
    pub y_off: u32,
    /// The size it is drawn at, in pixels.
    pub width: u32,
    pub height: u32,
    /// The part of the image drawn: x, y, width, height in its source pixels.
    pub src: [u32; 4],
    /// Below text when negative; below cell backgrounds under `i32::MIN / 2`.
    pub z: i32,
}

/// A virtual placement: unicode placeholder cells (U+10EEEE) draw it, a grid of `cols` × `rows`
/// cells the client lays out itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Virtual {
    pub image: u32,
    pub gen: u64,
    /// Zero for one placed without an id.
    pub placement: u32,
    pub cols: u32,
    pub rows: u32,
}

/// Every image placement on the active screen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Placements {
    /// The server's cell size in pixels, which placement pixels are measured in.
    pub cell_w: u32,
    pub cell_h: u32,
    pub list: Vec<Placement>,
    pub virtuals: Vec<Virtual>,
}

impl Placements {
    /// The images, by id and generation, its placements show.
    pub fn images(&self) -> impl Iterator<Item = (u32, u64)> + '_ {
        let placed = self.list.iter().map(|p| (p.image, p.gen));
        placed.chain(self.virtuals.iter().map(|v| (v.image, v.gen)))
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty() && self.virtuals.is_empty()
    }
}

/// Stable name of a side effect: the record that caused it and its index in
/// that record, the same after a replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectId {
    pub session: u128,
    pub epoch: u32,
    pub rseq: u64,
    pub index: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_palette_is_the_ansi_colours_then_xterms_cube_and_ramp() {
        assert_eq!(start_entry(1), START_ANSI[1]);
        assert_eq!(start_entry(16), [0, 0, 0]);
        assert_eq!(start_entry(196), [0xff, 0, 0]);
        assert_eq!(start_entry(231), [0xff; 3]);
        assert_eq!(start_entry(232), [8; 3]);
        assert_eq!(start_entry(255), [238; 3]);
    }

    #[test]
    fn shrink_keeps_the_aspect_and_averages_by_alpha() {
        let (w, h) = (6000u32, 4000u32);
        let mut rgba = vec![0u8; w as usize * h as usize * 4];
        // Left half opaque red, right half transparent green.
        for (i, p) in rgba.chunks_exact_mut(4).enumerate() {
            let x = i as u32 % w;
            p.copy_from_slice(if x < w / 2 {
                &[255, 0, 0, 255]
            } else {
                &[0, 255, 0, 0]
            });
        }
        let (sw, sh, px) = shrink(w, h, &rgba, IMAGE_MAX_SIDE).unwrap();
        assert_eq!((sw, sh), (1024, 683));
        assert_eq!(px.len(), 1024 * 683 * 4);
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
        let last = (1024 * 683 - 1) * 4;
        assert_eq!(px[last + 3], 0);
    }

    #[test]
    fn shrink_leaves_small_or_short_images() {
        assert!(shrink(1024, 10, &[0; 1024 * 10 * 4], IMAGE_MAX_SIDE).is_none());
        assert!(shrink(2000, 10, &[0; 4], IMAGE_MAX_SIDE).is_none());
        let (w, h, _) = shrink(1, 5000, &[9; 5000 * 4], IMAGE_MAX_SIDE).unwrap();
        assert_eq!((w, h), (1, 1024));
    }
}
