//! Font ligatures (`RendererConfig::ligatures`): a font without them draws the same either way, a font with them
//! (Cascadia Code, which ships with Windows 11) draws `=>` differently, and the cursor, a selection and a change of
//! colour inside `=>` each break the ligature. Tests that need Cascadia Code pass without checking anything where it
//! is not installed; the shaping and run splitting themselves are unit tests in `src/ligatures.rs`.

use nekominal_render::fonts::FontConfig;
use nekominal_render::{CursorShape, CursorView, PaneState, Renderer, RendererConfig, RowSpan, Theme};

use super::common::*;

fn config(family: &str, ligatures: bool) -> RendererConfig {
    RendererConfig { fonts: FontConfig { text: vec![family.into()], ..FontConfig::default() }, ligatures, ..RendererConfig::default() }
}

/// The frame of `scene` with ligatures on and off; `None` when `family` is not installed.
fn both(scene: &Scene, family: &str) -> Option<(Frame, Frame)> {
    Renderer::new(&gpu(), config(family, false)).ok()?;
    let theme = Theme::campbell();
    Some((render(scene, &theme, config(family, true)), render(scene, &theme, config(family, false))))
}

/// Whether two frames have the same pixels in columns `cols` of row 0.
fn same_cells(a: &Frame, b: &Frame, cols: std::ops::Range<u32>) -> bool {
    (0..a.cell.1).all(|y| (cols.start * a.cell.0..cols.end * a.cell.0).all(|x| a.px(x, y) == b.px(x, y)))
}

fn arrows() -> Scene {
    let mut s = Scene::new(24, 1);
    s.put(0, 0, "a => b != c -> d >= e", Style::new());
    s
}

fn bar_cursor(col: u16) -> Option<CursorView> {
    Some(CursorView { col, row: 0, shape: CursorShape::Bar, visible: true, focused: true, wide: false })
}

#[test]
fn a_font_without_ligatures_draws_the_same_with_them_on() {
    let (on, off) = both(&arrows(), "Consolas").expect("Consolas ships with Windows");
    assert!(on.rgba == off.rgba);
}

#[test]
fn cascadia_code_joins_an_arrow_and_leaves_the_letters_around_it_as_they_were() {
    let Some((on, off)) = both(&arrows(), "Cascadia Code") else { return };
    assert!(!same_cells(&on, &off, 2..4), "`=>` is one shape with ligatures on");
    assert!(same_cells(&on, &off, 0..1) && same_cells(&on, &off, 5..6), "`a` and `b` stay where they were");
}

#[test]
fn the_cursor_breaks_the_ligature_it_stands_in_whatever_its_shape() {
    for shape in [CursorShape::Bar, CursorShape::Block, CursorShape::Underline] {
        let mut s = arrows();
        s.cursor = Some(CursorView { shape, ..bar_cursor(3).unwrap() });
        let Some((on, off)) = both(&s, "Cascadia Code") else { return };
        assert!(same_cells(&on, &off, 0..5), "{shape:?}: `=>` with the cursor on `>` draws as two characters");
    }
}

#[test]
fn a_selection_or_a_change_of_colour_inside_an_arrow_breaks_it() {
    let mut selected = arrows();
    selected.selection.push(RowSpan { row: 0, start: 3, end: 8 });
    let mut coloured = Scene::new(8, 1);
    let c = coloured.put(0, 0, "a =", Style::new().fg(idx(1)));
    coloured.put(0, c, "> b", Style::new().fg(idx(2)));
    for s in [selected, coloured] {
        let Some((on, off)) = both(&s, "Cascadia Code") else { return };
        assert!(same_cells(&on, &off, 0..5));
    }
}

#[test]
fn a_full_screen_redraw_with_ligatures_on_shapes_nothing_new() {
    let Ok(mut renderer) = Renderer::new(&gpu(), config("Cascadia Code", true)) else { return };
    let mut s = Scene::new(40, 20);
    for row in 0..20 {
        s.put(row, 0, &format!("let x{row} = |a| a >= {row} && a != 0; // ->"), Style::new());
    }
    let mut pane = PaneState::new();
    update(&mut renderer, &mut pane, &s);
    let shaped = renderer.ligature_runs_shaped();
    assert!(shaped > 0);
    renderer.invalidate_all();
    let stats = update(&mut renderer, &mut pane, &s);
    assert_eq!(stats.rows_rebuilt, 20);
    assert_eq!(renderer.ligature_runs_shaped(), shaped, "every run comes from the cache");
}

#[test]
fn with_ligatures_off_nothing_is_shaped() {
    let mut renderer = Renderer::new(&gpu(), test_config()).expect("renderer");
    let mut pane = PaneState::new();
    update(&mut renderer, &mut pane, &arrows());
    assert_eq!(renderer.ligature_runs_shaped(), 0);
}

#[test]
fn turning_ligatures_on_redraws_every_row() {
    let Ok(mut renderer) = Renderer::new(&gpu(), config("Cascadia Code", false)) else { return };
    let mut pane = PaneState::new();
    let s = arrows();
    update(&mut renderer, &mut pane, &s);
    renderer.set_ligatures(true);
    assert_eq!(update(&mut renderer, &mut pane, &s).rows_rebuilt, 1);
    assert!(renderer.ligature_runs_shaped() > 0);
}
