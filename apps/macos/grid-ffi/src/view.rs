//! What a pane looks like to the app: rows cut into runs of one resolved
//! style, colours already resolved against the theme and the terminal's own
//! palette changes, so the app only draws.

use vorn_term_mirror::Mirror;
use vorn_term_proto::row;
use vorn_term_proto::screen::{attrs, Color, CursorStyle, StyleDef, Underline};

/// Bits of [`VgRun::flags`].
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

/// One run of cells: `ncols` columns from `col` on `row`, its UTF-8 text at
/// `text[text_off..text_off + text_len]`. Mirrors `VgRun` in `vorn_grid.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VgRun {
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

/// The app's colours, 0xRRGGBB. Mirrors `VgTheme` in `vorn_grid.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VgTheme {
    pub fg: u32,
    pub bg: u32,
    pub cursor: u32,
    pub ansi: [u32; 16],
}

impl Default for VgTheme {
    fn default() -> Self {
        VgTheme {
            fg: 0xd4d4d4,
            bg: 0x1e1e1e,
            cursor: 0xe8e8e8,
            ansi: [
                0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5,
                0x666666, 0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
            ],
        }
    }
}

/// A pane's screen at one revision, owned by the app until it frees it.
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
    pub runs: Vec<VgRun>,
    pub text: Vec<u8>,
}

/// The xterm 256-colour palette, with the theme's first sixteen.
pub fn xterm(theme: &VgTheme, i: u8) -> u32 {
    match i {
        0..=15 => theme.ansi[usize::from(i)],
        16..=231 => {
            let i = i - 16;
            let lv = |v: u8| if v == 0 { 0 } else { 55 + 40 * u32::from(v) };
            (lv(i / 36) << 16) | (lv((i / 6) % 6) << 8) | lv(i % 6)
        }
        _ => {
            let v = 8 + 10 * (u32::from(i) - 232);
            (v << 16) | (v << 8) | v
        }
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

struct Palette<'a> {
    theme: &'a VgTheme,
    fg: u32,
    bg: u32,
    over: Vec<(u8, u32)>,
}

impl<'a> Palette<'a> {
    fn of(m: &Mirror, theme: &'a VgTheme) -> Palette<'a> {
        let c = &m.term().colors;
        Palette {
            theme,
            fg: c.fg.map_or(theme.fg, rgb),
            bg: c.bg.map_or(theme.bg, rgb),
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
                .map_or_else(|| xterm(self.theme, i), |(_, v)| *v),
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
        for (a, bit) in [
            (attrs::BOLD, flags::BOLD),
            (attrs::ITALIC, flags::ITALIC),
            (attrs::FAINT, flags::FAINT),
            (attrs::STRIKE, flags::STRIKE),
        ] {
            if s.attrs & a != 0 {
                f |= bit;
            }
        }
        if s.underline != Underline::None {
            f |= flags::UNDERLINE;
        }
        (fg, bg, f)
    }
}

/// Builds the view of a mirror against `theme`.
pub fn build(m: &Mirror, theme: &VgTheme) -> PaneView {
    let t = m.term();
    let pal = Palette::of(m, theme);
    let mut v = PaneView {
        rev: m.rev(),
        cols: t.cols,
        rows: t.rows,
        cursor_x: t.cursor.x,
        cursor_y: t.cursor.y,
        cursor_visible: t.cursor.visible,
        cursor_style: match t.cursor.style {
            CursorStyle::BlockHollow => 1,
            CursorStyle::Bar => 2,
            CursorStyle::Underline => 3,
            CursorStyle::Block | CursorStyle::Unknown => 0,
        },
        fg: pal.fg,
        bg: pal.bg,
        cursor_color: t.colors.cursor.map_or(theme.cursor, rgb),
        runs: Vec::with_capacity(usize::from(t.rows) * 2),
        text: Vec::with_capacity(usize::from(t.rows) * usize::from(t.cols)),
    };
    for (y, r) in m.rows().iter().enumerate() {
        let Ok(y) = u16::try_from(y) else { break };
        let Ok(runs) = row::decode(&r.cells) else {
            continue;
        };
        let mut col: u16 = 0;
        'row: for run in runs {
            let (fg, bg, f) = pal.resolve(m.style(run.style));
            for cell in run.cells {
                let w: u16 = if cell.wide { 2 } else { 1 };
                let Some(next) = col.checked_add(w) else {
                    break 'row;
                };
                let ascii = !cell.wide && cell.text.len() <= 1;
                let text = if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                };
                // Text lengths fit u32: a frame is at most 64 MiB.
                let text_off = v.text.len() as u32;
                match v.runs.last_mut() {
                    Some(l)
                        if ascii
                            && l.flags & flags::CLUSTER == 0
                            && l.row == y
                            && u32::from(l.col) + u32::from(l.ncols) == u32::from(col)
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
                        v.runs.push(VgRun {
                            row: y,
                            col,
                            ncols: w,
                            flags,
                            fg,
                            bg,
                            text_off,
                            text_len: text.len() as u32,
                        });
                        v.text.extend_from_slice(text.as_bytes());
                    }
                }
                col = next;
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use vorn_term_proto::row::{Cell, Run, ROW_FMT};
    use vorn_term_proto::screen::{ColorsDelta, LinkDef, Row, Snapshot, TermState};

    pub(crate) fn style(id: u32, fg: Color, bg: Color, attrs: u16) -> StyleDef {
        StyleDef {
            id,
            fg,
            bg,
            attrs,
            ..StyleDef::default()
        }
    }

    pub(crate) fn line(y: u16, runs: &[(u32, &str)]) -> Row {
        let runs: Vec<Run> = runs
            .iter()
            .map(|(style, text)| Run {
                style: *style,
                link: 0,
                cells: text.chars().map(|c| Cell::new(c.to_string())).collect(),
            })
            .collect();
        Row {
            y,
            line: u64::from(y),
            flags: 0,
            cells: row::encode(&runs),
        }
    }

    pub(crate) fn snapshot(rev: u64, styles: Vec<StyleDef>, rows: Vec<Row>) -> Snapshot {
        Snapshot {
            state_gen: 1,
            rev,
            table_gen: 0,
            row_fmt: ROW_FMT,
            term: TermState {
                cols: 10,
                rows: 3,
                ..TermState::default()
            },
            styles,
            links: vec![LinkDef::default()],
            rows,
            ..Snapshot::default()
        }
    }

    fn plain() -> StyleDef {
        style(0, Color::Default, Color::Default, 0)
    }

    fn view_of(snap: Snapshot, theme: &VgTheme) -> PaneView {
        build(&Mirror::from_snapshot(snap).expect("valid snapshot"), theme)
    }

    fn run_text<'a>(v: &'a PaneView, r: &VgRun) -> &'a str {
        let at = r.text_off as usize;
        std::str::from_utf8(&v.text[at..at + r.text_len as usize]).expect("utf-8")
    }

    #[test]
    fn palette_corners() {
        let t = VgTheme::default();
        assert_eq!(xterm(&t, 16), 0x000000);
        assert_eq!(xterm(&t, 231), 0xffffff);
        assert_eq!(xterm(&t, 232), 0x080808);
        assert_eq!(xterm(&t, 196), 0xff0000);
        assert_eq!(xterm(&t, 1), t.ansi[1]);
    }

    #[test]
    fn theme_sets_defaults_and_ansi() {
        let theme = VgTheme {
            fg: 0x111111,
            bg: 0x222222,
            cursor: 0x333333,
            ansi: [0xabcdef; 16],
        };
        let red = style(1, Color::Palette(1), Color::Default, 0);
        let v = view_of(
            snapshot(1, vec![plain(), red], vec![line(0, &[(1, "x")])]),
            &theme,
        );
        assert_eq!((v.fg, v.bg, v.cursor_color), (0x111111, 0x222222, 0x333333));
        assert_eq!(v.runs.len(), 1);
        assert_eq!((v.runs[0].fg, v.runs[0].bg), (0xabcdef, 0x222222));
    }

    #[test]
    fn terminal_palette_overrides_theme() {
        let mut snap = snapshot(
            1,
            vec![plain(), style(1, Color::Palette(2), Color::Default, 0)],
            vec![line(0, &[(1, "x")])],
        );
        snap.term.colors = ColorsDelta {
            fg: Some((1, 2, 3)),
            palette: vec![(2, (0xaa, 0xbb, 0xcc))],
            ..ColorsDelta::default()
        };
        let v = view_of(snap, &VgTheme::default());
        assert_eq!(v.fg, 0x010203);
        assert_eq!(v.runs[0].fg, 0xaabbcc);
    }

    #[test]
    fn bold_brightens_inverse_swaps_invisible_hides() {
        let t = VgTheme::default();
        let styles = vec![
            plain(),
            style(1, Color::Palette(3), Color::Default, attrs::BOLD),
            style(2, Color::Rgb(1, 1, 1), Color::Rgb(2, 2, 2), attrs::INVERSE),
            style(3, Color::Palette(1), Color::Palette(4), attrs::INVISIBLE),
            style(
                4,
                Color::Palette(9),
                Color::Default,
                attrs::BOLD | attrs::FAINT,
            ),
        ];
        let rows = vec![line(0, &[(1, "a"), (2, "b"), (3, "c"), (4, "d")])];
        let v = view_of(snapshot(1, styles, rows), &t);
        let by_text = |s: &str| {
            *v.runs
                .iter()
                .find(|r| run_text(&v, r) == s)
                .expect("run present")
        };
        let a = by_text("a");
        assert_eq!((a.fg, a.flags), (t.ansi[11], flags::BOLD));
        let b = by_text("b");
        assert_eq!((b.fg, b.bg), (0x020202, 0x010101));
        let c = by_text("c");
        assert_eq!((c.fg, c.bg), (t.ansi[4], t.ansi[4]));
        let d = by_text("d");
        assert_eq!((d.fg, d.flags), (t.ansi[9], flags::BOLD | flags::FAINT));
    }

    #[test]
    fn runs_merge_ascii_and_isolate_clusters() {
        let rows = vec![Row {
            y: 0,
            line: 0,
            flags: 0,
            cells: row::encode(&[Run {
                style: 0,
                link: 0,
                cells: vec![
                    Cell::new("a"),
                    Cell::new("b"),
                    Cell::wide("界"),
                    Cell::new("é"),
                    Cell::new("c"),
                ],
            }]),
        }];
        let v = view_of(snapshot(1, vec![plain()], rows), &VgTheme::default());
        let got: Vec<(u16, u16, u16, &str)> = v
            .runs
            .iter()
            .map(|r| (r.col, r.ncols, r.flags, run_text(&v, r)))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, 2, 0, "ab"),
                (2, 2, flags::CLUSTER | flags::WIDE, "界"),
                (4, 1, flags::CLUSTER, "é"),
                (5, 1, 0, "c"),
            ]
        );
    }

    #[test]
    fn blank_default_runs_are_dropped() {
        let styles = vec![plain(), style(1, Color::Default, Color::Palette(4), 0)];
        let rows = vec![line(0, &[(0, "   "), (1, "  ")]), line(1, &[(0, "hi")])];
        let v = view_of(snapshot(1, styles, rows), &VgTheme::default());
        let got: Vec<(u16, u16, &str)> = v
            .runs
            .iter()
            .map(|r| (r.row, r.col, run_text(&v, r)))
            .collect();
        assert_eq!(got, vec![(0, 3, "  "), (1, 0, "hi")]);
    }
}
