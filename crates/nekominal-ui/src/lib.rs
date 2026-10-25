//! The self-drawn UI layer: rounded rectangles, shadows, text and icon glyphs recorded in DIP and drawn with Direct3D 11 in
//! one pass over whatever the target already holds (the terminal panes).
//!
//! A frame is `begin_frame`, any number of `rrect` / `shadow` / `text` / `icon` calls in painting order (with
//! `push_clip` / `pop_clip` around clipped parts), then `end_frame`. Every coordinate and size is in DIP; the scale
//! factor given to `begin_frame` turns them into pixels, and glyphs are placed on whole pixels. Text comes from
//! the font stack of the terminal renderer (DirectWrite lookup, per-character fallback for CJK and emoji), rasterized
//! with swash into a shared glyph atlas.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use nekominal_render::{
    atlas::AtlasTexture,
    d3d::{
        Attribute, DrawStates, Gpu, bind_textures, bind_uniforms, compile, constant_buffer, input_layout, pixel_shader,
        set_shaders, vertex_shader, write_buffer,
    },
    fonts::{CharClass, FaceId, FontChoice, FontConfig, FontStack, Style, is_cjk},
    gpu::{GLYPH_ATTRS, GlyphInstance, GrowBuffer, KIND_COLOR, KIND_MASK},
    raster::{GlyphKind, RasterOptions, Rasterizer},
};
use rustc_hash::FxHashMap;
use windows::Win32::Graphics::{
    Direct3D11::{ID3D11Buffer, ID3D11InputLayout, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11VertexShader},
    Dxgi::Common::{DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Straight (not premultiplied) RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color([r, g, b, a])
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color([r, g, b, 255])
    }

    pub const TRANSPARENT: Color = Color([0, 0, 0, 0]);
}

/// One rounded rectangle (or shadow) in pixels. 40 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct QuadInstance {
    rect: [f32; 4],
    /// Corner radius, border width.
    rb: [f32; 2],
    fill: [u8; 4],
    border: [u8; 4],
    /// Blur radius, kind (0 rectangle, 1 shadow).
    extra: [f32; 2],
}

const QUAD_ATTRS: [Attribute; 5] = [
    Attribute(DXGI_FORMAT_R32G32B32A32_FLOAT, 0),
    Attribute(DXGI_FORMAT_R32G32_FLOAT, 16),
    Attribute(DXGI_FORMAT_R8G8B8A8_UNORM, 24),
    Attribute(DXGI_FORMAT_R8G8B8A8_UNORM, 28),
    Attribute(DXGI_FORMAT_R32G32_FLOAT, 32),
];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    viewport: [f32; 2],
    pad: [f32; 2],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    face: FaceId,
    glyph: u16,
    px: u32,
    wght: u32,
    synth: u8,
}

#[derive(Clone, Copy)]
struct Bitmap {
    src: [u16; 2],
    size: [u16; 2],
    left: i32,
    top: i32,
    kind: u32,
}

#[derive(Clone, Copy)]
struct Cached {
    advance: f32,
    bitmap: Option<Bitmap>,
}

/// A run of instances of one kind drawn with one scissor rect.
struct Cmd {
    glyphs: bool,
    range: Range<u32>,
    clip: Option<[u32; 4]>,
}

pub struct Ui {
    gpu: Gpu,
    states: DrawStates,
    quad_layout: ID3D11InputLayout,
    quad_vs: ID3D11VertexShader,
    quad_ps: ID3D11PixelShader,
    glyph_layout: ID3D11InputLayout,
    glyph_vs: ID3D11VertexShader,
    glyph_ps: ID3D11PixelShader,
    uniform_buffer: ID3D11Buffer,
    atlas_gray: AtlasTexture,
    atlas_rgba: AtlasTexture,
    quad_buffer: GrowBuffer,
    glyph_buffer: GrowBuffer,
    pub fonts: FontStack,
    raster: Rasterizer,
    cache: FxHashMap<GlyphKey, Cached>,
    /// Pictures by their key and size in pixels; `None` for one that could not be made.
    images: FxHashMap<(u64, u32), Option<Bitmap>>,
    quads: Vec<QuadInstance>,
    glyphs: Vec<GlyphInstance>,
    cmds: Vec<Cmd>,
    clips: Vec<[u32; 4]>,
    scale: f32,
    size: (u32, u32),
}

/// The families the UI text is set in, before the CJK, emoji and symbol fallbacks of the stack.
fn ui_fonts() -> FontConfig {
    let names = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect();
    FontConfig { text: names(&["Segoe UI Variable Text", "Segoe UI"]), ..FontConfig::default() }
}

impl Ui {
    pub fn new(gpu: &Gpu) -> Result<Self, String> {
        let quad_src = include_str!("quad.hlsl");
        let glyph_src = include_str!("glyph.hlsl");
        let quad_vs_code = compile(quad_src, "vs_main", "vs_5_0")?;
        let glyph_vs_code = compile(glyph_src, "vs_main", "vs_5_0")?;
        Ok(Ui {
            gpu: gpu.clone(),
            states: DrawStates::new(gpu)?,
            quad_layout: input_layout(gpu, &quad_vs_code, &QUAD_ATTRS)?,
            quad_vs: vertex_shader(gpu, &quad_vs_code)?,
            quad_ps: pixel_shader(gpu, &compile(quad_src, "fs_main", "ps_5_0")?)?,
            glyph_layout: input_layout(gpu, &glyph_vs_code, &GLYPH_ATTRS)?,
            glyph_vs: vertex_shader(gpu, &glyph_vs_code)?,
            glyph_ps: pixel_shader(gpu, &compile(glyph_src, "fs_main", "ps_5_0")?)?,
            uniform_buffer: constant_buffer(gpu, std::mem::size_of::<Uniforms>())?,
            atlas_gray: AtlasTexture::new(gpu, DXGI_FORMAT_R8_UNORM, 1, 512, 2048)?,
            atlas_rgba: AtlasTexture::new(gpu, DXGI_FORMAT_R8G8B8A8_UNORM, 4, 256, 1024)?,
            quad_buffer: GrowBuffer::new(gpu, std::mem::size_of::<QuadInstance>() as u32, 4096)?,
            glyph_buffer: GrowBuffer::new(gpu, std::mem::size_of::<GlyphInstance>() as u32, 4096)?,
            fonts: FontStack::new(ui_fonts())?,
            raster: Rasterizer::new(),
            cache: FxHashMap::default(),
            images: FxHashMap::default(),
            quads: Vec::new(),
            glyphs: Vec::new(),
            cmds: Vec::new(),
            clips: Vec::new(),
            scale: 1.0,
            size: (1, 1),
        })
    }

    /// Starts a frame on a target of `size` pixels; `scale` is pixels per DIP.
    pub fn begin_frame(&mut self, size: (u32, u32), scale: f32) {
        self.size = (size.0.max(1), size.1.max(1));
        self.scale = scale;
        self.quads.clear();
        self.glyphs.clear();
        self.cmds.clear();
        self.clips.clear();
    }

    // --- shapes -------------------------------------------------------------------------------------------------

    /// A rounded rectangle; `border` is the colour and width (DIP) of an inner outline.
    pub fn rrect(&mut self, r: Rect, radius: f32, fill: Color, border: Option<(Color, f32)>) {
        let s = self.scale;
        let (border_color, border_width) = border.map_or((Color::TRANSPARENT, 0.0), |(c, w)| (c, w * s));
        self.push_quad(QuadInstance {
            rect: [r.x * s, r.y * s, r.w * s, r.h * s],
            rb: [radius * s, border_width],
            fill: fill.0,
            border: border_color.0,
            extra: [0.0, 0.0],
        });
    }

    pub fn fill(&mut self, r: Rect, color: Color) {
        self.rrect(r, 0.0, color, None);
    }

    /// A soft shadow of a rounded rectangle, `blur` DIP wide on each side.
    pub fn shadow(&mut self, r: Rect, radius: f32, blur: f32, color: Color) {
        let s = self.scale;
        self.push_quad(QuadInstance {
            rect: [r.x * s, r.y * s, r.w * s, r.h * s],
            rb: [radius * s, 0.0],
            fill: color.0,
            border: [0; 4],
            extra: [blur * s, 1.0],
        });
    }

    pub fn push_clip(&mut self, r: Rect) {
        let s = self.scale;
        let (x0, y0) = ((r.x * s).floor().max(0.0) as u32, (r.y * s).floor().max(0.0) as u32);
        let (x1, y1) = (((r.x + r.w) * s).ceil().max(0.0) as u32, ((r.y + r.h) * s).ceil().max(0.0) as u32);
        let (x0, y0) = (x0.min(self.size.0), y0.min(self.size.1));
        let (x1, y1) = (x1.min(self.size.0).max(x0), y1.min(self.size.1).max(y0));
        let mut clip = [x0, y0, x1 - x0, y1 - y0];
        if let Some(outer) = self.clips.last() {
            let (ox1, oy1) = (outer[0] + outer[2], outer[1] + outer[3]);
            let (nx0, ny0) = (clip[0].max(outer[0]), clip[1].max(outer[1]));
            let (nx1, ny1) = ((clip[0] + clip[2]).min(ox1).max(nx0), (clip[1] + clip[3]).min(oy1).max(ny0));
            clip = [nx0, ny0, nx1 - nx0, ny1 - ny0];
        }
        self.clips.push(clip);
    }

    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn push_quad(&mut self, quad: QuadInstance) {
        let index = self.quads.len() as u32;
        self.quads.push(quad);
        self.extend_cmd(false, index);
    }

    fn extend_cmd(&mut self, glyphs: bool, index: u32) {
        let clip = self.clips.last().copied();
        match self.cmds.last_mut() {
            Some(cmd) if cmd.glyphs == glyphs && cmd.clip == clip => cmd.range.end = index + 1,
            _ => self.cmds.push(Cmd { glyphs, range: index..index + 1, clip }),
        }
    }

    // --- text ---------------------------------------------------------------------------------------------------

    fn choice_for(&mut self, ch: char, bold: bool) -> Option<FontChoice> {
        let class = CharClass { wide: is_cjk(ch), emoji_vs: false };
        self.fonts.choice_for(ch, Style { bold, italic: false }, class)
    }

    /// The cached advance and bitmap of `ch` at `px` pixels per em, rasterizing it when it is new. `hint` fits the outline to
    /// the pixel grid, which the icons need: their strokes are one pixel wide, and at 125 % the bar of minimize would fall
    /// between two rows of pixels and be drawn half bright in both.
    fn glyph(&mut self, choice: FontChoice, ch: char, px: f32, hint: bool) -> Cached {
        let face = self.fonts.face(choice.face);
        let glyph = face.glyph(ch);
        let key = GlyphKey {
            face: choice.face,
            glyph,
            px: px.to_bits(),
            wght: choice.wght.map_or(u32::MAX, f32::to_bits),
            synth: u8::from(choice.synth_bold) | (u8::from(choice.synth_italic) << 1) | (u8::from(hint) << 2),
        };
        if let Some(cached) = self.cache.get(&key) {
            return *cached;
        }
        let advance = face.font_ref().glyph_metrics(&[]).scale(px).advance_width(glyph);
        let raster = self.raster.rasterize(face, &choice, glyph, px, 0.0, RasterOptions { lcd: false, hint });
        let bitmap = raster.filter(|r| r.width > 0 && r.height > 0).and_then(|r| {
            let color = r.kind == GlyphKind::Color;
            let src = self.insert(color, r.width, r.height, &r.data)?;
            let kind = if color { KIND_COLOR } else { KIND_MASK };
            Some(Bitmap { src, size: [r.width as u16, r.height as u16], left: r.left, top: r.top, kind })
        });
        let cached = Cached { advance, bitmap };
        self.cache.insert(key, cached);
        cached
    }

    /// Places a bitmap in the colour or the gray atlas. A full atlas is cleared, with every cached glyph and picture that
    /// pointed into it, and the insert tried once more; `None` only for a bitmap larger than an empty atlas.
    fn insert(&mut self, color: bool, w: u32, h: u32, data: &[u8]) -> Option<[u16; 2]> {
        for retry in [false, true] {
            if retry {
                // shortcut: glyphs recorded earlier in this frame may show reused atlas slots until the next frame.
                self.atlas_gray.clear();
                self.atlas_rgba.clear();
                self.cache.clear();
                self.images.clear();
            }
            let atlas = if color { &mut self.atlas_rgba } else { &mut self.atlas_gray };
            if let Ok((x, y)) = atlas.insert(&self.gpu, w, h, data) {
                return Some([x, y]);
            }
        }
        None
    }

    fn primary_metrics(&mut self, px: f32) -> (f32, f32) {
        let Some(choice) = self.fonts.primary(Style::REGULAR) else { return (px, px * 1.3) };
        let m = self.fonts.face(choice.face).font_ref().metrics(&[]).scale(px);
        (m.ascent, m.ascent + m.descent + m.leading)
    }

    /// Height of a line of text of `size` DIP.
    pub fn line_height(&mut self, size: f32) -> f32 {
        let s = self.scale;
        self.primary_metrics(size * s).1 / s
    }

    /// Width in DIP of `text` set at `size` DIP.
    pub fn measure(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        let px = size * self.scale;
        let mut width = 0.0;
        for ch in text.chars().filter(|c| !c.is_control()) {
            if let Some(choice) = self.choice_for(ch, bold) {
                width += self.glyph(choice, ch, px, false).advance;
            }
        }
        width / self.scale
    }

    /// Width in DIP of an icon glyph set at `size` DIP.
    pub fn measure_icon(&mut self, ch: char, size: f32) -> f32 {
        let px = size * self.scale;
        match self.fonts.family_choice("Segoe Fluent Icons", Style::REGULAR) {
            Some(choice) => self.glyph(choice, ch, px, true).advance / self.scale,
            None => 0.0,
        }
    }

    /// Draws `text` with its line box starting at (`x`, `y`) DIP; returns its width in DIP.
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, color: Color, bold: bool) -> f32 {
        self.run(None, x, y, text, size, color, bold)
    }

    /// Draws a square colour picture `size` DIP wide with its top left at (`x`, `y`) DIP, on whole pixels of the frame being
    /// drawn. `key` names the picture; the first time it is drawn at a size in pixels, `pixels` makes it for that size,
    /// `px * px` premultiplied RGBA. False when it could not be made, and the caller draws something else.
    pub fn image(&mut self, key: u64, x: f32, y: f32, size: f32, pixels: impl FnOnce(u32) -> Option<Vec<u8>>) -> bool {
        let px = (size * self.scale).round().max(1.0) as u32;
        let bitmap = match self.images.get(&(key, px)) {
            Some(cached) => *cached,
            None => {
                let made = pixels(px).filter(|data| px > 0 && data.len() == (px * px * 4) as usize).and_then(|data| {
                    let src = self.insert(true, px, px, &data)?;
                    Some(Bitmap { src, size: [px as u16, px as u16], left: 0, top: 0, kind: KIND_COLOR })
                });
                self.images.insert((key, px), made);
                made
            }
        };
        let Some(b) = bitmap else { return false };
        let s = self.scale;
        let index = self.glyphs.len() as u32;
        self.glyphs.push(GlyphInstance { dst: [(x * s).round() as i32, (y * s).round() as i32], size: b.size, src: b.src, fg: [255; 4], kind: b.kind });
        self.extend_cmd(true, index);
        true
    }

    /// Draws one glyph of the Segoe Fluent Icons font (a code point such as `'\u{E713}'`).
    pub fn icon(&mut self, x: f32, y: f32, ch: char, size: f32, color: Color) -> f32 {
        self.run(Some("Segoe Fluent Icons"), x, y, &ch.to_string(), size, color, false)
    }

    fn run(&mut self, family: Option<&str>, x: f32, y: f32, text: &str, size: f32, color: Color, bold: bool) -> f32 {
        let s = self.scale;
        let px = size * s;
        let baseline = (y * s + self.primary_metrics(px).0).round() as i32;
        let family_choice = family.and_then(|f| self.fonts.family_choice(f, Style::REGULAR));
        let mut pen = x * s;
        for ch in text.chars().filter(|c| !c.is_control()) {
            let Some(choice) = family_choice.or_else(|| self.choice_for(ch, bold)) else { continue };
            let cached = self.glyph(choice, ch, px, family.is_some());
            if let Some(b) = cached.bitmap {
                let index = self.glyphs.len() as u32;
                self.glyphs.push(GlyphInstance {
                    dst: [pen.round() as i32 + b.left, baseline - b.top],
                    size: b.size,
                    src: b.src,
                    fg: color.0,
                    kind: b.kind,
                });
                self.extend_cmd(true, index);
            }
            pen += cached.advance;
        }
        (pen - x * s) / s
    }

    // --- drawing ------------------------------------------------------------------------------------------------

    /// Draws everything recorded since `begin_frame` over `target` (loaded, not cleared).
    pub fn end_frame(&mut self, target: &ID3D11RenderTargetView) {
        if self.cmds.is_empty() {
            return;
        }
        let gpu = &self.gpu;
        self.atlas_gray.flush(gpu);
        self.atlas_rgba.flush(gpu);
        let uniforms = Uniforms { viewport: [self.size.0 as f32, self.size.1 as f32], pad: [0.0; 2] };
        write_buffer(gpu, &self.uniform_buffer, bytemuck::bytes_of(&uniforms));
        self.quad_buffer.write(gpu, bytemuck::cast_slice(&self.quads), self.quads.len() as u32);
        self.glyph_buffer.write(gpu, bytemuck::cast_slice(&self.glyphs), self.glyphs.len() as u32);

        gpu.bind_target(target, [0, 0, self.size.0, self.size.1]);
        gpu.begin_draws(&self.states);
        self.states.set_blend(gpu, false);
        bind_uniforms(gpu, &self.uniform_buffer);
        bind_textures(gpu, &[&self.atlas_gray.view, &self.atlas_rgba.view]);
        for cmd in &self.cmds {
            let [x, y, w, h] = cmd.clip.unwrap_or([0, 0, self.size.0, self.size.1]);
            if w == 0 || h == 0 {
                continue;
            }
            gpu.set_scissor([x, y, w, h]);
            if cmd.glyphs {
                set_shaders(gpu, &self.glyph_layout, &self.glyph_vs, &self.glyph_ps);
                gpu.draw_instances_from(&self.glyph_buffer.buffer, self.glyph_buffer.stride, cmd.range.clone());
            } else {
                set_shaders(gpu, &self.quad_layout, &self.quad_vs, &self.quad_ps);
                gpu.draw_instances_from(&self.quad_buffer.buffer, self.quad_buffer.stride, cmd.range.clone());
            }
        }
    }
}

#[cfg(test)]
mod shader_tests {
    use nekominal_render::d3d::compile;

    /// The shaders are compiled when the window starts, so a mistake in one would show only then: this makes it show in the
    /// tests.
    #[test]
    fn every_entry_of_the_interface_shaders_compiles() {
        let quad = include_str!("quad.hlsl");
        let glyph = include_str!("glyph.hlsl");
        for (source, entry, target) in [(quad, "vs_main", "vs_5_0"), (glyph, "vs_main", "vs_5_0"), (quad, "fs_main", "ps_5_0"), (glyph, "fs_main", "ps_5_0")] {
            let code = compile(source, entry, target).unwrap_or_else(|e| panic!("{e}"));
            assert!(code.starts_with(b"DXBC"), "{entry} did not give D3D bytecode");
        }
    }
}
