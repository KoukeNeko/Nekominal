//! The device and the surface of a window: wgpu on a Wayland or X11 window. The surface is premultiplied alpha when the
//! compositor takes it, so that a translucent theme shows what is behind the window.

pub use nekominal_render::gfx::Gpu;
use nekominal_render::gfx::{AdapterKind, RenderTarget, TARGET_FORMAT};

use crate::win::{Hwnd, window_of};

pub struct Surface {
    gpu: Gpu,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    frame: Option<wgpu::SurfaceTexture>,
    /// Frames shown so far, for `NEKOMINAL_SCREENSHOT`.
    shown: u32,
}

/// With `NEKOMINAL_SCREENSHOT` set to a file name, the window writes the frame it shows after this many frames to that
/// file as a PNG and goes on: a way to look at the window where the desktop gives a program no screenshots of its own.
const SCREENSHOT_FRAME: u32 = 20;

/// The device, and a surface of `size` pixels for `hwnd`.
pub fn create(hwnd: Hwnd, size: (u32, u32)) -> Result<(Gpu, Surface), String> {
    let window = window_of(hwnd).ok_or("the window is gone")?;
    let (gpu, surface) = Gpu::with_surface(AdapterKind::Real, |instance| {
        instance.create_surface(window).map(Some).map_err(|e| format!("create_surface: {e}"))
    })?;
    crate::util::log(&format!("adapter: {}", gpu.adapter_name));
    let surface = surface.ok_or("no surface")?;
    Ok((gpu.clone(), configure(gpu, surface, size)?))
}

/// A device for the window `hwnd` is on.
pub fn device_for(hwnd: Hwnd) -> Result<Gpu, String> {
    create(hwnd, (1, 1)).map(|(gpu, _)| gpu)
}

/// A surface of `size` pixels for another window, on the device the first window made.
pub fn create_surface(gpu: &Gpu, hwnd: Hwnd, size: (u32, u32)) -> Result<Surface, String> {
    let window = window_of(hwnd).ok_or("the window is gone")?;
    let surface = gpu.instance().create_surface(window).map_err(|e| format!("create_surface: {e}"))?;
    configure(gpu.clone(), surface, size)
}

fn configure(gpu: Gpu, surface: wgpu::Surface<'static>, size: (u32, u32)) -> Result<Surface, String> {
    let caps = surface.get_capabilities(gpu.adapter());
    if !caps.formats.contains(&TARGET_FORMAT) {
        return Err(format!("the surface does not take {TARGET_FORMAT:?} (it takes {:?})", caps.formats));
    }
    let alpha_mode = [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::Inherit]
        .into_iter()
        .find(|mode| caps.alpha_modes.contains(mode))
        .unwrap_or(wgpu::CompositeAlphaMode::Auto);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        format: TARGET_FORMAT,
        width: size.0.max(1),
        height: size.1.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
        color_space: Default::default(),
    };
    surface.configure(gpu.device(), &config);
    Ok(Surface { gpu, surface, config, frame: None, shown: 0 })
}

impl Surface {
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if size == self.size() {
            return;
        }
        self.frame = None;
        self.config.width = size.0;
        self.config.height = size.1;
        self.surface.configure(gpu.device(), &self.config);
    }

    /// The target to draw the next frame into; `None` when the surface is not usable now.
    pub fn acquire(&mut self) -> Option<RenderTarget> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // The surface changed under the program (a resize, a move to another output): configured again, the next try works.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(self.gpu.device(), &self.config);
                return None;
            }
            // Hidden or minimized, or no frame in time: the caller asks again at once, which would spin this thread.
            _ => {
                std::thread::sleep(std::time::Duration::from_millis(16));
                return None;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.frame = Some(frame);
        Some(view)
    }

    /// Shows the frame. True when the device is gone: everything made on it has to be made again on a new one.
    pub fn present(&mut self) -> bool {
        self.gpu.flush();
        if let Some(frame) = self.frame.take() {
            self.shown += 1;
            if self.shown == SCREENSHOT_FRAME && let Some(path) = std::env::var_os("NEKOMINAL_SCREENSHOT") {
                if let Err(e) = self.screenshot(&frame.texture, std::path::Path::new(&path)) {
                    crate::util::log(&format!("screenshot: {e}"));
                }
            }
            self.gpu.queue().present(frame);
        }
        self.gpu.is_lost()
    }

    /// A window smaller than its surface shows the part this brings under it. Not needed here: the surface is the size of
    /// the window.
    pub fn set_offset(&mut self, _x: f32, _y: f32) {}

    /// Lets go of the window, so that a new surface can be made for it.
    pub fn detach(&mut self) {
        self.frame = None;
    }
}

impl Surface {
    /// Writes `texture` (a frame of this surface) to `path` as a PNG.
    fn screenshot(&self, texture: &wgpu::Texture, path: &std::path::Path) -> Result<(), String> {
        let (width, height) = (texture.width(), texture.height());
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let device = self.gpu.device();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("screenshot") });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.gpu.queue().submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("poll: {e:?}"))?;
        receiver.recv().map_err(|e| e.to_string())?.map_err(|e| format!("map: {e:?}"))?;
        let mapped = buffer.slice(..).get_mapped_range().map_err(|e| format!("mapped range: {e:?}"))?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height as usize {
            for px in mapped[y * row as usize..][..width as usize * 4].chunks_exact(4) {
                rgba.extend([px[2], px[1], px[0], 255]);
            }
        }
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().and_then(|mut writer| writer.write_image_data(&rgba)).map_err(|e| e.to_string())
    }
}
