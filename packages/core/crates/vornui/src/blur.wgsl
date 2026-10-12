// Backdrop blur: the frame under a blur is halved (2x2 means) down to the
// blur's factor, blurred across then down with a Gaussian, and laid back
// under a rounded mask. Every pass loads texels exactly (no sampler), so
// the CPU renderer (cpu.rs, `backdrop`) can match it pass for pass; change
// both together. Each pass draws one triangle over the target, scissored
// to the region it writes.

struct Pass {
    // Composite: the blur's rect (x, y, w, h) and clip (x0, y0, x1, y1).
    rect: vec4<f32>,
    clip: vec4<f32>,
    // Corner radius, alpha, 1 / factor, taps.
    shape: vec4<f32>,
    // Gaussian direction (x, y), then the source's size in texels.
    dir_size: vec4<f32>,
    // Gaussian weights from the centre out; up to 16 taps.
    weights: array<vec4<f32>, 4>,
};

@group(0) @binding(0) var<uniform> u: Pass;
@group(0) @binding(1) var src: texture_2d<f32>;

@vertex
fn blur_vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn load(p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(u.dir_size.zw);
    return textureLoad(src, clamp(p, vec2<i32>(0), size - 1), 0);
}

fn weight(t: i32) -> f32 {
    return u.weights[t / 4][t % 4];
}

@fragment
fn down_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy) * 2;
    return (load(p) + load(p + vec2<i32>(1, 0)) + load(p + vec2<i32>(0, 1)) + load(p + vec2<i32>(1, 1))) * 0.25;
}

@fragment
fn gauss_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let d = vec2<i32>(u.dir_size.xy);
    var acc = load(p) * weight(0);
    let taps = i32(u.shape.w);
    for (var t = 1; t <= taps; t = t + 1) {
        acc = acc + (load(p + d * t) + load(p - d * t)) * weight(t);
    }
    return acc;
}

fn rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn composite_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = u.clip;
    if (pos.x < c.x || pos.y < c.y || pos.x >= c.z || pos.y >= c.w) {
        discard;
    }
    let half_size = u.rect.zw * 0.5;
    let r = min(u.shape.x, min(half_size.x, half_size.y));
    let d = rounded_box(pos.xy - u.rect.xy - half_size, half_size, r);
    let a = clamp(0.5 - d, 0.0, 1.0) * u.shape.y;
    // Bilinear by hand, so it matches the CPU texel for texel.
    let t = pos.xy * u.shape.z - 0.5;
    let t0 = floor(t);
    let f = t - t0;
    let i = vec2<i32>(t0);
    let top = mix(load(i), load(i + vec2<i32>(1, 0)), f.x);
    let bot = mix(load(i + vec2<i32>(0, 1)), load(i + vec2<i32>(1, 1)), f.x);
    return mix(top, bot, f.y) * a;
}
