//! What a pane looks like to a UI: rows cut into runs of one resolved
//! style, colors already resolved against the terminal's palette, so every
//! prototype draws exactly the same thing and only the drawing differs.

use vorn_term_mirror::Mirror;
use vorn_term_proto::row;
use vorn_term_proto::screen::{attrs, Color, CursorStyle, StyleDef, Underline};

/// Bits of [`RunC::flags`].
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

/// One run of cells: `ncols` columns from `col` on `row`, its text at
/// `text[text_off..text_off + text_len]` (UTF-8). Laid out for C and for a
/// JavaScript `DataView` alike: 24 bytes, little-endian.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunC {
    pub row: u16,
    pub col: u16,
    pub ncols: u16,
    pub flags: u16,
    /// 0xRRGGBB.
    pub fg: u32,
    pub bg: u32,
    pub text_off: u32,
    pub text_len: u32,
}

#[derive(Debug, Clone, Default)]
pub struct PaneView {
    pub rev: u64,
    pub cols: u16,
    pub rows: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    /// 0 block, 1 hollow block, 2 bar, 3 underline.
    pub cursor_style: u8,
    pub fg: u32,
    pub bg: u32,
    pub cursor_color: u32,
    pub runs: Vec<RunC>,
    pub text: Vec<u8>,
    /// The latency probe's glyph is in this view: once it is drawn, the
    /// UI stops the probe's clock.
    pub probe_hit: bool,
}

pub const DEFAULT_FG: u32 = 0xd4d4d4;
pub const DEFAULT_BG: u32 = 0x1e1e1e;
pub const DEFAULT_CURSOR: u32 = 0xe8e8e8;

const BASE16: [u32; 16] = [
    0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
    0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
];

/// The xterm 256-color palette.
pub fn xterm(i: u8) -> u32 {
    match i {
        0..=15 => BASE16[i as usize],
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

fn rgb((r, g, b): (u8, u8, u8)) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

struct Palette {
    fg: u32,
    bg: u32,
    over: Vec<(u8, u32)>,
}

impl Palette {
    fn of(m: &Mirror) -> Palette {
        let c = &m.term().colors;
        Palette {
            fg: c.fg.map_or(DEFAULT_FG, rgb),
            bg: c.bg.map_or(DEFAULT_BG, rgb),
            over: c.palette.iter().map(|(i, v)| (*i, rgb(*v))).collect(),
        }
    }

    fn color(&self, c: Color, default: u32) -> u32 {
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
    fn resolve(&self, s: Option<&StyleDef>) -> (u32, u32, u16) {
        let Some(s) = s else {
            return (self.fg, self.bg, 0);
        };
        let mut fg = self.color(s.fg, self.fg);
        let mut bg = self.color(s.bg, self.bg);
        // Bold brightens the first eight colors, as most terminals do.
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
        if s.attrs & attrs::BOLD != 0 {
            f |= flags::BOLD;
        }
        if s.attrs & attrs::ITALIC != 0 {
            f |= flags::ITALIC;
        }
        if s.attrs & attrs::FAINT != 0 {
            f |= flags::FAINT;
        }
        if s.attrs & attrs::STRIKE != 0 {
            f |= flags::STRIKE;
        }
        if s.underline != Underline::None {
            f |= flags::UNDERLINE;
        }
        (fg, bg, f)
    }
}

/// Builds the view of a mirror.
pub fn build(m: &Mirror, rev: u64) -> PaneView {
    let t = m.term();
    let pal = Palette::of(m);
    let mut v = PaneView {
        rev,
        cols: t.cols,
        rows: t.rows,
        cursor_x: t.cursor.x,
        cursor_y: t.cursor.y,
        cursor_visible: t.cursor.visible,
        cursor_style: match t.cursor.style {
            CursorStyle::BlockHollow => 1,
            CursorStyle::Bar => 2,
            CursorStyle::Underline => 3,
            _ => 0,
        },
        fg: pal.fg,
        bg: pal.bg,
        cursor_color: t.colors.cursor.map_or(DEFAULT_CURSOR, rgb),
        runs: Vec::with_capacity(t.rows as usize * 2),
        text: Vec::with_capacity(t.rows as usize * t.cols as usize),
        probe_hit: false,
    };
    for (y, r) in m.rows().iter().enumerate() {
        let Ok(runs) = row::decode(&r.cells) else {
            continue;
        };
        let mut col: u16 = 0;
        for run in runs {
            let (fg, bg, f) = pal.resolve(m.style(run.style));
            for cell in run.cells {
                let w = if cell.wide { 2 } else { 1 };
                let ascii = !cell.wide && cell.text.len() <= 1;
                let text = if cell.text.is_empty() { " " } else { &cell.text };
                let last = v.runs.last_mut();
                match last {
                    Some(l)
                        if ascii
                            && l.flags & flags::CLUSTER == 0
                            && l.row == y as u16
                            && l.col + l.ncols == col
                            && l.fg == fg
                            && l.bg == bg
                            && l.flags == f =>
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
                        v.runs.push(RunC {
                            row: y as u16,
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
    // Runs that are only blanks on the default background draw nothing.
    let text = &v.text;
    let bg = v.bg;
    v.runs.retain(|r| {
        r.bg != bg
            || r.flags & (flags::UNDERLINE | flags::STRIKE) != 0
            || text[r.text_off as usize..(r.text_off + r.text_len) as usize]
                .iter()
                .any(|b| *b != b' ')
    });
    v
}

/// The cluster in column `x` of viewport row `y`.
pub fn cell_at(m: &Mirror, x: u16, y: u16) -> Option<String> {
    let r = m.rows().get(y as usize)?;
    let mut col = 0u16;
    for run in row::decode(&r.cells).ok()? {
        for cell in run.cells {
            let w = if cell.wide { 2 } else { 1 };
            if x >= col && x < col + w {
                return Some(cell.text);
            }
            col += w;
        }
    }
    None
}

impl PaneView {
    /// The text of a run.
    pub fn run_text(&self, r: &RunC) -> &str {
        std::str::from_utf8(&self.text[r.text_off as usize..(r.text_off + r.text_len) as usize])
            .unwrap_or("?")
    }

    /// The view as bytes for a webview, read there with a `DataView`:
    /// a 48-byte header, the runs (24 bytes each), then the text.
    pub fn to_bytes(&self, pane: u32, out: &mut Vec<u8>) {
        out.extend_from_slice(&pane.to_le_bytes());
        out.extend_from_slice(&(self.rev as u32).to_le_bytes());
        for v in [self.cols, self.rows, self.cursor_x, self.cursor_y] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&[
            self.cursor_visible as u8,
            self.cursor_style,
            self.probe_hit as u8,
            0,
        ]);
        for v in [
            self.fg,
            self.bg,
            self.cursor_color,
            self.runs.len() as u32,
            self.text.len() as u32,
            0,
            0,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for r in &self.runs {
            for v in [r.row, r.col, r.ncols, r.flags] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            for v in [r.fg, r.bg, r.text_off, r.text_len] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out.extend_from_slice(&self.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_corners() {
        assert_eq!(xterm(16), 0x000000);
        assert_eq!(xterm(231), 0xffffff);
        assert_eq!(xterm(232), 0x080808);
        assert_eq!(xterm(196), 0xff0000);
    }

    #[test]
    fn header_is_48_bytes() {
        let mut out = Vec::new();
        PaneView::default().to_bytes(0, &mut out);
        assert_eq!(out.len(), 48);
    }
}
