//! A `SwapChainPanel` bound to wgpu (DX12). Knows nothing about terminals: the caller draws into the
//! texture view it is handed.
//!
//! Recipe (winui3-rust.md section 5, renderer.md section 18, re-verified by the vertical slice):
//! * `ISwapChainPanelNative` is cast from the panel and given to wgpu directly (no shim);
//! * the first `configure` creates the composition swap chain on the UI thread;
//! * `IDXGISwapChain2::SetMatrixTransform(1/scale)` after the first configure and on every scale change;
//!   it persists across `ResizeBuffers` (measured), re-applying is optional (`MatrixMode::Always`);
//! * target format `Bgra8Unorm`, never the `*Srgb` format wgpu lists first;
//! * one device recipe: renderer-wanted features, `MemoryHints::MemoryUsage`;
//! * `premultiplied` selects `PreMultiplied` alpha: the swap chain then composites over the window's system
//!   backdrop (Mica / Acrylic) but not over XAML elements behind the panel. Opaque content in a
//!   premultiplied swap chain is simply alpha 1, so an app that can change its opacity at runtime keeps
//!   this on for every panel (the alpha mode cannot change after the swap chain exists);
//! * one `Instance` and one `Device` for the whole process (critique.md trap 30: creating or destroying several
//!   DX12 devices in one process crashes intermittently). Every panel gets its own surface on that device;
//!   [`PanelSurface::suspend`] releases a hidden panel's swap chain and the next `on_metrics` recreates it.

use std::sync::OnceLock;
use std::time::Instant;

use nekominal_winui_sys::*;
use windows::Win32::Graphics::Dxgi::{DXGI_MATRIX_3X2_F, IDXGISwapChain2};
use windows::core::Interface as _;
use windows_core::Interface as _;

use crate::diag::log;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatrixMode {
    /// Re-apply after every configure.
    Always,
    /// Apply after the first configure and on scale changes only.
    Once,
    Never,
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceOptions {
    /// Composite as premultiplied alpha (translucent terminal over the window backdrop).
    pub premultiplied: bool,
    pub matrix: MatrixMode,
    /// Features the renderer wants when the adapter offers them.
    pub wanted_features: wgpu::Features,
}

impl Default for SurfaceOptions {
    fn default() -> Self {
        Self { premultiplied: false, matrix: MatrixMode::Always, wanted_features: wgpu::Features::DUAL_SOURCE_BLENDING }
    }
}

/// Size and scale of the panel after a `SizeChanged` / `CompositionScaleChanged`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelMetrics {
    pub size_px: (u32, u32),
    pub scale: (f32, f32),
    pub size_changed: bool,
    pub scale_changed: bool,
    /// The surface was (re)created by this call: everything drawn into the old one is gone.
    pub first: bool,
}

pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Presented {
    Drawn,
    /// The swap chain was reconfigured; the caller should draw again.
    Reconfigured,
    Skipped,
}

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
static GPU: OnceLock<Gpu> = OnceLock::new();

fn instance() -> &'static wgpu::Instance {
    INSTANCE.get_or_init(|| {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::DX12;
        wgpu::Instance::new(desc)
    })
}

/// The process-wide device, created on first use for an adapter that can present to `surface`.
fn shared_gpu(surface: &wgpu::Surface<'_>, wanted: wgpu::Features) -> Result<&'static Gpu, String> {
    if let Some(gpu) = GPU.get() {
        return Ok(gpu);
    }
    let t0 = Instant::now();
    let adapter = pollster::block_on(instance().request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: Some(surface),
        apply_limit_buckets: false,
    }))
    .map_err(|e| format!("request_adapter: {e}"))?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    }))
    .map_err(|e| format!("request_device: {e}"))?;
    log(&format!(
        "adapter {} {:?} driver={} features={:?} (+{:.0} ms)",
        info.name,
        info.backend,
        info.driver,
        device.features(),
        t0.elapsed().as_secs_f64() * 1000.0
    ));
    Ok(GPU.get_or_init(|| Gpu { adapter, device, queue }))
}

pub struct PanelSurface {
    panel: SwapChainPanel,
    opts: SurfaceOptions,
    gpu: Option<&'static Gpu>,
    surface: Option<wgpu::Surface<'static>>,
    size_px: (u32, u32),
    scale: (f32, f32),
}

impl PanelSurface {
    pub fn new(panel: SwapChainPanel, opts: SurfaceOptions) -> Self {
        Self { panel, opts, gpu: None, surface: None, size_px: (0, 0), scale: (1.0, 1.0) }
    }

    pub fn gpu(&self) -> Option<&'static Gpu> {
        self.gpu
    }

    pub fn size_px(&self) -> (u32, u32) {
        self.size_px
    }

    pub fn scale(&self) -> (f32, f32) {
        self.scale
    }

    pub fn is_active(&self) -> bool {
        self.surface.is_some()
    }

    /// Releases the swap chain of a panel that is not visible. The next [`PanelSurface::on_metrics`] with a
    /// non-empty size creates it again (and reports `first`).
    pub fn suspend(&mut self) {
        if self.surface.take().is_some() {
            self.size_px = (0, 0);
            log("surface suspended");
        }
    }

    /// Feed `SizeChanged` (DIP size) and `CompositionScaleChanged` here. Creates the surface on the first
    /// non-empty size. `Ok(None)` while the panel has no size.
    pub fn on_metrics(&mut self, width_dip: f64, height_dip: f64) -> Result<Option<PanelMetrics>, String> {
        let (sx, sy) = (self.panel.CompositionScaleX().unwrap_or(1.0), self.panel.CompositionScaleY().unwrap_or(1.0));
        let px = ((width_dip * f64::from(sx)).round() as u32, (height_dip * f64::from(sy)).round() as u32);
        if px.0 == 0 || px.1 == 0 {
            return Ok(None);
        }
        let first = self.surface.is_none();
        let scale_changed = (sx, sy) != self.scale;
        let size_changed = px != self.size_px;
        self.scale = (sx, sy);
        if first {
            self.init_surface(px)?;
        } else if size_changed {
            self.configure(px);
        }
        if first || scale_changed || self.opts.matrix == MatrixMode::Always {
            self.apply_matrix();
        }
        Ok(Some(PanelMetrics {
            size_px: px,
            scale: (sx, sy),
            size_changed: size_changed || first,
            scale_changed,
            first,
        }))
    }

    fn init_surface(&mut self, px: (u32, u32)) -> Result<(), String> {
        let native: ISwapChainPanelNative =
            self.panel.cast().map_err(|e| format!("cast ISwapChainPanelNative: {e}"))?;
        // SAFETY: `native` is the live ISwapChainPanelNative of the XAML panel; wgpu AddRef()s it.
        let surface =
            unsafe { instance().create_surface_unsafe(wgpu::SurfaceTargetUnsafe::SwapChainPanel(native.as_raw())) }
                .map_err(|e| format!("create_surface_unsafe: {e}"))?;
        self.gpu = Some(shared_gpu(&surface, self.opts.wanted_features)?);
        self.surface = Some(surface);
        self.configure(px);
        Ok(())
    }

    fn configure(&mut self, px: (u32, u32)) {
        let (Some(gpu), Some(surface)) = (self.gpu, self.surface.as_ref()) else { return };
        let Some(mut config) = surface.get_default_config(&gpu.adapter, px.0, px.1) else { return };
        config.format = FORMAT;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        config.alpha_mode = if self.opts.premultiplied {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            wgpu::CompositeAlphaMode::Opaque
        };
        let t = Instant::now();
        surface.configure(&gpu.device, &config);
        self.size_px = px;
        log(&format!(
            "configured {}x{} {:?} {:?} in {:.1} ms",
            px.0,
            px.1,
            config.format,
            config.alpha_mode,
            t.elapsed().as_secs_f64() * 1000.0
        ));
    }

    fn apply_matrix(&mut self) {
        if self.opts.matrix == MatrixMode::Never {
            return;
        }
        let Some(surface) = self.surface.as_ref() else { return };
        // SAFETY: only used to read the swap chain pointer out of the hal surface.
        let Some(hal) = (unsafe { surface.as_hal::<wgpu::hal::api::Dx12>() }) else { return };
        let Some(swap_chain) = hal.swap_chain() else { return };
        let result = swap_chain.cast::<IDXGISwapChain2>().and_then(|sc2| {
            let m = DXGI_MATRIX_3X2_F {
                _11: 1.0 / self.scale.0,
                _12: 0.0,
                _21: 0.0,
                _22: 1.0 / self.scale.1,
                _31: 0.0,
                _32: 0.0,
            };
            unsafe { sc2.SetMatrixTransform(&m) }
        });
        match result {
            Ok(()) => log(&format!("SetMatrixTransform(1/{}) ok", self.scale.0)),
            Err(e) => log(&format!("SetMatrixTransform failed: {e}")),
        }
    }

    /// Acquires the next back buffer, lets `draw` record into it and presents. Handles the wgpu 30
    /// `CurrentSurfaceTexture` outcomes: `Outdated` reconfigures, everything else skips the frame.
    pub fn present(
        &mut self,
        draw: impl FnOnce(&wgpu::Device, &wgpu::Queue, &wgpu::TextureView, (u32, u32)),
    ) -> Presented {
        let (Some(surface), Some(gpu)) = (self.surface.as_ref(), self.gpu) else { return Presented::Skipped };
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                let size = (frame.texture.width(), frame.texture.height());
                let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                draw(&gpu.device, &gpu.queue, &view, size);
                gpu.queue.present(frame);
                Presented::Drawn
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                let px = self.size_px;
                self.configure(px);
                if self.opts.matrix == MatrixMode::Always {
                    self.apply_matrix();
                }
                Presented::Reconfigured
            }
            other => {
                log(&format!("get_current_texture -> {:?}", std::mem::discriminant(&other)));
                Presented::Skipped
            }
        }
    }
}
