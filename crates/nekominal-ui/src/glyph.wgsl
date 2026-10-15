// Text and icon glyph bitmaps placed 1:1 on the pixel grid. kind 0: R8 coverage tinted with the colour,
// kind 2: premultiplied RGBA colour bitmap (emoji). Premultiplied output.

struct Uniforms {
    viewport: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var atlas_r8: texture_2d<f32>;
@group(1) @binding(1) var atlas_rgba: texture_2d<f32>;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) dst: vec2<i32>,
    @location(1) size: vec2<u32>,
    @location(2) src: vec2<u32>,
    @location(3) fg: vec4<f32>,
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
    var out: VsOut;
    out.pos = vec4<f32>(p.x / u.viewport.x * 2.0 - 1.0, 1.0 - p.y / u.viewport.y * 2.0, 0.0, 1.0);
    out.tex = vec2<f32>(in.src) + size * corner;
    out.fg = in.fg;
    out.kind = in.kind;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let ic = vec2<i32>(floor(in.tex));
    if (in.kind == 2u) {
        return textureLoad(atlas_rgba, ic, 0) * in.fg.a;
    }
    let a = textureLoad(atlas_r8, ic, 0).r * in.fg.a;
    return vec4<f32>(in.fg.rgb * a, a);
}
