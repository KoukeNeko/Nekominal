//! A floating list of rows in a window of its own: the "…" menu and the list of all tabs. The window is a pop-up with the DWM acrylic
//! backdrop, so the blur is real, not painted. This file holds the rows, their geometry and the window; the drawing
//! and the messages go through `App`, which owns the device and the UI layer.

use nekominal_model::commands::Command;
use nekominal_ui::Rect;
use windows::{
    Win32::{
        Foundation::{HWND, POINT},
        Graphics::{
            Dwm::{
                DWM_SYSTEMBACKDROP_TYPE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
                DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
            },
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::MARGINS,
            WindowsAndMessaging::{
                CreateWindowExW, IDC_ARROW, LoadCursorW, RegisterClassW, SW_SHOW, SetForegroundWindow, ShowWindow, WNDCLASSW,
                WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
    core::w,
};

use crate::gpu::{self, Gpu, Surface};

/// Width of a menu or a list, and of the command palette.
pub const WIDTH: f32 = 260.0;
pub const PALETTE_WIDTH: f32 = 520.0;
pub const FIND_WIDTH: f32 = 380.0;
/// Height of the query line above the rows of the palette.
pub const HEADER: f32 = 48.0;
/// The rows the palette shows before it scrolls.
const PALETTE_ROWS: f32 = 8.0;
const PADDING: f32 = 6.0;
const ROW_HEIGHT: f32 = 32.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
/// How far the pop-up hangs below the point it is anchored at.
const GAP: f32 = 4.0;

/// What choosing a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Command(Command),
    /// Select the tab with this index.
    Tab(usize),
}

/// One row: an action with its label and a hint at the right (a shortcut, a number), or a separator (no action).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub hint: String,
    pub action: Option<Action>,
    /// The row of what is current (the selected tab).
    pub current: bool,
}

impl Row {
    pub fn separator() -> Row {
        Row { label: String::new(), hint: String::new(), action: None, current: false }
    }

    pub fn is_separator(&self) -> bool {
        self.action.is_none()
    }
}

pub struct Popup {
    pub hwnd: HWND,
    pub surface: Surface,
    pub rows: Vec<Row>,
    /// The rectangle of each row in DIP, inside the window.
    pub rects: Vec<Rect>,
    /// Size of the window in DIP.
    pub size: (f32, f32),
    /// Height of the query line above the rows: 0 for a menu or a list.
    pub header: f32,
    pub kind: Kind,
    /// What the query line says at its right (the find bar's count).
    pub status: String,
    /// What was typed in the query line; `None` for a menu or a list.
    pub query: Option<String>,
    /// Height of all the rows together; more than `size.1` when the list scrolls.
    pub content_height: f32,
    /// How far the rows are scrolled up, in DIP.
    pub scroll: f32,
    pub hover: Option<usize>,
}

impl Popup {
    /// The height of the window below the query line, where the rows are.
    pub fn view_height(&self) -> f32 {
        self.size.1 - self.header
    }

    /// Scrolls by `delta` DIP (positive moves the rows up), within the content.
    pub fn scroll_by(&mut self, delta: f32) {
        self.scroll = clamp_scroll(self.scroll + delta, self.content_height, self.view_height());
    }

    /// Scrolls the least that shows row `i` whole.
    pub fn reveal(&mut self, i: usize) {
        self.scroll = reveal_scroll(self.scroll, self.rects[i], self.content_height, self.view_height());
    }

    /// The row under a point of the window (DIP): none in the query line.
    pub fn row_at_point(&self, x: f32, y: f32) -> Option<usize> {
        (y >= self.header).then(|| row_at(&self.rows, &self.rects, x, y - self.header + self.scroll)).flatten()
    }

    /// Replaces the rows (the palette after a keystroke): scrolled to the top, the first row under the cursor.
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        let (rects, size) = layout(&rows, self.size.0);
        self.first_hover(&rows);
        self.rows = rows;
        self.rects = rects;
        self.content_height = size.1;
        self.scroll = 0.0;
    }

    fn first_hover(&mut self, rows: &[Row]) {
        self.hover = rows.iter().position(|r| !r.is_separator());
    }
}

fn clamp_scroll(scroll: f32, content: f32, view: f32) -> f32 {
    scroll.clamp(0.0, (content - view).max(0.0))
}

/// The scroll offset that shows `rect` whole in a window of height `view`, moving as little as possible.
fn reveal_scroll(scroll: f32, rect: Rect, content: f32, view: f32) -> f32 {
    let scroll = if rect.y < scroll {
        rect.y - PADDING
    } else if rect.y + rect.h > scroll + view {
        rect.y + rect.h + PADDING - view
    } else {
        scroll
    };
    clamp_scroll(scroll, content, view)
}

/// The rectangles of `rows` and the size of the rows with their padding, in DIP, for a window `width` wide.
pub fn layout(rows: &[Row], width: f32) -> (Vec<Rect>, (f32, f32)) {
    let mut y = PADDING;
    let rects = rows
        .iter()
        .map(|row| {
            let height = if row.is_separator() { SEPARATOR_HEIGHT } else { ROW_HEIGHT };
            let rect = Rect::new(PADDING, y, width - 2.0 * PADDING, height);
            y += height;
            rect
        })
        .collect();
    (rects, (width, y + PADDING))
}

/// The row under a point of the content (DIP, scrolling included); separators are not under anything.
pub fn row_at(rows: &[Row], rects: &[Rect], x: f32, y: f32) -> Option<usize> {
    rects.iter().position(|r| r.contains(x, y)).filter(|&i| !rows[i].is_separator())
}

/// The next row that can be chosen after `from` in a direction (wrapping), for the arrow keys.
pub fn step(rows: &[Row], from: Option<usize>, forward: bool) -> Option<usize> {
    let n = rows.len();
    if n == 0 {
        return None;
    }
    let start = from.unwrap_or(if forward { n - 1 } else { 0 });
    (1..=n).map(|k| if forward { (start + k) % n } else { (start + n - k) % n }).find(|&i| !rows[i].is_separator())
}

/// What to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A menu or a list, as wide as `WIDTH`, as tall as its rows.
    List,
    /// The command palette: a query line above a fixed number of rows.
    Palette,
    /// The find bar: only the query line, with the count of matches at its right.
    Find,
}

/// Opens the pop-up with its top right corner at the screen point `anchor` (pixels), kept on the screen.
pub fn open(owner: HWND, gpu: &Gpu, rows: Vec<Row>, kind: Kind, anchor: (i32, i32), scale: f32, dark: bool) -> Result<Popup, String> {
    let (width, header, fixed_height, query) = match kind {
        Kind::List => (WIDTH, 0.0, None, None),
        Kind::Palette => (PALETTE_WIDTH, HEADER, Some(HEADER + PALETTE_ROWS * ROW_HEIGHT + 2.0 * PADDING), Some(String::new())),
        Kind::Find => (FIND_WIDTH, HEADER, Some(HEADER), Some(String::new())),
    };
    let (rects, rows_size) = layout(&rows, width);
    let content_height = rows_size.1;
    let size = (width, fixed_height.unwrap_or(header + rows_size.1));
    let (w, h) = ((size.0 * scale).round() as i32, (size.1 * scale).round() as i32);
    // SAFETY: plain Win32 calls on the thread of the owner window; every pointer outlives its call.
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(crate::win::popup_proc),
            hInstance: module.into(),
            lpszClassName: w!("NekominalPopup"),
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            ..Default::default()
        };
        // Registered once; a second registration fails and the first stays.
        RegisterClassW(&class);

        let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(MonitorFromPoint(POINT { x: anchor.0, y: anchor.1 }, MONITOR_DEFAULTTONEAREST), &mut monitor);
        let work = monitor.rcWork;
        let gap = (GAP * scale).round() as i32;
        let x = (anchor.0 - w).clamp(work.left, (work.right - w).max(work.left));
        // A list taller than the screen below the anchor scrolls.
        let h = if fixed_height.is_some() { h } else { h.min((work.bottom - anchor.1 - gap - (8.0 * scale) as i32).max(ROW_HEIGHT as i32)) };
        let size = (size.0, h as f32 / scale);
        let y = (anchor.1 + gap).clamp(work.top, (work.bottom - h).max(work.top));

        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("NekominalPopup"),
            w!(""),
            WS_POPUP,
            x,
            y,
            w,
            h,
            Some(owner),
            None,
            Some(module.into()),
            None,
        )
        .map_err(|e| e.to_string())?;

        // Transient-window acrylic, rounded corners, over the whole client area.
        let dark = i32::from(dark);
        let acrylic = DWM_SYSTEMBACKDROP_TYPE(3);
        let rounded: i32 = 2;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&dark as *const i32).cast(), 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, (&acrylic as *const DWM_SYSTEMBACKDROP_TYPE).cast(), 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&rounded as *const i32).cast(), 4);
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });

        let surface = match gpu::create_surface(gpu, hwnd, (w as u32, h as u32)) {
            Ok(surface) => surface,
            Err(e) => {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
                return Err(e);
            }
        };
        let _ = ShowWindow(hwnd, SW_SHOW);
        // Taking the activation is what lets the pop-up notice that the user went elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let mut popup = Popup { hwnd, surface, rows: Vec::new(), rects: Vec::new(), size, header, kind, status: String::new(), query, content_height, scroll: 0.0, hover: None };
        if popup.query.is_some() {
            popup.set_rows(rows);
        } else {
            popup.rows = rows;
            popup.rects = rects;
        }
        Ok(popup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(label: &str) -> Row {
        Row { label: label.into(), hint: String::new(), action: Some(Action::Command(Command::TabNew)), current: false }
    }

    fn rows() -> Vec<Row> {
        vec![row("a"), Row::separator(), row("b"), row("c")]
    }

    #[test]
    fn the_rows_stack_with_a_thin_separator_and_padding_around() {
        let (rects, size) = layout(&rows(), WIDTH);
        assert_eq!(rects[0].y, PADDING);
        assert_eq!(rects[1].h, SEPARATOR_HEIGHT);
        assert_eq!(rects[2].y, PADDING + ROW_HEIGHT + SEPARATOR_HEIGHT);
        assert_eq!(size.1, PADDING * 2.0 + ROW_HEIGHT * 3.0 + SEPARATOR_HEIGHT);
    }

    #[test]
    fn a_separator_is_never_under_the_pointer() {
        let rows = rows();
        let (rects, _) = layout(&rows, WIDTH);
        assert_eq!(row_at(&rows, &rects, 20.0, rects[0].y + 4.0), Some(0));
        assert_eq!(row_at(&rows, &rects, 20.0, rects[1].y + 4.0), None);
        assert_eq!(row_at(&rows, &rects, 20.0, rects[3].y + 4.0), Some(3));
        assert_eq!(row_at(&rows, &rects, 2.0, 20.0), None, "the padding is not a row");
    }

    #[test]
    fn the_arrow_keys_have_nothing_to_choose_in_an_empty_list() {
        assert_eq!(step(&[], None, true), None);
        assert_eq!(step(&[], None, false), None);
    }

    #[test]
    fn the_arrow_keys_skip_separators_and_wrap() {
        let rows = rows();
        assert_eq!(step(&rows, None, true), Some(0));
        assert_eq!(step(&rows, None, false), Some(3));
        assert_eq!(step(&rows, Some(0), true), Some(2), "down from the row above a separator");
        assert_eq!(step(&rows, Some(2), false), Some(0), "up from the row below a separator");
        assert_eq!(step(&rows, Some(3), true), Some(0), "wraps at the end");
    }

    #[test]
    fn scrolling_stays_inside_the_content_and_reveals_a_row_with_the_least_movement() {
        let rows: Vec<Row> = (0..20).map(|i| row(&i.to_string())).collect();
        let (rects, size) = layout(&rows, WIDTH);
        let view = 200.0;
        assert_eq!(clamp_scroll(-5.0, size.1, view), 0.0);
        assert_eq!(clamp_scroll(10_000.0, size.1, view), size.1 - view);
        assert_eq!(clamp_scroll(10.0, 100.0, view), 0.0, "a short list never scrolls");
        assert_eq!(reveal_scroll(0.0, rects[1], size.1, view), 0.0, "a visible row moves nothing");
        let last = rects[19];
        let scrolled = reveal_scroll(0.0, last, size.1, view);
        assert!(scrolled > 0.0 && last.y + last.h <= scrolled + view, "the last row is shown whole");
        assert_eq!(reveal_scroll(scrolled, rects[0], size.1, view), 0.0, "back to the first row");
    }
}
