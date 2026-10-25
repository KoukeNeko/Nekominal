//! Glyph atlas memory for CJK-heavy output, and the clear-and-restart policy when the size cap is reached.

use nekominal_render::fonts::{CharClass, Style as FontStyle};
use nekominal_render::{PaneState, Renderer, RendererConfig, Theme};

use super::common::*;

const MIB: f64 = 1024.0 * 1024.0;

/// `count` ideographs from U+4E00 onward, after skipping `skip` usable ones, that the CJK font chain really has
/// a glyph for. A missing glyph draws nothing, so counting only usable ones keeps the memory figures honest.
fn cjk_chars(renderer: &mut Renderer, skip: usize, count: usize) -> Vec<char> {
    let class = CharClass { wide: true, emoji_vs: false };
    let mut out = Vec::with_capacity(count);
    let mut skipped = 0;
    for code in 0x4E00u32..0xA000 {
        let Some(ch) = char::from_u32(code) else { continue };
        let Some(choice) = renderer.fonts.choice_for(ch, FontStyle::REGULAR, class) else { continue };
        if renderer.fonts.face(choice.face).glyph(ch) == 0 {
            continue;
        }
        if skipped < skip {
            skipped += 1;
        } else {
            out.push(ch);
            if out.len() == count {
                break;
            }
        }
    }
    assert_eq!(out.len(), count, "the CJK fallback chain covers {count} ideographs after {skip}");
    out
}

/// A scene of wide cells, two columns per character, `cols` columns wide.
fn cjk_scene(chars: &[char], cols: usize) -> Scene {
    let rows = (chars.len() * 2).div_ceil(cols);
    let mut scene = Scene::new(cols, rows);
    for (i, ch) in chars.iter().enumerate() {
        scene.put(i * 2 / cols, (i * 2) % cols, &ch.to_string(), Style::new());
    }
    scene
}

fn new_renderer(cfg: RendererConfig, theme: &Theme) -> Renderer {
    let gpu = gpu();
    let mut renderer = Renderer::new(&gpu, cfg).expect("renderer");
    renderer.set_theme(theme);
    renderer
}

#[test]
fn five_thousand_distinct_cjk_glyphs_fit_without_a_restart_and_the_memory_is_bounded() {
    for (label, theme) in [
        ("LCD (opaque pane)", Theme::campbell()),
        ("gray (translucent pane)", theme_with_opacity(Theme::campbell(), 0.72)),
    ] {
        assert_eq!(RendererConfig::default().atlas_max, 2048, "the documented policy");
        let mut renderer = new_renderer(test_config(), &theme);
        let chars = cjk_chars(&mut renderer, 0, 5000);
        // Switching the theme from the renderer's initial LCD state clears the caches once; count from here.
        let clears_before = renderer.stats.atlas_clears;
        let mut pane = PaneState::new();
        let started = std::time::Instant::now();
        update(&mut renderer, &mut pane, &cjk_scene(&chars, 200));
        let seconds = started.elapsed().as_secs_f64();
        let (gray, rgba) = renderer.atlas_memory_bytes();
        println!(
            "{label}: 5000 CJK glyphs in {seconds:.2}s | {} | GPU {:.1} MiB, with the CPU shadow {:.1} MiB | {} rasterized, {} clears",
            renderer.atlas_info(),
            (gray + rgba) as f64 / MIB,
            2.0 * (gray + rgba) as f64 / MIB,
            renderer.stats.glyphs_rasterized,
            renderer.stats.atlas_clears,
        );
        assert_eq!(renderer.stats.atlas_clears, clears_before, "{label}: the default cap holds 5000 CJK glyphs");
        assert!(renderer.stats.glyphs_rasterized >= 5000);
        // What the documented policy promises: the texture that holds the glyphs stays at 2048x2048 or less.
        let (used, limit) = if renderer.lcd_active() { (rgba, 4.0 * 2048.0 * 2048.0) } else { (gray, 2048.0 * 2048.0) };
        assert!(used as f64 <= limit, "{label}: {:.1} MiB", used as f64 / MIB);
    }
}

#[test]
fn a_full_atlas_clears_and_restarts_without_changing_a_pixel() {
    let theme = theme_with_opacity(Theme::campbell(), 0.72); // gray glyphs: the R8 atlas
    // 256x256 R8 texels hold about 190 wide glyphs; three frames of 100 different glyphs overflow it.
    let tiny = RendererConfig { atlas_initial: 256, atlas_max: 256, ..test_config() };
    let mut renderer = new_renderer(tiny, &theme);
    let mut pane = PaneState::new();

    let mut last = None;
    for frame in 0..3 {
        let chars = cjk_chars(&mut renderer, frame * 100, 100);
        let scene = cjk_scene(&chars, 100);
        let stats = update(&mut renderer, &mut pane, &scene);
        assert!(stats.needs_draw());
        last = Some((scene, stats.atlas_restarts));
    }
    let (scene, restarts_in_last_frame) = last.expect("three frames");
    assert!(renderer.stats.atlas_clears >= 1, "the third frame no longer fits: the atlas restarted");
    assert!(restarts_in_last_frame >= 1);
    let restarted = draw_frame(&mut renderer, &pane, &theme, scene.cols, scene.rows);

    let fresh = render(&scene, &theme, test_config());
    assert_eq!(restarted.rgba, fresh.rgba, "the frame after a restart matches a renderer that never restarted");
    assert!(restarted.rgba.iter().any(|&b| b != 0), "and it is not empty");
}

#[test]
fn a_frame_with_more_glyphs_than_the_atlas_holds_raises_the_limit_instead_of_drawing_wrong_glyphs() {
    let theme = theme_with_opacity(Theme::campbell(), 0.72); // gray glyphs: the R8 atlas
    // 128x128 R8 texels hold about 45 wide glyphs; one frame of 100 never fits.
    let tiny = RendererConfig { atlas_initial: 128, atlas_max: 128, ..test_config() };
    let mut renderer = new_renderer(tiny, &theme);
    let mut pane = PaneState::new();
    let chars = cjk_chars(&mut renderer, 0, 100);
    let scene = cjk_scene(&chars, 100);
    update(&mut renderer, &mut pane, &scene);
    assert!(renderer.cfg.atlas_max > 128, "the limit was raised");
    let drawn = draw_frame(&mut renderer, &pane, &theme, scene.cols, scene.rows);
    let fresh = render(&scene, &theme, test_config());
    assert_eq!(drawn.rgba, fresh.rgba, "the frame matches a renderer whose atlas was large enough");
}
