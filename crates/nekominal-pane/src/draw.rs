//! Drives one `nekominal_render::Renderer` for one pane. Knows nothing about the windowing toolkit: the shell
//! hands it a render target view to draw into.

use std::time::{Duration, Instant};

use nekominal_render::{self as r, d3d::Gpu};
use windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView;
use nekominal_term::{Snapshot, TerminalCore, alacritty_terminal::sync::FairMutex};

use crate::view;

pub struct PaneRenderer {
    renderer: r::Renderer,
    state: r::PaneState,
    theme: r::Theme,
    snapshot: Snapshot,
    cells: Vec<r::Cell>,
    /// The selected columns of every visible row, from the last snapshot.
    selection: Vec<r::RowSpan>,
    /// The cells of the link under the pointer, one span per row.
    hover_link: Vec<r::RowSpan>,
    /// The search matches in view, the current one marked.
    highlights: Vec<r::Highlight>,
    preedit: Option<String>,
    preedit_at: Option<r::Preedit>,
    force_draw: bool,
    focused: bool,
}

/// What `prepare` did, for logs and tests.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    pub needs_draw: bool,
    pub rows_rebuilt: usize,
    pub rows_total: usize,
    pub update: Duration,
}

impl PaneRenderer {
    pub fn new(gpu: &Gpu, config: r::RendererConfig, theme: r::Theme) -> Result<Self, String> {
        Self::new_sharing(gpu, config, theme, None)
    }

    /// Like [`PaneRenderer::new`], reusing the compiled pipelines of other (same device and format).
    pub fn new_sharing(
        gpu: &Gpu,
        config: r::RendererConfig,
        theme: r::Theme,
        other: Option<&PaneRenderer>,
    ) -> Result<Self, String> {
        let mut renderer = r::Renderer::new_sharing(gpu, config, other.map(|o| o.renderer.pipelines()))?;
        renderer.set_theme(&theme);
        Ok(Self {
            renderer,
            state: r::PaneState::new(),
            theme,
            snapshot: Snapshot::default(),
            cells: Vec::new(),
            selection: Vec::new(),
            hover_link: Vec::new(),
            highlights: Vec::new(),
            preedit: None,
            preedit_at: None,
            force_draw: true,
            focused: true,
        })
    }

    pub fn metrics(&self) -> r::fonts::CellMetrics {
        self.renderer.metrics()
    }

    pub fn lcd_active(&self) -> bool {
        self.renderer.lcd_active()
    }

    /// The last snapshot `prepare` took (visible text for accessibility and logs).
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Whole cells that fit in `size_px`.
    pub fn grid_for(&self, size_px: (u32, u32)) -> (usize, usize) {
        let m = self.renderer.metrics();
        ((size_px.0 / m.width).max(2) as usize, (size_px.1 / m.height).max(1) as usize)
    }

    /// Device pixels per DIP changed (DPI change, zoom): glyphs are re-rasterized at the new size.
    pub fn set_scale(&mut self, scale: f32) -> Result<(), String> {
        let (size_pt, line_height) = (self.renderer.cfg.font_size_pt, self.renderer.cfg.line_height);
        self.renderer.set_font(size_pt, scale, line_height)?;
        self.force_draw = true;
        Ok(())
    }

    /// Font size in points (the zoom commands and the settings page), keeping the current scale.
    pub fn set_font_size(&mut self, size_pt: f32) -> Result<(), String> {
        let (scale, line_height) = (self.renderer.cfg.scale, self.renderer.cfg.line_height);
        self.renderer.set_font(size_pt, scale, line_height)?;
        self.force_draw = true;
        Ok(())
    }

    /// The height of a line as a multiple of the font's own, keeping the size and the scale.
    pub fn set_line_height(&mut self, line_height: f32) -> Result<(), String> {
        let (size_pt, scale) = (self.renderer.cfg.font_size_pt, self.renderer.cfg.scale);
        self.renderer.set_font(size_pt, scale, line_height)?;
        self.force_draw = true;
        Ok(())
    }

    pub fn set_theme(&mut self, theme: r::Theme) {
        self.theme = theme;
        self.force_draw = true;
    }

    /// An unfocused pane is drawn with its text pulled toward the background.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.force_draw = true;
    }

    /// The link under the pointer is underlined; an empty list removes the underline.
    pub fn set_hover_link(&mut self, spans: Vec<r::RowSpan>) {
        if spans != self.hover_link {
            self.hover_link = spans;
            self.force_draw = true;
        }
    }

    /// The search matches in view (grid positions through `view::match_spans`); an empty list removes them.
    pub fn set_highlights(&mut self, highlights: Vec<r::Highlight>) {
        if highlights != self.highlights {
            self.highlights = highlights;
            self.force_draw = true;
        }
    }

    /// The text being composed by the IME, drawn at the cursor cell.
    pub fn set_preedit(&mut self, text: Option<String>) {
        if text != self.preedit {
            self.preedit = text;
            self.force_draw = true;
        }
    }

    /// The target changed (resize, device re-creation): the next `prepare` asks for a frame.
    pub fn invalidate(&mut self) {
        self.force_draw = true;
    }

    /// Cursor cell in physical pixels relative to the pane origin (IME candidate window anchor).
    pub fn cursor_cell_px(&self, term: &FairMutex<TerminalCore>) -> (f32, f32, f32, f32) {
        let m = self.renderer.metrics();
        let (row, col) = term.lock().cursor_cell();
        ((col as u32 * m.width) as f32, (row as u32 * m.height) as f32, m.width as f32, m.height as f32)
    }

    /// Snapshots the core when it changed and updates the renderer's instance buffers.
    pub fn prepare(&mut self, term: &FairMutex<TerminalCore>) -> Prepared {
        let dirty = {
            let mut core = term.lock();
            core.flush_expired_sync(Instant::now());
            if core.take_dirty() {
                core.snapshot(&mut self.snapshot);
                true
            } else {
                false
            }
        };
        if dirty {
            view::fill_cells(&self.snapshot, &mut self.cells);
            view::selection_spans(&self.snapshot, &mut self.selection);
        }
        if self.snapshot.columns == 0 || (!dirty && !self.force_draw) {
            return Prepared::default();
        }
        self.preedit_at = self.preedit.as_ref().map(|text| {
            let (row, col) = term.lock().cursor_cell();
            r::Preedit { row: row as u16, col: col as u16, text: text.clone() }
        });
        let themed = view::themed(&self.theme, &self.snapshot);
        self.renderer.set_theme(&if self.focused { themed } else { view::dimmed(&themed) });
        let grid = r::GridView {
            cols: self.snapshot.columns,
            rows: self.snapshot.rows,
            cells: &self.cells,
            cursor: view::cursor(&self.snapshot, self.focused, true),
            selection: &self.selection,
            highlights: &self.highlights,
            hover_link: &self.hover_link,
            preedit: self.preedit_at.as_ref(),
            scrollbar: None,
        };
        let started = Instant::now();
        let stats = self.renderer.update_pane(&mut self.state, &grid);
        Prepared {
            needs_draw: stats.needs_draw() || self.force_draw,
            rows_rebuilt: stats.rows_rebuilt,
            rows_total: stats.rows_total,
            update: started.elapsed(),
        }
    }

    /// Draws the pane into the `rect` (x, y, width, height in pixels) of a window-sized `target`. The pane paints its
    /// own background, so what the shell draws around it shows the backdrop. `clear` empties the target to transparent
    /// first: true for the first pane of a frame, false for the others.
    pub fn draw_in(&mut self, target: &ID3D11RenderTargetView, rect: [u32; 4], clear: bool) {
        let clear = clear.then_some([0.0; 4]);
        self.renderer.draw(target, clear, &[r::PaneDraw { pane: &self.state, rect }]);
        self.force_draw = false;
    }

    /// Draws the pane into `target` (`size_px` is the texture size). Call after a `prepare` that asked for it.
    pub fn draw(&mut self, target: &ID3D11RenderTargetView, size_px: (u32, u32)) {
        let bg = self.theme.background;
        let clear = if self.theme.background_opacity < 1.0 {
            [0.0; 4]
        } else {
            [f32::from(bg.0) / 255.0, f32::from(bg.1) / 255.0, f32::from(bg.2) / 255.0, 1.0]
        };
        self.renderer.draw(target, Some(clear), &[r::PaneDraw { pane: &self.state, rect: [0, 0, size_px.0, size_px.1] }]);
        self.force_draw = false;
    }
}
