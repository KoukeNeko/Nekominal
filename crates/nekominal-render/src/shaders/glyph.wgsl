// Atlas glyph quads. One instance = one glyph bitmap placed 1:1 on the pixel grid.
// kind 0: R8 coverage mask   (tinted with fg, grayscale AA)
// kind 1: RGBA LCD coverage  (grayscale fallback here; real LCD blend is fs_lcd)
// kind 2: RGBA premultiplied color bitmap (emoji)
//
// Text coverage correction follows Windows Terminal's atlas/dwrite.hlsl (MIT):
// enhanced contrast + gamma-aware alpha correction driven by the foreground
// brightness, so light-on-dark and dark-on-light text keep DirectWrite's weight.

struct Uniforms {
    viewport: vec2<f32>,
    contrast: vec2<f32>,
    gamma_ratios: vec4<f32>,
    flags: u32,
    text_boost: f32,   // extra grayscale enhanced contrast while the pane background is translucent
    pad1: u32,
    pad2: u32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var atlas_r8: texture_2d<f32>;
@group(1) @binding(1) var atlas_rgba: texture_2d<f32>;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) dst: vec2<i32>,    // top-left in pane pixels
    @location(1) size: vec2<u32>,   // bitmap size in texels == pixels
    @location(2) src: vec2<u32>,    // top-left in atlas texels
    @location(3) fg: vec4<f32>,     // straight RGBA, Unorm8x4
    @location(4) kind: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) tex: vec2<f32>,
    @location(1) @interpolate(flat) fg: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let corner = vec2<f32>(f32(in.vi & 1u), f32(in.vi >> 1u));
    let size = vec2<f32>(in.size);
    let p = vec2<f32>(in.dst) + size * corner;
    let ndc = vec2<f32>(p.x / u.viewport.x * 2.0 - 1.0, 1.0 - p.y / u.viewport.y * 2.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.tex = vec2<f32>(in.src) + size * corner;
    out.fg = in.fg;
    out.kind = in.kind;
    return out;
}

fn color_brightness(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.30, 0.59, 0.11));
}

fn light_on_dark_contrast(k: f32, c: vec3<f32>) -> f32 {
    let multiplier = saturate(4.0 * (0.75 - color_brightness(c)));
    return k * multiplier;
}

fn enhance_contrast(a: f32, k: f32) -> f32 {
    return a * (k + 1.0) / (a * k + 1.0);
}

fn enhance_contrast3(a: vec3<f32>, k: f32) -> vec3<f32> {
    return a * (k + 1.0) / (a * k + 1.0);
}

fn alpha_correction(a: f32, b: f32, g: vec4<f32>) -> f32 {
    let brightness_adjustment = g.x * b + g.y;
    let correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

fn alpha_correction3(a: vec3<f32>, b: vec3<f32>, g: vec4<f32>) -> vec3<f32> {
    let brightness_adjustment = g.x * b + g.y;
    let correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

fn correct_gray(a: f32, fg: vec3<f32>) -> f32 {
    if ((u.flags & 1u) == 0u) {
        return a;
    }
    let k = light_on_dark_contrast(u.contrast.x, fg) + u.text_boost;
    return alpha_correction(enhance_contrast(a, k), color_brightness(fg), u.gamma_ratios);
}

fn correct_lcd(a: vec3<f32>, fg: vec3<f32>) -> vec3<f32> {
    if ((u.flags & 1u) == 0u) {
        return a;
    }
    let k = light_on_dark_contrast(u.contrast.y, fg);
    return alpha_correction3(enhance_contrast3(a, k), fg, u.gamma_ratios);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Pixel centers interpolate to (src + i + 0.5): floor() is exact.
    let ic = vec2<i32>(floor(in.tex));
    if (in.kind == 2u) {
        return textureLoad(atlas_rgba, ic, 0) * in.fg.a;
    }
    var cov: f32;
    if (in.kind == 0u) {
        cov = textureLoad(atlas_r8, ic, 0).r;
    } else {
        let v = textureLoad(atlas_rgba, ic, 0).rgb;
        cov = (v.r + v.g + v.b) / 3.0;
    }
    let a = correct_gray(cov, in.fg.rgb) * in.fg.a;
    return vec4<f32>(in.fg.rgb * a, a);
}
