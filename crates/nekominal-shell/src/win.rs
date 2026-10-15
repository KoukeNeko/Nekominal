//! The Win32 side: the window class, the custom frame (the title bar is ours), and the messages handed to `App`.

use std::{cell::{Cell, RefCell}, collections::HashMap, rc::Rc};

use nekominal_input::MouseButton;
use nekominal_model::settings::Backdrop;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DWM_SYSTEMBACKDROP_TYPE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea,
                DwmSetWindowAttribute,
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
            Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC_EX, ReleaseCapture, SetCapture, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

use crate::{
    app::{App, Capture, Cursor, Detach, WM_WAKE},
    layout::{Hit, STRIP_LEFT},
};

/// Not in the generated bindings (it is a common-controls message).
const WM_MOUSELEAVE: u32 = 0x02A3;
/// The timer that scrolls the view while a selection drag is held outside the pane.
const DRAG_TIMER: usize = 1;

pub(crate) const WM_NEW_WINDOW: u32 = WM_APP + 2;

thread_local! {
    static APPS: RefCell<HashMap<isize, Rc<RefCell<App>>>> = RefCell::new(HashMap::new());
    static RETIRED: RefCell<Vec<Rc<RefCell<App>>>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` on the application unless it is not there yet or already busy (a message sent from inside a handler).
fn with_app<T>(hwnd: HWND, f: impl FnOnce(&mut App) -> T) -> Option<T> {
    let app = APPS.with(|apps| {
        let apps = apps.borrow();
        apps.get(&(hwnd.0 as isize)).cloned().or_else(|| {
            let owner = unsafe { GetWindow(hwnd, GW_OWNER) }.ok()?;
            apps.get(&(owner.0 as isize)).cloned()
        })
    })?;
    let mut app = app.try_borrow_mut().ok()?;
    Some(f(&mut app))
}

/// Raises what a change of the window means to a screen reader, outside the borrow of the window.
fn raise_access_events(hwnd: HWND, events: impl FnOnce(&mut App) -> Option<accesskit_windows::QueuedEvents>) {
    if let Some(events) = with_app(hwnd, events).flatten() {
        events.raise();
    }
}

fn reap_closed_windows() {
    // Drop UI and PTY resources outside window callbacks, while cleanup threads can still start.
    let retired = RETIRED.with(|apps| std::mem::take(&mut *apps.borrow_mut()));
    drop(retired);
}

fn lo(l: LPARAM) -> i32 {
    i32::from((l.0 & 0xFFFF) as i16)
}

fn hi(l: LPARAM) -> i32 {
    i32::from(((l.0 >> 16) & 0xFFFF) as i16)
}

fn key_scan(vk_code: i32, scan: u16, extended: bool) -> (u16, bool) {
    if scan != 0 || matches!(vk_code, nekominal_input::vk::VK_PACKET | nekominal_input::vk::VK_PROCESSKEY) {
        return (scan, extended);
    }
    // Virtual-key-only input can omit the scan code; recover it using this window thread's keyboard layout.
    let mapped = unsafe { MapVirtualKeyW(vk_code as u32, MAPVK_VK_TO_VSC_EX) };
    ((mapped & 0xFF) as u16, extended || mapped >> 8 == 0xE0)
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

/// The caption button a non-client hit test code names.
fn caption_hit(code: u32) -> Option<Hit> {
    match code {
        HTMINBUTTON => Some(Hit::Minimize),
        HTMAXBUTTON => Some(Hit::Maximize),
        HTCLOSE => Some(Hit::Close),
        _ => None,
    }
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
    let Some((x, y, height)) = with_app(hwnd, |app| app.ime_anchor()).flatten() else { return };
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
            if !with_app(hwnd, |app| app.is_fullscreen()).unwrap_or(false)
                && let Some(edge) = edge_hit(hwnd, x, y)
            {
                return LRESULT(edge as isize);
            }
            let mut point = POINT { x, y };
            // SAFETY: a window handle of this thread.
            unsafe { let _ = ScreenToClient(hwnd, &mut point); };
            match with_app(hwnd, |app| app.hit_test(point.x, point.y)) {
                Some(Hit::Drag) => LRESULT(HTCAPTION as isize),
                // The window answers for its own caption buttons; the maximize one is what shows the snap layouts.
                Some(Hit::Minimize) => LRESULT(HTMINBUTTON as isize),
                Some(Hit::Maximize) => LRESULT(HTMAXBUTTON as isize),
                Some(Hit::Close) => LRESULT(HTCLOSE as isize),
                _ => LRESULT(HTCLIENT as isize),
            }
        }
        WM_NCMOUSEMOVE => {
            let hit = caption_hit(wparam.0 as u32);
            if hit.is_some() {
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                // SAFETY: a valid request for this window.
                unsafe { let _ = TrackMouseEvent(&mut track); };
            }
            with_app(hwnd, |app| app.on_caption_hover(hit.unwrap_or(Hit::None)));
            if hit.is_some() { LRESULT(0) } else { unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) } }
        }
        WM_NCMOUSELEAVE => {
            with_app(hwnd, |app| {
                app.on_caption_hover(Hit::None);
                app.on_caption_press(Hit::None);
            });
            LRESULT(0)
        }
        // Without the default handling there is no modal loop: the button is pressed here and acts when let go of over
        // the same button.
        WM_NCLBUTTONDOWN if caption_hit(wparam.0 as u32).is_some() => {
            let hit = caption_hit(wparam.0 as u32).unwrap_or(Hit::None);
            with_app(hwnd, |app| app.on_caption_press(hit));
            LRESULT(0)
        }
        WM_NCLBUTTONUP if caption_hit(wparam.0 as u32).is_some() => {
            let hit = caption_hit(wparam.0 as u32).unwrap_or(Hit::None);
            if with_app(hwnd, |app| app.on_caption_release(hit)) == Some(true) {
                let command = match hit {
                    Hit::Minimize => SC_MINIMIZE,
                    Hit::Maximize if is_maximized(hwnd) => SC_RESTORE,
                    Hit::Maximize => SC_MAXIMIZE,
                    _ => SC_CLOSE,
                };
                // SAFETY: posting to our own window.
                unsafe { let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0)); }
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let size = client_size(hwnd);
            with_app(hwnd, |app| app.on_size(size, wparam.0 == SIZE_MAXIMIZED as usize));
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: lParam is the suggested new rectangle.
            unsafe {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            let scale = scale_of(hwnd);
            with_app(hwnd, |app| app.on_dpi(scale));
            LRESULT(0)
        }
        WM_PAINT => {
            let next_frame = with_app(hwnd, App::paint).unwrap_or(false);
            // SAFETY: the swap chain is the content; only the update region is cleared.
            unsafe { let _ = ValidateRect(Some(hwnd), None); };
            // After the validation, which would take it back: a slide of the tab chips asks for its next frame, and a
            // window with every chip in its place asks for none.
            if next_frame {
                with_app(hwnd, |app| app.redraw());
            }
            raise_access_events(hwnd, App::access_update);
            LRESULT(0)
        }
        WM_GETOBJECT => {
            // The result is turned into a message result with no borrow of the window held: UI Automation may send
            // another WM_GETOBJECT while it does.
            match with_app(hwnd, |app| app.access_getobject(wparam, lparam)).flatten() {
                Some(result) => result.into(),
                None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            if with_app(hwnd, App::request_close_window) == Some(true) {
                unsafe { let _ = DestroyWindow(hwnd); }
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let active = (wparam.0 & 0xFFFF) != 0;
            with_app(hwnd, |app| app.on_activate(active));
            raise_access_events(hwnd, |app| app.access_focus(active));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if follow_move(hwnd, lparam) {
                return LRESULT(0);
            }
            let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            // SAFETY: a valid request for this window.
            unsafe { let _ = TrackMouseEvent(&mut track); };
            // SAFETY: plain query.
            let time = unsafe { GetMessageTime() } as u32;
            with_app(hwnd, |app| app.on_mouse_move(lo(lparam), hi(lparam), time));
            if let Some(detach) = with_app(hwnd, App::take_detach).flatten() {
                start_detach(hwnd, detach);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(hwnd, App::on_mouse_leave);
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP => {
            if msg == WM_LBUTTONUP && end_follow(hwnd) {
                return LRESULT(0);
            }
            let down = matches!(msg, WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN);
            let button = match msg {
                WM_LBUTTONDOWN | WM_LBUTTONUP => MouseButton::Left,
                WM_MBUTTONDOWN | WM_MBUTTONUP => MouseButton::Middle,
                _ => MouseButton::Right,
            };
            // SAFETY: plain query.
            let time = unsafe { GetMessageTime() } as u32;
            let capture = with_app(hwnd, |app| app.on_button(lo(lparam), hi(lparam), button, down, time));
            match capture {
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
            with_app(hwnd, App::on_capture_lost);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == DRAG_TIMER => {
            with_app(hwnd, App::on_drag_tick);
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT && with_app(hwnd, |app| app.cursor()).flatten().is_some() => {
            let id = match with_app(hwnd, |app| app.cursor()).flatten() {
                Some(Cursor::ResizeH) => IDC_SIZEWE,
                Some(Cursor::ResizeV) => IDC_SIZENS,
                _ => IDC_HAND,
            };
            // SAFETY: loading and setting a system cursor.
            unsafe { SetCursor(LoadCursorW(None, id).ok()); }
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
            with_app(hwnd, |app| app.on_wheel(p.x, p.y, delta, time));
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
                        with_app(hwnd, |app| app.on_ime_commit(&text));
                    }
                    if flags & GCS_COMPSTR.0 != 0 {
                        let text = composition_text(himc, GCS_COMPSTR);
                        with_app(hwnd, |app| app.on_ime_preedit((!text.is_empty()).then_some(text)));
                    }
                    let _ = ImmReleaseContext(hwnd, himc);
                }
            }
            place_ime(hwnd);
            LRESULT(0)
        }
        WM_IME_ENDCOMPOSITION => {
            with_app(hwnd, |app| app.on_ime_preedit(None));
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if msg == WM_SYSKEYDOWN && wparam.0 == 0x73 {
                let dismissed = with_app(hwnd, |app| {
                    if !app.popup_keeps_owner_active() { return false; }
                    app.close_popup();
                    true
                }).unwrap_or(false);
                return if dismissed { LRESULT(0) } else { unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) } };
            }
            let (scan, extended, repeat) = (((lparam.0 >> 16) & 0xFF) as u16, (lparam.0 >> 24) & 1 == 1, (lparam.0 >> 30) & 1 == 1);
            let (scan, extended) = key_scan(wparam.0 as i32, scan, extended);
            let handled = with_app(hwnd, |app| app.on_key_down(wparam.0 as i32, scan, extended, repeat)).unwrap_or(false);
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
            with_app(hwnd, |app| app.on_char(wparam.0 as u16));
            LRESULT(0)
        }
        WM_SYSCHAR => LRESULT(0),
        // The default answer to an Alt chord that matches no menu is a beep.
        WM_MENUCHAR => LRESULT((1isize) << 16),
        WM_WAKE => {
            with_app(hwnd, App::on_wake);
            LRESULT(0)
        }
        WM_NEW_WINDOW => {
            if let Some(gpu) = with_app(hwnd, |app| app.shared_gpu()) {
                if let Err(e) = create_window(Some(gpu), None, true) { crate::util::log(&format!("new window: {e}")); }
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            // SAFETY: lParam points to a MINMAXINFO.
            unsafe { (*(lparam.0 as *mut MINMAXINFO)).ptMinTrackSize = POINT { x: 480, y: 320 } };
            LRESULT(0)
        }
        WM_DESTROY => {
            let (app, last) = APPS.with(|apps| {
                let mut apps = apps.borrow_mut();
                let app = apps.remove(&(hwnd.0 as isize));
                (app, apps.is_empty())
            });
            if let Some(app) = app {
                RETIRED.with(|apps| apps.borrow_mut().push(app));
                if last { unsafe { PostQuitMessage(0) }; }
            }
            LRESULT(0)
        }
        // SAFETY: default handling of every other message.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The DWM backdrop the window shows through: 1 none, 2 Mica, 3 acrylic, 4 Mica Alt. Needs the frame extended over
/// the whole client area.
pub(crate) fn set_backdrop(hwnd: HWND, backdrop: Backdrop, dark: bool) {
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

/// The messages of the menu pop-up. They go to `App`, which owns what is drawn; while `App` is busy (a message sent
/// from inside one of its own calls) `with_app` finds it taken and the message is dropped.
pub(crate) unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    const VK_ESCAPE: i32 = 0x1B;
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            with_app(hwnd, App::popup_paint);
            // SAFETY: the swap chain is the content; only the update region is cleared.
            unsafe { let _ = ValidateRect(Some(hwnd), None); };
            raise_access_events(hwnd, App::access_update);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            // SAFETY: a valid request for this window.
            unsafe { let _ = TrackMouseEvent(&mut track); };
            with_app(hwnd, |app| app.popup_move(Some((lo(lparam), hi(lparam)))));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(hwnd, |app| app.popup_move(None));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with_app(hwnd, |app| app.popup_click(lo(lparam), hi(lparam)));
            LRESULT(0)
        }
        WM_CHAR => {
            with_app(hwnd, |app| app.popup_char(wparam.0 as u16));
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
            with_app(hwnd, |app| app.popup_wheel(delta));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as i32;
            if key == VK_ESCAPE {
                with_app(hwnd, App::close_popup);
            } else {
                with_app(hwnd, |app| app.popup_key(key, lparam.0 & (1 << 30) != 0));
            }
            LRESULT(0)
        }
        WM_ACTIVATE if (wparam.0 & 0xFFFF) == 0 => {
            with_app(hwnd, App::popup_deactivated);
            LRESULT(0)
        }
        WM_CLOSE => {
            with_app(hwnd, App::close_popup);
            LRESULT(0)
        }
        // SAFETY: default handling of every other message.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// A window for the application. `at` is the screen position of its top left corner (the system chooses without it);
/// `first_tab` false leaves it without a tab, for the caller to give it one.
fn create_window(shared_gpu: Option<crate::gpu::Gpu>, at: Option<(i32, i32)>, first_tab: bool) -> Result<HWND, String> {
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        // No GDI surface: the window shows the DirectComposition content only.
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("NekominalShell"),
            w!("Nekominal"),
            // Without WS_SYSMENU DWM draws no caption buttons of its own (they would sit at the top, over ours).
            WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 & !WS_SYSMENU.0),
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
        let (x, y, keep_position) = at.map_or((0, 0, SWP_NOMOVE), |(x, y)| (x, y, SET_WINDOW_POS_FLAGS(0)));
        let _ = SetWindowPos(hwnd, None, x, y, (1100.0 * scale) as i32, (700.0 * scale) as i32, keep_position | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        let app = match App::new(hwnd, client_size(hwnd), scale, shared_gpu, first_tab) {
            Ok(app) => app,
            Err(e) => { let _ = DestroyWindow(hwnd); return Err(e); }
        };
        set_backdrop(hwnd, app.backdrop(), app.dark());
        APPS.with(|apps| apps.borrow_mut().insert(hwnd.0 as isize, Rc::new(RefCell::new(app))));
        let _ = ShowWindow(hwnd, SW_SHOW);
        with_app(hwnd, |app| app.redraw());

        Ok(hwnd)
    }
}

/// A window that follows the pointer while the left button is held: a tab pulled out of a window, or a window of one tab
/// pulled by its chip. The pointer holds it `grab` pixels from its top left corner.
#[derive(Clone, Copy)]
struct Follow {
    host: HWND,
    grab: (i32, i32),
}

thread_local! {
    static FOLLOW: Cell<Option<Follow>> = const { Cell::new(None) };
}

/// The window that follows the pointer takes the pointer along; over the strip of tabs of another window it joins that
/// window and the drag goes on there. True when the message was for the window that follows.
fn follow_move(hwnd: HWND, lparam: LPARAM) -> bool {
    let Some(follow) = FOLLOW.with(Cell::get).filter(|follow| follow.host == hwnd) else { return false };
    let mut at = POINT { x: lo(lparam), y: hi(lparam) };
    // SAFETY: a window of this thread.
    unsafe {
        let _ = ClientToScreen(hwnd, &mut at);
        let _ = SetWindowPos(hwnd, None, at.x - follow.grab.0, at.y - follow.grab.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    if let Some(target) = strip_under(at, hwnd) {
        join(hwnd, target, at, follow);
    }
    true
}

/// The button was let go of while a window follows the pointer: it stays where it is.
fn end_follow(hwnd: HWND) -> bool {
    if FOLLOW.with(Cell::get).is_none_or(|follow| follow.host != hwnd) {
        return false;
    }
    FOLLOW.with(|follow| follow.set(None));
    // SAFETY: releases the capture of this thread.
    unsafe { let _ = ReleaseCapture(); }
    true
}

/// The window other than `except` whose strip of tabs is under a screen point.
fn strip_under(point: POINT, except: HWND) -> Option<HWND> {
    let windows: Vec<HWND> = APPS.with(|apps| apps.borrow().keys().map(|key| HWND(*key as *mut _)).collect());
    windows.into_iter().find(|&candidate| {
        // SAFETY: windows of this thread.
        unsafe {
            if candidate == except || IsIconic(candidate).as_bool() {
                return false;
            }
            let mut client = point;
            let _ = ScreenToClient(candidate, &mut client);
            let mut area = RECT::default();
            let _ = GetClientRect(candidate, &mut area);
            client.x >= 0 && client.x < area.right && with_app(candidate, |app| app.over_strip(client.x, client.y)).unwrap_or(false)
        }
    })
}

/// The tab of the window that follows the pointer moves into the window whose strip is under it, held by the pointer.
fn join(host: HWND, target: HWND, at: POINT, follow: Follow) {
    let scale = scale_of(host);
    let grab = (follow.grab.0 as f32 - STRIP_LEFT * scale).max(0.0) / scale;
    let Some(tab) = with_app(host, |app| app.take_tab(0)).flatten() else { return };
    if with_app(target, |app| app.adopt_tab(tab)).is_none() {
        crate::util::log("the tab could not be handed to the other window");
        return;
    }
    let mut client = at;
    // SAFETY: windows of this thread. The capture moves to the window that has the tab now; the one that gave it up is
    // closing (it has no tab left).
    unsafe {
        let _ = ScreenToClient(target, &mut client);
        with_app(target, |app| app.begin_chip_drag(client.x, client.y, grab));
        FOLLOW.with(|follow| follow.set(None));
        SetCapture(target);
        let _ = SetForegroundWindow(target);
    }
}

/// A chip was dragged out of the strip of `source`: a window of one tab follows the pointer as it is, and a tab of
/// several becomes a window of its own that follows it.
fn start_detach(source: HWND, detach: Detach) {
    let scale = scale_of(source);
    let point = POINT { x: detach.pointer.0, y: detach.pointer.1 };
    // The pointer holds the window by its chip.
    let grab = ((STRIP_LEFT * scale + detach.grab) as i32, (24.0 * scale) as i32);
    let at = (point.x - grab.0, point.y - grab.1);
    if with_app(source, |app| app.tab_count()).unwrap_or(0) < 2 {
        // SAFETY: a window of this thread.
        unsafe {
            if is_maximized(source) {
                let _ = ShowWindow(source, SW_RESTORE);
            }
            let _ = SetWindowPos(source, None, at.0, at.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        FOLLOW.with(|follow| follow.set(Some(Follow { host: source, grab })));
        with_app(source, App::cancel_drag);
        return;
    }
    let Some(gpu) = with_app(source, |app| app.shared_gpu()) else { return };
    let window = match create_window(Some(gpu), Some(at), false) {
        Ok(window) => window,
        Err(e) => {
            crate::util::log(&format!("new window: {e}"));
            return;
        }
    };
    let Some(tab) = with_app(source, |app| app.take_tab(detach.tab)).flatten() else { return };
    with_app(window, |app| app.adopt_tab(tab));
    FOLLOW.with(|follow| follow.set(Some(Follow { host: window, grab })));
    // SAFETY: the button is held; the new window takes the pointer from the one the chip came out of.
    unsafe { SetCapture(window) };
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
        create_window(None, None, true)?;

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
            reap_closed_windows();
        }
        // PTY cleanup starts threads; release the app before Windows begins thread-local teardown.
        let apps = APPS.with(|apps| std::mem::take(&mut *apps.borrow_mut()));
        drop(apps);
        reap_closed_windows();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::key_scan;
    use nekominal_input::{physical_key, vk};
    use nekominal_model::{commands::{self, Command}, keymap::{self, Modifiers}};

    #[test]
    fn virtual_key_only_paste_still_reaches_the_application_warning() {
        let keymap = keymap::build_keymap(commands::default_bindings(), &Default::default()).keymap;
        let (scan, extended) = key_scan(0x56, 0, false);
        let key = physical_key(scan, extended).expect("physical V from the active layout");
        let modifiers = Modifiers { ctrl: true, shift: true, ..Modifiers::default() };
        assert_eq!(keymap.find(modifiers, key.name()), Some(Command::EditPaste));
    }

    #[test]
    fn real_scan_codes_and_ime_are_preserved_and_extended_keys_are_recovered() {
        assert_eq!(key_scan(0x5A, 0x2F, false), (0x2F, false));
        assert_eq!(key_scan(0x25, 0x4B, true), (0x4B, true));
        assert_eq!(key_scan(vk::VK_PACKET, 0, false), (0, false));
        assert_eq!(key_scan(vk::VK_PROCESSKEY, 0, false), (0, false));
        assert_eq!(key_scan(0x7A, 0, false), (0x57, false));
        assert_eq!(key_scan(0xA3, 0, false), (0x1D, true));
    }
}
