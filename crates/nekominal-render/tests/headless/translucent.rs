//! Rendering onto a translucent pane (`Theme::background_opacity < 1`, premultiplied alpha swap chain over a
//! blurred backdrop): premultiplied output, grayscale anti-aliasing, opaque highlights, and legible text.

use nekominal_render::{
    CellFlags, CursorShape, CursorView, Highlight, PaneState, Renderer, RendererConfig, RowSpan, Theme,
};

use super::common::*;

const BLOCK: &str = "\u{2588}";
const OPACITIES: [f32; 4] = [0.0, 0.5, 0.72, 1.0];

fn alpha_byte(opacity: f32) -> u8 {
    (opacity * 255.0).round() as u8
}

fn near(a: u8, b: u8, tol: u8) -> bool {
    a.abs_diff(b) <= tol
}

#[test]
fn default_background_is_the_theme_colour_premultiplied_by_the_opacity() {
    for dark in [true, false] {
        let base = if dark { Theme::campbell() } else { light_theme() };
        for opacity in OPACITIES {
            let theme = theme_with_opacity(base.clone(), opacity);
            let frame = render(&Scene::new(4, 2), &theme, test_config());
            let a = alpha_byte(opacity);
            let bg = theme.background;
            let expected = [bg.0, bg.1, bg.2].map(|c| (f32::from(c) * f32::from(a) / 255.0).round() as u8);
            let p = frame.at(1, 2, 3, 3);
            assert!(near(p[3], a, 1), "alpha {} for opacity {opacity}", p[3]);
            for c in 0..3 {
                assert!(near(p[c], expected[c], 1), "channel {c}: {} vs {} at opacity {opacity}", p[c], expected[c]);
            }
        }
    }
}

#[test]
fn a_fully_transparent_pane_outputs_nothing_where_there_is_no_ink() {
    let theme = theme_with_opacity(Theme::campbell(), 0.0);
    let frame = render(&Scene::new(4, 2), &theme, test_config());
    assert!(frame.rgba.iter().all(|&b| b == 0), "premultiplied transparent black everywhere");
}

#[test]
fn highlights_and_explicit_colours_stay_opaque_on_a_translucent_pane() {
    for opacity in [0.5, 0.72] {
        let theme = theme_with_opacity(Theme::campbell(), opacity);
        let mut scene = Scene::new(12, 2);
        scene.put(0, 0, " ", Style::new().bg(Color::Rgb(10, 120, 200))); // truecolor background
        scene.put(0, 1, " ", Style::new().bg(idx(2))); // palette background
        scene.put(0, 2, " ", Style::new().flag(CellFlags::INVERSE)); // reverse video default
        scene.put(0, 4, "  ", Style::new()); // selected below
        scene.put(0, 6, " ", Style::new()); // search match
        scene.put(0, 7, " ", Style::new()); // current search match
        scene.put(0, 8, " ", Style::new()); // block cursor
        scene.selection.push(RowSpan { row: 0, start: 4, end: 6 });
        scene.highlights.push(Highlight { span: RowSpan { row: 0, start: 6, end: 7 }, current: false });
        scene.highlights.push(Highlight { span: RowSpan { row: 0, start: 7, end: 8 }, current: true });
        scene.cursor =
            Some(CursorView { col: 8, row: 0, shape: CursorShape::Block, visible: true, focused: true, wide: false });
        let frame = render(&scene, &theme, test_config());

        let expect = |col: u32, c: Rgb, what: &str| {
            let p = frame.at(0, col, 2, 2);
            assert_eq!(p, [c.0, c.1, c.2, 255], "{what} at opacity {opacity}");
        };
        expect(0, Rgb(10, 120, 200), "truecolor background");
        expect(1, theme.ansi[2], "palette background");
        expect(2, theme.foreground, "reverse video");
        expect(4, theme.selection_bg, "selection");
        expect(5, theme.selection_bg, "selection (second cell)");
        expect(6, theme.search_match_bg, "search match");
        expect(7, theme.search_current_bg, "current search match");
        expect(8, theme.cursor, "block cursor");
        // and the untouched cell keeps the translucent default
        let p = frame.at(0, 10, 2, 2);
        assert!(near(p[3], alpha_byte(opacity), 1), "default background alpha {}", p[3]);
    }
}

#[test]
fn translucent_panes_disable_lcd_text_and_draw_grayscale_edges() {
    let mut scene = Scene::new(12, 1);
    scene.put(0, 0, "WMNwmn", Style::new().fg(Color::Rgb(255, 255, 255)));

    let opaque = Theme::campbell();
    let (frame_opaque, renderer) = render_with(&scene, &opaque, test_config());
    assert!(renderer.lcd_active(), "LCD text is on for an opaque pane (WARP reports dual-source blending)");
    let fringes = frame_opaque.pixels().iter().filter(|p| p[0] != p[1] || p[1] != p[2]).count();
    assert!(fringes > 0, "opaque LCD text has coloured edges");

    for opacity in [0.5, 0.72, 0.99] {
        let theme = theme_with_opacity(Theme::campbell(), opacity);
        let (frame, renderer) = render_with(&scene, &theme, test_config());
        assert!(!renderer.lcd_active(), "opacity {opacity}");
        let coloured = frame.pixels().iter().filter(|p| p[0] != p[1] || p[1] != p[2]).count();
        assert_eq!(coloured, 0, "neutral text on a neutral background stays neutral at opacity {opacity}");
    }
}

#[test]
fn switching_between_opaque_and_translucent_leaves_no_stale_glyphs_behind() {
    // LCD glyphs are cached per colour mode; a live opacity change must redraw with the right ones.
    let mut scene = Scene::new(14, 2);
    scene.put(0, 0, "Switch 你好 WM", Style::new().fg(Color::Rgb(250, 250, 250)));
    let opaque = Theme::campbell();
    let translucent = theme_with_opacity(Theme::campbell(), 0.6);

    let gpu = gpu();
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, FORMAT, test_config()).expect("renderer");
    let mut pane = PaneState::new();
    let mut draw = |theme: &Theme| {
        renderer.set_theme(theme);
        update(&mut renderer, &mut pane, &scene);
        draw_frame(&mut renderer, &pane, theme, scene.cols, scene.rows).rgba
    };
    let first_opaque = draw(&opaque);
    let first_translucent = draw(&translucent);
    let second_opaque = draw(&opaque);

    assert_eq!(first_opaque, render(&scene, &opaque, test_config()).rgba, "opaque LCD frame");
    assert_eq!(first_translucent, render(&scene, &translucent, test_config()).rgba, "gray frame after LCD");
    assert_eq!(second_opaque, first_opaque, "LCD again after gray");
    assert_ne!(first_opaque, first_translucent);
}

#[test]
fn output_is_valid_premultiplied_alpha_and_ink_pixels_are_opaque() {
    // Over a translucent pane the dark glyph body must be fully opaque: text is never see-through.
    for opacity in [0.0, 0.5, 0.72] {
        let theme = theme_with_opacity(Theme::campbell(), opacity);
        let mut scene = Scene::new(10, 1);
        scene.put(0, 0, "Hlm", Style::new().fg(Color::Rgb(240, 240, 240)).flag(CellFlags::BOLD));
        scene.put(0, 5, BLOCK, Style::new().fg(Color::Rgb(240, 200, 100)));
        let frame = render(&scene, &theme, test_config());
        for p in frame.pixels() {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "premultiplied colour never exceeds alpha: {p:?}");
        }
        let block = frame.at(0, 5, 4, 4);
        assert_eq!(block, [240, 200, 100, 255], "procedural glyphs are exact and opaque");
        let strongest = frame.pixels().iter().map(|p| p[3]).max();
        assert_eq!(strongest, Some(255));
        // Glyph body of the bold text: some pixel of the first three cells is (nearly) fully covered.
        let body_max = (0..3 * frame.cell.0)
            .flat_map(|x| (0..frame.cell.1).map(move |y| (x, y)))
            .map(|(x, y)| frame.px(x, y)[3])
            .max()
            .unwrap_or(0);
        assert!(body_max >= 250, "bold text reaches full coverage, got alpha {body_max} at opacity {opacity}");
    }
}

#[test]
fn text_over_a_backdrop_composites_to_the_expected_colours() {
    // What DWM does with the premultiplied frame: the pane colour shows through at `opacity`, ink is solid.
    let opacity = 0.5;
    let theme = theme_with_opacity(Theme::campbell(), opacity);
    let mut scene = Scene::new(4, 1);
    scene.put(0, 0, BLOCK, Style::new().fg(Color::Rgb(200, 100, 50)));
    let frame = render(&scene, &theme, test_config());
    let backdrop = Rgb(0x80, 0x90, 0xA0);
    let shown = frame.over(backdrop);
    let at = |col: u32| {
        let i = ((frame.cell.1 / 2 * frame.width + col * frame.cell.0 + 2) * 4) as usize;
        [shown[i], shown[i + 1], shown[i + 2]]
    };
    assert_eq!(at(0), [200, 100, 50], "ink hides the backdrop");
    let bg = theme.background;
    let a = f32::from(alpha_byte(opacity)) / 255.0;
    let expected = |pane: u8, back: u8| (f32::from(pane) * a + f32::from(back) * (1.0 - a)).round() as u8;
    let blended = at(2);
    assert!(near(blended[0], expected(bg.0, backdrop.0), 1), "{blended:?}");
    assert!(near(blended[1], expected(bg.1, backdrop.1), 1));
    assert!(near(blended[2], expected(bg.2, backdrop.2), 1));
}

// ---------------------------------------------------------------------------
// Legibility: the stroke weight of text must not drop when the pane gets translucent.
// ---------------------------------------------------------------------------

/// Sum of the corrected glyph coverage over the frame, from channel data that is neutral grey (so the green
/// channel stands for all). Opaque frames: `(px - bg) / (fg - bg)`. Translucent frames: the output alpha is
/// `c + a (1 - c)`, so `c = (A - a) / (1 - a)`.
fn ink_mass(frame: &Frame, theme: &Theme, fg: u8) -> f32 {
    let a = f32::from(alpha_byte(theme.background_opacity)) / 255.0;
    let bg = f32::from(theme.background.1);
    let mut mass = 0.0;
    for p in frame.pixels() {
        mass += if a >= 1.0 {
            (f32::from(p[1]) - bg) / (f32::from(fg) - bg)
        } else {
            (f32::from(p[3]) / 255.0 - a) / (1.0 - a)
        };
    }
    mass
}

fn text_scene(fg: u8) -> Scene {
    let mut scene = Scene::new(40, 3);
    let st = Style::new().fg(Color::Rgb(fg, fg, fg));
    scene.put(0, 0, "The quick brown fox jumps over 0123456789", st);
    scene.put(1, 0, "{[()]} <=> !=== -> == #include <stdio.h>", st);
    scene.put(2, 0, "繁體中文：終端機模擬器渲染測試。", st);
    scene
}

/// Ink mass at `opacity` with gray anti-aliasing in every case (`lcd: false` also for the opaque baseline).
fn mass_at(base: &Theme, fg: u8, opacity: f32, boost: f32) -> f32 {
    let theme = theme_with_opacity(base.clone(), opacity);
    let cfg = RendererConfig { lcd: false, translucent_text_boost: boost, ..test_config() };
    ink_mass(&render(&text_scene(fg), &theme, cfg), &theme, fg)
}

#[test]
fn without_the_boost_translucency_changes_only_alpha_not_stroke_weight() {
    for (base, fg) in [(Theme::campbell(), 0xCC), (light_theme(), 0x38)] {
        let opaque = mass_at(&base, fg, 1.0, 0.0);
        for opacity in [0.5, 0.72] {
            let translucent = mass_at(&base, fg, opacity, 0.0);
            let ratio = translucent / opaque;
            assert!((0.99..=1.01).contains(&ratio), "mass ratio {ratio} at opacity {opacity} (fg {fg:#x})");
        }
    }
}

/// Full-repaint CPU cost of `update_pane` on a 140x40 grid of coloured text, with the contrast floor off and
/// on (debug profile: only the ratio is meaningful):
/// `cargo test -p nekominal-render --test headless update_cost -- --ignored --nocapture`.
#[test]
#[ignore = "prints timings"]
fn update_cost() {
    let gpu = gpu();
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, FORMAT, test_config()).expect("renderer");
    let mut scene = Scene::new(140, 40);
    for row in 0..40 {
        for chunk in 0..9 {
            let fg = idx(1 + ((row + chunk) % 6) as u8);
            scene.put(row, chunk * 15, "colour text ", Style::new().fg(fg).bg(idx(((row + chunk) % 3) as u8)));
        }
    }
    let mut median = |min_contrast: f32| {
        let mut theme = theme_with_opacity(Theme::campbell(), 0.72);
        theme.min_contrast = min_contrast;
        renderer.set_theme(&theme);
        let mut pane = PaneState::new();
        update(&mut renderer, &mut pane, &scene); // warm the glyph caches
        let mut times: Vec<f64> = (0..30)
            .map(|_| {
                renderer.invalidate_all();
                let started = std::time::Instant::now();
                update(&mut renderer, &mut pane, &scene);
                started.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times[times.len() / 2]
    };
    let (off, on) = (median(1.0), median(4.5));
    println!("140x40 full rebuild (debug profile): min_contrast off {off:.0} us, on {on:.0} us ({:.2}x)", on / off);
}

/// Prints the table the default `translucent_text_boost` was chosen from:
/// `cargo test -p nekominal-render --test headless tuning_table -- --ignored --nocapture`.
#[test]
#[ignore = "prints the tuning table"]
fn tuning_table() {
    for (name, base, fg) in [("dark", Theme::campbell(), 0xCC), ("light", light_theme(), 0x38)] {
        let opaque = mass_at(&base, fg, 1.0, 0.0);
        for boost in [0.0, 0.4, 0.8, 1.2, 1.6, 2.4] {
            let row: Vec<String> = [0.9, 0.72, 0.5, 0.3]
                .iter()
                .map(|&o| format!("{:.3}", mass_at(&base, fg, o, boost) / opaque))
                .collect();
            println!("{name} boost {boost:.1}: ink mass vs opaque at opacity 0.9/0.72/0.5/0.3 = {}", row.join(" / "));
        }
    }
}

#[test]
fn the_boost_thickens_text_in_proportion_to_translucency_for_light_and_dark_text() {
    let boost = RendererConfig::default().translucent_text_boost;
    for (name, base, fg) in [("dark", Theme::campbell(), 0xCC), ("light", light_theme(), 0x38)] {
        let opaque = mass_at(&base, fg, 1.0, boost);
        let m72 = mass_at(&base, fg, 0.72, boost);
        let m50 = mass_at(&base, fg, 0.5, boost);
        println!("{name}: ink mass opaque {opaque:.1}, 0.72 -> {:.3}x, 0.5 -> {:.3}x", m72 / opaque, m50 / opaque);
        assert!(m72 > opaque * 1.06, "{name}: 0.72 should be heavier than opaque ({m72} vs {opaque})");
        assert!(m50 > m72 * 1.03, "{name}: 0.5 should be heavier than 0.72 ({m50} vs {m72})");
        assert!(m50 < opaque * 1.25, "{name}: but not blobby ({m50} vs {opaque})");
    }
}
