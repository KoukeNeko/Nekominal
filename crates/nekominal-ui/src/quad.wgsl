// Rounded rectangles with an optional border, and soft shadows. Pixel space, origin top-left, premultiplied output.

struct Uniforms {
    viewport: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,    // x, y, w, h
    @location(1) rb: vec2<f32>,      // corner radius, border width
    @location(2) fill: vec4<f32>,    // straight RGBA
    @location(3) border: vec4<f32>,  // straight RGBA
    @location(4) extra: vec2<f32>,   // blur, kind (0 rectangle, 1 shadow)
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half_size: vec2<f32>,
    @location(2) @interpolate(flat) rb: vec2<f32>,
    @location(3) @interpolate(flat) fill: vec4<f32>,
    @location(4) @interpolate(flat) border: vec4<f32>,
    @location(5) @interpolate(flat) extra: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let corner = vec2<f32>(f32(in.vi & 1u), f32(in.vi >> 1u));
    let half_size = in.rect.zw * 0.5;
    let center = in.rect.xy + half_size;
    // One pixel around a rectangle for the anti-aliased edge, the blur radius around a shadow.
    let grow = select(1.0, in.extra.x + 1.0, in.extra.y > 0.5);
    let p = center + (corner * 2.0 - 1.0) * (half_size + vec2<f32>(grow));
    var out: VsOut;
    out.pos = vec4<f32>(p.x / u.viewport.x * 2.0 - 1.0, 1.0 - p.y / u.viewport.y * 2.0, 0.0, 1.0);
    out.local = p - center;
    out.half_size = half_size;
    out.rb = in.rb;
    out.fill = in.fill;
    out.border = in.border;
    out.extra = in.extra;
    return out;
}

fn sdf_rounded_rect(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - (half_size - vec2<f32>(r));
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let r = min(in.rb.x, min(in.half_size.x, in.half_size.y));
    let d = sdf_rounded_rect(in.local, in.half_size, r);
    if (in.extra.y > 0.5) {
        let a = (1.0 - smoothstep(-in.extra.x, in.extra.x, d)) * in.fill.a;
        return vec4<f32>(in.fill.rgb * a, a);
    }
    let outer = clamp(0.5 - d, 0.0, 1.0);
    let inner = clamp(0.5 - (d + in.rb.y), 0.0, 1.0);
    let fill_a = in.fill.a * select(outer, inner, in.rb.y > 0.0);
    let border_a = in.border.a * (outer - inner);
    let rgb = in.border.rgb * border_a + in.fill.rgb * fill_a * (1.0 - border_a);
    return vec4<f32>(rgb, border_a + fill_a * (1.0 - border_a));
}
