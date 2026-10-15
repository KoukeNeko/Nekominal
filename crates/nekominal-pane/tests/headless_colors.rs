//! Headless acceptance tests for the Snapshot -> GridView -> Renderer path on WARP.
//! They guard the colour-resolution decision: colours stay symbolic until the renderer's Theme resolves them,
//! so reverse video is applied exactly once and default-background cells share the single pane background.

use nekominal_pane::PaneRenderer;
use nekominal_render::{
    self as r,
    offscreen::{AdapterKind, Gpu, OffscreenTarget, create_gpu},
};
use nekominal_term::{CoreConfig, TerminalCore, alacritty_terminal::sync::FairMutex};

/// One WARP device per test thread: the immediate context of a device is not thread safe, and the tests run in
/// parallel.
fn gpu() -> Gpu {
    thread_local! { static GPU: Gpu = create_gpu(AdapterKind::Warp).expect("WARP adapter"); }
    GPU.with(Gpu::clone)
}

struct Rendered {
    rgba: Vec<u8>,
    width: u32,
    cell: (u32, u32),
}

impl Rendered {
    /// RGB of the pixel (`dx`, `dy`) inside cell (`row`, `col`).
    fn at(&self, row: u32, col: u32, dx: u32, dy: u32) -> [u8; 3] {
        let x = col * self.cell.0 + dx;
        let y = row * self.cell.1 + dy;
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }
}

fn render(bytes: &[u8], cols: usize, rows: usize) -> Rendered {
    let gpu = gpu();
    let theme = r::Theme::campbell();
    let mut renderer = PaneRenderer::new(&gpu, r::RendererConfig::default(), theme).expect("renderer");
    let m = renderer.metrics();
    let term = FairMutex::new(TerminalCore::new(cols, rows, CoreConfig::default()));
    term.lock().feed(bytes);
    let prepared = renderer.prepare(&term);
    assert!(prepared.needs_draw);
    let size = (cols as u32 * m.width, rows as u32 * m.height);
    let target = OffscreenTarget::new(&gpu, size.0, size.1);
    renderer.draw(&target.view, size);
    let rgba = target.read_rgba(&gpu).expect("readback");
    Rendered { rgba, width: size.0, cell: (m.width, m.height) }
}

const FG: [u8; 3] = [0xCC, 0xCC, 0xCC];
const BG: [u8; 3] = [0x0C, 0x0C, 0x0C];

#[test]
fn reverse_video_is_applied_exactly_once() {
    // Cell 0 reverse (full block of reverse space), cell 2 plain space. Corner pixel (0,0) carries no glyph ink.
    let image = render(b"\x1b[7m \x1b[0m  ", 6, 1);
    assert_eq!(image.at(0, 0, 0, 0), FG, "reverse cell shows the foreground colour as background");
    assert_eq!(image.at(0, 2, 0, 0), BG, "plain cell shows the theme background");
}

#[test]
fn reverse_video_of_coloured_cells_swaps_once() {
    // red on blue, reversed: the cell background becomes red (a second inversion would show blue).
    let image = render(b"\x1b[31;44;7m \x1b[0m  ", 6, 1);
    assert_eq!(image.at(0, 0, 0, 0), [0xC5, 0x0F, 0x1F]);
}

#[test]
fn truecolor_and_indexed_backgrounds_resolve_through_the_theme() {
    let image = render(b"\x1b[48;2;0;100;200m \x1b[0m\x1b[41m \x1b[0m ", 6, 1);
    assert_eq!(image.at(0, 0, 0, 0), [0, 100, 200]);
    assert_eq!(image.at(0, 1, 0, 0), [0xC5, 0x0F, 0x1F], "SGR 41 is ANSI red of the Campbell theme");
    assert_eq!(image.at(0, 2, 0, 0), BG);
}

#[test]
fn hidden_text_draws_no_ink() {
    let image = render(b"\x1b[8mWWWW\x1b[0m", 6, 1);
    let (w, h) = image.cell;
    for dy in 0..h {
        for dx in 0..w {
            assert_eq!(image.at(0, 0, dx, dy), BG, "hidden cell must be pure background at ({dx},{dy})");
        }
    }
}
