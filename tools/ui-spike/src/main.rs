//! Spike for the self-drawn UI: a plain Win32 window whose content is a wgpu swap chain on a DirectComposition visual,
//! premultiplied alpha, over a DWM system backdrop (Mica, Mica Alt or acrylic). No WinUI, no Windows App Runtime.
//!
//!   ui-spike [mica|mica-alt|acrylic|none] [--exit-after SECONDS]
//!
//! It prints the memory of the process once the first frames are presented, so the number can be compared with the
//! WinUI shell (about 180 MB private idle).

use std::{cell::RefCell, time::Instant};

use windows::{
    core::{w, Interface},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::{
            DirectComposition::{DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual},
            Dwm::{
                DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE,
                DWMWA_USE_IMMERSIVE_DARK_MODE, DWM_SYSTEMBACKDROP_TYPE,
            },
            Dxgi::IDXGIDevice,
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::GetCurrentProcess,
        },
        UI::{Controls::MARGINS, WindowsAndMessaging::*},
    },
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const TIMER_REPORT: usize = 1;
const TIMER_EXIT: usize = 2;

struct State {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    dcomp: IDCompositionDevice,
    // Dropping the target or the visual removes the content from the window.
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    frames: u32,
    started: Instant,
}

thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

fn memory_mb() -> (f64, f64) {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // SAFETY: the counters struct is sized as declared and lives through the call.
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), counters.cb);
    }
    (counters.WorkingSetSize as f64 / 1_048_576.0, counters.PrivateUsage as f64 / 1_048_576.0)
}

fn render() {
    STATE.with(|cell| {
        let mut guard = cell.borrow_mut();
        let Some(state) = guard.as_mut() else { return };
        match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = state.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                // Premultiplied: a dark blue at 55 % alpha, so the backdrop shows through.
                let a = 0.55;
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.10 * a, g: 0.12 * a, b: 0.25 * a, a }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                state.queue.submit([encoder.finish()]);
                state.queue.present(frame);
                state.frames += 1;
                if state.frames == 1 {
                    // SAFETY: the DirectComposition device is alive; the first Commit shows the new content.
                    unsafe { state.dcomp.Commit().ok() };
                    println!("first frame after {:.0} ms", state.started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            wgpu::CurrentSurfaceTexture::Outdated => state.surface.configure(&state.device, &state.config),
            _ => {}
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            let (w, h) = ((lparam.0 & 0xFFFF) as u32, ((lparam.0 >> 16) & 0xFFFF) as u32);
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() && w > 0 && h > 0 {
                    state.config.width = w;
                    state.config.height = h;
                    state.surface.configure(&state.device, &state.config);
                }
            });
            render();
            LRESULT(0)
        }
        WM_PAINT => {
            render();
            // SAFETY: valid window; the swap chain draws the content, so the update region is just cleared.
            unsafe { let _ = windows::Win32::Graphics::Gdi::ValidateRect(Some(hwnd), None); }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            if wparam.0 == TIMER_REPORT {
                let (ws, private) = memory_mb();
                let frames = STATE.with(|cell| cell.borrow().as_ref().map_or(0, |s| s.frames));
                println!("memory: working set {ws:.0} MB, private {private:.0} MB, frames {frames}");
            } else if wparam.0 == TIMER_EXIT {
                // SAFETY: valid window handle.
                unsafe { let _ = DestroyWindow(hwnd); }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends the message loop of this thread.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: default handling of every other message.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = args.iter().find(|a| matches!(a.as_str(), "mica" | "mica-alt" | "acrylic" | "none")).map_or("mica", String::as_str);
    let exit_after = args.iter().position(|a| a == "--exit-after").and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<u32>().ok());
    let started = Instant::now();

    // SAFETY: plain Win32 calls on the thread that owns the window; every pointer outlives its call.
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: w!("NekominalUiSpike"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        // WS_EX_NOREDIRECTIONBITMAP: the window has no GDI surface, only the DirectComposition content.
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("NekominalUiSpike"),
            w!("ui-spike"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            640,
            None,
            None,
            Some(module.into()),
            None,
        )?;

        // The backdrop: 2 Mica, 3 acrylic, 4 Mica Alt; 1 none. Needs the frame extended over the whole client area.
        let backdrop = match kind {
            "acrylic" => 3,
            "mica-alt" => 4,
            "none" => 1,
            _ => 2,
        };
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&dark as *const i32).cast(), 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, (&DWM_SYSTEMBACKDROP_TYPE(backdrop) as *const DWM_SYSTEMBACKDROP_TYPE).cast(), 4);
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });

        // DirectComposition: a device, a target on the window, one visual that takes the swap chain.
        let dcomp: IDCompositionDevice = DCompositionCreateDevice(None::<&IDXGIDevice>)?;
        let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
        let visual = dcomp.CreateVisual()?;
        target.SetRoot(&visual)?;

        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(visual.as_raw()))
            .expect("create_surface_unsafe(CompositionVisual)");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .expect("adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .expect("device");
        let caps = surface.get_capabilities(&adapter);
        println!("adapter {} {:?}; alpha modes {:?}", adapter.get_info().name, adapter.get_info().backend, caps.alpha_modes);

        let mut rect = windows::Win32::Foundation::RECT::default();
        GetClientRect(hwnd, &mut rect)?;
        let mut config = surface.get_default_config(&adapter, (rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32).expect("config");
        config.format = FORMAT;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
        surface.configure(&device, &config);
        println!("configured {}x{} {:?} {:?}, backdrop {kind}", config.width, config.height, config.format, config.alpha_mode);

        STATE.with(|cell| {
            *cell.borrow_mut() = Some(State { surface, device, queue, config, dcomp, _target: target, _visual: visual, frames: 0, started })
        });
        render();
        SetTimer(Some(hwnd), TIMER_REPORT, 5000, None);
        if let Some(seconds) = exit_after {
            SetTimer(Some(hwnd), TIMER_EXIT, seconds * 1000, None);
        }

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    let (ws, private) = memory_mb();
    println!("exit: working set {ws:.0} MB, private {private:.0} MB");
    Ok(())
}
