//! The window of the settings: a window of its own, owned by the one that opened it, with the frame and the title bar of the
//! system and a swap chain on the device of its owner. What it shows and what it does belongs to `App` (the page); this
//! file only makes the window. Its messages go through `win::settings_proc`.

use accesskit_windows::Adapter;
use nekominal_model::settings::Backdrop;
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetClientRect, GetWindowRect, IDC_ARROW, LoadCursorW, RegisterClassW, SW_SHOW, ShowWindow,
                WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_OVERLAPPEDWINDOW,
            },
        },
    },
    core::{HSTRING, w},
};

use crate::gpu::{self, Gpu, Surface};

/// How big the window is when it opens, in DIP (the frame is in it), and the smallest it can be made.
const SIZE: (f32, f32) = (1000.0, 760.0);
pub const MIN_SIZE: (f32, f32) = (760.0, 480.0);

pub struct SettingsWin {
    pub hwnd: HWND,
    pub surface: Surface,
    /// Pixels per DIP on the monitor the window is on.
    pub scale: f32,
    /// What a screen reader is told about the window (see `App::settings_access_update`).
    pub access: Option<Adapter>,
}

impl SettingsWin {
    /// The size of the part of the window the page is drawn in, in DIP.
    pub fn area(&self) -> (f32, f32) {
        let (w, h) = self.surface.size();
        (w as f32 / self.scale, h as f32 / self.scale)
    }

    /// Takes the window away; the swap chain goes with the value.
    pub fn close(self) {
        // SAFETY: destroys a window of this thread.
        unsafe { let _ = DestroyWindow(self.hwnd); }
    }
}

/// Opens the window in the middle of `owner` (kept on its screen), over the backdrop the settings ask for.
pub fn open(owner: HWND, gpu: &Gpu, backdrop: Backdrop, dark: bool, title: &str) -> Result<SettingsWin, String> {
    // SAFETY: plain Win32 calls on the thread of the owner window; every pointer outlives its call.
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(crate::win::settings_proc),
            hInstance: module.into(),
            lpszClassName: w!("NekominalSettings"),
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            ..Default::default()
        };
        // Registered once; a second registration fails and the first stays.
        RegisterClassW(&class);

        let scale = GetDpiForWindow(owner) as f32 / 96.0;
        let (width, height) = ((SIZE.0 * scale) as i32, (SIZE.1 * scale) as i32);
        let mut around = RECT::default();
        let _ = GetWindowRect(owner, &mut around);
        let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut monitor);
        let work = monitor.rcWork;
        let (width, height) = (width.min(work.right - work.left), height.min(work.bottom - work.top));
        let x = (around.left + (around.right - around.left - width) / 2).clamp(work.left, (work.right - width).max(work.left));
        let y = (around.top + (around.bottom - around.top - height) / 2).clamp(work.top, (work.bottom - height).max(work.top));

        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("NekominalSettings"),
            &HSTRING::from(title),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            width,
            height,
            Some(owner),
            None,
            Some(module.into()),
            None,
        )
        .map_err(|e| e.to_string())?;

        crate::win::set_backdrop(hwnd, backdrop, dark);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let size = ((client.right - client.left).max(1) as u32, (client.bottom - client.top).max(1) as u32);
        let surface = match gpu::create_surface(gpu, hwnd, size) {
            Ok(surface) => surface,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(e);
            }
        };
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let access = Some(Adapter::new(hwnd, true, crate::access::NoActions));
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(SettingsWin { hwnd, surface, scale, access })
    }
}
