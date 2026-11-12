//! The wgpu side of the renderer (`gfx` on Linux): the device and its queue, and the small helpers (buffers, textures,
//! programs, draw state) the renderer and the UI layer are built from, with the names of the Direct3D 11 side (`d3d.rs`) so
//! the code above this layer is the same on both. One `Gpu` is used from one thread.
//!
//! Direct3D 11 draws as the calls come; wgpu records a pass. The calls here are recorded into one command encoder and one
//! render pass at a time, started by the first `bind_target` and ended when the target changes or on [`Gpu::flush`] (the
//! presenting of a frame does that), which hands them to the queue. Data written meanwhile (`write_buffer`,
//! `upload_texels`) reaches the GPU before every command of that submission: a buffer is written once in a frame, as the
//! renderer and the UI layer do.

use std::{
    ops::Range,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::gpu::{Attribute, BufferKind, Format, TexelFormat};

/// What a pass draws into.
pub type RenderTarget = wgpu::TextureView;
pub type Buffer = wgpu::Buffer;
pub type Texture = wgpu::Texture;
/// A texture as the shaders read it.
pub type TextureView = wgpu::TextureView;

/// The format of every target: the swap chain of a window, an offscreen texture. The pipelines are made for it.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdapterKind {
    /// The low-power hardware adapter (the integrated GPU where there are two).
    Real,
    /// The adapter of a monitor: there is no telling which one drives it here, so the low-power one.
    Display(isize),
    /// A software rasterizer (llvmpipe, lavapipe).
    Warp,
}

#[derive(Clone)]
pub struct Gpu {
    shared: Arc<Shared>,
    pub adapter_name: String,
}

struct Shared {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    dual_source: bool,
    lost: Arc<AtomicBool>,
    uniform_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    /// Stands in for a texture the program does not read: every binding of the layout needs one.
    blank: wgpu::TextureView,
    frame: Mutex<Frame>,
}

/// What has been recorded since the last submit and what is bound for the next draw.
#[derive(Default)]
struct Frame {
    encoder: Option<wgpu::CommandEncoder>,
    pass: Option<ActivePass>,
    program: Option<wgpu::RenderPipeline>,
    uniforms: Option<wgpu::Buffer>,
    uniform_group: Option<wgpu::BindGroup>,
    atlases: [Option<wgpu::TextureView>; 2],
    picture: Option<wgpu::TextureView>,
    texture_group: Option<wgpu::BindGroup>,
}

struct ActivePass {
    pass: wgpu::RenderPass<'static>,
    target: wgpu::TextureView,
    size: (u32, u32),
}

/// The PCI ids of the graphics card the computer started its display on: with two cards the compositor draws on that one,
/// and a window drawn on the other would be copied across for every frame (or refused by the compositor).
fn boot_gpu() -> Option<(u32, u32)> {
    let hex = |path: std::path::PathBuf| u32::from_str_radix(std::fs::read_to_string(path).ok()?.trim().trim_start_matches("0x"), 16).ok();
    std::fs::read_dir("/sys/class/drm").ok()?.flatten().find_map(|card| {
        let device = card.path().join("device");
        (std::fs::read_to_string(device.join("boot_vga")).ok()?.trim() == "1").then(|| Some((hex(device.join("vendor"))?, hex(device.join("device"))?)))?
    })
}

/// The adapter to draw with: the one `NEKOMINAL_ADAPTER` names (a part of its name), the card the display started on, else
/// the low-power one; a software rasterizer for `AdapterKind::Warp`.
fn pick_adapter(instance: &wgpu::Instance, kind: AdapterKind, surface: Option<&wgpu::Surface<'static>>) -> Result<wgpu::Adapter, String> {
    if kind != AdapterKind::Warp {
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
        let presents = |adapter: &wgpu::Adapter| surface.is_none_or(|surface| !surface.get_capabilities(adapter).formats.is_empty());
        let named = std::env::var("NEKOMINAL_ADAPTER").ok().filter(|name| !name.is_empty()).map(|name| name.to_lowercase());
        let boot = boot_gpu();
        let chosen = adapters.iter().find(|adapter| {
            let info = adapter.get_info();
            info.device_type != wgpu::DeviceType::Cpu
                && presents(adapter)
                && match &named {
                    Some(name) => info.name.to_lowercase().contains(name),
                    None => boot.is_some_and(|(vendor, device)| info.vendor == vendor && info.device == device),
                }
        });
        if let Some(adapter) = chosen {
            return Ok(adapter.clone());
        }
    }
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: kind == AdapterKind::Warp,
        compatible_surface: surface,
        ..Default::default()
    }))
    .map_err(|e| gpu_error("no adapter", e))
}

fn gpu_error(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

impl Gpu {
    /// A device with no window, for tests and tools.
    pub fn new(kind: AdapterKind) -> Result<Gpu, String> {
        Gpu::with_surface(kind, |_| Ok(None)).map(|(gpu, _)| gpu)
    }

    /// A device that can present on the surface `make_surface` makes from the instance it is given (a surface must come from
    /// the instance of its device).
    pub fn with_surface(
        kind: AdapterKind,
        make_surface: impl FnOnce(&wgpu::Instance) -> Result<Option<wgpu::Surface<'static>>, String>,
    ) -> Result<(Gpu, Option<wgpu::Surface<'static>>), String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = make_surface(&instance)?;
        let adapter = pick_adapter(&instance, kind, surface.as_ref())?;
        let info = adapter.get_info();
        let dual_source = adapter.features().contains(wgpu::Features::DUAL_SOURCE_BLENDING);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nekominal"),
            required_features: if dual_source { wgpu::Features::DUAL_SOURCE_BLENDING } else { wgpu::Features::empty() },
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| gpu_error("request_device", e))?;
        let lost = Arc::new(AtomicBool::new(false));
        {
            let lost = lost.clone();
            device.set_device_lost_callback(move |_, _| lost.store(true, Ordering::Relaxed));
        }
        device.on_uncaptured_error(Arc::new(|error| eprintln!("wgpu: {error}")));

        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let texture = |binding| {
            entry(
                binding,
                wgpu::ShaderStages::FRAGMENT,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )
        };
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniforms"),
            entries: &[entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            )],
        });
        // The two atlases, a picture and the sampler of the picture.
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("textures"),
            entries: &[
                texture(0),
                texture(1),
                texture(2),
                entry(3, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nekominal"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("picture"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("blank"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let shared = Shared {
            instance,
            adapter,
            device,
            queue,
            dual_source,
            lost,
            uniform_layout,
            texture_layout,
            pipeline_layout,
            sampler,
            blank,
            frame: Mutex::new(Frame::default()),
        };
        Ok((Gpu { shared: Arc::new(shared), adapter_name: info.name }, surface))
    }

    pub fn instance(&self) -> &wgpu::Instance {
        &self.shared.instance
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.shared.adapter
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.shared.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.shared.queue
    }

    /// Whether the device blends with a second pixel shader output (LCD text).
    pub fn dual_source_supported(&self) -> bool {
        self.shared.dual_source
    }

    /// Whether the device was lost (a driver reset, the GPU removed): everything made on it has to be made again.
    pub fn is_lost(&self) -> bool {
        self.shared.lost.load(Ordering::Relaxed)
    }

    fn frame(&self) -> MutexGuard<'_, Frame> {
        self.shared.frame.lock().unwrap()
    }

    /// Ends the pass and hands the commands recorded so far to the queue.
    pub fn flush(&self) {
        let mut frame = self.frame();
        frame.pass = None;
        if let Some(encoder) = frame.encoder.take() {
            self.shared.queue.submit([encoder.finish()]);
        }
    }

    /// Starts a pass over `target` unless the one under way is over it and `load` keeps what it holds.
    fn begin_pass(&self, frame: &mut Frame, target: &RenderTarget, load: wgpu::LoadOp<wgpu::Color>) {
        if matches!(load, wgpu::LoadOp::Load)
            && let Some(active) = &frame.pass
            && active.target == *target
        {
            return;
        }
        frame.pass = None;
        let device = &self.shared.device;
        let encoder = frame.encoder.get_or_insert_with(|| device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") }));
        let pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        let texture = target.texture();
        // A new pass binds nothing: the state of the last one is not inherited.
        frame.program = None;
        frame.uniform_group = None;
        frame.texture_group = None;
        frame.pass = Some(ActivePass { pass, target: target.clone(), size: (texture.width(), texture.height()) });
    }

    /// Makes `target` the render target, with the viewport and the scissor rectangle both set to `rect`
    /// (x, y, width, height in pixels).
    pub fn bind_target(&self, target: &RenderTarget, rect: [u32; 4]) {
        let mut frame = self.frame();
        self.begin_pass(&mut frame, target, wgpu::LoadOp::Load);
        let active = frame.pass.as_mut().expect("a pass was begun");
        let [x, y, w, h] = clamp(rect, active.size);
        if w > 0 && h > 0 {
            active.pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
            active.pass.set_scissor_rect(x, y, w, h);
        }
    }

    pub fn set_scissor(&self, rect: [u32; 4]) {
        let mut frame = self.frame();
        let Some(active) = frame.pass.as_mut() else { return };
        let [x, y, w, h] = clamp(rect, active.size);
        if w > 0 && h > 0 {
            active.pass.set_scissor_rect(x, y, w, h);
        }
    }

    /// Clears the target, whatever was drawn into it before.
    pub fn clear(&self, target: &RenderTarget, color: [f32; 4]) {
        let mut frame = self.frame();
        let [r, g, b, a] = color.map(f64::from);
        self.begin_pass(&mut frame, target, wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }));
    }

    /// The state every draw of the renderer shares: triangle strips of four vertices, one instance per record.
    pub fn begin_draws(&self, _state: &DrawStates) {}

    /// Draws `count` instances of a quad, the instance records coming from `buffer`.
    pub fn draw_instances(&self, buffer: &Buffer, stride: u32, count: u32) {
        self.draw_instances_from(buffer, stride, 0..count);
    }

    /// Draws the instances `range` of `buffer`.
    pub fn draw_instances_from(&self, buffer: &Buffer, _stride: u32, range: Range<u32>) {
        let mut frame = self.frame();
        let frame = &mut *frame;
        let (Some(active), Some(program), Some(uniforms)) = (frame.pass.as_mut(), frame.program.as_ref(), frame.uniforms.as_ref()) else {
            return;
        };
        let shared = &self.shared;
        let uniform_group = frame.uniform_group.get_or_insert_with(|| {
            shared.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("uniforms"),
                layout: &shared.uniform_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
            })
        });
        let texture_group = frame.texture_group.get_or_insert_with(|| {
            let view = |view: &Option<wgpu::TextureView>| view.clone().unwrap_or_else(|| shared.blank.clone());
            let (first, second, picture) = (view(&frame.atlases[0]), view(&frame.atlases[1]), view(&frame.picture));
            shared.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("textures"),
                layout: &shared.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&first) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&second) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&picture) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&shared.sampler) },
                ],
            })
        });
        let pass = &mut active.pass;
        pass.set_pipeline(program);
        pass.set_bind_group(0, &*uniform_group, &[]);
        pass.set_bind_group(1, &*texture_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..4, range);
    }
}

/// `rect` inside a target of `size`.
fn clamp(rect: [u32; 4], size: (u32, u32)) -> [u32; 4] {
    let [x, y, w, h] = rect;
    let (x, y) = (x.min(size.0), y.min(size.1));
    [x, y, w.min(size.0 - x), h.min(size.1 - y)]
}

/// Blending lives in the program here (see [`Shader::dual_source`]); the type stays for the renderer, which sets it.
pub struct DrawStates;

impl DrawStates {
    pub fn new(_gpu: &Gpu) -> Result<DrawStates, String> {
        Ok(DrawStates)
    }

    pub fn set_blend(&self, _gpu: &Gpu, _dual_source: bool) {}
}

/// The WGSL of a program: the sources joined (the LCD entry is appended to the glyph shader), and the entry points.
#[derive(Clone, Copy)]
pub struct Shader {
    pub sources: &'static [&'static str],
    pub vs: &'static str,
    pub ps: &'static str,
    /// The pixel shader gives per-channel coverage as a second output that the blend takes (LCD text).
    pub dual_source: bool,
    /// The pixel shader's alpha is how much of what is under it is taken away (round corners of a translucent window).
    pub erase: bool,
}

/// A render pipeline: a vertex and a pixel shader, the layout of the instance records that feed them, and the blend.
pub struct Program {
    pipeline: wgpu::RenderPipeline,
}

fn vertex_format(format: Format) -> wgpu::VertexFormat {
    match format {
        Format::Float32x4 => wgpu::VertexFormat::Float32x4,
        Format::Float32x2 => wgpu::VertexFormat::Float32x2,
        Format::Sint32x2 => wgpu::VertexFormat::Sint32x2,
        Format::Uint16x2 => wgpu::VertexFormat::Uint16x2,
        Format::Unorm8x4 => wgpu::VertexFormat::Unorm8x4,
        Format::Uint32 => wgpu::VertexFormat::Uint32,
    }
}

impl Program {
    /// `attributes` are the inputs `@location(0)`, `@location(1)`, ... of the vertex shader, in order, one instance per
    /// record of `stride` bytes.
    pub fn new(gpu: &Gpu, shader: &Shader, attributes: &[Attribute], stride: u32) -> Result<Program, String> {
        let device = &gpu.shared.device;
        let mut source = String::new();
        if shader.dual_source {
            source.push_str("enable dual_source_blending;\n");
        }
        for part in shader.sources {
            source.push_str(part);
            source.push('\n');
        }
        let attributes: Vec<wgpu::VertexAttribute> = attributes
            .iter()
            .enumerate()
            .map(|(location, Attribute(format, offset))| wgpu::VertexAttribute {
                format: vertex_format(*format),
                offset: u64::from(*offset),
                shader_location: location as u32,
            })
            .collect();
        let blend = if shader.erase {
            let take_away = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
            wgpu::BlendState { color: take_away, alpha: take_away }
        } else if shader.dual_source {
            wgpu::BlendState {
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
            }
        } else {
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(shader.ps), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(shader.ps),
            layout: Some(&gpu.shared.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(shader.vs),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: u64::from(stride),
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(shader.ps),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: TARGET_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        match pollster::block_on(scope.pop()) {
            Some(error) => Err(gpu_error(shader.ps, error)),
            None => Ok(Program { pipeline }),
        }
    }
}

pub fn set_program(gpu: &Gpu, program: &Program) {
    gpu.frame().program = Some(program.pipeline.clone());
}

/// A buffer the CPU rewrites wholesale.
pub fn dynamic_buffer(gpu: &Gpu, kind: BufferKind, size: u64) -> Result<Buffer, String> {
    let usage = match kind {
        BufferKind::Vertex => wgpu::BufferUsages::VERTEX,
        BufferKind::Constant => wgpu::BufferUsages::UNIFORM,
    };
    Ok(gpu.shared.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: size.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }))
}

pub fn constant_buffer(gpu: &Gpu, size: usize) -> Result<Buffer, String> {
    dynamic_buffer(gpu, BufferKind::Constant, size.next_multiple_of(16) as u64)
}

/// Replaces the start of a dynamic buffer (`bytes` must not be longer than the buffer).
pub fn write_buffer(gpu: &Gpu, buffer: &Buffer, bytes: &[u8]) {
    gpu.shared.queue.write_buffer(buffer, 0, bytes);
}

pub fn bind_uniforms(gpu: &Gpu, buffer: &Buffer) {
    let mut frame = gpu.frame();
    if frame.uniforms.as_ref() != Some(buffer) {
        frame.uniforms = Some(buffer.clone());
        frame.uniform_group = None;
    }
}

fn texel_format(format: TexelFormat) -> wgpu::TextureFormat {
    match format {
        TexelFormat::R8 => wgpu::TextureFormat::R8Unorm,
        TexelFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
    }
}

fn sampled_texture(gpu: &Gpu, format: wgpu::TextureFormat, width: u32, height: u32) -> Texture {
    gpu.shared.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// A 2D texture of `size` x `size` texels that is only sampled, with a view of it.
pub fn atlas_texture(gpu: &Gpu, format: TexelFormat, size: u32) -> Result<(Texture, TextureView), String> {
    let texture = sampled_texture(gpu, texel_format(format), size, size);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok((texture, view))
}

/// A picture's texture: `width` x `height` RGBA8 texels, `pixels` being their rows without padding.
pub fn picture_texture(gpu: &Gpu, width: u32, height: u32, pixels: &[u8]) -> Option<TextureView> {
    let texture = sampled_texture(gpu, wgpu::TextureFormat::Rgba8Unorm, width, height);
    upload_texels(gpu, &texture, pixels, width * 4, (0, 0, width, height));
    Some(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Binds a picture's texture, to be drawn with the bilinear sampler.
pub fn bind_picture(gpu: &Gpu, view: &TextureView) {
    let mut frame = gpu.frame();
    frame.picture = Some(view.clone());
    frame.texture_group = None;
}

/// Writes `pixels` (rows of `pitch` bytes, starting at the box's corner) into the box `rect` (x0, y0, x1, y1, exclusive
/// max) of `texture`.
pub fn upload_texels(gpu: &Gpu, texture: &Texture, pixels: &[u8], pitch: u32, rect: (u32, u32, u32, u32)) {
    let (x0, y0, x1, y1) = rect;
    assert!(pixels.len() >= (pitch * (y1 - y0)) as usize, "the pixels cover the box");
    gpu.shared.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x: x0, y: y0, z: 0 }, aspect: wgpu::TextureAspect::All },
        pixels,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(pitch), rows_per_image: Some(y1 - y0) },
        wgpu::Extent3d { width: x1 - x0, height: y1 - y0, depth_or_array_layers: 1 },
    );
}

/// Copies the whole of `from` into the top-left corner of the larger `to`. Submitted at once: the uploads into `from` are
/// before it in the queue, and the uploads into `to` that follow must not be overwritten by a copy that runs after them.
pub fn copy_texture(gpu: &Gpu, to: &Texture, from: &Texture) {
    let mut encoder = gpu.shared.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grow") });
    encoder.copy_texture_to_texture(
        from.as_image_copy(),
        to.as_image_copy(),
        wgpu::Extent3d { width: from.width(), height: from.height(), depth_or_array_layers: 1 },
    );
    gpu.shared.queue.submit([encoder.finish()]);
}

/// Binds the atlases the glyph programs read.
pub fn bind_textures(gpu: &Gpu, views: &[&TextureView]) {
    let mut frame = gpu.frame();
    for (slot, view) in frame.atlases.iter_mut().zip(views) {
        *slot = Some((*view).clone());
    }
    frame.texture_group = None;
}
