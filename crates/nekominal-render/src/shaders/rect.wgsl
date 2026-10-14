// Solid rectangles: cell backgrounds, decorations, cursor shapes, scrollbar.
// One instance = one rect in pane pixel space (origin top-left, y down).

struct Uniforms {
    viewport: vec2<f32>,      // pane size in pixels
    contrast: vec2<f32>,      // x: grayscale enhanced contrast, y: LCD enhanced contrast
    gamma_ratios: vec4<f32>,  // DirectWrite-derived gamma correction ratios
    flags: u32,               // bit 0: apply contrast + gamma correction to text coverage
    text_boost: f32,          // extra grayscale enhanced contrast while the pane background is translucent
    pad1: u32,
    pad2: u32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,   // x, y, w, h
    @location(1) color: vec4<f32>,  // straight RGBA, Unorm8x4
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    // Triangle strip corners: 0:(0,0) 1:(1,0) 2:(0,1) 3:(1,1)
    let corner = vec2<f32>(f32(in.vi & 1u), f32(in.vi >> 1u));
    let p = in.rect.xy + in.rect.zw * corner;
    let ndc = vec2<f32>(p.x / u.viewport.x * 2.0 - 1.0, 1.0 - p.y / u.viewport.y * 2.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = vec4<f32>(in.color.rgb * in.color.a, in.color.a); // premultiplied
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
