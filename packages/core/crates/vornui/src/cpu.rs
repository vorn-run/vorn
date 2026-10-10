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
//! - **Threads.** Dirty bands of tiles are shared among up to four scoped
//!   threads.
//!
//! It is its own small rasterizer rather than a general one (tiny-skia)
//! because the scene has two primitives and must match the shader's
//! distance field and sampling exactly; a path rasterizer would need both
//! re-expressed as paths and would still differ at the edges.

use std::sync::Mutex;

use crate::atlas::Atlases;
use crate::scene::{Quad, Rgba, Scene, Sprite};

/// Tile side, in pixels.
const TILE: usize = 64;
/// Below this many dirty pixels a frame is drawn on the calling thread.
const PARALLEL_MIN_PX: usize = 128 * 1024;
const MAX_THREADS: usize = 4;

/// A frame in memory: RGBA8 (or BGRA8) pixels, one `u32` each in memory
/// order, so the buffer uploads as-is to a texture of that format.
pub struct CpuRaster {
    size: (u32, u32),
    pixels: Vec<u32>,
    bgra: bool,
    threads: usize,
    /// Per tile: the hash of what it showed last frame.
    hashes: Vec<u64>,
    /// Per tile: the primitives touching it this frame, in draw order.
    bins: Vec<Vec<u32>>,
    dirty: Vec<bool>,
    damage: Vec<(u32, u32)>,
}

/// A primitive with the pixels it may touch: its coverage within its clip
/// and the frame, as `[x0, y0, x1, y1)`.
#[derive(Clone, Copy)]
enum Prim<'a> {
    Quad(&'a Quad, [i32; 4]),
    Sprite(&'a Sprite, [i32; 4]),
}

impl CpuRaster {
    /// A renderer whose pixels are BGRA in memory when `bgra`, as some
    /// window surfaces want; RGBA otherwise.
    pub fn new(bgra: bool) -> CpuRaster {
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(MAX_THREADS);
        CpuRaster {
            size: (0, 0),
            pixels: Vec::new(),
            bgra,
            threads,
            hashes: Vec::new(),
            bins: Vec::new(),
            dirty: Vec::new(),
            damage: Vec::new(),
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
        let frame = [0, 0, w as i32, h as i32];
        for l in &scene.layers {
            let quads = l
                .quads
                .iter()
                .map(|q| Prim::Quad(q, coverage(q.rect, q.clip)));
            let sprites = l
                .sprites
                .iter()
                .map(|s| Prim::Sprite(s, coverage(s.rect, s.clip)));
            for p in quads.chain(sprites) {
                let b = intersect(p.bounds(), frame);
                if b[0] >= b[2] || b[1] >= b[3] {
                    continue;
                }
                let ph = p.hash();
                let idx = prims.len() as u32;
                prims.push(p.with_bounds(b));
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
        self.damage.clear();
        let mut bands = Vec::new();
        let mut dirty_px = 0;
        for (b, rows) in self.pixels.chunks_mut(w * TILE).enumerate() {
            let tiles = &self.dirty[b * tiles_x..(b + 1) * tiles_x];
            let count = tiles.iter().filter(|d| **d).count();
            if count == 0 {
                continue;
            }
            dirty_px += count * TILE * (rows.len() / w);
            let (y0, y1) = ((b * TILE) as u32, (b * TILE + rows.len() / w) as u32);
            match self.damage.last_mut() {
                Some(last) if last.1 == y0 => last.1 = y1,
                _ => self.damage.push((y0, y1)),
            }
            bands.push((b, rows));
        }
        let job = Job {
            w,
            tiles_x,
            dirty: &self.dirty,
            bins: &self.bins,
            prims: &prims,
            mask: atlases.mask.pixels().unwrap_or(&[]),
            mask_size: atlases.mask.size as usize,
            color: atlases.color.pixels().unwrap_or(&[]),
            color_size: atlases.color.size as usize,
            clear: clear_px,
            bgra: self.bgra,
        };
        if self.threads <= 1 || dirty_px < PARALLEL_MIN_PX || bands.len() < 2 {
            for (b, rows) in bands {
                job.band(b, rows);
            }
            return;
        }
        // Bands are taken one at a time so a busy band does not hold up an
        // idle thread.
        let queue = Mutex::new(bands.into_iter());
        let next = || queue.lock().ok().and_then(|mut q| q.next());
        std::thread::scope(|s| {
            for _ in 1..self.threads {
                s.spawn(|| {
                    while let Some((b, rows)) = next() {
                        job.band(b, rows);
                    }
                });
            }
            while let Some((b, rows)) = next() {
                job.band(b, rows);
            }
        });
    }
}

impl Prim<'_> {
    fn bounds(&self) -> [i32; 4] {
        match self {
            Prim::Quad(_, b) | Prim::Sprite(_, b) => *b,
        }
    }

    fn with_bounds(self, b: [i32; 4]) -> Self {
        match self {
            Prim::Quad(q, _) => Prim::Quad(q, b),
            Prim::Sprite(s, _) => Prim::Sprite(s, b),
        }
    }

    fn hash(&self) -> u64 {
        let (tag, bytes): (u64, &[u8]) = match self {
            Prim::Quad(q, _) => (1, bytemuck::bytes_of(*q)),
            Prim::Sprite(s, _) => (2, bytemuck::bytes_of(*s)),
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

/// The pixels whose centres fall inside `rect` and inside `clip`, as the
/// GPU rasterizes a quad and the shader's clip test keeps its fragments.
fn coverage(rect: [f32; 4], clip: [f32; 4]) -> [i32; 4] {
    let x0 = rect[0].max(clip[0]);
    let y0 = rect[1].max(clip[1]);
    let x1 = (rect[0] + rect[2]).min(clip[2]);
    let y1 = (rect[1] + rect[3]).min(clip[3]);
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
    mask: &'a [u8],
    mask_size: usize,
    color: &'a [u8],
    color_size: usize,
    clear: u32,
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
            for y in area[1]..area[3] {
                band.row(y, area[0], area[2]).fill(self.clear);
            }
            for &i in &self.bins[t] {
                match self.prims[i as usize] {
                    Prim::Quad(q, bounds) => self.quad(&mut band, q, intersect(bounds, area)),
                    Prim::Sprite(s, bounds) => {
                        self.sprite(&mut band, s, intersect(bounds, area));
                    }
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

    fn sprite(&self, band: &mut Band<'_>, s: &Sprite, a: [i32; 4]) {
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
                    for (dst, &m) in row.iter_mut().zip(src) {
                        if m == 0 {
                            continue;
                        }
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
                    let al = t[0] * s.color[3];
                    [s.color[0] * al, s.color[1] * al, s.color[2] * al, al]
                };
                row[d] = blend_f(row[d], c, self.bgra);
            }
        }
    }
}

/// The shader's quad colour at `local` (from the quad's top-left),
/// premultiplied.
fn quad_pixel(q: &Quad, half: [f32; 2], r: f32, bw: f32, lx: f32, ly: f32) -> [f32; 4] {
    let p = [lx - half[0], ly - half[1]];
    let qx = p[0].abs() - half[0] + r;
    let qy = p[1].abs() - half[1] + r;
    let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
    let outer = (0.5 - d).clamp(0.0, 1.0);
    let mut c = q.color;
    if bw > 0.0 {
        let inner = (0.5 - (d + bw)).clamp(0.0, 1.0);
        let b = q.border_color;
        let fill_a = c[3] * inner;
        let border_a = b[3] * (1.0 - inner);
        let a = fill_a + border_a * (1.0 - fill_a);
        let k = a.max(1e-5);
        let ch = |i: usize| (c[i] * fill_a + b[i] * border_a * (1.0 - fill_a)) / k;
        c = [ch(0), ch(1), ch(2), a];
    }
    let a = c[3] * outer;
    [c[0] * a, c[1] * a, c[2] * a, a]
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
        one.threads = 1;
        one.render(&s, &a, Rgba::hex(0x101012), (640, 480));
        let mut many = CpuRaster::new(false);
        many.threads = 4;
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
