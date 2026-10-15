//! The self-drawn UI layer: rounded rectangles, shadows, text and icon glyphs recorded in DIP and drawn with wgpu in
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
    fonts::{CharClass, FaceId, FontChoice, FontConfig, FontStack, Style, is_cjk},
    gpu::{GLYPH_ATTRS, GlyphInstance, GrowBuffer, KIND_COLOR, KIND_MASK},
    raster::{GlyphKind, RasterOptions, Rasterizer},
};
use rustc_hash::FxHashMap;

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

const QUAD_ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x4, 1 => Float32x2, 2 => Unorm8x4, 3 => Unorm8x4, 4 => Float32x2
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
    device: wgpu::Device,
    queue: wgpu::Queue,
    quad_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    atlas_layout: wgpu::BindGroupLayout,
    atlas_gray: AtlasTexture,
    atlas_rgba: AtlasTexture,
    atlas_group: Option<(wgpu::BindGroup, u32, u32)>,
    quad_buffer: GrowBuffer,
    glyph_buffer: GrowBuffer,
    pub fonts: FontStack,
    raster: Rasterizer,
    cache: FxHashMap<GlyphKey, Cached>,
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
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Result<Self, String> {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui atlases"),
            entries: &[texture(0), texture(1)],
        });
        let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui quad layout"),
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });
        let glyph_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui glyph layout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });
        let module = |label, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source.into()) })
        };
        let quad_module = module("ui quad", include_str!("quad.wgsl"));
        let glyph_module = module("ui glyph", include_str!("glyph.wgsl"));
        let pipeline = |label, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, stride, attrs: &'static [wgpu::VertexAttribute]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Instance, attributes: attrs })],
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let quad_pipeline = pipeline("ui quad", &quad_layout, &quad_module, std::mem::size_of::<QuadInstance>() as u64, &QUAD_ATTRS);
        let glyph_pipeline = pipeline("ui glyph", &glyph_layout, &glyph_module, std::mem::size_of::<GlyphInstance>() as u64, &GLYPH_ATTRS);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui uniforms"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });
        let vertex = wgpu::BufferUsages::VERTEX;
        Ok(Ui {
            device: device.clone(),
            queue: queue.clone(),
            quad_pipeline,
            glyph_pipeline,
            uniform_buffer,
            uniform_group,
            atlas_layout,
            atlas_gray: AtlasTexture::new(device, wgpu::TextureFormat::R8Unorm, 1, 512, 2048, "ui atlas r8"),
            atlas_rgba: AtlasTexture::new(device, wgpu::TextureFormat::Rgba8Unorm, 4, 256, 1024, "ui atlas rgba"),
            atlas_group: None,
            quad_buffer: GrowBuffer::new(device, "ui quads", vertex, 4096),
            glyph_buffer: GrowBuffer::new(device, "ui glyphs", vertex, 4096),
            fonts: FontStack::new(ui_fonts())?,
            raster: Rasterizer::new(),
            cache: FxHashMap::default(),
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

    /// The cached advance and bitmap of `ch` at `px` pixels per em, rasterizing it when it is new.
    fn glyph(&mut self, choice: FontChoice, ch: char, px: f32) -> Cached {
        let face = self.fonts.face(choice.face);
        let glyph = face.glyph(ch);
        let key = GlyphKey {
            face: choice.face,
            glyph,
            px: px.to_bits(),
            wght: choice.wght.map_or(u32::MAX, f32::to_bits),
            synth: u8::from(choice.synth_bold) | (u8::from(choice.synth_italic) << 1),
        };
        if let Some(cached) = self.cache.get(&key) {
            return *cached;
        }
        let advance = face.font_ref().glyph_metrics(&[]).scale(px).advance_width(glyph);
        let bitmap = self
            .raster
            .rasterize(face, &choice, glyph, px, 0.0, RasterOptions { lcd: false, hint: false })
            .filter(|r| r.width > 0 && r.height > 0)
            .and_then(|r| {
                let atlas = if r.kind == GlyphKind::Color { &mut self.atlas_rgba } else { &mut self.atlas_gray };
                let src = atlas.insert(&self.device, &self.queue, r.width, r.height, &r.data).ok()?;
                let kind = if r.kind == GlyphKind::Color { KIND_COLOR } else { KIND_MASK };
                Some(Bitmap { src: [src.0, src.1], size: [r.width as u16, r.height as u16], left: r.left, top: r.top, kind })
            });
        let cached = Cached { advance, bitmap };
        self.cache.insert(key, cached);
        cached
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
                width += self.glyph(choice, ch, px).advance;
            }
        }
        width / self.scale
    }

    /// Draws `text` with its line box starting at (`x`, `y`) DIP; returns its width in DIP.
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, color: Color, bold: bool) -> f32 {
        self.run(None, x, y, text, size, color, bold)
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
            let cached = self.glyph(choice, ch, px);
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

    fn ensure_atlas_group(&mut self) {
        let (g, c) = (self.atlas_gray.generation, self.atlas_rgba.generation);
        if matches!(&self.atlas_group, Some((_, gg, cc)) if (*gg, *cc) == (g, c)) {
            return;
        }
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui atlases"),
            layout: &self.atlas_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.atlas_gray.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.atlas_rgba.view) },
            ],
        });
        self.atlas_group = Some((group, g, c));
    }

    /// Draws everything recorded since `begin_frame` over `target` (loaded, not cleared).
    pub fn end_frame(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        if self.cmds.is_empty() {
            return;
        }
        self.atlas_gray.flush(&self.queue);
        self.atlas_rgba.flush(&self.queue);
        self.ensure_atlas_group();
        let uniforms = Uniforms { viewport: [self.size.0 as f32, self.size.1 as f32], pad: [0.0; 2] };
        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        self.quad_buffer.write(&self.device, &self.queue, bytemuck::cast_slice(&self.quads), self.quads.len() as u32);
        self.glyph_buffer.write(&self.device, &self.queue, bytemuck::cast_slice(&self.glyphs), self.glyphs.len() as u32);

        let atlas = &self.atlas_group.as_ref().expect("atlas group").0;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.uniform_group, &[]);
        for cmd in &self.cmds {
            let [x, y, w, h] = cmd.clip.unwrap_or([0, 0, self.size.0, self.size.1]);
            if w == 0 || h == 0 {
                continue;
            }
            pass.set_scissor_rect(x, y, w, h);
            if cmd.glyphs {
                pass.set_pipeline(&self.glyph_pipeline);
                pass.set_bind_group(1, atlas, &[]);
                pass.set_vertex_buffer(0, self.glyph_buffer.buffer.slice(..));
            } else {
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_vertex_buffer(0, self.quad_buffer.buffer.slice(..));
            }
            pass.draw(0..4, cmd.range.clone());
        }
    }
}
