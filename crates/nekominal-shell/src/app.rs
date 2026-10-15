//! The application of one window: tabs with panes, the chrome drawn around them, and the input.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nekominal_input::{
    KeyEvent, KeyInput, Mods, MouseButton, PointerEvent, PointerGrid, PointerKind, Utf16Assembler, vk,
};
use nekominal_model::{
    commands::{self, Command},
    keymap::{self, Keymap, Modifiers},
    layout::{Axis, PaneId},
    profiles::{self, DiscoveredProfile, Profile},
    settings::{self, Backdrop, Settings},
    store,
    themes::{self, Color as SchemeColor},
};
use nekominal_pane::{Pane, PaneRenderer, UiEvent};
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_render as r;
use nekominal_term::alacritty_terminal::grid::Scroll;
use nekominal_ui::{Color, Rect, Ui};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    Graphics::Gdi::InvalidateRect,
    UI::{
        Input::KeyboardAndMouse::GetKeyState,
        WindowsAndMessaging::{PostMessageW, WM_APP, WM_CLOSE},
    },
};

use crate::{
    clipboard,
    glass::{self, Glass},
    gpu::{self, Gpu, Surface},
    layout::{self, Hit, Layout},
    logic::{self, PxRect},
    open,
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
    text: Utf16Assembler,
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
        let ui = Ui::new(&gpu.device, &gpu.queue, gpu::FORMAT)?;
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
            layout: Layout::new(1.0, 1.0, 0, 0.0),
            caption: 138.0,
            hover: Hit::None,
            pressed: Hit::None,
            buttons: [false; 3],
            pointer_pane: None,
            hand: false,
            over_splitter: None,
            dragged_splitter: None,
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
        let view = PaneRenderer::new_sharing(&self.gpu.device, &self.gpu.queue, gpu::FORMAT, self.renderer_config(), self.theme.clone(), sibling)?;
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
        self.active = active;
        self.sync_focus();
        self.redraw();
    }

    /// Chips, panes and the cells each shown pane has.
    fn relayout(&mut self) {
        let (w, h) = self.surface.size();
        self.layout = Layout::new(w as f32 / self.scale, h as f32 / self.scale, self.tabs.len(), self.caption);
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
        let Some(frame) = self.surface.acquire(&self.gpu) else {
            self.redraw();
            return;
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        let mut first = true;
        if let Some(tab) = self.tabs.get_mut(self.selected) {
            for id in tab.shown() {
                let Some(entry) = tab.entry_mut(id) else { continue };
                entry.view.prepare(&entry.pane.term);
                let rect = [entry.rect.x.max(0) as u32, entry.rect.y.max(0) as u32, entry.rect.w.max(1) as u32, entry.rect.h.max(1) as u32];
                entry.view.draw_in(&mut encoder, &view, rect, first);
                first = false;
            }
        }
        if first {
            // Nothing to draw: the target still has to be cleared to transparent.
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            }));
        }

        self.ui.begin_frame(size, self.scale);
        self.paint_chrome();
        self.ui.end_frame(&mut encoder, &view);
        self.gpu.queue.submit([encoder.finish()]);
        self.surface.present(&self.gpu, frame);
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
                self.tabs[i].title().to_owned()
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
    }

    fn icon_centered(&mut self, rect: Rect, ch: char, size: f32, color: Color) {
        let line = self.ui.line_height(size);
        let width = self.ui.measure_icon(ch, size);
        self.ui.icon(rect.x + (rect.w - width) / 2.0, rect.y + (rect.h - line) / 2.0, ch, size, color);
    }
}
