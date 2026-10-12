// Quads: rounded rectangles with an optional inside border, antialiased by
// a signed distance, or soft shadows (a Gaussian-blurred rounded rectangle
// cut out under its box). Sprites: atlas regions, tinted masks or color
// images; glyph masks take a gamma and contrast correction. All drop
// fragments whose pixel centre falls outside their clip. The CPU renderer
// (cpu.rs) reproduces this file's arithmetic; change both together.

struct Globals {
    viewport: vec2<f32>,
    mask_size: vec2<f32>,
    color_size: vec2<f32>,
    pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var mask_tex: texture_2d<f32>;
@group(0) @binding(2) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

fn corner(vi: u32) -> vec2<f32> {
    return vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
}

fn to_clip(p: vec2<f32>) -> vec4<f32> {
    let n = p / g.viewport * 2.0 - 1.0;
    return vec4<f32>(n.x, -n.y, 0.0, 1.0);
}

fn clipped(p: vec4<f32>, clip: vec4<f32>) -> bool {
    return p.x < clip.x || p.y < clip.y || p.x >= clip.z || p.y >= clip.w;
}

struct QuadOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) params: vec4<f32>,
    @location(5) clip: vec4<f32>,
};

@vertex
fn quad_vs(
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) clip: vec4<f32>,
) -> QuadOut {
    let c = corner(vi);
    // Reach past the rect, so a fractional edge keeps its outer half pixel
    // of antialiasing and a shadow its blur.
    var reach = 1.0;
    if (params.w > 0.5) {
        reach = 3.0 * params.z + 1.0;
    }
    let p = rect.xy - reach + c * (rect.zw + 2.0 * reach);
    var o: QuadOut;
    o.pos = to_clip(p);
    o.half_size = rect.zw * 0.5;
    o.local = p - rect.xy - o.half_size;
    o.color = color;
    o.border_color = border_color;
    o.params = params;
    o.clip = clip;
    return o;
}

fn rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// Abramowitz and Stegun 7.1.27: within 5e-4 of erf, and cheap.
fn erf2(x: vec2<f32>) -> vec2<f32> {
    let s = sign(x);
    let a = abs(x);
    var t = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    t = t * t;
    return s - s / (t * t);
}

// The coverage at `p` (from the centre) of a rounded box blurred with a
// Gaussian of standard deviation `sigma`. Exact along x (a difference of
// erfs over the row's extent at that height), integrated along y in four
// steps over three standard deviations.
fn shadow_coverage(p: vec2<f32>, half_size: vec2<f32>, r: f32, sigma: f32) -> f32 {
    let lo = clamp(-3.0 * sigma, p.y - half_size.y, p.y + half_size.y);
    let hi = clamp(3.0 * sigma, p.y - half_size.y, p.y + half_size.y);
    let step = (hi - lo) * 0.25;
    var y = lo + step * 0.5;
    var sum = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        let row = p.y - y;
        let d = min(half_size.y - r - abs(row), 0.0);
        let w = half_size.x - r + sqrt(max(0.0, r * r - d * d));
        let e = 0.5 + 0.5 * erf2((p.x + vec2<f32>(-w, w)) * (0.70710678 / sigma));
        let g = exp(-(y * y) / (2.0 * sigma * sigma)) / (2.5066283 * sigma);
        sum = sum + (e.y - e.x) * g * step;
        y = y + step;
    }
    return sum;
}

fn shadow_fs(i: QuadOut) -> vec4<f32> {
    let r = min(i.params.x, min(i.half_size.x, i.half_size.y));
    let sigma = i.params.z;
    var cover: f32;
    if (sigma < 0.5) {
        cover = clamp(0.5 - rounded_box(i.local, i.half_size, r), 0.0, 1.0);
    } else {
        cover = shadow_coverage(i.local, i.half_size, r, sigma);
    }
    // CSS draws an outer shadow only outside its box.
    let b = i.border_color;
    let bh = b.zw * 0.5;
    let bp = i.pos.xy - b.xy - bh;
    let br = min(i.params.y, min(bh.x, bh.y));
    let inside = clamp(0.5 - rounded_box(bp, bh, br), 0.0, 1.0);
    let a = i.color.a * clamp(cover, 0.0, 1.0) * (1.0 - inside);
    return vec4<f32>(i.color.rgb * a, a);
}

@fragment
fn quad_fs(i: QuadOut) -> @location(0) vec4<f32> {
    if (clipped(i.pos, i.clip)) {
        discard;
    }
    if (i.params.w > 0.5) {
        return shadow_fs(i);
    }
    let r = min(i.params.x, min(i.half_size.x, i.half_size.y));
    let d = rounded_box(i.local, i.half_size, r);
    let outer = clamp(0.5 - d, 0.0, 1.0);
    let bw = i.params.y;
    var c = i.color;
    if (bw > 0.0) {
        let inner = clamp(0.5 - (d + bw), 0.0, 1.0);
        // The fill runs under the border, as CSS paints a background.
        let fill_a = i.color.a;
        let border_a = i.border_color.a * (1.0 - inner);
        let a = border_a + fill_a * (1.0 - border_a);
        let rgb = (i.border_color.rgb * border_a + i.color.rgb * fill_a * (1.0 - border_a)) / max(a, 1e-5);
        c = vec4<f32>(rgb, a);
    }
    let a = c.a * outer;
    return vec4<f32>(c.rgb * a, a);
}

struct SpriteOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) kind: vec4<f32>,
    @location(3) clip: vec4<f32>,
};

@vertex
fn sprite_vs(
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: vec4<f32>,
    @location(4) clip: vec4<f32>,
) -> SpriteOut {
    let c = corner(vi);
    var o: SpriteOut;
    o.pos = to_clip(rect.xy + c * rect.zw);
    var size = g.mask_size;
    if (kind.x > 0.5) {
        size = g.color_size;
    }
    o.uv = (uv.xy + c * uv.zw) / size;
    o.color = color;
    o.kind = kind;
    o.clip = clip;
    return o;
}

// scene.rs's TextGamma: contrast thickens thin coverage, then gamma lifts
// it toward linear-light blending, light text and dark text each their way.
fn text_coverage(m: f32, lum: f32, contrast: f32, gamma: f32) -> f32 {
    let a = clamp(m, 0.0, 1.0);
    let c = a * (contrast + 1.0) / (a * contrast + 1.0);
    let g = 1.0 / max(gamma, 0.1);
    let light = pow(c, g);
    let dark = 1.0 - pow(1.0 - c, g);
    return dark + (light - dark) * lum;
}

@fragment
fn sprite_fs(i: SpriteOut) -> @location(0) vec4<f32> {
    if (clipped(i.pos, i.clip)) {
        discard;
    }
    if (i.kind.x > 0.5) {
        let t = textureSampleLevel(color_tex, samp, i.uv, 0.0);
        let a = t.a * i.color.a;
        return vec4<f32>(t.rgb * a, a);
    }
    var m = textureSampleLevel(mask_tex, samp, i.uv, 0.0).r;
    if (i.kind.z > 0.0) {
        m = text_coverage(m, dot(i.color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), i.kind.y, i.kind.z);
    }
    let a = m * i.color.a;
    return vec4<f32>(i.color.rgb * a, a);
}
