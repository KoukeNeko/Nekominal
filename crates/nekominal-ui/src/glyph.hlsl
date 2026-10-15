// Text and icon glyph bitmaps placed 1:1 on the pixel grid. kind 0: R8 coverage tinted with the colour,
// kind 2: premultiplied RGBA colour bitmap (emoji). Premultiplied output.

cbuffer Uniforms : register(b0) {
    float2 viewport;
    float2 pad;
};
Texture2D<float> atlas_r8 : register(t0);
Texture2D<float4> atlas_rgba : register(t1);

struct VsIn {
    uint vi : SV_VertexID;
    int2 dst : ATTR0;
    uint2 size : ATTR1;
    uint2 src : ATTR2;
    float4 fg : ATTR3;
    uint kind : ATTR4;
};

struct VsOut {
    float4 pos : SV_Position;
    float2 tex : TEXCOORD0;
    nointerpolation float4 fg : COLOR0;
    nointerpolation uint kind : TEXCOORD1;
};

VsOut vs_main(VsIn i) {
    float2 corner = float2(i.vi & 1u, i.vi >> 1u);
    float2 size = float2(i.size);
    float2 p = float2(i.dst) + size * corner;
    VsOut o;
    o.pos = float4(p.x / viewport.x * 2.0 - 1.0, 1.0 - p.y / viewport.y * 2.0, 0.0, 1.0);
    o.tex = float2(i.src) + size * corner;
    o.fg = i.fg;
    o.kind = i.kind;
    return o;
}

float4 fs_main(VsOut i) : SV_Target {
    int2 ic = int2(floor(i.tex));
    if (i.kind == 2u) {
        return atlas_rgba.Load(int3(ic, 0)) * i.fg.a;
    }
    float a = atlas_r8.Load(int3(ic, 0)) * i.fg.a;
    return float4(i.fg.rgb * a, a);
}
