//! The small window of the update: the versions, what is being done, the last line Scoop wrote, and a bar of the steps.
//! Drawn as the app is drawn: Direct3D over the backdrop of the settings (Mica or acrylic), in the colours of the scheme,
//! with a title bar of its own. The work runs on a thread of its own and asks the window to draw again
//! ([`Ui::refresh`]); the window cannot be closed while the tabs are held, since closing it would end them.

use std::sync::{Arc, Mutex};

use nekominal_model::themes;
use nekominal_ui::{Color, Rect};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{InvalidateRect, ScreenToClient, ValidateRect},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext},
            Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

use crate::{
    glass::{self, Glass},
    widgets::{self, State as Look, col},
};

/// Asks the window to draw the view again.
pub const WM_REFRESH: u32 = WM_APP + 1;
/// The work is over: the window may close, and closes by itself after wParam milliseconds (0: when the user closes it).
pub const WM_FINISHED: u32 = WM_APP + 2;
const CLOSE_TIMER: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02A3;

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
        unsafe { let _ = PostMessageW(Some(HWND(self.0 as *mut _)), WM_REFRESH, WPARAM(0), LPARAM(0)); }
    }

    pub fn finish(self, close_after_ms: u32) {
        // SAFETY: as above.
        unsafe { let _ = PostMessageW(Some(HWND(self.0 as *mut _)), WM_FINISHED, WPARAM(close_after_ms as usize), LPARAM(0)); }
    }
}

/// The buttons of the title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Minimize,
    Close,
}

struct State {
    view: Arc<Mutex<View>>,
    title: String,
    finished: bool,
    active: bool,
    hover: Option<Button>,
    pressed: Option<Button>,
    glass: Glass,
    backdrop: nekominal_model::settings::Backdrop,
    opacity: f64,
    gpu: crate::gpu::Gpu,
    surface: crate::gpu::Surface,
    ui: nekominal_ui::Ui,
}

thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

/// The size of the window, in DIP: it has no frame, the title bar is drawn.
const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 196.0;
const TITLE: f32 = 32.0;
const BUTTON: f32 = 46.0;
const PAD: f32 = 20.0;

/// The buttons of the title bar, from the right: close, then minimize.
fn button_rect(button: Button) -> Rect {
    let n = match button { Button::Close => 1.0, Button::Minimize => 2.0 };
    Rect::new(WIDTH - n * BUTTON, 0.0, BUTTON, TITLE)
}

fn button_at(x: f32, y: f32) -> Option<Button> {
    [Button::Minimize, Button::Close].into_iter().find(|b| {
        let r = button_rect(*b);
        x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
    })
}

fn scale(hwnd: HWND) -> f32 {
    // SAFETY: a plain query for a window of this thread.
    unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}

/// Shows the window and runs `work` on a thread with it; returns when the window closes.
pub fn run(title: &str, view: Arc<Mutex<View>>, work: impl FnOnce(Ui) + Send + 'static) {
    // SAFETY: plain Win32 calls on the thread that owns the window.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = GetModuleHandleW(None).unwrap_or_default();
        // Resource 1 of the program, the icon of the app (`nekominal.rc`).
        let icon = LoadIconW(Some(module.into()), PCWSTR(1 as _)).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: module.into(),
            lpszClassName: w!("NekominalUpdater"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: icon,
            ..Default::default()
        };
        RegisterClassW(&class);
        // Without WS_SYSMENU DWM draws no caption buttons of its own over the drawn ones, as for the main window.
        let style = WS_OVERLAPPED | WS_CAPTION | WS_MINIMIZEBOX;
        let dpi = GetDpiForSystem();
        let px = |dip: f32| (dip * dpi as f32 / 96.0).round() as i32;
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        // The frame the system would draw is taken away in WM_NCCALCSIZE: the window is the size of its content.
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("NekominalUpdater"),
            PCWSTR(wide.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            px(WIDTH),
            px(HEIGHT),
            None,
            None,
            Some(module.into()),
            None,
        );
        let Ok(hwnd) = hwnd else {
            // No window (a session without a desktop): the work still runs, and ends the process.
            work(Ui(0));
            return;
        };
        let size = |hwnd: HWND| {
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            ((client.right - client.left).max(1) as u32, (client.bottom - client.top).max(1) as u32)
        };
        let (settings, _, _) = crate::app::load_settings();
        let glass = Glass::from_scheme(themes::find(&settings.schemes, &settings.theme));
        let drawn = crate::gpu::create(hwnd, size(hwnd)).and_then(|(gpu, surface)| Ok((nekominal_ui::Ui::new(&gpu)?, gpu, surface)));
        match drawn {
            Ok((ui, gpu, surface)) => {
                crate::win::set_backdrop(hwnd, settings.appearance.backdrop, !glass.light);
                STATE.with(|state| {
                    *state.borrow_mut() = Some(State {
                        view,
                        title: title.to_owned(),
                        finished: false,
                        active: true,
                        hover: None,
                        pressed: None,
                        glass,
                        backdrop: settings.appearance.backdrop,
                        opacity: settings.appearance.opacity,
                        gpu,
                        surface,
                        ui,
                    })
                });
            }
            Err(e) => {
                // Without a device there is nothing to draw with: the work runs without the window.
                crate::util::log(&format!("hold: window: {e}"));
                let _ = DestroyWindow(hwnd);
                work(Ui(0));
                return;
            }
        }
        // Sized for the monitor the window is on (the system's DPI may be another), and the frame recomputed, now that
        // WM_NCCALCSIZE takes it away.
        let on = scale(hwnd);
        let fit = |dip: f32| (dip * on).round() as i32;
        let _ = SetWindowPos(hwnd, None, 0, 0, fit(WIDTH), fit(HEIGHT), SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOZORDER);
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let ui = Ui(hwnd.0 as isize);
        let _ = std::thread::Builder::new().name("update".into()).spawn(move || work(ui));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        STATE.with(|state| state.borrow_mut().take());
    }
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|state| state.borrow_mut().as_mut().map(f))
}

fn redraw(hwnd: HWND) {
    // SAFETY: a window of this thread.
    unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
}

/// The point of `lparam` (screen coordinates) in DIP of the window.
fn client_dip(hwnd: HWND, lparam: LPARAM, screen: bool) -> (f32, f32) {
    let mut point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
    if screen {
        // SAFETY: a window of this thread.
        unsafe { let _ = ScreenToClient(hwnd, &mut point); }
    }
    let s = scale(hwnd);
    (point.x as f32 / s, point.y as f32 / s)
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the messages of a window of this thread, handled as Win32 documents them.
    unsafe {
        match msg {
            // No frame and no caption from the system: the whole window is the client area, the title bar is drawn.
            WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
            WM_NCHITTEST => {
                let (x, y) = client_dip(hwnd, lparam, true);
                if y < TITLE && button_at(x, y).is_none() { LRESULT(HTCAPTION as isize) } else { LRESULT(HTCLIENT as isize) }
            }
            WM_ACTIVATE => {
                with_state(|state| state.active = (wparam.0 & 0xFFFF) != 0);
                redraw(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = client_dip(hwnd, lparam, false);
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut track);
                if with_state(|state| std::mem::replace(&mut state.hover, button_at(x, y)) != state.hover) == Some(true) {
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with_state(|state| state.hover = None);
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = client_dip(hwnd, lparam, false);
                with_state(|state| state.pressed = button_at(x, y));
                SetCapture(hwnd);
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = client_dip(hwnd, lparam, false);
                let _ = ReleaseCapture();
                let pressed = with_state(|state| state.pressed.take()).flatten();
                match pressed.filter(|b| button_at(x, y) == Some(*b)) {
                    Some(Button::Minimize) => { let _ = ShowWindow(hwnd, SW_MINIMIZE); }
                    Some(Button::Close) => { let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)); }
                    None => {}
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_REFRESH => {
                redraw(hwnd);
                LRESULT(0)
            }
            WM_FINISHED => {
                with_state(|state| state.finished = true);
                redraw(hwnd);
                if wparam.0 > 0 {
                    SetTimer(Some(hwnd), CLOSE_TIMER, wparam.0 as u32, None);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == CLOSE_TIMER => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            // Closing while the tabs are held would end them: only once the work is over.
            WM_CLOSE => {
                if with_state(|state| state.finished) == Some(true) {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                LRESULT(0)
            }
            WM_SIZE => {
                let size = ((lparam.0 & 0xFFFF) as u32, ((lparam.0 >> 16) & 0xFFFF) as u32);
                with_state(|state| state.surface.resize(&state.gpu, size));
                redraw(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let s = scale(hwnd);
                with_state(|state| paint(state, s));
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Draws the view: the title bar with its buttons, the versions, what is being done, Scoop's last line, the bar.
fn paint(state: &mut State, scale: f32) {
    let view = state.view.lock().unwrap().clone();
    let g = state.glass;
    let size = state.surface.size();
    let Some(target) = state.surface.acquire() else { return };
    // From nothing: the tint below is not opaque, and over the frame before it would build up, the backdrop gone and
    // the words of an earlier step showing through.
    state.gpu.clear(&target, [0.0; 4]);
    let ui = &mut state.ui;
    ui.begin_frame(size, scale);
    let (width, height) = (size.0 as f32 / scale, size.1 as f32 / scale);
    // The tint of the app's bar over the backdrop, opaque where the backdrop shows nothing.
    let opacity = glass::terminal_opacity(state.backdrop, state.opacity, glass::Material { window_active: state.active, effects_on: true });
    let alpha = (glass::bar_tint_alpha(opacity) * 255.0).round() as u8;
    ui.fill(Rect::new(0.0, 0.0, width, height), Color([g.background.r, g.background.g, g.background.b, alpha]));
    ui.fill(Rect::new(0.0, 0.0, width, height), col(g.tab_rest()));

    let (text, quiet) = (col(g.text()), col(g.text_secondary()));
    // The title bar: the name of the window, and minimize and close; close does nothing while the tabs are held.
    let title_size = 12.0;
    let line = ui.line_height(title_size);
    ui.text(14.0, (TITLE - line) / 2.0, &state.title, title_size, if state.active { text } else { quiet }, false);
    for (button, glyph) in [(Button::Minimize, '\u{E921}'), (Button::Close, '\u{E8BB}')] {
        let rect = button_rect(button);
        let enabled = button != Button::Close || state.finished;
        if enabled {
            widgets::bar_button(ui, &g, rect, Look { hover: state.hover == Some(button), pressed: state.pressed == Some(button), chosen: false });
        }
        let icon = 10.0;
        let (w, h) = (ui.measure_icon(glyph, icon), ui.line_height(icon));
        ui.icon(rect.x + (rect.w - w) / 2.0, rect.y + (rect.h - h) / 2.0 + 1.5, glyph, icon, if enabled { text } else { quiet });
    }

    let right = width - PAD;
    let room = right - PAD;
    let versions = format!("Nekominal {} \u{2192} {}", view.from, view.to.as_deref().unwrap_or("\u{2026}"));
    ui.text(PAD, TITLE + 12.0, &versions, 17.0, text, true);
    let status_color = if view.failed { col(g.mark(g.danger)) } else { text };
    let small = ui.line_height(13.0);
    for (n, row) in crate::app::wrap(ui, &view.status, 13.0, room, 2).iter().enumerate() {
        ui.text(PAD, TITLE + 46.0 + n as f32 * small, row, 13.0, status_color, false);
    }
    // Scoop's last line, cut to one line.
    if let Some(first) = crate::app::wrap(ui, &view.detail, 12.0, room - 12.0, 1).first() {
        let cut = if first.len() < view.detail.len() { format!("{}\u{2026}", first.trim_end()) } else { first.clone() };
        ui.text(PAD, TITLE + 46.0 + 2.0 * small + 6.0, &cut, 12.0, quiet, false);
    }
    // The bar: a segment per step, lit up to the one being done.
    let (top, gap) = (height - PAD - 4.0, 6.0);
    let segment = (room - gap * (STEPS as f32 - 1.0)) / STEPS as f32;
    for step in 0..STEPS {
        let x = PAD + step as f32 * (segment + gap);
        let color = if view.failed && step == view.step.min(STEPS - 1) {
            col(g.mark(g.danger))
        } else if step < view.step {
            col(g.accent)
        } else {
            col(g.divider())
        };
        ui.rrect(Rect::new(x, top, segment, 4.0), 2.0, color, None);
    }
    ui.end_frame(&target);
    state.surface.present();
}
