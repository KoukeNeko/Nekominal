//! Cursor and focus rendering: block, bar and underline shapes, the hollow block of an unfocused pane, the
//! blink phase chosen by the caller, wide-character cursors, and the IME preedit underline.

use nekominal_render::{CursorShape, CursorView, PaneState, Preedit, Renderer, Theme};

use super::common::*;

fn view(col: u16, shape: CursorShape, visible: bool, focused: bool, wide: bool) -> CursorView {
    CursorView { col, row: 0, shape, visible, focused, wide }
}

fn cursor_scene(cursor: CursorView) -> Scene {
    let mut scene = Scene::new(8, 2);
    scene.cursor = Some(cursor);
    scene
}

fn rgb4(c: Rgb) -> [u8; 4] {
    [c.0, c.1, c.2, 255]
}

fn frame_of(scene: &Scene, theme: &Theme) -> (Frame, Renderer) {
    render_with(scene, theme, test_config())
}

#[test]
fn a_focused_block_cursor_fills_the_cell() {
    let theme = Theme::campbell();
    let (frame, _) = frame_of(&cursor_scene(view(2, CursorShape::Block, true, true, false)), &theme);
    let (w, h) = frame.cell;
    for (dx, dy) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1), (w / 2, h / 2)] {
        assert_eq!(frame.at(0, 2, dx, dy), rgb4(theme.cursor), "({dx},{dy})");
    }
    assert_eq!(frame.at(0, 1, 0, 0), rgb4(theme.background), "neighbours are untouched");
    assert_eq!(frame.at(0, 3, 0, 0), rgb4(theme.background));
}

#[test]
fn an_unfocused_pane_draws_a_hollow_block() {
    let theme = Theme::campbell();
    let (frame, renderer) = frame_of(&cursor_scene(view(2, CursorShape::Block, true, false, false)), &theme);
    let (w, h) = frame.cell;
    let t = renderer.metrics().underline_thickness;
    let (cursor, bg) = (rgb4(theme.cursor), rgb4(theme.background));
    // The outline: all four corners and the middle of each edge.
    for (dx, dy) in
        [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1), (w / 2, 0), (w / 2, h - 1), (0, h / 2), (w - 1, h / 2)]
    {
        assert_eq!(frame.at(0, 2, dx, dy), cursor, "outline pixel ({dx},{dy})");
    }
    // The outline is `t` thick and the inside is the plain background.
    assert_eq!(frame.at(0, 2, w / 2, t - 1), cursor);
    assert_eq!(frame.at(0, 2, w / 2, t), bg, "inside the outline");
    assert_eq!(frame.at(0, 2, w / 2, h / 2), bg);
    assert_eq!(frame.at(0, 2, t, h / 2), bg);
    assert_eq!(frame.at(0, 2, w - 1 - t, h / 2), bg);
    assert_eq!(frame.at(0, 3, 0, 0), bg);
}

#[test]
fn the_hollow_block_ignores_the_requested_shape() {
    let theme = Theme::campbell();
    let hollow = frame_of(&cursor_scene(view(1, CursorShape::Block, true, false, false)), &theme).0.rgba;
    for shape in [CursorShape::Bar, CursorShape::Underline] {
        let other = frame_of(&cursor_scene(view(1, shape, true, false, false)), &theme).0.rgba;
        assert_eq!(other, hollow, "{shape:?} while unfocused");
    }
}

#[test]
fn the_caller_drives_the_blink_phase() {
    let theme = Theme::campbell();
    let blank = frame_of(&Scene::new(8, 2), &theme).0.rgba;
    for (shape, focused) in [
        (CursorShape::Block, true),
        (CursorShape::Bar, true),
        (CursorShape::Underline, true),
        (CursorShape::Block, false),
    ] {
        let off = frame_of(&cursor_scene(view(2, shape, false, focused, false)), &theme).0.rgba;
        assert_eq!(off, blank, "{shape:?} focused={focused} in the off phase draws nothing");
        let on = frame_of(&cursor_scene(view(2, shape, true, focused, false)), &theme).0.rgba;
        assert_ne!(on, blank, "{shape:?} focused={focused} in the on phase is visible");
    }
}

#[test]
fn bar_and_underline_cursors_use_the_cursor_colour_at_the_cell_edges() {
    let theme = Theme::campbell();
    let (frame, renderer) = frame_of(&cursor_scene(view(3, CursorShape::Bar, true, true, false)), &theme);
    let (w, h) = frame.cell;
    let (cursor, bg) = (rgb4(theme.cursor), rgb4(theme.background));
    assert_eq!(frame.at(0, 3, 0, 0), cursor);
    assert_eq!(frame.at(0, 3, 0, h - 1), cursor, "the bar spans the whole cell height");
    assert_eq!(frame.at(0, 3, w - 1, h / 2), bg);

    let (frame, _) = frame_of(&cursor_scene(view(3, CursorShape::Underline, true, true, false)), &theme);
    let t = renderer.metrics().underline_thickness;
    assert_eq!(frame.at(0, 3, 0, h - 1), cursor);
    assert_eq!(frame.at(0, 3, w - 1, h - t), cursor);
    assert_eq!(frame.at(0, 3, w / 2, h - t - 1), bg);
    assert_eq!(frame.at(0, 3, w / 2, 0), bg);
}

#[test]
fn a_cursor_on_a_wide_character_covers_both_cells() {
    let theme = Theme::campbell();
    for (focused, shape) in [(true, CursorShape::Block), (false, CursorShape::Block), (true, CursorShape::Underline)] {
        let mut scene = Scene::new(8, 1);
        scene.put(0, 2, "中", Style::new());
        scene.cursor = Some(view(2, shape, true, focused, true));
        let (frame, _) = frame_of(&scene, &theme);
        let (w, h) = frame.cell;
        let cursor = rgb4(theme.cursor);
        // Last pixel row / last column of the second cell: only reachable when the cursor spans two cells.
        let probe = match (focused, shape) {
            (true, CursorShape::Block) => (3, w / 2, h / 2),
            (false, _) => (3, w - 1, h / 2),
            _ => (3, w / 2, h - 1),
        };
        assert_eq!(frame.at(0, probe.0, probe.1, probe.2), cursor, "focused={focused} {shape:?}");
        assert_ne!(frame.at(0, 4, 0, 0), cursor, "the cursor stops after the wide character");
    }
}

#[test]
fn a_block_cursor_recolours_the_text_under_it() {
    let theme = Theme::campbell();
    let mut scene = Scene::new(8, 1);
    scene.put(0, 2, "\u{2588}", Style::new().fg(idx(1)));
    scene.cursor = Some(view(2, CursorShape::Block, true, true, false));
    let (frame, _) = frame_of(&scene, &theme);
    // The block glyph is drawn in the cursor text colour over the cursor colour: the same colour the cursor
    // cell would otherwise be filled with is replaced entirely by cursor_text.
    assert_eq!(frame.at(0, 2, 4, 4), rgb4(theme.cursor_text));
    assert_eq!(frame.at(0, 2, 0, 0), rgb4(theme.cursor_text), "a full block covers the cursor background");
}

#[test]
fn blinking_a_bar_cursor_rebuilds_no_rows_and_a_block_cursor_rebuilds_one() {
    let gpu = gpu();
    let mut renderer = Renderer::new(&gpu, test_config()).expect("renderer");
    renderer.set_theme(&Theme::campbell());
    let mut pane = PaneState::new();
    let mut scene = Scene::new(20, 6);
    scene.put(2, 0, "some text", Style::new());

    for shape in [CursorShape::Bar, CursorShape::Block] {
        scene.cursor = Some(CursorView { col: 3, row: 2, shape, visible: true, focused: true, wide: false });
        update(&mut renderer, &mut pane, &scene);
        scene.cursor = Some(CursorView { col: 3, row: 2, shape, visible: false, focused: true, wide: false });
        let stats = update(&mut renderer, &mut pane, &scene);
        let expected = if shape == CursorShape::Block { 1 } else { 0 };
        assert_eq!(stats.rows_rebuilt, expected, "{shape:?}: rows rebuilt when the blink phase flips");
        // Hiding an overlay-only cursor uploads nothing (the overlay becomes empty) but must still ask for a frame.
        assert!(stats.needs_draw(), "{shape:?}: the blink flip changes the picture");

        let unchanged = update(&mut renderer, &mut pane, &scene);
        assert!(!unchanged.needs_draw(), "{shape:?}: nothing changed since the last update");
    }
}

#[test]
fn the_ime_preedit_is_underlined_on_its_own_background() {
    let theme = Theme::campbell();
    let mut scene = Scene::new(12, 2);
    // Spaces carry no ink, so the cell background is observable exactly.
    scene.preedit = Some(Preedit { row: 0, col: 2, text: "  ".into() });
    let (frame, renderer) = frame_of(&scene, &theme);
    let m = renderer.metrics();
    for col in 2..4 {
        assert_eq!(frame.at(0, col, 0, 0), rgb4(theme.preedit_bg), "preedit background in column {col}");
        assert_eq!(frame.at(0, col, 1, m.underline_top), rgb4(theme.foreground), "underline in column {col}");
    }
    assert_eq!(frame.at(0, 1, 0, 0), rgb4(theme.background));
    assert_eq!(frame.at(0, 4, 0, 0), rgb4(theme.background));
    assert_eq!(frame.at(0, 1, 1, m.underline_top), rgb4(theme.background), "no underline outside the composition");
    assert_eq!(frame.at(1, 2, 0, 0), rgb4(theme.background), "other rows are untouched");
}

#[test]
fn a_wide_preedit_character_is_underlined_across_both_of_its_cells() {
    let theme = Theme::campbell();
    let mut scene = Scene::new(12, 1);
    scene.preedit = Some(Preedit { row: 0, col: 2, text: "你".into() });
    let (frame, renderer) = frame_of(&scene, &theme);
    let m = renderer.metrics();
    let fg = rgb4(theme.foreground);
    for col in [2, 3] {
        assert_eq!(frame.at(0, col, 1, m.underline_top), fg, "underline in column {col}");
        assert_eq!(frame.at(0, col, m.width - 1, m.underline_top), fg);
    }
    assert_eq!(frame.at(0, 4, 1, m.underline_top), rgb4(theme.background), "the composition ends after the character");
}
