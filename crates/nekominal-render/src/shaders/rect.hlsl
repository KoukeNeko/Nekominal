// Solid rectangles: cell backgrounds, decorations, cursor shapes, scrollbar.
// One instance = one rect in pane pixel space (origin top-left, y down).

cbuffer Uniforms : register(b0) {
    float2 viewport;      // pane size in pixels
    float2 contrast;      // x: grayscale enhanced contrast, y: LCD enhanced contrast
    float4 gamma_ratios;  // DirectWrite-derived gamma correction ratios
    uint flags;           // bit 0: apply contrast + gamma correction to text coverage
    float text_boost;     // extra grayscale enhanced contrast while the pane background is translucent
    uint pad1;
    uint pad2;
};

struct VsIn {
    uint vi : SV_VertexID;
    float4 rect : ATTR0;   // x, y, w, h
    float4 color : ATTR1;  // straight RGBA, Unorm8x4
};

struct VsOut {
    float4 pos : SV_Position;
    nointerpolation float4 color : COLOR0;
};

VsOut vs_main(VsIn i) {
    // Triangle strip corners: 0:(0,0) 1:(1,0) 2:(0,1) 3:(1,1)
    float2 corner = float2(i.vi & 1u, i.vi >> 1u);
    float2 p = i.rect.xy + i.rect.zw * corner;
    VsOut o;
    o.pos = float4(p.x / viewport.x * 2.0 - 1.0, 1.0 - p.y / viewport.y * 2.0, 0.0, 1.0);
    o.color = float4(i.color.rgb * i.color.a, i.color.a); // premultiplied
    return o;
}

float4 fs_main(VsOut i) : SV_Target {
    return i.color;
}
