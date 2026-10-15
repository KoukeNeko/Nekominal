//! The device and the swap chain of a window: wgpu on a DirectComposition visual, premultiplied alpha, so the DWM
//! backdrop (Mica or acrylic) shows through wherever the shell draws with alpha below one.

use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            DirectComposition::{DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual},
            Dxgi::IDXGIDevice,
        },
    },
    core::Interface,
};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub struct Gpu {
    _instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

pub struct Surface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    dcomp: IDCompositionDevice,
    // Dropping the target or the visual takes the content off the window.
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    committed: bool,
}

/// The device and a swap chain of `size` pixels for `hwnd`. The adapter is the low-power one (the integrated GPU
/// where there are two): a terminal does not need the other, and it costs the battery.
pub fn create(hwnd: HWND, size: (u32, u32)) -> Result<(Gpu, Surface), String> {
    // SAFETY: the DirectComposition objects are created for a window of this thread and kept in `Surface` for as long
    // as the swap chain lives; wgpu AddRef()s the visual it is given.
    unsafe {
        let dcomp: IDCompositionDevice =
            DCompositionCreateDevice(None::<&IDXGIDevice>).map_err(|e| format!("DCompositionCreateDevice: {e}"))?;
        let target = dcomp.CreateTargetForHwnd(hwnd, true).map_err(|e| format!("CreateTargetForHwnd: {e}"))?;
        let visual = dcomp.CreateVisual().map_err(|e| format!("CreateVisual: {e}"))?;
        target.SetRoot(&visual).map_err(|e| format!("SetRoot: {e}"))?;

        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(visual.as_raw()))
            .map_err(|e| format!("create_surface_unsafe: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("request_adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: nekominal_render::Renderer::wanted_features(&adapter),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .map_err(|e| format!("request_device: {e}"))?;
        let mut config = surface
            .get_default_config(&adapter, size.0.max(1), size.1.max(1))
            .ok_or("the surface has no default configuration")?;
        config.format = FORMAT;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
        surface.configure(&device, &config);
        Ok((
            Gpu { _instance: instance, adapter, device, queue },
            Surface { surface, config, dcomp, _target: target, _visual: visual, committed: false },
        ))
    }
}

impl Surface {
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if size != self.size() {
            self.config.width = size.0;
            self.config.height = size.1;
            self.surface.configure(&gpu.device, &self.config);
        }
    }

    /// The texture to draw the next frame into; `None` when the swap chain had to be reconfigured or is not ready.
    pub fn acquire(&mut self, gpu: &Gpu) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&gpu.device, &self.config);
                None
            }
            _ => None,
        }
    }

    pub fn present(&mut self, gpu: &Gpu, frame: wgpu::SurfaceTexture) {
        gpu.queue.present(frame);
        if !self.committed {
            // The first Commit shows the swap chain the visual was given.
            // SAFETY: the DirectComposition device is alive.
            unsafe { self.dcomp.Commit().ok() };
            self.committed = true;
        }
    }
}
