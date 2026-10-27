// Pictures over the grid (Sixel, kitty graphics). One instance = one textured quad in pane pixels; the texture is
// bound per picture at t2 and holds premultiplied RGBA, sampled bilinearly so a picture scaled into cells stays smooth.

cbuffer Uniforms : register(b0) {
    float2 viewport;      // pane size in pixels
    float2 contrast;
    float4 gamma_ratios;
    uint flags;
    float text_boost;
    uint pad1;
    uint pad2;
};
Texture2D<float4> picture : register(t2);
SamplerState smooth : register(s0);

struct VsIn {
    uint vi : SV_VertexID;
    float4 dst : ATTR0;   // x, y, w, h in pane pixels
    float4 src : ATTR1;   // u, v, w, h in texture coordinates (0..1)
};

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

VsOut vs_main(VsIn i) {
    float2 corner = float2(i.vi & 1u, i.vi >> 1u);
    float2 p = i.dst.xy + i.dst.zw * corner;
    VsOut o;
    o.pos = float4(p.x / viewport.x * 2.0 - 1.0, 1.0 - p.y / viewport.y * 2.0, 0.0, 1.0);
    o.uv = i.src.xy + i.src.zw * corner;
    return o;
}

float4 fs_main(VsOut i) : SV_Target {
    return picture.Sample(smooth, i.uv);
}
