// One instanced quad per cell background, glyph, decoration or cursor.
struct U {
    viewport: vec2<f32>,
    atlas: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Inst {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: u32,
};

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> Out {
    let c = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    let p = i.rect.xy + c * i.rect.zw;
    var o: Out;
    o.pos = vec4<f32>(p.x / u.viewport.x * 2.0 - 1.0, 1.0 - p.y / u.viewport.y * 2.0, 0.0, 1.0);
    o.uv = (i.uv.xy + c * i.uv.zw) / u.atlas;
    o.color = i.color;
    o.kind = i.kind;
    return o;
}

// Kind 0: a filled rectangle; 1: a glyph's coverage tinted with the colour;
// 2: a colour glyph. Output is premultiplied.
@fragment
fn fs(o: Out) -> @location(0) vec4<f32> {
    let t = textureSampleLevel(tex, samp, o.uv, 0.0);
    if o.kind == 0u {
        return vec4<f32>(o.color.rgb * o.color.a, o.color.a);
    }
    if o.kind == 1u {
        let a = t.a * o.color.a;
        return vec4<f32>(o.color.rgb * a, a);
    }
    return t * o.color.a;
}
