// Rounded rectangles with an optional border, and soft shadows. Pixel space, origin top-left, premultiplied output.

cbuffer Uniforms : register(b0) {
    float2 viewport;
    float2 pad;
};

struct VsIn {
    uint vi : SV_VertexID;
    float4 rect : ATTR0;    // x, y, w, h
    float2 rb : ATTR1;      // corner radius, border width
    float4 fill : ATTR2;    // straight RGBA
    float4 border : ATTR3;  // straight RGBA
    float2 extra : ATTR4;   // blur, kind (0 rectangle, 1 shadow)
};

struct VsOut {
    float4 pos : SV_Position;
    float2 local : TEXCOORD0;
    nointerpolation float2 half_size : TEXCOORD1;
    nointerpolation float2 rb : TEXCOORD2;
    nointerpolation float4 fill : COLOR0;
    nointerpolation float4 border : COLOR1;
    nointerpolation float2 extra : TEXCOORD3;
};

VsOut vs_main(VsIn i) {
    float2 corner = float2(i.vi & 1u, i.vi >> 1u);
    float2 half_size = i.rect.zw * 0.5;
    float2 center = i.rect.xy + half_size;
    // One pixel around a rectangle for the anti-aliased edge, the blur radius around a shadow.
    float grow = i.extra.y > 0.5 ? i.extra.x + 1.0 : 1.0;
    float2 p = center + (corner * 2.0 - 1.0) * (half_size + grow);
    VsOut o;
    o.pos = float4(p.x / viewport.x * 2.0 - 1.0, 1.0 - p.y / viewport.y * 2.0, 0.0, 1.0);
    o.local = p - center;
    o.half_size = half_size;
    o.rb = i.rb;
    o.fill = i.fill;
    o.border = i.border;
    o.extra = i.extra;
    return o;
}

float sdf_rounded_rect(float2 p, float2 half_size, float r) {
    float2 q = abs(p) - (half_size - r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float4 fs_main(VsOut i) : SV_Target {
    float r = min(i.rb.x, min(i.half_size.x, i.half_size.y));
    float d = sdf_rounded_rect(i.local, i.half_size, r);
    if (i.extra.y > 0.5) {
        float a = (1.0 - smoothstep(-i.extra.x, i.extra.x, d)) * i.fill.a;
        return float4(i.fill.rgb * a, a);
    }
    float outer = clamp(0.5 - d, 0.0, 1.0);
    float inner = clamp(0.5 - (d + i.rb.y), 0.0, 1.0);
    float fill_a = i.fill.a * (i.rb.y > 0.0 ? inner : outer);
    float border_a = i.border.a * (outer - inner);
    float3 rgb = i.border.rgb * border_a + i.fill.rgb * fill_a * (1.0 - border_a);
    return float4(rgb, border_a + fill_a * (1.0 - border_a));
}
