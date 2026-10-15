// Atlas glyph quads. One instance = one glyph bitmap placed 1:1 on the pixel grid.
// kind 0: R8 coverage mask   (tinted with fg, grayscale AA)
// kind 1: RGBA LCD coverage  (grayscale fallback in fs_main; the real LCD blend is fs_lcd)
// kind 2: RGBA premultiplied color bitmap (emoji)
//
// Text coverage correction follows Windows Terminal's atlas/dwrite.hlsl (MIT):
// enhanced contrast + gamma-aware alpha correction driven by the foreground
// brightness, so light-on-dark and dark-on-light text keep DirectWrite's weight.

cbuffer Uniforms : register(b0) {
    float2 viewport;
    float2 contrast;
    float4 gamma_ratios;
    uint flags;
    float text_boost;   // extra grayscale enhanced contrast while the pane background is translucent
    uint pad1;
    uint pad2;
};
Texture2D<float> atlas_r8 : register(t0);
Texture2D<float4> atlas_rgba : register(t1);

struct VsIn {
    uint vi : SV_VertexID;
    int2 dst : ATTR0;    // top-left in pane pixels
    uint2 size : ATTR1;  // bitmap size in texels == pixels
    uint2 src : ATTR2;   // top-left in atlas texels
    float4 fg : ATTR3;   // straight RGBA, Unorm8x4
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

float color_brightness(float3 c) {
    return dot(c, float3(0.30, 0.59, 0.11));
}

float light_on_dark_contrast(float k, float3 c) {
    float multiplier = saturate(4.0 * (0.75 - color_brightness(c)));
    return k * multiplier;
}

float enhance_contrast(float a, float k) {
    return a * (k + 1.0) / (a * k + 1.0);
}

float3 enhance_contrast3(float3 a, float k) {
    return a * (k + 1.0) / (a * k + 1.0);
}

float alpha_correction(float a, float b, float4 g) {
    float brightness_adjustment = g.x * b + g.y;
    float correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

float3 alpha_correction3(float3 a, float3 b, float4 g) {
    float3 brightness_adjustment = g.x * b + g.y;
    float3 correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

float correct_gray(float a, float3 fg) {
    if ((flags & 1u) == 0u) {
        return a;
    }
    float k = light_on_dark_contrast(contrast.x, fg) + text_boost;
    return alpha_correction(enhance_contrast(a, k), color_brightness(fg), gamma_ratios);
}

float3 correct_lcd(float3 a, float3 fg) {
    if ((flags & 1u) == 0u) {
        return a;
    }
    float k = light_on_dark_contrast(contrast.y, fg);
    return alpha_correction3(enhance_contrast3(a, k), fg, gamma_ratios);
}

float4 fs_main(VsOut i) : SV_Target {
    // Pixel centers interpolate to (src + i + 0.5): floor() is exact.
    int2 ic = int2(floor(i.tex));
    if (i.kind == 2u) {
        return atlas_rgba.Load(int3(ic, 0)) * i.fg.a;
    }
    float cov;
    if (i.kind == 0u) {
        cov = atlas_r8.Load(int3(ic, 0));
    } else {
        float3 v = atlas_rgba.Load(int3(ic, 0)).rgb;
        cov = (v.r + v.g + v.b) / 3.0;
    }
    float a = correct_gray(cov, i.fg.rgb) * i.fg.a;
    return float4(i.fg.rgb * a, a);
}

// Dual-source output: per-channel coverage is the second blend source:
//   result.rgb = fg.rgb * cov.rgb + dst.rgb * (1 - cov.rgb)
// so no background color has to be known and neighbouring glyph overhang is safe.
struct LcdOut {
    float4 color : SV_Target0;
    float4 coverage : SV_Target1;
};

LcdOut fs_lcd(VsOut i) {
    int2 ic = int2(floor(i.tex));
    float3 cov = atlas_rgba.Load(int3(ic, 0)).rgb;
    float3 a3 = correct_lcd(cov, i.fg.rgb) * i.fg.a;
    LcdOut o;
    o.color = float4(i.fg.rgb, 1.0);
    o.coverage = float4(a3, 1.0);
    return o;
}
