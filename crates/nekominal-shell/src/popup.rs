//! Floating lists and confirmations in their own windows, with the acrylic of the system behind the translucent content.
//! This file holds the rows, their geometry and the window; the drawing
//! and the messages go through `App`, which owns the device and the UI layer.

use nekominal_model::{commands::Command, settings::Backdrop};
use nekominal_ui::{Rect, Ui, Color};
use windows::{
    Win32::{
        Foundation::{HWND, POINT},
        Graphics::{
            Dwm::{
                DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute,
            },
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        },
        System::LibraryLoader::{GetModuleHandleW, GetProcAddress},
        UI::{
            WindowsAndMessaging::{
                CreateWindowExW, IDC_ARROW, LoadCursorW, RegisterClassW, SW_SHOWNOACTIVATE, ShowWindow, WNDCLASSW,
                WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WINDOW_EX_STYLE,
            },
        },
    },
    core::{s, w},
};

use crate::gpu::{self, Gpu, Surface};
use crate::{glass::Glass, logic::Mark};

/// Width of a menu or a list, and of the command palette.
pub const WIDTH: f32 = 260.0;
pub const PALETTE_WIDTH: f32 = 520.0;
pub const FIND_WIDTH: f32 = 380.0;
pub const DIALOG_WIDTH: f32 = 480.0;
pub const DIALOG_HEADER: f32 = 112.0;
/// Height of the query line above the rows of the palette.
pub const HEADER: f32 = 48.0;
pub const QUERY_LEFT: f32 = 44.0;
const WINDOW_STYLE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_NOREDIRECTIONBITMAP.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0);
/// The rows the palette shows before it scrolls.
const PALETTE_ROWS: f32 = 8.0;
const PADDING: f32 = crate::design::GAP;
const ROW_HEIGHT: f32 = 32.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
/// How far the pop-up hangs below the point it is anchored at.
const GAP: f32 = 4.0;

/// What choosing a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Confirm,
    Cancel,
    Command(Command),
    /// Select the tab with this index.
    Tab(usize),
    /// Use this backdrop behind the window.
    Backdrop(Backdrop),
    /// Open the settings page at the row with this id.
    ShowSetting(&'static str),
    /// Open a tab with the profile at this index.
    NewTab(usize),
    /// Use the color scheme at this index of `themes::theme_names` of the settings.
    ColorScheme(usize),
    /// A command of the menu of the window (`SC_MINIMIZE`, `SC_CLOSE`…), for the window the menu was opened on.
    Window(u32),
    /// Name the tab the line of the palette is for after what the line holds, or give it back its automatic title.
    RenameTab,
    AutoTitle,
    /// Give the selected tab this colour of `logic::TAB_COLORS`, or none.
    TabColor(Option<usize>),
    /// Type the snippet at this index of the settings; type the suggestion at this index of the list on show.
    Snippet(usize),
    Suggestion(usize),
    /// Show the match at this index of the list of Search all tabs.
    SearchHit(usize),
}

/// One row: an action with its label and a hint at the right (a shortcut, a number), or a separator (no action).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub hint: String,
    pub action: Option<Action>,
    /// The row of what is current (the selected tab).
    pub current: bool,
    /// What is drawn before the label (the logo of a shell).
    pub mark: Option<Mark>,
    /// The program the row stands for, whose icon goes where `mark` would.
    pub program: Option<std::path::PathBuf>,
}

/// The side of the square a mark is drawn in, and the space after it.
pub const MARK_SIZE: f32 = crate::widgets::MARK_SIZE;
pub const MARK_GAP: f32 = 2.0 * crate::design::GAP;

impl Row {
    pub fn separator() -> Row {
        Row { label: String::new(), hint: String::new(), action: None, current: false, mark: None, program: None }
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
    /// The question above the buttons of a confirmation, not an editable query.
    pub heading: String,
    /// What was typed in the query line; `None` for a menu or a list.
    pub query: Option<String>,
    pub preedit: String,
    /// The painted query caret in popup DIP; the owner's IME uses the same position.
    pub caret: (f32, f32, f32),
    /// Height of all the rows together; more than `size.1` when the list scrolls.
    pub content_height: f32,
    /// How far the rows are scrolled up, in DIP.
    pub scroll: f32,
    pub hover: Option<usize>,
    /// The caret and the selection in the query.
    pub cursor: crate::textfield::Cursor,
    /// The find bar stays open when something else is clicked, and then the keyboard is the terminal's again.
    pub has_keys: bool,
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

/// Two equally sized buttons; Cancel is first and is the initial keyboard choice.
pub fn dialog_buttons(width: f32) -> (Vec<Rect>, (f32, f32)) {
    let button_width = (width - 3.0 * PADDING) / 2.0;
    (vec![Rect::new(PADDING, PADDING, button_width, ROW_HEIGHT),
          Rect::new(2.0 * PADDING + button_width, PADDING, button_width, ROW_HEIGHT)],
     (width, ROW_HEIGHT + 2.0 * PADDING))
}

/// Draws the question and its two buttons into the current UI frame, also usable offscreen.
pub fn paint_confirmation(ui: &mut Ui, heading: &str, rows: &[Row], hover: Option<usize>, width: f32, glass: Glass) {
    let color = |c: nekominal_model::themes::Color| Color([c.r, c.g, c.b, c.a]);
    let font_size = 13.0;
    let line = ui.line_height(font_size);
    ui.push_clip(Rect::new(16.0, 12.0, width - 32.0, DIALOG_HEADER - 24.0));
    let mut text = String::new();
    let mut y = 16.0;
    for ch in heading.chars() {
        text.push(ch);
        if ui.measure(&text, font_size, false) > width - 32.0 {
            text.pop();
            ui.text(16.0, y, &text, font_size, color(glass.text()), false);
            y += line;
            text.clear();
            text.push(ch);
        }
    }
    ui.text(16.0, y, &text, font_size, color(glass.text()), false);
    ui.pop_clip();
    let (rects, _) = dialog_buttons(width);
    for (i, (row, rect)) in rows.iter().zip(rects).enumerate() {
        let rect = Rect::new(rect.x, rect.y + DIALOG_HEADER, rect.w, rect.h);
        crate::widgets::button(ui, &glass, rect, crate::widgets::State::chosen(hover == Some(i), false));
        let x = rect.x + (rect.w - ui.measure(&row.label, font_size, false)) / 2.0;
        ui.text(x, rect.y + (rect.h - line) / 2.0, &row.label, font_size, color(glass.text()), false);
    }
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
    Confirm,
    /// A menu or a list, as wide as `WIDTH`, as tall as its rows.
    List,
    /// The command palette: a query line above a fixed number of rows.
    Palette,
    /// The find bar: only the query line, with the count of matches at its right.
    Find,
}

/// Blurs what is behind the window. The system backdrop attribute does not do it for these windows: they never take the
/// focus, and the backdrop of a window that is not the active one is a flat colour. The accent policy keeps blurring.
fn blur_behind(hwnd: HWND, tint: u32) -> Result<(), String> {
    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }
    #[repr(C)]
    struct CompositionAttribute {
        attribute: u32,
        data: *mut AccentPolicy,
        size: usize,
    }
    const ACCENT_POLICY: u32 = 19;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
    type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionAttribute) -> i32;
    // SAFETY: user32 is loaded in every GUI process; the export has this signature and the structs match its ABI.
    unsafe {
        let module = GetModuleHandleW(w!("user32.dll")).map_err(|e| e.to_string())?;
        let function = GetProcAddress(module, s!("SetWindowCompositionAttribute")).ok_or("SetWindowCompositionAttribute is missing")?;
        let function: SetWindowCompositionAttribute = std::mem::transmute(function);
        let mut policy = AccentPolicy { state: ACCENT_ENABLE_ACRYLICBLURBEHIND, flags: 0, gradient_color: tint, animation_id: 0 };
        let mut attribute = CompositionAttribute { attribute: ACCENT_POLICY, data: &mut policy, size: std::mem::size_of::<AccentPolicy>() };
        if function(hwnd, &mut attribute) == 0 {
            return Err("the acrylic request was refused".into());
        }
    }
    Ok(())
}

/// Opens the pop-up with its top right corner at the screen point `anchor` (pixels), kept on the screen.
pub fn open(owner: HWND, gpu: &Gpu, rows: Vec<Row>, kind: Kind, anchor: (i32, i32), scale: f32, dark: bool) -> Result<Popup, String> {
    let (width, header, fixed_height, query) = match kind {
        Kind::Confirm => (DIALOG_WIDTH, DIALOG_HEADER, Some(DIALOG_HEADER + ROW_HEIGHT + 2.0 * PADDING), None),
        Kind::List => (WIDTH, 0.0, None, None),
        Kind::Palette => (PALETTE_WIDTH, HEADER, Some(HEADER + PALETTE_ROWS * ROW_HEIGHT + 2.0 * PADDING), Some(String::new())),
        Kind::Find => (FIND_WIDTH, HEADER, Some(HEADER), Some(String::new())),
    };
    let (rects, rows_size) = if kind == Kind::Confirm { dialog_buttons(width) } else { layout(&rows, width) };
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
            WINDOW_STYLE,
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

        // Rounded corners and the blur behind, over the whole client area.
        let dark = i32::from(dark);
        let rounded: i32 = 3; // DWMWCP_ROUNDSMALL: a corner of 4, the one of design.rs
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&dark as *const i32).cast(), 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&rounded as *const i32).cast(), 4);
        if let Err(e) = blur_behind(hwnd, 0x01_00_00_00) {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
            return Err(format!("popup acrylic: {e}"));
        }

        let surface = match gpu::create_surface(gpu, hwnd, (w as u32, h as u32)) {
            Ok(surface) => surface,
            Err(e) => {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
                return Err(e);
            }
        };
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let mut popup = Popup { hwnd, surface, rows: Vec::new(), rects: Vec::new(), size, header, kind, status: String::new(), heading: String::new(), query, preedit: String::new(), caret: (QUERY_LEFT, 16.0, 16.0), content_height, scroll: 0.0, hover: None, cursor: Default::default(), has_keys: true };
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
    use nekominal_input::Utf16Assembler;
    use super::*;

    #[test]
    fn opening_a_menu_preserves_the_owner_activation() {
        assert!(WINDOW_STYLE.contains(WS_EX_NOACTIVATE));
    }

    #[test]
    fn query_text_accepts_chinese_and_surrogate_pairs_without_accepting_submit() {
        let mut text = String::new();
        let mut input = Utf16Assembler::default();
        let mut cursor = crate::textfield::Cursor::default();
        let mut edit_query = |text: &mut String, input: &mut Utf16Assembler, unit: u16| crate::textfield::type_unit(text, &mut cursor, input, unit);
        for unit in "找你😀".encode_utf16() { edit_query(&mut text, &mut input, unit); }
        assert_eq!(text, "找你😀");
        assert!(!edit_query(&mut text, &mut input, 13));
        assert_eq!(text, "找你😀");
        assert!(edit_query(&mut text, &mut input, 8));
        assert_eq!(text, "找你");
        assert!(!edit_query(&mut text, &mut input, 0xD83D));
        edit_query(&mut text, &mut input, 8);
        assert!(!edit_query(&mut text, &mut input, 0xDE00));
        assert_eq!(text, "找");
    }

    fn row(label: &str) -> Row {
        Row { label: label.into(), hint: String::new(), action: Some(Action::Command(Command::TabNew)), current: false, mark: None, program: None }
    }

    fn rows() -> Vec<Row> {
        vec![row("a"), Row::separator(), row("b"), row("c")]
    }

    #[test]
    fn dialog_buttons_are_separate_equal_and_hit_only_their_own_action() {
        let rows = vec![Row { action: Some(Action::Cancel), ..row("Cancel") },
                        Row { action: Some(Action::Confirm), ..row("Close") }];
        let (rects, size) = dialog_buttons(DIALOG_WIDTH);
        assert_eq!(rects[0].w, rects[1].w);
        assert!(rects[0].x + rects[0].w < rects[1].x);
        for (i, rect) in rects.iter().enumerate() {
            assert_eq!(row_at(&rows, &rects, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0), Some(i));
            assert!(rect.y + rect.h <= size.1);
        }
        assert_eq!(step(&rows, None, true), Some(0), "Enter initially cancels");
        assert_eq!(row_at(&rows, &rects, 0.0, 0.0), None);
    }

    #[test]
    fn confirmation_renders_the_question_and_both_buttons_at_two_scales() {
        use nekominal_render::offscreen::{AdapterKind, OffscreenTarget, create_gpu, write_png};
        let gpu = create_gpu(AdapterKind::Warp).expect("D3D11 WARP");
        let glass = Glass::from_scheme(nekominal_model::themes::scheme("Tokyo Night"));
        let rows = vec![Row { action: Some(Action::Cancel), ..row("取消") },
                        Row { action: Some(Action::Confirm), ..row("關閉") }];
        for scale in [1.0, 1.5] {
            let height = DIALOG_HEADER + dialog_buttons(DIALOG_WIDTH).1.1;
            let size = ((DIALOG_WIDTH * scale) as u32, (height * scale) as u32);
            let target = OffscreenTarget::new(&gpu, size.0, size.1);
            gpu.clear(&target.view, [0.03, 0.03, 0.03, 1.0]);
            let mut ui = Ui::new(&gpu).expect("UI");
            ui.begin_frame(size, scale);
            let line_height = ui.line_height(13.0);
            paint_confirmation(&mut ui, "關閉「專案終端機：測試一個很長的分頁名稱，確認對話框能自動換行而且不遮住按鈕」？",
                               &rows, Some(0), DIALOG_WIDTH, glass);
            ui.end_frame(&target.view);
            let rgba = target.read_rgba(&gpu).expect("readback");
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/ui-test/confirmation-{scale}.png"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            write_png(&path, size.0, size.1, &rgba).unwrap();
            let ink = |rect: Rect| {
                let mut count = 0;
                for y in (rect.y * scale) as u32..((rect.y + rect.h) * scale) as u32 {
                    for x in (rect.x * scale) as u32..((rect.x + rect.w) * scale) as u32 {
                        let at = ((y * size.0 + x) * 4) as usize;
                        if rgba[at..at + 3].iter().all(|c| *c > 150) { count += 1; }
                    }
                }
                count
            };
            assert!(ink(Rect::new(16.0, 12.0, DIALOG_WIDTH - 32.0, 28.0)) > 30, "question missing");
            assert!(ink(Rect::new(16.0, 16.0 + line_height, DIALOG_WIDTH - 32.0, line_height)) > 20, "wrapped text missing");
            for rect in dialog_buttons(DIALOG_WIDTH).0 {
                assert!(ink(Rect::new(rect.x + 12.0, DIALOG_HEADER + rect.y + 6.0, rect.w - 24.0, rect.h - 12.0)) > 10,
                        "button label missing at {scale}x");
            }
        }
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
