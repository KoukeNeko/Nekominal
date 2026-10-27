//! Pictures over the grid: where a quad lands (cells, pixel offsets, scaling, a top above the view), which layer its z
//! puts it in against cell backgrounds and text, and that it goes when the view no longer has it.

use std::sync::Arc;

use nekominal_render::{ImageView, PaneState, Renderer, Theme};

use super::common::*;

const RED: [u8; 4] = [255, 0, 0, 255];

/// A `width` x `height` picture of one colour.
fn picture(key: u64, width: u32, height: u32, rgba: [u8; 4]) -> ImageView {
    ImageView {
        key,
        width,
        height,
        pixels: Arc::from(rgba.repeat((width * height) as usize)),
        src: [0, 0, width, height],
        row: 0,
        col: 0,
        offset: [0, 0],
        cells: None,
        z: 0,
    }
}

fn opaque(p: [u8; 4]) -> [u8; 3] {
    [p[0], p[1], p[2]]
}

fn scene() -> Scene {
    let mut s = Scene::new(10, 4);
    // A full block in green over (1, 3) and a blue background on (1, 4): the layering probes.
    s.put(1, 3, "█", Style::new().fg(Color::Rgb(0, 255, 0)));
    s.put(1, 4, " ", Style::new().bg(Color::Rgb(0, 0, 255)));
    s
}

fn frame(scene: &Scene) -> Frame {
    render(scene, &Theme::campbell(), test_config())
}

#[test]
fn a_picture_scaled_into_cells_covers_exactly_those_cells() {
    let mut s = scene();
    s.images.push(ImageView { row: 1, col: 1, cells: Some([2, 2]), ..picture(1, 2, 2, RED) });
    let f = frame(&s);
    let (cw, ch) = f.cell;
    for (row, col) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
        assert_eq!(f.at(row, col, cw / 2, ch / 2), RED, "cell ({row}, {col})");
        assert_eq!(f.at(row, col, 0, 0), RED, "top-left pixel of ({row}, {col})");
        assert_eq!(f.at(row, col, cw - 1, ch - 1), RED, "bottom-right pixel of ({row}, {col})");
    }
    let background = [12, 12, 12, 255];
    assert_eq!(f.at(0, 1, cw / 2, ch - 1), background, "above");
    assert_eq!(f.at(3, 1, cw / 2, 0), background, "below");
    assert_eq!(f.at(1, 0, cw - 1, ch / 2), background, "left");
    assert_eq!(opaque(f.at(1, 3, cw / 2, ch / 2)), [0, 255, 0], "right: the text cell is untouched");
}

#[test]
fn a_picture_at_its_own_size_starts_at_its_pixel_offset_and_a_top_above_the_view_is_clipped() {
    let mut s = scene();
    s.images.push(ImageView { row: 2, col: 6, offset: [3, 5], ..picture(2, 4, 4, RED) });
    let (cw, ch) = frame(&Scene::new(1, 1)).cell;
    s.images.push(ImageView { row: -1, col: 8, ..picture(3, cw, ch * 2, RED) });
    let f = frame(&s);
    assert_eq!(f.at(2, 6, 3, 5), RED);
    assert_eq!(f.at(2, 6, 6, 8), RED);
    assert_ne!(f.at(2, 6, 2, 5), RED, "left of the offset");
    assert_ne!(f.at(2, 6, 7, 5), RED, "4 pixels wide");
    assert_eq!(f.at(0, 8, cw / 2, ch - 1), RED, "the lower half of a picture that starts above the view");
    assert_ne!(f.at(1, 8, cw / 2, 0), RED);
}

#[test]
fn z_puts_a_picture_over_the_text_between_backgrounds_and_text_or_under_the_backgrounds() {
    let probe = |z: i32| {
        let mut s = scene();
        s.images.push(ImageView { row: 1, col: 3, cells: Some([2, 1]), z, ..picture(4, 1, 1, RED) });
        let f = frame(&s);
        let (cw, ch) = f.cell;
        (opaque(f.at(1, 3, cw / 2, ch / 2)), opaque(f.at(1, 4, cw / 2, ch / 2)))
    };
    assert_eq!(probe(0), ([255, 0, 0], [255, 0, 0]), "over text and background");
    assert_eq!(probe(-1), ([0, 255, 0], [255, 0, 0]), "under the text, over the background");
    assert_eq!(probe(i32::MIN), ([0, 255, 0], [0, 0, 255]), "under both");
}

#[test]
fn a_translucent_picture_blends_with_what_is_under_it_and_a_picture_that_leaves_the_view_is_gone() {
    let gpu = gpu();
    let theme = Theme::campbell();
    let mut renderer = Renderer::new(&gpu, test_config()).expect("renderer");
    renderer.set_theme(&theme);
    let mut pane = PaneState::new();
    let mut s = Scene::new(4, 2);
    s.images.push(ImageView { cells: Some([1, 1]), ..picture(5, 1, 1, [255, 255, 255, 128]) });
    assert!(update(&mut renderer, &mut pane, &s).needs_draw());
    let f = draw_frame(&mut renderer, &pane, &theme, 4, 2);
    let p = f.at(0, 0, 1, 1);
    assert!((132..=136).contains(&p[0]), "half white over the 12-grey background: {p:?}");

    assert!(!update(&mut renderer, &mut pane, &s).needs_draw(), "nothing changed");
    s.images.clear();
    assert!(update(&mut renderer, &mut pane, &s).needs_draw(), "the picture went");
    let f = draw_frame(&mut renderer, &pane, &theme, 4, 2);
    assert_eq!(f.at(0, 0, 1, 1), [12, 12, 12, 255]);
}
