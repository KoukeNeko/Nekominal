//! The application of one window: tabs with panes, the chrome drawn around them, and the input.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nekominal_input::{
    KeyEvent, KeyInput, Mods, MouseButton, PointerEvent, PointerGrid, PointerKind, Utf16Assembler, vk,
};
use nekominal_model::{
    commands::{self, Command},
    palette::{self, PaletteAction, PaletteTab},
    keymap::{self, Keymap, Modifiers},
    layout::{Axis, PaneId},
    menu::{APP_MENU, MenuRow},
    profiles::{self, DiscoveredProfile, Profile},
    settings::{self, Backdrop, Settings},
    strings::{Lang, Text},
    store,
    themes::{self, Color as SchemeColor},
};
use nekominal_pane::{Pane, PaneRenderer, UiEvent, view as pane_view};
use nekominal_term::SearchMatch;
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_render as r;
use nekominal_term::alacritty_terminal::grid::Scroll;
use nekominal_ui::{Color, Rect, Ui};
use windows::Win32::{
    Foundation::{HWND, LPARAM, POINT, WPARAM},
    Globalization::GetUserDefaultLocaleName,
    Graphics::Gdi::{ClientToScreen, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    UI::{
        Input::KeyboardAndMouse::GetKeyState,
        WindowsAndMessaging::{
            DestroyWindow, GWL_STYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowPlacement, HWND_TOP, PostMessageW,
            SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPlacement,
            SetWindowPos, WINDOWPLACEMENT, WM_APP, WM_CLOSE, WS_OVERLAPPEDWINDOW,
        },
    },
};

use crate::{
    clipboard, find,
    glass::{self, Glass},
    gpu::{self, Gpu, Surface},
    layout::{self, Hit, Layout},
    logic::{self, PxRect},
    open, popup,
    tab::{PaneEntry, Tab},
    util,
};

/// Posted by a reader thread when a pane has output to draw.
pub const WM_WAKE: u32 = WM_APP + 1;

/// What the window does with the mouse capture after a button event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    None,
    Take,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Hand,
    ResizeH,
    ResizeV,
}

/// Corner radius of every rounded rectangle, in DIP (the same number as `design.rs` of the WinUI shell).
pub const CORNER_RADIUS: f32 = 8.0;
const TEXT_SIZE: f32 = 13.0;
const ICON_SIZE: f32 = 10.0;

static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

pub struct App {
    hwnd: HWND,
    gpu: Gpu,
    surface: Surface,
    ui: Ui,
    scale: f32,
    maximized: bool,
    active: bool,
    settings: Settings,
    keymap: Keymap,
    glass: Glass,
    theme: r::Theme,
    profile: Profile,
    tabs: Vec<Tab>,
    selected: usize,
    layout: Layout,
    /// Width in DIP of the caption buttons DWM draws at the top right.
    caption: f32,
    hover: Hit,
    pressed: Hit,
    /// Which of the left, middle and right buttons are down in a pane.
    buttons: [bool; 3],
    /// The pane a held button went down in: it keeps the pointer until the last button is up.
    pointer_pane: Option<PaneId>,
    /// The pointer is over a link: the window shows the hand.
    hand: bool,
    /// The axis of the divider under the pointer (the window shows a resize cursor).
    over_splitter: Option<Axis>,
    /// The path of the divider being dragged.
    dragged_splitter: Option<Vec<u8>>,
    /// The "…" menu while it is open.
    popup: Option<popup::Popup>,
    /// The placement and style the window had before full screen, while it is full screen.
    fullscreen: Option<(WINDOWPLACEMENT, isize)>,
    /// The search of the find bar while it is open.
    find: Option<FindState>,
    /// When the menu last closed: the click that closed it (outside it) must not open it again.
    popup_closed: Option<Instant>,
    text: Utf16Assembler,
}

/// The matches of the find bar and the one the bar is on.
struct FindState {
    matches: Vec<SearchMatch>,
    current: usize,
}

/// Matches kept at most: more than this are of no use to anyone and cost a scan of the whole buffer.
const FIND_LIMIT: usize = 10_000;

/// Whether `App::run` does something for the command.
fn wired(command: Command) -> bool {
    matches!(
        command,
        Command::TabNew
            | Command::TabClose
            | Command::TabNext
            | Command::TabPrevious
            | Command::PaneSplitRight
            | Command::PaneSplitDown
            | Command::PaneClose
            | Command::PaneZoom
            | Command::PaneFocus(_)
            | Command::PaneResize(_)
            | Command::EditCopy
            | Command::EditSmartCopy
            | Command::EditPaste
            | Command::EditSelectAll
            | Command::EditClear
            | Command::ScrollPageUp
            | Command::ScrollPageDown
            | Command::ScrollTop
            | Command::ScrollBottom
            | Command::PromptPrevious
            | Command::PromptNext
            | Command::AppPalette
            | Command::EditFind
            | Command::ViewFullscreen
            | Command::AppearanceBackdropNext
            | Command::AppearanceOpacityUp
            | Command::AppearanceOpacityDown
    )
}

fn col(c: SchemeColor) -> Color {
    Color([c.r, c.g, c.b, c.a])
}

fn discovered(found: &nekominal_pty::profiles::Profile) -> DiscoveredProfile {
    DiscoveredProfile {
        id: found.id.clone(),
        name: found.name.clone(),
        kind: found.kind,
        icon: found.icon,
        program: found.program.clone(),
        args: found.args.clone(),
    }
}

fn held() -> Modifiers {
    let down = |vk: i32| unsafe { GetKeyState(vk) } < 0;
    Modifiers { ctrl: down(0x11), shift: down(0x10), alt: down(0x12), meta: down(0x5B) || down(0x5C) }
}

fn input_mods(m: Modifiers) -> Mods {
    let mut mods = Mods::NONE;
    for (on, flag) in [(m.shift, Mods::SHIFT), (m.ctrl, Mods::CTRL), (m.alt, Mods::ALT), (m.meta, Mods::SUPER)] {
        if on {
            mods |= flag;
        }
    }
    mods
}

impl App {
    pub fn new(hwnd: HWND, size_px: (u32, u32), scale: f32) -> Result<App, String> {
        let text = store::settings_path().and_then(|path| store::load(&path).ok().flatten());
        let settings = settings::parse(text.as_deref()).settings;
        let keymap = keymap::build_keymap(commands::default_bindings(), &settings.keybindings).keymap;
        let scheme = themes::scheme(&settings.theme);
        let glass = Glass::from_scheme(scheme);
        let opacity = glass::terminal_opacity(settings.appearance.backdrop, settings.appearance.opacity, glass::Material::NORMAL);
        let theme = glass::render_theme(scheme, opacity);

        let found: Vec<DiscoveredProfile> = nekominal_pty::profiles::discover().iter().map(discovered).collect();
        let all = profiles::merge(&found, &settings.profiles);
        let profile = profiles::resolve_default(&all, &settings.default_profile)
            .or(all.first())
            .cloned()
            .ok_or("no shell was found")?;

        let (gpu, surface) = gpu::create(hwnd, size_px)?;
        let ui = Ui::new(&gpu)?;
        let mut app = App {
            hwnd,
            gpu,
            surface,
            ui,
            scale,
            maximized: false,
            active: true,
            settings,
            keymap,
            glass,
            theme,
            profile,
            tabs: Vec::new(),
            selected: 0,
            layout: Layout::new(1.0, 1.0, 0, 0.0, 0),
            caption: 138.0,
            hover: Hit::None,
            pressed: Hit::None,
            buttons: [false; 3],
            pointer_pane: None,
            hand: false,
            over_splitter: None,
            dragged_splitter: None,
            popup: None,
            fullscreen: None,
            find: None,
            popup_closed: None,
            text: Utf16Assembler::default(),
        };
        app.relayout();
        app.new_tab()?;
        Ok(app)
    }

    pub fn backdrop(&self) -> Backdrop {
        self.settings.appearance.backdrop
    }

    pub fn dark(&self) -> bool {
        !self.glass.light
    }

    // --- the selected tab and its focused pane ------------------------------------------------------------------

    fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.selected)
    }

    fn focused(&self) -> Option<&PaneEntry> {
        self.tab()?.focused_entry()
    }

    fn pane_area(&self) -> PxRect {
        let (w, h) = self.surface.size();
        let top = self.pane_top() as i32;
        PxRect { x: 0, y: top, w: w as i32, h: (h as i32 - top).max(1) }
    }

    /// Pixels between the top of the window and the top of the panes.
    fn pane_top(&self) -> f32 {
        (layout::TITLE_HEIGHT * self.scale).round()
    }

    // --- tabs and panes -----------------------------------------------------------------------------------------

    fn spawn_config(&self, cols: u16, rows: u16) -> SpawnConfig {
        let mut env = nekominal_pty::profiles::default_environment();
        if self.settings.agent_environment {
            env.extend(nekominal_pty::profiles::agent_environment());
        }
        let program = nekominal_pty::profiles::resolve_program(&self.profile.program)
            .unwrap_or_else(|| self.profile.program.clone().into());
        SpawnConfig {
            program,
            args: profiles::launch_args(&self.profile, self.settings.shell_integration),
            cwd: self.profile.cwd.as_deref().map(std::path::PathBuf::from).filter(|dir| dir.is_dir()),
            env,
            cols,
            rows,
            width_model: WidthModel::default(),
        }
    }

    fn renderer_config(&self) -> r::RendererConfig {
        let mut fonts = r::fonts::FontConfig::default();
        if !self.settings.font.family.is_empty() {
            fonts.text = self.settings.font.family.clone();
        }
        r::RendererConfig {
            fonts,
            font_size_pt: self.settings.font.size as f32,
            scale: self.scale,
            line_height: self.settings.font.line_height as f32,
            lcd: self.theme.background_opacity >= 1.0,
            atlas_initial: 512,
            ..Default::default()
        }
    }

    /// A pane that will have about `size_px` pixels: its renderer and the session of the profile.
    fn make_entry(&self, id: PaneId, size_px: (u32, u32)) -> Result<PaneEntry, String> {
        let sibling = self.tabs.iter().find_map(|tab| tab.panes.first()).map(|entry| &entry.view);
        let view = PaneRenderer::new_sharing(&self.gpu, self.renderer_config(), self.theme.clone(), sibling)?;
        let (cols, rows) = view.grid_for(size_px);
        let hwnd = self.hwnd.0 as isize;
        let wake: nekominal_pane::Wake = Arc::new(move || {
            // One posted message per burst of output: the handler clears the flag when it draws.
            if !WAKE_PENDING.swap(true, Ordering::AcqRel) {
                // SAFETY: posting to a window handle from another thread is allowed; a dead window drops it.
                unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0)).ok() };
            }
        });
        let config = self.spawn_config(cols as u16, rows as u16);
        let core = logic::core_config(&self.settings);
        let pane = Pane::spawn_woken(config, core, wake).map_err(|e| format!("pty: {e}"))?;
        Ok(PaneEntry {
            id,
            pane,
            view,
            grid: (cols as u16, rows as u16),
            rect: PxRect { x: 0, y: 0, w: size_px.0 as i32, h: size_px.1 as i32 },
            title: String::new(),
            exited: false,
        })
    }

    fn new_tab(&mut self) -> Result<(), String> {
        let area = self.pane_area();
        let tab = Tab::new(|id| self.make_entry(id, (area.w as u32, area.h as u32)))?;
        self.tabs.push(tab);
        self.selected = self.tabs.len() - 1;
        self.relayout();
        self.sync_focus();
        self.redraw();
        Ok(())
    }

    fn split(&mut self, axis: Axis) {
        let Some(tab) = self.tab() else { return };
        let Some(focused) = tab.focused_entry() else { return };
        // About half of the pane that is split; the layout corrects it right after.
        let size = match axis {
            Axis::Row => (focused.rect.w as u32 / 2, focused.rect.h as u32),
            Axis::Column => (focused.rect.w as u32, focused.rect.h as u32 / 2),
        };
        let id = self.tabs[self.selected].next_id();
        match self.make_entry(id, size) {
            Ok(entry) => {
                self.tabs[self.selected].split(axis, entry);
                self.relayout();
                self.sync_focus();
            }
            Err(e) => util::log(&format!("split: {e}")),
        }
        self.redraw();
    }

    /// Closes a pane; the tab goes with its last pane, and the window with its last tab.
    fn close_pane(&mut self, tab_index: usize, id: PaneId) {
        let Some(tab) = self.tabs.get_mut(tab_index) else { return };
        if tab.remove(id) {
            self.close_tab(tab_index);
            return;
        }
        self.relayout();
        self.sync_focus();
        self.redraw();
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        for entry in &tab.panes {
            entry.pane.shutdown();
        }
        if self.tabs.is_empty() {
            // SAFETY: closes the window of this thread; the message loop ends with it.
            unsafe { PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).ok() };
            return;
        }
        if self.selected > index || self.selected >= self.tabs.len() {
            self.selected = self.selected.saturating_sub(1);
        }
        self.relayout();
        self.sync_focus();
        self.redraw();
    }

    fn select(&mut self, index: usize) {
        if index < self.tabs.len() && index != self.selected {
            self.selected = index;
            self.relayout();
            self.sync_focus();
            self.redraw();
        }
    }

    /// Tells every pane whether it has the keyboard: the focused pane of the selected tab, in an active window.
    fn sync_focus(&mut self) {
        let active = self.active;
        for (t, tab) in self.tabs.iter_mut().enumerate() {
            let selected = t == self.selected;
            let focused = tab.focused;
            for entry in &mut tab.panes {
                let on = active && selected && entry.id == focused;
                entry.view.set_focused(on);
                entry.pane.set_focused(on);
                entry.view.invalidate();
            }
        }
    }

    // --- events from the window ---------------------------------------------------------------------------------

    pub fn on_size(&mut self, size_px: (u32, u32), maximized: bool) {
        self.maximized = maximized;
        self.surface.resize(&self.gpu, size_px);
        self.relayout();
        self.redraw();
    }

    pub fn on_dpi(&mut self, scale: f32) {
        self.scale = scale;
        for tab in &mut self.tabs {
            for entry in &mut tab.panes {
                let _ = entry.view.set_scale(scale);
            }
        }
        self.relayout();
        self.redraw();
    }

    /// The system caption buttons are `width_px` wide: the chips keep clear of them.
    pub fn on_caption(&mut self, width_px: f32) {
        self.caption = width_px / self.scale;
        self.relayout();
        self.redraw();
    }

    pub fn on_activate(&mut self, active: bool) {
        // The menu takes the activation while it is open; the window is still the one in use.
        if !active && self.popup.is_some() {
            return;
        }
        self.active = active;
        self.sync_focus();
        self.redraw();
    }

    /// Chips, panes and the cells each shown pane has.
    fn relayout(&mut self) {
        let (w, h) = self.surface.size();
        self.layout = Layout::new(w as f32 / self.scale, h as f32 / self.scale, self.tabs.len(), self.caption, self.selected);
        let area = self.pane_area();
        let divider = logic::divider_px(f64::from(self.scale));
        let Some(tab) = self.tabs.get_mut(self.selected) else { return };
        tab.place(area, divider);
        for id in tab.shown() {
            let Some(entry) = tab.entry_mut(id) else { continue };
            let (cols, rows) = entry.view.grid_for((entry.rect.w.max(1) as u32, entry.rect.h.max(1) as u32));
            if (cols as u16, rows as u16) != entry.grid {
                entry.grid = (cols as u16, rows as u16);
                entry.pane.resize(cols as u16, rows as u16);
            }
            entry.view.invalidate();
        }
    }

    pub fn redraw(&self) {
        // SAFETY: invalidating our own window; WM_PAINT arrives when the queue is otherwise empty.
        unsafe { InvalidateRect(Some(self.hwnd), None, false).ok() };
    }

    /// Output arrived (or a pane's program ended): take the events and ask for a frame.
    pub fn on_wake(&mut self) {
        WAKE_PENDING.store(false, Ordering::Release);
        let mut ended: Vec<(usize, PaneId)> = Vec::new();
        for (t, tab) in self.tabs.iter_mut().enumerate() {
            for entry in &mut tab.panes {
                while let Ok(event) = entry.pane.events.try_recv() {
                    match event {
                        UiEvent::Title(title) => entry.title = logic::clean_title(&title),
                        UiEvent::ClipboardStore(text) => {
                            clipboard::write_text(&text);
                        }
                        UiEvent::ClipboardLoad(request) => {
                            entry.pane.answer_clipboard(&request, &clipboard::read_text().unwrap_or_default());
                        }
                        UiEvent::Exit(code) => {
                            entry.exited = true;
                            if code == 0 {
                                ended.push((t, entry.id));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // Later tabs first, so an index stays right while others go.
        for (t, id) in ended.into_iter().rev() {
            self.close_pane(t, id);
        }
        self.redraw();
    }

    // --- mouse --------------------------------------------------------------------------------------------------

    fn point(&self, x: i32, y: i32) -> (f32, f32) {
        (x as f32 / self.scale, y as f32 / self.scale)
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Hit {
        let (x, y) = self.point(x, y);
        self.layout.hit(x, y)
    }

    /// The grid of a pane in pixels, as the pointer wants it.
    fn pointer_grid(entry: &PaneEntry) -> PointerGrid {
        let m = entry.view.metrics();
        PointerGrid { cell_width: m.width as f32, cell_height: m.height as f32, columns: entry.grid.0, rows: entry.grid.1 }
    }

    /// Sends a pointer event to a pane and does what it asks of the shell.
    fn pointer(&mut self, id: PaneId, kind: PointerKind, x: i32, y: i32, time_ms: u32) {
        let mods = input_mods(held());
        let Some(entry) = self.tab().and_then(|t| t.entry(id)) else { return };
        let event = PointerEvent::new(kind, (x - entry.rect.x) as f32, (y - entry.rect.y) as f32).with_mods(mods).with_time(time_ms);
        let effects = entry.pane.pointer(&event, &Self::pointer_grid(entry));
        if effects.selected && self.settings.copy_on_select {
            self.copy_selection();
        }
        if let Some(uri) = effects.open_link
            && let Err(e) = open::shell_open(&uri)
        {
            util::log(&format!("open link: {e}"));
        }
        self.redraw();
    }

    pub fn on_mouse_move(&mut self, x: i32, y: i32, time_ms: u32) {
        let hit = self.hit_test(x, y);
        if hit != self.hover {
            self.hover = hit;
            self.redraw();
        }
        if let Some(path) = self.dragged_splitter.clone() {
            let area = self.pane_area();
            if let Some(tab) = self.tabs.get_mut(self.selected) {
                tab.drag_splitter(area, &path, x, y);
            }
            self.relayout();
            self.redraw();
            return;
        }
        self.over_splitter = self.splitter_at(x, y).map(|s| s.axis);
        let dragging = self.buttons.contains(&true);
        let under = self.tab().and_then(|t| t.pane_at(x, y));
        if dragging {
            if let Some(id) = self.pointer_pane {
                self.pointer(id, PointerKind::Move, x, y, time_ms);
            }
        } else if let Some(id) = under {
            self.pointer(id, PointerKind::Move, x, y, time_ms);
        }
        self.refresh_hover_link(x, y, if dragging { None } else { under });
    }

    /// Underlines the link under the pointer (and asks for the hand cursor); removes both when there is none.
    fn refresh_hover_link(&mut self, x: i32, y: i32, over: Option<PaneId>) {
        let link = over.and_then(|id| {
            let entry = self.tab()?.entry(id)?;
            let grid = Self::pointer_grid(entry);
            let (px, py) = ((x - entry.rect.x) as f32, (y - entry.rect.y) as f32);
            let inside = px >= 0.0
                && py >= 0.0
                && px < grid.cell_width * f32::from(grid.columns)
                && py < grid.cell_height * f32::from(grid.rows);
            let (col, row) = inside.then(|| nekominal_input::cell_at(px, py, &grid))?;
            entry.pane.link_at(usize::from(row), usize::from(col)).map(|link| (id, link))
        });
        let hand = link.is_some();
        let spans: Vec<r::RowSpan> = link
            .iter()
            .flat_map(|(_, link)| link.spans.iter())
            .map(|&(row, start, end)| r::RowSpan { row: row as u16, start: start as u16, end: end as u16 })
            .collect();
        let hovered = link.as_ref().map(|(id, _)| *id);
        if let Some(tab) = self.tabs.get_mut(self.selected) {
            for entry in &mut tab.panes {
                entry.view.set_hover_link(if Some(entry.id) == hovered { spans.clone() } else { Vec::new() });
            }
        }
        if hand != self.hand {
            self.hand = hand;
            self.redraw();
        }
    }

    /// The cursor the window shows over the panes instead of the arrow.
    pub fn cursor(&self) -> Option<Cursor> {
        match self.over_splitter {
            Some(Axis::Row) => Some(Cursor::ResizeH),
            Some(Axis::Column) => Some(Cursor::ResizeV),
            None => self.hand.then_some(Cursor::Hand),
        }
    }

    fn splitter_at(&self, x: i32, y: i32) -> Option<nekominal_model::layout::Splitter> {
        let slop = (3.0 * self.scale).round() as i32;
        self.tab()?.splitter_at(self.pane_area(), logic::divider_px(f64::from(self.scale)), x, y, slop)
    }

    pub fn on_mouse_leave(&mut self) {
        if self.hover != Hit::None {
            self.hover = Hit::None;
            self.redraw();
        }
        self.refresh_hover_link(0, 0, None);
    }

    /// A mouse button changed. The result says whether the window should take or give back the mouse capture.
    pub fn on_button(&mut self, x: i32, y: i32, button: MouseButton, down: bool, time_ms: u32) -> Capture {
        let hit = self.hit_test(x, y);
        // A left press outside the panes, and its release, are for the chrome.
        if button == MouseButton::Left && (self.pressed != Hit::None || (down && hit != Hit::Pane)) {
            if down {
                self.pressed = hit;
                self.redraw();
                return Capture::None;
            }
            let pressed = std::mem::replace(&mut self.pressed, Hit::None);
            self.redraw();
            if pressed == hit {
                match hit {
                    Hit::Chip(i) => self.select(i),
                    Hit::ChipClose(i) => self.close_tab(i),
                    Hit::NewTab => {
                        if let Err(e) = self.new_tab() {
                            util::log(&format!("new tab: {e}"));
                        }
                    }
                    Hit::TabList => self.open_tab_list(),
                    Hit::Menu => self.open_menu(),
                    _ => {}
                }
            }
            return Capture::None;
        }
        if button == MouseButton::Left && !self.buttons.contains(&true) {
            if down && let Some(splitter) = self.splitter_at(x, y) {
                self.dragged_splitter = Some(splitter.path);
                return Capture::Take;
            }
            if !down && self.dragged_splitter.take().is_some() {
                return Capture::Release;
            }
        }
        let index = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            // The side buttons have no use in the terminal yet.
            MouseButton::Back | MouseButton::Forward => return Capture::None,
        };
        let before = self.buttons.contains(&true);
        let target = if down && !before {
            // A press starts in the pane under it, which takes the keyboard.
            let Some(id) = self.tab().and_then(|t| t.pane_at(x, y)) else { return Capture::None };
            self.pointer_pane = Some(id);
            if self.tabs[self.selected].focus(id) {
                self.sync_focus();
            }
            id
        } else {
            let Some(id) = self.pointer_pane else { return Capture::None };
            id
        };
        self.buttons[index] = down;
        let program_has_it = self
            .tab()
            .and_then(|t| t.entry(target))
            .is_some_and(|e| e.pane.input_modes().program_gets_pointer(input_mods(held())));
        if down && button == MouseButton::Right && !program_has_it {
            // The terminal owns the right button: copy what is selected, paste when nothing is.
            if self.focused().is_some_and(|e| e.pane.selection_text().is_some()) {
                self.copy_selection();
                if let Some(entry) = self.focused() {
                    entry.pane.clear_selection();
                }
            } else {
                self.paste();
            }
            self.buttons[index] = false;
            self.redraw();
            return Capture::None;
        }
        let kind = if down { PointerKind::Press(button) } else { PointerKind::Release(button) };
        self.pointer(target, kind, x, y, time_ms);
        let after = self.buttons.contains(&true);
        if !after {
            self.pointer_pane = None;
        }
        match (before, after) {
            (false, true) => Capture::Take,
            (true, false) => Capture::Release,
            _ => Capture::None,
        }
    }

    pub fn on_wheel(&mut self, x: i32, y: i32, delta: i16, time_ms: u32) {
        if let Some(id) = self.tab().and_then(|t| t.pane_at(x, y)) {
            self.pointer(id, PointerKind::Wheel { delta: i32::from(delta), horizontal: false }, x, y, time_ms);
        }
    }

    /// The mouse capture ended: held buttons are no longer held.
    pub fn on_capture_lost(&mut self) {
        self.buttons = [false; 3];
        self.dragged_splitter = None;
        let Some(id) = self.pointer_pane.take() else { return };
        let Some(entry) = self.tab().and_then(|t| t.entry(id)) else { return };
        let effects = entry.pane.pointer_cancel(&Self::pointer_grid(entry));
        if effects.selected && self.settings.copy_on_select {
            self.copy_selection();
        }
        self.redraw();
    }

    /// A step of the scrolling a drag held outside the pane asks for.
    pub fn on_drag_tick(&mut self) {
        let Some(id) = self.pointer_pane else { return };
        if self.tab().and_then(|t| t.entry(id)).is_some_and(|e| e.pane.drag_scroll()) {
            self.redraw();
        }
    }

    // --- input method -------------------------------------------------------------------------------------------

    /// The text being composed, drawn at the cursor; `None` when the composition ended.
    pub fn on_ime_preedit(&mut self, text: Option<String>) {
        if let Some(entry) = self.tabs.get_mut(self.selected).and_then(Tab::focused_entry_mut) {
            entry.view.set_preedit(text);
        }
        self.redraw();
    }

    /// Text the input method finished: it goes to the program as typed.
    pub fn on_ime_commit(&mut self, text: &str) {
        if let Some(entry) = self.focused() {
            entry.pane.send_text(text);
        }
    }

    /// Where the cursor cell is in the window, in pixels: left, top and height (the candidate window hangs under it).
    pub fn ime_anchor(&self) -> Option<(i32, i32, i32)> {
        let entry = self.focused()?;
        let (x, y, _, height) = entry.view.cursor_cell_px(&entry.pane.term);
        Some((entry.rect.x + x as i32, entry.rect.y + y as i32, height as i32))
    }

    // --- clipboard ----------------------------------------------------------------------------------------------

    fn copy_selection(&mut self) -> bool {
        self.focused().and_then(|e| e.pane.selection_text()).is_some_and(|text| clipboard::write_text(&text))
    }

    fn paste(&mut self) {
        if let (Some(text), Some(entry)) = (clipboard::read_text(), self.focused()) {
            entry.pane.paste(&text);
        }
    }

    // --- keyboard -----------------------------------------------------------------------------------------------

    pub fn run(&mut self, command: Command) {
        match command {
            Command::TabNew => {
                if let Err(e) = self.new_tab() {
                    util::log(&format!("new tab: {e}"));
                }
            }
            Command::TabClose => self.close_tab(self.selected),
            Command::TabNext => self.select((self.selected + 1) % self.tabs.len().max(1)),
            Command::TabPrevious => self.select((self.selected + self.tabs.len().max(1) - 1) % self.tabs.len().max(1)),
            Command::PaneSplitRight => self.split(Axis::Row),
            Command::PaneSplitDown => self.split(Axis::Column),
            Command::PaneClose => {
                if let Some(id) = self.tab().map(|t| t.focused) {
                    self.close_pane(self.selected, id);
                }
            }
            Command::PaneZoom => {
                if let Some(tab) = self.tabs.get_mut(self.selected) {
                    tab.zoomed = !tab.zoomed && tab.panes.len() > 1;
                }
                self.relayout();
            }
            Command::PaneFocus(direction) => {
                if self.tabs.get_mut(self.selected).is_some_and(|t| t.focus_direction(direction)) {
                    self.sync_focus();
                }
            }
            Command::PaneResize(direction) => {
                if let Some(tab) = self.tabs.get_mut(self.selected) {
                    tab.resize_focused(direction);
                }
                self.relayout();
            }
            Command::AppPalette => self.open_palette(),
            Command::EditFind => self.open_find(),
            Command::ViewFullscreen => self.toggle_fullscreen(),
            Command::AppearanceBackdropNext => {
                let next = Backdrop::ALL[(self.settings.appearance.backdrop.index() as usize + 1) % Backdrop::ALL.len()];
                self.set_appearance(next, self.settings.appearance.opacity);
            }
            Command::AppearanceOpacityUp => {
                self.set_appearance(self.settings.appearance.backdrop, self.settings.appearance.opacity_after(1));
            }
            Command::AppearanceOpacityDown => {
                self.set_appearance(self.settings.appearance.backdrop, self.settings.appearance.opacity_after(-1));
            }
            Command::EditCopy => {
                self.copy_selection();
            }
            Command::EditSmartCopy => {
                if self.copy_selection()
                    && let Some(entry) = self.focused()
                {
                    entry.pane.clear_selection();
                }
            }
            Command::EditPaste => self.paste(),
            other => {
                let Some(entry) = self.focused() else { return };
                match other {
                    Command::EditSelectAll => entry.pane.select_all(),
                    Command::EditClear => entry.pane.clear_buffer(),
                    Command::ScrollPageUp => entry.pane.scroll(Scroll::PageUp),
                    Command::ScrollPageDown => entry.pane.scroll(Scroll::PageDown),
                    Command::ScrollTop => entry.pane.scroll(Scroll::Top),
                    Command::ScrollBottom => entry.pane.scroll(Scroll::Bottom),
                    Command::PromptPrevious => {
                        entry.pane.scroll_to_prompt(false);
                    }
                    Command::PromptNext => {
                        entry.pane.scroll_to_prompt(true);
                    }
                    unwired => util::log(&format!("command {} is not wired yet", unwired.id())),
                }
            }
        }
        self.redraw();
    }

    /// A key went down. `true` when the window handled it (a shortcut, or the terminal encoded it).
    pub fn on_key_down(&mut self, vk_code: i32, scan: u16, extended: bool, repeat: bool) -> bool {
        let held = held();
        if let Some(name) = nekominal_input::physical_key(scan, extended).map(|key| key.name())
            && let Some(command) = self.keymap.find(held, name)
            && !(command == Command::EditSmartCopy && self.focused().is_none_or(|e| e.pane.selection_text().is_none()))
        {
            if !repeat {
                self.run(command);
            }
            return true;
        }
        if vk_code == vk::VK_PROCESSKEY || vk_code == vk::VK_PACKET {
            return false;
        }
        let mods = input_mods(held);
        let Some(key) = vk::key_for_vk_scan(vk_code, scan, extended, mods) else { return false };
        let Some(entry) = self.focused() else { return false };
        let event = if repeat { KeyEvent::Repeat } else { KeyEvent::Press };
        entry.pane.send_key(&KeyInput::new(key).with_mods(mods).with_event(event));
        true
    }

    /// One UTF-16 unit of text from the keyboard or the IME.
    pub fn on_char(&mut self, unit: u16) {
        if unit < 0x20 || unit == 0x7F {
            return;
        }
        if let Some(ch) = self.text.push(unit)
            && let Some(entry) = self.focused()
        {
            entry.pane.send_text(&ch.to_string());
        }
    }

    // --- drawing ------------------------------------------------------------------------------------------------

    pub fn paint(&mut self) {
        let size = self.surface.size();
        let Some(view) = self.surface.acquire() else {
            self.redraw();
            return;
        };

        let highlights = self.find_highlights();
        let mut first = true;
        if let Some(tab) = self.tabs.get_mut(self.selected) {
            let focused = tab.focused;
            for id in tab.shown() {
                let Some(entry) = tab.entry_mut(id) else { continue };
                entry.view.set_highlights(if id == focused { highlights.clone() } else { Vec::new() });
                entry.view.prepare(&entry.pane.term);
                let rect = [entry.rect.x.max(0) as u32, entry.rect.y.max(0) as u32, entry.rect.w.max(1) as u32, entry.rect.h.max(1) as u32];
                entry.view.draw_in(&view, rect, first);
                first = false;
            }
        }
        if first {
            // Nothing to draw: the target still has to be cleared to transparent.
            self.gpu.clear(&view, [0.0; 4]);
        }

        self.ui.begin_frame(size, self.scale);
        self.paint_chrome();
        self.ui.end_frame(&view);
        self.surface.present();
    }

    fn paint_chrome(&mut self) {
        let g = self.glass;
        let width = self.layout.size.0;
        let tint = {
            let opacity = glass::terminal_opacity(self.settings.appearance.backdrop, self.settings.appearance.opacity, glass::Material { window_active: self.active, effects_on: true });
            let alpha = glass::bar_tint_alpha(opacity);
            Color([g.background.r, g.background.g, g.background.b, (alpha * 255.0).round() as u8])
        };
        self.ui.fill(Rect::new(0.0, 0.0, width, layout::TITLE_HEIGHT), tint);
        self.ui.fill(Rect::new(0.0, layout::TITLE_HEIGHT - 1.0, width, 1.0), col(g.divider()));

        // The dividers between panes.
        let area = self.pane_area();
        let divider = logic::divider_px(f64::from(self.scale));
        if let Some(tab) = self.tabs.get(self.selected) {
            let s = self.scale;
            for (_, rect) in tab.splitters(area, divider) {
                self.ui.fill(Rect::new(rect.x as f32 / s, rect.y as f32 / s, rect.w as f32 / s, rect.h as f32 / s), col(g.divider()));
            }
        }

        let line = self.ui.line_height(TEXT_SIZE);
        for (i, chip) in self.layout.chips.clone().into_iter().enumerate() {
            if !self.layout.is_shown(i) {
                continue;
            }
            let selected = i == self.selected;
            let hover = matches!(self.hover, Hit::Chip(n) | Hit::ChipClose(n) if n == i);
            let (fill, border) = if selected {
                (col(g.chip_selected()), Some((col(g.chip_selected_border()), 1.0)))
            } else if hover {
                (col(g.hover()), None)
            } else {
                (Color::TRANSPARENT, None)
            };
            self.ui.rrect(chip, CORNER_RADIUS, fill, border);
            let title = if logic::shows_ordinal(f64::from(chip.w)) {
                (i + 1).to_string()
            } else {
                self.tab_label(i)
            };
            let close = Layout::close_button(chip).filter(|_| selected || hover);
            let right = close.map_or(chip.x + chip.w - 10.0, |c| c.x - 4.0);
            self.ui.push_clip(Rect::new(chip.x, chip.y, (right - chip.x).max(0.0), chip.h));
            let color = if selected { col(g.text()) } else { col(g.text_secondary()) };
            self.ui.text(chip.x + 12.0, chip.y + (chip.h - line) / 2.0, &title, TEXT_SIZE, color, false);
            self.ui.pop_clip();
            if let Some(close) = close {
                if self.hover == Hit::ChipClose(i) {
                    self.ui.rrect(close, CORNER_RADIUS, col(g.pressed()), None);
                }
                self.icon_centered(close, '\u{E711}', ICON_SIZE, col(g.text()));
            }
        }
        let plus = self.layout.new_tab;
        if self.hover == Hit::NewTab {
            self.ui.rrect(plus, CORNER_RADIUS, col(g.hover()), None);
        }
        self.icon_centered(plus, '\u{E710}', 12.0, col(g.text()));
        let list = self.layout.tab_list;
        if self.hover == Hit::TabList || self.popup.as_ref().is_some_and(|p| p.rows.iter().any(|r| matches!(r.action, Some(popup::Action::Tab(_))))) {
            self.ui.rrect(list, CORNER_RADIUS, col(g.hover()), None);
        }
        self.icon_centered(list, '\u{E70D}', 12.0, col(g.text()));
        // The position plate: where the selected tab is among all of them, which a strip of narrow chips cannot say.
        let strip = self.layout.tab_list.x;
        let tier = if self.layout.size.0 < logic::MEDIUM_FROM as f32 { logic::BarTier::Narrow } else { logic::BarTier::Wide };
        if let Some(plate) = logic::plate(self.selected, self.tabs.len(), f64::from(strip), tier) {
            let text = match plate {
                logic::Plate::Position { current, total } => format!("{current} / {total}"),
                logic::Plate::Total(total) => total.to_string(),
            };
            let (x, room) = (list.x + list.w + 8.0, self.layout.menu.x - list.x - list.w - 12.0);
            let line = self.ui.line_height(TEXT_SIZE);
            self.ui.push_clip(Rect::new(x, list.y, room.max(0.0), list.h));
            self.ui.text(x, list.y + (list.h - line) / 2.0, &text, TEXT_SIZE, col(g.text_secondary()), false);
            self.ui.pop_clip();
        }
        let menu = self.layout.menu;
        if self.hover == Hit::Menu || self.popup.is_some() {
            self.ui.rrect(menu, CORNER_RADIUS, col(g.hover()), None);
        }
        self.icon_centered(menu, '\u{E712}', 12.0, col(g.text()));
    }

    // --- the menu -----------------------------------------------------------------------------------------------

    fn language(&self) -> Lang {
        let mut name = [0u16; 85];
        // SAFETY: the buffer has the room LOCALE_NAME_MAX_LENGTH asks for.
        let len = unsafe { GetUserDefaultLocaleName(&mut name) };
        let tag = String::from_utf16_lossy(&name[..(len.max(1) as usize - 1)]);
        self.settings.language.resolve(&tag)
    }

    /// The rows of the menu: `APP_MENU` without what the shell cannot do yet, and without separators left dangling.
    fn menu_rows(&self) -> Vec<popup::Row> {
        // shortcut: only the commands `run` handles are offered; the find bar, full screen and the settings join
        // when they exist, and the backdrop and opacity rows with the settings drawer.
        let lang = self.language();
        let mut rows: Vec<popup::Row> = Vec::new();
        for row in APP_MENU {
            match row {
                MenuRow::Command(command) if wired(*command) => rows.push(popup::Row {
                    label: command.label(lang),
                    hint: self.keymap.shortcut_text(*command).to_string(),
                    action: Some(popup::Action::Command(*command)),
                    current: false,
                }),
                MenuRow::Backdrop => {
                    for backdrop in Backdrop::ALL {
                        let name = match backdrop {
                            Backdrop::Acrylic => Text::BackdropAcrylic,
                            Backdrop::Mica => Text::BackdropMica,
                            Backdrop::MicaAlt => Text::BackdropMicaAlt,
                            Backdrop::None => Text::BackdropNone,
                        };
                        rows.push(popup::Row {
                            label: format!("{}: {}", Text::SettingsBackdrop.get(lang), name.get(lang)),
                            hint: String::new(),
                            action: Some(popup::Action::Backdrop(backdrop)),
                            current: backdrop == self.settings.appearance.backdrop,
                        });
                    }
                }
                MenuRow::Opacity => {
                    for command in [Command::AppearanceOpacityUp, Command::AppearanceOpacityDown] {
                        rows.push(popup::Row {
                            label: command.label(lang),
                            hint: format!("{}%", (self.settings.appearance.opacity * 100.0).round()),
                            action: Some(popup::Action::Command(command)),
                            current: false,
                        });
                    }
                }
                MenuRow::Separator if rows.last().is_some_and(|r| !r.is_separator()) => rows.push(popup::Row::separator()),
                _ => {}
            }
        }
        if rows.last().is_some_and(popup::Row::is_separator) {
            rows.pop();
        }
        rows
    }

    /// The screen point (pixels) at the bottom right of a button (DIP): where a list hangs from.
    fn button_anchor(&self, button: Rect) -> (i32, i32) {
        self.screen_point((button.x + button.w) * self.scale, (button.y + button.h) * self.scale)
    }

    fn screen_point(&self, x: f32, y: f32) -> (i32, i32) {
        let mut point = POINT { x: x.round() as i32, y: y.round() as i32 };
        // SAFETY: converts a point of our own window.
        unsafe { let _ = ClientToScreen(self.hwnd, &mut point); }
        (point.x, point.y)
    }

    fn open_menu(&mut self) {
        let rows = self.menu_rows();
        let anchor = self.button_anchor(self.layout.menu);
        self.open_popup(rows, popup::Kind::List, anchor);
    }

    /// The command palette: what matches `query` of the commands the shell can run and the open tabs.
    fn palette_rows(&self, query: &str) -> Vec<popup::Row> {
        let tabs: Vec<PaletteTab> = (0..self.tabs.len()).map(|i| PaletteTab { id: i as u32, title: self.tab_label(i) }).collect();
        let items = palette::items(self.language(), &self.keymap, &[], &tabs);
        palette::search(items, query)
            .into_iter()
            .filter_map(|item| {
                let action = match item.action {
                    PaletteAction::Run(command) if wired(command) => popup::Action::Command(command),
                    PaletteAction::GotoTab(index) => popup::Action::Tab(index as usize),
                    // shortcut: new tab with a profile, color scheme and settings entries wait for the settings
                    // drawer and the profile picker.
                    _ => return None,
                };
                Some(popup::Row { label: item.label, hint: item.shortcut, action: Some(action), current: false })
            })
            .collect()
    }

    fn open_palette(&mut self) {
        let rows = self.palette_rows("");
        // Centered under the top bar: the pop-up hangs from its top right corner.
        let anchor = self.screen_point(
            (self.layout.size.0 + popup::PALETTE_WIDTH) / 2.0 * self.scale,
            (layout::TITLE_HEIGHT + 16.0) * self.scale,
        );
        self.open_popup(rows, popup::Kind::Palette, anchor);
    }

    /// Puts a backdrop and an opacity of the background in effect, and keeps them in the settings file.
    fn set_appearance(&mut self, backdrop: Backdrop, opacity: f64) {
        let appearance = &mut self.settings.appearance;
        if (appearance.backdrop, appearance.opacity) == (backdrop, opacity) {
            return;
        }
        (appearance.backdrop, appearance.opacity) = (backdrop, opacity);
        let scheme = themes::scheme(&self.settings.theme);
        let shown = glass::terminal_opacity(backdrop, opacity, glass::Material::NORMAL);
        self.theme = glass::render_theme(scheme, shown);
        for entry in self.tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()) {
            entry.view.set_theme(self.theme.clone());
        }
        crate::win::set_backdrop(self.hwnd, backdrop, self.dark());
        if let Some(path) = store::settings_path()
            && let Err(e) = store::save_keeping_original(&path, &settings::serialize(&self.settings))
        {
            util::log(&format!("settings: {e}"));
        }
        self.redraw();
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen.is_some()
    }

    /// Full screen is the window without its frame over the whole monitor it is on; the way back is the style and
    /// the placement it had.
    fn toggle_fullscreen(&mut self) {
        // SAFETY: plain Win32 calls on our own window; the structures live through the calls.
        unsafe {
            if let Some((placement, style)) = self.fullscreen.take() {
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, style);
                let _ = SetWindowPlacement(self.hwnd, &placement);
                let _ = SetWindowPos(self.hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED);
                return;
            }
            let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE);
            let mut placement = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetWindowPlacement(self.hwnd, &mut placement).is_err()
                || !GetMonitorInfoW(MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool()
            {
                return;
            }
            self.fullscreen = Some((placement, style));
            SetWindowLongPtrW(self.hwnd, GWL_STYLE, style & !(WS_OVERLAPPEDWINDOW.0 as isize));
            let r = monitor.rcMonitor;
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOP), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOOWNERZORDER | SWP_FRAMECHANGED);
        }
    }

    fn open_find(&mut self) {
        let anchor = self.screen_point(self.layout.size.0 * self.scale - 24.0 * self.scale, layout::TITLE_HEIGHT * self.scale);
        self.open_popup(Vec::new(), popup::Kind::Find, anchor);
    }

    /// Searches the focused pane for what the bar holds, and shows the last match (the nearest to the prompt).
    fn find_search(&mut self, query: &str) {
        let matches = match self.focused() {
            Some(entry) if !query.is_empty() => {
                entry.pane.term.lock().search_all(&find::literal_pattern(query), FIND_LIMIT).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let current = matches.len().saturating_sub(1);
        self.find = Some(FindState { matches, current });
        self.find_show_current();
    }

    fn find_step(&mut self, forward: bool) {
        if let Some(find) = self.find.as_mut() {
            find.current = find::step(find.current, find.matches.len(), forward);
        }
        self.find_show_current();
    }

    /// Scrolls the pane to the current match when it is out of view, and updates the count in the bar.
    fn find_show_current(&mut self) {
        let Some(find) = self.find.as_ref() else { return };
        let query = self.popup.as_ref().and_then(|p| p.query.clone()).unwrap_or_default();
        let status = find::status(&query, find.current, find.matches.len());
        if let (Some(found), Some(entry)) = (find.matches.get(find.current), self.focused()) {
            let mut core = entry.pane.term.lock();
            let (rows, top) = (core.rows() as i32, -(core.display_offset() as i32));
            if found.start.0 < top || found.end.0 > top + rows - 1 {
                core.scroll_to_line(found.start.0 - rows / 2);
            }
        }
        if let Some(popup) = self.popup.as_mut() {
            popup.status = status;
            // SAFETY: invalidating our own window.
            unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
        }
        self.redraw();
    }

    /// The matches of the find bar that are in view in the focused pane, for the renderer.
    fn find_highlights(&self) -> Vec<r::Highlight> {
        let (Some(find), Some(entry)) = (self.find.as_ref(), self.focused()) else { return Vec::new() };
        let core = entry.pane.term.lock();
        let (rows, columns, offset) = (core.rows(), core.columns(), core.display_offset());
        let (top, bottom) = (-(offset as i32), rows as i32 - 1 - offset as i32);
        let mut highlights = Vec::new();
        for (i, found) in find.matches.iter().enumerate() {
            if found.end.0 < top || found.start.0 > bottom {
                continue;
            }
            let mut spans = Vec::new();
            pane_view::match_spans(found.start, found.end, offset, rows, columns, &mut spans);
            highlights.extend(spans.into_iter().map(|span| r::Highlight { span, current: i == find.current }));
        }
        highlights
    }

    /// A character typed into the palette: it goes into the query, and the rows follow.
    pub fn popup_char(&mut self, unit: u16) {
        let Some(query) = self.popup.as_ref().and_then(|p| p.query.clone()) else { return };
        let mut query = query;
        match unit {
            0x08 => {
                query.pop();
            }
            u if u >= 0x20 && u != 0x7F => {
                // Surrogates (outside the BMP) are left out: the input method sends them one half at a time.
                let Some(ch) = char::from_u32(u32::from(u)) else { return };
                query.push(ch);
            }
            _ => return,
        }
        if self.popup.as_ref().is_some_and(|p| p.kind == popup::Kind::Find) {
            if let Some(popup) = self.popup.as_mut() {
                popup.query = Some(query.clone());
            }
            self.find_search(&query);
            return;
        }
        let rows = self.palette_rows(&query);
        let Some(popup) = self.popup.as_mut() else { return };
        popup.query = Some(query);
        popup.set_rows(rows);
        // SAFETY: invalidating our own window.
        unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
    }

    /// The list of all tabs: one row per tab with its number, the selected one marked.
    fn open_tab_list(&mut self) {
        let rows = (0..self.tabs.len())
            .map(|i| popup::Row {
                label: self.tab_label(i),
                hint: (i + 1).to_string(),
                action: Some(popup::Action::Tab(i)),
                current: i == self.selected,
            })
            .collect();
        let anchor = self.button_anchor(self.layout.tab_list);
        self.open_popup(rows, popup::Kind::List, anchor);
        let selected = self.selected;
        if let Some(popup) = self.popup.as_mut() {
            popup.hover = Some(selected);
            popup.reveal(selected);
        }
    }

    /// Opens a list with its top right corner at the screen point `anchor`.
    fn open_popup(&mut self, rows: Vec<popup::Row>, kind: popup::Kind, anchor: (i32, i32)) {
        if self.popup.is_some() || self.popup_closed.is_some_and(|at| at.elapsed() < Duration::from_millis(250)) {
            return;
        }
        match popup::open(self.hwnd, &self.gpu, rows, kind, anchor, self.scale, self.dark()) {
            Ok(popup) => {
                // SAFETY: invalidating the window just made.
                unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
                self.popup = Some(popup);
            }
            Err(e) => util::log(&format!("menu: {e}")),
        }
        self.redraw();
    }

    pub fn close_popup(&mut self) {
        let Some(popup) = self.popup.take() else { return };
        self.find = None;
        self.popup_closed = Some(Instant::now());
        // SAFETY: destroys a window of this thread; the swap chain goes with the `Popup`.
        unsafe { let _ = DestroyWindow(popup.hwnd); }
        drop(popup);
        self.redraw();
    }

    /// The menu lost the activation: another window was clicked, or this one.
    pub fn popup_deactivated(&mut self) {
        self.close_popup();
        // SAFETY: plain query.
        if unsafe { GetForegroundWindow() } != self.hwnd {
            self.on_activate(false);
        }
    }

    /// The pointer moved over the menu (pixels), or left it.
    pub fn popup_move(&mut self, at: Option<(i32, i32)>) {
        let scale = self.scale;
        let Some(popup) = self.popup.as_mut() else { return };
        let row = at.and_then(|(x, y)| popup.row_at_point(x as f32 / scale, y as f32 / scale));
        if row != popup.hover {
            popup.hover = row;
            // SAFETY: invalidating our own window.
            unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
        }
    }

    pub fn popup_click(&mut self, x: i32, y: i32) {
        let scale = self.scale;
        let Some(popup) = self.popup.as_ref() else { return };
        if let Some(row) = popup.row_at_point(x as f32 / scale, y as f32 / scale) {
            self.choose(row);
        }
    }

    pub fn popup_key(&mut self, vk_code: i32) {
        const VK_RETURN: i32 = 0x0D;
        const VK_UP: i32 = 0x26;
        const VK_DOWN: i32 = 0x28;
        let Some(popup) = self.popup.as_mut() else { return };
        if popup.kind == popup::Kind::Find {
            // Enter goes to the next match, with Shift to the one before; the arrows go the same ways.
            let forward = match vk_code {
                VK_RETURN => Some(!held().shift),
                VK_DOWN => Some(true),
                VK_UP => Some(false),
                _ => None,
            };
            if let Some(forward) = forward {
                self.find_step(forward);
            }
            return;
        }
        match vk_code {
            VK_UP | VK_DOWN => {
                popup.hover = popup::step(&popup.rows, popup.hover, vk_code == VK_DOWN);
                if let Some(row) = popup.hover {
                    popup.reveal(row);
                }
                // SAFETY: invalidating our own window.
                unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
            }
            VK_RETURN => {
                if let Some(row) = popup.hover {
                    self.choose(row);
                }
            }
            _ => {}
        }
    }

    /// The wheel over the list (a notch is `delta` 120).
    pub fn popup_wheel(&mut self, delta: i16) {
        let Some(popup) = self.popup.as_mut() else { return };
        popup.scroll_by(-f32::from(delta) / 120.0 * 3.0 * 32.0);
        // SAFETY: invalidating our own window.
        unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
    }

    /// Closes the list and does what its row `row` says.
    fn choose(&mut self, row: usize) {
        let action = self.popup.as_ref().and_then(|p| p.rows[row].action);
        self.close_popup();
        match action {
            Some(popup::Action::Command(command)) => self.run(command),
            Some(popup::Action::Tab(index)) => self.select(index),
            Some(popup::Action::Backdrop(backdrop)) => self.set_appearance(backdrop, self.settings.appearance.opacity),
            None => {}
        }
    }

    /// What tab `i` shows: the title its program set, the profile name until there is one.
    fn tab_label(&self, i: usize) -> String {
        logic::tab_title(self.tabs[i].title(), &self.profile.name)
    }

    pub fn popup_paint(&mut self) {
        let scale = self.scale;
        let g = self.glass;
        let Some(popup) = self.popup.as_mut() else { return };
        let Some(view) = popup.surface.acquire() else { return };
        self.gpu.clear(&view, [0.0; 4]);
        let ui = &mut self.ui;
        ui.begin_frame(popup.surface.size(), scale);
        // The acrylic is the window; a tint over it keeps the text readable on any wallpaper.
        let tint = Color([g.background.r, g.background.g, g.background.b, 140]);
        ui.rrect(Rect::new(0.0, 0.0, popup.size.0, popup.size.1), CORNER_RADIUS, tint, Some((col(g.divider()), 1.0)));
        let line = ui.line_height(TEXT_SIZE);
        let header = popup.header;
        // The rows scroll under the query line, so they are clipped to the part below it.
        ui.push_clip(Rect::new(0.0, header, popup.size.0, popup.size.1 - header));
        for (i, (row, rect)) in popup.rows.iter().zip(&popup.rects).enumerate() {
            let rect = Rect::new(rect.x, header + rect.y - popup.scroll, rect.w, rect.h);
            if rect.y + rect.h < header || rect.y > popup.size.1 {
                continue;
            }
            if row.is_separator() {
                ui.fill(Rect::new(rect.x + 8.0, rect.y + rect.h / 2.0, rect.w - 16.0, 1.0), col(g.divider()));
                continue;
            }
            if popup.hover == Some(i) {
                ui.rrect(rect, CORNER_RADIUS, col(g.hover()), None);
            } else if row.current {
                ui.rrect(rect, CORNER_RADIUS, col(g.chip_selected()), Some((col(g.chip_selected_border()), 1.0)));
            }
            let y = rect.y + (rect.h - line) / 2.0;
            let hint_width = ui.measure(&row.hint, TEXT_SIZE, false);
            ui.text(rect.x + rect.w - 12.0 - hint_width, y, &row.hint, TEXT_SIZE, col(g.text_secondary()), false);
            let label_end = rect.x + rect.w - 12.0 - hint_width - 8.0;
            ui.push_clip(Rect::new(rect.x, rect.y.max(header), (label_end - rect.x).max(0.0), rect.h));
            ui.text(rect.x + 12.0, y, &row.label, TEXT_SIZE, col(g.text()), false);
            ui.pop_clip();
        }
        ui.pop_clip();
        if let Some(query) = &popup.query {
            let y = (header - line) / 2.0;
            let icon_line = ui.line_height(14.0);
            ui.icon(16.0, (header - icon_line) / 2.0, '\u{E721}', 14.0, col(g.text_secondary()));
            let width = ui.text(44.0, y, query, TEXT_SIZE, col(g.text()), false);
            // The caret: a bar after the text, not blinking, so the window needs no timer.
            ui.fill(Rect::new(44.0 + width + 1.0, y, 1.5, line), col(g.text()));
            if !popup.status.is_empty() {
                let width = ui.measure(&popup.status, TEXT_SIZE, false);
                ui.text(popup.size.0 - 16.0 - width, y, &popup.status, TEXT_SIZE, col(g.text_secondary()), false);
            }
            if popup.kind != popup::Kind::Find {
                ui.fill(Rect::new(0.0, header - 1.0, popup.size.0, 1.0), col(g.divider()));
            }
        }
        ui.end_frame(&view);
        popup.surface.present();
    }

    fn icon_centered(&mut self, rect: Rect, ch: char, size: f32, color: Color) {
        let line = self.ui.line_height(size);
        let width = self.ui.measure_icon(ch, size);
        self.ui.icon(rect.x + (rect.w - width) / 2.0, rect.y + (rect.h - line) / 2.0, ch, size, color);
    }
}
