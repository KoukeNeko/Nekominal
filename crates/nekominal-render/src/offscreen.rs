//! Headless GPU context, offscreen render target, readback and PNG output.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdapterKind {
    /// First non-CPU DX12 adapter (integrated/discrete GPU).
    Real,
    /// Microsoft Basic Render Driver (software rasterizer).
    Warp,
}

pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub timestamp_period: f32,
    pub timings: Vec<(&'static str, std::time::Duration)>,
}

pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

pub fn list_adapters(instance: &wgpu::Instance) -> Vec<wgpu::AdapterInfo> {
    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12)).iter().map(|a| a.get_info()).collect()
}

pub fn create_gpu(kind: AdapterKind) -> Result<GpuContext, String> {
    let mut timings = Vec::new();
    let t = std::time::Instant::now();
    let instance = instance();
    timings.push(("wgpu Instance::new (DX12)", t.elapsed()));
    let t = std::time::Instant::now();
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
    timings.push(("enumerate_adapters", t.elapsed()));
    let adapter = match kind {
        AdapterKind::Real => adapters
            .into_iter()
            .find(|a| {
                a.get_info().device_type != wgpu::DeviceType::Cpu && a.get_info().device_type != wgpu::DeviceType::Other
            })
            .ok_or("no hardware DX12 adapter")?,
        AdapterKind::Warp => adapters
            .into_iter()
            .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
            .ok_or("WARP adapter not found")?,
    };
    let info = adapter.get_info();
    let features = crate::renderer::Renderer::wanted_features(&adapter);
    let t = std::time::Instant::now();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nk device"),
        required_features: features,
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .map_err(|e| format!("request_device: {e}"))?;
    timings.push(("request_device", t.elapsed()));
    let timestamp_period = queue.get_timestamp_period();
    Ok(GpuContext { instance, adapter, device, queue, info, timestamp_period, timings })
}

pub struct OffscreenTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    readback: wgpu::Buffer,
    padded_row: u32,
}

impl OffscreenTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nk offscreen"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row = (width * 4).div_ceil(align) * align;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nk readback"),
            size: padded_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        OffscreenTarget { texture, view, width, height, format, readback, padded_row }
    }

    pub fn encode_readback(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
    }

    /// Map the readback buffer and return tightly packed RGBA8 (converted from BGRA when needed).
    pub fn read_rgba(&self, device: &wgpu::Device) -> Result<Vec<u8>, String> {
        let slice = self.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("poll: {e:?}"))?;
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("map: {e:?}"))?;
        let data = slice.get_mapped_range().map_err(|e| format!("mapped range: {e:?}"))?;
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height as usize {
            let row = &data[y * self.padded_row as usize..y * self.padded_row as usize + self.width as usize * 4];
            out.extend_from_slice(row);
        }
        drop(data);
        self.readback.unmap();
        if matches!(self.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            for px in out.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
        }
        Ok(out)
    }
}

pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())
}
