//! The application of one window: tabs with panes, the chrome drawn around them, and the input.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use accesskit_windows::{Adapter, QueuedEvents};
use nekominal_input::{
    KeyEvent, KeyInput, Mods, MouseButton, PointerEvent, PointerGrid, PointerKind, Utf16Assembler, vk,
};
use nekominal_model::{
    bell::{self, BellContext, BellEffects},
    commands::{self, Command},
    palette::{self, PaletteAction, PaletteTab},
    keymap::{self, Keymap, Modifiers},
    layout::{Axis, PaneId},
    menu::{APP_MENU, MenuRow},
    profiles::{self, DiscoveredProfile, Profile},
    settings::{self, Backdrop, RightClick, Settings},
    settings_ui::{self, Control, Descriptor, Section, Value},
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
use windows::{core::HSTRING, Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Globalization::GetUserDefaultLocaleName,
    System::Diagnostics::Debug::MessageBeep,
    Graphics::Gdi::{ClientToScreen, ScreenToClient, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    UI::{
        Input::KeyboardAndMouse::GetKeyState,
        WindowsAndMessaging::{
            DestroyWindow, FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx, GWL_STYLE, GetForegroundWindow, GetSystemMetrics, MESSAGEBOX_STYLE,
            SM_SHOWSOUNDS, SetWindowTextW, GetWindowLongPtrW, GetWindowPlacement, GetWindowRect, HWND_TOP, PostMessageW,
            SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPlacement,
            SetWindowPos, WINDOWPLACEMENT, WM_APP, WM_CLOSE, WS_OVERLAPPEDWINDOW,
        },
    },
}};

use crate::{
    access, clipboard, find, motion,
    settings_page::{self, Page, Part},
    glass::{self, Glass},
    gpu::{self, Gpu, Surface},
    layout::{self, Hit, Layout},
    logic::{self, Attention, PxRect},
    open, popup, status,
    tab::{PaneEntry, Tab, WakeTarget},
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

/// How far below the title bar a dragged chip is let go to count as pulled out of the window, in DIP.
const TEAR_MARGIN: f32 = 12.0;

/// Corner radius of every rounded rectangle, in DIP (the same number as `design.rs` of the WinUI shell).
pub const CORNER_RADIUS: f32 = 8.0;
const TEXT_SIZE: f32 = 13.0;
const ICON_SIZE: f32 = 10.0;
/// How far the caption glyphs move down so that they sit in the middle of the title bar, in DIP.
const CAPTION_GLYPH_NUDGE: f32 = 1.5;

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
    /// Every profile a tab can open, the default among them.
    profiles: Vec<Profile>,
    /// The settings page while it is open: it takes the place of the panes.
    page: Option<Page>,
    /// When the taskbar button last flashed or a sound last played: they are limited to one a second.
    last_alert: Option<Instant>,
    /// The right button opened the menu and is still down: letting go of it must not close the menu.
    context_release: bool,
    /// The button of the bar that opened the pop-up that is open: it stays lit while the pop-up hangs from it.
    popup_from: Option<Hit>,
    /// Where each tab chip is drawn now, by the number of its tab, while it slides to the place the layout gives it.
    chip_view: HashMap<u64, Rect>,
    /// The same, in the order of the tabs, for the frame being drawn.
    chip_rects: Vec<Rect>,
    /// When the last frame of a slide was drawn; None while every chip is in its place.
    chip_clock: Option<Instant>,
    chip_size: (u32, u32),
    /// A chip held by the pointer.
    drag: Option<ChipDrag>,
    /// Where the position plate of the tab strip is drawn, when it is: it opens the list of all tabs.
    plate_rect: Option<Rect>,
    /// The title of the window as it was last set.
    window_title: String,
    tabs: Vec<Tab>,
    selected: usize,
    layout: Layout,
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
    /// A chip let go of outside the title bar, for the window code to move.
    detach: Option<Detach>,
    /// The "…" menu while it is open.
    popup: Option<popup::Popup>,
    confirmation: Option<Confirmation>,
    /// The placement and style the window had before full screen, while it is full screen.
    fullscreen: Option<(WINDOWPLACEMENT, isize)>,
    /// The search of the find bar while it is open.
    find: Option<FindState>,
    /// When the menu last closed: the click that closed it (outside it) must not open it again.
    popup_closed: Option<Instant>,
    text: Utf16Assembler,
    /// What a screen reader sees of the window; it asks through WM_GETOBJECT.
    access: Option<Adapter>,
}

/// A tab chip pressed with the left button; it follows the pointer once the pointer has moved a little.
#[derive(Clone, Copy, Debug)]
struct ChipDrag {
    tab: usize,
    /// Where the pointer is in the chip, in DIP.
    grab: f32,
    start: f32,
    /// The pointer, in DIP.
    x: f32,
    active: bool,
}

/// A chip dragged out of the bar: which tab, where the pointer is on the screen (pixels) and how far from the left of the
/// chip it took hold (pixels).
#[derive(Clone, Copy, Debug)]
pub struct Detach {
    pub tab: usize,
    pub pointer: (i32, i32),
    pub grab: f32,
}

/// How far the pointer goes before a pressed chip is dragged, in DIP.
const DRAG_START: f32 = 4.0;

/// The matches of the find bar and the one the bar is on.
struct FindState {
    matches: Vec<SearchMatch>,
    current: usize,
}

/// The exact operation held until the user accepts. Closing any pane cancels it before indices can change.
enum Confirmation {
    Paste { tab: usize, pane: PaneId, text: String },
    ClosePane { tab: usize, pane: PaneId },
    CloseTab(usize),
    CloseWindow,
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
            | Command::AppSettings
            | Command::AppOpenSettingsFile
            | Command::AppReloadSettings
            | Command::TabProfiles
            | Command::AppNewWindow
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
    pub fn new(hwnd: HWND, size_px: (u32, u32), scale: f32, shared_gpu: Option<Gpu>, first_tab: bool) -> Result<App, String> {
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

        let (gpu, surface) = match shared_gpu {
            Some(gpu) => { let surface = gpu::create_surface(&gpu, hwnd, size_px)?; (gpu, surface) }
            None => gpu::create(hwnd, size_px)?,
        };
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
            profiles: all,
            page: None,
            last_alert: None,
            context_release: false,
            popup_from: None,
            chip_view: HashMap::new(),
            chip_rects: Vec::new(),
            chip_clock: None,
            chip_size: (0, 0),
            drag: None,
            plate_rect: None,
            window_title: String::new(),
            tabs: Vec::new(),
            selected: 0,
            layout: Layout::new(1.0, 1.0, 0, 0.0, 0),
            hover: Hit::None,
            pressed: Hit::None,
            buttons: [false; 3],
            pointer_pane: None,
            hand: false,
            over_splitter: None,
            dragged_splitter: None,
            popup: None,
            detach: None,
            confirmation: None,
            fullscreen: None,
            find: None,
            popup_closed: None,
            text: Utf16Assembler::default(),
            access: Some(Adapter::new(hwnd, true, access::NoActions)),
        };
        app.relayout();
        if first_tab {
            app.new_tab()?;
        }
        Ok(app)
    }

    pub fn backdrop(&self) -> Backdrop {
        self.settings.appearance.backdrop
    }

    pub fn dark(&self) -> bool {
        !self.glass.light
    }

    pub fn shared_gpu(&self) -> Gpu { self.gpu.clone() }

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
        let target = Arc::new(WakeTarget { hwnd: AtomicIsize::new(self.hwnd.0 as isize), pending: AtomicBool::new(false) });
        let reader_target = target.clone();
        let wake: nekominal_pane::Wake = Arc::new(move || {
            if !reader_target.pending.swap(true, Ordering::AcqRel) {
                let hwnd = HWND(reader_target.hwnd.load(Ordering::Acquire) as *mut _);
                // SAFETY: posting to a window handle from another thread is allowed; a dead window drops it.
                unsafe { PostMessageW(Some(hwnd), WM_WAKE, WPARAM(0), LPARAM(0)).ok() };
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
            cwd: None,
            profile: self.profile.name.clone(),
            shell: std::path::Path::new(&self.profile.program).file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default(),
            exited: false,
            attention: None,
            note: None,
            progress: None,
            wake: target,
        })
    }

    fn new_tab(&mut self) -> Result<(), String> {
        self.close_page();
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
        if self.confirmation.is_some() {
            self.close_popup();
        }
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
        if self.confirmation.is_some() {
            self.close_popup();
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

    /// Takes a tab out of this window with its sessions running, for another window to adopt. The window closes
    /// when it was the last tab.
    pub fn take_tab(&mut self, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        if self.tabs.is_empty() {
            // SAFETY: closes the window of this thread; nothing is left in it to ask about.
            unsafe { PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).ok() };
            return Some(tab);
        }
        if self.selected > index || self.selected >= self.tabs.len() {
            self.selected = self.selected.saturating_sub(1);
        }
        self.relayout();
        self.sync_focus();
        self.redraw();
        Some(tab)
    }

    /// Shows a tab another window gave up, selected.
    pub fn adopt_tab(&mut self, mut tab: Tab) {
        for entry in &mut tab.panes {
            // The reader threads now wake this window; output that came in during the move gets one message.
            entry.wake.hwnd.store(self.hwnd.0 as isize, Ordering::Release);
            entry.wake.pending.store(true, Ordering::Release);
            if let Err(e) = entry.view.set_scale(self.scale) {
                util::log(&format!("scale: {e}"));
            }
        }
        // SAFETY: posting to our own window.
        unsafe { PostMessageW(Some(self.hwnd), WM_WAKE, WPARAM(0), LPARAM(0)).ok() };
        tab.mark_seen();
        self.tabs.push(tab);
        self.selected = self.tabs.len() - 1;
        self.relayout();
        self.sync_focus();
        self.redraw();
    }

    /// The chip the user dragged out of the title bar: the window code makes it a window that follows the pointer.
    pub fn take_detach(&mut self) -> Option<Detach> {
        self.detach.take()
    }

    /// The drag of a chip ends without a place for it (it became a window of its own).
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.pressed = Hit::None;
        self.redraw();
    }

    /// The pointer is over the strip of tabs or what is empty of the title bar, where a tab can be put down.
    pub fn over_strip(&self, x: i32, y: i32) -> bool {
        !matches!(self.hit_test(x, y), Hit::Pane | Hit::None | Hit::Minimize | Hit::Maximize | Hit::Close)
    }

    /// The last tab (just handed over by another window) is held by the pointer, grab DIP from the left of its chip,
    /// and the drag goes on in this window.
    pub fn begin_chip_drag(&mut self, x: i32, y: i32, grab: f32) {
        let Some(tab) = self.tabs.len().checked_sub(1) else { return };
        let (pointer, _) = self.point(x, y);
        self.pressed = Hit::Chip(tab);
        self.drag = Some(ChipDrag { tab, grab, start: pointer, x: pointer, active: true });
        self.redraw();
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    fn select(&mut self, index: usize) {
        self.close_page();
        if index < self.tabs.len() && index != self.selected {
            self.selected = index;
            self.tabs[index].mark_seen();
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

    /// The pointer is over a caption button (or over none): the window gets these as non-client messages.
    pub fn on_caption_hover(&mut self, hit: Hit) {
        if self.hover != hit {
            self.hover = hit;
            self.redraw();
        }
    }

    pub fn on_caption_press(&mut self, hit: Hit) {
        self.pressed = hit;
        self.redraw();
    }

    /// A caption button was let go of: whether it is the one that was pressed.
    pub fn on_caption_release(&mut self, hit: Hit) -> bool {
        let pressed = std::mem::replace(&mut self.pressed, Hit::None);
        self.redraw();
        pressed == hit
    }

    pub fn on_activate(&mut self, active: bool) {
        if !active { self.close_popup(); }
        self.active = active;
        self.update_window_title();
        if active && let Some(tab) = self.tabs.get_mut(self.selected) {
            tab.mark_seen();
        }
        self.sync_focus();
        self.redraw();
    }

    /// Chips, panes and the cells each shown pane has.
    fn relayout(&mut self) {
        let (w, h) = self.surface.size();
        self.layout = Layout::new(w as f32 / self.scale, h as f32 / self.scale, self.tabs.len(), layout::CAPTION, self.selected);
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
        for entry in self.tabs.iter().flat_map(|tab| &tab.panes) {
            entry.wake.pending.store(false, Ordering::Release);
        }
        let mut ended: Vec<(usize, PaneId)> = Vec::new();
        let (selected, active) = (self.selected, self.active);
        let mut ring = BellEffects::default();
        // SAFETY: a plain query.
        let show_sounds = unsafe { GetSystemMetrics(SM_SHOWSOUNDS) } != 0;
        for (t, tab) in self.tabs.iter_mut().enumerate() {
            // Bells and notifications are for what the user is not looking at.
            let watched = t == selected && active;
            let focused = tab.focused;
            for entry in &mut tab.panes {
                while let Ok(event) = entry.pane.events.try_recv() {
                    entry.progress = status::progress_after(entry.progress, &event);
                    match event {
                        UiEvent::Title(title) => entry.title = logic::clean_title(&title),
                        UiEvent::Cwd(path) => entry.cwd = Some(path),
                        UiEvent::ClipboardStore(text) => {
                            clipboard::write_text(&text);
                        }
                        UiEvent::ClipboardLoad(request) => {
                            entry.pane.answer_clipboard(&request, &clipboard::read_text().unwrap_or_default());
                        }
                        UiEvent::Bell => {
                            let context = BellContext {
                                window_active: active,
                                pane_in_view: t == selected && focused == entry.id,
                                since_last_alert: self.last_alert.map(|at| at.elapsed()),
                                show_sounds,
                            };
                            let effects = bell::bell_effects(&self.settings.bell, &context);
                            if effects.chip {
                                entry.attention = entry.attention.or(Some(Attention::Bell));
                            }
                            ring.taskbar_flash |= effects.taskbar_flash;
                            ring.sound |= effects.sound;
                        }
                        UiEvent::Notification(notification) if !watched => {
                            if entry.attention != Some(Attention::Exit) {
                                entry.attention = Some(Attention::Notification);
                            }
                            let text = format!("{} {}", notification.title, notification.body);
                            entry.note = Some(logic::clean_title(&text));
                        }
                        UiEvent::Exit(code) => {
                            entry.exited = true;
                            if code == 0 {
                                ended.push((t, entry.id));
                            } else {
                                entry.attention = Some(Attention::Exit);
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
        self.alert(ring);
        self.update_window_title();
        self.find_refresh();
        self.redraw();
    }

    /// The taskbar button flashes and the sound plays that a bell asked for, at most once a second.
    fn alert(&mut self, effects: BellEffects) {
        if !effects.taskbar_flash && !effects.sound {
            return;
        }
        if effects.taskbar_flash {
            let flash = FLASHWINFO { cbSize: std::mem::size_of::<FLASHWINFO>() as u32, hwnd: self.hwnd, dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG, uCount: 0, dwTimeout: 0 };
            // SAFETY: the structure lives through the call and names our own window.
            unsafe { let _ = FlashWindowEx(&flash); }
        }
        if effects.sound {
            // SAFETY: plays the sound of the system for a plain message.
            unsafe { let _ = MessageBeep(MESSAGEBOX_STYLE(0)); }
        }
        self.last_alert = Some(Instant::now());
    }

    /// The title of the window: the name of the application, with the count of the tabs that wait for the user while the
    /// window is not active (the taskbar button and Alt+Tab show it).
    fn update_window_title(&mut self) {
        let tabs: Vec<logic::TabAttention> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let status = tab.status();
                logic::TabAttention { state: status.state, unread: status.attention.is_some(), selected: i == self.selected, waiting: logic::is_urgent(status.state) }
            })
            .collect();
        let title = logic::window_title("Nekominal", logic::waiting_count(&tabs), self.active);
        if title != self.window_title {
            // SAFETY: names our own window.
            unsafe { let _ = SetWindowTextW(self.hwnd, &HSTRING::from(title.as_str())); }
            self.window_title = title;
        }
    }

    // --- mouse --------------------------------------------------------------------------------------------------

    fn point(&self, x: i32, y: i32) -> (f32, f32) {
        (x as f32 / self.scale, y as f32 / self.scale)
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Hit {
        let (x, y) = self.point(x, y);
        if self.plate_rect.is_some_and(|plate| plate.contains(x, y)) {
            return Hit::Plate;
        }
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
        if self.popup_keeps_owner_active() { return; }
        let hit = self.hit_test(x, y);
        if hit != self.hover {
            self.hover = hit;
            self.redraw();
        }
        if let Some(drag) = self.drag.as_mut() {
            let pointer = x as f32 / self.scale;
            drag.x = pointer;
            drag.active |= (pointer - drag.start).abs() > DRAG_START;
            // Out of the bar, up, down or aside, the chip does not stay in it: it follows the pointer as a window.
            let (height, width) = (y as f32 / self.scale, self.layout.size.0);
            let outside = drag.active && (height < -TEAR_MARGIN || height > layout::TITLE_HEIGHT + TEAR_MARGIN || pointer < 0.0 || pointer > width);
            let held = (drag.tab, drag.grab * self.scale);
            if outside && self.detach.is_none() {
                self.detach = Some(Detach { tab: held.0, pointer: self.screen_point(x as f32, y as f32), grab: held.1 });
            }
            self.redraw();
            return;
        }
        if self.page.is_some() {
            self.page_hover(x, y);
            return;
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
        // Leaving the client area for a caption button arrives after the button was entered: the hover is the button's.
        if self.hover != Hit::None && !matches!(self.hover, Hit::Minimize | Hit::Maximize | Hit::Close) {
            self.hover = Hit::None;
            self.redraw();
        }
        self.page_leave();
        self.refresh_hover_link(0, 0, None);
    }

    /// A mouse button changed. The result says whether the window should take or give back the mouse capture.
    pub fn on_button(&mut self, x: i32, y: i32, button: MouseButton, down: bool, time_ms: u32) -> Capture {
        if self.popup_keeps_owner_active() {
            // Consume both halves of the outside click; it dismisses the menu without reaching the shell.
            if !down && std::mem::take(&mut self.context_release) && button == MouseButton::Right {
                return Capture::None;
            }
            if !down { self.close_popup(); }
            return Capture::None;
        }
        if self.page.is_some() && self.pressed == Hit::None && (self.page.as_ref().is_some_and(|page| page.drag.is_some()) || self.hit_test(x, y) == Hit::Pane) {
            return self.page_button(x, y, button, down);
        }
        let hit = self.hit_test(x, y);
        // A left press outside the panes, and its release, are for the chrome.
        if button == MouseButton::Left && (self.pressed != Hit::None || (down && hit != Hit::Pane)) {
            if down {
                self.pressed = hit;
                if let Hit::Chip(i) = hit {
                    let (pointer, _) = self.point(x, y);
                    let chip = self.chip_rects.get(i).copied().unwrap_or(self.layout.chips[i]);
                    self.drag = Some(ChipDrag { tab: i, grab: pointer - chip.x, start: pointer, x: pointer, active: false });
                }
                self.redraw();
                // A chip keeps the pointer: letting go outside the window moves the tab.
                return if matches!(hit, Hit::Chip(_)) { Capture::Take } else { Capture::None };
            }
            let pressed = std::mem::replace(&mut self.pressed, Hit::None);
            self.redraw();
            if let Hit::Chip(i) = pressed {
                let drag = self.drag.take().filter(|drag| drag.active);
                if let Some(drag) = drag {
                    let to = self.drag_target(&drag);
                    self.move_tab_to(drag.tab, to);
                    self.select(to);
                } else if pressed == hit {
                    self.select(i);
                }
                return Capture::Release;
            }
            if pressed == hit {
                match hit {
                    Hit::ChipClose(i) => self.request_close_tab(i),
                    Hit::NewTab => {
                        if let Err(e) = self.new_tab() {
                            util::log(&format!("new tab: {e}"));
                        }
                    }
                    Hit::Profiles => self.open_profiles(),
                    Hit::Plate => self.open_tab_list(),
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
        if down && button == MouseButton::Right && !program_has_it && self.settings.right_click == RightClick::Menu {
            self.buttons[index] = false;
            self.open_context_menu(x, y);
            self.context_release = self.popup.is_some();
            return Capture::None;
        }
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
        if self.popup_keeps_owner_active() { return; }
        if self.page.is_some() {
            self.page_wheel(delta);
            return;
        }
        if let Some(id) = self.tab().and_then(|t| t.pane_at(x, y)) {
            self.pointer(id, PointerKind::Wheel { delta: i32::from(delta), horizontal: false }, x, y, time_ms);
        }
    }

    /// The mouse capture ended: held buttons are no longer held.
    pub fn on_capture_lost(&mut self) {
        self.buttons = [false; 3];
        if let Some(page) = self.page.as_mut() {
            page.drag = None;
        }
        self.drag = None;
        self.pressed = Hit::None;
        self.redraw();
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
        if let Some(popup) = self.popup.as_mut() {
            if popup.query.is_some() {
                popup.preedit = text.unwrap_or_default();
                unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
            }
            return;
        }
        if let Some(entry) = self.tabs.get_mut(self.selected).and_then(Tab::focused_entry_mut) {
            entry.view.set_preedit(text);
        }
        self.redraw();
    }

    /// Text the input method finished: it goes to the program as typed.
    pub fn on_ime_commit(&mut self, text: &str) {
        if let Some(popup) = self.popup.as_mut() {
            popup.preedit.clear();
            for unit in text.encode_utf16() { self.popup_char(unit); }
            return;
        }
        if self.page.is_some() {
            for unit in text.encode_utf16() { self.page_text(unit); }
            return;
        }
        if let Some(entry) = self.focused() {
            entry.pane.send_text(text);
        }
    }

    /// Where the cursor cell is in the window, in pixels: left, top and height (the candidate window hangs under it).
    pub fn ime_anchor(&self) -> Option<(i32, i32, i32)> {
        if let Some(popup) = self.popup.as_ref() {
            popup.query.as_ref()?;
            let mut rect = RECT::default();
            // SAFETY: both windows are owned by this thread; convert the painted caret to the IME owner's coordinates.
            unsafe {
                GetWindowRect(popup.hwnd, &mut rect).ok()?;
                let mut point = POINT { x: rect.left + (popup.caret.0 * self.scale) as i32,
                                        y: rect.top + (popup.caret.1 * self.scale) as i32 };
                if !ScreenToClient(self.hwnd, &mut point).as_bool() { return None; }
                return Some((point.x, point.y, (popup.caret.2 * self.scale) as i32));
            }
        }
        let entry = self.focused()?;
        let (x, y, _, height) = entry.view.cursor_cell_px(&entry.pane.term);
        Some((entry.rect.x + x as i32, entry.rect.y + y as i32, height as i32))
    }

    // --- clipboard ----------------------------------------------------------------------------------------------

    fn copy_selection(&mut self) -> bool {
        self.focused().and_then(|e| e.pane.selection_text()).is_some_and(|text| clipboard::write_text(&text))
    }

    /// Puts the first line of the clipboard into the line of text that has the keyboard: the query of a pop-up, the search
    /// of the settings page or the field being edited there. False when there is no such line.
    fn paste_into_field(&mut self) -> bool {
        let in_popup = self.popup.as_ref().is_some_and(|popup| popup.query.is_some());
        if !in_popup && self.page.is_none() {
            return false;
        }
        let text = clipboard::read_text().unwrap_or_default();
        let line = text.lines().next().unwrap_or_default().to_owned();
        for unit in line.encode_utf16() {
            if in_popup {
                self.popup_char(unit);
            } else {
                self.page_text(unit);
            }
        }
        true
    }

    fn paste(&mut self) {
        if self.paste_into_field() {
            return;
        }
        if let (Some(text), Some(entry)) = (clipboard::read_text(), self.focused()) {
            let warning = logic::paste_warning(&text, entry.pane.input_modes().bracketed_paste, &self.settings.paste);
            let title = match warning {
                None => return entry.pane.paste(&text),
                Some(logic::PasteWarning::Lines(lines)) => Text::DialogPasteLines.format(self.language(), &[&lines]),
                Some(logic::PasteWarning::Bytes(bytes)) => Text::DialogPasteSize.format(self.language(), &[&logic::size_label(bytes)]),
            };
            let pane = entry.id;
            self.confirm(title, Text::EditPaste, Confirmation::Paste { tab: self.selected, pane, text });
        }
    }

    fn request_close_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index) else { return };
        let running = tab.panes.iter().filter(|p| !p.exited && p.pane.is_busy()).count() as u32;
        if self.settings.should_confirm_close(running) {
            let title = Text::DialogCloseTab.format(self.language(), &[&self.tab_label(index)]);
            self.confirm(title, Text::CommonClose, Confirmation::CloseTab(index));
        } else {
            self.close_tab(index);
        }
    }

    fn request_close_pane(&mut self, tab: usize, pane: PaneId) {
        let Some(entry) = self.tabs.get(tab).and_then(|t| t.entry(pane)) else { return };
        let running = u32::from(!entry.exited && entry.pane.is_busy());
        if self.settings.should_confirm_close(running) {
            let title = Text::DialogCloseSessions.format(self.language(), &[&running]);
            self.confirm(title, Text::CommonClose, Confirmation::ClosePane { tab, pane });
        } else {
            self.close_pane(tab, pane);
        }
    }

    /// WM_CLOSE may destroy the window only after the same check used by pane and tab close.
    pub fn request_close_window(&mut self) -> bool {
        let running = self.tabs.iter().flat_map(|t| &t.panes).filter(|p| !p.exited && p.pane.is_busy()).count() as u32;
        if !self.settings.should_confirm_close(running) {
            self.close_popup();
            return true;
        }
        if self.confirmation.is_none() {
            let title = Text::DialogCloseSessions.format(self.language(), &[&running]);
            self.confirm(title, Text::CommonClose, Confirmation::CloseWindow);
        }
        false
    }

    fn confirm(&mut self, title: String, accept: Text, action: Confirmation) {
        if self.confirmation.is_some() {
            return;
        }
        self.close_popup();
        let lang = self.language();
        let rows = vec![
            popup::Row { label: Text::CommonCancel.get(lang).into(), hint: String::new(), action: Some(popup::Action::Cancel), current: false },
            popup::Row { label: accept.get(lang).into(), hint: String::new(), action: Some(popup::Action::Confirm), current: false },
        ];
        let size = self.surface.size();
        let height = popup::DIALOG_HEADER + popup::dialog_buttons(popup::DIALOG_WIDTH).1.1;
        let anchor = self.screen_point((size.0 as f32 + popup::DIALOG_WIDTH * self.scale) / 2.0,
                                      (size.1 as f32 - height * self.scale) / 2.0 - 4.0 * self.scale);
        match popup::open(self.hwnd, &self.gpu, rows, popup::Kind::Confirm, anchor, self.scale, self.dark()) {
            Ok(mut popup) => {
                popup.heading = title;
                popup.hover = Some(0);
                self.confirmation = Some(action);
                self.text = Utf16Assembler::default();
                // SAFETY: the owner retains focus and routes input to this confirmation.
                unsafe {
                    let _ = InvalidateRect(Some(popup.hwnd), None, false);
                }
                self.popup = Some(popup);
            }
            Err(e) => util::log(&format!("confirmation: {e}")),
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
            Command::TabClose => self.request_close_tab(self.selected),
            Command::TabNext => self.select((self.selected + 1) % self.tabs.len().max(1)),
            Command::TabPrevious => self.select((self.selected + self.tabs.len().max(1) - 1) % self.tabs.len().max(1)),
            Command::PaneSplitRight => self.split(Axis::Row),
            Command::PaneSplitDown => self.split(Axis::Column),
            Command::PaneClose => {
                if let Some(id) = self.tab().map(|t| t.focused) {
                    self.request_close_pane(self.selected, id);
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
            Command::AppSettings => self.toggle_settings_page(),
            Command::AppOpenSettingsFile => self.open_settings_file(),
            Command::AppReloadSettings => self.reload_settings(),
            Command::TabProfiles => self.open_profiles(),
            Command::AppNewWindow => {
                if let Err(e) = unsafe { PostMessageW(Some(self.hwnd), crate::win::WM_NEW_WINDOW, WPARAM(0), LPARAM(0)) } {
                    util::log(&format!("new window request: {e}"));
                }
            }
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
        if self.popup_keeps_owner_active() {
            if vk_code == vk::VK_PROCESSKEY || vk_code == vk::VK_PACKET { return false; }
            if vk_code == 0x1B { self.close_popup(); } else { self.popup_key(vk_code, repeat); }
            return true;
        }
        let held = held();
        if self.page.is_some() && !held.ctrl && !held.alt && vk_code != vk::VK_PROCESSKEY && vk_code != vk::VK_PACKET && self.page_key(vk_code, repeat) {
            return true;
        }
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
        // The terminal under the page gets no keys.
        if self.page.is_some() {
            return true;
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
        if self.popup.is_some() { self.popup_char(unit); return; }
        if self.page.is_some() {
            if unit >= 0x20 || unit == 0x08 { self.page_text(unit); }
            return;
        }
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

    /// The window answers WM_GETOBJECT: a screen reader or another client of UI Automation looks at it.
    pub fn access_getobject(&mut self, wparam: WPARAM, lparam: LPARAM) -> Option<impl Into<LRESULT> + use<>> {
        // Built for every request, because the adapter does not say whether it will need it; clients ask once.
        let mut initial = access::Initial(Some(access::tree(&self.access_view())));
        self.access.as_mut()?.handle_wm_getobject(wparam, lparam, &mut initial)
    }

    /// Tells the clients that listen what changed; nothing is built when none does. The caller raises the events with no
    /// borrow of the window held.
    pub fn access_update(&mut self) -> Option<QueuedEvents> {
        let mut adapter = self.access.take()?;
        let events = adapter.update_if_active(|| access::tree(&self.access_view()));
        self.access = Some(adapter);
        events
    }

    pub fn access_focus(&mut self, focused: bool) -> Option<QueuedEvents> {
        self.access.as_mut()?.update_window_focus_state(focused)
    }

    fn access_view(&self) -> access::View {
        let lang = self.language();
        let page = self.page.as_ref().map(|page| {
            let entries = settings_page::entries(page, lang, &self.keymap);
            access::PopupView {
                kind: access::PopupKind::List,
                heading: page.query.clone(),
                rows: entries
                    .iter()
                    .map(|entry| match entry {
                        settings_page::Entry::Setting(row) => (row.label(lang).to_owned(), settings_page::value_text(row, &self.settings, &self.profiles, lang)),
                        settings_page::Entry::Key(command) => (command.label(lang), self.keymap.shortcut_text(*command)),
                    })
                    .collect(),
                focused: page.focus,
            }
        });
        let popup = self.popup.as_ref().map(|popup| access::PopupView {
            kind: match popup.kind {
                popup::Kind::List => access::PopupKind::Menu,
                popup::Kind::Confirm => access::PopupKind::Dialog,
                popup::Kind::Palette | popup::Kind::Find => access::PopupKind::List,
            },
            heading: if popup.kind == popup::Kind::Confirm { popup.heading.clone() } else { popup.query.clone().unwrap_or_default() },
            rows: popup.rows.iter().filter(|row| !row.is_separator()).map(|row| (row.label.clone(), row.hint.clone())).collect(),
            focused: popup.hover.map(|at| popup.rows[..at].iter().filter(|row| !row.is_separator()).count()),
        });
        access::View {
            window: "Nekominal".into(),
            tabs_label: Text::TabAll.get(lang).into(),
            tabs: (0..self.tabs.len())
                .map(|i| {
                    let chip = self.chip_rects.get(i).copied().or_else(|| self.layout.chips.get(i).copied()).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
                    let s = f64::from(self.scale);
                    (self.tab_label(i), i == self.selected, [f64::from(chip.x) * s, f64::from(chip.y) * s, f64::from(chip.x + chip.w) * s, f64::from(chip.y + chip.h) * s])
                })
                .collect(),
            terminal_label: Text::SettingsTerminal.get(lang).into(),
            terminal_text: self.focused().map(|entry| entry.view.snapshot().text()).unwrap_or_default(),
            popup: popup.or(page),
        }
    }

    /// Draws a frame. True when another one is due at once: a tab chip is still sliding to its place.
    pub fn paint(&mut self) -> bool {
        let size = self.surface.size();
        let Some(view) = self.surface.acquire() else {
            self.redraw();
            return false;
        };

        let highlights = self.find_highlights();
        let mut first = true;
        if self.page.is_some() {
            // The page is drawn instead of the panes: nothing of the terminal shows through it.
        } else if let Some(tab) = self.tabs.get_mut(self.selected) {
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
        let sliding = self.advance_chips();
        self.paint_chrome();
        self.paint_page();
        self.ui.end_frame(&view);
        self.surface.present();
        sliding
    }

    /// The settings page over the area of the panes.
    fn paint_page(&mut self) {
        let Some(frame) = self.page_frame() else { return };
        let Some(page) = self.page.as_ref() else { return };
        let scroll = page.scroll;
        let g = self.glass;
        let accent = col(g.accent);
        let area = self.layout.pane;
        self.ui.fill(area, Color([g.background.r, g.background.g, g.background.b, 205]));
        let schemes: Vec<&str> = themes::theme_names().collect();
        let tone_color = |tone: settings_page::Tone| match tone {
            settings_page::Tone::Text => col(g.text()),
            settings_page::Tone::Quiet => col(g.text_secondary()),
        };
        let mut clipped = false;
        for item in &frame.items {
            if item.scrolls && !clipped {
                self.ui.push_clip(frame.viewport);
                clipped = true;
            }
            let rect = frame.on_screen(item, scroll);
            if item.scrolls && (rect.y + rect.h < frame.viewport.y || rect.y > frame.viewport.y + frame.viewport.h) {
                continue;
            }
            match &item.visual {
                settings_page::Visual::None => {}
                settings_page::Visual::Line => self.ui.fill(rect, col(g.divider())),
                settings_page::Visual::Plate(fill) => match *fill {
                    settings_page::Fill::Nav { selected, hover } => {
                        if selected {
                            self.ui.rrect(rect, CORNER_RADIUS, col(g.chip_selected()), Some((col(g.chip_selected_border()), 1.0)));
                        } else if hover {
                            self.ui.rrect(rect, CORNER_RADIUS, col(g.hover()), None);
                        }
                    }
                    settings_page::Fill::Row { focus } => {
                        self.ui.rrect(rect, CORNER_RADIUS, col(g.cluster()), focus.then_some((accent, 1.5)));
                    }
                    settings_page::Fill::Button { selected, hover } => {
                        if selected {
                            self.ui.rrect(rect, CORNER_RADIUS, col(g.chip_selected()), Some((col(g.chip_selected_border()), 1.0)));
                        } else {
                            self.ui.rrect(rect, CORNER_RADIUS, if hover { col(g.hover()) } else { col(g.cluster()) }, None);
                        }
                    }
                    settings_page::Fill::Field { editing } => {
                        self.ui.rrect(rect, CORNER_RADIUS, col(g.cluster()), Some((if editing { accent } else { col(g.divider()) }, 1.0)));
                    }
                    settings_page::Fill::Switch { on, hover } => {
                        let fill = if on { accent } else if hover { col(g.hover()) } else { col(g.pressed()) };
                        self.ui.rrect(rect, CORNER_RADIUS, fill, Some((col(g.divider()), 1.0)));
                    }
                    settings_page::Fill::Track => self.ui.rrect(rect, rect.h / 2.0, col(g.divider()), None),
                    settings_page::Fill::Accent => self.ui.rrect(rect, rect.h / 2.0, accent, None),
                    settings_page::Fill::Knob => self.ui.rrect(rect, rect.w / 2.0, col(g.text()), None),
                    settings_page::Fill::Radio { on, hover } => {
                        let ring = if on { accent } else if hover { col(g.text()) } else { col(g.text_secondary()) };
                        self.ui.rrect(rect, rect.w / 2.0, Color::TRANSPARENT, Some((ring, 1.5)));
                        if on {
                            self.ui.rrect(Rect::new(rect.x + 4.0, rect.y + 4.0, rect.w - 8.0, rect.h - 8.0), (rect.w - 8.0) / 2.0, accent, None);
                        }
                    }
                },
                settings_page::Visual::Label { text, size, tone, bold, align } => {
                    let width = self.ui.measure(text, *size, *bold);
                    let line = self.ui.line_height(*size);
                    let x = match align {
                        settings_page::Align::Left => rect.x,
                        settings_page::Align::Center => rect.x + (rect.w - width) / 2.0,
                        settings_page::Align::Right => rect.x + rect.w - width,
                    };
                    self.ui.push_clip(rect);
                    self.ui.text(x.max(rect.x), rect.y + (rect.h - line) / 2.0, text, *size, tone_color(*tone), *bold);
                    self.ui.pop_clip();
                }
                settings_page::Visual::Glyph { ch, size, tone } => self.icon_centered(rect, *ch, *size, tone_color(*tone)),
                settings_page::Visual::Tile { scheme, selected, hover } => {
                    let Some(scheme) = schemes.get(*scheme).map(|name| themes::scheme(name)) else { continue };
                    let border = if *selected { (accent, 2.0) } else if *hover { (col(g.text()), 1.0) } else { (col(g.divider()), 1.0) };
                    self.ui.rrect(rect, CORNER_RADIUS, col(scheme.background), Some(border));
                    self.ui.push_clip(rect);
                    self.ui.text(rect.x + 10.0, rect.y + 8.0, scheme.name, 12.0, col(scheme.foreground), false);
                    for (i, color) in scheme.ansi[1..7].iter().enumerate() {
                        self.ui.rrect(Rect::new(rect.x + 10.0 + i as f32 * 20.0, rect.y + rect.h - 26.0, 16.0, 16.0), 4.0, col(*color), None);
                    }
                    self.ui.pop_clip();
                }
                settings_page::Visual::Caret => self.ui.fill(rect, col(g.text())),
            }
        }
        if clipped {
            self.ui.pop_clip();
        }
    }

    fn paint_chrome(&mut self) {        let g = self.glass;
        let width = self.layout.size.0;
        let tint = {
            let opacity = glass::terminal_opacity(self.settings.appearance.backdrop, self.settings.appearance.opacity, glass::Material { window_active: self.active, effects_on: true });
            let alpha = glass::bar_tint_alpha(opacity);
            Color([g.background.r, g.background.g, g.background.b, (alpha * 255.0).round() as u8])
        };
        self.ui.fill(Rect::new(0.0, 0.0, width, layout::TITLE_HEIGHT), tint);
        self.ui.fill(Rect::new(0.0, layout::TITLE_HEIGHT - 1.0, width, 1.0), col(g.divider()));

        // The caption buttons: a plate as tall as the chips in the middle of the bar, the glyph in the middle of the plate.
        let glyphs = [(Hit::Minimize, '\u{E921}'), (Hit::Maximize, if self.maximized { '\u{E923}' } else { '\u{E922}' }), (Hit::Close, '\u{E8BB}')];
        for (cell, (hit, glyph)) in self.layout.caption_buttons.into_iter().zip(glyphs) {
            let plate = Rect::new(cell.x + 4.0, (layout::TITLE_HEIGHT - layout::CHIP_HEIGHT) / 2.0, cell.w - 8.0, layout::CHIP_HEIGHT);
            let pressed = self.pressed == hit;
            let color = if self.active { col(g.text()) } else { col(g.text_secondary()) };
            // The same plate as the other buttons of the bar, in the colours of the scheme.
            if self.hover == hit || pressed {
                self.ui.rrect(plate, CORNER_RADIUS, if pressed { col(g.pressed()) } else { col(g.hover()) }, None);
            }
            // The glyphs of the icon font sit a little above the middle of their line; the nudge brings them to it.
            self.icon_centered(Rect::new(plate.x, plate.y + CAPTION_GLYPH_NUDGE, plate.w, plate.h), glyph, ICON_SIZE, color);
        }

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
        let dragged = self.drag.filter(|drag| drag.active).map(|drag| drag.tab);
        // The dragged chip last, over the others.
        let order: Vec<usize> = (0..self.layout.chips.len()).filter(|i| Some(*i) != dragged).chain(dragged).collect();
        for i in order {
            if !self.layout.is_shown(i) && Some(i) != dragged {
                continue;
            }
            let chip = self.chip_rects.get(i).copied().unwrap_or(self.layout.chips[i]);
            if Some(i) == dragged {
                self.ui.shadow(chip, CORNER_RADIUS, 12.0, Color([0, 0, 0, 90]));
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
            // What the tab asks for: its glyph in the status colour, ahead of the title.
            let status = self.tabs[i].status();
            let mut title_x = chip.x + 12.0;
            if let Some(glyph) = status::glyph(status.state) {
                let icon_line = self.ui.line_height(ICON_SIZE);
                let at = chip.x + 10.0;
                self.ui.icon(at, chip.y + (chip.h - icon_line) / 2.0, glyph, ICON_SIZE, col(status::color(g, status.state)));
                title_x = at + ICON_SIZE + 6.0;
            }
            self.ui.text(title_x, chip.y + (chip.h - line) / 2.0, &title, TEXT_SIZE, color, false);
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
        let list = self.layout.profiles;
        if self.hover == Hit::Profiles || self.popup_from == Some(Hit::Profiles) {
            self.ui.rrect(list, CORNER_RADIUS, col(g.hover()), None);
        }
        self.icon_centered(list, '\u{E70D}', 12.0, col(g.text()));
        // The position plate: where the selected tab is among all of them, which a strip of narrow chips cannot say.
        let strip = self.layout.profiles.x;
        let tier = if self.layout.size.0 < logic::MEDIUM_FROM as f32 { logic::BarTier::Narrow } else { logic::BarTier::Wide };
        if let Some(plate) = logic::plate(self.selected, self.tabs.len(), f64::from(strip), tier) {
            let text = match plate {
                logic::Plate::Position { current, total } => format!("{current} / {total}"),
                logic::Plate::Total(total) => total.to_string(),
            };
            let (x, room) = (list.x + list.w + 8.0, self.layout.menu.x - list.x - list.w - 12.0);
            let line = self.ui.line_height(TEXT_SIZE);
            let width = self.ui.measure(&text, TEXT_SIZE, false).min(room.max(0.0));
            let plate = Rect::new(x - 8.0, list.y, width + 16.0, list.h);
            self.plate_rect = Some(plate);
            if self.hover == Hit::Plate || self.popup_from == Some(Hit::Plate) {
                self.ui.rrect(plate, CORNER_RADIUS, col(g.hover()), None);
            }
            self.ui.push_clip(Rect::new(x, list.y, room.max(0.0), list.h));
            self.ui.text(x, list.y + (list.h - line) / 2.0, &text, TEXT_SIZE, col(g.text_secondary()), false);
            self.ui.pop_clip();
        } else {
            self.plate_rect = None;
        }
        let menu = self.layout.menu;
        if self.hover == Hit::Menu || self.popup_from == Some(Hit::Menu) {
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
        self.popup_from = self.popup.as_ref().map(|_| Hit::Menu);
    }

    /// The command palette: what matches `query` of the commands the shell can run and the open tabs.
    fn palette_rows(&self, query: &str) -> Vec<popup::Row> {
        let tabs: Vec<PaletteTab> = (0..self.tabs.len()).map(|i| PaletteTab { id: i as u32, title: self.tab_label(i) }).collect();
        let items = palette::items(self.language(), &self.keymap, &self.profiles, &tabs);
        palette::search(items, query)
            .into_iter()
            .filter_map(|item| {
                let action = match item.action {
                    PaletteAction::Run(command) if wired(command) => popup::Action::Command(command),
                    PaletteAction::GotoTab(index) => popup::Action::Tab(index as usize),
                    PaletteAction::NewTab(id) => popup::Action::NewTab(self.profiles.iter().position(|p| p.id == id)?),
                    PaletteAction::SetColorScheme(name) => popup::Action::ColorScheme(themes::theme_names().find(|n| *n == name)?),
                    PaletteAction::OpenSetting(id) => popup::Action::ShowSetting(settings_ui::find(&id)?.id),
                    PaletteAction::Run(_) => return None,
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
        self.apply_look();
        self.save_settings();
        self.redraw();
    }

    /// Puts the color scheme, the backdrop and the opacity of the settings in effect in the window and its panes.
    fn apply_look(&mut self) {
        let (backdrop, opacity) = (self.settings.appearance.backdrop, self.settings.appearance.opacity);
        let scheme = themes::scheme(&self.settings.theme);
        self.glass = Glass::from_scheme(scheme);
        let shown = glass::terminal_opacity(backdrop, opacity, glass::Material::NORMAL);
        self.theme = glass::render_theme(scheme, shown);
        for entry in self.tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()) {
            entry.view.set_theme(self.theme.clone());
        }
        crate::win::set_backdrop(self.hwnd, backdrop, self.dark());
    }

    fn save_settings(&self) {
        if let Some(path) = store::settings_path()
            && let Err(e) = store::save_keeping_original(&path, &settings::serialize(&self.settings))
        {
            util::log(&format!("settings: {e}"));
        }
    }

    // --- the settings page --------------------------------------------------------------------------------------

    /// The rectangles of the page for the state it is in now.
    fn page_frame(&mut self) -> Option<settings_page::Frame> {
        let lang = self.language();
        let page = self.page.as_ref()?;
        let ui = &mut self.ui;
        let view = settings_page::View { lang, settings: &self.settings, keymap: &self.keymap, profiles: &self.profiles, area: self.layout.pane };
        Some(settings_page::build(page, &view, &mut |text, size, bold| ui.measure(text, size, bold)))
    }

    fn toggle_settings_page(&mut self) {
        if self.page.is_some() {
            self.close_page();
        } else {
            self.open_settings_page(None);
        }
    }

    /// Opens the page at the section of the row `focus`, with that row under the keyboard.
    fn open_settings_page(&mut self, focus: Option<&str>) {
        self.close_popup();
        let mut page = self.page.take().unwrap_or_default();
        page.query.clear();
        page.editing = None;
        page.scroll = 0.0;
        page.focus = None;
        let row = focus.and_then(settings_ui::find);
        if let Some(row) = row {
            page.section = Section::ALL.iter().position(|s| *s == row.section).unwrap_or(0);
        }
        self.page = Some(page);
        if let Some(row) = row
            && let Some(frame) = self.page_frame()
            && let Some(at) = frame.entries.iter().position(|entry| matches!(entry, settings_page::Entry::Setting(r) if r.id == row.id))
            && let Some(page) = self.page.as_mut()
        {
            page.focus = Some(at);
            page.scroll = frame.reveal(at, 0.0).min(frame.max_scroll());
        }
        self.redraw();
    }

    fn close_page(&mut self) {
        if self.page.take().is_some() {
            // The panes were not drawn while the page was open.
            self.relayout();
            self.redraw();
        }
    }

    fn set_setting(&mut self, row: &'static Descriptor, value: Value) {
        let before = self.settings.clone();
        (row.set)(&mut self.settings, value);
        self.apply_settings(&before);
    }

    /// Moves the setting with this id one step on (or back), a large step with `large`.
    fn step_setting(&mut self, id: &str, forward: bool, large: bool) {
        let Some(row) = settings_ui::find(id) else { return };
        if let Some(value) = settings_page::stepped(row, &self.settings, &self.profiles, forward, large) {
            self.set_setting(row, value);
        }
    }

    /// The text field of a row stops being edited; what was typed becomes the value.
    fn page_commit_edit(&mut self) {
        let Some((id, text)) = self.page.as_mut().and_then(|page| page.editing.take()) else { return };
        if let Some(row) = settings_ui::find(id) {
            self.set_setting(row, Value::Text(text));
        }
        self.redraw();
    }

    /// The part of the page under a point of the window (pixels).
    fn page_part_at(&mut self, x: i32, y: i32) -> Option<Part> {
        let (x, y) = self.point(x, y);
        let scroll = self.page.as_ref()?.scroll;
        self.page_frame()?.part_at(scroll, x, y)
    }

    pub fn page_hover(&mut self, x: i32, y: i32) {
        if let Some(id) = self.page.as_ref().and_then(|page| page.drag) {
            self.drag_slider(id, x, y);
            return;
        }
        let part = self.page_part_at(x, y);
        if let Some(page) = self.page.as_mut()
            && page.hover != part
        {
            page.hover = part;
            self.redraw();
        }
    }

    pub fn page_leave(&mut self) {
        if let Some(page) = self.page.as_mut()
            && page.hover.take().is_some()
        {
            self.redraw();
        }
    }

    /// The slider with this id takes the number under the pointer.
    fn drag_slider(&mut self, id: &'static str, x: i32, y: i32) {
        let (px, _) = self.point(x, y);
        let Some(row) = settings_ui::find(id) else { return };
        let Control::Slider(range) = row.control else { return };
        let Some(scroll) = self.page.as_ref().map(|page| page.scroll) else { return };
        let Some(frame) = self.page_frame() else { return };
        let Some(area) = frame.items.iter().find(|item| item.part == Some(Part::Slider(id))) else { return };
        let track = settings_page::slider_track(frame.on_screen(area, scroll));
        self.set_setting(row, Value::Number(settings_page::slider_value(range, track, px)));
    }

    /// A press or release of the left button over the page.
    pub fn page_button(&mut self, x: i32, y: i32, button: MouseButton, down: bool) -> Capture {
        if button != MouseButton::Left {
            return Capture::None;
        }
        if !down {
            return if self.page.as_mut().and_then(|page| page.drag.take()).is_some() { Capture::Release } else { Capture::None };
        }
        let part = self.page_part_at(x, y);
        if !matches!(part, Some(Part::Field(id)) if self.page.as_ref().and_then(|p| p.editing.as_ref()).is_some_and(|(editing, _)| *editing == id)) {
            self.page_commit_edit();
        }
        let Some(part) = part else { return Capture::None };
        let mut capture = Capture::None;
        match part {
            Part::Nav(i) => {
                if let Some(page) = self.page.as_mut() {
                    (page.section, page.query, page.scroll, page.focus) = (i, String::new(), 0.0, None);
                }
            }
            Part::Search => {}
            Part::File => self.open_settings_file(),
            Part::Toggle(id) => self.step_setting(id, true, false),
            Part::Option(id, i) => {
                if let Some(row) = settings_ui::find(id) {
                    self.set_setting(row, Value::Index(i));
                }
            }
            Part::Minus(id) => self.step_setting(id, false, false),
            Part::Plus(id) => self.step_setting(id, true, false),
            Part::Slider(id) => {
                if let Some(page) = self.page.as_mut() {
                    page.drag = Some(id);
                }
                self.drag_slider(id, x, y);
                capture = Capture::Take;
            }
            Part::Field(id) => {
                if let Some(row) = settings_ui::find(id)
                    && let Value::Text(text) = (row.get)(&self.settings)
                    && let Some(page) = self.page.as_mut()
                    && page.editing.is_none()
                {
                    page.editing = Some((id, text));
                }
            }
            Part::Tile(i) => {
                if let (Some(row), Some(name)) = (settings_ui::find("theme"), themes::theme_names().nth(i)) {
                    self.set_setting(row, Value::Text(name.to_owned()));
                }
            }
            Part::Profile(i) => {
                if let (Some(row), Some(profile)) = (settings_ui::find("defaultProfile"), self.profiles.get(i)) {
                    let id = profile.id.clone();
                    self.set_setting(row, Value::Text(id));
                }
            }
            Part::Reset(id) => {
                if let Some(row) = settings_ui::find(id) {
                    let before = self.settings.clone();
                    row.reset(&mut self.settings);
                    self.apply_settings(&before);
                }
            }
        }
        self.redraw();
        capture
    }

    pub fn page_wheel(&mut self, delta: i16) {
        let Some(max) = self.page_frame().map(|frame| frame.max_scroll()) else { return };
        if let Some(page) = self.page.as_mut() {
            page.scroll = (page.scroll - f32::from(delta) / 120.0 * 3.0 * 52.0).clamp(0.0, max);
            self.redraw();
        }
    }

    /// A key while the page is open; `false` when the page does not use it.
    fn page_key(&mut self, vk_code: i32, repeat: bool) -> bool {
        const VK_TAB: i32 = 0x09;
        const VK_RETURN: i32 = 0x0D;
        const VK_ESCAPE: i32 = 0x1B;
        const VK_PRIOR: i32 = 0x21;
        const VK_NEXT: i32 = 0x22;
        const VK_LEFT: i32 = 0x25;
        const VK_UP: i32 = 0x26;
        const VK_RIGHT: i32 = 0x27;
        const VK_DOWN: i32 = 0x28;
        const VK_DELETE: i32 = 0x2E;
        let shift = held().shift;
        if self.page.as_ref().is_some_and(|page| page.editing.is_some()) {
            match vk_code {
                VK_RETURN if !repeat => self.page_commit_edit(),
                VK_ESCAPE => {
                    if let Some(page) = self.page.as_mut() {
                        page.editing = None;
                    }
                    self.redraw();
                }
                _ => {}
            }
            return true;
        }
        let Some(frame) = self.page_frame() else { return false };
        let Some(page) = self.page.as_ref() else { return false };
        let focus = page.focus;
        let focused_row = focus.and_then(|i| frame.entries.get(i)).and_then(|entry| match entry {
            settings_page::Entry::Setting(row) => Some(*row),
            settings_page::Entry::Key(_) => None,
        });
        match vk_code {
            VK_ESCAPE => {
                if page.query.is_empty() {
                    self.close_page();
                } else if let Some(page) = self.page.as_mut() {
                    (page.query, page.scroll, page.focus) = (String::new(), 0.0, None);
                }
            }
            VK_UP | VK_DOWN => {
                let last = frame.entries.len().saturating_sub(1);
                let next = match (focus, vk_code == VK_DOWN) {
                    (None, true) => 0,
                    (None, false) => last,
                    (Some(i), true) => (i + 1).min(last),
                    (Some(i), false) => i.saturating_sub(1),
                };
                if let Some(page) = self.page.as_mut() {
                    page.focus = Some(next);
                    page.scroll = frame.reveal(next, page.scroll).clamp(0.0, frame.max_scroll());
                }
            }
            VK_LEFT | VK_RIGHT => {
                let Some(row) = focused_row else { return true };
                self.step_setting(row.id, vk_code == VK_RIGHT, shift);
            }
            VK_RETURN if !repeat => {
                let Some(row) = focused_row else { return true };
                if matches!(row.control, Control::Text) {
                    if let (Value::Text(text), Some(page)) = ((row.get)(&self.settings), self.page.as_mut()) {
                        page.editing = Some((row.id, text));
                    }
                } else {
                    self.step_setting(row.id, true, false);
                }
            }
            VK_PRIOR | VK_NEXT => {
                let max = frame.max_scroll();
                if let Some(page) = self.page.as_mut() {
                    let page_height = frame.viewport.h * 0.9;
                    page.scroll = (page.scroll + if vk_code == VK_NEXT { page_height } else { -page_height }).clamp(0.0, max);
                }
            }
            VK_TAB if page.query.is_empty() => {
                let count = Section::ALL.len();
                if let Some(page) = self.page.as_mut() {
                    page.section = (page.section + if shift { count - 1 } else { 1 }) % count;
                    (page.scroll, page.focus) = (0.0, None);
                }
            }
            VK_DELETE => {
                let Some(row) = focused_row else { return false };
                let before = self.settings.clone();
                row.reset(&mut self.settings);
                self.apply_settings(&before);
            }
            _ => return false,
        }
        self.redraw();
        true
    }

    /// Text typed while the page is open goes into the field being edited, or into the search.
    fn page_text(&mut self, unit: u16) {
        let Some(page) = self.page.as_mut() else { return };
        let changed = match page.editing.as_mut() {
            Some((_, text)) => popup::edit_query(text, &mut self.text, unit),
            None => {
                let changed = popup::edit_query(&mut page.query, &mut self.text, unit);
                if changed {
                    (page.scroll, page.focus) = (0.0, None);
                }
                changed
            }
        };
        if changed {
            self.redraw();
        }
    }

    fn open_settings_file(&mut self) {
        let Some(path) = store::settings_path() else { return };
        if !path.exists() {
            self.save_settings();
        }
        if let Err(e) = open::shell_open(&path.to_string_lossy()) {
            util::log(&format!("open settings: {e}"));
        }
    }

    /// Reads the settings file again and puts what changed in effect. Nothing is written back.
    fn reload_settings(&mut self) {
        let text = store::settings_path().and_then(|path| store::load(&path).ok().flatten());
        let loaded = settings::parse(text.as_deref()).settings;
        let before = std::mem::replace(&mut self.settings, loaded);
        self.keymap = keymap::build_keymap(commands::default_bindings(), &self.settings.keybindings).keymap;
        self.apply_changes(&before);
        self.redraw();
    }

    /// Puts what differs from `before` in effect and keeps the settings in the file.
    fn apply_settings(&mut self, before: &Settings) {
        if self.settings == *before {
            return;
        }
        self.apply_changes(before);
        self.save_settings();
        self.redraw();
    }

    /// Puts what differs from `before` in effect.
    fn apply_changes(&mut self, before: &Settings) {
        if self.settings.theme != before.theme || self.settings.appearance != before.appearance {
            self.apply_look();
        }
        let effects = logic::effects(before, &self.settings);
        if effects.font {
            for entry in self.tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()) {
                if let Err(e) = entry.view.set_font_size(self.settings.font.size as f32).and_then(|()| entry.view.set_line_height(self.settings.font.line_height as f32)) {
                    util::log(&format!("font: {e}"));
                }
            }
            self.relayout();
        }
        for entry in self.tabs.iter().flat_map(|tab| &tab.panes) {
            if effects.osc52 {
                entry.pane.set_osc52(logic::osc52_for(self.settings.osc52));
            }
            if effects.cursor {
                entry.pane.set_cursor_style(logic::cursor_style_for(&self.settings.cursor));
            }
            if effects.scrollback {
                entry.pane.set_scrollback_lines(self.settings.scrollback as usize);
            }
        }
        if self.settings.default_profile != before.default_profile
            && let Some(profile) = profiles::resolve_default(&self.profiles, &self.settings.default_profile)
        {
            self.profile = profile.clone();
        }
    }

    /// The profiles a tab can open, one row each.
    fn open_profiles(&mut self) {
        let rows = self
            .profiles
            .iter()
            .enumerate()
            .map(|(i, profile)| popup::Row {
                label: profile.name.clone(),
                hint: String::new(),
                action: Some(popup::Action::NewTab(i)),
                current: profile.id == self.profile.id,
            })
            .collect();
        let anchor = self.button_anchor(self.layout.profiles);
        self.open_popup(rows, popup::Kind::List, anchor);
        self.popup_from = self.popup.as_ref().map(|_| Hit::Profiles);
    }

    /// A tab with the profile at `index`; the tabs and panes made later still use the default profile.
    fn new_tab_with(&mut self, index: usize) {
        let Some(profile) = self.profiles.get(index).cloned() else { return };
        let default = std::mem::replace(&mut self.profile, profile);
        if let Err(e) = self.new_tab() {
            util::log(&format!("new tab: {e}"));
        }
        self.profile = default;
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

    /// Output arrived while the find bar is open: the matches are looked for again, so that the count and the marks keep
    /// up with the screen. The match the bar was on stays, or the last one when it was on the last; the pane does not
    /// scroll.
    fn find_refresh(&mut self) {
        let Some(query) = self.popup.as_ref().filter(|popup| popup.kind == popup::Kind::Find).and_then(|popup| popup.query.clone()) else { return };
        if query.is_empty() {
            return;
        }
        let matches = match self.focused() {
            Some(entry) => entry.pane.term.lock().search_all(&find::literal_pattern(&query), FIND_LIMIT).unwrap_or_default(),
            None => Vec::new(),
        };
        let Some(old) = self.find.as_ref() else { return };
        let last = matches.len().saturating_sub(1);
        let current = if old.current + 1 >= old.matches.len() { last } else { old.current.min(last) };
        let status = find::status(&query, current, matches.len());
        self.find = Some(FindState { matches, current });
        if let Some(popup) = self.popup.as_mut() {
            popup.status = status;
            // SAFETY: invalidating our own window.
            unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
        }
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
        let Some(query) = self.popup.as_mut().and_then(|p| p.query.as_mut()) else { return };
        if !popup::edit_query(query, &mut self.text, unit) { return; }
        let query = query.clone();
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

    /// The menu of the right button (the setting `rightClick` is `menu`), with its left edge at the pointer.
    fn open_context_menu(&mut self, x: i32, y: i32) {
        let lang = self.language();
        let has_selection = self.focused().is_some_and(|entry| entry.pane.selection_text().is_some());
        let rows: Vec<popup::Row> = [Command::EditCopy, Command::EditPaste, Command::EditSelectAll]
            .into_iter()
            .filter(|command| *command != Command::EditCopy || has_selection)
            .map(|command| popup::Row { label: command.label(lang), hint: self.keymap.shortcut_text(command), action: Some(popup::Action::Command(command)), current: false })
            .collect();
        let anchor = self.screen_point(x as f32 + popup::WIDTH * self.scale, y as f32);
        self.open_popup(rows, popup::Kind::List, anchor);
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
        let anchor = self.button_anchor(self.plate_rect.unwrap_or(self.layout.profiles));
        self.open_popup(rows, popup::Kind::List, anchor);
        self.popup_from = self.popup.as_ref().map(|_| Hit::Plate);
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
                self.text = Utf16Assembler::default();
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
        self.popup_from = None;
        self.confirmation = None;
        self.text = Utf16Assembler::default();
        self.find = None;
        self.popup_closed = Some(Instant::now());
        // SAFETY: destroys a window of this thread; the swap chain goes with the `Popup`.
        unsafe { let _ = DestroyWindow(popup.hwnd); }
        drop(popup);
        self.redraw();
    }

    pub fn popup_keeps_owner_active(&self) -> bool {
        self.popup.is_some()
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

    pub fn popup_key(&mut self, vk_code: i32, repeat: bool) {
        const VK_RETURN: i32 = 0x0D;
        const VK_UP: i32 = 0x26;
        const VK_DOWN: i32 = 0x28;
        // Ctrl+V into the line of a palette or a find bar.
        if vk_code == 0x56 && held().ctrl && !repeat && self.popup.as_ref().is_some_and(|popup| popup.query.is_some()) {
            self.paste_into_field();
            return;
        }
        let Some(popup) = self.popup.as_mut() else { return };
        if popup.kind == popup::Kind::Confirm {
            if repeat {
                return;
            }
            match vk_code {
                0x09 | 0x25 | 0x27 => {
                    popup.hover = popup::step(&popup.rows, popup.hover, vk_code != 0x25 && !(vk_code == 0x09 && held().shift));
                    unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
                }
                VK_RETURN | 0x20 => {
                    if let Some(row) = popup.hover { self.choose(row); }
                }
                _ => {}
            }
            return;
        }
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
        let confirmation = if action == Some(popup::Action::Confirm) { self.confirmation.take() } else { None };
        self.close_popup();
        // An explicit menu choice may open another popup; only outside dismissal needs the click debounce.
        self.popup_closed = None;
        match action {
            Some(popup::Action::Confirm) => match confirmation {
                Some(Confirmation::Paste { tab, pane, text }) => {
                    if let Some(entry) = self.tabs.get(tab).and_then(|t| t.entry(pane)) {
                        if !entry.exited { entry.pane.paste(&text); }
                    }
                }
                Some(Confirmation::ClosePane { tab, pane }) => self.close_pane(tab, pane),
                Some(Confirmation::CloseTab(tab)) => self.close_tab(tab),
                Some(Confirmation::CloseWindow) => unsafe { let _ = DestroyWindow(self.hwnd); },
                None => {}
            },
            Some(popup::Action::Cancel) => {},
            Some(popup::Action::Command(command)) => self.run(command),
            Some(popup::Action::Tab(index)) => self.select(index),
            Some(popup::Action::Backdrop(backdrop)) => self.set_appearance(backdrop, self.settings.appearance.opacity),
            Some(popup::Action::ShowSetting(id)) => self.open_settings_page(Some(id)),
            Some(popup::Action::NewTab(index)) => self.new_tab_with(index),
            Some(popup::Action::ColorScheme(name)) => {
                let before = self.settings.clone();
                self.settings.theme = name.to_owned();
                self.apply_settings(&before);
            }
            None => {}
        }
    }

    /// The place a chip let go of now takes in the order of the tabs.
    fn drag_target(&self, drag: &ChipDrag) -> usize {
        let Some(first) = self.layout.chips.first() else { return drag.tab };
        let widths: Vec<f64> = self.layout.chips.iter().map(|chip| f64::from(chip.w)).collect();
        let centre = drag.x - drag.grab + self.layout.chips[drag.tab].w / 2.0;
        logic::reorder_target(&widths, f64::from(layout::CHIP_GAP), drag.tab, f64::from(centre - first.x))
    }

    /// Puts tab `from` at the place `to`; the selection stays with the tab it was on.
    fn move_tab_to(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.selected = if self.selected == from {
            to
        } else if from < self.selected && self.selected <= to {
            self.selected - 1
        } else if to <= self.selected && self.selected < from {
            self.selected + 1
        } else {
            self.selected
        };
        self.relayout();
        self.redraw();
    }

    /// Moves the chips a frame of their slide toward their places: those the layout gives, with the dragged chip
    /// following the pointer and the others making way for it. True while a chip is not in its place yet.
    fn advance_chips(&mut self) -> bool {
        let now = Instant::now();
        let seconds = self.chip_clock.map_or(0.0, |at| now.duration_since(at).as_secs_f32());
        // A window that changes its size (a drag of its edge) has the chips follow at once.
        let size = self.surface.size();
        let snap = size != self.chip_size;
        let active = self.drag.filter(|drag| drag.active);
        let to = active.map(|drag| self.drag_target(&drag));
        let mut moving = false;
        let mut view = HashMap::new();
        let mut rects = Vec::with_capacity(self.tabs.len());
        for (i, tab) in self.tabs.iter().enumerate() {
            let slot = match (active, to) {
                (Some(drag), Some(to)) => motion::slot_of(i, drag.tab, to),
                _ => i,
            };
            let Some(target) = self.layout.chips.get(slot).copied() else {
                rects.push(Rect::new(0.0, 0.0, 0.0, 0.0));
                continue;
            };
            let shown = match active {
                Some(drag) if drag.tab == i => {
                    let left = (drag.x - drag.grab).clamp(layout::STRIP_LEFT, (self.layout.menu.x - target.w).max(layout::STRIP_LEFT));
                    Rect::new(left, target.y, target.w, target.h)
                }
                _ => {
                    // A new tab grows from nothing in its place.
                    let from = self.chip_view.get(&tab.uid).copied().unwrap_or(Rect::new(target.x, target.y, 0.0, target.h));
                    let next = if snap { target } else { motion::approach(from, target, seconds) };
                    moving |= !motion::at_rest(next, target);
                    next
                }
            };
            view.insert(tab.uid, shown);
            rects.push(shown);
        }
        moving |= active.is_some();
        self.chip_view = view;
        self.chip_rects = rects;
        self.chip_size = size;
        self.chip_clock = moving.then_some(now);
        moving
    }

    /// What tab `i` shows: what the title format of the settings makes of the focused pane.
    fn tab_label(&self, i: usize) -> String {
        let lang = self.language();
        let untitled = Text::TabUntitled.get(lang);
        let Some(entry) = self.tabs[i].focused_entry() else { return untitled.to_owned() };
        let parts = logic::TitleParts {
            program: entry.title.clone(),
            cwd: entry.cwd.clone(),
            profile: entry.profile.clone(),
            shell: entry.shell.clone(),
            home: std::env::var("USERPROFILE").ok(),
            untitled: untitled.to_owned(),
            ..Default::default()
        };
        logic::default_title(&parts, &self.settings.tabs.title.parts, false).0
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
        if popup.kind == popup::Kind::Confirm {
            popup::paint_confirmation(ui, &popup.heading, &popup.rows, popup.hover, popup.size.0, g);
            ui.end_frame(&view);
            popup.surface.present();
            return;
        }
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
            let status_width = ui.measure(&popup.status, TEXT_SIZE, false);
            let end = popup.size.0 - 16.0 - if popup.status.is_empty() { 0.0 } else { status_width + 12.0 };
            let width = ui.measure(query, TEXT_SIZE, false);
            let composing = ui.measure(&popup.preedit, TEXT_SIZE, false);
            let scroll = (width + composing + 4.0 - (end - popup::QUERY_LEFT)).max(0.0);
            let start = popup::QUERY_LEFT - scroll;
            ui.push_clip(Rect::new(popup::QUERY_LEFT, y, (end - popup::QUERY_LEFT).max(0.0), line));
            ui.text(start, y, query, TEXT_SIZE, col(g.text()), false);
            ui.text(start + width, y, &popup.preedit, TEXT_SIZE, col(g.text()), false);
            if composing > 0.0 {
                ui.fill(Rect::new(start + width, y + line - 1.0, composing, 1.0), col(g.accent));
            }
            let caret = start + width + composing + 1.0;
            popup.caret = (caret, y, line);
            // The caret: a bar after the text, not blinking, so the window needs no timer.
            ui.fill(Rect::new(caret, y, 1.5, line), col(g.text()));
            ui.pop_clip();
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
