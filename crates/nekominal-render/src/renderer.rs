//! The terminal grid renderer: owns fonts, atlases, pipelines and per-pane
//! instance caches. Public flow per frame:
//!
//! ```text
//! renderer.set_theme(&theme);                       // rarely
//! renderer.update_pane(&mut pane, &grid_view);      // CPU: dirty rows only
//! renderer.draw(&target, clear, &[PaneDraw{..}]);
//! ```

use std::sync::Arc;

use rustc_hash::FxHashMap;
use windows::Win32::Graphics::{
    Direct3D11::{ID3D11Buffer, ID3D11RenderTargetView},
    Dxgi::Common::{DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM},
};

use crate::atlas::{AtlasFull, AtlasTexture};
use crate::cell::{CellFlags, Theme};
use crate::d3d::{Gpu, bind_textures, bind_uniforms, constant_buffer, set_shaders, write_buffer};
use crate::fonts::{CellMetrics, FontConfig, FontStack, Style, compute_metrics};
use crate::gpu::{self, GlyphInstance, GrowBuffer, Pipelines, RectInstance, Uniforms};
use crate::images::Layer;
use crate::raster::{GlyphKind, RasterOptions, Rasterizer};

#[derive(Clone)]
pub struct RendererConfig {
    pub fonts: FontConfig,
    pub font_size_pt: f32,
    /// Device pixels per DIP (1.0 = 100%, 1.25 = 125%, ...). Font pixels =
    /// pt * 96/72 * scale, so text is rasterized at native resolution.
    pub scale: f32,
    pub line_height: f32,
    /// Ask for LCD subpixel text. Used only when the device supports
    /// dual-source blending and the theme background is opaque.
    pub lcd: bool,
    /// TrueType/PostScript hinting through swash (skrifa).
    pub hint: bool,
    /// DirectWrite-style contrast + gamma correction of text coverage.
    pub text_correction: bool,
    pub gamma: f32,
    pub grayscale_contrast: f32,
    pub lcd_contrast: f32,
    /// Extra enhanced contrast for grayscale text, scaled by `1 - background_opacity`. A translucent pane
    /// blends text with a backdrop the renderer cannot know, so the DirectWrite correction (which assumes
    /// the background is the opposite of the text colour) makes strokes look thin; this thickens them. The
    /// default 1.6 adds about 9% ink at 72% opacity and 12-14% at 50% (measured as coverage mass of mixed
    /// Latin / CJK text, light and dark). 0.0 turns it off.
    pub translucent_text_boost: f32,
    pub atlas_initial: u32,
    /// Largest side of a glyph atlas texture. 2048 holds about 17,000 distinct wide CJK glyphs (5,000 use 28%
    /// of it), which is more than the Traditional Chinese characters in everyday use; past it the atlas
    /// clears and re-rasterizes what is on screen. Worst case per `Renderer`: 4 MiB (R8) + 16 MiB (RGBA) of
    /// GPU texture, plus the same again as CPU shadow copies. Panes that share a window should share one
    /// `Renderer` (it draws any number of `PaneState`s) instead of each owning an atlas.
    pub atlas_max: u32,
}

impl Default for RendererConfig {
    fn default() -> Self {
        RendererConfig {
            fonts: FontConfig::default(),
            font_size_pt: 12.0,
            scale: 1.0,
            line_height: 1.0,
            lcd: true,
            hint: false,
            text_correction: true,
            gamma: 1.8,
            grayscale_contrast: 0.5,
            lcd_contrast: 0.5,
            translucent_text_boost: 1.6,
            atlas_initial: 1024,
            atlas_max: 2048,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedBitmap {
    pub kind: GlyphKind,
    pub src: [u16; 2],
    pub size: [u16; 2],
}

/// A bitmap placed relative to the top-left corner of its cell.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedGlyph {
    pub bitmap: CachedBitmap,
    pub dx: i32,
    pub dy: i32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct ClusterKey {
    pub ch: char,
    pub zw: Box<[char]>,
    pub style: Style,
    pub span: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
    pub face: u16,
    pub glyph: u16,
    /// px * 64, rounded.
    pub px_q: u32,
    /// wght axis value (0 for static faces).
    pub wght: u16,
    /// bit0 synth bold, bit1 synth italic, bit2 lcd, bit3 hint
    pub flags: u8,
    /// quarter-pixel subpixel offset index 0..4
    pub subpx: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SpriteKey {
    Char(char),
    Underline(u8),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedGlyph {
    pub bitmap: Option<CachedBitmap>,
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Default)]
pub struct RendererStats {
    pub glyphs_rasterized: usize,
    pub sprites_drawn: usize,
    pub atlas_clears: u32,
    pub missing_glyphs: usize,
    /// Time spent in swash rasterization.
    pub raster_time: std::time::Duration,
    /// Time spent uploading bitmaps (queue.write_texture) including atlas growth.
    pub upload_time: std::time::Duration,
    /// Time spent resolving fonts for characters (cmap probing, DirectWrite fallback, face loading).
    pub font_time: std::time::Duration,
}

#[derive(Clone, Debug, Default)]
pub struct UpdateStats {
    pub rows_rebuilt: usize,
    pub rows_total: usize,
    pub bg_instances: u32,
    pub glyph_instances: u32,
    pub lcd_instances: u32,
    pub deco_instances: u32,
    pub overlay_instances: u32,
    pub uploaded_bytes: usize,
    pub atlas_restarts: u32,
    /// The overlay rectangles (cursor shapes, scrollbar) differ from the previous update, including the case
    /// where they all disappeared.
    pub overlay_changed: bool,
}

pub(crate) struct RowInstances {
    pub bg: Vec<RectInstance>,
    pub glyphs: Vec<GlyphInstance>,
    pub lcd: Vec<GlyphInstance>,
    pub deco: Vec<RectInstance>,
}

impl RowInstances {
    pub fn new() -> Self {
        RowInstances { bg: Vec::new(), glyphs: Vec::new(), lcd: Vec::new(), deco: Vec::new() }
    }
}

pub(crate) struct PaneGpu {
    pub bg: GrowBuffer,
    pub glyphs: GrowBuffer,
    pub lcd: GrowBuffer,
    pub deco: GrowBuffer,
    pub overlay: GrowBuffer,
    /// One rect covering the whole pane viewport with the theme background.
    pub pane_bg: GrowBuffer,
    pub uniforms: ID3D11Buffer,
}

/// Per-pane CPU caches + GPU buffers. One per terminal viewport.
pub struct PaneState {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) epoch: u64,
    pub(crate) gpu_gen: u64,
    pub(crate) row_keys: Vec<u64>,
    pub(crate) row_data: Vec<RowInstances>,
    pub(crate) overlay_cpu: Vec<RectInstance>,
    pub(crate) overlay_prev: Vec<RectInstance>,
    pub(crate) gpu: Option<PaneGpu>,
    pub(crate) scratch: Vec<u8>,
    pub(crate) images: crate::images::PaneImages,
}

impl Default for PaneState {
    fn default() -> Self {
        Self::new()
    }
}

impl PaneState {
    pub fn new() -> Self {
        PaneState {
            cols: 0,
            rows: 0,
            epoch: u64::MAX,
            gpu_gen: u64::MAX,
            row_keys: Vec::new(),
            row_data: Vec::new(),
            overlay_cpu: Vec::new(),
            overlay_prev: Vec::new(),
            gpu: None,
            scratch: Vec::new(),
            images: Default::default(),
        }
    }

    /// Instance buffer bytes currently reserved on the GPU.
    pub fn gpu_bytes(&self) -> u64 {
        self.gpu
            .as_ref()
            .map(|g| {
                g.bg.capacity() + g.glyphs.capacity() + g.lcd.capacity() + g.deco.capacity() + g.overlay.capacity()
            })
            .unwrap_or(0)
    }
}

pub struct PaneDraw<'a> {
    pub pane: &'a PaneState,
    /// x, y, width, height in target pixels. Drawing is clipped to this rect.
    pub rect: [u32; 4],
}

pub struct Renderer {
    pub(crate) gpu: Gpu,
    pub cfg: RendererConfig,
    pub(crate) theme: Arc<Theme>,
    pub(crate) pipelines: Arc<Pipelines>,
    pub(crate) atlas_gray: AtlasTexture,
    pub(crate) atlas_rgba: AtlasTexture,
    pub fonts: FontStack,
    pub(crate) raster: Rasterizer,
    pub(crate) metrics: CellMetrics,
    pub(crate) glyph_cache: FxHashMap<GlyphKey, CachedGlyph>,
    pub(crate) sprite_cache: FxHashMap<(SpriteKey, u32, u32), Option<CachedBitmap>>,
    pub(crate) cluster_cache: FxHashMap<ClusterKey, Arc<[PlacedGlyph]>>,
    pub(crate) epoch: u64,
    pub(crate) gpu_gen: u64,
    pub(crate) lcd_supported: bool,
    /// The picture pipeline, made when the first picture shows up.
    pub(crate) image_pipeline: Option<crate::images::ImagePipeline>,
    pub stats: RendererStats,
    pub init_timings: Vec<(&'static str, std::time::Duration)>,
}

impl UpdateStats {
    /// True when `update_pane` changed anything that needs a new frame.
    pub fn needs_draw(&self) -> bool {
        self.rows_rebuilt > 0 || self.uploaded_bytes > 0 || self.atlas_restarts > 0 || self.overlay_changed
    }
}

impl Renderer {
    /// `device` should have been created with `Features::DUAL_SOURCE_BLENDING`
    /// when the adapter offers it (see [`Renderer::wanted_features`]).
    pub fn new(
        gpu: &Gpu,
        cfg: RendererConfig,
    ) -> Result<Self, String> {
        Self::new_sharing(gpu, cfg, None)
    }

    /// The compiled pipelines, to give to the next renderer of the same device and format.
    pub fn pipelines(&self) -> Arc<Pipelines> {
        self.pipelines.clone()
    }

    /// Like [`Renderer::new`], reusing the pipelines of another renderer instead of compiling them again.
    pub fn new_sharing(
        gpu: &Gpu,
        cfg: RendererConfig,
        shared: Option<Arc<Pipelines>>,
    ) -> Result<Self, String> {
        let mut init_timings = Vec::new();
        // Dual-source blending is part of feature level 11.
        let lcd_supported = true;
        let t = std::time::Instant::now();
        let pipelines = match shared {
            Some(shared) => shared,
            None => Arc::new(Pipelines::new(gpu)?),
        };
        init_timings.push(("pipelines (HLSL, 3 pixel shaders)", t.elapsed()));
        let t = std::time::Instant::now();
        let atlas_gray = AtlasTexture::new(gpu, DXGI_FORMAT_R8_UNORM, 1, cfg.atlas_initial, cfg.atlas_max)?;
        let atlas_rgba = AtlasTexture::new(gpu, DXGI_FORMAT_R8G8B8A8_UNORM, 4, cfg.atlas_initial, cfg.atlas_max)?;
        init_timings.push(("atlas textures", t.elapsed()));
        let t = std::time::Instant::now();
        let fonts = FontStack::new(cfg.fonts.clone())?;
        init_timings.push(("FontStack::new (DirectWrite locator + primary faces)", t.elapsed()));
        let mut r = Renderer {
            gpu: gpu.clone(),
            theme: Arc::new(Theme::campbell()),
            pipelines,
            atlas_gray,
            atlas_rgba,
            fonts,
            raster: Rasterizer::new(),
            metrics: CellMetrics {
                width: 1,
                height: 1,
                baseline: 1,
                underline_top: 0,
                underline_thickness: 1,
                strike_top: 0,
                strike_thickness: 1,
                px_size: 1.0,
                advance: 1.0,
                ascent: 1.0,
                descent: 0.0,
                leading: 0.0,
            },
            glyph_cache: FxHashMap::default(),
            sprite_cache: FxHashMap::default(),
            cluster_cache: FxHashMap::default(),
            epoch: 0,
            gpu_gen: 0,
            lcd_supported,
            image_pipeline: None,
            stats: RendererStats::default(),
            init_timings,
            cfg,
        };
        let t = std::time::Instant::now();
        r.recompute_metrics()?;
        r.init_timings.push(("cell metrics", t.elapsed()));
        Ok(r)
    }

    fn recompute_metrics(&mut self) -> Result<(), String> {
        let px = self.cfg.font_size_pt * 96.0 / 72.0 * self.cfg.scale;
        let primary = self.fonts.primary(Style::REGULAR).ok_or("no primary font")?;
        self.metrics = compute_metrics(self.fonts.face(primary.face), px, self.cfg.line_height);
        Ok(())
    }

    pub fn metrics(&self) -> CellMetrics {
        self.metrics
    }

    /// True when LCD text is actually in effect (config + device + opaque theme).
    pub fn lcd_active(&self) -> bool {
        self.cfg.lcd && self.lcd_supported && self.theme.background_opacity >= 1.0
    }

    pub fn atlas_memory_bytes(&self) -> (u64, u64) {
        (self.atlas_gray.memory_bytes(), self.atlas_rgba.memory_bytes())
    }

    pub fn atlas_info(&self) -> String {
        format!(
            "r8 {}x{} used {:.1}% (grows {}, clears {}) | rgba {}x{} used {:.1}% (grows {}, clears {})",
            self.atlas_gray.size,
            self.atlas_gray.size,
            self.atlas_gray.used_fraction() * 100.0,
            self.atlas_gray.grows,
            self.atlas_gray.clears,
            self.atlas_rgba.size,
            self.atlas_rgba.size,
            self.atlas_rgba.used_fraction() * 100.0,
            self.atlas_rgba.grows,
            self.atlas_rgba.clears
        )
    }

    /// Force every pane to rebuild all rows on its next `update_pane`.
    pub fn invalidate_all(&mut self) {
        self.epoch += 1;
    }

    pub fn set_theme(&mut self, theme: &Theme) {
        if *self.theme == *theme {
            return;
        }
        let lcd_before = self.lcd_active();
        self.theme = Arc::new(theme.clone());
        if self.lcd_active() != lcd_before {
            self.reset_glyph_caches();
        }
        self.epoch += 1;
    }

    /// Change font size / scale / line height (e.g. DPI change, zoom).
    pub fn set_font(&mut self, size_pt: f32, scale: f32, line_height: f32) -> Result<(), String> {
        self.cfg.font_size_pt = size_pt;
        self.cfg.scale = scale;
        self.cfg.line_height = line_height;
        self.recompute_metrics()?;
        self.reset_glyph_caches();
        self.epoch += 1;
        Ok(())
    }

    /// Change the families of the primary text chain (empty keeps the default chain); the other chains stay. Glyphs are
    /// resolved and rasterized again from the new fonts.
    pub fn set_text_families(&mut self, families: Vec<String>) -> Result<(), String> {
        let mut fonts = self.cfg.fonts.clone();
        fonts.text = if families.is_empty() { FontConfig::default().text } else { families };
        let stack = FontStack::new(fonts.clone())?;
        self.cfg.fonts = fonts;
        self.fonts = stack;
        self.recompute_metrics()?;
        self.reset_glyph_caches();
        self.epoch += 1;
        Ok(())
    }

    pub(crate) fn reset_glyph_caches(&mut self) {
        self.glyph_cache.clear();
        self.sprite_cache.clear();
        self.cluster_cache.clear();
        self.atlas_gray.clear();
        self.atlas_rgba.clear();
        self.stats.atlas_clears += 1;
        self.epoch += 1;
    }

    /// Rebuild every GPU object on a new device (device lost / adapter change).
    /// CPU state (fonts, metrics, theme) is kept; atlases are re-rasterized on demand.
    pub fn recreate_gpu(&mut self, gpu: &Gpu) -> Result<(), String> {
        self.gpu = gpu.clone();
        self.pipelines = Arc::new(Pipelines::new(gpu)?);
        self.atlas_gray = AtlasTexture::new(gpu, DXGI_FORMAT_R8_UNORM, 1, self.cfg.atlas_initial, self.cfg.atlas_max)?;
        self.atlas_rgba = AtlasTexture::new(gpu, DXGI_FORMAT_R8G8B8A8_UNORM, 4, self.cfg.atlas_initial, self.cfg.atlas_max)?;
        self.glyph_cache.clear();
        self.sprite_cache.clear();
        self.cluster_cache.clear();
        self.image_pipeline = None;
        self.gpu_gen += 1;
        self.epoch += 1;
        Ok(())
    }

    pub(crate) fn raster_options(&self) -> RasterOptions {
        RasterOptions { lcd: self.lcd_active(), hint: self.cfg.hint }
    }

    pub(crate) fn insert_bitmap(
        &mut self,
        kind: GlyphKind,
        w: u32,
        h: u32,
        data: &[u8],
    ) -> Result<[u16; 2], AtlasFull> {
        let atlas = match kind {
            GlyphKind::Mask => &mut self.atlas_gray,
            GlyphKind::Lcd | GlyphKind::Color => &mut self.atlas_rgba,
        };
        let t = std::time::Instant::now();
        let r = atlas.insert(&self.gpu, w, h, data);
        self.stats.upload_time += t.elapsed();
        let (x, y) = r?;
        Ok([x, y])
    }

    pub(crate) fn pane_gpu(&self) -> Result<PaneGpu, String> {
        let gpu = &self.gpu;
        Ok(PaneGpu {
            bg: GrowBuffer::new(gpu, std::mem::size_of::<RectInstance>() as u32, 4096)?,
            glyphs: GrowBuffer::new(gpu, std::mem::size_of::<GlyphInstance>() as u32, 8192)?,
            lcd: GrowBuffer::new(gpu, std::mem::size_of::<GlyphInstance>() as u32, 8192)?,
            deco: GrowBuffer::new(gpu, std::mem::size_of::<RectInstance>() as u32, 4096)?,
            overlay: GrowBuffer::new(gpu, std::mem::size_of::<RectInstance>() as u32, 1024)?,
            pane_bg: GrowBuffer::new(gpu, std::mem::size_of::<RectInstance>() as u32, 256)?,
            uniforms: constant_buffer(gpu, std::mem::size_of::<Uniforms>())?,
        })
    }

    fn uniforms_for(&self, w: u32, h: u32) -> Uniforms {
        Uniforms {
            viewport: [w as f32, h as f32],
            contrast: [self.cfg.grayscale_contrast, self.cfg.lcd_contrast],
            gamma_ratios: gpu::gamma_ratios(self.cfg.gamma),
            flags: self.cfg.text_correction as u32,
            text_boost: self.cfg.translucent_text_boost * (1.0 - self.theme.background_opacity.clamp(0.0, 1.0)),
            pad: [0; 2],
        }
    }

    /// Draws one or more panes into `target`, which is cleared to `clear` (or kept when `None`). Call
    /// `update_pane` for every pane first.
    ///
    /// With a translucent theme (`background_opacity < 1`) the target is a premultiplied-alpha surface and
    /// `clear` must be transparent: the pane background rect carries the opacity, and an opaque clear color
    /// underneath would defeat it.
    pub fn draw(&mut self, target: &ID3D11RenderTargetView, clear: Option<[f32; 4]>, panes: &[PaneDraw<'_>]) {
        let gpu = &self.gpu;
        self.atlas_gray.flush(gpu);
        self.atlas_rgba.flush(gpu);
        if let Some(color) = clear {
            gpu.clear(target, color);
        }
        let pipes = &self.pipelines;
        gpu.begin_draws(&pipes.states);
        bind_textures(gpu, &[&self.atlas_gray.view, &self.atlas_rgba.view]);
        for p in panes {
            let Some(pane) = &p.pane.gpu else { continue };
            let [x, y, w, h] = p.rect;
            let uniforms = self.uniforms_for(w, h);
            write_buffer(gpu, &pane.uniforms, bytemuck::bytes_of(&uniforms));
            let bg = self.theme.background;
            let rect = RectInstance {
                rect: [0.0, 0.0, w as f32, h as f32],
                color: [bg.0, bg.1, bg.2, (self.theme.background_opacity.clamp(0.0, 1.0) * 255.0).round() as u8],
            };
            // One instance in a buffer sized for more: the others are never drawn.
            write_buffer(gpu, &pane.pane_bg.buffer, bytemuck::bytes_of(&rect));

            gpu.bind_target(target, [x, y, w, h]);
            bind_uniforms(gpu, &pane.uniforms);

            pipes.states.set_blend(gpu, false);
            set_shaders(gpu, &pipes.rect_layout, &pipes.rect_vs, &pipes.rect_ps);
            gpu.draw_instances(&pane.pane_bg.buffer, pane.pane_bg.stride, 1);
            if self.draw_images(p.pane, Layer::UnderBackgrounds) {
                set_shaders(gpu, &pipes.rect_layout, &pipes.rect_vs, &pipes.rect_ps);
            }
            pane.bg.draw(gpu);
            self.draw_images(p.pane, Layer::UnderText);

            set_shaders(gpu, &pipes.glyph_layout, &pipes.glyph_vs, &pipes.glyph_ps);
            pane.glyphs.draw(gpu);
            if pane.lcd.count > 0 {
                pipes.states.set_blend(gpu, true);
                set_shaders(gpu, &pipes.glyph_layout, &pipes.glyph_vs, &pipes.glyph_lcd_ps);
                pane.lcd.draw(gpu);
                pipes.states.set_blend(gpu, false);
            }

            set_shaders(gpu, &pipes.rect_layout, &pipes.rect_vs, &pipes.rect_ps);
            pane.deco.draw(gpu);
            if self.draw_images(p.pane, Layer::OverText) {
                set_shaders(gpu, &pipes.rect_layout, &pipes.rect_vs, &pipes.rect_ps);
            }
            pane.overlay.draw(gpu);
        }
    }
}

pub(crate) fn style_of(flags: CellFlags) -> Style {
    Style { bold: flags.has(CellFlags::BOLD), italic: flags.has(CellFlags::ITALIC) }
}
