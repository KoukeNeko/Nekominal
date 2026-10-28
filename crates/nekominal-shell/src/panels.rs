//! The layer the floating panels of a window are drawn in: the context card, the subagents card and its drawer, the tip of a
//! row of the sidebar. They are drawn as the pop-ups are, translucent over the acrylic of the system, and the acrylic blurs
//! only what is behind a window: so they get a window of their own, over the client area of theirs, shaped to the panels.
//!
//! The layer is drawn in the coordinates of the client area it covers, so the panels are painted where they always were,
//! and it hands its mouse to the window under it, whose hit tests go on as before. Out of the panels it has no shape: the
//! pointer and the clicks there reach the window under it.

use nekominal_ui::Rect;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::{
            Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute},
            Gdi::{ClientToScreen, CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, RGN_OR, SetWindowRgn, ValidateRect},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

use crate::gpu::{self, Gpu, Surface};

/// Not in the generated bindings (it is a common-controls message).
const WM_MOUSELEAVE: u32 = 0x02A3;

pub struct Layer {
    pub hwnd: HWND,
    pub surface: Surface,
    /// The shape last given to the window, in pixels of its client area: (left, top, right, bottom, corner).
    shape: Vec<(i32, i32, i32, i32, i32)>,
    shown: bool,
}

/// Makes the layer of `owner`, hidden. It is owned, so it stays above its window and only above it, and it never takes the
/// focus: the keys stay the window's.
pub fn open(owner: HWND, gpu: &Gpu, dark: bool) -> Result<Layer, String> {
    // SAFETY: plain Win32 calls on the thread of the owner window; every pointer outlives its call.
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(layer_proc),
            hInstance: module.into(),
            lpszClassName: w!("NekominalPanels"),
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            ..Default::default()
        };
        // Registered once; a second registration fails and the first stays.
        RegisterClassW(&class);
        let style = WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_NOREDIRECTIONBITMAP.0 | WS_EX_TOOLWINDOW.0);
        let hwnd = CreateWindowExW(style, w!("NekominalPanels"), w!(""), WS_POPUP, 0, 0, 1, 1, Some(owner), None, Some(module.into()), None)
            .map_err(|e| e.to_string())?;
        let dark = i32::from(dark);
        let square: i32 = 1; // DWMWCP_DONOTROUND: the region gives the corners
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&dark as *const i32).cast(), 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&square as *const i32).cast(), 4);
        if let Err(e) = crate::popup::blur_behind(hwnd, 0x01_00_00_00) {
            let _ = DestroyWindow(hwnd);
            return Err(format!("panel acrylic: {e}"));
        }
        let surface = match gpu::create_surface(gpu, hwnd, (1, 1)) {
            Ok(surface) => surface,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(e);
            }
        };
        Ok(Layer { hwnd, surface, shape: Vec::new(), shown: false })
    }
}

impl Layer {
    /// Lays the layer over the client area of `owner`, `size` pixels, and its swap chain to the same size.
    pub fn place(&mut self, owner: HWND, gpu: &Gpu, size: (u32, u32)) {
        self.surface.resize(gpu, size);
        self.follow(owner, Some(size));
    }

    /// Moves the layer with its window, which moved; `size` resizes it too.
    pub fn follow(&self, owner: HWND, size: Option<(u32, u32)>) {
        let mut at = POINT::default();
        // SAFETY: plain query and a move of a window of this thread.
        unsafe {
            let _ = ClientToScreen(owner, &mut at);
            let flags = SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | if size.is_some() { SET_WINDOW_POS_FLAGS(0) } else { SWP_NOSIZE };
            let (w, h) = size.map_or((0, 0), |(w, h)| (w as i32, h as i32));
            let _ = SetWindowPos(self.hwnd, None, at.x, at.y, w, h, flags);
        }
    }

    /// Gives the window the shape of the panels (rectangles and the radius of their corners, in DIP) and shows it.
    pub fn shape(&mut self, panels: &[(Rect, f32)], scale: f32) {
        let shape = pixels(panels, scale);
        if shape != self.shape {
            // SAFETY: GDI regions made here; the window takes the one it is given and the others are deleted.
            unsafe {
                let region = CreateRectRgn(0, 0, 0, 0);
                for &(l, t, r, b, d) in &shape {
                    // A round rectangle region leaves out its right and bottom edge: one more pixel takes them in.
                    let part = if d > 0 { CreateRoundRectRgn(l, t, r + 1, b + 1, d, d) } else { CreateRectRgn(l, t, r, b) };
                    CombineRgn(Some(region), Some(region), Some(part), RGN_OR);
                    let _ = DeleteObject(part.into());
                }
                SetWindowRgn(self.hwnd, Some(region), true);
            }
            self.shape = shape;
        }
        if !self.shown {
            // SAFETY: showing a window of this thread without taking the focus.
            unsafe { let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE); }
            self.shown = true;
        }
    }

    /// Hides the layer, no panel being open, and gives back the memory of its swap chain.
    pub fn hide(&mut self, gpu: &Gpu) {
        if !self.shown {
            return;
        }
        // SAFETY: hiding a window of this thread.
        unsafe { let _ = ShowWindow(self.hwnd, SW_HIDE); }
        self.shown = false;
        self.surface.resize(gpu, (1, 1));
    }

    pub fn shown(&self) -> bool {
        self.shown
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        self.surface.detach();
        // SAFETY: a window this layer made.
        unsafe { let _ = DestroyWindow(self.hwnd); }
    }
}

/// The panels in pixels of the client area: (left, top, right, bottom, the diameter of the corners).
fn pixels(panels: &[(Rect, f32)], scale: f32) -> Vec<(i32, i32, i32, i32, i32)> {
    let px = |v: f32| (v * scale).round() as i32;
    panels.iter().map(|(r, radius)| (px(r.x), px(r.y), px(r.x + r.w), px(r.y + r.h), px(2.0 * radius))).collect()
}

/// The window under the screen point of the cursor is the layer of `owner`: the pointer left the window for its panels.
pub fn cursor_on_layer(layer: HWND) -> bool {
    let mut at = POINT::default();
    // SAFETY: plain queries.
    unsafe { GetCursorPos(&mut at).is_ok() && WindowFromPoint(at) == layer }
}

/// The messages of the layer. Its client area lies on the one of its owner, so the mouse goes to the owner as it is, with
/// the same coordinates; the owner keeps the keys.
unsafe extern "system" fn layer_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: plain query of a window of this thread.
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) }.unwrap_or_default();
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            // SAFETY: the swap chain is the content; the window paints from its owner's frames.
            unsafe { let _ = ValidateRect(Some(hwnd), None); }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            // SAFETY: a valid request for this window, and a message to its owner on the same thread.
            unsafe {
                let _ = TrackMouseEvent(&mut track);
                SendMessageW(owner, msg, Some(wparam), Some(lparam))
            }
        }
        // Left for the window under it: that window has the pointer now and says so itself. Left for elsewhere: the owner
        // is told, as if the pointer had left it.
        WM_MOUSELEAVE => {
            let mut at = POINT::default();
            // SAFETY: plain queries, and a message to the owner on the same thread.
            unsafe {
                let back = GetCursorPos(&mut at).is_ok() && WindowFromPoint(at) == owner;
                if !back {
                    SendMessageW(owner, WM_MOUSELEAVE, None, None);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MOUSEWHEEL => {
            // SAFETY: a message to the owner on the same thread.
            unsafe { SendMessageW(owner, msg, Some(wparam), Some(lparam)) }
        }
        WM_SETCURSOR => {
            // The owner sets the hand over what can be clicked; elsewhere the arrow of the class.
            // SAFETY: a message to the owner on the same thread, then the default handling.
            unsafe {
                let set = SendMessageW(owner, WM_SETCURSOR, Some(WPARAM(owner.0 as usize)), Some(lparam));
                if set.0 != 0 { set } else { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        // SAFETY: default handling of every other message.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_is_shaped_in_the_pixels_of_the_screen_it_is_on() {
        // A card at (10, 44), 340 × 200 DIP with corners of 8, and a square drawer, at 150 %.
        let panels = [(Rect::new(10.0, 44.0, 340.0, 200.0), 8.0), (Rect::new(600.0, 40.0, 340.0, 500.0), 0.0)];
        assert_eq!(pixels(&panels, 1.5), vec![(15, 66, 525, 366, 24), (900, 60, 1410, 810, 0)]);
        assert_eq!(pixels(&panels[..1], 1.0), vec![(10, 44, 350, 244, 16)]);
    }
}
