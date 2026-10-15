//! The device and the swap chain of a window: Direct3D 11 on a DirectComposition visual, premultiplied alpha, so the
//! DWM backdrop (Mica or acrylic) shows through wherever the shell draws with alpha below one.

pub use nekominal_render::d3d::Gpu;
use nekominal_render::d3d::AdapterKind;
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D11::{ID3D11RenderTargetView, ID3D11Texture2D},
            DirectComposition::{DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual},
            Dxgi::{
                Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC},
                DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice,
                IDXGIFactory2, IDXGISwapChain1,
            },
        },
    },
    core::Interface,
};

const BUFFERS: u32 = 2;

pub struct Surface {
    swap: IDXGISwapChain1,
    target: Option<ID3D11RenderTargetView>,
    size: (u32, u32),
    dcomp: IDCompositionDevice,
    // Dropping the target or the visual takes the content off the window.
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    committed: bool,
}

/// The device, and a swap chain of `size` pixels for `hwnd`. The adapter is the low-power one (the integrated GPU
/// where there are two): a terminal does not need the other, and it costs the battery.
pub fn create(hwnd: HWND, size: (u32, u32)) -> Result<(Gpu, Surface), String> {
    let gpu = Gpu::new(AdapterKind::Real)?;
    let surface = create_surface(&gpu, hwnd, size)?;
    Ok((gpu, surface))
}

/// A swap chain of `size` pixels for another window, on the device the first window made.
pub fn create_surface(gpu: &Gpu, hwnd: HWND, size: (u32, u32)) -> Result<Surface, String> {
    let size = (size.0.max(1), size.1.max(1));
    // SAFETY: the DirectComposition objects are created for a window of this thread and kept in `Surface` for as long
    // as the swap chain lives.
    unsafe {
        let dxgi: IDXGIDevice = gpu.device.cast().map_err(|e| format!("IDXGIDevice: {e}"))?;
        let factory: IDXGIFactory2 =
            dxgi.GetAdapter().and_then(|a| a.GetParent()).map_err(|e| format!("IDXGIFactory2: {e}"))?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: size.0,
            Height: size.1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: BUFFERS,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };
        let swap = factory
            .CreateSwapChainForComposition(&gpu.device, &desc, None)
            .map_err(|e| format!("CreateSwapChainForComposition: {e}"))?;
        let dcomp: IDCompositionDevice =
            DCompositionCreateDevice(&dxgi).map_err(|e| format!("DCompositionCreateDevice: {e}"))?;
        let target = dcomp.CreateTargetForHwnd(hwnd, true).map_err(|e| format!("CreateTargetForHwnd: {e}"))?;
        let visual = dcomp.CreateVisual().map_err(|e| format!("CreateVisual: {e}"))?;
        visual.SetContent(&swap).map_err(|e| format!("SetContent: {e}"))?;
        target.SetRoot(&visual).map_err(|e| format!("SetRoot: {e}"))?;
        let mut surface = Surface { swap, target: None, size, dcomp, _target: target, _visual: visual, committed: false };
        surface.target = Some(surface.create_target(gpu)?);
        Ok(surface)
    }
}

impl Surface {
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    fn create_target(&self, gpu: &Gpu) -> Result<ID3D11RenderTargetView, String> {
        // SAFETY: the back buffer and the device live through the calls.
        unsafe {
            let back: ID3D11Texture2D = self.swap.GetBuffer(0).map_err(|e| format!("GetBuffer: {e}"))?;
            let mut view = None;
            gpu.device.CreateRenderTargetView(&back, None, Some(&mut view)).map_err(|e| format!("CreateRenderTargetView: {e}"))?;
            view.ok_or_else(|| "no render target view".into())
        }
    }

    pub fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if size == self.size {
            return;
        }
        // Every reference to the back buffers has to go before they can be resized.
        self.target = None;
        // SAFETY: nothing is bound any more; the swap chain lives through the call.
        unsafe {
            gpu.context.OMSetRenderTargets(None, None);
            gpu.context.Flush();
            if self.swap.ResizeBuffers(BUFFERS, size.0, size.1, DXGI_FORMAT_UNKNOWN, Default::default()).is_err() {
                return;
            }
        }
        self.size = size;
        self.target = self.create_target(gpu).ok();
    }

    /// The target to draw the next frame into; `None` when the swap chain is not usable.
    pub fn acquire(&mut self) -> Option<ID3D11RenderTargetView> {
        self.target.clone()
    }

    pub fn present(&mut self) {
        // SAFETY: the swap chain is alive. A removed device shows up here; the window then keeps its last frame.
        unsafe { self.swap.Present(1, Default::default()).ok().ok() };
        if !self.committed {
            // The first Commit shows the swap chain the visual was given.
            // SAFETY: the DirectComposition device is alive.
            unsafe { self.dcomp.Commit().ok() };
            self.committed = true;
        }
    }
}
