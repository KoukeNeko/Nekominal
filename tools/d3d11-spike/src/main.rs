//! Spike: what a Win32 window with a D3D11 swap chain on a DirectComposition visual costs in memory, with one
//! pipeline and the two glyph atlases the renderer has. Compare with `ui-spike` (the same with wgpu on D3D12).
//!
//!   d3d11-spike [--exit-after SECONDS]

use windows::{
    core::{s, w, Interface},
    Win32::{
        Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::{
            Direct3D::{Fxc::D3DCompile, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::*,
            DirectComposition::{DCompositionCreateDevice, IDCompositionDevice},
            Dxgi::{Common::*, *},
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::GetCurrentProcess,
        },
        UI::WindowsAndMessaging::*,
    },
};

const SHADER: &str = "
struct VsOut { float4 pos : SV_Position; float4 color : COLOR; };
VsOut vs(uint id : SV_VertexID, float4 rect : RECT, float4 color : COLOR) {
    float2 corner = float2(id & 1, id >> 1);
    VsOut o; o.pos = float4((rect.xy + corner * rect.zw) * 2 - 1, 0, 1); o.color = color; return o;
}
float4 ps(VsOut i) : SV_Target { return i.color; }
";

fn memory_mb() -> (f64, f64) {
    let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
    c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // SAFETY: the counters struct is sized as declared and lives through the call.
    unsafe { let _ = GetProcessMemoryInfo(GetCurrentProcess(), (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), c.cb); }
    (c.WorkingSetSize as f64 / 1_048_576.0, c.PrivateUsage as f64 / 1_048_576.0)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_DESTROY => {
            // SAFETY: ends the message loop of this thread.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() -> windows::core::Result<()> {
    let seconds: u64 = std::env::args().skip_while(|a| a != "--exit-after").nth(1).and_then(|s| s.parse().ok()).unwrap_or(8);
    // SAFETY: plain Win32/D3D11 calls on one thread; every pointer outlives its call.
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: module.into(), lpszClassName: w!("D3d11Spike"), ..Default::default() };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, w!("D3d11Spike"), w!("d3d11-spike"), WS_OVERLAPPEDWINDOW | WS_VISIBLE, 100, 100, 1000, 640, None, None, Some(module.into()), None)?;

        let mut device = None;
        let mut context = None;
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, Some(&[D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))?;
        let (device, context) = (device.unwrap(), context.unwrap());
        let dxgi: IDXGIDevice = device.cast()?;
        let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: 1000, Height: 640, Format: DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT, BufferCount: 2, SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED, ..Default::default()
        };
        let swap = factory.CreateSwapChainForComposition(&device, &desc, None)?;
        let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
        let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
        let visual = dcomp.CreateVisual()?;
        visual.SetContent(&swap)?;
        target.SetRoot(&visual)?;
        dcomp.Commit()?;

        // One pipeline (vertex + pixel shader, instanced quads) and the two atlases at 512 px.
        let compile = |entry: windows::core::PCSTR, profile: windows::core::PCSTR| {
            let mut blob = None;
            D3DCompile(SHADER.as_ptr().cast(), SHADER.len(), None, None, None, entry, profile, 0, 0, &mut blob, None).unwrap();
            blob.unwrap()
        };
        let (vs_blob, ps_blob) = (compile(s!("vs"), s!("vs_4_0")), compile(s!("ps"), s!("ps_4_0")));
        let bytes = |b: &windows::Win32::Graphics::Direct3D::ID3DBlob| std::slice::from_raw_parts(b.GetBufferPointer().cast::<u8>(), b.GetBufferSize());
        let mut vs = None;
        device.CreateVertexShader(bytes(&vs_blob), None, Some(&mut vs))?;
        let mut ps = None;
        device.CreatePixelShader(bytes(&ps_blob), None, Some(&mut ps))?;
        for format in [DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM] {
            let tex = D3D11_TEXTURE2D_DESC { Width: 512, Height: 512, MipLevels: 1, ArraySize: 1, Format: format, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32, ..Default::default() };
            let mut t = None;
            device.CreateTexture2D(&tex, None, Some(&mut t))?;
            std::mem::forget(t);
        }

        let back: ID3D11Texture2D = swap.GetBuffer(0)?;
        let mut rtv = None;
        device.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
        let a = 0.55f32;
        for _ in 0..3 {
            context.ClearRenderTargetView(rtv.as_ref().unwrap(), &[0.10 * a, 0.12 * a, 0.25 * a, a]);
            swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        std::mem::forget((vs, ps));
        SetTimer(Some(hwnd), 1, (seconds * 1000) as u32, None);
        let mut message = MSG::default();
        let mut reported = false;
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if message.message == WM_TIMER && !reported {
                let (ws, private) = memory_mb();
                println!("d3d11: working set {ws:.0} MB, private {private:.0} MB");
                reported = true;
                let _ = DestroyWindow(hwnd);
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}
