// Appended to glyph.wgsl (after `enable dual_source_blending;`) when the device
// supports DUAL_SOURCE_BLENDING. Per-channel coverage is the second blend source:
//   result.rgb = fg.rgb * cov.rgb + dst.rgb * (1 - cov.rgb)
// so no background color has to be known and neighbouring glyph overhang is safe.

struct LcdOut {
    @location(0) @blend_src(0) color: vec4<f32>,
    @location(0) @blend_src(1) coverage: vec4<f32>,
};

@fragment
fn fs_lcd(in: VsOut) -> LcdOut {
    let ic = vec2<i32>(floor(in.tex));
    let cov = textureLoad(atlas_rgba, ic, 0).rgb;
    let a3 = correct_lcd(cov, in.fg.rgb) * in.fg.a;
    var out: LcdOut;
    out.color = vec4<f32>(in.fg.rgb, 1.0);
    out.coverage = vec4<f32>(a3, 1.0);
    return out;
}
