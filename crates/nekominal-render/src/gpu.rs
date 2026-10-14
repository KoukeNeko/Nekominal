//! GPU-side types: instance layouts, uniforms, pipelines, growable buffers.

use bytemuck::{Pod, Zeroable};

/// One solid rectangle in pane pixels. 20 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct RectInstance {
    /// x, y, width, height in pixels.
    pub rect: [f32; 4],
    /// Straight (non-premultiplied) RGBA8.
    pub color: [u8; 4],
}

/// One glyph bitmap placed 1:1 on the pixel grid. 24 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GlyphInstance {
    /// Top-left in pane pixels (may be negative for overhang).
    pub dst: [i32; 2],
    /// Bitmap size in texels (== pixels drawn).
    pub size: [u16; 2],
    /// Top-left in the atlas, texels.
    pub src: [u16; 2],
    pub fg: [u8; 4],
    /// 0 = R8 mask, 1 = RGBA LCD mask, 2 = RGBA premultiplied color.
    pub kind: u32,
}

pub const KIND_MASK: u32 = 0;
pub const KIND_LCD: u32 = 1;
pub const KIND_COLOR: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Uniforms {
    pub viewport: [f32; 2],
    /// x: grayscale enhanced contrast, y: LCD enhanced contrast.
    pub contrast: [f32; 2],
    pub gamma_ratios: [f32; 4],
    /// bit 0: apply contrast + gamma correction to text coverage.
    pub flags: u32,
    /// Extra enhanced contrast for grayscale text, added for every foreground colour. Non-zero only while
    /// the pane background is translucent (see `RendererConfig::translucent_text_boost`).
    pub text_boost: f32,
    pub pad: [u32; 2],
}

/// DirectWrite-style gamma ratios (Windows Terminal `atlas/dwrite.cpp`, MIT).
/// `gamma` is clamped to 1.0..=2.2 in 0.1 steps; DirectWrite's default is 1.8.
pub fn gamma_ratios(gamma: f32) -> [f32; 4] {
    #[rustfmt::skip]
    const TABLE: [[f32; 4]; 13] = [
        [0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0],
        [0.0166 / 4.0, -0.0807 / 4.0, 0.2227 / 4.0, -0.0751 / 4.0],
        [0.0350 / 4.0, -0.1760 / 4.0, 0.4325 / 4.0, -0.1370 / 4.0],
        [0.0543 / 4.0, -0.2821 / 4.0, 0.6302 / 4.0, -0.1876 / 4.0],
        [0.0739 / 4.0, -0.3963 / 4.0, 0.8167 / 4.0, -0.2287 / 4.0],
        [0.0933 / 4.0, -0.5161 / 4.0, 0.9926 / 4.0, -0.2616 / 4.0],
        [0.1121 / 4.0, -0.6395 / 4.0, 1.1588 / 4.0, -0.2877 / 4.0],
        [0.1300 / 4.0, -0.7649 / 4.0, 1.3159 / 4.0, -0.3080 / 4.0],
        [0.1469 / 4.0, -0.8911 / 4.0, 1.4644 / 4.0, -0.3234 / 4.0],
        [0.1627 / 4.0, -1.0170 / 4.0, 1.6051 / 4.0, -0.3347 / 4.0],
        [0.1773 / 4.0, -1.1420 / 4.0, 1.7385 / 4.0, -0.3426 / 4.0],
        [0.1908 / 4.0, -1.2652 / 4.0, 1.8650 / 4.0, -0.3476 / 4.0],
        [0.2031 / 4.0, -1.3864 / 4.0, 1.9851 / 4.0, -0.3501 / 4.0],
    ];
    const NORM13: f32 = ((0x10000 as f64) / (255.0 * 255.0) * 4.0) as f32;
    const NORM24: f32 = ((0x100 as f64) / 255.0 * 4.0) as f32;
    let index = ((gamma * 10.0).round() as usize).clamp(10, 22) - 10;
    let r = TABLE[index];
    [r[0] * NORM13, r[1] * NORM24, r[2] * NORM13, r[3] * NORM24]
}

/// Buffer that is rewritten wholesale each time and only reallocated when the
/// payload outgrows it (power-of-two growth).
pub struct GrowBuffer {
    pub buffer: wgpu::Buffer,
    capacity: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
    pub count: u32,
}

impl GrowBuffer {
    pub fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages, capacity: u64) -> Self {
        let capacity = capacity.max(256).next_power_of_two();
        GrowBuffer { buffer: Self::alloc(device, label, usage, capacity), capacity, usage, label, count: 0 }
    }

    fn alloc(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8], count: u32) {
        self.count = count;
        if bytes.is_empty() {
            return;
        }
        if bytes.len() as u64 > self.capacity {
            self.capacity = (bytes.len() as u64).next_power_of_two();
            self.buffer = Self::alloc(device, self.label, self.usage, self.capacity);
        }
        queue.write_buffer(&self.buffer, 0, bytes);
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }
}

pub const RECT_ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x4, 1 => Unorm8x4];
pub const GLYPH_ATTRS: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![0 => Sint32x2, 1 => Uint16x2, 2 => Uint16x2, 3 => Unorm8x4, 4 => Uint32];

pub struct Pipelines {
    pub uniform_layout: wgpu::BindGroupLayout,
    pub atlas_layout: wgpu::BindGroupLayout,
    pub rect: wgpu::RenderPipeline,
    pub glyph: wgpu::RenderPipeline,
    /// Present only when the device supports dual-source blending.
    pub glyph_lcd: Option<wgpu::RenderPipeline>,
}

impl Pipelines {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, dual_source: bool) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nk uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
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
            label: Some("nk atlases"),
            entries: &[tex_entry(0), tex_entry(1)],
        });
        let rect_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nk rect layout"),
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });
        let glyph_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nk glyph layout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });

        let rect_src = include_str!("shaders/rect.wgsl");
        let glyph_src = include_str!("shaders/glyph.wgsl");
        let rect_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nk rect"),
            source: wgpu::ShaderSource::Wgsl(rect_src.into()),
        });
        let glyph_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nk glyph"),
            source: wgpu::ShaderSource::Wgsl(glyph_src.into()),
        });

        let premul = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let make = |label: &str,
                    layout: &wgpu::PipelineLayout,
                    module: &wgpu::ShaderModule,
                    fs: &str,
                    stride: u64,
                    attrs: &'static [wgpu::VertexAttribute],
                    blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: attrs,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let rect = make("nk rect", &rect_layout, &rect_module, "fs_main", 20, &RECT_ATTRS, premul);
        let glyph = make("nk glyph", &glyph_layout, &glyph_module, "fs_main", 24, &GLYPH_ATTRS, premul);

        let glyph_lcd = dual_source.then(|| {
            let src =
                format!("enable dual_source_blending;\n{}\n{}", glyph_src, include_str!("shaders/glyph_lcd_fs.wgsl"));
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("nk glyph lcd"),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let blend = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Src1,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            };
            make("nk glyph lcd", &glyph_layout, &module, "fs_lcd", 24, &GLYPH_ATTRS, Some(blend))
        });

        Pipelines { uniform_layout, atlas_layout, rect, glyph, glyph_lcd }
    }
}
