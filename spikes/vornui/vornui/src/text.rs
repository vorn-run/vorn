//! Text: cosmic-text shapes, swash rasterizes, the atlas keeps the result.
//!
//! UI strings are shaped once per (string, style, width) and the shaped
//! glyphs reused while the string is on screen. Terminal cells take a faster
//! path: every cell sits on an integer pixel, so a cell's glyph is rasterized
//! once per (cluster, style) and a frame only copies atlas references.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, LayoutGlyph, Metrics, Shaping, Style, SwashCache,
    SwashContent, Weight, Wrap,
};

use crate::gpu::{Rect, Renderer, Rgba, Scene};

/// How a UI string looks. Sizes are logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
    pub mono: bool,
}

impl TextStyle {
    pub fn ui(size: f32) -> TextStyle {
        TextStyle {
            size,
            line_height: (size * 1.5).round(),
            weight: 400,
            mono: false,
        }
    }
}

/// A shaped string, in physical pixels.
pub struct Shaped {
    glyphs: Vec<(LayoutGlyph, f32)>,
    pub w: f32,
    pub h: f32,
}

#[derive(Hash, PartialEq, Eq)]
struct ShapeKey {
    text: Box<str>,
    size: u32,
    line: u32,
    weight: u16,
    mono: bool,
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

    /// Moves to another display density. Old rasters stay in the atlas
    /// unused; a complete layer would evict them.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.shaped.clear();
        self.ascii.fill(None);
        self.clusters.clear();
        self.cell = self.measure_cell();
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
            max_w: max_w.map(|w| (w * s).ceil() as u32),
        };
        if let Some(sh) = self.shaped.get(&key) {
            self.used.insert(Rc::as_ptr(sh));
            return sh.clone();
        }
        let attrs = self.attrs(style.weight, style.mono, false);
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
        let mut glyphs = Vec::new();
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for run in b.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
            glyphs.extend(run.glyphs.iter().map(|g| (g.clone(), run.line_y)));
        }
        let sh = Rc::new(Shaped { glyphs, w, h });
        self.used.insert(Rc::as_ptr(&sh));
        self.shaped.insert(key, sh.clone());
        sh
    }

    /// Drops shaped strings not used since the last call, so a screen that
    /// keeps changing text does not grow the cache without bound.
    pub fn trim(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.shaped.retain(|_, v| used.contains(&Rc::as_ptr(v)));
    }

    fn raster(&mut self, r: &mut Renderer, queue: &wgpu::Queue, key: CacheKey) -> Option<Raster> {
        if let Some(hit) = self.rasters.get(&key) {
            return *hit;
        }
        let out = self
            .swash
            .get_image_uncached(&mut self.fonts, key)
            .and_then(|img| {
                let (w, h) = (img.placement.width, img.placement.height);
                let (uv, colored) = match img.content {
                    SwashContent::Mask => (r.mask.insert(queue, w, h, &img.data)?, false),
                    SwashContent::Color => (r.color.insert(queue, w, h, &img.data)?, true),
                    // Subpixel masks are not requested; keep their coverage.
                    SwashContent::SubpixelMask => {
                        let m: Vec<u8> = img.data.chunks(4).map(|p| p[1]).collect();
                        (r.mask.insert(queue, w, h, &m)?, false)
                    }
                };
                Some(Raster {
                    uv,
                    left: img.placement.left as f32,
                    top: img.placement.top as f32,
                    colored,
                })
            });
        self.rasters.insert(key, out);
        out
    }

    /// Draws `sh` with its top-left at `(x, y)` physical pixels.
    pub fn draw(
        &mut self,
        scene: &mut Scene,
        r: &mut Renderer,
        queue: &wgpu::Queue,
        sh: &Shaped,
        (x, y): (f32, f32),
        color: Rgba,
    ) {
        for (g, line_y) in &sh.glyphs {
            let p = g.physical((x, y + line_y), 1.0);
            let Some(ra) = self.raster(r, queue, p.cache_key) else {
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

    fn shape_cell(
        &mut self,
        r: &mut Renderer,
        queue: &wgpu::Queue,
        text: &str,
        style: u8,
    ) -> Vec<CellPiece> {
        let size = self.term_size * self.scale;
        let weight = if style & BOLD != 0 { 700 } else { 400 };
        let attrs = self.attrs(weight, true, style & ITALIC != 0);
        let mut b = Buffer::new(&mut self.fonts, Metrics::new(size, self.cell.h));
        b.set_text(text, &attrs, Shaping::Advanced, None);
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
                let ra = self.raster(r, queue, p.cache_key)?;
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
        r: &mut Renderer,
        queue: &wgpu::Queue,
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
                    let p = self.shape_cell(r, queue, text, style).first().copied();
                    self.ascii[slot] = Some(p);
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
                let p: Rc<[CellPiece]> = self.shape_cell(r, queue, text, style).into();
                self.clusters.insert(key, p.clone());
                p
            }
        };
        for p in pieces.iter() {
            put(scene, p, x, y, color);
        }
    }
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
