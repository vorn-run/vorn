//! Glyphs: CoreText rasterises each cluster once into an RGBA atlas that the
//! GPU samples. Plain text is stored as coverage (drawn white on black with
//! the platform's font smoothing, so stems weigh what native text weighs);
//! clusters that come out coloured (emoji) are stored as colour.

use std::collections::HashMap;
use std::ffi::c_void;

type Ref = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: Ref);
    fn CFStringCreateWithBytes(a: Ref, b: *const u8, n: isize, enc: u32, ext: u8) -> Ref;
    fn CFDictionaryCreate(a: Ref, k: *const Ref, v: *const Ref, n: isize, kc: Ref, vc: Ref) -> Ref;
    fn CFAttributedStringCreate(a: Ref, s: Ref, attrs: Ref) -> Ref;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    static kCFBooleanTrue: Ref;
}

#[link(name = "CoreText", kind = "framework")]
extern "C" {
    fn CTFontCreateWithName(name: Ref, size: f64, m: Ref) -> Ref;
    fn CTFontCreateCopyWithSymbolicTraits(f: Ref, size: f64, m: Ref, v: u32, mask: u32) -> Ref;
    fn CTFontGetAscent(f: Ref) -> f64;
    fn CTFontGetDescent(f: Ref) -> f64;
    fn CTFontGetLeading(f: Ref) -> f64;
    fn CTFontGetGlyphsForCharacters(f: Ref, c: *const u16, g: *mut u16, n: isize) -> bool;
    fn CTFontGetAdvancesForGlyphs(f: Ref, o: u32, g: *const u16, adv: *mut [f64; 2], n: isize) -> f64;
    fn CTLineCreateWithAttributedString(s: Ref) -> Ref;
    fn CTLineDraw(line: Ref, ctx: Ref);
    static kCTFontAttributeName: Ref;
    static kCTForegroundColorFromContextAttributeName: Ref;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateWithName(name: Ref) -> Ref;
    fn CGBitmapContextCreate(d: *mut c_void, w: usize, h: usize, bpc: usize, bpr: usize, cs: Ref, info: u32) -> Ref;
    fn CGContextSetRGBFillColor(c: Ref, r: f64, g: f64, b: f64, a: f64);
    fn CGContextFillRect(c: Ref, r: CGRect);
    fn CGContextSetTextPosition(c: Ref, x: f64, y: f64);
    fn CGContextSetAllowsFontSmoothing(c: Ref, b: bool);
    fn CGContextSetShouldSmoothFonts(c: Ref, b: bool);
    static kCGColorSpaceSRGB: Ref;
}

const UTF8: u32 = 0x0800_0100;
const PREMUL_RGBA: u32 = 1 | (4 << 12);
const RGBX: u32 = 5 | (4 << 12);

fn cfstr(s: &str) -> Ref {
    // SAFETY: the bytes are valid UTF-8 for their length.
    unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) }
}

/// The sRGB colour space, owned for the life of the process.
pub fn srgb() -> Ref {
    // SAFETY: a CoreGraphics constant.
    unsafe { CGColorSpaceCreateWithName(kCGColorSpaceSRGB) }
}

/// The terminal font at one pixel size: four variants and the cell.
pub struct Font {
    variants: [Ref; 4],
    /// Cell width and height and the baseline from the cell's top, in pixels.
    pub cell_w: f64,
    pub cell_h: f64,
    pub ascent: f64,
}

// SAFETY: CTFont is immutable and thread-safe.
unsafe impl Send for Font {}

impl Font {
    /// Menlo at `pt` points on a `scale`x display; the cell matches option
    /// A's (point metrics rounded up, then scaled).
    pub fn new(pt: f64, scale: f64) -> Font {
        let name = cfstr("Menlo");
        // SAFETY: plain CoreText calls on objects created here.
        unsafe {
            let base_pt = CTFontCreateWithName(name, pt, std::ptr::null());
            let mut g = 0u16;
            CTFontGetGlyphsForCharacters(base_pt, &(b'M' as u16), &mut g, 1);
            let mut adv = [0f64; 2];
            CTFontGetAdvancesForGlyphs(base_pt, 0, &g, &mut adv, 1);
            let asc = CTFontGetAscent(base_pt).ceil();
            let h = asc + CTFontGetDescent(base_pt).ceil() + CTFontGetLeading(base_pt).ceil();
            CFRelease(base_pt);
            let px = pt * scale;
            let base = CTFontCreateWithName(name, px, std::ptr::null());
            CFRelease(name);
            let v = |t: u32| {
                let f = CTFontCreateCopyWithSymbolicTraits(base, px, std::ptr::null(), t, t);
                if f.is_null() {
                    base
                } else {
                    f
                }
            };
            // Bold is trait 2, italic trait 1.
            Font {
                variants: [base, v(2), v(1), v(3)],
                cell_w: adv[0] * scale,
                cell_h: h * scale,
                ascent: asc * scale,
            }
        }
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        let mut seen: Vec<Ref> = Vec::new();
        for f in self.variants {
            if !seen.contains(&f) {
                // SAFETY: each distinct font was created by `new`.
                unsafe { CFRelease(f) };
                seen.push(f);
            }
        }
    }
}

/// Where a glyph sits in the atlas and how to place it: `(x, y)` is its
/// top-left relative to the cell's top-left, in pixels.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub uv: [f32; 4],
    pub off: [f32; 2],
    pub color: bool,
    pub empty: bool,
}

pub const ATLAS: u32 = 2048;

/// The atlas's CPU side: a shelf packer and the pixels not yet uploaded.
pub struct Atlas {
    map: HashMap<(String, u8, u8), Glyph>,
    /// Printable ASCII per variant, looked up without hashing a String.
    ascii: Vec<Option<Glyph>>,
    /// Bumped when the atlas starts over, so surfaces redraw.
    pub generation: u64,
    x: u32,
    y: u32,
    shelf: u32,
    /// Pixel rectangles to copy to the texture: (x, y, w, h, rgba).
    pub uploads: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    /// The atlas filled up and started over: draw the frame again.
    pub overflowed: bool,
    space: Ref,
}

// SAFETY: the colour space is immutable.
unsafe impl Send for Atlas {}

impl Atlas {
    pub fn new() -> Atlas {
        Atlas {
            map: HashMap::new(),
            ascii: vec![None; 4 * 128],
            generation: 0,
            x: 0,
            y: 0,
            shelf: 0,
            uploads: Vec::new(),
            overflowed: false,
            space: srgb(),
        }
    }

    /// Forgets every glyph (the font changed or the atlas is full).
    pub fn clear(&mut self) {
        self.map.clear();
        self.ascii.iter_mut().for_each(|g| *g = None);
        self.generation += 1;
        self.x = 0;
        self.y = 0;
        self.shelf = 0;
        self.uploads.clear();
    }

    /// The glyph of `text` in font variant `v` spanning `ncols` cells.
    pub fn get(&mut self, font: &Font, text: &str, v: u8, ncols: u8, ascii: bool) -> Glyph {
        let slot = (ascii && ncols == 1 && text.len() == 1).then(|| v as usize * 128 + text.as_bytes()[0] as usize);
        if let Some(g) = slot.and_then(|i| self.ascii[i]) {
            return g;
        }
        if slot.is_none() {
            if let Some(g) = self.map.get(&(text.to_owned(), v, ncols)) {
                return *g;
            }
        }
        let g = self.raster(font, text, v, ncols, ascii).unwrap_or_else(|| {
            // Full: start over; this frame's earlier glyphs are re-uploaded
            // as they are asked for again next frame.
            self.clear();
            self.overflowed = true;
            self.raster(font, text, v, ncols, ascii).expect("one glyph fits")
        });
        match slot {
            Some(i) => self.ascii[i] = Some(g),
            None => {
                self.map.insert((text.to_owned(), v, ncols), g);
            }
        }
        g
    }

    fn raster(&mut self, font: &Font, text: &str, v: u8, ncols: u8, ascii: bool) -> Option<Glyph> {
        let pad = (font.cell_w / 2.0).ceil() as u32;
        let w = (font.cell_w * ncols as f64).ceil() as u32 + 2 * pad;
        let h = font.cell_h.ceil() as u32 + 2 * pad;
        if self.x + w > ATLAS {
            self.x = 0;
            self.y += self.shelf;
            self.shelf = 0;
        }
        if self.y + h > ATLAS {
            return None;
        }
        let draw = |opaque: bool| -> Vec<u8> {
            let mut px = vec![0u8; (w * h * 4) as usize];
            // SAFETY: the context draws into `px`, which outlives it.
            unsafe {
                let ctx = CGBitmapContextCreate(
                    px.as_mut_ptr().cast(),
                    w as usize,
                    h as usize,
                    8,
                    (w * 4) as usize,
                    self.space,
                    if opaque { RGBX } else { PREMUL_RGBA },
                );
                if opaque {
                    CGContextSetRGBFillColor(ctx, 0.0, 0.0, 0.0, 1.0);
                    CGContextFillRect(ctx, CGRect { x: 0.0, y: 0.0, w: w as f64, h: h as f64 });
                }
                CGContextSetAllowsFontSmoothing(ctx, opaque);
                CGContextSetShouldSmoothFonts(ctx, opaque);
                CGContextSetRGBFillColor(ctx, 1.0, 1.0, 1.0, 1.0);
                let s = cfstr(text);
                let keys = [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName];
                let vals = [font.variants[v as usize & 3], kCFBooleanTrue];
                let attrs = CFDictionaryCreate(
                    std::ptr::null(),
                    keys.as_ptr(),
                    vals.as_ptr(),
                    2,
                    (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
                    (&raw const kCFTypeDictionaryValueCallBacks).cast(),
                );
                let a = CFAttributedStringCreate(std::ptr::null(), s, attrs);
                let line = CTLineCreateWithAttributedString(a);
                // Baseline: `pad` below the bitmap's top plus the ascent,
                // whole pixels so stems stay sharp.
                let base = h as f64 - pad as f64 - font.ascent.round();
                CGContextSetTextPosition(ctx, pad as f64, base);
                CTLineDraw(line, ctx);
                for r in [line, a, attrs, s, ctx] {
                    CFRelease(r);
                }
            }
            px
        };
        let mut color = false;
        let mut px = Vec::new();
        if !ascii {
            px = draw(false);
            color = px.chunks_exact(4).any(|p| {
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                (r - g).abs() > 24 || (g - b).abs() > 24
            });
        }
        if !color {
            px = draw(true);
            for p in px.chunks_exact_mut(4) {
                p[3] = p[1];
                p[0] = 255;
                p[1] = 255;
                p[2] = 255;
            }
        }
        let empty = px.chunks_exact(4).all(|p| p[3] == 0);
        let (x, y) = (self.x, self.y);
        if !empty {
            self.uploads.push((x, y, w, h, px));
        }
        self.x += w;
        self.shelf = self.shelf.max(h);
        Some(Glyph {
            uv: [x as f32, y as f32, w as f32, h as f32],
            off: [-(pad as f32), -(pad as f32)],
            color,
            empty,
        })
    }
}
