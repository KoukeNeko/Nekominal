//! The Linux side: the winit event loop, the windows (the title bar is ours: no decorations), and the events handed to
//! `App`, in the form `win.rs` hands them on Windows (virtual keys with scan codes, UTF-16 units, pixels, milliseconds).
//!
//! Timers are kept here (winit has none): the loop wakes for the next due one. Other threads reach a window through
//! [`post`], which sends an event to the loop.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use nekominal_input::MouseButton;
use nekominal_model::{
    keymap::Modifiers,
    session::{PixelRect, Placement},
    settings::Backdrop,
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, Ime, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11},
    window::{CursorIcon, Fullscreen, ResizeDirection, Window, WindowId, WindowLevel},
};

use crate::{
    app::{App, Capture, Cursor, Detach, PopupPointer},
    layout::Hit,
    plat::{Message, WindowCommand},
};

/// The handle of a window: the number of its winit window. The application holds it and hands it back to the functions
/// here, as it holds an `HWND` on Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hwnd(pub isize);

/// The timer that scrolls the view while a selection drag is held outside the pane.
const DRAG_TIMER: usize = 1;
/// Looks at what the panes run, for the icons of the tabs: once a second.
const JOB_TIMER: usize = 2;
/// Asks for the next frame of the marks of the tabs that move (a spinner, a pulse), while one is on show.
const MARK_TIMER: usize = 3;
/// Turns the cursor off and on, at the pace of the settings, while it blinks.
const BLINK_TIMER: usize = 4;
/// The frame output asked for while it streams, put off to the pace of `OUTPUT_FRAME` (see `App::redraw_output`).
pub(crate) const FRAME_TIMER: usize = 6;
/// Once, a while after the floating panels closed: their layer gives back the memory of its swap chain.
pub(crate) const PANELS_TIMER: usize = 7;

/// The application id of the windows (Wayland) and their class (X11).
const APP_ID: &str = "nekominal";

/// Width in pixels of the border where a press resizes the window.
const BORDER: f64 = 6.0;
/// Two presses on the title bar this close are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// One notch of a wheel, as Windows counts it.
const WHEEL_DELTA: f32 = 120.0;
/// How many pixels of a touchpad scroll are a notch.
const PIXELS_PER_NOTCH: f32 = 40.0;

enum UserEvent {
    Post(isize, Message),
    /// The automation API has requests for the windows (`api::drain`).
    Api,
}

static PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

struct Slot {
    window: Arc<Window>,
    app: Option<Rc<RefCell<App>>>,
}

struct Timer {
    window: isize,
    id: usize,
    due: Instant,
    every: Duration,
}

#[derive(Default)]
struct Pointer {
    position: (i32, i32),
    /// The button that holds the pointer in a drag outside the pane, and the timer that scrolls with it.
    captured: bool,
    caption_hit: Hit,
    last_caption_press: Option<(Instant, (i32, i32))>,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<isize, Slot>> = RefCell::new(HashMap::new());
    static RETIRED: RefCell<Vec<Rc<RefCell<App>>>> = const { RefCell::new(Vec::new()) };
    static TIMERS: RefCell<Vec<Timer>> = const { RefCell::new(Vec::new()) };
    static POINTERS: RefCell<HashMap<isize, Pointer>> = RefCell::new(HashMap::new());
    static HELD: Cell<Modifiers> = const { Cell::new(Modifiers { ctrl: false, shift: false, alt: false, meta: false }) };
    static FOCUSED: Cell<Option<isize>> = const { Cell::new(None) };
    static DROPPED: RefCell<HashMap<isize, Vec<std::path::PathBuf>>> = RefCell::new(HashMap::new());
}

fn now_ms() -> u32 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u32
}

fn id_of(window: WindowId) -> isize {
    u64::from(window) as isize
}

pub fn window_of(hwnd: Hwnd) -> Option<Arc<Window>> {
    WINDOWS.with(|windows| windows.borrow().get(&hwnd.0).map(|slot| slot.window.clone()))
}

/// Runs `f` on the application unless it is not there yet or already busy (a message sent from inside a handler).
fn with_app<T>(hwnd: Hwnd, f: impl FnOnce(&mut App) -> T) -> Option<T> {
    let app = WINDOWS.with(|windows| windows.borrow().get(&hwnd.0).and_then(|slot| slot.app.clone()))?;
    let mut app = app.try_borrow_mut().ok()?;
    Some(f(&mut app))
}

/// Every window, the one in front first, for the automation API (`api.rs`), which borrows each only as `with_app` does.
pub(crate) fn apps() -> Vec<(Hwnd, Rc<RefCell<App>>)> {
    let front = FOCUSED.with(Cell::get);
    let mut apps: Vec<(isize, Rc<RefCell<App>>)> =
        WINDOWS.with(|windows| windows.borrow().iter().filter_map(|(key, slot)| slot.app.clone().map(|app| (*key, app))).collect());
    apps.sort_by_key(|(key, _)| (Some(*key) != front, *key));
    apps.into_iter().map(|(key, app)| (Hwnd(key), app)).collect()
}

// --- what the application asks of its window -------------------------------------------------------------------------

/// The number a window is known by outside the application: its automation id, the target of a thread's wake.
pub fn id(hwnd: Hwnd) -> isize {
    hwnd.0
}

pub fn from_id(id: isize) -> Hwnd {
    Hwnd(id)
}

/// Where the window is on the screen; none where the system does not say (Wayland).
pub fn origin(hwnd: Hwnd) -> Option<(i32, i32)> {
    let position = window_of(hwnd)?.outer_position().ok()?;
    Some((position.x, position.y))
}

/// A window of the application is a part of the window it belongs to here, never a window of its own that has to follow.
pub fn move_by(_hwnd: Hwnd, _dx: i32, _dy: i32) {}

/// Whether the system plays no sounds for the deaf: a program here has no way of asking.
pub fn show_sounds() -> bool {
    false
}

/// The bell of the terminal: the one the desktop has for its alerts, asked for by the `canberra-gtk-play` of the desktop
/// when there is one, else the bell character on the terminal this program was started from.
pub fn beep() {
    if crate::util::on_path("canberra-gtk-play") {
        let _ = std::process::Command::new("canberra-gtk-play").args(["--id=bell", "--description=Nekominal"]).spawn();
    }
}

/// The whole window over the monitor it is on, and what is needed to put it back.
pub struct Restore;

pub fn enter_fullscreen(hwnd: Hwnd) -> Option<Restore> {
    set_fullscreen(hwnd, true);
    Some(Restore)
}

pub fn leave_fullscreen(hwnd: Hwnd, _restore: Restore) {
    set_fullscreen(hwnd, false);
}

/// Whether the device of the window was lost.
pub fn device_lost(gpu: &crate::gpu::Gpu) -> bool {
    gpu.is_lost()
}

/// The window along the top of the monitor it is on, as wide as it and `share` of its height. A program cannot place its
/// windows here (Wayland): the window is asked to be the size and stays where the compositor puts it.
pub fn dock_at_top(hwnd: Hwnd, share: f32) {
    if let Some(window) = window_of(hwnd) {
        let monitor = virtual_screen(hwnd);
        let _ = window.request_inner_size(PhysicalSize::new(monitor.width, (monitor.height as f32 * share).round() as u32));
    }
}

/// Destroys a window of the application. The windows that need it (the pop-ups) are not made here.
pub fn destroy(_hwnd: Hwnd) {}

/// Brings the window up from wherever it is: restored if it was minimized, shown if it was hidden, and in front.
pub fn raise(hwnd: Hwnd) {
    if let Some(window) = window_of(hwnd) {
        if window.is_minimized().unwrap_or(false) {
            window.set_minimized(false);
        }
        window.set_visible(true);
        window.focus_window();
    }
}

/// Tells the loop there are requests of the automation API to answer.
pub fn post_api() -> bool {
    PROXY.get().is_some_and(|proxy| proxy.send_event(UserEvent::Api).is_ok())
}

/// Sends `message` to the window; any thread may.
pub fn post(hwnd: Hwnd, message: Message) -> bool {
    PROXY.get().is_some_and(|proxy| proxy.send_event(UserEvent::Post(hwnd.0, message)).is_ok())
}

/// Asks for a frame of the window.
pub fn invalidate(hwnd: Hwnd) {
    if let Some(window) = window_of(hwnd) {
        window.request_redraw();
    }
}

/// Starts (or restarts) a timer of the window that fires every `ms`. Always true.
pub fn set_timer(hwnd: Hwnd, id: usize, ms: u32) -> bool {
    let every = Duration::from_millis(u64::from(ms.max(1)));
    TIMERS.with(|timers| {
        let mut timers = timers.borrow_mut();
        timers.retain(|timer| !(timer.window == hwnd.0 && timer.id == id));
        timers.push(Timer { window: hwnd.0, id, due: Instant::now() + every, every });
    });
    true
}

pub fn kill_timer(hwnd: Hwnd, id: usize) {
    TIMERS.with(|timers| timers.borrow_mut().retain(|timer| !(timer.window == hwnd.0 && timer.id == id)));
}

pub fn set_title(hwnd: Hwnd, title: &str) {
    if let Some(window) = window_of(hwnd) {
        window.set_title(title);
    }
}

/// Draws the attention of the user to the window.
pub fn flash(hwnd: Hwnd) {
    if let Some(window) = window_of(hwnd) {
        window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
    }
}

pub fn is_foreground(hwnd: Hwnd) -> bool {
    FOCUSED.with(Cell::get) == Some(hwnd.0)
}

pub fn client_size(hwnd: Hwnd) -> (u32, u32) {
    window_of(hwnd).map_or((1, 1), |window| {
        let size = window.inner_size();
        (size.width.max(1), size.height.max(1))
    })
}

pub fn is_maximized(hwnd: Hwnd) -> bool {
    window_of(hwnd).is_some_and(|window| window.is_maximized())
}

pub fn is_minimized(hwnd: Hwnd) -> bool {
    window_of(hwnd).is_some_and(|window| window.is_minimized().unwrap_or(false))
}

pub fn is_visible(hwnd: Hwnd) -> bool {
    window_of(hwnd).is_some_and(|window| window.is_visible().unwrap_or(true))
}

/// The DIP scale of the window.
pub fn scale_of(hwnd: Hwnd) -> f32 {
    window_of(hwnd).map_or(1.0, |window| window.scale_factor() as f32)
}

/// A point of the client area as a point of the screen. A program cannot place its windows by the screen here (Wayland),
/// and its pop-ups are drawn in the window: a point of the screen is a point of the window.
pub fn client_to_screen(_hwnd: Hwnd, point: (i32, i32)) -> (i32, i32) {
    point
}

/// The rectangle of the whole screen of the user: the monitor of the window.
pub fn virtual_screen(hwnd: Hwnd) -> PixelRect {
    let monitor = window_of(hwnd).and_then(|window| window.current_monitor());
    monitor.map_or(PixelRect { x: 0, y: 0, width: 1920, height: 1080 }, |monitor| {
        let (position, size) = (monitor.position(), monitor.size());
        PixelRect { x: position.x, y: position.y, width: size.width, height: size.height }
    })
}

/// Where the window is, for the session file.
pub fn placement(hwnd: Hwnd) -> Option<Placement> {
    let window = window_of(hwnd)?;
    let (position, size) = (window.outer_position().unwrap_or(PhysicalPosition::new(0, 0)), window.inner_size());
    Some(Placement { rect: PixelRect { x: position.x, y: position.y, width: size.width, height: size.height }, maximized: window.is_maximized() })
}

pub fn set_fullscreen(hwnd: Hwnd, on: bool) {
    if let Some(window) = window_of(hwnd) {
        window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
    }
}

pub fn is_fullscreen(hwnd: Hwnd) -> bool {
    window_of(hwnd).is_some_and(|window| window.fullscreen().is_some())
}

/// Keeps the window above the others; where the system lets a program ask.
pub fn set_topmost(hwnd: Hwnd, on: bool) {
    if let Some(window) = window_of(hwnd) {
        window.set_window_level(if on { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal });
    }
}

/// The backdrop of the window: a compositor blurs nothing for a program here, so the translucency is the alpha of the
/// frame alone, which `gpu_unix` asks the compositor to honour.
pub fn set_backdrop(hwnd: Hwnd, _backdrop: Backdrop, _dark: bool) {
    if let Some(window) = window_of(hwnd) {
        window.set_transparent(true);
    }
}

/// The modifier keys that are down.
pub fn held() -> Modifiers {
    HELD.with(Cell::get)
}

/// A key press that typed a character is followed by its text in the same event of the keyboard: whether it has some
/// is told to the application before the press, as `PeekMessageW` tells it on Windows.
pub fn char_follows(_hwnd: Hwnd) -> bool {
    CHAR_FOLLOWS.with(Cell::get)
}

thread_local! {
    static CHAR_FOLLOWS: Cell<bool> = const { Cell::new(false) };
}

/// The language of the user, as a locale name such as `zh-TW`.
pub fn locale_name() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| std::env::var(name).ok().and_then(|value| locale_name_of(&value)))
}

/// `zh_TW.UTF-8` as `zh-TW`; none for the locale of no language (`C`, `POSIX`).
fn locale_name_of(raw: &str) -> Option<String> {
    let name = raw.split(['.', '@']).next()?;
    (!name.is_empty() && name != "C" && name != "POSIX").then(|| name.replace('_', "-"))
}

/// Asks every window of the shell to close, each with the check of its busy shells, as its close button does.
pub fn close_all_windows() {
    let windows: Vec<isize> = WINDOWS.with(|windows| windows.borrow().keys().copied().collect());
    for window in windows {
        post(Hwnd(window), Message::Close);
    }
}

// --- the loop ----------------------------------------------------------------------------------------------------------

struct Shell {
    first: bool,
}

/// Keys that are not text, by their place on the keyboard: the scan code of the first set, whether it has the extended
/// prefix, and the virtual key Windows names it by.
fn physical_key(code: KeyCode) -> Option<(u16, bool, i32)> {
    use KeyCode::*;
    Some(match code {
        Escape => (0x01, false, 0x1B),
        Digit1 => (0x02, false, 0x31),
        Digit2 => (0x03, false, 0x32),
        Digit3 => (0x04, false, 0x33),
        Digit4 => (0x05, false, 0x34),
        Digit5 => (0x06, false, 0x35),
        Digit6 => (0x07, false, 0x36),
        Digit7 => (0x08, false, 0x37),
        Digit8 => (0x09, false, 0x38),
        Digit9 => (0x0A, false, 0x39),
        Digit0 => (0x0B, false, 0x30),
        Minus => (0x0C, false, 0xBD),
        Equal => (0x0D, false, 0xBB),
        Backspace => (0x0E, false, 0x08),
        Tab => (0x0F, false, 0x09),
        KeyQ => (0x10, false, 0x51),
        KeyW => (0x11, false, 0x57),
        KeyE => (0x12, false, 0x45),
        KeyR => (0x13, false, 0x52),
        KeyT => (0x14, false, 0x54),
        KeyY => (0x15, false, 0x59),
        KeyU => (0x16, false, 0x55),
        KeyI => (0x17, false, 0x49),
        KeyO => (0x18, false, 0x4F),
        KeyP => (0x19, false, 0x50),
        BracketLeft => (0x1A, false, 0xDB),
        BracketRight => (0x1B, false, 0xDD),
        Enter => (0x1C, false, 0x0D),
        ControlLeft => (0x1D, false, 0x11),
        KeyA => (0x1E, false, 0x41),
        KeyS => (0x1F, false, 0x53),
        KeyD => (0x20, false, 0x44),
        KeyF => (0x21, false, 0x46),
        KeyG => (0x22, false, 0x47),
        KeyH => (0x23, false, 0x48),
        KeyJ => (0x24, false, 0x4A),
        KeyK => (0x25, false, 0x4B),
        KeyL => (0x26, false, 0x4C),
        Semicolon => (0x27, false, 0xBA),
        Quote => (0x28, false, 0xDE),
        Backquote => (0x29, false, 0xC0),
        ShiftLeft => (0x2A, false, 0x10),
        Backslash => (0x2B, false, 0xDC),
        KeyZ => (0x2C, false, 0x5A),
        KeyX => (0x2D, false, 0x58),
        KeyC => (0x2E, false, 0x43),
        KeyV => (0x2F, false, 0x56),
        KeyB => (0x30, false, 0x42),
        KeyN => (0x31, false, 0x4E),
        KeyM => (0x32, false, 0x4D),
        Comma => (0x33, false, 0xBC),
        Period => (0x34, false, 0xBE),
        Slash => (0x35, false, 0xBF),
        ShiftRight => (0x36, false, 0x10),
        NumpadMultiply => (0x37, false, 0x6A),
        AltLeft => (0x38, false, 0x12),
        Space => (0x39, false, 0x20),
        CapsLock => (0x3A, false, 0x14),
        F1 => (0x3B, false, 0x70),
        F2 => (0x3C, false, 0x71),
        F3 => (0x3D, false, 0x72),
        F4 => (0x3E, false, 0x73),
        F5 => (0x3F, false, 0x74),
        F6 => (0x40, false, 0x75),
        F7 => (0x41, false, 0x76),
        F8 => (0x42, false, 0x77),
        F9 => (0x43, false, 0x78),
        F10 => (0x44, false, 0x79),
        NumLock => (0x45, false, 0x90),
        ScrollLock => (0x46, false, 0x91),
        Numpad7 => (0x47, false, 0x67),
        Numpad8 => (0x48, false, 0x68),
        Numpad9 => (0x49, false, 0x69),
        NumpadSubtract => (0x4A, false, 0x6D),
        Numpad4 => (0x4B, false, 0x64),
        Numpad5 => (0x4C, false, 0x65),
        Numpad6 => (0x4D, false, 0x66),
        NumpadAdd => (0x4E, false, 0x6B),
        Numpad1 => (0x4F, false, 0x61),
        Numpad2 => (0x50, false, 0x62),
        Numpad3 => (0x51, false, 0x63),
        Numpad0 => (0x52, false, 0x60),
        NumpadDecimal => (0x53, false, 0x6E),
        IntlBackslash => (0x56, false, 0xE2),
        F11 => (0x57, false, 0x7A),
        F12 => (0x58, false, 0x7B),
        F13 => (0x64, false, 0x7C),
        F14 => (0x65, false, 0x7D),
        F15 => (0x66, false, 0x7E),
        F16 => (0x67, false, 0x7F),
        F17 => (0x68, false, 0x80),
        F18 => (0x69, false, 0x81),
        F19 => (0x6A, false, 0x82),
        F20 => (0x6B, false, 0x83),
        F21 => (0x6C, false, 0x84),
        F22 => (0x6D, false, 0x85),
        F23 => (0x6E, false, 0x86),
        F24 => (0x76, false, 0x87),
        NumpadEnter => (0x1C, true, 0x0D),
        ControlRight => (0x1D, true, 0x11),
        NumpadDivide => (0x35, true, 0x6F),
        PrintScreen => (0x37, true, 0x2C),
        AltRight => (0x38, true, 0x12),
        Pause => (0x45, true, 0x13),
        Home => (0x47, true, 0x24),
        ArrowUp => (0x48, true, 0x26),
        PageUp => (0x49, true, 0x21),
        ArrowLeft => (0x4B, true, 0x25),
        ArrowRight => (0x4D, true, 0x27),
        End => (0x4F, true, 0x23),
        ArrowDown => (0x50, true, 0x28),
        PageDown => (0x51, true, 0x22),
        Insert => (0x52, true, 0x2D),
        Delete => (0x53, true, 0x2E),
        SuperLeft => (0x5B, true, 0x5B),
        SuperRight => (0x5C, true, 0x5C),
        ContextMenu => (0x5D, true, 0x5D),
        _ => return None,
    })
}

/// The virtual key a key press has: the letter or digit it types when it types one (the virtual keys of letters follow the
/// layout), otherwise the key of its place.
fn virtual_key(logical: &Key, place: i32) -> i32 {
    if let Key::Character(text) = logical
        && let Some(ch) = text.chars().next().filter(|_| text.chars().count() == 1)
        && ch.is_ascii_alphanumeric()
    {
        return ch.to_ascii_uppercase() as i32;
    }
    place
}

fn mouse_button(button: winit::event::MouseButton) -> Option<MouseButton> {
    use winit::event::MouseButton as Winit;
    match button {
        Winit::Left => Some(MouseButton::Left),
        Winit::Middle => Some(MouseButton::Middle),
        Winit::Right => Some(MouseButton::Right),
        _ => None,
    }
}

/// The side of the window a press at `position` (pixels in the window) resizes, if it is on the border.
fn edge_at(window: &Window, (x, y): (i32, i32)) -> Option<ResizeDirection> {
    if window.is_maximized() || window.fullscreen().is_some() {
        return None;
    }
    let size = window.inner_size();
    let (x, y, b) = (f64::from(x), f64::from(y), BORDER * window.scale_factor());
    let (left, right) = (x < b, x >= f64::from(size.width) - b);
    let (top, bottom) = (y < b, y >= f64::from(size.height) - b);
    Some(match (left, right, top, bottom) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

fn resize_cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::NsResize,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::EwResize,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::NwseResize,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::NeswResize,
    }
}

/// Makes a window for the application. `first_tab` false leaves it without a tab, for the caller to give it one.
fn create_window(el: &ActiveEventLoop, shared_gpu: Option<crate::gpu::Gpu>, first_tab: bool) -> Result<Hwnd, String> {
    crate::effects::refresh();
    let attributes = Window::default_attributes()
        .with_title("Nekominal")
        .with_inner_size(LogicalSize::new(1100.0, 700.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0))
        .with_decorations(false)
        .with_transparent(true)
        .with_visible(false);
    // The desktop finds the launcher and the icon of the window by this name (`distribution/nekominal.desktop`).
    let attributes = WindowAttributesExtWayland::with_name(attributes, APP_ID, "");
    let attributes = WindowAttributesExtX11::with_name(attributes, APP_ID, APP_ID);
    let window = Arc::new(el.create_window(attributes).map_err(|e| e.to_string())?);
    window.set_ime_allowed(true);
    let hwnd = Hwnd(id_of(window.id()));
    WINDOWS.with(|windows| windows.borrow_mut().insert(hwnd.0, Slot { window: window.clone(), app: None }));
    let size = window.inner_size();
    let app = match App::new(hwnd, (size.width.max(1), size.height.max(1)), window.scale_factor() as f32, shared_gpu, first_tab) {
        Ok(app) => app,
        Err(e) => {
            WINDOWS.with(|windows| windows.borrow_mut().remove(&hwnd.0));
            return Err(e);
        }
    };
    set_backdrop(hwnd, app.backdrop(), app.dark());
    WINDOWS.with(|windows| {
        if let Some(slot) = windows.borrow_mut().get_mut(&hwnd.0) {
            slot.app = Some(Rc::new(RefCell::new(app)));
        }
    });
    // The window of the session that was restored goes where it was and as big as it was.
    if let Some(placement) = with_app(hwnd, |app| app.take_placement()).flatten() {
        let rect = placement.rect;
        if rect.width > 0 && rect.height > 0 {
            let _ = window.request_inner_size(PhysicalSize::new(rect.width, rect.height));
        }
        window.set_maximized(placement.maximized);
    }
    set_timer(hwnd, JOB_TIMER, 1000);
    window.set_visible(true);
    window.focus_window();
    with_app(hwnd, |app| app.redraw());
    Ok(hwnd)
}

/// A tab pulled out of the strip of `source` becomes a window of its own; a window of one tab stays where it is (a program
/// cannot move its windows here).
fn start_detach(el: &ActiveEventLoop, source: Hwnd, detach: Detach) {
    if with_app(source, |app| app.tab_count()).unwrap_or(0) < 2 {
        with_app(source, App::cancel_drag);
        return;
    }
    let Some(gpu) = with_app(source, |app| app.shared_gpu()) else { return };
    let window = match create_window(el, Some(gpu), false) {
        Ok(window) => window,
        Err(e) => {
            crate::util::log(&format!("new window: {e}"));
            return;
        }
    };
    let Some(tab) = with_app(source, |app| app.take_tab(detach.tab)).flatten() else { return };
    with_app(window, |app| app.adopt_tab(tab));
    with_app(source, App::cancel_drag);
}

impl Shell {
    fn handle_message(&mut self, el: &ActiveEventLoop, hwnd: Hwnd, message: Message) {
        match message {
            Message::Wake => {
                with_app(hwnd, App::on_wake);
            }
            Message::GpuLost { forced } => recover_gpu(hwnd, forced),
            Message::Close => {
                if with_app(hwnd, App::request_close_window) == Some(true) {
                    self.destroy(el, hwnd);
                }
            }
            Message::DestroyWindow => self.destroy(el, hwnd),
            Message::NewWindow => {
                if let Some(gpu) = with_app(hwnd, |app| app.shared_gpu())
                    && let Err(e) = create_window(el, Some(gpu), true)
                {
                    crate::util::log(&format!("new window: {e}"));
                }
            }
            Message::Quake => crate::util::log("the drop-down window needs a global hotkey, which this system gives no program"),
            Message::Agents => open_agents_panel(hwnd),
            Message::ShowPane { tab, pane } => {
                with_app(hwnd, |app| app.show_pane(tab, pane));
            }
            Message::HandOff | Message::Elevated(_) => {}
            Message::Window(command) => match command {
                WindowCommand::Minimize => {
                    if let Some(window) = window_of(hwnd) {
                        window.set_minimized(true);
                    }
                }
                WindowCommand::Maximize | WindowCommand::Restore => {
                    if let Some(window) = window_of(hwnd) {
                        window.set_maximized(command == WindowCommand::Maximize);
                    }
                }
                WindowCommand::Move => {
                    if let Some(window) = window_of(hwnd) {
                        let _ = window.drag_window();
                    }
                }
                WindowCommand::Size => {
                    if let Some(window) = window_of(hwnd) {
                        let _ = window.drag_resize_window(ResizeDirection::SouthEast);
                    }
                }
                WindowCommand::Close => self.handle_message(el, hwnd, Message::Close),
            },
        }
    }

    /// Takes the window and its application out of the loop; the last one saves the session and ends the loop.
    fn destroy(&mut self, el: &ActiveEventLoop, hwnd: Hwnd) {
        let (slot, last) = WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            let slot = windows.remove(&hwnd.0);
            (slot, windows.is_empty())
        });
        TIMERS.with(|timers| timers.borrow_mut().retain(|timer| timer.window != hwnd.0));
        POINTERS.with(|pointers| pointers.borrow_mut().remove(&hwnd.0));
        if let Some(app) = slot.and_then(|slot| slot.app) {
            // The last window writes what the next run starts from.
            if last && let Ok(app) = app.try_borrow() {
                app.save_session();
            }
            RETIRED.with(|apps| apps.borrow_mut().push(app));
        }
        if last {
            el.exit();
        }
    }

    fn fire_timer(&mut self, hwnd: Hwnd, id: usize) {
        match id {
            API_TIMER => api_drain(),
            DRAG_TIMER => {
                with_app(hwnd, App::on_drag_tick);
            }
            BLINK_TIMER => {
                with_app(hwnd, App::on_blink_tick);
            }
            MARK_TIMER => {
                with_app(hwnd, |app| app.redraw());
            }
            FRAME_TIMER => {
                with_app(hwnd, App::on_frame_timer);
            }
            PANELS_TIMER => {
                with_app(hwnd, App::shrink_panels);
            }
            JOB_TIMER => {
                with_app(hwnd, App::on_job_tick);
            }
            _ => {}
        }
    }

    fn key(&mut self, hwnd: Hwnd, event: &winit::event::KeyEvent) {
        let place = match event.physical_key {
            PhysicalKey::Code(code) => physical_key(code),
            PhysicalKey::Unidentified(_) => None,
        };
        let Some((scan, extended, place_vk)) = place else {
            // A key with no place on the keyboard of the first scan code set: a named key of the layout, or text only.
            if event.state == ElementState::Pressed {
                self.text(hwnd, event);
            }
            return;
        };
        let vk = match &event.logical_key {
            Key::Named(NamedKey::Process) => nekominal_input::vk::VK_PROCESSKEY,
            logical => virtual_key(logical, place_vk),
        };
        match event.state {
            ElementState::Pressed => {
                CHAR_FOLLOWS.with(|follows| follows.set(event.text.as_ref().is_some_and(|text| text.chars().any(|c| c as u32 >= 0x20))));
                with_app(hwnd, |app| app.on_key_down(vk, scan, extended, event.repeat));
                CHAR_FOLLOWS.with(|follows| follows.set(false));
                self.text(hwnd, event);
            }
            ElementState::Released => {
                with_app(hwnd, |app| app.on_key_up(vk, scan, extended));
            }
        }
    }

    /// The text a key press typed, one UTF-16 unit at a time.
    fn text(&mut self, hwnd: Hwnd, event: &winit::event::KeyEvent) {
        let Some(text) = &event.text else { return };
        for unit in text.encode_utf16() {
            with_app(hwnd, |app| app.on_char(unit));
        }
    }

    fn pointer<T>(&mut self, hwnd: Hwnd, f: impl FnOnce(&mut Pointer) -> T) -> T {
        POINTERS.with(|pointers| f(pointers.borrow_mut().entry(hwnd.0).or_default()))
    }

    fn cursor_moved(&mut self, hwnd: Hwnd, position: PhysicalPosition<f64>) {
        let at = (position.x.round() as i32, position.y.round() as i32);
        let captured = self.pointer(hwnd, |pointer| {
            pointer.position = at;
            pointer.captured
        });
        let Some(window) = window_of(hwnd) else { return };
        // A pop-up is drawn in the window: the pointer over it is the pop-up's.
        if with_app(hwnd, |app| app.pointer_for_popup(at.0, at.1, PopupPointer::Move)) == Some(true) {
            window.set_cursor(CursorIcon::Default);
            return;
        }
        // The border: a resize cursor, and no mouse move for the application.
        if !captured && let Some(edge) = edge_at(&window, at) {
            window.set_cursor(resize_cursor(edge));
            return;
        }
        let hit = with_app(hwnd, |app| app.hit_test(at.0, at.1)).unwrap_or(Hit::None);
        let caption = matches!(hit, Hit::Minimize | Hit::Maximize | Hit::Close).then_some(hit);
        let before = self.pointer(hwnd, |pointer| std::mem::replace(&mut pointer.caption_hit, caption.unwrap_or(Hit::None)));
        if caption.unwrap_or(Hit::None) != before {
            with_app(hwnd, |app| app.on_caption_hover(caption.unwrap_or(Hit::None)));
        }
        let cursor = with_app(hwnd, |app| app.cursor()).flatten();
        window.set_cursor(match cursor {
            Some(Cursor::ResizeH) => CursorIcon::EwResize,
            Some(Cursor::ResizeV) => CursorIcon::NsResize,
            Some(Cursor::Hand) => CursorIcon::Pointer,
            None => CursorIcon::Default,
        });
        if caption.is_some() {
            return;
        }
        with_app(hwnd, |app| app.on_mouse_move(at.0, at.1, now_ms()));
    }

    fn button(&mut self, el: &ActiveEventLoop, hwnd: Hwnd, button: winit::event::MouseButton, state: ElementState) {
        let Some(window) = window_of(hwnd) else { return };
        let down = state == ElementState::Pressed;
        let at = self.pointer(hwnd, |pointer| pointer.position);
        // A press outside a pop-up closes it, and one on it is its own; a button let go of on it chooses.
        let event = if down { PopupPointer::Press } else { PopupPointer::Release };
        if with_app(hwnd, |app| app.pointer_for_popup(at.0, at.1, event)) == Some(true) {
            return;
        }
        if button == winit::event::MouseButton::Left {
            // A press on the border resizes.
            if down && !with_app(hwnd, |app| app.is_fullscreen()).unwrap_or(false) && let Some(edge) = edge_at(&window, at) {
                let _ = window.drag_resize_window(edge);
                return;
            }
            let hit = with_app(hwnd, |app| app.hit_test(at.0, at.1)).unwrap_or(Hit::None);
            let popup_open = with_app(hwnd, |app| app.popup_keeps_owner_active()).unwrap_or(false);
            // A press on the title bar while a menu is open closes the menu and does nothing else.
            if down && popup_open && matches!(hit, Hit::Drag | Hit::Minimize | Hit::Maximize | Hit::Close) {
                with_app(hwnd, App::close_popup);
                return;
            }
            match hit {
                Hit::Minimize | Hit::Maximize | Hit::Close => {
                    if down {
                        with_app(hwnd, |app| app.on_caption_press(hit));
                    } else if with_app(hwnd, |app| app.on_caption_release(hit)) == Some(true) {
                        let command = match hit {
                            Hit::Minimize => WindowCommand::Minimize,
                            Hit::Maximize if window.is_maximized() => WindowCommand::Restore,
                            Hit::Maximize => WindowCommand::Maximize,
                            _ => WindowCommand::Close,
                        };
                        self.handle_message(el, hwnd, Message::Window(command));
                    }
                    return;
                }
                Hit::Drag if down && !with_app(hwnd, |app| app.is_fullscreen()).unwrap_or(false) => {
                    let now = Instant::now();
                    let double = self.pointer(hwnd, |pointer| {
                        let double = pointer.last_caption_press.is_some_and(|(at_time, _)| now.duration_since(at_time) < DOUBLE_CLICK);
                        pointer.last_caption_press = Some((now, at));
                        double
                    });
                    if double {
                        window.set_maximized(!window.is_maximized());
                    } else {
                        let _ = window.drag_window();
                    }
                    return;
                }
                _ => {}
            }
        }
        if button == winit::event::MouseButton::Right && down {
            let hit = with_app(hwnd, |app| app.hit_test(at.0, at.1)).unwrap_or(Hit::None);
            if hit == Hit::Drag {
                with_app(hwnd, |app| app.open_window_menu(hwnd, at.0, at.1));
                return;
            }
        }
        let Some(button) = mouse_button(button) else { return };
        let capture = with_app(hwnd, |app| app.on_button(at.0, at.1, button, down, now_ms()));
        match capture {
            // The pointer is held by the system while a button is down; a timer scrolls the view while the drag is held
            // above or below the pane.
            Some(Capture::Take) => {
                self.pointer(hwnd, |pointer| pointer.captured = true);
                set_timer(hwnd, DRAG_TIMER, 50);
            }
            Some(Capture::Release) => {
                self.pointer(hwnd, |pointer| pointer.captured = false);
                kill_timer(hwnd, DRAG_TIMER);
            }
            _ => {}
        }
        if let Some(detach) = with_app(hwnd, App::take_detach).flatten() {
            start_detach(el, hwnd, detach);
        }
    }

    fn redraw(&mut self, hwnd: Hwnd) {
        let started = Instant::now();
        let next_frame = with_app(hwnd, App::paint).unwrap_or(false);
        if started.elapsed() > Duration::from_millis(24) {
            crate::util::log(&format!("slow frame: {:?}", started.elapsed()));
        }
        // A slide of the tab chips asks for its next frame, and a window with every chip in its place asks for none.
        if next_frame {
            with_app(hwnd, |app| app.redraw());
        }
        // A moving mark asks for frames at its own pace, not at every refresh: the timer stops with the last of them.
        if with_app(hwnd, |app| app.animating()).unwrap_or(false) {
            set_timer(hwnd, MARK_TIMER, crate::motion::MARK_FRAME_MS);
        } else {
            kill_timer(hwnd, MARK_TIMER);
        }
        // The blink keeps its own pace: its timer changes only when the blink starts, stops or changes speed.
        match with_app(hwnd, |app| app.blink_timer()).flatten() {
            Some(Some(ms)) => {
                set_timer(hwnd, BLINK_TIMER, ms);
            }
            Some(None) => kill_timer(hwnd, BLINK_TIMER),
            None => {}
        }
        // The input method's windows go where the cursor cell is.
        if let Some((x, y, height)) = with_app(hwnd, |app| app.ime_anchor()).flatten()
            && let Some(window) = window_of(hwnd)
        {
            window.set_ime_cursor_area(PhysicalPosition::new(x, y), PhysicalSize::new(2, height.max(1) as u32));
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, hwnd: Hwnd, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.handle_message(el, hwnd, Message::Close),
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    let maximized = is_maximized(hwnd);
                    with_app(hwnd, |app| app.on_size((size.width, size.height), maximized));
                }
            }
            WindowEvent::Moved(_) => {
                with_app(hwnd, App::on_move);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                with_app(hwnd, |app| app.on_dpi(scale_factor as f32));
            }
            WindowEvent::RedrawRequested => self.redraw(hwnd),
            WindowEvent::Focused(active) => {
                FOCUSED.with(|focused| {
                    if active {
                        focused.set(Some(hwnd.0));
                    } else if focused.get() == Some(hwnd.0) {
                        focused.set(None);
                    }
                });
                with_app(hwnd, |app| app.on_activate(active));
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                HELD.with(|held| {
                    held.set(Modifiers { ctrl: state.control_key(), shift: state.shift_key(), alt: state.alt_key(), meta: state.super_key() })
                });
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(hwnd, &event),
            WindowEvent::CursorMoved { position, .. } => self.cursor_moved(hwnd, position),
            WindowEvent::CursorLeft { .. } => {
                if !with_app(hwnd, |app| app.pointer_on_panels()).unwrap_or(false) {
                    with_app(hwnd, App::on_mouse_leave);
                }
                self.pointer(hwnd, |pointer| pointer.caption_hit = Hit::None);
                with_app(hwnd, |app| {
                    app.on_caption_hover(Hit::None);
                    app.on_caption_press(Hit::None);
                });
            }
            WindowEvent::MouseInput { state, button, .. } => self.button(el, hwnd, button, state),
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines,
                    MouseScrollDelta::PixelDelta(pixels) => pixels.y as f32 / PIXELS_PER_NOTCH,
                };
                let at = self.pointer(hwnd, |pointer| pointer.position);
                let amount = (notches * WHEEL_DELTA).round().clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
                if amount != 0 && with_app(hwnd, |app| app.wheel_for_popup(at.0, at.1, amount)) != Some(true) {
                    with_app(hwnd, |app| app.on_wheel(at.0, at.1, amount, now_ms()));
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, _) => {
                    with_app(hwnd, |app| app.on_ime_preedit((!text.is_empty()).then_some(text)));
                }
                Ime::Commit(text) => {
                    with_app(hwnd, |app| {
                        app.on_ime_preedit(None);
                        app.on_ime_commit(&text);
                    });
                }
                Ime::Enabled | Ime::Disabled => {
                    with_app(hwnd, |app| app.on_ime_preedit(None));
                }
            },
            WindowEvent::DroppedFile(path) => {
                DROPPED.with(|dropped| dropped.borrow_mut().entry(hwnd.0).or_default().push(path));
            }
            _ => {}
        }
    }
}

/// The timer, of no window, that tries the requests of the API again: a window that was busy did not take them.
const API_TIMER: usize = 8;

/// Answers what the automation API has queued; a window that was busy leaves some for the next try.
fn api_drain() {
    kill_timer(Hwnd(0), API_TIMER);
    if !crate::api::drain() {
        set_timer(Hwnd(0), API_TIMER, 50);
    }
}

/// Makes everything again on a new device, once no window is borrowed.
fn recover_gpu(hwnd: Hwnd, forced: bool) {
    if !forced && with_app(hwnd, |app| app.gpu_works()).unwrap_or(true) {
        return;
    }
    let gpu = match crate::gpu::device_for(hwnd) {
        Ok(gpu) => gpu,
        Err(e) => {
            crate::util::log(&format!("device lost, and no new one: {e}"));
            return;
        }
    };
    let apps: Vec<Rc<RefCell<App>>> = WINDOWS.with(|windows| windows.borrow().values().filter_map(|slot| slot.app.clone()).collect());
    for app in apps {
        if let Ok(mut app) = app.try_borrow_mut()
            && let Err(e) = app.recreate_gpu(&gpu)
        {
            crate::util::log(&format!("device lost: {e}"));
        }
    }
}

/// The agents panel on `hwnd`, listing the panes of every window: this one's first, then the others by handle.
fn open_agents_panel(hwnd: Hwnd) {
    let mut windows = apps();
    windows.sort_by_key(|(key, _)| (*key != hwnd, key.0));
    let seen: Vec<_> = windows.iter().enumerate().filter_map(|(order, (_, app))| app.try_borrow().ok().map(|app| app.agents_seen(order))).flatten().collect();
    with_app(hwnd, |app| app.open_agents(seen));
}

impl ApplicationHandler<UserEvent> for Shell {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if std::mem::take(&mut self.first) {
            if let Err(e) = create_window(el, None, true) {
                crate::util::log(&format!("first window: {e}"));
                std::fs::write(std::env::temp_dir().join("nekominal-next.log"), format!("{e}\n")).ok();
                el.exit();
            }
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Post(window, message) => self.handle_message(el, Hwnd(window), message),
            UserEvent::Api => api_drain(),
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, window: WindowId, event: WindowEvent) {
        self.window_event(el, Hwnd(id_of(window)), event);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // Files dropped come one event each; they go to the application together.
        let dropped: Vec<(isize, Vec<std::path::PathBuf>)> = DROPPED.with(|dropped| dropped.borrow_mut().drain().collect());
        for (window, paths) in dropped {
            let at = POINTERS.with(|pointers| pointers.borrow().get(&window).map_or((0, 0), |pointer| pointer.position));
            with_app(Hwnd(window), |app| app.drop_files(at.0, at.1, &paths));
        }
        // Timers that are due; each goes on at its own pace.
        let now = Instant::now();
        let due: Vec<(isize, usize)> = TIMERS.with(|timers| {
            let mut timers = timers.borrow_mut();
            timers
                .iter_mut()
                .filter(|timer| timer.due <= now)
                .map(|timer| {
                    timer.due = now + timer.every;
                    (timer.window, timer.id)
                })
                .collect()
        });
        for (window, id) in due {
            self.fire_timer(Hwnd(window), id);
        }
        // An application that has no window left is released here, outside any of its own calls.
        RETIRED.with(|apps| apps.borrow_mut().clear());
        el.set_control_flow(match TIMERS.with(|timers| timers.borrow().iter().map(|timer| timer.due).min()) {
            Some(due) => ControlFlow::WaitUntil(due),
            None => ControlFlow::Wait,
        });
    }
}

pub fn run() -> Result<(), String> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().map_err(|e| e.to_string())?;
    let _ = PROXY.set(event_loop.create_proxy());
    START.get_or_init(Instant::now);
    // Before the first window, whose panes get its variables; its file goes when this returns.
    let _api = crate::app::load_settings().0.api.enabled.then(crate::api::start);
    let result = event_loop.run_app(&mut Shell { first: true }).map_err(|e| e.to_string());
    // PTY cleanup starts threads; release the applications before the thread-local teardown.
    let windows = WINDOWS.with(|windows| std::mem::take(&mut *windows.borrow_mut()));
    drop(windows);
    RETIRED.with(|apps| apps.borrow_mut().clear());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_of_the_keyboard_has_the_scan_code_and_the_virtual_key_windows_gives_it() {
        assert_eq!(physical_key(KeyCode::KeyA), Some((0x1E, false, 0x41)));
        assert_eq!(physical_key(KeyCode::Enter), Some((0x1C, false, 0x0D)));
        assert_eq!(physical_key(KeyCode::NumpadEnter), Some((0x1C, true, 0x0D)), "the keypad's Enter is the same code with the prefix");
        assert_eq!(physical_key(KeyCode::ArrowUp), Some((0x48, true, 0x26)));
        assert_eq!(physical_key(KeyCode::ControlRight), Some((0x1D, true, 0x11)));
        assert_eq!(physical_key(KeyCode::F24), Some((0x76, false, 0x87)));
        assert_eq!(physical_key(KeyCode::MediaPlayPause), None, "a key with no place in the first scan code set");
    }

    #[test]
    fn the_virtual_key_of_a_letter_follows_the_layout_and_any_other_key_its_place() {
        // The key that is Q on a US keyboard types `a` on a French one: the virtual key is A, the scan code stays Q's.
        assert_eq!(virtual_key(&Key::Character("a".into()), 0x51), 0x41);
        assert_eq!(virtual_key(&Key::Character("7".into()), 0x37), 0x37);
        assert_eq!(virtual_key(&Key::Character("é".into()), 0x32), 0x32, "not a letter of ASCII: the place");
        assert_eq!(virtual_key(&Key::Named(NamedKey::Tab), 0x09), 0x09);
    }

    #[test]
    fn the_locale_of_the_environment_is_read_as_a_language_tag() {
        assert_eq!(locale_name_of("zh_TW.UTF-8"), Some("zh-TW".to_owned()));
        assert_eq!(locale_name_of("en_US"), Some("en-US".to_owned()));
        assert_eq!(locale_name_of("C"), None);
    }
}
