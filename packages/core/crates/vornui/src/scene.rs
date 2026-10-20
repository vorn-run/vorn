//! What a frame draws, independent of how: layers of rounded quads, soft
//! shadows, atlas sprites and backdrop blurs in physical pixels, each with
//! the clip it was recorded under. The GPU and CPU renderers both draw
//! exactly this, so a scene is the contract between layout and pixels.

use bytemuck::{Pod, Zeroable};

/// A color with straight alpha, components 0..=1, in sRGB: blending happens
/// in sRGB space, as a browser composites CSS colors.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rgba(pub [f32; 4]);

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba([0.0; 4]);
    pub const BLACK: Rgba = Rgba([0.0, 0.0, 0.0, 1.0]);

    /// `0xRRGGBB`, opaque.
    pub const fn hex(c: u32) -> Rgba {
        Rgba::hexa(c, 1.0)
    }

    /// `0xRRGGBB` at alpha `a`: Tailwind's `bg-black/50` is `hexa(0, 0.5)`.
    pub const fn hexa(c: u32, a: f32) -> Rgba {
        Rgba::rgba_const((c >> 16) as u8, (c >> 8) as u8, c as u8, a)
    }

    /// CSS `rgba(r, g, b, a)`.
    pub const fn rgba_const(r: u8, g: u8, b: u8, a: f32) -> Rgba {
        Rgba([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a])
    }

    /// Tailwind's `white/[a]`.
    pub const fn white(a: f32) -> Rgba {
        Rgba([1.0, 1.0, 1.0, a])
    }

    /// The same color with its alpha multiplied by `a`.
    pub fn alpha(self, a: f32) -> Rgba {
        Rgba([self.0[0], self.0[1], self.0[2], self.0[3] * a])
    }

    /// `self` at `t = 0`, `to` at `t = 1`, mixed premultiplied so a fade
    /// from transparent does not pass through black.
    pub fn mix(self, to: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let (a, b) = (self.0, to.0);
        let alpha = a[3] + (b[3] - a[3]) * t;
        if alpha <= 0.0 {
            return Rgba::TRANSPARENT;
        }
        let ch = |i: usize| (a[i] * a[3] + (b[i] * b[3] - a[i] * a[3]) * t) / alpha;
        Rgba([ch(0), ch(1), ch(2), alpha])
    }
}

/// A rectangle; physical pixels in a scene, logical pixels in layout.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Big enough to clip nothing a window can show.
    pub const EVERYTHING: Rect = Rect {
        x: -1.0e7,
        y: -1.0e7,
        w: 2.0e7,
        h: 2.0e7,
    };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The overlap of two rectangles; empty (zero size) when they miss.
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }

    /// Grown by `d` on every side (shrunk when negative).
    pub fn outset(&self, d: f32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }

    pub fn scaled(&self, k: f32) -> Rect {
        Rect::new(self.x * k, self.y * k, self.w * k, self.h * k)
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Each edge moved to the nearest whole pixel, so a box's edges and
    /// borders land on pixel boundaries instead of blending across two.
    pub fn snapped(&self) -> Rect {
        let (x, y) = (self.x.round(), self.y.round());
        Rect::new(x, y, self.right().round() - x, self.bottom().round() - y)
    }

    fn corners(&self) -> [f32; 4] {
        [self.x, self.y, self.right(), self.bottom()]
    }
}

/// A rounded rectangle with an optional inside border, or a soft shadow,
/// as the GPU takes it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Quad {
    /// x, y, w, h.
    pub rect: [f32; 4],
    pub color: [f32; 4],
    /// The border's colour; for a shadow, the box it is cut out under
    /// (x, y, w, h).
    pub border_color: [f32; 4],
    /// Corner radius, then border width (a shadow's box radius), the blur's
    /// standard deviation, and [`Quad::BOX`] or [`Quad::SHADOW`].
    pub params: [f32; 4],
    /// Visible region: x0, y0, x1, y1.
    pub clip: [f32; 4],
}

impl Quad {
    /// A filled, bordered rounded rectangle.
    pub const BOX: f32 = 0.0;
    /// A Gaussian-blurred rounded rectangle, outside its box only.
    pub const SHADOW: f32 = 1.0;
    /// Below this standard deviation a shadow is drawn with a hard edge.
    pub const MIN_SIGMA: f32 = 0.5;

    pub fn is_shadow(&self) -> bool {
        self.params[3] > 0.5
    }

    /// How far past its rect the quad may colour pixels: half a pixel of
    /// antialiasing, or a shadow's three standard deviations.
    pub fn reach(&self) -> f32 {
        if self.is_shadow() {
            3.0 * self.params[2] + 1.0
        } else {
            1.0
        }
    }
}

/// A CSS `box-shadow` (outer): `offset` and `blur` and `spread` in the
/// units of the box it is cast by; `blur` is CSS's blur radius, twice the
/// Gaussian's standard deviation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub offset: (f32, f32),
    pub blur: f32,
    pub spread: f32,
    pub color: Rgba,
}

impl Shadow {
    pub const fn new(x: f32, y: f32, blur: f32, spread: f32, color: Rgba) -> Shadow {
        Shadow {
            offset: (x, y),
            blur,
            spread,
            color,
        }
    }

    /// Every length times `k`: logical to physical pixels.
    pub fn scaled(&self, k: f32) -> Shadow {
        Shadow {
            offset: (self.offset.0 * k, self.offset.1 * k),
            blur: self.blur * k,
            spread: self.spread * k,
            color: self.color,
        }
    }
}

/// A CSS `backdrop-filter: blur()`: what earlier layers drew under `rect`,
/// blurred with a Gaussian of standard deviation `sigma` and masked to the
/// rounded rect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blur {
    /// x, y, w, h.
    pub rect: [f32; 4],
    pub sigma: f32,
    pub radius: f32,
    /// The opacity in force when it was recorded.
    pub alpha: f32,
    /// Visible region: x0, y0, x1, y1.
    pub clip: [f32; 4],
}

impl Blur {
    /// The largest standard deviation drawn; wider blurs are clamped to it.
    pub const MAX_SIGMA: f32 = 64.0;

    /// The power-of-two downsampling a blur runs at: the largest that
    /// leaves at least two texels of standard deviation, at least 2, at
    /// most 32. Each halving is a 2x2 box average, so a wide blur costs
    /// about what a narrow one does.
    pub fn factor(&self) -> u32 {
        let mut f = 2;
        while f < 32 && (f * 2) as f32 * 2.0 <= self.sigma {
            f *= 2;
        }
        f
    }

    /// The Gaussian's standard deviation left to apply at the downsampled
    /// size, after what the box averages and the bilinear upsample
    /// already spread, in downsampled texels.
    pub fn residual_sigma(&self) -> f32 {
        let f = self.factor() as f32;
        let spent = (f * f - 1.0) / 12.0 + f * f / 6.0;
        (self.sigma * self.sigma - spent).max(0.0).sqrt() / f
    }

    /// Taps each side of the centre in the Gaussian passes.
    pub fn taps(&self) -> i32 {
        (self.residual_sigma() * 3.0).ceil() as i32
    }

    /// The pixels the blur reads around what it shows: enough for the
    /// Gaussian and the downsampling chain.
    pub fn margin(&self) -> f32 {
        let f = self.factor() as f32;
        (self.taps() as f32 + 2.0) * f + 2.0
    }

    /// Where the blur shows: its rect within its clip.
    pub fn bounds(&self) -> Rect {
        let r = Rect::new(self.rect[0], self.rect[1], self.rect[2], self.rect[3]);
        let c = &self.clip;
        r.intersect(&Rect::new(c[0], c[1], c[2] - c[0], c[3] - c[1]))
    }
}

/// The text-rendering correction applied to a glyph mask's coverage, so
/// text drawn with sRGB blending has the weight it would with linear
/// blending and a platform's stem darkening.
///
/// `contrast` thickens thin coverage: a = a(1 + c) / (1 + ac). `gamma`
/// lifts coverage toward what linear-light blending gives: light text on a
/// dark field takes a^(1/γ), dark text on a light field 1 - (1 - a)^(1/γ),
/// mixed by the text colour's luminance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextGamma {
    pub gamma: f32,
    pub contrast: f32,
}

impl TextGamma {
    /// No correction: coverage blended as rasterized.
    pub const OFF: TextGamma = TextGamma {
        gamma: 1.0,
        contrast: 0.0,
    };

    /// The text weight today's app has on this platform: measured on macOS, estimated elsewhere.
    pub fn platform() -> TextGamma {
        if cfg!(target_os = "macos") {
            TextGamma {
                gamma: 1.4,
                contrast: 0.5,
            }
        } else if cfg!(windows) {
            TextGamma {
                gamma: 1.6,
                contrast: 0.5,
            }
        } else {
            TextGamma {
                gamma: 1.3,
                contrast: 0.2,
            }
        }
    }

    pub fn is_off(&self) -> bool {
        self.gamma == 1.0 && self.contrast == 0.0
    }

    /// The corrected coverage of `a` for text of colour `rgb`.
    pub fn apply(&self, a: f32, rgb: [f32; 3]) -> f32 {
        text_coverage(a, luminance(rgb), self.contrast, self.gamma)
    }
}

impl Default for TextGamma {
    fn default() -> TextGamma {
        TextGamma::platform()
    }
}

/// Rec. 709 luma of an sRGB colour, as `sprite_fs` weighs light and dark
/// text.
pub(crate) fn luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

/// `shader.wgsl`'s `text_coverage`: the [`TextGamma`] formula.
pub(crate) fn text_coverage(a: f32, lum: f32, contrast: f32, gamma: f32) -> f32 {
    let a = a.clamp(0.0, 1.0);
    let a = a * (contrast + 1.0) / (a * contrast + 1.0);
    let g = 1.0 / gamma.max(0.1);
    let light = a.powf(g);
    let dark = 1.0 - (1.0 - a).powf(g);
    dark + (light - dark) * lum
}

/// An atlas region drawn into a rectangle.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Sprite {
    /// x, y, w, h.
    pub rect: [f32; 4],
    /// Atlas texels: x, y, w, h.
    pub uv: [f32; 4],
    pub color: [f32; 4],
    /// [`Sprite::MASK`] or [`Sprite::COLOR`]; then, for a glyph mask, its
    /// [`TextGamma`]'s contrast and gamma (gamma 0: no correction), and for
    /// a colour image the radius its corners are rounded to (0: square);
    /// unused.
    pub kind: [f32; 4],
    /// Visible region: x0, y0, x1, y1.
    pub clip: [f32; 4],
}

impl Sprite {
    /// A coverage mask from the mask atlas, tinted with `color`.
    pub const MASK: f32 = 0.0;
    /// An RGBA image from the color atlas, its alpha times `color`'s.
    pub const COLOR: f32 = 1.0;

    pub fn is_color(&self) -> bool {
        self.kind[0] > 0.5
    }

    /// The radius a colour image's corners are rounded to; 0 for square.
    pub fn corner_radius(&self) -> f32 {
        if self.is_color() {
            self.kind[1].max(0.0)
        } else {
            0.0
        }
    }

    /// The glyph correction this sprite's mask takes, if any.
    pub fn text_gamma(&self) -> Option<TextGamma> {
        (!self.is_color() && self.kind[2] > 0.0).then_some(TextGamma {
            contrast: self.kind[1],
            gamma: self.kind[2],
        })
    }
}

/// One layer: its blurs, its quads, then its sprites, over everything
/// before it.
#[derive(Default, Debug, Clone)]
pub struct Layer {
    pub quads: Vec<Quad>,
    pub sprites: Vec<Sprite>,
    /// Backdrop blurs of what earlier layers drew, under this layer's quads.
    pub blurs: Vec<Blur>,
}

impl Layer {
    fn clear(&mut self) {
        self.quads.clear();
        self.sprites.clear();
        self.blurs.clear();
    }

    fn is_empty(&self) -> bool {
        self.quads.is_empty() && self.sprites.is_empty()
    }
}

/// A scale about a point, then a shift, applied to what is recorded while
/// it is pushed: how a menu grows in from 95 %.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform {
    scale: f32,
    origin: (f32, f32),
    offset: (f32, f32),
}

impl Transform {
    const IDENTITY: Transform = Transform {
        scale: 1.0,
        origin: (0.0, 0.0),
        offset: (0.0, 0.0),
    };

    fn apply(&self, r: Rect) -> Rect {
        if *self == Transform::IDENTITY {
            return r;
        }
        let (ox, oy) = self.origin;
        let k = self.scale;
        Rect::new(
            ox + (r.x - ox) * k + self.offset.0,
            oy + (r.y - oy) * k + self.offset.1,
            r.w * k,
            r.h * k,
        )
    }
}

/// What a frame draws: layers in order.
///
/// Clips, opacity and transforms are stacks applied as primitives are
/// recorded, so the renderers never see a stack, only per-primitive values.
#[derive(Debug)]
pub struct Scene {
    pub layers: Vec<Layer>,
    current: usize,
    clips: Vec<Rect>,
    opacity: Vec<f32>,
    transforms: Vec<Transform>,
}

impl Default for Scene {
    fn default() -> Scene {
        Scene {
            layers: vec![Layer::default()],
            current: 0,
            clips: Vec::new(),
            opacity: Vec::new(),
            transforms: Vec::new(),
        }
    }
}

impl Scene {
    /// Empties the scene, keeping its allocations.
    pub fn clear(&mut self) {
        self.layers.truncate(1);
        if let Some(l) = self.layers.first_mut() {
            l.clear();
        } else {
            self.layers.push(Layer::default());
        }
        self.current = 0;
        self.clips.clear();
        self.opacity.clear();
        self.transforms.clear();
    }

    /// Starts a layer drawn over every layer so far, and records into it.
    pub fn layer(&mut self) -> usize {
        self.layers.push(Layer::default());
        self.current = self.layers.len() - 1;
        self.current
    }

    /// The layer being recorded into.
    pub fn current_layer(&self) -> usize {
        self.current
    }

    /// Records into layer `i` (made if missing): how an app paints its own
    /// boxes under the menus that layout already drew above them.
    pub fn set_layer(&mut self, i: usize) {
        while self.layers.len() <= i {
            self.layers.push(Layer::default());
        }
        self.current = i;
    }

    fn top(&mut self) -> &mut Layer {
        if self.layers.len() <= self.current {
            self.set_layer(self.current);
        }
        &mut self.layers[self.current]
    }

    /// The clip in force: the intersection of every pushed clip.
    pub fn clip(&self) -> Rect {
        self.clips.last().copied().unwrap_or(Rect::EVERYTHING)
    }

    /// Clips what follows to `r` (within the current clip) until the
    /// matching [`Scene::pop_clip`].
    pub fn push_clip(&mut self, r: Rect) {
        let r = self.transform().apply(r);
        let c = self.clip().intersect(&r);
        self.clips.push(c);
    }

    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn alpha(&self) -> f32 {
        self.opacity.last().copied().unwrap_or(1.0)
    }

    /// Multiplies the alpha of what follows by `a` until [`Scene::pop_opacity`].
    pub fn push_opacity(&mut self, a: f32) {
        let a = self.alpha() * a.clamp(0.0, 1.0);
        self.opacity.push(a);
    }

    pub fn pop_opacity(&mut self) {
        self.opacity.pop();
    }

    fn transform(&self) -> Transform {
        self.transforms
            .last()
            .copied()
            .unwrap_or(Transform::IDENTITY)
    }

    /// Scales what follows by `scale` about `origin`, then shifts it by
    /// `offset`, until [`Scene::pop_transform`]. Does not nest: an inner
    /// transform replaces the outer one.
    pub fn push_transform(&mut self, scale: f32, origin: (f32, f32), offset: (f32, f32)) {
        self.transforms.push(Transform {
            scale,
            origin,
            offset,
        });
    }

    pub fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    /// A rounded rectangle filled with `fill`, with an inside border.
    pub fn quad(&mut self, r: Rect, fill: Rgba, radius: f32, border: Option<(f32, Rgba)>) {
        let t = self.transform();
        let r = t.apply(r);
        let clip = self.clip();
        // A quad's antialiased edge reaches half a pixel past its rect.
        if r.is_empty() || r.outset(1.0).intersect(&clip).is_empty() {
            return;
        }
        let a = self.alpha();
        let (bw, bc) = border.unwrap_or((0.0, fill));
        if fill.0[3] * a <= 0.0 && (bw <= 0.0 || bc.0[3] * a <= 0.0) {
            return;
        }
        self.top().quads.push(Quad {
            rect: [r.x, r.y, r.w, r.h],
            color: fill.alpha(a).0,
            border_color: bc.alpha(a).0,
            params: [radius * t.scale, bw * t.scale, 0.0, 0.0],
            clip: clip.corners(),
        });
    }

    /// The atlas texels `uv` drawn into `r`; tinted when `colored` is false.
    pub fn sprite(&mut self, r: Rect, uv: [f32; 4], color: Rgba, colored: bool) {
        let kind = if colored { Sprite::COLOR } else { Sprite::MASK };
        self.push_sprite(r, uv, color, [kind, 0.0, 0.0, 0.0]);
    }

    /// A glyph: the mask (or, for an emoji, colour) texels `uv` drawn into
    /// `r`, a mask's coverage corrected by `gamma`.
    pub fn glyph(&mut self, r: Rect, uv: [f32; 4], color: Rgba, colored: bool, gamma: TextGamma) {
        let kind = if colored {
            [Sprite::COLOR, 0.0, 0.0, 0.0]
        } else if gamma.is_off() {
            [Sprite::MASK, 0.0, 0.0, 0.0]
        } else {
            [Sprite::MASK, gamma.contrast, gamma.gamma, 0.0]
        };
        self.push_sprite(r, uv, color, kind);
    }

    /// A colour image, the texels `uv` drawn into `r` with its corners
    /// rounded to `radius`, as a picture in a rounded frame: the corners
    /// are cut in the shader, so a picture needs no mask of its own.
    pub fn rounded_image(&mut self, r: Rect, uv: [f32; 4], color: Rgba, radius: f32) {
        let k = self.transform().scale;
        self.push_sprite(r, uv, color, [Sprite::COLOR, radius.max(0.0) * k, 0.0, 0.0]);
    }

    fn push_sprite(&mut self, r: Rect, uv: [f32; 4], color: Rgba, kind: [f32; 4]) {
        let r = self.transform().apply(r);
        let clip = self.clip();
        if r.is_empty() || r.intersect(&clip).is_empty() {
            return;
        }
        let a = self.alpha();
        if color.0[3] * a <= 0.0 {
            return;
        }
        self.top().sprites.push(Sprite {
            rect: [r.x, r.y, r.w, r.h],
            uv,
            color: color.alpha(a).0,
            kind,
            clip: clip.corners(),
        });
    }

    /// `shadow` cast by the box `r` with corner radius `radius`, as CSS
    /// draws an outer `box-shadow`: offset, grown by its spread, blurred,
    /// and never under the box itself. Record it before the box.
    pub fn shadow(&mut self, r: Rect, radius: f32, shadow: &Shadow) {
        let t = self.transform();
        let k = t.scale;
        let boxr = t.apply(r);
        let s = shadow.scaled(k);
        let sr = boxr.translate(s.offset.0, s.offset.1).outset(s.spread);
        let clip = self.clip();
        let sigma = (s.blur * 0.5).max(0.0);
        let a = self.alpha() * s.color.0[3];
        let quad = Quad {
            rect: [sr.x, sr.y, sr.w, sr.h],
            color: s.color.alpha(self.alpha()).0,
            border_color: [boxr.x, boxr.y, boxr.w, boxr.h],
            params: [
                // CSS keeps a square box's shadow square whatever its spread.
                if radius > 0.0 {
                    (radius * k + s.spread).max(0.0)
                } else {
                    0.0
                },
                radius * k,
                sigma,
                Quad::SHADOW,
            ],
            clip: clip.corners(),
        };
        if sr.is_empty() || a <= 0.0 || sr.outset(quad.reach()).intersect(&clip).is_empty() {
            return;
        }
        self.top().quads.push(quad);
    }

    /// Blurs what earlier layers drew under `r` (CSS `backdrop-filter:
    /// blur(sigma)`), masked to corner radius `radius`. A blur draws first
    /// in its layer, so when this layer already holds something a new
    /// layer is started on top for it and what follows.
    pub fn backdrop_blur(&mut self, r: Rect, sigma: f32, radius: f32) {
        let t = self.transform();
        let r = t.apply(r);
        let clip = self.clip();
        let a = self.alpha();
        if r.is_empty() || sigma <= 0.0 || a <= 0.0 || r.intersect(&clip).is_empty() {
            return;
        }
        if !self.top().is_empty() {
            self.layer();
        }
        let blur = Blur {
            rect: [r.x, r.y, r.w, r.h],
            sigma: (sigma * t.scale).min(Blur::MAX_SIGMA),
            radius: radius * t.scale,
            alpha: a,
            clip: clip.corners(),
        };
        self.top().blurs.push(blur);
    }

    /// Quads and sprites over every layer.
    pub fn counts(&self) -> (usize, usize) {
        self.layers
            .iter()
            .fold((0, 0), |(q, s), l| (q + l.quads.len(), s + l.sprites.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_nest_and_cull() {
        let mut s = Scene::default();
        s.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        s.push_clip(Rect::new(50.0, 50.0, 100.0, 100.0));
        assert_eq!(s.clip(), Rect::new(50.0, 50.0, 50.0, 50.0));
        s.quad(Rect::new(0.0, 0.0, 10.0, 10.0), Rgba::hex(0xff), 0.0, None);
        assert_eq!(s.counts(), (0, 0), "outside the clip is never recorded");
        s.quad(
            Rect::new(60.0, 60.0, 10.0, 10.0),
            Rgba::hex(0xff),
            0.0,
            None,
        );
        assert_eq!(s.layers[0].quads[0].clip, [50.0, 50.0, 100.0, 100.0]);
        s.pop_clip();
        s.pop_clip();
        assert_eq!(s.clip(), Rect::EVERYTHING);
    }

    #[test]
    fn opacity_multiplies_and_transparent_is_skipped() {
        let mut s = Scene::default();
        s.push_opacity(0.5);
        s.push_opacity(0.5);
        s.quad(Rect::new(0.0, 0.0, 1.0, 1.0), Rgba::hex(0), 0.0, None);
        assert_eq!(s.layers[0].quads[0].color[3], 0.25);
        s.pop_opacity();
        s.pop_opacity();
        s.quad(Rect::new(0.0, 0.0, 1.0, 1.0), Rgba::TRANSPARENT, 0.0, None);
        assert_eq!(s.counts(), (1, 0));
    }

    #[test]
    fn transforms_scale_about_their_origin() {
        let mut s = Scene::default();
        s.push_transform(0.5, (10.0, 10.0), (0.0, -4.0));
        s.quad(Rect::new(10.0, 10.0, 20.0, 20.0), Rgba::hex(0), 4.0, None);
        let q = s.layers[0].quads[0];
        assert_eq!(q.rect, [10.0, 6.0, 10.0, 10.0]);
        assert_eq!(q.params[0], 2.0);
    }

    #[test]
    fn mix_fades_through_the_color_not_black() {
        let white = Rgba::hex(0xffffff);
        let m = Rgba::TRANSPARENT.mix(white, 0.5);
        assert_eq!(m.0, [1.0, 1.0, 1.0, 0.5]);
        assert_eq!(white.mix(Rgba::hex(0), 1.0), Rgba::hex(0));
    }
}
