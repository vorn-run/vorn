//! The CPU renderer, for machines whose only wgpu adapter rasterizes in
//! software (WARP, llvmpipe), where the GPU pipelines cost hundreds of
//! milliseconds a frame.
//!
//! It draws the same [`Scene`] with the same arithmetic as `shader.wgsl`, so
//! a frame looks the same whichever renderer drew it. Three things make it
//! fast enough:
//!
//! - **Damage.** The frame is cut into 64-pixel tiles. Each tile's hash
//!   covers, in order, every primitive that touches it; only tiles whose
//!   hash changed are drawn again. A terminal that prints a line redraws
//!   that pane's tiles, not the window.
//! - **Fast paths.** Glyphs and icons sit on whole pixels at their atlas
//!   size, so they are copied texel for texel; quad interiors are filled as
//!   spans; only antialiased edges run the full distance-field math.
//! - **Threads.** Dirty bands of tiles are shared among a pool of up to four
//!   threads that live as long as the renderer. On Windows they run above
//!   normal priority: the terminals the frame shows can flood every core
//!   with output, and threads spawned per frame at normal priority waited
//!   their turn to start while the frame waited on them.
//!
//! It is its own small rasterizer rather than a general one (tiny-skia)
//! because the scene has few primitives and must match the shader's
//! distance field and sampling exactly; a path rasterizer would need them
//! re-expressed as paths and would still differ at the edges.
//!
//! A backdrop blur reads what is under it, so it splits the frame: the
//! primitives before it are drawn into every dirty tile, the blur runs over
//! the whole frame, then drawing resumes. Tiles a blur reads from are
//! redrawn together, so it never reads a pixel that already shows what is
//! drawn over it.

use rayon::prelude::*;

use crate::atlas::Atlases;
use crate::scene::{luminance, text_coverage, Blur, Quad, Rgba, Scene, Sprite};

/// Tile side, in pixels.
const TILE: usize = 64;
/// Below this many dirty pixels a frame is drawn on the calling thread.
const PARALLEL_MIN_PX: usize = 128 * 1024;
const MAX_THREADS: usize = 4;

/// A frame in memory: RGBA8 (or BGRA8) pixels, one `u32` each in memory
/// order; BGRA is what a window's pixel buffer takes once alpha is cleared.
pub struct CpuRaster {
    size: (u32, u32),
    pixels: Vec<u32>,
    bgra: bool,
    /// Draws the bands of large frames; `None` on a single core or if the
    /// threads could not be started.
    pool: Option<rayon::ThreadPool>,
    /// Per tile: the hash of what it showed last frame.
    hashes: Vec<u64>,
    /// Per tile: the primitives touching it this frame, in draw order.
    bins: Vec<Vec<u32>>,
    dirty: Vec<bool>,
    damage: Vec<(u32, u32)>,
    /// Glyph coverage correction tables, kept across frames.
    luts: Vec<[u8; 256]>,
    /// Per (contrast, gamma) bits, each luminance level's index into
    /// `luts`: a direct lookup, as a terminal can show thousands of colours.
    lut_index: Vec<([u32; 2], [u16; LUM_LEVELS + 1])>,
}

/// A sprite's index into [`CpuRaster::luts`] when it has none.
const NO_LUT: u16 = u16::MAX;
/// Text luminance steps the correction tables are made for. The mix is
/// linear in luminance, so 64 steps put every table within 1/255 of exact.
const LUM_LEVELS: usize = 64;
/// More (contrast, gamma) settings than this and the tables start over.
const MAX_GAMMAS: usize = 8;
/// The correction table of uncorrected text.
const IDENTITY: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = i as u8;
        i += 1;
    }
    t
};

/// A primitive with the pixels it may touch: its coverage within its clip
/// and the frame, as `[x0, y0, x1, y1)`.
#[derive(Clone, Copy)]
enum Prim<'a> {
    Quad(&'a Quad, [i32; 4]),
    /// With its glyph correction's index in the frame's tables.
    Sprite(&'a Sprite, [i32; 4], u16),
    Blur(&'a Blur, [i32; 4]),
}

impl CpuRaster {
    /// A renderer whose pixels are BGRA in memory when `bgra`, as some
    /// window surfaces want; RGBA otherwise.
    pub fn new(bgra: bool) -> CpuRaster {
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(MAX_THREADS);
        let pool = (threads > 1)
            .then(|| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .thread_name(|i| format!("vornui-raster-{i}"))
                    .start_handler(|_| raise_priority())
                    .build()
                    .ok()
            })
            .flatten();
        CpuRaster {
            size: (0, 0),
            pixels: Vec::new(),
            bgra,
            pool,
            hashes: Vec::new(),
            bins: Vec::new(),
            dirty: Vec::new(),
            damage: Vec::new(),
            luts: Vec::new(),
            lut_index: Vec::new(),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The last frame, row-major, 4 bytes a pixel.
    pub fn bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.pixels)
    }

    /// Row ranges `[y0, y1)` the last [`CpuRaster::render`] changed.
    pub fn damage(&self) -> &[(u32, u32)] {
        &self.damage
    }

    /// Copies the frame into `dst` as `0x00RRGGBB` pixels (a BGRA frame
    /// with its alpha byte cleared): the damaged rows, or all of them when
    /// `whole`. Answers the damage, or `None` when the sizes differ or the
    /// frame is RGBA.
    pub fn copy_xrgb(&self, dst: &mut [u32], whole: bool) -> Option<&[(u32, u32)]> {
        if !self.bgra || dst.len() != self.pixels.len() {
            return None;
        }
        let w = self.size.0 as usize;
        let mut copy = |y0: u32, y1: u32| {
            let rows = y0 as usize * w..y1 as usize * w;
            for (d, s) in dst[rows.clone()].iter_mut().zip(&self.pixels[rows]) {
                *d = s & 0x00ff_ffff;
            }
        };
        if whole {
            copy(0, self.size.1);
        } else {
            self.damage.iter().for_each(|&(y0, y1)| copy(y0, y1));
        }
        Some(&self.damage)
    }

    pub fn is_bgra(&self) -> bool {
        self.bgra
    }

    /// Forgets the last frame, so the next one is drawn whole.
    pub fn invalidate(&mut self) {
        self.hashes.clear();
    }

    /// Draws `scene` into a `size` frame, redrawing only tiles whose
    /// content changed since the last call.
    pub fn render(&mut self, scene: &Scene, atlases: &Atlases, clear: Rgba, size: (u32, u32)) {
        RAISED.with(|r| {
            if !r.replace(true) {
                raise_priority();
            }
        });
        let (w, h) = (size.0 as usize, size.1 as usize);
        let tiles_x = w.div_ceil(TILE);
        let tiles_y = h.div_ceil(TILE);
        let n = tiles_x * tiles_y;
        if self.size != size {
            self.size = size;
            self.pixels = vec![0; w * h];
            self.hashes.clear();
        }
        self.bins.resize_with(n, Vec::new);
        self.bins.truncate(n);
        self.bins.iter_mut().for_each(Vec::clear);
        let clear_px = pack([clear.0[0], clear.0[1], clear.0[2], 1.0], 1.0, self.bgra);
        let seed = mix(
            mix(
                mix(FX_SEED, u64::from(clear_px)),
                (w as u64) << 32 | h as u64,
            ),
            atlases.epoch,
        );
        let mut tile_hash = vec![seed; n];
        let mut prims = Vec::new();
        let mut blurs = Vec::new();
        if self.lut_index.len() > MAX_GAMMAS {
            self.lut_index.clear();
            self.luts.clear();
        }
        let frame = [0, 0, w as i32, h as i32];
        for l in &scene.layers {
            let blur_prims = l
                .blurs
                .iter()
                .map(|b| Prim::Blur(b, coverage(b.rect, b.clip, 1.0)));
            let quads = l
                .quads
                .iter()
                .map(|q| Prim::Quad(q, coverage(q.rect, q.clip, q.reach())));
            let sprites = l
                .sprites
                .iter()
                .map(|s| Prim::Sprite(s, coverage(s.rect, s.clip, 0.0), NO_LUT));
            for p in blur_prims.chain(quads).chain(sprites) {
                let b = intersect(p.bounds(), frame);
                if b[0] >= b[2] || b[1] >= b[3] {
                    continue;
                }
                let ph = p.hash();
                let idx = prims.len() as u32;
                let p = match p {
                    Prim::Sprite(s, _, _) => Prim::Sprite(s, b, self.lut(s)),
                    Prim::Blur(..) => {
                        blurs.push(idx);
                        p.with_bounds(b)
                    }
                    Prim::Quad(..) => p.with_bounds(b),
                };
                prims.push(p);
                let (tx0, tx1) = (b[0] as usize / TILE, (b[2] as usize - 1) / TILE);
                let (ty0, ty1) = (b[1] as usize / TILE, (b[3] as usize - 1) / TILE);
                for ty in ty0..=ty1 {
                    for tx in tx0..=tx1 {
                        let t = ty * tiles_x + tx;
                        self.bins[t].push(idx);
                        tile_hash[t] = mix(tile_hash[t], ph);
                    }
                }
            }
        }
        let fresh = self.hashes.len() != n;
        self.dirty.clear();
        self.dirty.extend(
            tile_hash
                .iter()
                .enumerate()
                .map(|(i, h)| fresh || self.hashes[i] != *h),
        );
        self.hashes = tile_hash;
        spread_blur_damage(&mut self.dirty, tiles_x, &prims, &blurs, frame);
        self.damage.clear();
        let mut bands = Vec::new();
        let mut dirty_px = 0;
        for b in 0..tiles_y {
            let tiles = &self.dirty[b * tiles_x..(b + 1) * tiles_x];
            let count = tiles.iter().filter(|d| **d).count();
            if count == 0 {
                continue;
            }
            let rows = TILE.min(h - b * TILE);
            dirty_px += count * TILE * rows;
            let (y0, y1) = ((b * TILE) as u32, (b * TILE + rows) as u32);
            match self.damage.last_mut() {
                Some(last) if last.1 == y0 => last.1 = y1,
                _ => self.damage.push((y0, y1)),
            }
            bands.push(b);
        }
        // Drawing stops at each blur, which runs over the whole frame.
        let mut start = 0u32;
        for end in blurs.iter().copied().chain(Some(prims.len() as u32)) {
            let job = Job {
                w,
                tiles_x,
                dirty: &self.dirty,
                bins: &self.bins,
                prims: &prims,
                range: (start, end),
                mask: atlases.mask.pixels().unwrap_or(&[]),
                mask_size: atlases.mask.size as usize,
                color: atlases.color.pixels().unwrap_or(&[]),
                color_size: atlases.color.size as usize,
                luts: &self.luts,
                clear: (start == 0).then_some(clear_px),
                bgra: self.bgra,
            };
            let rows = self
                .pixels
                .chunks_mut(w * TILE)
                .enumerate()
                .filter(|(b, _)| bands.binary_search(b).is_ok());
            match &self.pool {
                Some(pool) if dirty_px >= PARALLEL_MIN_PX && bands.len() >= 2 => {
                    // One band per task so a busy band does not hold up an idle thread.
                    let rows: Vec<_> = rows.collect();
                    pool.install(|| {
                        rows.into_par_iter()
                            .with_max_len(1)
                            .for_each(|(b, rows)| job.band(b, rows));
                    });
                }
                _ => rows.for_each(|(b, rows)| job.band(b, rows)),
            }
            if let Some(Prim::Blur(blur, b)) = prims.get(end as usize) {
                if any_dirty(&self.dirty, tiles_x, *b) {
                    backdrop(&mut self.pixels, (w, h), blur, *b);
                }
            }
            start = end + 1;
        }
    }

    /// The index of `s`'s glyph correction table, made if new.
    fn lut(&mut self, s: &Sprite) -> u16 {
        let Some(g) = s.text_gamma() else {
            return NO_LUT;
        };
        let key = [g.contrast.to_bits(), g.gamma.to_bits()];
        let slot = match self.lut_index.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                self.lut_index.push((key, [NO_LUT; LUM_LEVELS + 1]));
                self.lut_index.len() - 1
            }
        };
        let lum = luminance([s.color[0], s.color[1], s.color[2]]).clamp(0.0, 1.0);
        let level = (lum * LUM_LEVELS as f32).round() as usize;
        let i = self.lut_index[slot].1[level];
        if i != NO_LUT {
            return i;
        }
        let lum = level as f32 / LUM_LEVELS as f32;
        let mut t = [0u8; 256];
        for (m, v) in t.iter_mut().enumerate() {
            *v = unit(text_coverage(m as f32 / 255.0, lum, g.contrast, g.gamma)) as u8;
        }
        self.luts.push(t);
        let i = (self.luts.len() - 1) as u16;
        self.lut_index[slot].1[level] = i;
        i
    }
}

/// The tiles `b` covers, clamped to a `tiles_x` wide grid: inclusive
/// column and row ranges.
fn tiles_of(b: [i32; 4]) -> ([usize; 2], [usize; 2]) {
    (
        [b[0] as usize / TILE, (b[2] as usize - 1) / TILE],
        [b[1] as usize / TILE, (b[3] as usize - 1) / TILE],
    )
}

fn any_dirty(dirty: &[bool], tiles_x: usize, b: [i32; 4]) -> bool {
    let ([x0, x1], [y0, y1]) = tiles_of(b);
    (y0..=y1).any(|ty| dirty[ty * tiles_x + x0..=ty * tiles_x + x1].contains(&true))
}

/// Makes a blur's whole source dirty when any of it is: the blur reads
/// every pixel within its margin, and a clean tile there shows what was
/// drawn over the blur last frame, not what is under it. Repeats until no
/// blur's source changes, as one blur's spread can reach another's.
fn spread_blur_damage(
    dirty: &mut [bool],
    tiles_x: usize,
    prims: &[Prim<'_>],
    blurs: &[u32],
    frame: [i32; 4],
) {
    let sources: Vec<[i32; 4]> = blurs
        .iter()
        .filter_map(|&i| match prims[i as usize] {
            Prim::Blur(bl, b) => {
                let m = bl.margin().ceil() as i32;
                Some(intersect([b[0] - m, b[1] - m, b[2] + m, b[3] + m], frame))
            }
            _ => None,
        })
        .collect();
    loop {
        let mut changed = false;
        for &s in &sources {
            if !any_dirty(dirty, tiles_x, s) {
                continue;
            }
            let ([x0, x1], [y0, y1]) = tiles_of(s);
            for ty in y0..=y1 {
                for d in &mut dirty[ty * tiles_x + x0..=ty * tiles_x + x1] {
                    changed |= !*d;
                    *d = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

thread_local! {
    /// Whether this thread's priority was already raised for rendering.
    static RAISED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Lets the calling thread preempt normal-priority work, so a frame is not
/// stalled behind the processes whose output it shows. Windows only: elsewhere
/// raising a thread's priority needs privileges the app does not have.
fn raise_priority() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        // SAFETY: the pseudo-handle names the calling thread and needs no closing.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        }
    }
}

impl Prim<'_> {
    fn bounds(&self) -> [i32; 4] {
        match self {
            Prim::Quad(_, b) | Prim::Sprite(_, b, _) | Prim::Blur(_, b) => *b,
        }
    }

    fn with_bounds(self, b: [i32; 4]) -> Self {
        match self {
            Prim::Quad(q, _) => Prim::Quad(q, b),
            Prim::Sprite(s, _, l) => Prim::Sprite(s, b, l),
            Prim::Blur(bl, _) => Prim::Blur(bl, b),
        }
    }

    fn hash(&self) -> u64 {
        let (tag, bytes): (u64, &[u8]) = match self {
            Prim::Quad(q, _) => (1, bytemuck::bytes_of(*q)),
            Prim::Sprite(s, _, _) => (2, bytemuck::bytes_of(*s)),
            Prim::Blur(b, _) => {
                let fields = [b.sigma, b.radius, b.alpha];
                let h = b.rect.iter().chain(&b.clip).chain(&fields);
                return h.fold(mix(FX_SEED, 3), |h, v| mix(h, u64::from(v.to_bits())));
            }
        };
        bytes.chunks_exact(8).fold(mix(FX_SEED, tag), |h, c| {
            mix(h, u64::from_le_bytes(c.try_into().unwrap_or([0; 8])))
        })
    }
}

const FX_SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// An order-dependent 64-bit mix (FxHash's step with a final rotate).
fn mix(h: u64, v: u64) -> u64 {
    (h.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95)
}

/// The pixels whose centres fall inside `rect` grown by `reach` and inside
/// `clip`, as the GPU rasterizes a quad and the shader's clip test keeps its
/// fragments.
fn coverage(rect: [f32; 4], clip: [f32; 4], reach: f32) -> [i32; 4] {
    let x0 = (rect[0] - reach).max(clip[0]);
    let y0 = (rect[1] - reach).max(clip[1]);
    let x1 = (rect[0] + rect[2] + reach).min(clip[2]);
    let y1 = (rect[1] + rect[3] + reach).min(clip[3]);
    let edge = |v: f32| (v - 0.5).ceil().clamp(-1.0e6, 1.0e6) as i32;
    [edge(x0), edge(y0), edge(x1), edge(y1)]
}

fn intersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

/// What every band of one frame reads.
struct Job<'a> {
    w: usize,
    tiles_x: usize,
    dirty: &'a [bool],
    bins: &'a [Vec<u32>],
    prims: &'a [Prim<'a>],
    /// The primitives this pass draws, `[start, end)`: those between two
    /// blurs.
    range: (u32, u32),
    mask: &'a [u8],
    mask_size: usize,
    color: &'a [u8],
    color_size: usize,
    luts: &'a [[u8; 256]],
    /// What dirty tiles start as, on the first pass only.
    clear: Option<u32>,
    bgra: bool,
}

/// A band's pixels and where they sit in the frame.
struct Band<'p> {
    px: &'p mut [u32],
    w: usize,
    y0: i32,
}

impl Band<'_> {
    fn row(&mut self, y: i32, x0: i32, x1: i32) -> &mut [u32] {
        let at = (y - self.y0) as usize * self.w;
        &mut self.px[at + x0 as usize..at + x1 as usize]
    }
}

impl Job<'_> {
    fn band(&self, b: usize, px: &mut [u32]) {
        let rows = px.len() / self.w;
        let y0 = (b * TILE) as i32;
        let mut band = Band { px, w: self.w, y0 };
        for tx in 0..self.tiles_x {
            let t = b * self.tiles_x + tx;
            if !self.dirty[t] {
                continue;
            }
            let area = [
                (tx * TILE) as i32,
                y0,
                ((tx + 1) * TILE).min(self.w) as i32,
                y0 + rows as i32,
            ];
            if let Some(clear) = self.clear {
                for y in area[1]..area[3] {
                    band.row(y, area[0], area[2]).fill(clear);
                }
            }
            let (start, end) = self.range;
            for &i in &self.bins[t] {
                if i < start {
                    continue;
                }
                if i >= end {
                    break;
                }
                match self.prims[i as usize] {
                    Prim::Quad(q, bounds) if q.is_shadow() => {
                        self.shadow(&mut band, q, intersect(bounds, area));
                    }
                    Prim::Quad(q, bounds) => self.quad(&mut band, q, intersect(bounds, area)),
                    Prim::Sprite(s, bounds, lut) => {
                        let lut = self.luts.get(usize::from(lut));
                        self.sprite(&mut band, s, intersect(bounds, area), lut);
                    }
                    Prim::Blur(..) => {}
                }
            }
        }
    }

    fn quad(&self, band: &mut Band<'_>, q: &Quad, a: [i32; 4]) {
        let [x, y, w, h] = q.rect;
        let half = [w * 0.5, h * 0.5];
        let r = q.params[0].min(half[0]).min(half[1]).max(0.0);
        let bw = q.params[1];
        let fill = pack(q.color, q.color[3], self.bgra);
        // Pixels at least this far inside every edge are the plain fill.
        let inset = r.max(bw + 1.0).max(1.0);
        let ix0 = (x + inset - 0.5).ceil() as i32;
        let ix1 = (x + w - inset - 0.5).ceil() as i32;
        let iy0 = (y + inset - 0.5).ceil() as i32;
        let iy1 = (y + h - inset - 0.5).ceil() as i32;
        for py in a[1]..a[3] {
            let interior_row = py >= iy0 && py < iy1;
            let (s0, s1) = if interior_row {
                (ix0.clamp(a[0], a[2]), ix1.clamp(a[0], a[2]))
            } else {
                (a[2], a[2])
            };
            let row = band.row(py, a[0], a[2]);
            for (px, d) in (a[0]..s0).chain(s1.max(s0)..a[2]).map(|px| (px, px - a[0])) {
                let c = quad_pixel(q, half, r, bw, px as f32 + 0.5 - x, py as f32 + 0.5 - y);
                let dst = &mut row[d as usize];
                *dst = blend_f(*dst, c, self.bgra);
            }
            if s1 > s0 {
                fill_span(&mut row[(s0 - a[0]) as usize..(s1 - a[0]) as usize], fill);
            }
        }
    }

    /// Draws shadow `q` over `a`, skipping the pixels its box cuts out.
    fn shadow(&self, band: &mut Band<'_>, q: &Quad, a: [i32; 4]) {
        let sh = ShadowShape::of(q);
        let [bx, by, bw, bh] = q.border_color;
        // Pixels at least this far inside the box are cut out entirely.
        let inset = sh.box_r + 1.0;
        let ix0 = (bx + inset - 0.5).ceil() as i32;
        let ix1 = (bx + bw - inset - 0.5).ceil() as i32;
        let iy0 = (by + inset - 0.5).ceil() as i32;
        let iy1 = (by + bh - inset - 0.5).ceil() as i32;
        for py in a[1]..a[3] {
            let (s0, s1) = if py >= iy0 && py < iy1 {
                (ix0.clamp(a[0], a[2]), ix1.clamp(a[0], a[2]))
            } else {
                (a[2], a[2])
            };
            let row = band.row(py, a[0], a[2]);
            for px in (a[0]..s0).chain(s1.max(s0)..a[2]) {
                let c = sh.pixel(q, px as f32 + 0.5, py as f32 + 0.5);
                let dst = &mut row[(px - a[0]) as usize];
                *dst = blend_f(*dst, c, self.bgra);
            }
        }
    }

    fn sprite(&self, band: &mut Band<'_>, s: &Sprite, a: [i32; 4], lut: Option<&[u8; 256]>) {
        if a[0] >= a[2] || a[1] >= a[3] {
            return;
        }
        let colored = s.is_color();
        let (tex, size, bpp) = if colored {
            (self.color, self.color_size, 4)
        } else {
            (self.mask, self.mask_size, 1)
        };
        if tex.len() < size * size * bpp || size == 0 {
            return;
        }
        let [x, y, w, h] = s.rect;
        let uv = s.uv;
        let exact = w == uv[2] && h == uv[3] && x.fract() == 0.0 && y.fract() == 0.0;
        if exact {
            let ca = unit(s.color[3]);
            let tint = pack(s.color, s.color[3], self.bgra);
            for py in a[1]..a[3] {
                let ty = uv[1] as usize + (py - y as i32) as usize;
                let tx0 = uv[0] as usize + (a[0] - x as i32) as usize;
                let n = (a[2] - a[0]) as usize;
                let row = band.row(py, a[0], a[2]);
                if colored {
                    let src = &tex[(ty * size + tx0) * 4..(ty * size + tx0 + n) * 4];
                    for (dst, t) in row.iter_mut().zip(src.chunks_exact(4)) {
                        let al = div255(u32::from(t[3]) * ca);
                        if al == 0 {
                            continue;
                        }
                        let c = premul_texel(t, al, self.bgra);
                        *dst = over(*dst, c);
                    }
                } else {
                    let src = &tex[ty * size + tx0..ty * size + tx0 + n];
                    let lut = lut.unwrap_or(&IDENTITY);
                    for (dst, &m) in row.iter_mut().zip(src) {
                        if m == 0 {
                            continue;
                        }
                        let m = lut[usize::from(m)];
                        *dst = over(*dst, scale(tint, u32::from(m)));
                    }
                }
            }
            return;
        }
        // Scaled or off-pixel: bilinear, as the GPU's linear sampler.
        let (sx, sy) = (uv[2] / w, uv[3] / h);
        for py in a[1]..a[3] {
            let v = uv[1] + (py as f32 + 0.5 - y) * sy - 0.5;
            let row = band.row(py, a[0], a[2]);
            for (d, px) in (a[0]..a[2]).enumerate() {
                let u = uv[0] + (px as f32 + 0.5 - x) * sx - 0.5;
                let t = bilinear(tex, size, bpp, u, v);
                let c = if colored {
                    let al = t[3] * s.color[3];
                    [t[0] * al, t[1] * al, t[2] * al, al]
                } else {
                    let m = match lut {
                        Some(l) => {
                            let f = t[0].clamp(0.0, 1.0) * 255.0;
                            let (i, k) = (f.floor() as usize, f.fract());
                            let lo = f32::from(l[i]);
                            let hi = f32::from(l[(i + 1).min(255)]);
                            (lo + (hi - lo) * k) / 255.0
                        }
                        None => t[0],
                    };
                    let al = m * s.color[3];
                    [s.color[0] * al, s.color[1] * al, s.color[2] * al, al]
                };
                row[d] = blend_f(row[d], c, self.bgra);
            }
        }
    }
}

/// The shader's `rounded_box`: the signed distance from `p` (from the
/// centre) to a box of half size `half` and corner radius `r`.
fn rounded_box(p: [f32; 2], half: [f32; 2], r: f32) -> f32 {
    let qx = p[0].abs() - half[0] + r;
    let qy = p[1].abs() - half[1] + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// The shader's quad colour at `local` (from the quad's top-left),
/// premultiplied.
fn quad_pixel(q: &Quad, half: [f32; 2], r: f32, bw: f32, lx: f32, ly: f32) -> [f32; 4] {
    let d = rounded_box([lx - half[0], ly - half[1]], half, r);
    let outer = (0.5 - d).clamp(0.0, 1.0);
    let mut c = q.color;
    if bw > 0.0 {
        let inner = (0.5 - (d + bw)).clamp(0.0, 1.0);
        let b = q.border_color;
        // The fill runs under the border, as CSS paints a background.
        let fill_a = c[3];
        let border_a = b[3] * (1.0 - inner);
        let a = border_a + fill_a * (1.0 - border_a);
        let k = a.max(1e-5);
        let ch = |i: usize| (b[i] * border_a + c[i] * fill_a * (1.0 - border_a)) / k;
        c = [ch(0), ch(1), ch(2), a];
    }
    let a = c[3] * outer;
    [c[0] * a, c[1] * a, c[2] * a, a]
}

/// One level of a blur's downsampling chain: the texels of `area` (in
/// that level's grid), RGBA8 as the GPU stores them between passes.
struct Level {
    area: [i32; 4],
    px: Vec<[u8; 4]>,
}

impl Level {
    fn new(area: [i32; 4], mut texel: impl FnMut(i32, i32) -> [u8; 4]) -> Level {
        let mut px = Vec::with_capacity(((area[2] - area[0]) * (area[3] - area[1])) as usize);
        for y in area[1]..area[3] {
            for x in area[0]..area[2] {
                px.push(texel(x, y));
            }
        }
        Level { area, px }
    }

    /// The texel at `(x, y)`, clamped to the level's area as the GPU
    /// clamps its loads to the texture.
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        let x = x.clamp(self.area[0], self.area[2] - 1) - self.area[0];
        let y = y.clamp(self.area[1], self.area[3] - 1) - self.area[1];
        self.px[(y * (self.area[2] - self.area[0]) + x) as usize]
    }

    /// The level's area at half the size, each texel the mean of 2x2.
    fn half(&self, area: [i32; 4]) -> Level {
        Level::new(area, |x, y| {
            let t = [
                self.at(2 * x, 2 * y),
                self.at(2 * x + 1, 2 * y),
                self.at(2 * x, 2 * y + 1),
                self.at(2 * x + 1, 2 * y + 1),
            ];
            std::array::from_fn(|c| {
                let sum: u32 = t.iter().map(|p| u32::from(p[c])).sum();
                unit(sum as f32 / (4.0 * 255.0)) as u8
            })
        })
    }

    /// The level blurred along `dir` with normalised `weights` (centre
    /// first, the same each side).
    fn gauss(&self, dir: (i32, i32), weights: &[f32]) -> Level {
        Level::new(self.area, |x, y| {
            let mut acc = [0.0f32; 4];
            let centre = self.at(x, y);
            for (c, v) in acc.iter_mut().enumerate() {
                *v = f32::from(centre[c]) * weights[0];
            }
            for (t, w) in weights.iter().enumerate().skip(1) {
                let t = t as i32;
                let a = self.at(x + t * dir.0, y + t * dir.1);
                let b = self.at(x - t * dir.0, y - t * dir.1);
                for (c, v) in acc.iter_mut().enumerate() {
                    *v += (f32::from(a[c]) + f32::from(b[c])) * w;
                }
            }
            acc.map(|v| unit(v / 255.0) as u8)
        })
    }

    /// Linearly filtered at texel coordinate `(u, v)` (centres at whole
    /// numbers), as `blur.wgsl`'s `bilinear`.
    fn bilinear(&self, u: f32, v: f32) -> [f32; 4] {
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let t = [
            self.at(x0, y0),
            self.at(x0 + 1, y0),
            self.at(x0, y0 + 1),
            self.at(x0 + 1, y0 + 1),
        ];
        std::array::from_fn(|c| {
            let ch = |i: usize| f32::from(t[i][c]) / 255.0;
            let top = ch(0) + (ch(1) - ch(0)) * fx;
            let bot = ch(2) + (ch(3) - ch(2)) * fx;
            top + (bot - top) * fy
        })
    }
}

/// The Gaussian's weights from the centre out, normalised over both sides.
pub(crate) fn gauss_weights(blur: &Blur) -> Vec<f32> {
    let s = blur.residual_sigma();
    let taps = blur.taps().max(0);
    if s <= 0.0 || taps == 0 {
        return vec![1.0];
    }
    let w: Vec<f32> = (0..=taps)
        .map(|t| (-((t * t) as f32) / (2.0 * s * s)).exp())
        .collect();
    let total = w[0] + 2.0 * w[1..].iter().sum::<f32>();
    w.iter().map(|v| v / total).collect()
}

/// Draws `blur` into `pixels` over `bounds`, from what they show now: the
/// frame around it halved down to the blur's factor, blurred across then
/// down, and laid back under a rounded mask. `blur.wgsl` does the same
/// on the GPU, pass for pass.
fn backdrop(pixels: &mut [u32], (w, h): (usize, usize), blur: &Blur, bounds: [i32; 4]) {
    let m = blur.margin().ceil() as i32;
    let src = intersect(
        [bounds[0] - m, bounds[1] - m, bounds[2] + m, bounds[3] + m],
        [0, 0, w as i32, h as i32],
    );
    if src[0] >= src[2] || src[1] >= src[3] {
        return;
    }
    let mut level = Level::new(src, |x, y| {
        pixels[y as usize * w + x as usize].to_le_bytes()
    });
    let f = blur.factor() as i32;
    let mut size = 1;
    while size < f {
        size *= 2;
        let down = |v: i32| v / size;
        let up = |v: i32| (v + size - 1) / size;
        level = level.half([down(src[0]), down(src[1]), up(src[2]), up(src[3])]);
    }
    let weights = gauss_weights(blur);
    let level = level.gauss((1, 0), &weights).gauss((0, 1), &weights);
    let [x, y, bw, bh] = blur.rect;
    let half = [bw * 0.5, bh * 0.5];
    let r = blur.radius.min(half[0]).min(half[1]).max(0.0);
    let k = 1.0 / f as f32;
    for py in bounds[1]..bounds[3] {
        let cy = py as f32 + 0.5;
        let row = &mut pixels[py as usize * w..(py as usize + 1) * w];
        for px in bounds[0]..bounds[2] {
            let cx = px as f32 + 0.5;
            let d = rounded_box([cx - x - half[0], cy - y - half[1]], half, r);
            let a = (0.5 - d).clamp(0.0, 1.0) * blur.alpha;
            if a <= 0.0 {
                continue;
            }
            let c = level.bilinear(cx * k - 0.5, cy * k - 0.5).map(|v| v * a);
            let dst = &mut row[px as usize];
            *dst = blend_f(*dst, c, false);
        }
    }
}

/// A shadow quad's shape, worked out once per primitive.
struct ShadowShape {
    half: [f32; 2],
    r: f32,
    sigma: f32,
    box_half: [f32; 2],
    box_r: f32,
}

impl ShadowShape {
    fn of(q: &Quad) -> ShadowShape {
        let half = [q.rect[2] * 0.5, q.rect[3] * 0.5];
        let box_half = [q.border_color[2] * 0.5, q.border_color[3] * 0.5];
        ShadowShape {
            half,
            r: q.params[0].min(half[0]).min(half[1]).max(0.0),
            sigma: q.params[2],
            box_half,
            box_r: q.params[1].min(box_half[0]).min(box_half[1]).max(0.0),
        }
    }

    /// The shader's `shadow_fs` at pixel centre `(gx, gy)`, premultiplied.
    fn pixel(&self, q: &Quad, gx: f32, gy: f32) -> [f32; 4] {
        let b = q.border_color;
        let bp = [gx - b[0] - self.box_half[0], gy - b[1] - self.box_half[1]];
        let inside = (0.5 - rounded_box(bp, self.box_half, self.box_r)).clamp(0.0, 1.0);
        if inside >= 1.0 {
            return [0.0; 4];
        }
        let p = [gx - q.rect[0] - self.half[0], gy - q.rect[1] - self.half[1]];
        let cover = if self.sigma < Quad::MIN_SIGMA {
            (0.5 - rounded_box(p, self.half, self.r)).clamp(0.0, 1.0)
        } else {
            shadow_coverage(p, self.half, self.r, self.sigma)
        };
        let a = q.color[3] * cover.clamp(0.0, 1.0) * (1.0 - inside);
        [q.color[0] * a, q.color[1] * a, q.color[2] * a, a]
    }
}

/// The shader's `erf2`, one lane.
fn erf(x: f32) -> f32 {
    let s = if x < 0.0 { -1.0 } else { 1.0 };
    let a = x.abs();
    let t = 1.0 + (0.278_393 + (0.230_389 + 0.078_108 * (a * a)) * a) * a;
    let t = t * t;
    s - s / (t * t)
}

/// The shader's `shadow_coverage`.
fn shadow_coverage(p: [f32; 2], half: [f32; 2], r: f32, sigma: f32) -> f32 {
    let lo = (-3.0 * sigma).clamp(p[1] - half[1], p[1] + half[1]);
    let hi = (3.0 * sigma).clamp(p[1] - half[1], p[1] + half[1]);
    let step = (hi - lo) * 0.25;
    let k = std::f32::consts::FRAC_1_SQRT_2 / sigma;
    let norm = 1.0 / ((2.0 * std::f32::consts::PI).sqrt() * sigma);
    let mut y = lo + step * 0.5;
    let mut sum = 0.0;
    for _ in 0..4 {
        let row = p[1] - y;
        let d = (half[1] - r - row.abs()).min(0.0);
        let w = half[0] - r + (r * r - d * d).max(0.0).sqrt();
        let e0 = 0.5 + 0.5 * erf((p[0] - w) * k);
        let e1 = 0.5 + 0.5 * erf((p[0] + w) * k);
        let g = (-(y * y) / (2.0 * sigma * sigma)).exp() * norm;
        sum += (e1 - e0) * g * step;
        y += step;
    }
    sum
}

/// A texel of a 1- or 4-byte atlas, linearly filtered at texel coordinate
/// `(u, v)` (texel centres at whole numbers), clamped to the atlas edge.
fn bilinear(tex: &[u8], size: usize, bpp: usize, u: f32, v: f32) -> [f32; 4] {
    let max = size as f32 - 1.0;
    let (u, v) = (u.clamp(0.0, max), v.clamp(0.0, max));
    let (x0, y0) = (u.floor() as usize, v.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(size - 1), (y0 + 1).min(size - 1));
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    let at = |x: usize, y: usize, c: usize| f32::from(tex[(y * size + x) * bpp + c]) / 255.0;
    let mut out = [0.0; 4];
    for (c, o) in out.iter_mut().enumerate().take(bpp) {
        let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
        let bot = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
        *o = top * (1.0 - fy) + bot * fy;
    }
    out
}

fn unit(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
}

/// A straight colour at alpha `a`, premultiplied and packed.
fn pack(c: [f32; 4], a: f32, bgra: bool) -> u32 {
    let (r, b) = if bgra { (c[2], c[0]) } else { (c[0], c[2]) };
    let a = a.clamp(0.0, 1.0);
    unit(r * a) | unit(c[1] * a) << 8 | unit(b * a) << 16 | unit(a) << 24
}

fn premul_texel(t: &[u8], a: u32, bgra: bool) -> u32 {
    let (r, b) = if bgra { (t[2], t[0]) } else { (t[0], t[2]) };
    div255(u32::from(r) * a)
        | div255(u32::from(t[1]) * a) << 8
        | div255(u32::from(b) * a) << 16
        | a << 24
}

/// `x / 255`, rounded, for `x` up to 255 × 255.
fn div255(x: u32) -> u32 {
    let x = x + 128;
    (x + (x >> 8)) >> 8
}

/// Every channel of a packed pixel times `k / 255`, two channels per
/// multiply.
fn scale(c: u32, k: u32) -> u32 {
    let rb = (c & 0x00ff_00ff) * k + 0x0080_0080;
    let ag = ((c >> 8) & 0x00ff_00ff) * k + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
    let ag = (ag + ((ag >> 8) & 0x00ff_00ff)) & 0xff00_ff00;
    rb | ag
}

/// Premultiplied `src` over `dst`.
fn over(dst: u32, src: u32) -> u32 {
    src + scale(dst, 255 - (src >> 24))
}

fn fill_span(row: &mut [u32], c: u32) {
    match c >> 24 {
        0 => {}
        255 => row.fill(c),
        _ => row.iter_mut().for_each(|d| *d = over(*d, c)),
    }
}

/// Premultiplied float `src` over `dst`, rounded as a unorm target is.
fn blend_f(dst: u32, src: [f32; 4], bgra: bool) -> u32 {
    if src[3] <= 0.0 {
        return dst;
    }
    let (r, b) = if bgra {
        (src[2], src[0])
    } else {
        (src[0], src[2])
    };
    let k = 1.0 - src[3];
    let ch = |s: f32, sh: u32| {
        let d = ((dst >> sh) & 0xff) as f32 / 255.0;
        unit(s + d * k) << sh
    };
    ch(r, 0) | ch(src[1], 8) | ch(b, 16) | ch(src[3], 24)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Rect;

    fn frame(scene: &Scene, atlases: &Atlases, size: (u32, u32)) -> CpuRaster {
        let mut r = CpuRaster::new(false);
        r.render(scene, atlases, Rgba::hex(0), size);
        r
    }

    fn px(r: &CpuRaster, x: usize, y: usize) -> [u8; 4] {
        r.pixels[y * r.size().0 as usize + x].to_le_bytes()
    }

    #[test]
    fn swar_matches_per_channel_math() {
        for &(c, k) in &[
            (0xff80_4020u32, 128u32),
            (0x01ff_00fe, 255),
            (0xffff_ffff, 1),
        ] {
            let want = c.to_le_bytes().map(|v| div255(u32::from(v) * k) as u8);
            assert_eq!(scale(c, k).to_le_bytes(), want);
        }
        for x in 0..=255 * 255 {
            assert_eq!(div255(x), (x as f32 / 255.0).round() as u32, "{x}");
        }
    }

    #[test]
    fn fills_square_quads_exactly() {
        let mut s = Scene::default();
        s.quad(
            Rect::new(2.0, 2.0, 4.0, 4.0),
            Rgba::hex(0xff0000),
            0.0,
            None,
        );
        let a = Atlases::cpu(4, 4);
        let r = frame(&s, &a, (8, 8));
        assert_eq!(px(&r, 2, 2), [255, 0, 0, 255]);
        assert_eq!(px(&r, 5, 5), [255, 0, 0, 255]);
        assert_eq!(px(&r, 6, 6), [0, 0, 0, 255]);
        assert_eq!(px(&r, 1, 2), [0, 0, 0, 255]);
    }

    #[test]
    fn rounded_corners_and_borders_follow_the_distance_field() {
        let mut s = Scene::default();
        let white = Rgba::hex(0xffffff);
        s.quad(
            Rect::new(0.0, 0.0, 20.0, 20.0),
            Rgba::hex(0x0000ff),
            6.0,
            Some((2.0, white)),
        );
        let a = Atlases::cpu(4, 4);
        let r = frame(&s, &a, (20, 20));
        assert_eq!(px(&r, 0, 0), [0, 0, 0, 255], "corner is outside the radius");
        assert_eq!(px(&r, 10, 0), [255, 255, 255, 255], "top edge is border");
        assert_eq!(px(&r, 10, 10), [0, 0, 255, 255], "centre is fill");
        let edge = px(&r, 1, 2);
        assert!(edge[0] > 0 && edge[0] < 255, "corner arc is antialiased");
    }

    #[test]
    fn clips_cut_at_pixel_centres() {
        let mut s = Scene::default();
        s.push_clip(Rect::new(0.0, 0.0, 3.0, 8.0));
        s.quad(
            Rect::new(0.0, 0.0, 8.0, 8.0),
            Rgba::hex(0x00ff00),
            0.0,
            None,
        );
        s.pop_clip();
        let a = Atlases::cpu(4, 4);
        let r = frame(&s, &a, (8, 8));
        assert_eq!(px(&r, 2, 4), [0, 255, 0, 255]);
        assert_eq!(px(&r, 3, 4), [0, 0, 0, 255]);
    }

    #[test]
    fn masks_tint_and_blend() {
        let mut a = Atlases::cpu(8, 4);
        let uv = a.insert_mask(2, 1, &[255, 128]).unwrap();
        let mut s = Scene::default();
        s.sprite(
            Rect::new(1.0, 1.0, 2.0, 1.0),
            uv,
            Rgba::hex(0xffffff),
            false,
        );
        let r = frame(&s, &a, (4, 4));
        assert_eq!(px(&r, 1, 1), [255, 255, 255, 255]);
        assert_eq!(px(&r, 2, 1), [128, 128, 128, 255]);
    }

    #[test]
    fn only_changed_tiles_are_redrawn() {
        let a = Atlases::cpu(4, 4);
        let mut r = CpuRaster::new(false);
        let mut s = Scene::default();
        s.quad(Rect::new(0.0, 0.0, 10.0, 10.0), Rgba::hex(0xff), 0.0, None);
        s.quad(
            Rect::new(100.0, 70.0, 10.0, 10.0),
            Rgba::hex(0xff),
            0.0,
            None,
        );
        r.render(&s, &a, Rgba::hex(0), (200, 200));
        assert_eq!(r.damage(), &[(0, 200)], "the first frame is whole");
        r.render(&s, &a, Rgba::hex(0), (200, 200));
        assert!(r.damage().is_empty(), "nothing changed");
        s.layers[0].quads[1].color = Rgba::hex(0xff00).0;
        r.render(&s, &a, Rgba::hex(0), (200, 200));
        assert_eq!(r.damage(), &[(64, 128)], "one band of tiles");
        assert_eq!(px(&r, 105, 75), [0, 255, 0, 255]);
        assert_eq!(px(&r, 5, 5), [0, 0, 255, 255], "untouched tile kept");
    }

    #[test]
    fn copies_only_damaged_rows_as_xrgb() {
        let a = Atlases::cpu(4, 4);
        let mut r = CpuRaster::new(true);
        let mut s = Scene::default();
        s.quad(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rgba::hex(0x112233),
            0.0,
            None,
        );
        s.quad(
            Rect::new(0.0, 70.0, 10.0, 10.0),
            Rgba::hex(0x445566),
            0.0,
            None,
        );
        r.render(&s, &a, Rgba::hex(0), (100, 100));
        let mut dst = vec![7u32; 100 * 100];
        assert_eq!(r.copy_xrgb(&mut dst, true), Some(&[(0, 100)][..]));
        assert_eq!(dst[0], 0x112233, "alpha cleared");
        s.layers[0].quads[1].color = Rgba::hex(0x778899).0;
        r.render(&s, &a, Rgba::hex(0), (100, 100));
        dst[0] = 7;
        assert_eq!(r.copy_xrgb(&mut dst, false), Some(&[(64, 100)][..]));
        assert_eq!(dst[0], 7, "undamaged rows left alone");
        assert_eq!(dst[75 * 100 + 5], 0x778899);
        assert_eq!(r.copy_xrgb(&mut dst[1..], true), None, "size mismatch");
        assert_eq!(CpuRaster::new(false).copy_xrgb(&mut [], true), None);
    }

    #[test]
    fn threads_draw_what_one_thread_draws() {
        let a = Atlases::cpu(4, 4);
        let mut s = Scene::default();
        for i in 0..40 {
            let f = i as f32;
            s.quad(
                Rect::new(f * 13.3, f * 9.7, 90.0, 60.0),
                Rgba::hexa(0x336699 + i * 1000, 0.7),
                f % 9.0,
                Some((1.5, Rgba::hex(0xeeeeee))),
            );
        }
        let mut one = CpuRaster::new(false);
        one.pool = None;
        one.render(&s, &a, Rgba::hex(0x101012), (640, 480));
        let mut many = CpuRaster::new(false);
        many.pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().ok();
        assert!(many.pool.is_some());
        many.render(&s, &a, Rgba::hex(0x101012), (640, 480));
        assert!(one.pixels == many.pixels);
    }

    #[test]
    fn bgra_swaps_red_and_blue() {
        let mut s = Scene::default();
        s.quad(
            Rect::new(0.0, 0.0, 2.0, 2.0),
            Rgba::hex(0xff0000),
            0.0,
            None,
        );
        let a = Atlases::cpu(4, 4);
        let mut r = CpuRaster::new(true);
        r.render(&s, &a, Rgba::hex(0), (2, 2));
        assert_eq!(r.pixels[0].to_le_bytes(), [0, 0, 255, 255]);
    }
}
