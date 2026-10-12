//! Text: cosmic-text shapes, swash rasterizes, the atlases keep the result.
//!
//! UI strings are shaped once per (string, style, width) and the shaped
//! glyphs reused while the string is on screen; a [`Shaped`] also keeps each
//! glyph's byte range so text inputs can place a caret and hit-test a click.
//! Terminal cells take a faster path: every cell sits on an integer pixel,
//! so a cell's glyph is rasterized once per (cluster, style) and a frame
//! only copies atlas references.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, LayoutGlyph, Metrics, Shaping, Style, SwashCache,
    SwashContent, Weight, Wrap,
};

use crate::atlas::Atlases;
use crate::scene::{Rect, Rgba, Scene};

/// How a UI string looks. Sizes are logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
    pub mono: bool,
    /// Letter spacing in ems (Tailwind's `tracking-wider` is 0.05).
    pub tracking: f32,
    pub italic: bool,
}

impl TextStyle {
    /// The UI face at `size`, with Tailwind's default 1.5 line height.
    pub fn ui(size: f32) -> TextStyle {
        TextStyle {
            size,
            line_height: (size * 1.5).round(),
            weight: 400,
            mono: false,
            tracking: 0.0,
            italic: false,
        }
    }

    pub fn mono(size: f32) -> TextStyle {
        TextStyle {
            mono: true,
            ..TextStyle::ui(size)
        }
    }
}

/// One visual line of a [`Shaped`] string, in physical pixels; `start..end`
/// are byte offsets into the string.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub top: f32,
    pub height: f32,
    pub w: f32,
    pub start: usize,
    pub end: usize,
}

/// Where one glyph sits: `x` and `w` in physical pixels, the bytes it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GlyphPos {
    x: f32,
    w: f32,
    start: usize,
    end: usize,
    line: usize,
}

/// A shaped string, in physical pixels.
pub struct Shaped {
    glyphs: Vec<(LayoutGlyph, f32)>,
    pos: Vec<GlyphPos>,
    pub lines: Vec<Line>,
    pub w: f32,
    pub h: f32,
}

impl Shaped {
    /// The visual line holding byte `i` (a caret before that byte).
    pub fn line_of(&self, i: usize) -> usize {
        let last = self.lines.len().saturating_sub(1);
        // A caret at a wrap point belongs to the line it starts.
        self.lines
            .iter()
            .position(|l| i >= l.start && i < l.end)
            .or_else(|| self.lines.iter().rposition(|l| i >= l.start))
            .unwrap_or(0)
            .min(last)
    }

    /// Where a caret before byte `i` goes: x, and its line.
    pub fn caret(&self, i: usize) -> (f32, usize) {
        let line = self.line_of(i);
        let on_line = self.pos.iter().filter(|g| g.line == line);
        let mut x = 0.0;
        for g in on_line {
            if i <= g.start {
                return (g.x, line);
            }
            if i < g.end {
                // Inside a cluster (a ligature): share its width by bytes.
                let t = (i - g.start) as f32 / (g.end - g.start) as f32;
                return (g.x + g.w * t, line);
            }
            x = g.x + g.w;
        }
        (x, line)
    }

    /// The byte offset a click at `(x, y)` (physical, from the top-left)
    /// puts the caret before.
    pub fn index_at(&self, x: f32, y: f32) -> usize {
        let Some(line) = self
            .lines
            .iter()
            .position(|l| y < l.top + l.height)
            .or(self.lines.len().checked_sub(1))
        else {
            return 0;
        };
        let mut end = self.lines[line].start;
        for g in self.pos.iter().filter(|g| g.line == line) {
            if x < g.x + g.w / 2.0 {
                return g.start;
            }
            end = g.end;
        }
        // Past the last glyph: the end of the line, before any newline.
        end.max(self.lines[line].start)
    }
}

#[derive(Hash, PartialEq, Eq)]
struct ShapeKey {
    text: Box<str>,
    size: u32,
    line: u32,
    weight: u16,
    mono: bool,
    tracking: u32,
    italic: bool,
    max_w: Option<u32>,
}

/// A rasterized glyph in one of the atlases, relative to its pen position.
#[derive(Clone, Copy)]
struct Raster {
    uv: [f32; 4],
    left: f32,
    top: f32,
    colored: bool,
}

/// One glyph of a terminal cell, offset from the cell's top-left pixel.
#[derive(Clone, Copy)]
struct CellPiece {
    raster: Raster,
    dx: f32,
    dy: f32,
}

/// Terminal cell style bits that pick a face.
pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;

/// The monospace cell, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub w: f32,
    pub h: f32,
    pub baseline: f32,
}

pub struct TextSystem {
    pub fonts: FontSystem,
    swash: SwashCache,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    ui_family: &'static str,
    mono_family: &'static str,
    shaped: HashMap<ShapeKey, Rc<Shaped>>,
    /// Shaped strings used since the last [`TextSystem::trim`].
    used: HashSet<*const Shaped>,
    rasters: HashMap<CacheKey, Option<Raster>>,
    term_size: f32,
    cell: Cell,
    ascii: Vec<Option<Option<CellPiece>>>,
    clusters: HashMap<(Box<str>, u8), Rc<[CellPiece]>>,
}

impl TextSystem {
    /// `ui` and `mono` are family names; `term_size` the terminal font size
    /// in logical pixels.
    pub fn new(scale: f32, ui: &'static str, mono: &'static str, term_size: f32) -> TextSystem {
        let mut fonts = FontSystem::new();
        fonts.db_mut().set_sans_serif_family(ui);
        fonts.db_mut().set_monospace_family(mono);
        let mut t = TextSystem {
            fonts,
            swash: SwashCache::new(),
            scale,
            ui_family: ui,
            mono_family: mono,
            shaped: HashMap::new(),
            used: HashSet::new(),
            rasters: HashMap::new(),
            term_size,
            cell: Cell {
                w: 0.0,
                h: 0.0,
                baseline: 0.0,
            },
            ascii: vec![None; 128 * 4],
            clusters: HashMap::new(),
        };
        t.cell = t.measure_cell();
        t
    }

    fn attrs(&self, weight: u16, mono: bool, italic: bool) -> Attrs<'static> {
        let family = if mono {
            Family::Name(self.mono_family)
        } else {
            Family::Name(self.ui_family)
        };
        let mut a = Attrs::new().family(family).weight(Weight(weight));
        if italic {
            a = a.style(Style::Italic);
        }
        a
    }

    /// The terminal cell: the advance of `M` and a 1.2 line, in whole
    /// physical pixels so every cell starts on a pixel.
    fn measure_cell(&mut self) -> Cell {
        let size = self.term_size * self.scale;
        let line = (size * 1.2).ceil();
        let attrs = self.attrs(400, true, false);
        let mut b = Buffer::new(&mut self.fonts, Metrics::new(size, line));
        b.set_text("M", &attrs, Shaping::Advanced, None);
        b.shape_until_scroll(&mut self.fonts, false);
        let (w, baseline) = b
            .layout_runs()
            .next()
            .and_then(|r| r.glyphs.first().map(|g| (g.w, r.line_y)))
            .unwrap_or((size * 0.6, size));
        Cell {
            w: w.round().max(1.0),
            h: line,
            baseline: baseline.round(),
        }
    }

    /// Moves to another display density; rasters at the old one are
    /// dropped with the next atlas reset.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.shaped.clear();
        self.ascii.fill(None);
        self.clusters.clear();
        self.cell = self.measure_cell();
    }

    /// Forgets every raster, after the atlases they pointed into were reset.
    pub fn clear_rasters(&mut self) {
        self.rasters.clear();
        self.ascii.fill(None);
        self.clusters.clear();
    }

    pub fn cell(&self) -> Cell {
        self.cell
    }

    /// The cell in logical pixels, which is what pane sizes are computed in.
    pub fn cell_logical(&self) -> (f32, f32) {
        (self.cell.w / self.scale, self.cell.h / self.scale)
    }

    /// Shapes `text` (cached), wrapping at `max_w` logical pixels if given.
    pub fn shape(&mut self, text: &str, style: TextStyle, max_w: Option<f32>) -> Rc<Shaped> {
        let s = self.scale;
        let key = ShapeKey {
            text: text.into(),
            size: (style.size * s).to_bits(),
            line: (style.line_height * s).to_bits(),
            weight: style.weight,
            mono: style.mono,
            tracking: style.tracking.to_bits(),
            italic: style.italic,
            max_w: max_w.map(|w| (w * s).ceil() as u32),
        };
        if let Some(sh) = self.shaped.get(&key) {
            self.used.insert(Rc::as_ptr(sh));
            return sh.clone();
        }
        let mut attrs = self.attrs(style.weight, style.mono, style.italic);
        if style.tracking != 0.0 {
            attrs = attrs.letter_spacing(style.tracking);
        }
        let mut b = Buffer::new(
            &mut self.fonts,
            Metrics::new(style.size * s, style.line_height * s),
        );
        b.set_wrap(if max_w.is_some() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        b.set_size(max_w.map(|w| (w * s).ceil()), None);
        b.set_text(text, &attrs, Shaping::Advanced, None);
        b.shape_until_scroll(&mut self.fonts, false);
        let para_starts: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let mut glyphs = Vec::new();
        let mut pos = Vec::new();
        let mut lines = Vec::new();
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for run in b.layout_runs() {
            let base = para_starts.get(run.line_i).copied().unwrap_or(text.len());
            let para_end = text[base..].find('\n').map_or(text.len(), |n| base + n);
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
            let li = lines.len();
            let (mut start, mut end) = (usize::MAX, base);
            for g in run.glyphs {
                let (gs, ge) = (base + g.start, base + g.end);
                start = start.min(gs);
                end = end.max(ge);
                pos.push(GlyphPos {
                    x: g.x,
                    w: g.w,
                    start: gs,
                    end: ge,
                    line: li,
                });
                glyphs.push((g.clone(), run.line_y));
            }
            if start == usize::MAX {
                start = base;
                end = para_end;
            }
            lines.push(Line {
                top: run.line_top,
                height: run.line_height,
                w: run.line_w,
                start,
                end,
            });
        }
        let sh = Rc::new(Shaped {
            glyphs,
            pos,
            lines,
            w,
            h,
        });
        self.used.insert(Rc::as_ptr(&sh));
        self.shaped.insert(key, sh.clone());
        sh
    }

    /// Shapes one line of `text`, cut with an ellipsis to fit `max_w`
    /// logical pixels (CSS `truncate`).
    pub fn shape_truncated(&mut self, text: &str, style: TextStyle, max_w: f32) -> Rc<Shaped> {
        let full = self.shape(text, style, None);
        let limit = max_w * self.scale;
        if full.w <= limit + 0.5 {
            return full;
        }
        let ell = self.shape("…", style, None).w;
        let cut = full
            .pos
            .iter()
            .take_while(|g| g.x + g.w + ell <= limit)
            .last()
            .map_or(0, |g| g.end);
        let mut s = text[..cut].trim_end().to_owned();
        s.push('…');
        self.shape(&s, style, None)
    }

    /// Drops shaped strings not used since the last call, so a screen that
    /// keeps changing text does not grow the cache without bound.
    pub fn trim(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.shaped.retain(|_, v| used.contains(&Rc::as_ptr(v)));
    }

    fn raster(&mut self, atlases: &mut Atlases, key: CacheKey) -> Option<Raster> {
        if let Some(hit) = self.rasters.get(&key) {
            return *hit;
        }
        let img = self.swash.get_image_uncached(&mut self.fonts, key);
        let out = img.and_then(|img| {
            let (w, h) = (img.placement.width, img.placement.height);
            let (uv, colored) = match img.content {
                SwashContent::Mask => (atlases.insert_mask(w, h, &img.data)?, false),
                SwashContent::Color => (atlases.insert_color(w, h, &img.data)?, true),
                // Subpixel masks are not requested; keep their coverage.
                SwashContent::SubpixelMask => {
                    let m: Vec<u8> = img.data.chunks(4).map(|p| p[1]).collect();
                    (atlases.insert_mask(w, h, &m)?, false)
                }
            };
            Some(Raster {
                uv,
                left: img.placement.left as f32,
                top: img.placement.top as f32,
                colored,
            })
        });
        // A full atlas is not remembered as "no glyph": after the reset the
        // glyph is rasterized again.
        if out.is_some() || !atlases.full {
            self.rasters.insert(key, out);
        }
        out
    }

    /// Draws `sh` with its top-left at `(x, y)` physical pixels.
    pub fn draw(
        &mut self,
        scene: &mut Scene,
        atlases: &mut Atlases,
        sh: &Shaped,
        (x, y): (f32, f32),
        color: Rgba,
    ) {
        for (g, line_y) in &sh.glyphs {
            let p = g.physical((x, y + line_y), 1.0);
            let Some(ra) = self.raster(atlases, p.cache_key) else {
                continue;
            };
            let rect = Rect::new(
                p.x as f32 + ra.left,
                p.y as f32 - ra.top,
                ra.uv[2],
                ra.uv[3],
            );
            scene.sprite(rect, ra.uv, color, ra.colored);
        }
    }

    /// Draws `sh` as [`TextSystem::draw`] does, each glyph in the colour of
    /// the span (sorted, by byte range) holding its first byte.
    pub fn draw_spans(
        &mut self,
        scene: &mut Scene,
        atlases: &mut Atlases,
        sh: &Shaped,
        (x, y): (f32, f32),
        color: Rgba,
        spans: &[(std::ops::Range<usize>, Rgba)],
    ) {
        for ((g, line_y), pos) in sh.glyphs.iter().zip(&sh.pos) {
            let at = spans.partition_point(|(r, _)| r.end <= pos.start);
            let c = spans
                .get(at)
                .filter(|(r, _)| r.start <= pos.start)
                .map_or(color, |(_, c)| *c);
            let p = g.physical((x, y + line_y), 1.0);
            let Some(ra) = self.raster(atlases, p.cache_key) else {
                continue;
            };
            let rect = Rect::new(
                p.x as f32 + ra.left,
                p.y as f32 - ra.top,
                ra.uv[2],
                ra.uv[3],
            );
            scene.sprite(rect, ra.uv, c, ra.colored);
        }
    }

    /// The terminal font size, logical pixels.
    pub fn term_size(&self) -> f32 {
        self.term_size
    }

    /// Changes the terminal font size; cells are measured again.
    pub fn set_term_size(&mut self, size: f32) {
        if (size - self.term_size).abs() < f32::EPSILON {
            return;
        }
        self.term_size = size;
        self.ascii.fill(None);
        self.clusters.clear();
        self.cell = self.measure_cell();
    }

    /// A terminal cluster's pieces: in the terminal font, or for a symbol
    /// that is text by default (U+23FA) and came back from a colour emoji
    /// face, in the first text symbol face that draws it in one colour.
    fn shape_cell(&mut self, atlases: &mut Atlases, text: &str, style: u8) -> Vec<CellPiece> {
        let weight = if style & BOLD != 0 { 700 } else { 400 };
        let attrs = self.attrs(weight, true, style & ITALIC != 0);
        let pieces = self.shape_cell_in(atlases, text, &attrs);
        if !pieces.iter().any(|p| p.raster.colored) || !text_presentation(text) {
            return pieces;
        }
        for family in TEXT_SYMBOL_FAMILIES {
            let known = self
                .fonts
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n == family));
            if !known {
                continue;
            }
            let a = attrs.clone().family(Family::Name(family));
            let alt = self.shape_cell_in(atlases, text, &a);
            if !alt.is_empty() && !alt.iter().any(|p| p.raster.colored) {
                return alt;
            }
        }
        pieces
    }

    fn shape_cell_in(
        &mut self,
        atlases: &mut Atlases,
        text: &str,
        attrs: &Attrs<'_>,
    ) -> Vec<CellPiece> {
        let size = self.term_size * self.scale;
        let mut b = Buffer::new(&mut self.fonts, Metrics::new(size, self.cell.h));
        b.set_text(text, attrs, Shaping::Advanced, None);
        b.shape_until_scroll(&mut self.fonts, false);
        let glyphs: Vec<LayoutGlyph> = b
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().cloned())
            .collect();
        let baseline = self.cell.baseline;
        glyphs
            .iter()
            .filter_map(|g| {
                let p = g.physical((0.0, baseline), 1.0);
                let ra = self.raster(atlases, p.cache_key)?;
                Some(CellPiece {
                    raster: ra,
                    dx: p.x as f32 + ra.left,
                    dy: p.y as f32 - ra.top,
                })
            })
            .collect()
    }

    /// Draws one terminal cell's text with the cell's top-left at the
    /// integer pixel `(x, y)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_cell(
        &mut self,
        scene: &mut Scene,
        atlases: &mut Atlases,
        text: &str,
        style: u8,
        (x, y): (f32, f32),
        color: Rgba,
    ) {
        let b = text.as_bytes();
        if b.len() == 1 && b[0] < 128 {
            let slot = usize::from(b[0]) * 4 + usize::from(style & 3);
            let piece = match self.ascii[slot] {
                Some(p) => p,
                None => {
                    let p = self.shape_cell(atlases, text, style).first().copied();
                    if p.is_some() || !atlases.full {
                        self.ascii[slot] = Some(p);
                    }
                    p
                }
            };
            if let Some(p) = piece {
                put(scene, &p, x, y, color);
            }
            return;
        }
        let key = (Box::<str>::from(text), style & 3);
        let pieces = match self.clusters.get(&key) {
            Some(p) => p.clone(),
            None => {
                let p: Rc<[CellPiece]> = self.shape_cell(atlases, text, style).into();
                if !atlases.full {
                    self.clusters.insert(key, p.clone());
                }
                p
            }
        };
        for p in pieces.iter() {
            put(scene, p, x, y, color);
        }
    }
}

/// Faces with one-colour symbols, tried in order for a cluster that should
/// not be drawn as an emoji.
const TEXT_SYMBOL_FAMILIES: &[&str] = if cfg!(target_os = "macos") {
    &[
        "Apple Symbols",
        "STIX Two Math",
        "Arial Unicode MS",
        "Menlo",
    ]
} else if cfg!(windows) {
    &["Segoe UI Symbol", "Cambria Math", "Consolas"]
} else {
    &[
        "DejaVu Sans Mono",
        "DejaVu Sans",
        "Noto Sans Symbols 2",
        "Noto Sans Symbols",
        "Symbola",
    ]
};

/// Unicode's `Emoji_Presentation=Yes` ranges (emoji-data.txt, Unicode 16):
/// the code points drawn as emoji without a variation selector.
const EMOJI_PRESENTATION: &[(u32, u32)] = &[
    (0x231A, 0x231B),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1E6, 0x1F1FF),
    (0x1F201, 0x1F201),
    (0x1F21A, 0x1F21A),
    (0x1F22F, 0x1F22F),
    (0x1F232, 0x1F236),
    (0x1F238, 0x1F23A),
    (0x1F250, 0x1F251),
    (0x1F300, 0x1F320),
    (0x1F32D, 0x1F335),
    (0x1F337, 0x1F37C),
    (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA),
    (0x1F3CF, 0x1F3D3),
    (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F43E),
    (0x1F440, 0x1F440),
    (0x1F442, 0x1F4FC),
    (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567),
    (0x1F57A, 0x1F57A),
    (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4),
    (0x1F5FB, 0x1F64F),
    (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2),
    (0x1F6D5, 0x1F6D7),
    (0x1F6DC, 0x1F6DF),
    (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC),
    (0x1F7E0, 0x1F7EB),
    (0x1F7F0, 0x1F7F0),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1F9FF),
    (0x1FA70, 0x1FA7C),
    (0x1FA80, 0x1FA89),
    (0x1FA8F, 0x1FAC6),
    (0x1FACE, 0x1FADC),
    (0x1FADF, 0x1FAE9),
    (0x1FAF0, 0x1FAF8),
];

/// Whether a cluster should be drawn as text: its first code point is not
/// emoji by default and no VS16 (U+FE0F) asks for emoji, as terminals do.
pub fn text_presentation(cluster: &str) -> bool {
    let Some(first) = cluster.chars().next() else {
        return true;
    };
    if cluster.contains('\u{fe0f}') {
        return false;
    }
    let c = u32::from(first);
    let at = EMOJI_PRESENTATION.partition_point(|&(_, hi)| hi < c);
    !EMOJI_PRESENTATION.get(at).is_some_and(|&(lo, _)| lo <= c)
}

fn put(scene: &mut Scene, p: &CellPiece, x: f32, y: f32, color: Rgba) {
    let ra = &p.raster;
    scene.sprite(
        Rect::new(x + p.dx, y + p.dy, ra.uv[2], ra.uv[3]),
        ra.uv,
        color,
        ra.colored,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_text_unless_emoji_by_default_or_asked_for() {
        assert!(text_presentation("\u{23fa}"));
        assert!(text_presentation("\u{2714}"));
        assert!(text_presentation("a"));
        assert!(!text_presentation("\u{23fa}\u{fe0f}"));
        assert!(!text_presentation("\u{231a}"));
        assert!(!text_presentation("\u{1f680}"));
        assert!(!text_presentation("\u{1f1fa}\u{1f1f8}"));
    }

    #[test]
    fn emoji_ranges_are_sorted_for_the_search() {
        assert!(EMOJI_PRESENTATION
            .windows(2)
            .all(|w| w[0].1 < w[1].0 && w[0].0 <= w[0].1));
    }
}
