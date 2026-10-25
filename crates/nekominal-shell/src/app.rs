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
    keybindings,
    palette::{self, PaletteAction, PaletteTab},
    keymap::{self, Keymap, Modifiers},
    layout::{Axis, PaneId},
    menu::{APP_MENU, MenuRow},
    profiles::{self, DiscoveredProfile, Profile},
    settings::{self, Backdrop, RightClick, Settings},
    session::{self, PaneRecord, PixelRect, Placement, Session, TabRecord},
    settings_ui::{self, Control, Descriptor, Section, Value},
    strings::{Lang, Text},
    store,
    themes,
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
            SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_SHOWSOUNDS, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOWMAXIMIZED, SetWindowTextW, GetWindowLongPtrW, GetWindowPlacement, GetWindowRect, HWND_TOP, PostMessageW,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPlacement,
            SetWindowPos, WINDOWPLACEMENT, WM_APP, WM_CLOSE, WM_SYSCOMMAND, WS_OVERLAPPEDWINDOW,
        },
    },
}};

use crate::{
    access, agent, clipboard, design, find, icons, motion, textfield, welcome,
    settings_page::{self, Page, Part},
    settings_window::{self, SettingsWin},
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

/// Corner radius of every rounded rectangle, in DIP: the one of `design.rs`.
pub const CORNER_RADIUS: f32 = design::RADIUS;
/// How opaque the settings page is over the glass (of 255): the selected tab above it takes the same.
const PAGE_ALPHA: u8 = 205;
const TEXT_SIZE: f32 = 13.0;
const ICON_SIZE: f32 = 10.0;
/// How far an icon moves down so that its glyph, not its line of text, is what sits in the middle, in DIP: the glyphs of the
/// icon font are drawn above the middle of their line.
const ICON_NUDGE: f32 = 1.5;
/// The icon of the program a tab runs, in DIP.
const TAB_ICON: f32 = 16.0;
/// The size of the text of the right of the bar, of the second line of a row of the sidebar, the space between the parts
/// of the right of the bar and the length of the meter of the context, in DIP.
const PIECE_SIZE: f32 = 12.0;
const SMALL_SIZE: f32 = 11.5;
const PIECE_GAP: f32 = 14.0;
const METER: f32 = 32.0;
/// A tab of the strip narrower than this shows its number instead of its title, and one narrower than `ICON_FROM` leaves the
/// icon of its program out, in DIP.
const ORDINAL_BELOW: f32 = 64.0;
const ICON_FROM: f32 = 132.0;

fn piece_width(ui: &mut Ui, piece: &Piece) -> f32 {
    match piece {
        Piece::State(_, text) => 12.0 + ui.measure(text, PIECE_SIZE, false),
        Piece::Meter(_) => METER + 6.0 + ui.measure("100%", PIECE_SIZE, false),
        Piece::Glyph(_, text) => ICON_SIZE + 6.0 + ui.measure(text, PIECE_SIZE, false),
        Piece::Text(text) => ui.measure(text, PIECE_SIZE, false),
    }
}

/// Whether Windows shows animations (Settings › Accessibility › Visual effects): with them off, the marks of the tabs stand
/// still.
fn animations_on() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
    let mut on = windows::core::BOOL(1);
    // SAFETY: the query writes one BOOL into the variable it is given.
    let asked = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(std::ptr::from_mut(&mut on).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    asked.is_err() || on.as_bool()
}

/// `text` in lines no wider than `width`: broken between words, or anywhere in a run without spaces (Chinese).
fn wrap(ui: &mut Ui, text: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for ch in text.chars() {
        line.push(ch);
        if ui.measure(&line, size, false) <= width {
            continue;
        }
        line.pop();
        // A word that does not fit goes to the next line whole.
        let rest = match line.rfind(' ').filter(|_| ch != ' ') {
            Some(at) => { let rest = line[at + 1..].to_owned(); line.truncate(at); rest }
            None => String::new(),
        };
        lines.push(std::mem::take(&mut line));
        line = rest;
        if ch != ' ' {
            line.push(ch);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The word for what a tab is doing, none for a tab at rest.
fn state_text(state: logic::TabState) -> Option<Text> {
    match state {
        logic::TabState::Working => Some(Text::AgentWorking),
        logic::TabState::Blocked => Some(Text::AgentBlocked),
        logic::TabState::Done => Some(Text::AgentDone),
        logic::TabState::Error => Some(Text::AgentError),
        logic::TabState::Idle | logic::TabState::Attention => None,
    }
}

/// One part of what the right of the title bar says about the focused pane: a dot in a colour before its text, a meter,
/// a glyph before its text, or text alone.
enum Piece {
    State(Color, String),
    Meter(f32),
    Glyph(char, String),
    Text(String),
}

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
    /// Every profile a tab can open, the default among them, and the program file of each, for its icon.
    profiles: Vec<Profile>,
    programs: Vec<Option<std::path::PathBuf>>,
    /// What the settings page remembers while its window is open, and that window.
    page: Option<Page>,
    settings_win: Option<SettingsWin>,
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
    /// Where the count of the tabs that wait is drawn, when there is one: it goes to the next of them.
    waiting_rect: Option<Rect>,
    /// The room the right of the title bar takes (the waiting count and the numbers of the agent), and the tabs of the
    /// sidebar by group, as the layout was last made with them.
    right_width: f32,
    groups: Vec<Vec<usize>>,
    /// The first start: the window shows the choice of the layout of the tabs over everything until it is made.
    welcome: Option<welcome::Welcome>,
    /// The marks of the states move (Windows shows animations), the frame being drawn has one that moves, and the time
    /// they count from.
    motion: bool,
    animating: bool,
    marks_epoch: Instant,
    /// Where the window was last, on the screen: a move takes the pop-up along by the difference.
    origin: Option<(i32, i32)>,
    /// Claude Code as the Agent page last found it.
    claude: agent::Claude,
    /// The settings file could be read when it was last loaded: a file that could not is never written over. And when
    /// it was last written, by this window or another, as this window last saw it.
    settings_readable: bool,
    settings_stamp: Option<std::time::SystemTime>,
    /// Where the window was when the session was saved, for the window code to put it back.
    placement: Option<Placement>,
    /// Keys (by scan code) that went down as a shortcut of the shell: the program does not get their release either.
    shortcut_keys: Vec<u16>,
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
    /// When the find bar last searched again for new output.
    find_at: Option<Instant>,
    /// The window the menu of the window was last opened on: the one its commands go to.
    window_menu_for: HWND,
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
            | Command::ViewSidebar
            | Command::AppUpdate
            | Command::AppearanceBackdropNext
            | Command::AppearanceOpacityUp
            | Command::AppearanceOpacityDown
    )
}

use crate::widgets::{self, col, State};

/// The settings of the file, and whether it could be read: missing or empty is fine (the defaults), but text that is not
/// JSON, or not UTF-8, is a file the user is in the middle of editing, which a save must not replace with the defaults.
fn load_settings() -> (Settings, bool) {
    let loaded = store::settings_path().map(|path| store::load(&path)).transpose();
    let (text, readable) = match loaded {
        Ok(text) => (text.flatten(), true),
        Err(e) => {
            util::log(&format!("settings: {e}"));
            (None, false)
        }
    };
    let parsed = settings::parse(text.as_deref());
    for problem in &parsed.problems {
        util::log(&format!("settings: {} {:?}", problem.key, problem.expected));
    }
    let whole_file = parsed.problems.iter().any(|problem| problem.key.is_empty());
    (parsed.settings, readable && !whole_file)
}

/// When the settings file was last written.
fn settings_stamp() -> Option<std::time::SystemTime> {
    std::fs::metadata(store::settings_path()?).ok()?.modified().ok()
}

/// Draws the icon of the program file at `path` in `square` (DIP), at the scale of the window being drawn; false when the
/// file has none, and the caller draws its glyph.
fn program_icon(ui: &mut Ui, path: &std::path::Path, square: Rect) -> bool {
    ui.image(icons::key(path), square.x, square.y, square.w, |px| icons::load(path, px))
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

/// Whether the key just pressed typed a character: WM_CHAR, which TranslateMessage put in the queue before the key came here.
fn char_follows(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_NOREMOVE, PeekMessageW, WM_CHAR};
    let mut msg = MSG::default();
    // SAFETY: looks at the queue of our own window without taking anything from it.
    unsafe { PeekMessageW(&mut msg, Some(hwnd), WM_CHAR, WM_CHAR, PM_NOREMOVE).as_bool() && msg.wParam.0 >= 0x20 }
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
        let (settings, settings_readable) = load_settings();
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

        let first_window = shared_gpu.is_none();
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
            programs: all.iter().map(|p| nekominal_pty::profiles::resolve_program(&p.program)).collect(),
            profiles: all,
            page: None,
            settings_win: None,
            last_alert: None,
            context_release: false,
            popup_from: None,
            chip_view: HashMap::new(),
            chip_rects: Vec::new(),
            chip_clock: None,
            chip_size: (0, 0),
            drag: None,
            plate_rect: None,
            waiting_rect: None,
            right_width: 0.0,
            groups: Vec::new(),
            welcome: None,
            motion: true,
            animating: false,
            marks_epoch: Instant::now(),
            origin: None,
            claude: agent::claude(),
            settings_readable,
            settings_stamp: settings_stamp(),
            placement: None,
            shortcut_keys: Vec::new(),
            window_title: String::new(),
            tabs: Vec::new(),
            selected: 0,
            layout: Layout::new(1.0, 1.0, 0, 0.0, 0, layout::Mode::TOP, 0.0, &[]),
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
            find_at: None,
            window_menu_for: hwnd,
            popup_closed: None,
            text: Utf16Assembler::default(),
            access: Some(Adapter::new(hwnd, true, access::NoActions)),
        };
        if first_window && !app.settings.welcomed {
            app.welcome = Some(welcome::Welcome::default());
        }
        app.relayout();
        // Only the window the process starts with picks up the saved session; a window opened later starts empty.
        let first_window = first_window && app.settings.restore_session;
        if first_tab && !(first_window && app.restore_session()) {
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

    /// The frame last drawn has a mark that moves: the window keeps a timer for the next frames while it does.
    pub fn animating(&self) -> bool { self.animating }

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
        let left = (self.mode().side_width() * self.scale).round() as i32;
        PxRect { x: left, y: top, w: (w as i32 - left).max(1), h: (h as i32 - top).max(1) }
    }

    /// Where the tabs are shown, as the settings say.
    fn mode(&self) -> layout::Mode {
        layout::Mode { layout: self.settings.tabs.layout, sidebar_open: self.settings.tabs.sidebar_open }
    }

    /// The tabs in the order the sidebar lists them, by group: the user's own order in one group, or, when the list is
    /// grouped by state, those that wait for the user, those that work and the others.
    fn sidebar_groups(&self) -> Vec<Vec<usize>> {
        if !self.settings.tabs.group_by_state || self.settings.tabs.layout != settings::TabLayout::List {
            return vec![(0..self.tabs.len()).collect()];
        }
        let mut groups = vec![Vec::new(), Vec::new(), Vec::new()];
        for (i, tab) in self.tabs.iter().enumerate() {
            let state = tab.status().state;
            let group = if logic::is_urgent(state) { 0 } else if state == logic::TabState::Working { 1 } else { 2 };
            groups[group].push(i);
        }
        groups
    }

    /// Makes the layout again from the size of the window, the settings, the room the right of the bar takes and the
    /// groups of the sidebar. The panes do not move with it: only the width of the sidebar moves them, and `relayout` places
    /// them.
    fn rebuild_layout(&mut self) {
        let (w, h) = self.surface.size();
        self.layout = Layout::new(w as f32 / self.scale, h as f32 / self.scale, self.tabs.len(), layout::CAPTION, self.selected, self.mode(), self.right_width, &self.groups);
    }

    /// Pixels between the top of the window and the top of the panes.
    fn pane_top(&self) -> f32 {
        (layout::TITLE_HEIGHT * self.scale).round()
    }

    // --- tabs and panes -----------------------------------------------------------------------------------------

    fn spawn_config(&self, profile: &Profile, directory: Option<&str>, session: &str, cols: u16, rows: u16) -> SpawnConfig {
        let mut env = nekominal_pty::profiles::default_environment();
        if self.settings.agent_environment {
            env.extend(nekominal_pty::profiles::agent_environment());
        }
        // Which pane a program runs in, for the status line command of Claude Code, and the settings that make that command
        // this program, while the status bar is on and Claude Code has no status line of the user's own.
        env.insert(agent::SESSION_VAR.to_owned(), session.to_owned());
        if self.settings.status_bar.show && !self.claude.own_status_line && let Some(file) = agent::claude_settings_file() {
            env.insert(agent::CLAUDE_SETTINGS_VAR.to_owned(), file.to_string_lossy().into_owned());
        }
        let program = nekominal_pty::profiles::resolve_program(&profile.program)
            .unwrap_or_else(|| profile.program.clone().into());
        SpawnConfig {
            program,
            args: profiles::launch_args(profile, self.settings.shell_integration),
            cwd: directory.or(profile.cwd.as_deref()).map(std::path::PathBuf::from).filter(|dir| dir.is_dir()),
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
        self.make_entry_with(id, size_px, &self.profile, None)
    }

    /// A pane that runs profile in directory (the directory of the profile when there is none).
    fn make_entry_with(&self, id: PaneId, size_px: (u32, u32), profile: &Profile, directory: Option<&str>) -> Result<PaneEntry, String> {
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
        let session = agent::new_session();
        let config = self.spawn_config(profile, directory, &session, cols as u16, rows as u16);
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
            profile: profile.name.clone(),
            profile_id: profile.id.clone(),
            shell: std::path::Path::new(&profile.program).file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default(),
            exited: false,
            job: None,
            job_path: None,
            agent_seen: false,
            attention: None,
            note: None,
            progress: None,
            wake: target,
            session,
            usage: None,
            usage_stamp: None,
            branch: None,
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
        self.forget_tab_positions();
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

    /// A tab is about to go: what holds the position of a tab (a chip held by the pointer, a pressed chip, the list of the
    /// tabs) would point at another tab, or past the last one, once the tabs after it move up.
    fn forget_tab_positions(&mut self) {
        self.drag = None;
        self.pressed = Hit::None;
        if self.popup.as_ref().is_some_and(|popup| popup.kind == popup::Kind::List) {
            self.close_popup();
        }
    }

    /// Takes a tab out of this window with its sessions running, for another window to adopt. The window closes
    /// when it was the last tab.
    pub fn take_tab(&mut self, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        self.forget_tab_positions();
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
        !self.mode().layout.is_side() && !matches!(self.hit_test(x, y), Hit::Pane | Hit::Waiting | Hit::None | Hit::Minimize | Hit::Maximize | Hit::Close)
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

    /// The window moved (dragged by its title bar, snapped, moved with the keyboard): the pop-up that is open moves with it,
    /// by as much, so that a menu stays hanging from its button and the find bar over its pane.
    pub fn on_move(&mut self) {
        let mut rect = RECT::default();
        // SAFETY: plain queries and a move of windows of this thread.
        unsafe {
            if GetWindowRect(self.hwnd, &mut rect).is_err() {
                return;
            }
            let origin = (rect.left, rect.top);
            if let (Some(before), Some(popup)) = (self.origin, self.popup.as_ref())
                && before != origin
            {
                let mut at = RECT::default();
                if GetWindowRect(popup.hwnd, &mut at).is_ok() {
                    let (dx, dy) = (origin.0 - before.0, origin.1 - before.1);
                    let _ = SetWindowPos(popup.hwnd, None, at.left + dx, at.top + dy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            self.origin = Some(origin);
        }
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
        // Another window may have changed the settings while this one was in the background: take them before this one
        // changes and writes its own copy.
        if active && settings_stamp() != self.settings_stamp {
            self.reload_settings();
        }
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
        self.groups = self.sidebar_groups();
        self.rebuild_layout();
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
        // The find bar searches the focused pane: only its output can change the matches.
        let focused_woke = self.focused().is_some_and(|entry| entry.wake.pending.load(Ordering::Acquire));
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
        // shortcut: at most four searches a second while output streams, so the count can lag the last burst until the next
        // output; a timer would settle it if that ever matters.
        if focused_woke && self.find_at.is_none_or(|at| at.elapsed() >= Duration::from_millis(250)) {
            self.find_at = Some(Instant::now());
            self.find_refresh();
        }
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
        if self.waiting_rect.is_some_and(|rect| rect.contains(x, y)) {
            return Hit::Waiting;
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
        if self.welcome.is_some() {
            let (x, y) = self.point(x, y);
            self.welcome_hover(x, y);
            return;
        }
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
        self.refresh_hover_link(0, 0, None);
    }

    /// A mouse button changed. The result says whether the window should take or give back the mouse capture.
    pub fn on_button(&mut self, x: i32, y: i32, button: MouseButton, down: bool, time_ms: u32) -> Capture {
        if self.popup_keeps_owner_active() {
            if down && self.popup.as_ref().is_some_and(|popup| popup.kind == popup::Kind::Find) {
                // A click outside the find bar leaves it open, with its marks: the keyboard goes back to the terminal.
                if let Some(popup) = self.popup.as_mut() {
                    popup.has_keys = false;
                    // SAFETY: invalidating our own window.
                    unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
                }
            } else {
                // Consume both halves of the outside click; it dismisses the menu without reaching the shell.
                if !down && std::mem::take(&mut self.context_release) && button == MouseButton::Right {
                    return Capture::None;
                }
                if !down { self.close_popup(); }
                return Capture::None;
            }
        }
        if self.welcome.is_some() {
            if button == MouseButton::Left && !down {
                let (x, y) = self.point(x, y);
                self.welcome_click(x, y);
            }
            return Capture::None;
        }
        let hit = self.hit_test(x, y);
        // A left press outside the panes, and its release, are for the chrome.
        if button == MouseButton::Left && (self.pressed != Hit::None || (down && hit != Hit::Pane)) {
            if down {
                self.pressed = hit;
                // shortcut: only the strip on top reorders by dragging; the rows of the sidebar are clicked, not dragged.
                if let Hit::Chip(i) = hit
                    && !self.mode().layout.is_side()
                {
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
                    Hit::Waiting => self.run(Command::TabNextNeedsResponse),
                    Hit::Sidebar => self.run(Command::ViewSidebar),
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
        if let Some(id) = self.tab().and_then(|t| t.pane_at(x, y)) {
            self.pointer(id, PointerKind::Wheel { delta: i32::from(delta), horizontal: false }, x, y, time_ms);
        }
    }

    /// The mouse capture ended: held buttons are no longer held.
    pub fn on_capture_lost(&mut self) {
        self.buttons = [false; 3];
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
    /// Looks at the processes of the panes the tabs show: the program a tab runs is what its icon shows.
    pub fn on_job_tick(&mut self) {
        let mut changed = false;
        for tab in &mut self.tabs {
            let Some(entry) = tab.focused_entry_mut() else { continue };
            let Some(job) = entry.pane.job() else { continue };
            if job.name.to_ascii_lowercase().starts_with("claude") {
                entry.agent_seen = true;
            }
            // The shell is alone again: whatever agent ran in the pane has ended, and what it last said is history.
            if job.root && (entry.agent_seen || entry.usage.is_some()) {
                agent::forget(&entry.session);
                (entry.agent_seen, entry.usage, entry.usage_stamp) = (false, None, None);
                changed = true;
            }
            if Some(&job.name) != entry.job.as_ref() {
                entry.job = Some(job.name);
                entry.job_path = job.path;
                changed = true;
            }
        }
        changed |= self.refresh_usage();
        if changed {
            self.redraw();
        }
    }

    /// Reads again what the agent of the focused pane said about its session, when it said something new, and the branch
    /// of the folder the pane is in. True when the status bar has to be drawn again.
    fn refresh_usage(&mut self) -> bool {
        let Some(entry) = self.tabs.get_mut(self.selected).and_then(|tab| tab.focused_entry_mut()) else { return false };
        let stamp = agent::usage_stamp(&entry.session);
        let mut changed = false;
        if stamp != entry.usage_stamp {
            entry.usage_stamp = stamp;
            let usage = stamp.and_then(|_| agent::read_usage(&entry.session));
            changed |= usage != entry.usage;
            entry.usage = usage;
        }
        let folder = entry.usage.as_ref().and_then(|u| u.cwd.clone()).or_else(|| entry.cwd.clone());
        let branch = folder.as_deref().and_then(|dir| agent::git_branch(std::path::Path::new(dir)));
        if branch != entry.branch {
            entry.branch = branch;
            changed = true;
        }
        changed
    }

    pub fn on_drag_tick(&mut self) {
        let Some(id) = self.pointer_pane else { return };
        if self.tab().and_then(|t| t.entry(id)).is_some_and(|e| e.pane.drag_scroll()) {
            self.redraw();
        }
    }

    // --- input method -------------------------------------------------------------------------------------------

    /// The text being composed, drawn at the cursor; `None` when the composition ended.
    pub fn on_ime_preedit(&mut self, text: Option<String>) {
        // Only a pop-up with a line to type in takes the composition; under a menu it stays the terminal's.
        if self.popup_keeps_owner_active() && let Some(popup) = self.popup.as_mut().filter(|popup| popup.query.is_some()) {
            popup.preedit = text.unwrap_or_default();
            unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
            return;
        }
        if let Some(entry) = self.tabs.get_mut(self.selected).and_then(Tab::focused_entry_mut) {
            entry.view.set_preedit(text);
        }
        self.redraw();
    }

    /// Text the input method finished: it goes to the program as typed.
    pub fn on_ime_commit(&mut self, text: &str) {
        if self.popup_keeps_owner_active() && let Some(popup) = self.popup.as_mut().filter(|popup| popup.query.is_some()) {
            popup.preedit.clear();
            for unit in text.encode_utf16() { self.popup_char(unit); }
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

    /// Puts the first line of the clipboard into the query of a pop-up. False when there is no such line.
    fn paste_into_field(&mut self) -> bool {
        if !self.popup.as_ref().is_some_and(|popup| popup.query.is_some()) {
            return false;
        }
        let text = clipboard::read_text().unwrap_or_default();
        let line = text.lines().next().unwrap_or_default().to_owned();
        for unit in line.encode_utf16() {
            self.popup_char(unit);
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

    /// Scoop does not replace a program that runs, and the update would be typed in this very terminal: a PowerShell window
    /// of its own waits for this process and its OpenConsole to end, runs the update and starts the app again, while every
    /// window here closes and saves the session.
    fn update_and_restart(&mut self) {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        // The program is ...\scoop\apps\nekominal\<version or current>\Nekominal.exe.
        let app_dir = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.parent()?.to_owned()));
        let Some(app_dir) = app_dir.filter(|dir| dir.parent().and_then(|apps| apps.file_name()).is_some_and(|name| name.eq_ignore_ascii_case("apps"))) else {
            util::log("update: this copy was not installed with Scoop, so there is nothing to update");
            return;
        };
        let quoted = |path: &std::path::Path| path.display().to_string().replace('\'', "''");
        let script = format!(
            "$Host.UI.RawUI.WindowTitle = 'Nekominal update'; Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
             Get-Process OpenConsole -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -like '{dir}\\*' }} | Wait-Process; \
             scoop update nekominal; Start-Process '{exe}'",
            pid = std::process::id(),
            dir = quoted(&app_dir),
            exe = quoted(&app_dir.join("current").join("Nekominal.exe")),
        );
        match std::process::Command::new("powershell.exe").args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script]).creation_flags(CREATE_NEW_CONSOLE).spawn() {
            Ok(_) => crate::win::close_all_windows(),
            Err(e) => util::log(&format!("update: {e}")),
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
            popup::Row { label: Text::CommonCancel.get(lang).into(), hint: String::new(), action: Some(popup::Action::Cancel), current: false, mark: None, program: None },
            popup::Row { label: accept.get(lang).into(), hint: String::new(), action: Some(popup::Action::Confirm), current: false, mark: None, program: None },
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
            Command::TabNextNeedsResponse => {
                let waiting: Vec<bool> = self.tabs.iter().map(|tab| logic::is_urgent(tab.status().state)).collect();
                if let Some(next) = logic::next_waiting(&waiting, self.selected) {
                    self.select(next);
                }
            }
            Command::AppUpdate => self.update_and_restart(),
            Command::ViewSidebar => {
                let before = self.settings.clone();
                self.settings.tabs.sidebar_open = !self.settings.tabs.sidebar_open;
                self.apply_settings(&before);
            }
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
            Command::AppSettings => self.open_settings_page(None),
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
        if let Some(name) = nekominal_input::physical_key(scan, extended).map(|key| key.name())
            && let Some(command) = self.keymap.find(held, name)
            && !(command == Command::EditSmartCopy && self.focused().is_none_or(|e| e.pane.selection_text().is_none()))
        {
            if !repeat {
                self.run(command);
                self.shortcut_keys.push(scan);
            }
            return true;
        }
        if vk_code == vk::VK_PROCESSKEY || vk_code == vk::VK_PACKET {
            return false;
        }
        // AltGr is Ctrl+Alt to Windows. When the chord types a character (AltGr+Q is @ on a German layout), that character is
        // already in the queue as WM_CHAR and is what the program gets; the chord is no key of its own.
        if held.ctrl && held.alt && char_follows(self.hwnd) {
            return false;
        }
        let mods = input_mods(held);
        let Some(key) = vk::key_for_vk_scan(vk_code, scan, extended, mods) else { return false };
        let Some(entry) = self.focused() else { return false };
        let event = if repeat { KeyEvent::Repeat } else { KeyEvent::Press };
        entry.pane.send_key(&KeyInput::new(key).with_mods(mods).with_event(event));
        true
    }

    /// A key came up. A program that asked for key releases (the kitty keyboard protocol) is told; the encoder says
    /// nothing for the others.
    pub fn on_key_up(&mut self, vk_code: i32, scan: u16, extended: bool) {
        if let Some(at) = self.shortcut_keys.iter().position(|pressed| *pressed == scan) {
            self.shortcut_keys.remove(at);
            return;
        }
        if self.popup.is_some() || vk_code == vk::VK_PROCESSKEY || vk_code == vk::VK_PACKET {
            return;
        }
        let mods = input_mods(held());
        let Some(key) = vk::key_for_vk_scan(vk_code, scan, extended, mods) else { return };
        if let Some(entry) = self.focused() {
            entry.pane.send_key(&KeyInput::new(key).with_mods(mods).with_event(KeyEvent::Release));
        }
    }

    /// One UTF-16 unit of text from the keyboard or the IME.
    pub fn on_char(&mut self, unit: u16) {
        if self.popup_keeps_owner_active() { self.popup_char(unit); return; }
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

    /// The window of the settings answers WM_GETOBJECT: a screen reader looks at it.
    pub fn settings_access_getobject(&mut self, wparam: WPARAM, lparam: LPARAM) -> Option<impl Into<LRESULT> + use<>> {
        let mut initial = access::Initial(Some(access::page_tree(&self.page_access_view()?)));
        self.settings_win.as_mut()?.access.as_mut()?.handle_wm_getobject(wparam, lparam, &mut initial)
    }

    /// Tells the clients that listen to the window of the settings what changed; the caller raises the events with no
    /// borrow of the application held.
    pub fn settings_access_update(&mut self) -> Option<QueuedEvents> {
        let mut adapter = self.settings_win.as_mut()?.access.take()?;
        let view = self.page_access_view();
        let events = view.and_then(|view| adapter.update_if_active(|| access::page_tree(&view)));
        if let Some(win) = self.settings_win.as_mut() {
            win.access = Some(adapter);
        }
        events
    }

    pub fn settings_access_focus(&mut self, focused: bool) -> Option<QueuedEvents> {
        self.settings_win.as_mut()?.access.as_mut()?.update_window_focus_state(focused)
    }

    fn page_access_view(&mut self) -> Option<access::PageView> {
        let scale = f64::from(self.settings_win.as_ref()?.scale);
        let frame = self.page_frame()?;
        let (lang, page) = (self.language(), self.page.as_ref()?);
        let searching = !page.query.trim().is_empty();
        let tab_bounds = |i: usize| {
            let rect = frame.items.iter().find(|item| item.part == Some(Part::Nav(i))).map_or(Rect::new(0.0, 0.0, 0.0, 0.0), |item| item.rect);
            [f64::from(rect.x) * scale, f64::from(rect.y) * scale, f64::from(rect.x + rect.w) * scale, f64::from(rect.y + rect.h) * scale]
        };
        Some(access::PageView {
            window: Text::AppSettings.get(lang).to_owned(),
            sections_label: Text::AppSettings.get(lang).to_owned(),
            sections: Section::ALL.iter().enumerate().map(|(i, section)| (section.text().get(lang).to_owned(), !searching && i == page.section, tab_bounds(i))).collect(),
            search_label: Text::SettingsSearch.get(lang).to_owned(),
            query: page.query.clone(),
            rows: settings_page::entries(page, lang, &self.keymap)
                .iter()
                .map(|entry| match entry {
                    settings_page::Entry::Setting(row) => (row.label(lang).to_owned(), settings_page::value_text(row, &self.settings, &self.profiles, lang)),
                    settings_page::Entry::Key(command) => (command.label(lang), self.keymap.shortcut_text(*command)),
                    settings_page::Entry::Claude => ("Claude Code".to_owned(), settings_page::claude_note(&self.claude, &self.settings).get(lang).to_owned()),
                })
                .collect(),
            focused: page.focus,
        })
    }

    fn access_view(&self) -> access::View {
        let lang = self.language();
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
            popup,
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
        self.motion = animations_on();
        self.animating = false;
        let sliding = self.advance_chips();
        let relaid = self.paint_chrome();
        self.paint_welcome();
        self.ui.end_frame(&view);
        self.surface.present();
        sliding || relaid
    }

    /// Draws a frame of the window of the settings.
    pub fn settings_paint(&mut self) {
        let Some(win) = self.settings_win.as_mut() else { return };
        let Some(view) = win.surface.acquire() else { return };
        let (size, scale) = (win.surface.size(), win.scale);
        self.gpu.clear(&view, [0.0; 4]);
        self.ui.begin_frame(size, scale);
        self.paint_page();
        self.ui.end_frame(&view);
        if let Some(win) = self.settings_win.as_mut() {
            win.surface.present();
        }
    }

    /// The settings page over the whole window.
    fn paint_page(&mut self) {
        let Some(frame) = self.page_frame() else { return };
        let Some(page) = self.page.as_ref() else { return };
        let scroll = page.scroll;
        let g = self.glass;
        let accent = col(g.accent);
        let area = self.page_area();
        let Some((window_width, hover, pressed, maximized)) = self.settings_win.as_ref().map(|win| (win.area().0, win.hover, win.pressed, win.maximized)) else { return };
        self.ui.fill(Rect::new(0.0, 0.0, area.w, area.y + area.h), Color([g.background.r, g.background.g, g.background.b, PAGE_ALPHA]));
        // The title row: the band of the tabs goes on above them, the title at the left and the caption buttons at the
        // right, with the plate of the bar under the one the pointer is on.
        self.ui.fill(Rect::new(0.0, 0.0, window_width, settings_window::TITLE), col(g.tab_rest()));
        let title_line = self.ui.line_height(TEXT_SIZE);
        let title = Text::AppSettings.get(self.language());
        self.ui.text(design::INSET, (settings_window::TITLE - title_line) / 2.0, title, TEXT_SIZE, col(g.text_secondary()), false);
        let glyphs = [(Hit::Minimize, '\u{E921}'), (Hit::Maximize, if maximized { '\u{E923}' } else { '\u{E922}' }), (Hit::Close, '\u{E8BB}')];
        for (cell, (button, glyph)) in settings_window::caption_buttons(window_width).into_iter().zip(glyphs) {
            widgets::bar_button(&mut self.ui, &g, cell, State { hover: hover == button, pressed: pressed == button, chosen: false });
            self.icon_centered(cell, glyph, ICON_SIZE, col(g.text()));
        }
        self.ui.fill(Rect::new(0.0, settings_window::TITLE - 1.0, window_width, 1.0), col(g.divider()));
        let soft = { let h = col(g.hover()); Color([h.0[0], h.0[1], h.0[2], h.0[3] / 2]) };
        let background = Color([g.background.r, g.background.g, g.background.b, 255]);
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
                settings_page::Visual::Plate(fill) => match *fill {
                    settings_page::Fill::Strip => {
                        self.ui.fill(rect, col(g.tab_rest()));
                        self.ui.fill(Rect::new(rect.x + rect.w - 1.0, rect.y, 1.0, rect.h), col(g.divider()));
                    }
                    settings_page::Fill::Tab { selected, hover } => {
                        if selected {
                            widgets::plate(&mut self.ui, rect, col(g.hover()));
                        } else if hover {
                            widgets::plate(&mut self.ui, rect, soft);
                        }
                    }
                    settings_page::Fill::Row { focus } => {
                        if focus {
                            widgets::plate(&mut self.ui, rect, soft);
                        }
                        self.ui.fill(Rect::new(rect.x, rect.y, rect.w, 1.0), col(g.divider()));
                    }
                    settings_page::Fill::Button { selected, hover } => widgets::button(&mut self.ui, &g, rect, State::chosen(selected, hover)),
                    settings_page::Fill::Field { editing } => widgets::field(&mut self.ui, &g, rect, editing),
                    settings_page::Fill::Switch { on, hover } => widgets::switch(&mut self.ui, &g, rect, on, hover),
                    settings_page::Fill::Slider { share } => widgets::slider(&mut self.ui, &g, rect, share),
                    settings_page::Fill::Radio { on, hover } => widgets::radio(&mut self.ui, &g, rect, on, hover),
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
                settings_page::Visual::Program { path, mark } => {
                    if !path.as_deref().is_some_and(|path| program_icon(&mut self.ui, path, rect)) {
                        widgets::mark(&mut self.ui, rect, *mark, col(g.text_secondary()));
                    }
                }
                settings_page::Visual::Edit { text, cursor, placeholder } => {
                    widgets::line_edit(&mut self.ui, &g, rect, TEXT_SIZE, &widgets::LineEdit { text, cursor: *cursor, preedit: "", placeholder });
                }
                settings_page::Visual::Layout { layout, selected, hover } => {
                    let style = settings::TabLayout::ALL.get(*layout).copied().unwrap_or(settings::TabLayout::Top);
                    let border = if *selected { (accent, 2.0) } else if *hover { (col(g.text_secondary()), 1.0) } else { (col(g.divider()), 1.0) };
                    self.paint_miniature(Rect::new(rect.x, rect.y, rect.w, rect.h), style, background);
                    self.ui.rrect(rect, 6.0, Color::TRANSPARENT, Some(border));
                }
                settings_page::Visual::Tile { scheme, selected, hover } => {
                    let Some(scheme) = schemes.get(*scheme).map(|name| themes::scheme(name)) else { continue };
                    let border = if *selected { (accent, 2.0) } else if *hover { (col(g.text()), 1.0) } else { (col(g.divider()), 1.0) };
                    self.ui.rrect(rect, CORNER_RADIUS, col(scheme.background), Some(border));
                    self.ui.push_clip(rect);
                    self.ui.text(rect.x + 10.0, rect.y + 8.0, scheme.name, 12.0, col(scheme.foreground), false);
                    for (i, color) in scheme.ansi[1..7].iter().enumerate() {
                        self.ui.rrect(Rect::new(rect.x + 10.0 + i as f32 * 20.0, rect.y + rect.h - 26.0, 16.0, 16.0), CORNER_RADIUS, col(*color), None);
                    }
                    self.ui.pop_clip();
                }
            }
        }
        if clipped {
            self.ui.pop_clip();
        }
    }

    /// A button of the bar lit: a square plate as tall as the bar, above its lower edge.
    fn bar_plate(&mut self, button: Rect, state: State) {
        let g = self.glass;
        widgets::bar_button(&mut self.ui, &g, Rect::new(button.x, 0.0, button.w, layout::TITLE_HEIGHT - 1.0), state);
    }

    /// A button of the bar: its plate while the pointer is on it or its pop-up hangs from it, and its glyph, quiet unless
    /// it is on or under the pointer.
    fn bar_icon(&mut self, rect: Rect, hit: Hit, glyph: char, on: bool) {
        let g = self.glass;
        let lit = self.hover == hit || self.popup_from == Some(hit);
        if lit {
            self.bar_plate(rect, State::hover(true));
        }
        let color = if on || lit { col(g.text()) } else { col(g.text_secondary()) };
        self.icon_centered(rect, glyph, 12.0, color);
    }

    /// The mark of what a tab is doing in its colour, from `x` and centred on `y`: a shape for each state, so that the colour
    /// is not all that tells them apart. A tab at work has a spinner and one that waits for the user pulses, unless Windows
    /// is set to show no animations. False for a tab at rest, which has none.
    fn state_dot(&mut self, x: f32, y: f32, state: logic::TabState) -> bool {
        let Some(glyph) = status::glyph(state) else { return false };
        let g = self.glass;
        let color = col(status::color(g, state));
        let seconds = self.marks_epoch.elapsed().as_secs_f32();
        let with_alpha = |share: f32| Color([color.0[0], color.0[1], color.0[2], (f32::from(color.0[3]) * share).round() as u8]);
        match state {
            logic::TabState::Working if self.motion => {
                self.animating = true;
                let (cx, radius, dot) = (x + ICON_SIZE / 2.0, ICON_SIZE / 2.0 - 1.0, 2.2);
                for n in 0..motion::SPINNER_DOTS {
                    let (dx, dy) = motion::spinner_place(n);
                    let at = Rect::new(cx + dx * radius - dot / 2.0, y + dy * radius - dot / 2.0, dot, dot);
                    self.ui.rrect(at, dot / 2.0, with_alpha(motion::spinner_dot(n, seconds)), None);
                }
            }
            logic::TabState::Blocked if self.motion => {
                self.animating = true;
                let line = self.ui.line_height(ICON_SIZE);
                self.ui.icon(x, y - line / 2.0 + ICON_NUDGE, glyph, ICON_SIZE, with_alpha(motion::pulse(seconds)));
            }
            _ => {
                let line = self.ui.line_height(ICON_SIZE);
                self.ui.icon(x, y - line / 2.0 + ICON_NUDGE, glyph, ICON_SIZE, color);
            }
        }
        true
    }

    /// The icon of the program tab `i` runs, in `square`: the icon of its file, as the automatic icon of iTerm2 has it, or
    /// a glyph for one without.
    fn tab_icon(&mut self, i: usize, square: Rect, color: Color) {
        let path = self.tabs[i].job_path().map(std::path::Path::to_owned);
        if path.is_some_and(|path| program_icon(&mut self.ui, &path, square)) {
            return;
        }
        let glyph = self.tabs[i].job().map_or('\u{E756}', logic::job_icon);
        self.icon_centered(square, glyph, ICON_SIZE, color);
    }

    /// Where tab `i` is: the folder of its focused pane, and the branch of that folder when there is one. A title that is
    /// the folder already (the default) has the program instead of the folder repeated.
    fn place_text(&self, i: usize) -> Option<String> {
        let entry = self.tabs.get(i)?.focused_entry()?;
        let folder = entry.usage.as_ref().and_then(|u| u.cwd.clone()).or_else(|| entry.cwd.clone())?;
        let home = std::env::var("USERPROFILE").ok();
        let mut folder = logic::folder_label(&folder, home.as_deref());
        if folder == self.tab_label(i)
            && let Some(program) = self.tabs[i].job()
        {
            folder = std::path::Path::new(program).file_stem().map_or_else(|| program.to_owned(), |stem| stem.to_string_lossy().into_owned());
        }
        Some(match &entry.branch {
            Some(branch) => format!("{folder} · {branch}"),
            None => folder,
        })
    }

    /// Draws the bar, the sidebar and the dividers. True when the layout changed with what the bar says: the chips go to
    /// their new places in the frames after this one (a redraw asked for while painting would be cleared with the frame).
    fn paint_chrome(&mut self) -> bool {
        let g = self.glass;
        let lang = self.language();
        let width = self.layout.size.0;
        let bar = layout::TITLE_HEIGHT;

        // What the right of the bar says and how wide it is: the layout keeps that room free of tabs. The numbers of the
        // agent give way when the window is narrow; the count of the tabs that wait stays.
        let mut pieces = self.bar_pieces();
        let waiting = self.tabs.iter().enumerate().filter(|(i, tab)| *i != self.selected && logic::is_urgent(tab.status().state)).count();
        let badge = (waiting > 0).then(|| Text::TabsWaiting.format(lang, &[&waiting]));
        let badge_width = badge.as_ref().map_or(0.0, |text| 23.0 + self.ui.measure(text, PIECE_SIZE, false) + 10.0);
        let mut widths: Vec<f32> = pieces.iter().map(|piece| piece_width(&mut self.ui, piece)).collect();
        // A narrow window keeps its room for the tabs: the numbers go before the tabs shrink to their numbers.
        let room = ((width - 640.0).clamp(0.0, width * 0.45) - badge_width).max(0.0);
        while widths.iter().map(|w| w + PIECE_GAP).sum::<f32>() > room && widths.pop().is_some() {
            pieces.pop();
        }
        let right_width = badge_width + if badge.is_some() { 12.0 } else { 0.0 } + widths.iter().map(|w| w + PIECE_GAP).sum::<f32>();
        // The groups of the sidebar follow the states of the tabs, but not while the pointer is over the sidebar: a row does
        // not move from under it.
        let over_side = matches!(self.hover, Hit::Chip(_) | Hit::ChipClose(_) | Hit::Side);
        let groups = if over_side { self.groups.clone() } else { self.sidebar_groups() };
        let relaid = right_width != self.right_width || groups != self.groups;
        if relaid {
            (self.right_width, self.groups) = (right_width, groups);
            self.rebuild_layout();
        }

        let opacity = glass::terminal_opacity(self.settings.appearance.backdrop, self.settings.appearance.opacity, glass::Material { window_active: self.active, effects_on: true });
        let tint = {
            let alpha = glass::bar_tint_alpha(opacity);
            Color([g.background.r, g.background.g, g.background.b, (alpha * 255.0).round() as u8])
        };
        // The bar and the sidebar: one band darker than the terminal, a hairline where they meet it.
        for band in [Some(Rect::new(0.0, 0.0, width, bar)), self.layout.sidebar].into_iter().flatten() {
            self.ui.fill(band, tint);
            self.ui.fill(band, col(g.tab_rest()));
        }
        self.ui.fill(Rect::new(0.0, bar - 1.0, width, 1.0), col(g.divider()));
        if let Some(side) = self.layout.sidebar {
            self.ui.fill(Rect::new(side.x + side.w - 1.0, side.y, 1.0, side.h), col(g.divider()));
        }

        // The caption buttons: square plates as tall as the bar, the glyph in the middle of the plate.
        let glyphs = [(Hit::Minimize, '\u{E921}'), (Hit::Maximize, if self.maximized { '\u{E923}' } else { '\u{E922}' }), (Hit::Close, '\u{E8BB}')];
        for (cell, (hit, glyph)) in self.layout.caption_buttons.into_iter().zip(glyphs) {
            let pressed = self.pressed == hit;
            let color = if self.active { col(g.text()) } else { col(g.text_secondary()) };
            if self.hover == hit || pressed {
                self.bar_plate(cell, State { hover: true, pressed, chosen: false });
            }
            self.icon_centered(cell, glyph, ICON_SIZE, color);
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

        if self.mode().layout.is_side() {
            self.paint_title();
            self.paint_sidebar();
        } else {
            self.paint_strip();
        }
        if let Some(button) = self.layout.sidebar_button {
            let open = self.settings.tabs.sidebar_open;
            self.bar_icon(button, Hit::Sidebar, '\u{E8A0}', open);
        }
        let (plus, list, menu) = (self.layout.new_tab, self.layout.profiles, self.layout.menu);
        self.bar_icon(plus, Hit::NewTab, '\u{E710}', false);
        self.bar_icon(list, Hit::Profiles, '\u{E70D}', false);
        // The position plate: with more tabs than the strip shows, where the selected one is among all of them.
        self.plate_rect = None;
        if !self.mode().layout.is_side() && self.layout.shown < self.tabs.len() {
            let text = format!("{} / {}", self.selected + 1, self.tabs.len());
            let line = self.ui.line_height(TEXT_SIZE);
            let x = list.x + list.w + 8.0;
            let plate = Rect::new(x - 8.0, 0.0, self.ui.measure(&text, TEXT_SIZE, false) + 16.0, bar);
            self.plate_rect = Some(plate);
            if self.hover == Hit::Plate || self.popup_from == Some(Hit::Plate) {
                self.bar_plate(plate, State::hover(true));
            }
            self.ui.text(x, (bar - line) / 2.0, &text, TEXT_SIZE, col(g.text_secondary()), false);
        }
        self.bar_icon(menu, Hit::Menu, '\u{E712}', false);

        // The right of the bar: the count of the tabs that wait, then what the focused pane's agent is doing.
        let right = self.layout.right;
        self.waiting_rect = None;
        self.ui.push_clip(right);
        let mut x = right.x;
        if let Some(text) = &badge {
            let pill = Rect::new(x, (bar - 22.0) / 2.0, badge_width, 22.0);
            let wait = col(status::color(g, logic::TabState::Blocked));
            let alpha = if self.hover == Hit::Waiting { 64 } else { 36 };
            self.ui.rrect(pill, 11.0, Color([wait.0[0], wait.0[1], wait.0[2], alpha]), None);
            self.ui.rrect(Rect::new(pill.x + 10.0, bar / 2.0 - 3.0, 6.0, 6.0), 3.0, wait, None);
            let line = self.ui.line_height(PIECE_SIZE);
            self.ui.text(pill.x + 23.0, (bar - line) / 2.0, text, PIECE_SIZE, wait, false);
            self.waiting_rect = Some(pill);
            x += badge_width + 12.0;
        }
        for (piece, w) in pieces.iter().zip(&widths) {
            self.paint_piece(piece, x);
            x += w + PIECE_GAP;
        }
        self.ui.pop_clip();
        relaid
    }

    /// The tabs on top: words on the bar, the selected one with a line in the accent under it, the one under the pointer
    /// on a plate. A tab shows the dot of its state, the icon of its program and its title.
    fn paint_strip(&mut self) {
        let g = self.glass;
        let width = self.layout.size.0;
        let line = self.ui.line_height(TEXT_SIZE);
        // The dragged chip last, over the others.
        let dragged = self.drag.filter(|drag| drag.active).map(|drag| drag.tab);
        let order: Vec<usize> = (0..self.layout.chips.len()).filter(|i| Some(*i) != dragged).chain(dragged).collect();
        for i in order {
            if !self.layout.is_shown(i) && Some(i) != dragged {
                continue;
            }
            let chip = self.chip_rects.get(i).copied().unwrap_or(self.layout.chips[i]);
            // As tall as the bar, like the plates of its buttons.
            let plate = Rect::new(chip.x, 0.0, chip.w, layout::TITLE_HEIGHT - 1.0);
            let selected = i == self.selected;
            let hover = matches!(self.hover, Hit::Chip(n) | Hit::ChipClose(n) if n == i);
            if Some(i) == dragged {
                self.ui.push_clip(Rect::new(0.0, 0.0, width, layout::TITLE_HEIGHT));
                self.ui.shadow(plate, CORNER_RADIUS, 12.0, Color([0, 0, 0, 90]));
                self.ui.rrect(plate, CORNER_RADIUS, Color([g.background.r, g.background.g, g.background.b, 255]), None);
                self.ui.pop_clip();
            } else if hover && !selected {
                // The selected tab has its line: a plate under the pointer is for the others.
                self.bar_plate(plate, State::hover(true));
            }
            let close = Layout::close_button(chip).filter(|_| selected || hover);
            let right = close.map_or(chip.x + chip.w - 8.0, |c| c.x - design::GAP);
            let color = if selected || hover { col(g.text()) } else { col(g.text_secondary()) };
            self.ui.push_clip(Rect::new(chip.x, chip.y, (right - chip.x).max(0.0), chip.h));
            let mut x = chip.x + 8.0;
            if self.state_dot(x, chip.y + chip.h / 2.0, self.tabs[i].status().state) {
                x += ICON_SIZE + 6.0;
            }
            // A narrow tab gives up the icon of its program first, its title only when even a few letters do not fit.
            let ordinal = chip.w < ORDINAL_BELOW;
            if chip.w >= ICON_FROM {
                self.tab_icon(i, Rect::new(x, chip.y + (chip.h - TAB_ICON) / 2.0, TAB_ICON, TAB_ICON), color);
                x += TAB_ICON + 7.0;
            }
            let title = if ordinal { (i + 1).to_string() } else { self.tab_label(i) };
            self.ui.text(x, chip.y + (chip.h - line) / 2.0, &title, TEXT_SIZE, color, false);
            self.ui.pop_clip();
            if selected {
                self.ui.rrect(Rect::new(chip.x + 2.0, chip.y + chip.h - 2.0, chip.w - 4.0, 2.0), 1.0, col(g.accent), None);
            }
            if let Some(close) = close {
                if self.hover == Hit::ChipClose(i) {
                    widgets::plate(&mut self.ui, close, col(g.pressed()));
                }
                self.icon_centered(close, '\u{E711}', ICON_SIZE, col(g.text_secondary()));
            }
        }
    }

    /// With the tabs in the sidebar, the bar names the selected tab: the dot of its state, its title, and where it is.
    fn paint_title(&mut self) {
        let g = self.glass;
        let title = self.layout.title;
        if title.w <= 0.0 || self.selected >= self.tabs.len() {
            return;
        }
        let line = self.ui.line_height(TEXT_SIZE);
        self.ui.push_clip(title);
        let mut x = title.x;
        if self.state_dot(x, title.y + title.h / 2.0, self.tabs[self.selected].status().state) {
            x += ICON_SIZE + 8.0;
        }
        let label = self.tab_label(self.selected);
        self.ui.text(x, title.y + (title.h - line) / 2.0, &label, TEXT_SIZE, col(g.text()), false);
        x += self.ui.measure(&label, TEXT_SIZE, false) + 10.0;
        if let Some(place) = self.place_text(self.selected) {
            let small = self.ui.line_height(SMALL_SIZE);
            self.ui.text(x, title.y + (title.h - small) / 2.0, &place, SMALL_SIZE, col(g.text_secondary()), false);
        }
        self.ui.pop_clip();
    }

    /// The sidebar: a row a tab, in the user's order or by group. As a list a row has the icon of its program, its title
    /// and its state on a line and where it is under them; as icons, the icon of its program in a ring of the colour of its
    /// state, with its title beside it while the pointer is on it.
    fn paint_sidebar(&mut self) {
        let Some(side) = self.layout.sidebar else { return };
        let g = self.glass;
        let lang = self.language();
        let list = self.settings.tabs.layout == settings::TabLayout::List;
        let quiet = col(g.text_secondary());
        let soft = { let h = col(g.hover()); Color([h.0[0], h.0[1], h.0[2], h.0[3] / 2]) };
        self.ui.push_clip(side);
        let names = [Text::GroupWaiting, Text::AgentWorking, Text::GroupOther];
        let small = self.ui.line_height(SMALL_SIZE);
        for (rect, group) in self.layout.headings.clone() {
            let text = names.get(group).map_or("", |name| name.get(lang));
            self.ui.text(rect.x + 10.0, rect.y + rect.h - small - 4.0, text, SMALL_SIZE, quiet, false);
        }
        let line = self.ui.line_height(TEXT_SIZE);
        let mut tip = None;
        for i in 0..self.tabs.len() {
            if !self.layout.is_shown(i) {
                continue;
            }
            let row = self.chip_rects.get(i).copied().unwrap_or(self.layout.chips[i]);
            let selected = i == self.selected;
            let hover = matches!(self.hover, Hit::Chip(n) | Hit::ChipClose(n) if n == i);
            let state = self.tabs[i].status().state;
            if selected {
                widgets::plate(&mut self.ui, row, col(g.hover()));
            } else if hover {
                widgets::plate(&mut self.ui, row, soft);
            }
            if !list {
                self.tab_icon(i, Rect::new(row.x + (row.w - 18.0) / 2.0, row.y + (row.h - 18.0) / 2.0, 18.0, 18.0), quiet);
                if state != logic::TabState::Idle {
                    let ring = Rect::new(row.x - 2.0, row.y - 2.0, row.w + 4.0, row.h + 4.0);
                    let color = col(status::color(g, state));
                    // A tab that waits for the user breathes, and one at work has its spinner in the corner of its ring.
                    let alpha = if state == logic::TabState::Blocked && self.motion {
                        self.animating = true;
                        motion::pulse(self.marks_epoch.elapsed().as_secs_f32())
                    } else {
                        1.0
                    };
                    let faded = Color([color.0[0], color.0[1], color.0[2], (f32::from(color.0[3]) * alpha).round() as u8]);
                    self.ui.rrect(ring, 10.0, Color::TRANSPARENT, Some((faded, 1.5)));
                    if state == logic::TabState::Working && self.motion {
                        let badge = Rect::new(ring.x + ring.w - 8.0, ring.y - 2.0, ICON_SIZE + 2.0, ICON_SIZE + 2.0);
                        self.ui.rrect(badge, badge.w / 2.0, Color([g.background.r, g.background.g, g.background.b, 255]), None);
                        self.state_dot(badge.x + 1.0, badge.y + badge.h / 2.0, state);
                    }
                }
                if hover {
                    tip = Some((i, row));
                }
                continue;
            }
            let wait = logic::is_urgent(state);
            let name_color = if wait { col(status::color(g, state)) } else if selected || hover { col(g.text()) } else { quiet };
            let top = row.y + 7.0;
            let icon = Rect::new(row.x + 10.0, top, TAB_ICON, TAB_ICON);
            self.tab_icon(i, icon, name_color);
            // The right of the first line: the close button under the pointer, the state otherwise.
            let mut right = row.x + row.w - 10.0;
            if let Some(close) = Layout::close_button(Rect::new(row.x, top - 1.0, row.w, 18.0)).filter(|_| hover) {
                if self.hover == Hit::ChipClose(i) {
                    widgets::plate(&mut self.ui, close, col(g.pressed()));
                }
                self.icon_centered(close, '\u{E711}', ICON_SIZE, quiet);
                right = close.x - 4.0;
            } else if let Some(text) = state_text(state) {
                let text = text.get(lang);
                let w = self.ui.measure(text, SMALL_SIZE, false);
                right -= w;
                self.ui.text(right, top + (TAB_ICON - small) / 2.0, text, SMALL_SIZE, col(status::color(g, state)), false);
                right -= 6.0;
                // The states that last have their moving mark before the word: the spinner, the pulse.
                if matches!(state, logic::TabState::Working | logic::TabState::Blocked) {
                    right -= ICON_SIZE;
                    self.state_dot(right, top + TAB_ICON / 2.0, state);
                    right -= 6.0;
                }
            }
            let x = icon.x + TAB_ICON + 10.0;
            let label = self.tab_label(i);
            self.ui.push_clip(Rect::new(x, row.y, (right - x).max(0.0), row.h));
            self.ui.text(x, top + (TAB_ICON - line) / 2.0, &label, TEXT_SIZE, name_color, false);
            let panes = self.tabs[i].panes.len();
            if panes > 1 {
                // A split tab is one row: the number of its panes after its title.
                let at = x + self.ui.measure(&label, TEXT_SIZE, false) + 6.0;
                let count = panes.to_string();
                let badge = Rect::new(at, top + 1.0, self.ui.measure(&count, SMALL_SIZE, false) + 8.0, 14.0);
                self.ui.rrect(badge, 3.0, Color::TRANSPARENT, Some((col(g.divider()), 1.0)));
                self.ui.text(badge.x + 4.0, badge.y + (badge.h - small) / 2.0, &count, SMALL_SIZE, quiet, false);
            }
            self.ui.pop_clip();
            if let Some(place) = self.place_text(i) {
                self.ui.push_clip(Rect::new(x, row.y, (row.x + row.w - 10.0 - x).max(0.0), row.h));
                self.ui.text(x, row.y + row.h - 7.0 - small, &place, SMALL_SIZE, quiet, false);
                self.ui.pop_clip();
            }
        }
        self.ui.pop_clip();
        if let Some((i, row)) = tip {
            // The ring says the state by its colour alone: the tip says it in words.
            let state = state_text(self.tabs[i].status().state).map(|text| text.get(lang));
            let text = [Some(self.tab_label(i)), state.map(str::to_owned), self.place_text(i)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let line = self.ui.line_height(PIECE_SIZE);
            let rect = Rect::new(row.x + row.w + 10.0, row.y + (row.h - 26.0) / 2.0, self.ui.measure(&text, PIECE_SIZE, false) + 20.0, 26.0);
            self.ui.shadow(rect, CORNER_RADIUS, 12.0, Color([0, 0, 0, 90]));
            self.ui.rrect(rect, CORNER_RADIUS, Color([g.background.r, g.background.g, g.background.b, 250]), Some((col(g.divider()), 1.0)));
            self.ui.text(rect.x + 10.0, rect.y + (rect.h - line) / 2.0, &text, PIECE_SIZE, col(g.text()), false);
        }
    }

    // --- the first start ----------------------------------------------------------------------------------------

    /// The choice of where the tabs go, over everything below the bar, until it is made.
    fn paint_welcome(&mut self) {
        let Some(state) = self.welcome.clone() else { return };
        let g = self.glass;
        let lang = self.language();
        let (width, height) = self.layout.size;
        let geo = welcome::geometry(width, height);
        let background = Color([g.background.r, g.background.g, g.background.b, 255]);
        self.ui.fill(Rect::new(0.0, layout::TITLE_HEIGHT, width, height - layout::TITLE_HEIGHT), background);
        let (text, quiet, accent) = (col(g.text()), col(g.text_secondary()), col(g.accent));
        for (words, size, bold, y, color) in [(Text::WelcomeTitle, 22.0, true, geo.title_y, text), (Text::WelcomeLater, TEXT_SIZE, false, geo.lead_y, quiet)] {
            let words = words.get(lang);
            let w = self.ui.measure(words, size, bold);
            self.ui.text((width - w) / 2.0, y, words, size, color, bold);
        }
        let pad = welcome::CARD_PAD;
        for (card, side) in [(geo.top, false), (geo.side, true)] {
            let on = state.layout.is_side() == side;
            let hover = state.hover == Some(if side { welcome::Part::Side } else { welcome::Part::Top });
            let border = if on { (accent, 2.0) } else if hover { (quiet, 1.0) } else { (col(g.divider()), 1.0) };
            // The card is a little lighter than the page under it.
            let lift = |c: u8| c.saturating_add(10);
            self.ui.rrect(card, 10.0, Color([lift(background.0[0]), lift(background.0[1]), lift(background.0[2]), 255]), Some(border));
            let mini = Rect::new(card.x + pad, card.y + pad, card.w - 2.0 * pad, welcome::MINI_HEIGHT);
            let style = if side && state.layout == settings::TabLayout::Icons { settings::TabLayout::Icons } else if side { settings::TabLayout::List } else { settings::TabLayout::Top };
            self.paint_miniature(mini, style, background);
            let name_y = mini.y + mini.h + 14.0;
            let pick = Rect::new(card.x + pad + 2.0, name_y + 2.0, 16.0, 16.0);
            widgets::radio(&mut self.ui, &g, pick, on, hover);
            let name = if side { Text::WelcomeSide } else { Text::LayoutTop };
            let line = self.ui.line_height(14.0);
            self.ui.text(pick.x + 24.0, name_y + (20.0 - line) / 2.0, name.get(lang), 14.0, text, true);
            let note = if side { Text::WelcomeSideNote } else { Text::WelcomeTopNote };
            let small = self.ui.line_height(SMALL_SIZE + 1.0);
            for (n, line) in wrap(&mut self.ui, note.get(lang), SMALL_SIZE + 1.0, card.w - 2.0 * pad - 4.0).iter().enumerate() {
                self.ui.text(card.x + pad + 2.0, name_y + 26.0 + n as f32 * (small + 2.0), line, SMALL_SIZE + 1.0, quiet, false);
            }
        }
        // The style of the sidebar, in its card.
        let small = self.ui.line_height(PIECE_SIZE);
        let label = geo.style_label;
        self.ui.text(label.x + 2.0, label.y + (label.h - small) / 2.0, Text::WelcomeStyle.get(lang), PIECE_SIZE, quiet, false);
        for (rect, part, layout, words) in [(geo.list, welcome::Part::List, settings::TabLayout::List, Text::StyleList), (geo.icons, welcome::Part::Icons, settings::TabLayout::Icons, Text::StyleIcons)] {
            let chosen = state.layout == layout || (state.layout == settings::TabLayout::Top && layout == settings::TabLayout::List);
            widgets::button(&mut self.ui, &g, rect, State::chosen(chosen, state.hover == Some(part)));
            let words = words.get(lang);
            let w = self.ui.measure(words, PIECE_SIZE, false);
            self.ui.text(rect.x + (rect.w - w) / 2.0, rect.y + (rect.h - small) / 2.0, words, PIECE_SIZE, if chosen { text } else { quiet }, false);
        }
        let start = geo.start;
        let lift = if state.hover == Some(welcome::Part::Start) { 24 } else { 0 };
        self.ui.rrect(start, 6.0, Color([accent.0[0].saturating_add(lift), accent.0[1].saturating_add(lift), accent.0[2].saturating_add(lift), 255]), None);
        let words = Text::WelcomeStart.get(lang);
        let (w, line) = (self.ui.measure(words, TEXT_SIZE, true), self.ui.line_height(TEXT_SIZE));
        self.ui.text(start.x + (start.w - w) / 2.0, start.y + (start.h - line) / 2.0, words, TEXT_SIZE, background, true);
    }

    /// A window in a few rectangles: its bar, the tabs on it or in a sidebar, and lines of a terminal.
    fn paint_miniature(&mut self, mini: Rect, style: settings::TabLayout, background: Color) {
        let g = self.glass;
        let line = col(g.divider());
        let faint = col(g.hover());
        let colors = [logic::TabState::Blocked, logic::TabState::Working, logic::TabState::Done].map(|state| col(status::color(g, state)));
        self.ui.rrect(mini, 6.0, background, Some((line, 1.0)));
        self.ui.fill(Rect::new(mini.x, mini.y + 14.0, mini.w, 1.0), line);
        let wait = colors[0];
        self.ui.rrect(Rect::new(mini.x + mini.w - 38.0, mini.y + 4.0, 30.0, 6.0), 3.0, Color([wait.0[0], wait.0[1], wait.0[2], 128]), None);
        let side = match style {
            settings::TabLayout::Top => {
                for n in 0..3 {
                    let color = if n == 0 { col(g.accent) } else { faint };
                    self.ui.rrect(Rect::new(mini.x + 8.0 + n as f32 * 33.0, mini.y + 5.0, 26.0, 4.0), 2.0, color, None);
                }
                0.0
            }
            settings::TabLayout::List => {
                self.ui.rrect(Rect::new(mini.x + 8.0, mini.y + 5.0, 10.0, 4.0), 2.0, faint, None);
                for n in 0..4 {
                    let row = Rect::new(mini.x + 6.0, mini.y + 23.0 + n as f32 * 18.0, 84.0, 12.0);
                    self.ui.rrect(row, 3.0, faint, None);
                    self.ui.rrect(Rect::new(row.x + 4.0, row.y + 4.0, 4.0, 4.0), 2.0, colors.get(n).copied().unwrap_or(line), None);
                }
                96.0
            }
            settings::TabLayout::Icons => {
                self.ui.rrect(Rect::new(mini.x + 8.0, mini.y + 5.0, 10.0, 4.0), 2.0, faint, None);
                for n in 0..4 {
                    let cell = Rect::new(mini.x + 8.0, mini.y + 23.0 + n as f32 * 20.0, 12.0, 12.0);
                    self.ui.rrect(cell, 3.0, faint, colors.get(n).map(|c| (*c, 1.0)));
                }
                28.0
            }
        };
        if side > 0.0 {
            self.ui.fill(Rect::new(mini.x + side, mini.y + 15.0, 1.0, mini.h - 15.0), line);
        }
        for (n, share) in [0.7, 0.5, 0.8, 0.4].into_iter().enumerate() {
            let x = mini.x + side + 10.0;
            self.ui.rrect(Rect::new(x, mini.y + 25.0 + n as f32 * 10.0, (mini.w - side - 20.0) * share, 4.0), 2.0, faint, None);
        }
    }

    /// The pointer moved over the first start: what it is on lights.
    fn welcome_hover(&mut self, x: f32, y: f32) {
        let part = welcome::geometry(self.layout.size.0, self.layout.size.1).hit(x, y);
        if let Some(state) = self.welcome.as_mut()
            && state.hover != part
        {
            state.hover = part;
            self.redraw();
        }
    }

    /// A click on the first start: a card or a style picks it, the button keeps the choice and ends the screen.
    fn welcome_click(&mut self, x: f32, y: f32) {
        let Some(part) = welcome::geometry(self.layout.size.0, self.layout.size.1).hit(x, y) else { return };
        let Some(state) = self.welcome.as_mut() else { return };
        let done = state.click(part);
        let layout = state.layout;
        if done {
            self.welcome = None;
            let before = self.settings.clone();
            self.settings.tabs.layout = layout;
            self.settings.welcomed = true;
            self.apply_settings(&before);
        }
        self.redraw();
    }

    // --- what the bar says about the focused pane ---------------------------------------------------------------

    /// Whether the pane runs an agent: one that reported its state or its session, or Claude Code, seen by the name of its
    /// program, until the shell is alone again (a tool it runs, `bash.exe`, does not end it).
    fn runs_agent(entry: &PaneEntry) -> bool {
        entry.agent_seen || entry.usage.is_some() || entry.pane.agent_summary().is_some()
    }

    /// What the right of the bar says about the focused pane, as the settings of the status bar choose: what its agent is
    /// doing (its state, model, context, cost and time), or without an agent the branch of its folder.
    fn bar_pieces(&self) -> Vec<Piece> {
        let g = self.glass;
        let lang = self.language();
        let show = &self.settings.status_bar;
        let mut pieces = Vec::new();
        let Some(entry) = self.focused().filter(|_| show.show) else { return pieces };
        if !Self::runs_agent(entry) {
            if show.no_agent == settings::NoAgent::Place
                && let Some(branch) = entry.branch.clone().filter(|_| show.branch)
            {
                pieces.push(Piece::Glyph('\u{E81C}', branch));
            }
            return pieces;
        }
        if show.state
            && let Some(record) = entry.pane.agent_summary()
        {
            use nekominal_term::agent_status::AgentState;
            let state = match record.state {
                AgentState::Working => logic::TabState::Working,
                AgentState::Blocked => logic::TabState::Blocked,
                AgentState::Done | AgentState::Idle => logic::TabState::Done,
                AgentState::Error => logic::TabState::Error,
            };
            if let Some(text) = state_text(state) {
                pieces.push(Piece::State(col(status::color(g, state)), text.get(lang).to_owned()));
            }
        }
        if let Some(usage) = &entry.usage {
            if let Some(model) = usage.model.clone().filter(|_| show.model) {
                pieces.push(Piece::Text(model));
            }
            if let Some(share) = usage.context.filter(|_| show.context) {
                pieces.push(Piece::Meter(share as f32));
            }
            if let Some(cost) = usage.cost.filter(|_| show.cost) {
                pieces.push(Piece::Text(format!("${cost:.2}")));
            }
            if let Some(ms) = usage.duration_ms.filter(|_| show.time) {
                let s = ms / 1000;
                let time = if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) };
                pieces.push(Piece::Text(time));
            }
        }
        pieces
    }

    fn paint_piece(&mut self, piece: &Piece, x: f32) {
        let g = self.glass;
        let bar = layout::TITLE_HEIGHT;
        let line = self.ui.line_height(PIECE_SIZE);
        let text_y = (bar - line) / 2.0;
        let quiet = col(g.text_secondary());
        match piece {
            Piece::State(color, text) => {
                self.ui.rrect(Rect::new(x, bar / 2.0 - 3.0, 6.0, 6.0), 3.0, *color, None);
                self.ui.text(x + 12.0, text_y, text, PIECE_SIZE, col(g.text()), false);
            }
            Piece::Meter(share) => {
                let track = Rect::new(x, (bar - 3.0) / 2.0, METER, 3.0);
                let fill = col(if *share > 90.0 { g.mark(g.danger) } else if *share > 70.0 { g.mark(g.warning) } else { g.accent });
                self.ui.rrect(track, 1.5, col(g.divider()), None);
                self.ui.rrect(Rect::new(track.x, track.y, METER * (share / 100.0).clamp(0.0, 1.0), 3.0), 1.5, fill, None);
                self.ui.text(x + METER + 6.0, text_y, &format!("{share:.0}%"), PIECE_SIZE, quiet, false);
            }
            Piece::Glyph(ch, text) => {
                let icon_line = self.ui.line_height(ICON_SIZE);
                self.ui.icon(x, (bar - icon_line) / 2.0 + ICON_NUDGE, *ch, ICON_SIZE, quiet);
                self.ui.text(x + ICON_SIZE + 6.0, text_y, text, PIECE_SIZE, quiet, false);
            }
            Piece::Text(text) => {
                self.ui.text(x, text_y, text, PIECE_SIZE, quiet, false);
            }
        }
    }

    // --- the menu -----------------------------------------------------------------------------------------------

    /// The menu of the window, opened by a right click on the title bar of `window` (the shell's or the settings') at the
    /// screen point (`x`, `y`): restore, move, size, minimize, maximize and close, as Windows has it for a native title
    /// bar, with what the state of the window allows, and in the shell's the settings under them. These windows have no menu
    /// of the system to show: they leave out `WS_SYSMENU` so that DWM draws no caption buttons over theirs.
    pub fn open_window_menu(&mut self, window: HWND, x: i32, y: i32) {
        use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsZoomed, SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_MOVE, SC_RESTORE, SC_SIZE};
        let lang = self.language();
        // SAFETY: plain queries of a window of this thread.
        let (maximized, minimized) = unsafe { (IsZoomed(window).as_bool(), IsIconic(window).as_bool()) };
        let row = |text: Text, command: u32, glyph: char, hint: &str| popup::Row {
            label: text.get(lang).to_owned(),
            hint: hint.to_owned(),
            action: Some(popup::Action::Window(command)),
            current: false,
            mark: Some(logic::Mark::Glyph(glyph)),
            program: None,
        };
        let mut rows = Vec::new();
        if maximized || minimized {
            rows.push(row(Text::CommonRestore, SC_RESTORE, '\u{E923}', ""));
        }
        if !maximized {
            // A blank glyph keeps their labels in the column of the others.
            rows.push(row(Text::WindowMove, SC_MOVE, ' ', ""));
            rows.push(row(Text::WindowSize, SC_SIZE, ' ', ""));
        }
        rows.push(row(Text::WindowMinimize, SC_MINIMIZE, '\u{E921}', ""));
        if !maximized {
            rows.push(row(Text::WindowMaximize, SC_MAXIMIZE, '\u{E922}', ""));
        }
        rows.push(popup::Row::separator());
        rows.push(row(Text::CommonClose, SC_CLOSE, '\u{E8BB}', "Alt+F4"));
        if window == self.hwnd {
            rows.push(popup::Row::separator());
            let settings = Command::AppSettings;
            rows.push(popup::Row { label: settings.label(lang), hint: self.keymap.shortcut_text(settings), action: Some(popup::Action::Command(settings)), current: false, mark: Some(logic::Mark::Glyph('\u{E713}')), program: None });
        }
        self.window_menu_for = window;
        // The pop-up hangs from its top right corner: put its left edge at the pointer.
        self.open_popup(rows, popup::Kind::List, (x + (popup::WIDTH * self.scale).round() as i32, y));
    }

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
                    mark: None,
                    program: None,
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
                            mark: None,
                            program: None,
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
                            mark: None,
                            program: None,
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
                Some(popup::Row { label: item.label, hint: item.shortcut, action: Some(action), current: false, mark: None, program: None })
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
        if let Some(win) = self.settings_win.as_ref() {
            crate::win::set_backdrop(win.hwnd, backdrop, self.dark());
        }
    }

    fn save_settings(&mut self) {
        if !self.settings_readable {
            util::log("settings: the file could not be read, so it is not written over; fix it and the changes made here are saved again");
            return;
        }
        if let Some(path) = store::settings_path()
            && let Err(e) = store::save_keeping_original(&path, &settings::serialize(&self.settings))
        {
            util::log(&format!("settings: {e}"));
        }
        self.settings_stamp = settings_stamp();
    }

    // --- the settings window ------------------------------------------------------------------------------------

    /// The area of the page: the client area of the window of the settings below its title row, in DIP.
    fn page_area(&self) -> Rect {
        let (w, h) = self.settings_win.as_ref().map_or((0.0, 0.0), |win| win.area());
        Rect::new(0.0, settings_window::TITLE, w, (h - settings_window::TITLE).max(0.0))
    }

    /// Asks for a frame of the window of the settings.
    fn page_redraw(&self) {
        if let Some(win) = self.settings_win.as_ref() {
            // SAFETY: invalidating a window of this thread.
            unsafe { let _ = InvalidateRect(Some(win.hwnd), None, false); }
        }
    }

    /// The rectangles of the page for the state it is in now.
    fn page_frame(&mut self) -> Option<settings_page::Frame> {
        let lang = self.language();
        let area = self.page_area();
        let page = self.page.as_ref()?;
        let ui = &mut self.ui;
        let view = settings_page::View { lang, settings: &self.settings, keymap: &self.keymap, profiles: &self.profiles, programs: &self.programs, claude: &self.claude, area };
        Some(settings_page::build(page, &view, &mut |text, size, bold| ui.measure(text, size, bold)))
    }

    /// Opens the window of the settings at the section of the row `focus`, with that row under the keyboard. A window that
    /// is open already is brought to the front, and kept as it was when no row is asked for.
    fn open_settings_page(&mut self, focus: Option<&str>) {
        self.close_popup();
        // Claude Code may have been installed or connected elsewhere since the page last looked.
        self.claude = agent::claude();
        let open = self.settings_win.is_some();
        let mut page = self.page.take().unwrap_or_default();
        let row = focus.and_then(settings_ui::find);
        if !open || row.is_some() {
            page.query.clear();
            page.cursor = textfield::Cursor::default();
            page.editing = None;
            page.scroll = 0.0;
            page.focus = None;
        }
        if let Some(row) = row {
            page.section = Section::ALL.iter().position(|s| *s == row.section).unwrap_or(0);
        }
        self.page = Some(page);
        if !open {
            let title = Text::AppSettings.get(self.language());
            match settings_window::open(self.hwnd, &self.gpu, self.settings.appearance.backdrop, self.dark(), title) {
                Ok(win) => self.settings_win = Some(win),
                Err(e) => {
                    util::log(&format!("settings window: {e}"));
                    self.page = None;
                    return;
                }
            }
        }
        if let Some(row) = row
            && let Some(frame) = self.page_frame()
            && let Some(at) = frame.entries.iter().position(|entry| matches!(entry, settings_page::Entry::Setting(r) if r.id == row.id))
            && let Some(page) = self.page.as_mut()
        {
            page.focus = Some(at);
            page.scroll = frame.reveal(at, 0.0).min(frame.max_scroll());
        }
        if let Some(win) = self.settings_win.as_ref() {
            // SAFETY: a window of this thread.
            unsafe { let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(win.hwnd); }
        }
        self.page_redraw();
    }

    /// The window of the settings is to close: asked for by posting the close to it, so that the window goes away outside
    /// the call that asked.
    fn request_close_settings(&self) {
        if let Some(win) = self.settings_win.as_ref() {
            // SAFETY: posting to a window of this thread.
            unsafe { let _ = PostMessageW(Some(win.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)); }
        }
    }

    /// Lets go of the window of the settings and of what the page remembers. The caller destroys the window once it holds
    /// no borrow of the application: the windows that are activated by it going away are told then.
    pub fn close_settings_window(&mut self) -> Option<SettingsWin> {
        self.page = None;
        self.settings_win.take()
    }

    fn settings_point(&self, x: i32, y: i32) -> (f32, f32) {
        let scale = self.settings_win.as_ref().map_or(self.scale, |win| win.scale);
        (x as f32 / scale, y as f32 / scale)
    }

    /// What is under a point of the window of the settings (pixels): a caption button, the title row, or the page.
    pub fn settings_hit_test(&self, x: i32, y: i32) -> Hit {
        let Some(win) = self.settings_win.as_ref() else { return Hit::None };
        let (x, y) = self.settings_point(x, y);
        settings_window::hit(win.area().0, x, y)
    }

    pub fn settings_caption_hover(&mut self, hit: Hit) {
        if let Some(win) = self.settings_win.as_mut()
            && win.hover != hit
        {
            win.hover = hit;
            self.page_redraw();
        }
    }

    pub fn settings_caption_press(&mut self, hit: Hit) {
        if let Some(win) = self.settings_win.as_mut() {
            win.pressed = hit;
            self.page_redraw();
        }
    }

    /// A caption button was let go of: whether it is the one that was pressed.
    pub fn settings_caption_release(&mut self, hit: Hit) -> bool {
        let Some(win) = self.settings_win.as_mut() else { return false };
        let pressed = std::mem::replace(&mut win.pressed, Hit::None);
        self.page_redraw();
        pressed == hit
    }

    /// The window of the settings has this size in pixels now.
    pub fn settings_size(&mut self, size_px: (u32, u32), maximized: bool) {
        if let Some(win) = self.settings_win.as_mut() {
            win.surface.resize(&self.gpu, size_px);
            win.maximized = maximized;
        }
        // Rows that were scrolled to the end of a tall page stay inside a page that has grown.
        if let Some(max) = self.page_frame().map(|frame| frame.max_scroll())
            && let Some(page) = self.page.as_mut()
        {
            page.scroll = page.scroll.min(max);
        }
        self.page_redraw();
    }

    /// The window of the settings moved to a monitor with another scale.
    pub fn settings_dpi(&mut self, scale: f32) {
        if let Some(win) = self.settings_win.as_mut() {
            win.scale = scale;
        }
        self.page_redraw();
    }

    pub fn settings_move(&mut self, x: i32, y: i32) {
        let (x, y) = self.settings_point(x, y);
        self.page_hover(x, y);
    }

    pub fn settings_leave(&mut self) {
        if let Some(page) = self.page.as_mut()
            && page.hover.take().is_some()
        {
            self.page_redraw();
        }
    }

    pub fn settings_button(&mut self, x: i32, y: i32, down: bool) -> Capture {
        // A click on the settings dismisses a menu open over them, and does nothing else.
        if self.popup.is_some() {
            if !down { self.close_popup(); }
            return Capture::None;
        }
        let (x, y) = self.settings_point(x, y);
        self.page_button(x, y, MouseButton::Left, down)
    }

    pub fn settings_wheel(&mut self, delta: i16) {
        self.page_wheel(delta);
    }

    /// The mouse capture ended: a slider is no longer held.
    pub fn settings_capture_lost(&mut self) {
        if let Some(page) = self.page.as_mut() {
            page.drag = None;
        }
    }

    pub fn settings_char(&mut self, unit: u16) {
        if unit >= 0x20 || unit == 0x08 {
            self.page_text(unit);
        }
    }

    /// A key went down in the window of the settings. `true` when the page took it.
    pub fn settings_key(&mut self, vk_code: i32, scan: u16, extended: bool, repeat: bool) -> bool {
        // The menu of the window, opened over the settings, has their keyboard while it is open.
        if self.popup.is_some() {
            if vk_code == 0x1B { self.close_popup(); } else { self.popup_key(vk_code, repeat); }
            return true;
        }
        if self.page.is_none() {
            return false;
        }
        let input_method = vk_code == vk::VK_PROCESSKEY || vk_code == vk::VK_PACKET;
        if self.page.as_ref().is_some_and(|page| page.capturing.is_some()) && self.capture_chord(vk_code, scan, extended, repeat) {
            return true;
        }
        let held = held();
        if !input_method && self.page_field_key(vk_code) {
            return true;
        }
        if !input_method && !held.ctrl && !held.alt && self.page_key(vk_code, repeat) {
            return true;
        }
        // Of the shortcuts of the window two mean something here: the one of the settings, which closes them, and the one
        // that pastes into a line of text.
        if let Some(name) = nekominal_input::physical_key(scan, extended).map(|key| key.name())
            && let Some(command) = self.keymap.find(held, name)
        {
            match command {
                Command::AppSettings if !repeat => self.request_close_settings(),
                Command::EditPaste if !repeat => self.paste_into_page(),
                _ => return false,
            }
            return true;
        }
        false
    }

    /// Puts the first line of the clipboard into the line of text that has the keyboard: the search or the field being edited.
    fn paste_into_page(&mut self) {
        let text = clipboard::read_text().unwrap_or_default();
        let line = text.lines().next().unwrap_or_default().to_owned();
        for unit in line.encode_utf16() {
            self.page_text(unit);
        }
    }

    /// Where the caret of the line of text being typed in is, in pixels of the window of the settings: left, top and height
    /// (the candidate window of the input method hangs under it).
    pub fn settings_ime_anchor(&mut self) -> Option<(i32, i32, i32)> {
        let scale = self.settings_win.as_ref()?.scale;
        let page = self.page.as_ref()?;
        let (scroll, editing) = (page.scroll, page.editing.as_ref().map(|(id, _)| *id));
        let frame = self.page_frame()?;
        let wanted = editing.map_or(Part::Search, Part::Field);
        let item = frame.items.iter().find(|item| item.part == Some(wanted))?;
        let rect = frame.on_screen(item, scroll);
        let left = rect.x + if editing.is_some() { 10.0 } else { 40.0 };
        Some(((left * scale) as i32, ((rect.y + 4.0) * scale) as i32, ((rect.h - 8.0) * scale) as i32))
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
        self.page_redraw();
    }

    /// The part of the page under a point of the window (DIP).
    fn page_part_at(&mut self, x: f32, y: f32) -> Option<Part> {
        let scroll = self.page.as_ref()?.scroll;
        self.page_frame()?.part_at(scroll, x, y)
    }

    fn page_hover(&mut self, x: f32, y: f32) {
        if let Some(id) = self.page.as_ref().and_then(|page| page.drag) {
            self.drag_slider(id, x);
            return;
        }
        let part = self.page_part_at(x, y);
        if let Some(page) = self.page.as_mut()
            && page.hover != part
        {
            page.hover = part;
            self.page_redraw();
        }
    }

    /// The slider with this id takes the number under the pointer (`px` is its place in the window, in DIP).
    fn drag_slider(&mut self, id: &'static str, px: f32) {
        let Some(row) = settings_ui::find(id) else { return };
        let Control::Slider(range) = row.control else { return };
        let Some(scroll) = self.page.as_ref().map(|page| page.scroll) else { return };
        let Some(frame) = self.page_frame() else { return };
        let Some(area) = frame.items.iter().find(|item| item.part == Some(Part::Slider(id))) else { return };
        let track = settings_page::slider_track(frame.on_screen(area, scroll));
        self.set_setting(row, Value::Number(settings_page::slider_value(range, track, px)));
    }

    /// A press or release of the left button over the page.
    fn page_button(&mut self, x: f32, y: f32, button: MouseButton, down: bool) -> Capture {
        if button != MouseButton::Left {
            return Capture::None;
        }
        if !down {
            return if self.page.as_mut().and_then(|page| page.drag.take()).is_some() { Capture::Release } else { Capture::None };
        }
        let part = self.page_part_at(x, y);
        if let Some(page) = self.page.as_mut() {
            // A press anywhere gives up the wait for a shortcut, unless it is on the row that waits.
            if page.capturing.is_some() && part != page.capturing.map(Part::Binding) {
                (page.capturing, page.reserved) = (None, false);
            }
        }
        if !matches!(part, Some(Part::Field(id)) if self.page.as_ref().and_then(|p| p.editing.as_ref()).is_some_and(|(editing, _)| *editing == id)) {
            self.page_commit_edit();
        }
        let Some(part) = part else { return Capture::None };
        let mut capture = Capture::None;
        match part {
            Part::Nav(i) => {
                if let Some(page) = self.page.as_mut() {
                    (page.section, page.query, page.scroll, page.focus) = (i, String::new(), 0.0, None);
                    page.cursor = textfield::Cursor::default();
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
                self.drag_slider(id, x);
                capture = Capture::Take;
            }
            Part::Field(id) => {
                if let Some(row) = settings_ui::find(id)
                    && let Value::Text(text) = (row.get)(&self.settings)
                    && let Some(page) = self.page.as_mut()
                    && page.editing.is_none()
                {
                    page.edit_cursor = textfield::Cursor { caret: textfield::length(&text), anchor: Some(0) };
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
            Part::Binding(command) => {
                if let Some(page) = self.page.as_mut() {
                    (page.capturing, page.reserved) = (Some(command), false);
                }
            }
            Part::ResetKey(command) => self.change_bindings(|bindings| keybindings::reset(bindings, command)),
        }
        self.page_redraw();
        capture
    }

    fn page_wheel(&mut self, delta: i16) {
        let Some(max) = self.page_frame().map(|frame| frame.max_scroll()) else { return };
        if let Some(page) = self.page.as_mut() {
            page.scroll = (page.scroll - f32::from(delta) / 120.0 * 3.0 * 52.0).clamp(0.0, max);
            self.page_redraw();
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
                    self.page_redraw();
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
            settings_page::Entry::Key(_) | settings_page::Entry::Claude => None,
        });
        let focused_command = focus.and_then(|i| frame.entries.get(i)).and_then(|entry| match entry {
            settings_page::Entry::Key(command) => Some(*command),
            settings_page::Entry::Setting(_) | settings_page::Entry::Claude => None,
        });
        match vk_code {
            VK_ESCAPE => {
                if page.query.is_empty() {
                    self.request_close_settings();
                } else if let Some(page) = self.page.as_mut() {
                    (page.query, page.scroll, page.focus) = (String::new(), 0.0, None);
                    page.cursor = textfield::Cursor::default();
                }
            }
            // Nothing to move to: a search that found nothing.
            VK_UP | VK_DOWN if frame.entries.is_empty() => {}
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
            VK_RETURN if !repeat && focused_command.is_some() => {
                if let (Some(command), Some(page)) = (focused_command, self.page.as_mut()) {
                    (page.capturing, page.reserved) = (Some(command), false);
                }
            }
            VK_RETURN if !repeat => {
                let Some(row) = focused_row else { return true };
                if matches!(row.control, Control::Text) {
                    if let (Value::Text(text), Some(page)) = ((row.get)(&self.settings), self.page.as_mut()) {
                        page.edit_cursor = textfield::Cursor { caret: textfield::length(&text), anchor: Some(0) };
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
            VK_DELETE if focused_command.is_some() => {
                if let Some(command) = focused_command {
                    self.change_bindings(|bindings| keybindings::reset(bindings, command));
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
        self.page_redraw();
        true
    }

    /// Changes the shortcuts the user bound, puts the new key map in effect and keeps the settings.
    fn change_bindings(&mut self, change: impl FnOnce(&mut std::collections::BTreeMap<String, Vec<String>>)) {
        let before = self.settings.clone();
        change(&mut self.settings.keybindings);
        self.keymap = keymap::build_keymap(commands::default_bindings(), &self.settings.keybindings).keymap;
        self.apply_settings(&before);
    }

    /// A key while the page waits for the shortcut of a command: the chord pressed becomes its shortcut. Escape gives up,
    /// Delete and Backspace take the shortcuts away, a chord that programs in the terminal need is refused. True when the
    /// key was taken.
    fn capture_chord(&mut self, vk_code: i32, scan: u16, extended: bool, repeat: bool) -> bool {
        let Some(command) = self.page.as_ref().and_then(|page| page.capturing) else { return false };
        // The input method has this key; there is nothing to read from it.
        if vk_code == vk::VK_PROCESSKEY || vk_code == vk::VK_PACKET {
            return false;
        }
        let mods = held();
        let modifier_only = matches!(vk_code, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
        if repeat || modifier_only {
            return true;
        }
        match vk_code {
            0x1B => {}
            0x08 | 0x2E if !mods.ctrl && !mods.alt && !mods.shift && !mods.meta => self.change_bindings(|bindings| keybindings::set(bindings, command, &[])),
            _ => {
                let chord = nekominal_input::physical_key(scan, extended).and_then(|key| keybindings::chord_of(key.name(), mods));
                let Some(chord) = chord else { return true };
                if keybindings::needed_by_programs(chord) {
                    if let Some(page) = self.page.as_mut() {
                        page.reserved = true;
                    }
                    self.page_redraw();
                    return true;
                }
                let keymap = self.keymap.clone();
                let replacing = keymap.shortcuts(command).first().copied();
                self.change_bindings(|bindings| keybindings::assign(bindings, &keymap, command, replacing, chord));
            }
        }
        if let Some(page) = self.page.as_mut() {
            (page.capturing, page.reserved) = (None, false);
        }
        self.page_redraw();
        true
    }

    /// The keys of a line of text (the caret, the selection, Delete, Ctrl+A, Ctrl+C, Ctrl+X) in the field being edited, or in
    /// the search while no row has the keyboard. True when the key was one of them.
    fn page_field_key(&mut self, vk_code: i32) -> bool {
        let mods = held();
        let Some(page) = self.page.as_mut() else { return false };
        let editing = page.editing.is_some();
        let edit = if let Some((_, text)) = page.editing.as_mut() {
            textfield::key(text, &mut page.edit_cursor, vk_code, mods.ctrl, mods.shift)
        } else if page.focus.is_none() && !page.query.is_empty() {
            textfield::key(&mut page.query, &mut page.cursor, vk_code, mods.ctrl, mods.shift)
        } else {
            return false;
        };
        match edit {
            textfield::Edit::Ignored => return false,
            textfield::Edit::Done { changed } => {
                if changed && !editing {
                    (page.scroll, page.focus) = (0.0, None);
                }
            }
            textfield::Edit::Copy(text) => {
                clipboard::write_text(&text);
            }
            textfield::Edit::Cut(text) => {
                clipboard::write_text(&text);
                if !editing {
                    (page.scroll, page.focus) = (0.0, None);
                }
            }
        }
        self.page_redraw();
        true
    }

    /// Text typed while the page is open goes into the field being edited, or into the search.
    fn page_text(&mut self, unit: u16) {
        let Some(page) = self.page.as_mut() else { return };
        let changed = match page.editing.as_mut() {
            Some((_, text)) => textfield::type_unit(text, &mut page.edit_cursor, &mut self.text, unit),
            None => {
                let changed = textfield::type_unit(&mut page.query, &mut page.cursor, &mut self.text, unit);
                if changed {
                    (page.scroll, page.focus) = (0.0, None);
                }
                changed
            }
        };
        if changed {
            self.page_redraw();
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
        let (loaded, readable) = load_settings();
        self.settings_readable = readable;
        self.settings_stamp = settings_stamp();
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
        if self.settings.tabs != before.tabs {
            self.relayout();
        }
        let effects = logic::effects(before, &self.settings);
        if effects.font {
            for entry in self.tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()) {
                let result = (|| {
                    if self.settings.font.family != before.font.family {
                        entry.view.set_font_families(self.settings.font.family.clone())?;
                    }
                    entry.view.set_font_size(self.settings.font.size as f32)?;
                    entry.view.set_line_height(self.settings.font.line_height as f32)
                })();
                if let Err(e) = result {
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
                mark: Some(logic::profile_mark(profile.icon)),
                program: self.programs.get(i).cloned().flatten(),
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
        if let Some(popup) = self.popup.as_mut().filter(|popup| popup.kind == popup::Kind::Find) {
            popup.has_keys = true;
            // SAFETY: invalidating our own window.
            unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
            return;
        }
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
        let Some(popup) = self.popup.as_mut() else { return };
        let Some(query) = popup.query.as_mut() else { return };
        if !textfield::type_unit(query, &mut popup.cursor, &mut self.text, unit) {
            return;
        }
        self.query_changed();
    }

    /// The query of the palette or of the find bar changed: the rows or the matches follow it.
    fn query_changed(&mut self) {
        let Some(query) = self.popup.as_ref().and_then(|popup| popup.query.clone()) else { return };
        if self.popup.as_ref().is_some_and(|popup| popup.kind == popup::Kind::Find) {
            self.find_search(&query);
            return;
        }
        let rows = self.palette_rows(&query);
        let Some(popup) = self.popup.as_mut() else { return };
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
            .map(|command| popup::Row { label: command.label(lang), hint: self.keymap.shortcut_text(command), action: Some(popup::Action::Command(command)), current: false, mark: None, program: None })
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
                mark: None,
                program: None,
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

    /// A pop-up has the keyboard and the pointer: all but a find bar that something else was clicked after.
    pub fn popup_keeps_owner_active(&self) -> bool {
        self.popup.as_ref().is_some_and(|popup| popup.kind != popup::Kind::Find || popup.has_keys)
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
        // A click on the find bar gives it the keyboard back.
        if let Some(popup) = self.popup.as_mut().filter(|popup| popup.kind == popup::Kind::Find) {
            popup.has_keys = true;
        }
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
        // The keys of a line of text: the caret, the selection, Delete, Ctrl+A, Ctrl+C and Ctrl+X.
        let edit = {
            let mods = held();
            match self.popup.as_mut() {
                Some(popup) if popup.kind != popup::Kind::Confirm => match popup.query.as_mut() {
                    Some(query) => textfield::key(query, &mut popup.cursor, vk_code, mods.ctrl, mods.shift),
                    None => textfield::Edit::Ignored,
                },
                _ => textfield::Edit::Ignored,
            }
        };
        match edit {
            textfield::Edit::Ignored => {}
            textfield::Edit::Done { changed } => {
                if changed {
                    self.query_changed();
                } else if let Some(popup) = self.popup.as_ref() {
                    // SAFETY: invalidating our own window.
                    unsafe { let _ = InvalidateRect(Some(popup.hwnd), None, false); }
                }
                return;
            }
            textfield::Edit::Copy(text) => {
                clipboard::write_text(&text);
                return;
            }
            textfield::Edit::Cut(text) => {
                clipboard::write_text(&text);
                self.query_changed();
                return;
            }
        }
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
                // Destroyed outside this borrow of the window, so that the last window can still save the session then.
                Some(Confirmation::CloseWindow) => unsafe { let _ = PostMessageW(Some(self.hwnd), crate::win::WM_DESTROY_WINDOW, WPARAM(0), LPARAM(0)); },
                None => {}
            },
            Some(popup::Action::Cancel) => {},
            Some(popup::Action::Command(command)) => self.run(command),
            Some(popup::Action::Tab(index)) => self.select(index),
            Some(popup::Action::Backdrop(backdrop)) => self.set_appearance(backdrop, self.settings.appearance.opacity),
            Some(popup::Action::ShowSetting(id)) => self.open_settings_page(Some(id)),
            Some(popup::Action::NewTab(index)) => self.new_tab_with(index),
            // SAFETY: posting to a window of this thread.
            Some(popup::Action::Window(command)) => unsafe { let _ = PostMessageW(Some(self.window_menu_for), WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0)); },
            Some(popup::Action::ColorScheme(name)) => {
                let before = self.settings.clone();
                self.settings.theme = name.to_owned();
                self.apply_settings(&before);
            }
            None => {}
        }
    }

    /// Writes the tabs, the layout of their panes, the directories they were in and the place of the window, for the next run
    /// to start where this one stopped. A window with no tab left writes an empty session: the next run starts afresh.
    pub fn save_session(&self) {
        let Some(path) = store::session_path() else { return };
        let tabs = self
            .tabs
            .iter()
            .map(|tab| TabRecord {
                layout: tab.tree.clone(),
                panes: tab.panes.iter().map(|entry| (entry.id, PaneRecord { profile: entry.profile_id.clone(), cwd: entry.cwd.clone() })).collect(),
                active: tab.focused,
                zoomed: tab.zoomed.then_some(tab.focused),
            })
            .collect();
        let mut placement = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        // SAFETY: a window of this thread and a structure that lives through the call.
        let window = unsafe { GetWindowPlacement(self.hwnd, &mut placement) }.ok().map(|()| {
            let rect = placement.rcNormalPosition;
            Placement {
                rect: PixelRect { x: rect.left, y: rect.top, width: (rect.right - rect.left).max(0) as u32, height: (rect.bottom - rect.top).max(0) as u32 },
                maximized: placement.showCmd == SW_SHOWMAXIMIZED.0 as u32,
            }
        });
        let session = Session { window, tabs, active_tab: self.selected };
        if let Err(e) = store::save(&path, &session.to_json()) {
            util::log(&format!("session: {e}"));
        }
    }

    /// Starts the tabs of the saved session. False when there is nothing to restore (or nothing could be started).
    fn restore_session(&mut self) -> bool {
        let Some(text) = store::session_path().and_then(|path| store::load(&path).ok().flatten()) else { return false };
        let parsed = session::Session::parse(&text);
        for problem in &parsed.problems {
            util::log(&format!("session: {problem}"));
        }
        let area = self.pane_area();
        let size = (area.w.max(1) as u32, area.h.max(1) as u32);
        for record in &parsed.session.tabs {
            let restored = Tab::restore(record.layout.clone(), record.active, record.zoomed.is_some(), |id| {
                let pane = record.panes.get(&id);
                let profile = pane.and_then(|pane| self.profiles.iter().find(|p| p.id == pane.profile)).unwrap_or(&self.profile);
                self.make_entry_with(id, size, profile, pane.and_then(|pane| pane.cwd.as_deref()))
            });
            if let Some(tab) = restored {
                self.tabs.push(tab);
            }
        }
        if self.tabs.is_empty() {
            return false;
        }
        self.selected = parsed.session.active_tab.min(self.tabs.len() - 1);
        self.placement = parsed.session.window;
        self.relayout();
        self.sync_focus();
        true
    }

    /// Where the window was in the session that was restored, once; None when it would be out of reach on the screens.
    pub fn take_placement(&mut self) -> Option<Placement> {
        let placement = self.placement.take()?;
        // SAFETY: plain queries.
        let screen = unsafe {
            PixelRect {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                width: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(0) as u32,
                height: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(0) as u32,
            }
        };
        (placement.rect.width >= session::MIN_WINDOW_SIZE && placement.rect.height >= session::MIN_WINDOW_SIZE && placement.is_reachable_on(&[screen])).then_some(placement)
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
            widgets::menu_row(ui, &g, rect, popup.hover == Some(i), row.current);
            let y = rect.y + (rect.h - line) / 2.0;
            let hint_width = ui.measure(&row.hint, TEXT_SIZE, false);
            ui.text(rect.x + rect.w - 12.0 - hint_width, y, &row.hint, TEXT_SIZE, col(g.text_secondary()), false);
            let label_end = rect.x + rect.w - 12.0 - hint_width - 8.0;
            let mut label_x = rect.x + 12.0;
            if let Some(mark) = row.mark {
                let square = Rect::new(label_x, rect.y + (rect.h - popup::MARK_SIZE) / 2.0, popup::MARK_SIZE, popup::MARK_SIZE);
                if !row.program.as_deref().is_some_and(|path| program_icon(ui, path, square)) {
                    widgets::mark(ui, square, mark, col(g.text_secondary()));
                }
                label_x += popup::MARK_SIZE + popup::MARK_GAP;
            }
            ui.push_clip(Rect::new(rect.x, rect.y.max(header), (label_end - rect.x).max(0.0), rect.h));
            ui.text(label_x, y, &row.label, TEXT_SIZE, col(g.text()), false);
            ui.pop_clip();
        }
        ui.pop_clip();
        if let Some(query) = &popup.query {
            let y = (header - line) / 2.0;
            let icon_line = ui.line_height(14.0);
            ui.icon(16.0, (header - icon_line) / 2.0, '\u{E721}', 14.0, col(g.text_secondary()));
            let status_width = ui.measure(&popup.status, TEXT_SIZE, false);
            let end = popup.size.0 - 16.0 - if popup.status.is_empty() { 0.0 } else { status_width + 12.0 };
            let area = Rect::new(popup::QUERY_LEFT, 0.0, (end - popup::QUERY_LEFT).max(0.0), header);
            popup.caret = widgets::line_edit(ui, &g, area, TEXT_SIZE, &widgets::LineEdit { text: query, cursor: Some(popup.cursor), preedit: &popup.preedit, placeholder: "" });
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
        self.ui.icon(rect.x + (rect.w - width) / 2.0, rect.y + (rect.h - line) / 2.0 + ICON_NUDGE, ch, size, color);
    }
}
