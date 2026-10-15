//! The Win32 side: the window class, the custom frame (the title bar is ours), and the messages handed to `App`.

use std::cell::RefCell;

use nekominal_input::MouseButton;
use nekominal_model::settings::Backdrop;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DWM_SYSTEMBACKDROP_TYPE, DWMWA_CAPTION_BUTTON_BOUNDS, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
                DwmDefWindowProc, DwmExtendFrameIntoClientArea, DwmGetWindowAttribute, DwmSetWindowAttribute,
            },
            Gdi::{ClientToScreen, ScreenToClient, ValidateRect},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::MARGINS,
            HiDpi::{
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, GetSystemMetricsForDpi,
                SetProcessDpiAwarenessContext,
            },
            Input::Ime::{
                CANDIDATEFORM, CFS_CANDIDATEPOS, CFS_POINT, COMPOSITIONFORM, GCS_COMPSTR, GCS_RESULTSTR, HIMC,
                IME_COMPOSITION_STRING, ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow,
                ImmSetCompositionWindow,
            },
            Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

use crate::{
    app::{App, Capture, WM_WAKE},
    layout::Hit,
};

/// Not in the generated bindings (it is a common-controls message).
const WM_MOUSELEAVE: u32 = 0x02A3;
/// The timer that scrolls the view while a selection drag is held outside the pane.
const DRAG_TIMER: usize = 1;

thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }

/// Runs `f` on the application unless it is not there yet or already busy (a message sent from inside a handler).
fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut app| app.as_mut().map(f)))
}

fn lo(l: LPARAM) -> i32 {
    i32::from((l.0 & 0xFFFF) as i16)
}

fn hi(l: LPARAM) -> i32 {
    i32::from(((l.0 >> 16) & 0xFFFF) as i16)
}

fn scale_of(hwnd: HWND) -> f32 {
    // SAFETY: a window handle of this thread.
    unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}

fn client_size(hwnd: HWND) -> (u32, u32) {
    let mut rect = RECT::default();
    // SAFETY: a window handle of this thread and a rect that lives through the call.
    unsafe { GetClientRect(hwnd, &mut rect).ok() };
    ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32)
}

fn is_maximized(hwnd: HWND) -> bool {
    // SAFETY: a window handle of this thread.
    unsafe { IsZoomed(hwnd).as_bool() }
}

/// Width in pixels of the caption buttons DWM draws at the top right, measured from the client area's right edge.
fn caption_px(hwnd: HWND) -> f32 {
    let mut bounds = RECT::default();
    let mut window = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: attribute and rectangles of the sizes the calls document, on a window of this thread.
    unsafe {
        if DwmGetWindowAttribute(hwnd, DWMWA_CAPTION_BUTTON_BOUNDS, (&mut bounds as *mut RECT).cast(), std::mem::size_of::<RECT>() as u32).is_err() {
            return 138.0 * scale_of(hwnd);
        }
        let _ = GetWindowRect(hwnd, &mut window);
        let _ = ClientToScreen(hwnd, &mut origin);
    }
    // The bounds are in window coordinates: from their left edge to the right edge of the client area.
    let client_right = client_size(hwnd).0 as i32;
    (client_right - (bounds.left - (origin.x - window.left))).max(0) as f32
}

/// Width in pixels of the resize border around the window.
fn border(hwnd: HWND) -> i32 {
    // SAFETY: plain metrics query.
    unsafe { GetSystemMetricsForDpi(SM_CXSIZEFRAME, GetDpiForWindow(hwnd)) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, GetDpiForWindow(hwnd)) }
}

/// Where the pointer is at the window's edge, for resizing; `None` inside.
fn edge_hit(hwnd: HWND, x: i32, y: i32) -> Option<u32> {
    if is_maximized(hwnd) {
        return None;
    }
    let mut rect = RECT::default();
    // SAFETY: as above.
    unsafe { GetWindowRect(hwnd, &mut rect).ok() };
    let b = border(hwnd).max(5);
    let (left, right) = (x < rect.left + b, x >= rect.right - b);
    let (top, bottom) = (y < rect.top + b, y >= rect.bottom - b);
    Some(match (left, right, top, bottom) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, ..) => HTLEFT,
        (_, true, ..) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (_, _, _, true) => HTBOTTOM,
        _ => return None,
    })
    .map(|code| code as u32)
}

/// The composition string of the input method (`what` is the preedit or the result), as text.
fn composition_text(himc: HIMC, what: IME_COMPOSITION_STRING) -> String {
    // SAFETY: the buffer is as long as the call says it needs.
    unsafe {
        let bytes = ImmGetCompositionStringW(himc, what, None, 0);
        if bytes <= 0 {
            return String::new();
        }
        let mut units = vec![0u16; bytes as usize / 2];
        ImmGetCompositionStringW(himc, what, Some(units.as_mut_ptr().cast()), bytes as u32);
        String::from_utf16_lossy(&units)
    }
}

/// Puts the composition and candidate windows of the input method at the cursor cell of the pane.
fn place_ime(hwnd: HWND) {
    let Some((x, y, height)) = with_app(|app| app.ime_anchor()).flatten() else { return };
    // SAFETY: the context belongs to this window and is released below; the forms live through the calls.
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return;
        }
        let composition = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: POINT { x, y }, rcArea: RECT::default() };
        let _ = ImmSetCompositionWindow(himc, &composition);
        let candidates = CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_CANDIDATEPOS, ptCurrentPos: POINT { x, y: y + height }, rcArea: RECT::default() };
        let _ = ImmSetCandidateWindow(himc, &candidates);
        let _ = ImmReleaseContext(hwnd, himc);
    }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // The caption buttons are DWM's: it answers the hit tests, the hovering and the clicks on them first.
    if matches!(msg, WM_NCHITTEST | WM_NCMOUSEMOVE | WM_NCLBUTTONDOWN | WM_NCLBUTTONUP | WM_NCMOUSELEAVE) {
        let mut result = LRESULT(0);
        // SAFETY: the arguments are those of this message.
        if unsafe { DwmDefWindowProc(hwnd, msg, wparam, lparam, &mut result) }.as_bool() {
            return result;
        }
    }
    match msg {
        // The client area is the whole window except the frame the system draws left, right and below; the title
        // bar is ours. When maximized the window overhangs the screen by the frame: keep the top inside it.
        WM_NCCALCSIZE if wparam.0 != 0 => {
            // SAFETY: with wParam set, lParam points to an NCCALCSIZE_PARAMS.
            unsafe {
                let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                let top = params.rgrc[0].top;
                let result = DefWindowProcW(hwnd, msg, wparam, lparam);
                params.rgrc[0].top = if is_maximized(hwnd) { top + border(hwnd) } else { top };
                result
            }
        }
        WM_NCHITTEST => {
            let (x, y) = (lo(lparam), hi(lparam));
            if let Some(edge) = edge_hit(hwnd, x, y) {
                return LRESULT(edge as isize);
            }
            let mut point = POINT { x, y };
            // SAFETY: a window handle of this thread.
            unsafe { let _ = ScreenToClient(hwnd, &mut point); };
            match with_app(|app| app.hit_test(point.x, point.y)) {
                Some(Hit::Drag) => LRESULT(HTCAPTION as isize),
                _ => LRESULT(HTCLIENT as isize),
            }
        }
        WM_SIZE => {
            let size = client_size(hwnd);
            let caption = caption_px(hwnd);
            with_app(|app| {
                app.on_caption(caption);
                app.on_size(size, wparam.0 == SIZE_MAXIMIZED as usize)
            });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: lParam is the suggested new rectangle.
            unsafe {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            let scale = scale_of(hwnd);
            with_app(|app| app.on_dpi(scale));
            LRESULT(0)
        }
        WM_PAINT => {
            with_app(App::paint);
            // SAFETY: the swap chain is the content; only the update region is cleared.
            unsafe { let _ = ValidateRect(Some(hwnd), None); };
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_ACTIVATE => {
            let active = (wparam.0 & 0xFFFF) != 0;
            with_app(|app| app.on_activate(active));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            // SAFETY: a valid request for this window.
            unsafe { let _ = TrackMouseEvent(&mut track); };
            // SAFETY: plain query.
            let time = unsafe { GetMessageTime() } as u32;
            with_app(|app| app.on_mouse_move(lo(lparam), hi(lparam), time));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(App::on_mouse_leave);
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP => {
            let down = matches!(msg, WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN);
            let button = match msg {
                WM_LBUTTONDOWN | WM_LBUTTONUP => MouseButton::Left,
                WM_MBUTTONDOWN | WM_MBUTTONUP => MouseButton::Middle,
                _ => MouseButton::Right,
            };
            // SAFETY: plain query.
            let time = unsafe { GetMessageTime() } as u32;
            match with_app(|app| app.on_button(lo(lparam), hi(lparam), button, down, time)) {
                // The capture keeps the moves coming while a button is held outside the window (a selection drag),
                // and a timer scrolls the view while the drag is held above or below the pane.
                // SAFETY: calls on this window from its own thread.
                Some(Capture::Take) => unsafe {
                    SetCapture(hwnd);
                    SetTimer(Some(hwnd), DRAG_TIMER, 50, None);
                },
                Some(Capture::Release) => unsafe {
                    let _ = KillTimer(Some(hwnd), DRAG_TIMER);
                    let _ = ReleaseCapture();
                },
                _ => {}
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            // SAFETY: stopping a timer of this window.
            unsafe { let _ = KillTimer(Some(hwnd), DRAG_TIMER); }
            with_app(App::on_capture_lost);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == DRAG_TIMER => {
            with_app(App::on_drag_tick);
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT && with_app(|app| app.hand_cursor()).unwrap_or(false) => {
            // SAFETY: loading and setting a system cursor.
            unsafe { SetCursor(LoadCursorW(None, IDC_HAND).ok()); }
            LRESULT(1)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
            // The wheel reports screen coordinates.
            let mut p = POINT { x: lo(lparam), y: hi(lparam) };
            // SAFETY: as above.
            unsafe { let _ = ScreenToClient(hwnd, &mut p); }
            // SAFETY: plain query.
            let time = unsafe { GetMessageTime() } as u32;
            with_app(|app| app.on_wheel(p.x, p.y, delta, time));
            LRESULT(0)
        }        // The input method composes into the terminal: its text is drawn inline at the cursor by the pane, so the
        // system's own composition window stays hidden (these messages are not passed on), and the candidate window
        // is placed under the cursor cell.
        WM_IME_STARTCOMPOSITION => {
            place_ime(hwnd);
            LRESULT(0)
        }
        WM_IME_COMPOSITION => {
            // SAFETY: the context belongs to this window and is released below.
            unsafe {
                let himc = ImmGetContext(hwnd);
                if !himc.is_invalid() {
                    let flags = lparam.0 as u32;
                    if flags & GCS_RESULTSTR.0 != 0 {
                        let text = composition_text(himc, GCS_RESULTSTR);
                        with_app(|app| app.on_ime_commit(&text));
                    }
                    if flags & GCS_COMPSTR.0 != 0 {
                        let text = composition_text(himc, GCS_COMPSTR);
                        with_app(|app| app.on_ime_preedit((!text.is_empty()).then_some(text)));
                    }
                    let _ = ImmReleaseContext(hwnd, himc);
                }
            }
            place_ime(hwnd);
            LRESULT(0)
        }
        WM_IME_ENDCOMPOSITION => {
            with_app(|app| app.on_ime_preedit(None));
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let (scan, extended, repeat) = (((lparam.0 >> 16) & 0xFF) as u16, (lparam.0 >> 24) & 1 == 1, (lparam.0 >> 30) & 1 == 1);
            let handled = with_app(|app| app.on_key_down(wparam.0 as i32, scan, extended, repeat)).unwrap_or(false);
            // Alt+Space opens the system menu; every other Alt chord belongs to the terminal.
            if handled || (msg == WM_SYSKEYDOWN && wparam.0 != 0x20) {
                return LRESULT(0);
            }
            // SAFETY: default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // A lone Alt press and release must not start the (empty) window menu.
        WM_SYSKEYUP if wparam.0 != 0x20 => LRESULT(0),
        WM_CHAR => {
            with_app(|app| app.on_char(wparam.0 as u16));
            LRESULT(0)
        }
        WM_SYSCHAR => LRESULT(0),
        // The default answer to an Alt chord that matches no menu is a beep.
        WM_MENUCHAR => LRESULT((1isize) << 16),
        WM_WAKE => {
            with_app(App::on_wake);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            // SAFETY: lParam points to a MINMAXINFO.
            unsafe { (*(lparam.0 as *mut MINMAXINFO)).ptMinTrackSize = POINT { x: 480, y: 320 } };
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

/// The DWM backdrop the window shows through: 1 none, 2 Mica, 3 acrylic, 4 Mica Alt. Needs the frame extended over
/// the whole client area.
fn set_backdrop(hwnd: HWND, backdrop: Backdrop, dark: bool) {
    let kind = match backdrop {
        Backdrop::Acrylic => 3,
        Backdrop::Mica => 2,
        Backdrop::MicaAlt => 4,
        Backdrop::None => 1,
    };
    let dark = i32::from(dark);
    // SAFETY: attribute values of the sizes DWM documents, on a window of this thread.
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&dark as *const i32).cast(), 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, (&DWM_SYSTEMBACKDROP_TYPE(kind) as *const DWM_SYSTEMBACKDROP_TYPE).cast(), 4);
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
    }
}

pub fn run() -> Result<(), String> {
    // SAFETY: plain Win32 calls on the thread that owns the window.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: w!("NekominalShell"),
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            ..Default::default()
        };
        RegisterClassW(&class);
        // No GDI surface: the window shows the DirectComposition content only.
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("NekominalShell"),
            w!("Nekominal"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            700,
            None,
            None,
            Some(module.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        let scale = scale_of(hwnd);
        // The window was created at 1100x700 pixels; make that DIP.
        let _ = SetWindowPos(hwnd, None, 0, 0, (1100.0 * scale) as i32, (700.0 * scale) as i32, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        let app = App::new(hwnd, client_size(hwnd), scale)?;
        set_backdrop(hwnd, app.backdrop(), app.dark());
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let caption = caption_px(hwnd);
        with_app(|app| app.on_caption(caption));
        with_app(|app| app.redraw());

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}
