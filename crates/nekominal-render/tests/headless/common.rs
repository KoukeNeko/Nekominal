//! Shared scaffolding for the headless renderer tests: one WARP device for the whole binary, a small scene
//! builder, rendering to RGBA bytes, and golden-image comparison.

use std::path::{Path, PathBuf};

use nekominal_render::fonts::FontConfig;
use nekominal_render::offscreen::{AdapterKind, Gpu, OffscreenTarget, create_gpu, write_png};
use nekominal_render::{
    Cell, CellFlags, CursorView, GridView, Highlight, PaneDraw, PaneState, Preedit, Renderer, RendererConfig, RowSpan,
    Theme, UpdateStats,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub use nekominal_render::{Color, Rgb, Underline};

/// One WARP device per test thread: the immediate context of a device is not thread safe, and the tests run in
/// parallel.
pub fn gpu() -> Gpu {
    thread_local! { static GPU: Gpu = create_gpu(AdapterKind::Warp).expect("WARP adapter"); }
    GPU.with(Gpu::clone)
}

/// Consolas ships with every desktop Windows, so the goldens do not depend on an optional font. CJK, emoji and
/// symbols still come from the default chains (Microsoft JhengHei, Segoe UI Emoji, Segoe UI Symbol).
pub fn test_config() -> RendererConfig {
    RendererConfig {
        fonts: FontConfig { text: vec!["Consolas".into()], ..FontConfig::default() },
        ..RendererConfig::default()
    }
}

pub fn rgb(hex: u32) -> Rgb {
    Rgb::from_hex(hex)
}

/// Text attributes for `Scene::put`.
#[derive(Clone, Copy)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
    pub underline_color: Option<Color>,
}

impl Style {
    pub fn new() -> Self {
        Style { fg: Color::Default, bg: Color::Default, flags: CellFlags(0), underline_color: None }
    }
    pub fn fg(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    pub fn bg(mut self, c: Color) -> Self {
        self.bg = c;
        self
    }
    pub fn flag(mut self, bit: u16) -> Self {
        self.flags = self.flags.with(bit);
        self
    }
    pub fn underline(mut self, u: Underline) -> Self {
        self.flags = self.flags.with_underline(u);
        self
    }
    pub fn underline_color(mut self, c: Color) -> Self {
        self.underline_color = Some(c);
        self
    }
}

pub fn idx(i: u8) -> Color {
    Color::Indexed(i)
}

/// A grid plus the overlay state a frame needs.
pub struct Scene {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    pub selection: Vec<RowSpan>,
    pub highlights: Vec<Highlight>,
    pub cursor: Option<CursorView>,
    pub preedit: Option<Preedit>,
}

impl Scene {
    pub fn new(cols: usize, rows: usize) -> Self {
        Scene {
            cols,
            rows,
            cells: vec![Cell::default(); cols * rows],
            selection: Vec::new(),
            highlights: Vec::new(),
            cursor: None,
            preedit: None,
        }
    }

    /// Emulator-style placement: wide clusters take two cells, zero-width marks attach to the base cell.
    /// Returns the column after the text.
    pub fn put(&mut self, row: usize, mut col: usize, text: &str, st: Style) -> usize {
        for grapheme in text.graphemes(true) {
            let mut chars = grapheme.chars();
            let base = chars.next().expect("grapheme has a char");
            let rest: Vec<char> = chars.collect();
            let width =
                if rest.is_empty() { base.width().unwrap_or(1) } else { UnicodeWidthStr::width(grapheme) }.clamp(1, 2);
            if row >= self.rows || col + width > self.cols {
                break;
            }
            let mut cell = Cell { ch: base, fg: st.fg, bg: st.bg, flags: st.flags, extra: None };
            if !rest.is_empty() {
                cell = cell.zerowidth(&rest);
            }
            if let Some(c) = st.underline_color {
                cell = cell.underline_color(c);
            }
            if width == 2 {
                cell.flags = cell.flags.with(CellFlags::WIDE);
            }
            self.cells[row * self.cols + col] = cell;
            if width == 2 {
                self.cells[row * self.cols + col + 1] =
                    Cell { ch: ' ', fg: st.fg, bg: st.bg, flags: CellFlags(CellFlags::WIDE_SPACER), extra: None };
            }
            col += width;
        }
        col
    }
}

/// The pixels of one frame as the swap chain would receive them (premultiplied when the theme is translucent).
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub cell: (u32, u32),
}

impl Frame {
    pub fn pixels(&self) -> &[[u8; 4]] {
        self.rgba.as_chunks::<4>().0
    }

    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// Pixel (`dx`, `dy`) inside cell (`row`, `col`).
    pub fn at(&self, row: u32, col: u32, dx: u32, dy: u32) -> [u8; 4] {
        self.px(col * self.cell.0 + dx, row * self.cell.1 + dy)
    }

    /// Source-over onto an opaque backdrop: what the compositor (DWM) shows for a premultiplied frame.
    pub fn over(&self, backdrop: Rgb) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rgba.len());
        let back = [backdrop.0, backdrop.1, backdrop.2];
        for p in self.pixels() {
            let keep = 255 - u32::from(p[3]);
            for c in 0..3 {
                out.push((u32::from(p[c]) + (u32::from(back[c]) * keep + 127) / 255).min(255) as u8);
            }
            out.push(255);
        }
        out
    }
}

pub fn render(scene: &Scene, theme: &Theme, cfg: RendererConfig) -> Frame {
    render_with(scene, theme, cfg).0
}

/// Like `render`, but also hands back the renderer so tests can ask for metrics and LCD state.
pub fn render_with(scene: &Scene, theme: &Theme, cfg: RendererConfig) -> (Frame, Renderer) {
    let gpu = gpu();
    let mut renderer = Renderer::new(&gpu, cfg).expect("renderer");
    renderer.set_theme(theme);
    let mut pane = PaneState::new();
    let stats = update(&mut renderer, &mut pane, scene);
    assert!(stats.needs_draw());
    let frame = draw_frame(&mut renderer, &pane, theme, scene.cols, scene.rows);
    (frame, renderer)
}

/// `update_pane` for a scene.
pub fn update(renderer: &mut Renderer, pane: &mut PaneState, scene: &Scene) -> UpdateStats {
    renderer.update_pane(
        pane,
        &GridView {
            cols: scene.cols,
            rows: scene.rows,
            cells: &scene.cells,
            cursor: scene.cursor,
            selection: &scene.selection,
            highlights: &scene.highlights,
            hover_link: &[],
            preedit: scene.preedit.as_ref(),
            scrollbar: None,
            marks: &[],
        },
    )
}

/// Draws an already updated pane into an offscreen target and reads it back.
pub fn draw_frame(renderer: &mut Renderer, pane: &PaneState, theme: &Theme, cols: usize, rows: usize) -> Frame {
    let gpu = gpu();
    let m = renderer.metrics();
    let (width, height) = (cols as u32 * m.width, rows as u32 * m.height);
    let target = OffscreenTarget::new(&gpu, width, height);
    // The same clear rule as `PaneRenderer::draw`.
    let clear = if theme.background_opacity < 1.0 {
        [0.0; 4]
    } else {
        let bg = theme.background;
        [f32::from(bg.0) / 255.0, f32::from(bg.1) / 255.0, f32::from(bg.2) / 255.0, 1.0]
    };
    renderer.draw(&target.view, Some(clear), &[PaneDraw { pane, rect: [0, 0, width, height] }]);
    let rgba = target.read_rgba(&gpu).expect("readback");
    Frame { rgba, width, height, cell: (m.width, m.height) }
}

pub fn theme_with_opacity(mut theme: Theme, opacity: f32) -> Theme {
    theme.set_background_opacity(opacity);
    theme
}

/// Windows Terminal "One Half Light": a light scheme to exercise dark-on-light text.
pub fn light_theme() -> Theme {
    Theme::from_parts(
        rgb(0xFAFAFA),
        rgb(0x383A42),
        rgb(0x4F525D),
        rgb(0xBFCEFF),
        [
            rgb(0x383A42),
            rgb(0xE45649),
            rgb(0x50A14F),
            rgb(0xC18301),
            rgb(0x0184BC),
            rgb(0xA626A4),
            rgb(0x0997B3),
            rgb(0xFAFAFA),
            rgb(0x4F525D),
            rgb(0xDF6C75),
            rgb(0x98C379),
            rgb(0xE4C07A),
            rgb(0x61AFEF),
            rgb(0xC577DD),
            rgb(0x56B6C2),
            rgb(0xFFFFFF),
        ],
    )
}

// ---------------------------------------------------------------------------
// Golden images
// ---------------------------------------------------------------------------

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join(format!("{name}.png"))
}

fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{}: expected 8-bit RGBA, got {:?}/{:?}", path.display(), info.color_type, info.bit_depth));
    }
    buf.truncate(info.buffer_size());
    Ok((info.width, info.height, buf))
}

/// Compares `rgba` with `tests/golden/<name>.png`: every channel of every pixel must be within 1/255.
/// `NEKOMINAL_BLESS_GOLDEN=1` rewrites the golden instead; a failing run leaves the actual image in the
/// target directory for inspection.
pub fn assert_golden(name: &str, width: u32, height: u32, rgba: &[u8]) {
    let path = golden_path(name);
    if std::env::var_os("NEKOMINAL_BLESS_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden dir")).expect("create golden dir");
        write_png(&path, width, height, rgba).expect("write golden");
        return;
    }
    let (gw, gh, golden) = read_png(&path).unwrap_or_else(|e| {
        panic!("missing or unreadable golden ({e}); run with NEKOMINAL_BLESS_GOLDEN=1 once and review the image")
    });
    let actual_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden-actual");
    let fail = |why: String| -> ! {
        std::fs::create_dir_all(&actual_dir).expect("create actual dir");
        let actual = actual_dir.join(format!("{name}.png"));
        write_png(&actual, width, height, rgba).expect("write actual");
        panic!("golden {name}: {why}; actual image written to {}", actual.display());
    };
    if (gw, gh) != (width, height) {
        fail(format!("size {width}x{height} but golden is {gw}x{gh}"));
    }
    let mut worst = 0u8;
    let mut bad = 0usize;
    let mut first = None;
    for (i, (a, g)) in rgba.iter().zip(&golden).enumerate() {
        let d = a.abs_diff(*g);
        if d > 1 {
            bad += 1;
            first.get_or_insert(i / 4);
        }
        worst = worst.max(d);
    }
    if bad > 0 {
        let at = first.unwrap_or(0);
        fail(format!(
            "{bad} channel values differ by more than 1 (worst {worst}), first at pixel ({}, {})",
            at as u32 % width,
            at as u32 / width
        ));
    }
}
