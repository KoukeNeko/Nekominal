//! Presentation-path probe: DX12 swap chains created by wgpu 30 (HWND flip model and
//! DirectComposition visual / "SwapChainPanel-style" composition), configure/resize,
//! present modes, premultiplied alpha, and IDXGISwapChain2::SetMatrixTransform.
//!
//! A real WinUI 3 `SwapChainPanel` cannot be created headlessly. wgpu-hal creates the
//! swap chain for it with the same `IDXGIFactory2::CreateSwapChainForComposition` call
//! as the DirectComposition visual path exercised here; the only difference is the final
//! `ISwapChainPanelNative::SetSwapChain` vs `IDCompositionVisual::SetContent`.

use std::ffi::c_void;
use std::num::NonZeroIsize;
use std::time::{Duration, Instant};

use nekominal_render::*;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dxgi::{DXGI_MATRIX_3X2_F, IDXGISwapChain2};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCWSTR, w};

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn make_window(w_px: i32, h_px: i32, ex_style: WINDOW_EX_STYLE) -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(None).expect("module handle");
        let class = w!("NkPresentProbe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            ex_style,
            class,
            PCWSTR::null(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            w_px,
            h_px,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .expect("window")
    }
}

fn raw_target(hwnd: HWND) -> wgpu::SurfaceTargetUnsafe {
    let win = wgpu::rwh::Win32WindowHandle::new(NonZeroIsize::new(hwnd.0 as isize).unwrap());
    wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::Windows(wgpu::rwh::WindowsDisplayHandle::new())),
        raw_window_handle: wgpu::rwh::RawWindowHandle::Win32(win),
    }
}

/// Compile-time proof of the WinUI 3 hosting entry point (not runnable headlessly):
/// `panel` is the `ISwapChainPanelNative*` obtained from the XAML `SwapChainPanel`.
#[allow(dead_code)]
unsafe fn surface_for_swap_chain_panel(instance: &wgpu::Instance, panel: *mut c_void) -> wgpu::Surface<'static> {
    unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::SwapChainPanel(panel)).expect("panel surface") }
}

struct Probe {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wgpu::Adapter,
    renderer: Renderer,
    pane: PaneState,
    grid: Vec<Cell>,
    cols: usize,
    rows: usize,
}

impl Probe {
    fn frame(&mut self, surface: &wgpu::Surface<'_>, clear: wgpu::Color) -> Option<(Duration, Duration)> {
        let t0 = Instant::now();
        let tex = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                println!("    get_current_texture -> {:?}", std::mem::discriminant(&other));
                return None;
            }
        };
        let acquire = t0.elapsed();
        let view = tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (w, h) = (tex.texture.width(), tex.texture.height());
        let gv = GridView::plain(self.cols, self.rows, &self.grid);
        self.renderer.update_pane(&mut self.pane, &gv);
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        self.renderer.draw(&mut enc, &view, Some(clear), &[PaneDraw { pane: &self.pane, rect: [0, 0, w, h] }], None);
        self.queue.submit([enc.finish()]);
        let t1 = Instant::now();
        self.queue.present(tex);
        Some((acquire, t1.elapsed()))
    }
}

fn describe_caps(caps: &wgpu::SurfaceCapabilities) {
    println!("    formats: {:?}", caps.formats);
    println!("    present modes: {:?}", caps.present_modes);
    println!("    alpha modes: {:?}", caps.alpha_modes);
    println!("    usages: {:?}", caps.usages);
}

fn run(kind: wgpu::Dx12SwapchainKind, label: &str, alpha: wgpu::CompositeAlphaMode) {
    println!("\n=== {label} ({kind:?}) ===");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        backend_options: wgpu::BackendOptions {
            dx12: wgpu::Dx12BackendOptions { presentation_system: kind, ..Default::default() },
            ..Default::default()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let ex =
        if kind == wgpu::Dx12SwapchainKind::DxgiFromVisual { WS_EX_NOREDIRECTIONBITMAP } else { WINDOW_EX_STYLE(0) };
    let hwnd = make_window(800, 600, ex);
    let surface = unsafe { instance.create_surface_unsafe(raw_target(hwnd)) }.expect("create_surface_unsafe");

    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
    let adapter = adapters
        .into_iter()
        .find(|a| {
            a.get_info().device_type == wgpu::DeviceType::IntegratedGpu
                || a.get_info().device_type == wgpu::DeviceType::DiscreteGpu
        })
        .expect("gpu adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: Renderer::wanted_features(&adapter),
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .expect("device");
    println!("  adapter {}", adapter.get_info().name);
    let caps = surface.get_capabilities(&adapter);
    describe_caps(&caps);

    let format = wgpu::TextureFormat::Bgra8Unorm; // sRGB-encoded values written as-is; NOT the *Srgb variant
    let mut cfg = surface.get_default_config(&adapter, 800, 600).expect("default config");
    cfg.format = format;
    cfg.present_mode = wgpu::PresentMode::Fifo;
    cfg.desired_maximum_frame_latency = 2;
    cfg.alpha_mode = alpha;
    let t = Instant::now();
    surface.configure(&device, &cfg);
    println!("  configure 800x600 {:?} {:?}: {:?}", cfg.present_mode, cfg.alpha_mode, t.elapsed());

    let mut theme = Theme::campbell();
    if alpha == wgpu::CompositeAlphaMode::PreMultiplied {
        theme.background_opacity = 0.6;
    }
    let renderer = Renderer::new(
        &device,
        &queue,
        format,
        RendererConfig { lcd: alpha == wgpu::CompositeAlphaMode::Opaque, ..Default::default() },
    )
    .expect("renderer");
    let mut probe =
        Probe { device, queue, adapter, renderer, pane: PaneState::new(), grid: Vec::new(), cols: 0, rows: 0 };
    probe.renderer.set_theme(&theme);
    let m = probe.renderer.metrics();
    let fill = |probe: &mut Probe, w: u32, h: u32| {
        probe.cols = (w / m.width) as usize;
        probe.rows = (h / m.height) as usize;
        probe.grid = vec![Cell::default(); probe.cols * probe.rows];
        let text = "Nekominal swap chain probe 繁體中文 ┌──┐ 😀";
        for (i, ch) in text.chars().enumerate() {
            if i < probe.cols {
                probe.grid[probe.cols + i] = Cell::new(ch);
            }
        }
    };
    fill(&mut probe, 800, 600);
    let clear = wgpu::Color {
        r: 0.05 * theme.background_opacity as f64,
        g: 0.05 * theme.background_opacity as f64,
        b: 0.05 * theme.background_opacity as f64,
        a: theme.background_opacity as f64,
    };

    // Frame pacing under Fifo.
    let mut intervals = Vec::new();
    let mut presents = Vec::new();
    let mut last = Instant::now();
    for _ in 0..90 {
        if let Some((_acq, pres)) = probe.frame(&surface, clear) {
            presents.push(pres.as_secs_f64() * 1e3);
        }
        let now = Instant::now();
        intervals.push((now - last).as_secs_f64() * 1e3);
        last = now;
    }
    intervals.drain(..10);
    intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    presents.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "  Fifo frame interval over 80 frames: median {:.2} ms, p95 {:.2} ms, min {:.2}, max {:.2} | present() call median {:.2} ms",
        intervals[intervals.len() / 2],
        intervals[(intervals.len() as f64 * 0.95) as usize],
        intervals[0],
        intervals[intervals.len() - 1],
        presents[presents.len() / 2]
    );

    // Resize sequence: reconfigure + redraw.
    for (w, h) in [(1000u32, 700u32), (640, 480), (1920, 1080), (800, 600)] {
        cfg.width = w;
        cfg.height = h;
        let t = Instant::now();
        surface.configure(&probe.device, &cfg);
        let reconf = t.elapsed();
        fill(&mut probe, w, h);
        let ok = probe.frame(&surface, clear).is_some();
        println!("  resize -> {w}x{h}: configure {:?}, frame ok={ok}", reconf);
    }

    // Other present modes.
    for mode in [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate] {
        if caps.present_modes.contains(&mode) {
            cfg.present_mode = mode;
            surface.configure(&probe.device, &cfg);
            let mut iv = Vec::new();
            let mut last = Instant::now();
            for _ in 0..60 {
                probe.frame(&surface, clear);
                let n = Instant::now();
                iv.push((n - last).as_secs_f64() * 1e3);
                last = n;
            }
            iv.drain(..10);
            iv.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!("  {:?}: frame interval median {:.2} ms", mode, iv[iv.len() / 2]);
        }
    }
    cfg.present_mode = wgpu::PresentMode::Fifo;
    surface.configure(&probe.device, &cfg);

    // DPI compensation: the host (not wgpu) must call SetMatrixTransform on composition swap chains.
    let sc = unsafe { surface.as_hal::<wgpu::hal::api::Dx12>() }.and_then(|s| s.swap_chain());
    match sc {
        Some(sc3) => {
            let sc2: &IDXGISwapChain2 = &sc3;
            let m = DXGI_MATRIX_3X2_F { _11: 0.8, _12: 0.0, _21: 0.0, _22: 0.8, _31: 0.0, _32: 0.0 };
            let r = unsafe { sc2.SetMatrixTransform(&m) };
            println!("  IDXGISwapChain2::SetMatrixTransform(0.8 scale) -> {:?}", r.map_err(|e| e.code()));
            let _ = sc3.cast::<IDXGISwapChain2>();
        }
        None => println!("  swap_chain() not available"),
    }
    let _ = &probe.adapter;
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn main() {
    run(wgpu::Dx12SwapchainKind::DxgiFromHwnd, "HWND flip-model swap chain, opaque", wgpu::CompositeAlphaMode::Opaque);
    run(
        wgpu::Dx12SwapchainKind::DxgiFromVisual,
        "DirectComposition visual (same call path as SwapChainPanel), OPAQUE (what WinUI 3 SwapChainPanel supports)",
        wgpu::CompositeAlphaMode::Opaque,
    );
    run(
        wgpu::Dx12SwapchainKind::DxgiFromVisual,
        "DirectComposition visual (same call path as SwapChainPanel), premultiplied alpha",
        wgpu::CompositeAlphaMode::PreMultiplied,
    );
}
