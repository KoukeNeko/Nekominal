//! The small window of the update: the versions, what is being done, the last line Scoop wrote, and a bar of the steps.
//! Plain Win32 and GDI, dark like the app, sized for the monitor's DPI. The work runs on a thread of its own and asks the
//! window to draw again ([`Ui::refresh`]); the window cannot be closed while the tabs are held, since closing it would
//! end them.

use std::sync::{Arc, Mutex};

use windows_sys::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Dwm::DwmSetWindowAttribute,
            Gdi::{
                BeginPaint, CreateFontW, CreateSolidBrush, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_WORDBREAK, DeleteObject,
                DrawTextW, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

/// Asks the window to draw the view again.
pub const WM_REFRESH: u32 = WM_APP + 1;
/// The work is over: the window may close, and closes by itself after wParam milliseconds (0: when the user closes it).
pub const WM_FINISHED: u32 = WM_APP + 2;
const CLOSE_TIMER: usize = 1;

/// The steps of the bar.
pub const STEPS: usize = 4;

#[derive(Clone, Debug, Default)]
pub struct View {
    pub from: String,
    pub to: Option<String>,
    pub status: String,
    /// The last line Scoop wrote.
    pub detail: String,
    /// How many of the [`STEPS`] are done.
    pub step: usize,
    pub failed: bool,
}

/// The window, for the thread of the work.
#[derive(Clone, Copy)]
pub struct Ui(isize);

impl Ui {
    pub fn refresh(self) {
        // SAFETY: posting to a window from another thread; a window that is gone drops it.
        unsafe { PostMessageW(self.0 as HWND, WM_REFRESH, 0, 0) };
    }

    pub fn finish(self, close_after_ms: u32) {
        // SAFETY: as above.
        unsafe { PostMessageW(self.0 as HWND, WM_FINISHED, close_after_ms as usize, 0) };
    }
}

struct State {
    view: Arc<Mutex<View>>,
    finished: bool,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// COLORREF of an RGB colour.
const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

const BACKGROUND: u32 = rgb(0x1f, 0x1f, 0x22);
const TEXT: u32 = rgb(0xe8, 0xe8, 0xea);
const DIM: u32 = rgb(0x9a, 0x9a, 0xa2);
const ACCENT: u32 = rgb(0x6c, 0xa8, 0xff);
const DANGER: u32 = rgb(0xff, 0x6b, 0x6b);
const TRACK: u32 = rgb(0x3a, 0x3a, 0x40);

/// The size of the window's client area, in DIP.
const WIDTH: i32 = 480;
const HEIGHT: i32 = 172;

/// Shows the window and runs `work` on a thread with it; returns when the window closes.
pub fn run(title: &str, view: Arc<Mutex<View>>, work: impl FnOnce(Ui) + Send + 'static) {
    // SAFETY: plain Win32 calls on the thread that owns the window.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = GetModuleHandleW(std::ptr::null());
        // Resource 1 of the program, the icon of the app (`updater.rc`).
        let icon = LoadIconW(module, 1 as _);
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: module,
            lpszClassName: w!("NekominalUpdater"),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hIcon: icon,
            ..std::mem::zeroed()
        };
        RegisterClassW(&class);
        STATE.with(|state| *state.borrow_mut() = Some(State { view, finished: false }));
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let dpi = GetDpiForSystem();
        let mut rect = RECT { left: 0, top: 0, right: WIDTH * dpi as i32 / 96, bottom: HEIGHT * dpi as i32 / 96 };
        AdjustWindowRectExForDpi(&mut rect, style, 0, 0, dpi);
        let title = wide(title);
        let hwnd = CreateWindowExW(
            0,
            w!("NekominalUpdater"),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            module,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            // No window (a session without a desktop): the work still runs, and ends the process.
            work(Ui(0));
            return;
        }
        // The title bar dark too (DWMWA_USE_IMMERSIVE_DARK_MODE).
        let dark: i32 = 1;
        DwmSetWindowAttribute(hwnd, 20, (&dark as *const i32).cast(), 4);
        ShowWindow(hwnd, SW_SHOWNORMAL);
        let ui = Ui(hwnd as isize);
        let _ = std::thread::Builder::new().name("update".into()).spawn(move || work(ui));
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the messages of a window of this thread, handled as Win32 documents them.
    unsafe {
        match msg {
            WM_REFRESH => {
                InvalidateRect(hwnd, std::ptr::null(), 1);
                0
            }
            WM_FINISHED => {
                STATE.with(|state| state.borrow_mut().as_mut().map(|state| state.finished = true));
                InvalidateRect(hwnd, std::ptr::null(), 1);
                if wparam > 0 {
                    SetTimer(hwnd, CLOSE_TIMER, wparam as u32, None);
                }
                0
            }
            WM_TIMER if wparam == CLOSE_TIMER => {
                DestroyWindow(hwnd);
                0
            }
            // Closing while the tabs are held would end them: only once the work is over.
            WM_CLOSE => {
                if STATE.with(|state| state.borrow().as_ref().is_some_and(|state| state.finished)) {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_DPICHANGED => {
                let r = &*(lparam as *const RECT);
                SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                0
            }
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                paint(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Draws the view.
///
/// # Safety
/// `hwnd` is a window of this thread, in its WM_PAINT.
unsafe fn paint(hwnd: HWND) {
    let view = STATE.with(|state| state.borrow().as_ref().map(|state| state.view.lock().unwrap().clone())).unwrap_or_default();
    // SAFETY: the caller guarantees the window; every GDI object made here is deleted here.
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let px = |dip: f32| (dip * scale).round() as i32;
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let fill = |rect: RECT, color: u32| {
            let brush = CreateSolidBrush(color);
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
        };
        fill(client, BACKGROUND);
        SetBkMode(dc, TRANSPARENT as i32);
        let font = |size: f32, weight: i32| CreateFontW(-px(size), 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, w!("Segoe UI"));
        let (big, small) = (font(17.0, 600), font(13.0, 400));
        let text = |text: &str, font, color: u32, top: f32, height: f32, wrap: bool| {
            let old = SelectObject(dc, font);
            SetTextColor(dc, color);
            let mut rect = RECT { left: px(20.0), top: px(top), right: client.right - px(20.0), bottom: px(top + height) };
            let mut units: Vec<u16> = text.encode_utf16().collect();
            let flags = DT_LEFT | DT_NOPREFIX | if wrap { DT_WORDBREAK | DT_END_ELLIPSIS } else { DT_SINGLELINE | DT_END_ELLIPSIS };
            DrawTextW(dc, units.as_mut_ptr(), units.len() as i32, &mut rect, flags);
            SelectObject(dc, old);
        };
        let versions = format!("Nekominal {} \u{2192} {}", view.from, view.to.as_deref().unwrap_or("\u{2026}"));
        text(&versions, big, TEXT, 16.0, 26.0, false);
        text(&view.status, small, if view.failed { DANGER } else { TEXT }, 50.0, 40.0, true);
        text(&view.detail, small, DIM, 94.0, 22.0, false);
        // The bar: a segment per step, lit up to the one being done.
        let (left, right, top) = (px(20.0), client.right - px(20.0), px(134.0));
        let gap = px(6.0);
        let width = (right - left - gap * (STEPS as i32 - 1)) / STEPS as i32;
        for step in 0..STEPS {
            let x = left + step as i32 * (width + gap);
            let color = if view.failed && step == view.step.min(STEPS - 1) { DANGER } else if step < view.step { ACCENT } else { TRACK };
            fill(RECT { left: x, top, right: x + width, bottom: top + px(4.0) }, color);
        }
        DeleteObject(big);
        DeleteObject(small);
        EndPaint(hwnd, &ps);
    }
}
