//! The window of the settings: a window of its own, owned by the one that opened it, with a swap chain on the device of its
//! owner. The frame is the system's (the edges to size it by, the shadow), the title bar is ours as in the window of the
//! shell: a row with the title and the three caption buttons, and the page below it. What it shows and what it does belongs
//! to `App` (the page); this file only makes the window and says where its title bar is. Its messages go through
//! `win::settings_proc`.

use accesskit_windows::Adapter;
use nekominal_model::settings::Backdrop;
use nekominal_ui::Rect;
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetClientRect, GetWindowRect, IDC_ARROW, LoadCursorW, RegisterClassW, SW_SHOW, SWP_FRAMECHANGED,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow, WINDOW_STYLE, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP,
                WS_OVERLAPPEDWINDOW, WS_SYSMENU,
            },
        },
    },
    core::{HSTRING, w},
};

use crate::{
    gpu::{self, Gpu, Surface},
    layout::Hit,
};

/// How big the window is when it opens, in DIP (the frame is in it), and the smallest it can be made.
const SIZE: (f32, f32) = (1000.0, 760.0);
pub const MIN_SIZE: (f32, f32) = (760.0, 480.0);
/// The title row: its height, and the width of each of the three caption buttons at its right, in DIP.
pub const TITLE: f32 = 40.0;
pub const CAPTION_CELL: f32 = 46.0;

/// The minimize, maximize (or restore) and close buttons of a window `width` DIP wide, each as tall as the title row.
pub fn caption_buttons(width: f32) -> [Rect; 3] {
    [0.0, 1.0, 2.0].map(|i| Rect::new(width - 3.0 * CAPTION_CELL + i * CAPTION_CELL, 0.0, CAPTION_CELL, TITLE))
}

/// What is under a point of a window `width` DIP wide: a caption button, the rest of the title row (the window is dragged
/// by it), or the page.
pub fn hit(width: f32, x: f32, y: f32) -> Hit {
    for (hit, button) in [Hit::Minimize, Hit::Maximize, Hit::Close].into_iter().zip(caption_buttons(width)) {
        if button.contains(x, y) {
            return hit;
        }
    }
    if (0.0..TITLE).contains(&y) { Hit::Drag } else { Hit::None }
}

pub struct SettingsWin {
    pub hwnd: HWND,
    pub surface: Surface,
    /// Pixels per DIP on the monitor the window is on.
    pub scale: f32,
    /// The caption button the pointer is over and the one being pressed.
    pub hover: Hit,
    pub pressed: Hit,
    pub maximized: bool,
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
            hIcon: crate::win::app_icon(),
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
            // Without WS_SYSMENU DWM draws no caption buttons of its own (they would sit at the top, over ours).
            WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 & !WS_SYSMENU.0),
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

        // The client area is the whole window but for the frame at the sides and below (WM_NCCALCSIZE): the frame is asked
        // to be worked out again, now that the window has the style it has.
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
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
        Ok(SettingsWin { hwnd, surface, scale, hover: Hit::None, pressed: Hit::None, maximized: false, access })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caption_buttons_fill_the_corner_of_the_title_row() {
        let [minimize, maximize, close] = caption_buttons(1000.0);
        assert_eq!((minimize.x, close.x + close.w), (1000.0 - 3.0 * CAPTION_CELL, 1000.0));
        assert!(caption_buttons(1000.0).iter().all(|b| b.y == 0.0 && b.h == TITLE && b.w == CAPTION_CELL));
        assert_eq!(minimize.x + minimize.w, maximize.x);
        assert_eq!(maximize.x + maximize.w, close.x);
    }

    #[test]
    fn the_title_row_is_dragged_by_what_the_buttons_leave_and_the_page_begins_below_it() {
        let [minimize, maximize, close] = caption_buttons(1000.0);
        assert_eq!(hit(1000.0, minimize.x + 2.0, 2.0), Hit::Minimize);
        assert_eq!(hit(1000.0, maximize.x + 2.0, TITLE - 1.0), Hit::Maximize);
        assert_eq!(hit(1000.0, 999.0, 20.0), Hit::Close);
        assert_eq!(hit(1000.0, 100.0, 5.0), Hit::Drag, "the title is dragged by");
        assert_eq!(hit(1000.0, 100.0, TITLE), Hit::None, "the page begins at the end of the row");
        assert_eq!(hit(1000.0, close.x + 2.0, TITLE + 2.0), Hit::None, "below the buttons is the page, not the button");
    }
}
