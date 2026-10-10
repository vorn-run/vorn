// Quads: rounded rectangles with an optional inside border, antialiased by
// a signed distance. Sprites: atlas regions, tinted masks or color images.

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

struct QuadOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) params: vec4<f32>,
};

@vertex
fn quad_vs(
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    @location(3) params: vec4<f32>,
) -> QuadOut {
    let c = corner(vi);
    var o: QuadOut;
    o.pos = to_clip(rect.xy + c * rect.zw);
    o.half_size = rect.zw * 0.5;
    o.local = (c - 0.5) * rect.zw;
    o.color = color;
    o.border_color = border_color;
    o.params = params;
    return o;
}

fn rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn quad_fs(i: QuadOut) -> @location(0) vec4<f32> {
    let r = min(i.params.x, min(i.half_size.x, i.half_size.y));
    let d = rounded_box(i.local, i.half_size, r);
    let outer = clamp(0.5 - d, 0.0, 1.0);
    let bw = i.params.y;
    var c = i.color;
    if (bw > 0.0) {
        let inner = clamp(0.5 - (d + bw), 0.0, 1.0);
        let fill_a = i.color.a * inner;
        let border_a = i.border_color.a * (1.0 - inner);
        let a = fill_a + border_a * (1.0 - fill_a);
        let rgb = (i.color.rgb * fill_a + i.border_color.rgb * border_a * (1.0 - fill_a)) / max(a, 1e-5);
        c = vec4<f32>(rgb, a);
    }
    let a = c.a * outer;
    return vec4<f32>(c.rgb * a, a);
}

struct SpriteOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) kind: f32,
};

@vertex
fn sprite_vs(
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: vec4<f32>,
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
    o.kind = kind.x;
    return o;
}

@fragment
fn sprite_fs(i: SpriteOut) -> @location(0) vec4<f32> {
    if (i.kind > 0.5) {
        let t = textureSample(color_tex, samp, i.uv);
        let a = t.a * i.color.a;
        return vec4<f32>(t.rgb * a, a);
    }
    let m = textureSample(mask_tex, samp, i.uv).r;
    let a = m * i.color.a;
    return vec4<f32>(i.color.rgb * a, a);
}
