//! Colour resolution: the renderer's `Theme` is the only place that turns symbolic cell colours into RGB, so
//! INVERSE, FAINT, HIDDEN, bold-is-bright, OSC 4 overrides and the contrast floor each happen exactly once.
//!
//! Solid full-block characters (U+2588) are drawn procedurally at full coverage, so their pixels are the exact
//! foreground colour and these tests do not depend on a font.

use nekominal_render::{CellFlags, Theme, contrast};

use super::common::*;

const BLOCK: &str = "\u{2588}";

fn campbell_render(scene: &Scene, theme: &Theme) -> Frame {
    render(scene, theme, test_config())
}

fn rgb3(p: [u8; 4]) -> [u8; 3] {
    [p[0], p[1], p[2]]
}

fn ansi(theme: &Theme, i: usize) -> [u8; 3] {
    let c = theme.ansi[i];
    [c.0, c.1, c.2]
}

#[test]
fn reverse_video_is_applied_exactly_once() {
    let theme = Theme::campbell();
    let mut scene = Scene::new(6, 1);
    scene.put(0, 0, " ", Style::new().flag(CellFlags::INVERSE));
    let frame = campbell_render(&scene, &theme);
    let (fg, bg) = (theme.foreground, theme.background);
    assert_eq!(rgb3(frame.at(0, 0, 0, 0)), [fg.0, fg.1, fg.2], "reverse cell shows the foreground as background");
    assert_eq!(rgb3(frame.at(0, 2, 0, 0)), [bg.0, bg.1, bg.2], "plain cell shows the theme background");
}

#[test]
fn reverse_video_of_coloured_cells_swaps_once() {
    // red on blue, reversed: the cell background becomes red. A second inversion would show blue.
    let theme = Theme::campbell();
    let mut scene = Scene::new(6, 1);
    scene.put(0, 0, " ", Style::new().fg(idx(1)).bg(idx(4)).flag(CellFlags::INVERSE));
    let frame = campbell_render(&scene, &theme);
    assert_eq!(rgb3(frame.at(0, 0, 0, 0)), ansi(&theme, 1));
}

#[test]
fn reverse_video_text_swaps_the_ink_colour_too() {
    // The block is drawn with the (swapped) foreground = the original background.
    let theme = Theme::campbell();
    let mut scene = Scene::new(6, 1);
    scene.put(0, 0, BLOCK, Style::new().fg(idx(1)).bg(idx(4)).flag(CellFlags::INVERSE));
    let frame = campbell_render(&scene, &theme);
    assert_eq!(rgb3(frame.at(0, 0, 4, 4)), ansi(&theme, 4));
}

#[test]
fn truecolor_and_indexed_backgrounds_resolve_through_the_theme() {
    let theme = Theme::campbell();
    let mut scene = Scene::new(6, 1);
    scene.put(0, 0, " ", Style::new().bg(Color::Rgb(0, 100, 200)));
    scene.put(0, 1, " ", Style::new().bg(idx(1)));
    let frame = campbell_render(&scene, &theme);
    assert_eq!(rgb3(frame.at(0, 0, 0, 0)), [0, 100, 200]);
    assert_eq!(rgb3(frame.at(0, 1, 0, 0)), ansi(&theme, 1));
}

#[test]
fn hidden_text_draws_no_ink() {
    let theme = Theme::campbell();
    let mut scene = Scene::new(6, 1);
    scene.put(0, 0, BLOCK, Style::new().flag(CellFlags::HIDDEN));
    let frame = campbell_render(&scene, &theme);
    let bg = theme.background;
    for dy in 0..frame.cell.1 {
        for dx in 0..frame.cell.0 {
            assert_eq!(rgb3(frame.at(0, 0, dx, dy)), [bg.0, bg.1, bg.2], "({dx},{dy})");
        }
    }
}

#[test]
fn bold_is_bright_is_decided_by_the_theme_only() {
    let mut bright = Theme::campbell();
    bright.bold_is_bright = true;
    let mut plain = bright.clone();
    plain.bold_is_bright = false;

    let mut scene = Scene::new(8, 1);
    scene.put(0, 0, BLOCK, Style::new().fg(idx(1)).flag(CellFlags::BOLD)); // bold red
    scene.put(0, 1, BLOCK, Style::new().fg(idx(9)).flag(CellFlags::BOLD)); // already bright
    scene.put(0, 2, BLOCK, Style::new().fg(idx(1))); // not bold
    scene.put(0, 3, BLOCK, Style::new().fg(Color::Rgb(10, 20, 30)).flag(CellFlags::BOLD)); // truecolor
    scene.put(0, 4, BLOCK, Style::new().flag(CellFlags::BOLD)); // default foreground

    let on = campbell_render(&scene, &bright);
    assert_eq!(rgb3(on.at(0, 0, 4, 4)), ansi(&bright, 9), "bold red becomes bright red");
    assert_eq!(rgb3(on.at(0, 1, 4, 4)), ansi(&bright, 9));
    assert_eq!(rgb3(on.at(0, 2, 4, 4)), ansi(&bright, 1));
    assert_eq!(rgb3(on.at(0, 3, 4, 4)), [10, 20, 30]);
    let fg = bright.foreground;
    assert_eq!(rgb3(on.at(0, 4, 4, 4)), [fg.0, fg.1, fg.2]);

    let off = campbell_render(&scene, &plain);
    assert_eq!(rgb3(off.at(0, 0, 4, 4)), ansi(&plain, 1), "without the option bold keeps the colour");
}

#[test]
fn faint_blends_the_foreground_toward_the_cell_background() {
    let theme = Theme::campbell();
    let mut scene = Scene::new(4, 1);
    scene.put(0, 0, BLOCK, Style::new().fg(idx(2)).bg(Color::Rgb(0, 0, 100)).flag(CellFlags::FAINT));
    scene.put(0, 1, BLOCK, Style::new().fg(idx(2)).flag(CellFlags::FAINT));
    let frame = campbell_render(&scene, &theme);
    let mix = |a: Rgb, b: Rgb| {
        let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * theme.faint_amount).round() as u8;
        [m(a.0, b.0), m(a.1, b.1), m(a.2, b.2)]
    };
    assert_eq!(rgb3(frame.at(0, 0, 4, 4)), mix(theme.ansi[2], Rgb(0, 0, 100)), "toward the cell's own background");
    assert_eq!(rgb3(frame.at(0, 1, 4, 4)), mix(theme.ansi[2], theme.background), "toward the theme background");
}

#[test]
fn osc4_overrides_replace_palette_slots_above_and_below_16() {
    let base = Theme::campbell();
    let mut theme = base.clone();
    theme.set_indexed(200, rgb(0x123456));
    theme.set_indexed(16, rgb(0xABCDEF));
    theme.set_indexed(255, rgb(0x010203));
    theme.set_indexed(5, rgb(0x654321));

    let mut scene = Scene::new(8, 1);
    for (col, slot) in [200u8, 16, 255, 5, 201].into_iter().enumerate() {
        scene.put(0, col, " ", Style::new().bg(idx(slot)));
    }
    let frame = campbell_render(&scene, &theme);
    assert_eq!(rgb3(frame.at(0, 0, 0, 0)), [0x12, 0x34, 0x56]);
    assert_eq!(rgb3(frame.at(0, 1, 0, 0)), [0xAB, 0xCD, 0xEF]);
    assert_eq!(rgb3(frame.at(0, 2, 0, 0)), [0x01, 0x02, 0x03]);
    assert_eq!(rgb3(frame.at(0, 3, 0, 0)), [0x65, 0x43, 0x21]);
    let untouched = base.indexed(201);
    assert_eq!(rgb3(frame.at(0, 4, 0, 0)), [untouched.0, untouched.1, untouched.2], "other slots keep the xterm table");

    // The override also reaches foreground colours.
    let mut scene = Scene::new(2, 1);
    scene.put(0, 0, BLOCK, Style::new().fg(idx(200)));
    assert_eq!(rgb3(campbell_render(&scene, &theme).at(0, 0, 4, 4)), [0x12, 0x34, 0x56]);
}

#[test]
fn min_contrast_is_off_by_default_and_raises_text_to_the_floor_when_set() {
    // The underline of a letter is a solid rectangle in the text colour (after the contrast floor), which makes
    // the adjusted colour observable without depending on glyph shapes.
    let bg = Color::Rgb(0, 0, 0x40);
    let fg = idx(4); // Campbell blue: 2.4:1 against this background
    let mut scene = Scene::new(4, 1);
    scene.put(0, 0, "x", Style::new().fg(fg).bg(bg).underline(Underline::Single));
    // exemptions: a graphic (full block), faint text, and blank cells keep their colour
    scene.put(0, 1, BLOCK, Style::new().fg(fg).bg(bg));
    scene.put(0, 2, "x", Style::new().fg(fg).bg(bg).underline(Underline::Single).flag(CellFlags::FAINT));

    let mut theme = Theme::campbell();
    assert_eq!(theme.min_contrast, 1.0, "off by default");
    let (off, renderer) = render_with(&scene, &theme, test_config());
    let m = renderer.metrics();
    let original = theme.ansi[4];
    let underline_row = m.underline_top;
    assert_eq!(rgb3(off.at(0, 0, 1, underline_row)), [original.0, original.1, original.2]);

    theme.min_contrast = 4.5;
    let on = render(&scene, &theme, test_config());
    let background = Rgb(0, 0, 0x40);
    let expected = contrast::ensure_min_contrast(original, background, 4.5);
    assert_ne!(expected, original);
    assert_eq!(rgb3(on.at(0, 0, 1, underline_row)), [expected.0, expected.1, expected.2]);
    assert!(contrast::contrast_ratio(expected, background) >= 4.5);

    assert_eq!(rgb3(on.at(0, 1, 4, 4)), [original.0, original.1, original.2], "graphic glyphs keep exact colours");
    let faint = contrast::lerp(original, background, theme.faint_amount);
    assert_eq!(rgb3(on.at(0, 2, 1, underline_row)), [faint.0, faint.1, faint.2], "faint text stays faint");
}

#[test]
fn the_contrast_floor_judges_text_on_a_translucent_pane_against_the_visible_backdrop_mix() {
    // Light scheme at 50% over a dark backdrop: the pane really looks mid grey, not #FAFAFA.
    let mut theme = theme_with_opacity(light_theme(), 0.5);
    theme.min_contrast = 4.5;
    let text = idx(0); // One Half Light black: 11:1 on the light pane colour, 3.5:1 on the mid grey it blends to
    let mut scene = Scene::new(2, 1);
    scene.put(0, 0, "x", Style::new().fg(text).underline(Underline::Single));

    let underline = |theme: &Theme| {
        let (frame, renderer) = render_with(&scene, theme, test_config());
        rgb3(frame.at(0, 0, 1, renderer.metrics().underline_top))
    };
    let cyan = theme.ansi[0];
    let judged_on_pane = contrast::ensure_min_contrast(cyan, theme.background, 4.5);
    assert_eq!(judged_on_pane, cyan, "passes against the light pane colour alone");

    // Premultiplied output: the underline is an opaque rectangle, so these are its true colours.
    assert_eq!(underline(&theme), [cyan.0, cyan.1, cyan.2], "no hint: judged against the pane colour");

    theme.backdrop_hint = Some(rgb(0x202020));
    let shown = theme.effective_background();
    assert_eq!(shown, contrast::lerp(rgb(0x202020), theme.background, 0.5));
    let expected = contrast::ensure_min_contrast(cyan, shown, 4.5);
    assert_ne!(expected, cyan, "against the visible mix the text has to move");
    assert_eq!(underline(&theme), [expected.0, expected.1, expected.2]);
    assert!(contrast::contrast_ratio(expected, shown) >= 4.5);

    // Opaque panes ignore the hint.
    theme.set_background_opacity(1.0);
    assert_eq!(theme.effective_background(), theme.background);
}

#[test]
fn from_parts_derives_readable_cursor_and_selection_text() {
    for theme in [Theme::campbell(), light_theme()] {
        let cursor_text = theme.cursor_text;
        let selection_text = theme.selection_fg.expect("derived");
        assert!(contrast::contrast_ratio(cursor_text, theme.cursor) >= 4.5, "cursor text on {:?}", theme.cursor);
        assert!(contrast::contrast_ratio(selection_text, theme.selection_bg) >= 4.5);
        assert!(contrast::contrast_ratio(theme.search_match_fg, theme.search_match_bg) >= 4.5);
        assert!(contrast::contrast_ratio(theme.search_current_fg, theme.search_current_bg) >= 4.5);
        assert_eq!(theme.background_opacity, 1.0);
        assert!(theme.bold_is_bright, "the single product default");
    }
}

#[test]
fn set_background_opacity_clamps() {
    let mut theme = Theme::campbell();
    theme.set_background_opacity(1.7);
    assert_eq!(theme.background_opacity, 1.0);
    theme.set_background_opacity(-0.2);
    assert_eq!(theme.background_opacity, 0.0);
    theme.set_background_opacity(0.72);
    assert_eq!(theme.background_opacity, 0.72);
    theme.set_background_opacity(f32::NAN);
    assert_eq!(theme.background_opacity, 1.0);
}
