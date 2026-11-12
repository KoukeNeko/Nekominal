// Pictures over the grid (Sixel, kitty graphics). One instance = one textured quad in pane pixels; the texture is
// bound per picture and holds premultiplied RGBA, sampled bilinearly so a picture scaled into cells stays smooth.
// The WGSL twin of image.hlsl.

struct Uniforms {
    viewport: vec2<f32>,      // pane size in pixels
    contrast: vec2<f32>,
    gamma_ratios: vec4<f32>,
    flags: u32,
    text_boost: f32,
    pad1: u32,
    pad2: u32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(2) var picture: texture_2d<f32>;
@group(1) @binding(3) var smooth_sampler: sampler;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) dst: vec4<f32>,   // x, y, w, h in pane pixels
    @location(1) src: vec4<f32>,   // u, v, w, h in texture coordinates (0..1)
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let corner = vec2<f32>(f32(in.vi & 1u), f32(in.vi >> 1u));
    let p = in.dst.xy + in.dst.zw * corner;
    var out: VsOut;
    out.pos = vec4<f32>(p.x / u.viewport.x * 2.0 - 1.0, 1.0 - p.y / u.viewport.y * 2.0, 0.0, 1.0);
    out.uv = in.src.xy + in.src.zw * corner;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(picture, smooth_sampler, in.uv);
}
