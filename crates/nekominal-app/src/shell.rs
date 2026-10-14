//! The window: glass, top bar, tabs, overlays, commands and the keyboard layer.
//!
//! One `Shell` per process (single window for now). Everything it owns lives on the UI thread; event closures
//! hold `Weak<Shell>` so closing a tab really frees its panes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nekominal_model::{
    bell::{BellContext, bell_effects},
    commands::{self, Command},
    keymap::{self, Keymap},
    layout::{Axis, PaneId},
    profiles::{self, DiscoveredProfile, Profile},
    settings::{self, Backdrop, Settings},
    store,
    strings::{Lang, Text},
    themes::{self},
};
use nekominal_pane::UiEvent;
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_term::osc::Progress;
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use crate::chip::{CHIP_GAP, Chip, TabEvent, TabId};
use crate::design;
use crate::glass::{self, Glass};
use crate::i18n::{self, localize, t, tf};
use crate::keyboard;
use crate::logic::{
    self, Attention, COUNT_BLOCK_WIDTH, PaneSignal, Plate, REVEAL_MARGIN, Reported, TitleParts, TitleSource,
};
use crate::palette::TabRow;
use crate::pane_view::{Look, PaneView};
use crate::tab::Tab;
use crate::util::*;
use crate::window;
use crate::{clipboard, findbar, palette, settings_page};

const SHELL_XAML: &str = include_str!("xaml/shell.xaml");

/// How long a change of settings waits for further changes before it is written to disk.
const SAVE_DELAY: Duration = Duration::from_millis(600);

/// Space on the bar that is not the tab strip: app mark, new-tab buttons, tool cluster margins, the smallest
/// useful drag area. Together with the cluster, the position plate and the caption buttons this bounds the width
/// of the strip.
const BAR_FIXED_WIDTH: f64 = 46.0 + 4.0 + 54.0 + 18.0 + 40.0;

/// Padding of the strip inside its scroll viewer, both sides together (`shell.xaml`).
const STRIP_PADDING: f64 = 4.0;

/// The smallest the strip gets, however narrow the window.
const STRIP_MIN_WIDTH: f64 = 120.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    None,
    Palette,
    /// The palette as the list of every tab.
    Tabs,
    Find,
    Settings,
}

struct Ui {
    top_bar: Grid,
    bar_tint: Border,
    tab_scroll: ScrollViewer,
    tab_strip: StackPanel,
    /// Counts of the chips hidden behind the left and right edge of the strip, and their fades.
    more_left: Border,
    more_left_text: TextBlock,
    fade_left: Border,
    more_right: Border,
    more_right_text: TextBlock,
    fade_right: Border,
    new_tab: Button,
    new_tab_menu: Button,
    tab_plate: Border,
    tab_plate_button: Button,
    tab_plate_text: TextBlock,
    drag: Grid,
    tool_cluster: Border,
    tool_split_right: Button,
    tool_split_down: Button,
    tool_search: Button,
    tool_palette: Button,
    tool_appearance: Button,
    tool_settings: Button,
    appearance_flyout: Flyout,
    backdrop_choice: RadioButtons,
    opacity_slider: Slider,
    opacity_value: TextBlock,
    left_inset: ColumnDefinition,
    right_inset: ColumnDefinition,
    terminal_area: Grid,
    body: Grid,
    message_bar: InfoBar,
}

pub struct Shell {
    pub window: Window,
    pub root: FrameworkElement,
    /// The palette brushes and the button style (`resources.xaml`), merged into the application resources.
    resources: ResourceDictionary,
    ui: Ui,
    pub settings: RefCell<Settings>,
    pub keymap: RefCell<Keymap>,
    pub profiles: RefCell<Vec<Profile>>,
    pub glass: Cell<Glass>,
    look: Rc<Look>,
    tabs: RefCell<Vec<Rc<Tab>>>,
    selected: Cell<usize>,
    /// Tabs by most recent use, and the walk of the next/previous tab keys through them.
    mru: RefCell<logic::Mru<TabId>>,
    /// The width of every chip as the last layout of the strip set it.
    strip_widths: RefCell<Vec<f64>>,
    /// The width of the visible part of the strip as the last layout set it (the viewer reports it late).
    strip_viewport: Cell<f64>,
    /// The selected chip must be scrolled into view once the viewer has laid out at its new size (it clamps a
    /// scroll to the size it had).
    reveal_pending: Cell<bool>,
    next_tab: Cell<TabId>,
    next_pane: Cell<PaneId>,
    pub overlay: Cell<Overlay>,
    terminal_focus: Cell<bool>,
    zoom_steps: Cell<i32>,
    save_due: Cell<Option<Instant>>,
    /// The backdrop the window has now: it is replaced only when the kind changes.
    backdrop_applied: Cell<Option<Backdrop>>,
    /// The opacity the controls asked for, applied by the next frame.
    pending_opacity: RefCell<logic::PendingOpacity>,
    /// The quick flyout is being brought in line with the settings: its events are not user input.
    syncing_controls: Cell<bool>,
    window_active: Cell<bool>,
    /// When the window last flashed its taskbar button or played a sound: those happen at most once a second.
    last_alert: Cell<Option<Instant>>,
    palette: RefCell<Option<Rc<palette::Palette>>>,
    find: RefCell<Option<Rc<findbar::FindBar>>>,
    settings_page: RefCell<Option<Rc<settings_page::SettingsPage>>>,
    discovery: RefCell<Option<mpsc::Receiver<Vec<nekominal_pty::profiles::Profile>>>>,
    revokers: RefCell<Vec<EventRevoker>>,
    rendering: RefCell<Option<EventRevoker>>,
    dialog: RefCell<Option<windows_future::IAsyncOperation<ContentDialogResult>>>,
    profile_menu: RefCell<Option<MenuFlyout>>,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

pub fn shell() -> Option<Rc<Shell>> {
    SHELL.with(|s| s.borrow().clone())
}

/// Runs `f` on the pane with this id, wherever it lives.
pub fn with_pane(id: PaneId, f: impl FnOnce(&PaneView)) {
    if let Some(shell) = shell()
        && let Some(pane) = shell.find_pane(id)
    {
        f(&pane);
    }
}

/// The language tag `Language::Auto` is resolved from. A user whose language list holds a Chinese language
/// (a Taiwanese input method next to an English Windows) gets Traditional Chinese; everyone else gets the
/// locale of the system.
fn system_locale() -> String {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    if let Some(chinese) = user_languages().into_iter().find(|tag| tag.to_ascii_lowercase().starts_with("zh")) {
        return chinese;
    }
    let mut buffer = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0).saturating_sub(1)])
}

/// The language list of the user (`Settings > Time & language`), most preferred first.
fn user_languages() -> Vec<String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_MULTI_SZ, RegGetValueW};
    use windows::core::w;
    let (subkey, value) = (w!("Control Panel\\International\\User Profile"), w!("Languages"));
    let mut bytes = 0u32;
    // SAFETY: the first call only asks for the size; the second fills a buffer of exactly that size.
    unsafe {
        if RegGetValueW(HKEY_CURRENT_USER, subkey, value, RRF_RT_REG_MULTI_SZ, None, None, Some(&mut bytes)).is_err()
            || bytes == 0
        {
            return Vec::new();
        }
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
        let mut capacity = bytes;
        if RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            value,
            RRF_RT_REG_MULTI_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut capacity),
        )
        .is_err()
        {
            return Vec::new();
        }
        buffer.split(|unit| *unit == 0).filter(|tag| !tag.is_empty()).map(String::from_utf16_lossy).collect()
    }
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

pub fn launch() -> Result<()> {
    let (tx, rx) = mpsc::channel();
    // Probing for shells starts processes (wsl.exe, vswhere.exe); it runs while the window comes up.
    let _ = std::thread::Builder::new().name("discover-shells".into()).spawn(move || {
        let _ = tx.send(nekominal_pty::profiles::discover());
    });

    let loaded = store::settings_path().map(|path| store::load(&path).map(|text| (path, text)));
    let (text, load_error) = match loaded {
        Some(Ok((_, text))) => (text, None),
        Some(Err(e)) => (None, Some(e.to_string())),
        None => (None, None),
    };
    let parsed = settings::parse(text.as_deref());
    let settings = parsed.settings;
    let lang = settings.language.resolve(&system_locale());
    i18n::set_lang(lang);
    let _ = ApplicationLanguages::SetPrimaryLanguageOverride(lang.tag());

    let window = Window::new()?;
    window.SetTitle("Nekominal")?;
    let resources: ResourceDictionary = XamlReader::Load(&design::resources_xaml())?.cast()?;
    Application::Current()?.Resources()?.MergedDictionaries()?.Append(&resources)?;
    let root = load_xaml(&localize(SHELL_XAML))?;

    let scheme = themes::scheme(&settings.theme);
    let glass = Glass::from_scheme(scheme);
    let opacity = glass::terminal_opacity(settings.appearance.backdrop, settings.appearance.opacity);
    let look = Rc::new(Look {
        theme: RefCell::new(glass::render_theme(scheme, opacity)),
        font_pt: Cell::new(settings.font.size as f32),
        line_height: Cell::new(settings.font.line_height as f32),
        families: RefCell::new(settings.font.family.clone()),
    });
    let built = keymap::build_keymap(commands::default_bindings(), &settings.keybindings);

    let elements = Ui {
        top_bar: find(&root, "TopBar")?,
        bar_tint: find(&root, "BarTint")?,
        tab_scroll: find(&root, "TabScroll")?,
        tab_strip: find(&root, "TabStrip")?,
        more_left: find(&root, "MoreLeft")?,
        more_left_text: find(&root, "MoreLeftText")?,
        fade_left: find(&root, "FadeLeft")?,
        more_right: find(&root, "MoreRight")?,
        more_right_text: find(&root, "MoreRightText")?,
        fade_right: find(&root, "FadeRight")?,
        new_tab: find(&root, "NewTabButton")?,
        new_tab_menu: find(&root, "NewTabMenuButton")?,
        tab_plate: find(&root, "TabPlate")?,
        tab_plate_button: find(&root, "TabPlateButton")?,
        tab_plate_text: find(&root, "TabPlateText")?,
        drag: find(&root, "DragRegion")?,
        tool_cluster: find(&root, "ToolCluster")?,
        tool_split_right: find(&root, "ToolSplitRight")?,
        tool_split_down: find(&root, "ToolSplitDown")?,
        tool_search: find(&root, "ToolSearch")?,
        tool_palette: find(&root, "ToolPalette")?,
        tool_appearance: find(&root, "ToolAppearance")?,
        tool_settings: find(&root, "ToolSettings")?,
        appearance_flyout: find(&root, "AppearanceFlyout")?,
        backdrop_choice: find(&root, "BackdropChoice")?,
        opacity_slider: find(&root, "OpacitySlider")?,
        opacity_value: find(&root, "OpacityValue")?,
        left_inset: find(&root, "LeftInsetColumn")?,
        right_inset: find(&root, "RightInsetColumn")?,
        terminal_area: find(&root, "TerminalArea")?,
        body: find(&root, "Body")?,
        message_bar: find(&root, "MessageBar")?,
    };

    let shell = Rc::new(Shell {
        window: window.clone(),
        root: root.clone(),
        resources,
        ui: elements,
        settings: RefCell::new(settings),
        keymap: RefCell::new(built.keymap),
        profiles: RefCell::new(Vec::new()),
        glass: Cell::new(glass),
        look,
        tabs: RefCell::new(Vec::new()),
        selected: Cell::new(0),
        mru: RefCell::new(logic::Mru::default()),
        strip_widths: RefCell::new(Vec::new()),
        strip_viewport: Cell::new(0.0),
        reveal_pending: Cell::new(false),
        next_tab: Cell::new(1),
        next_pane: Cell::new(1),
        overlay: Cell::new(Overlay::None),
        terminal_focus: Cell::new(false),
        zoom_steps: Cell::new(0),
        save_due: Cell::new(None),
        backdrop_applied: Cell::new(None),
        pending_opacity: RefCell::new(logic::PendingOpacity::default()),
        syncing_controls: Cell::new(false),
        window_active: Cell::new(true),
        last_alert: Cell::new(None),
        palette: RefCell::new(None),
        find: RefCell::new(None),
        settings_page: RefCell::new(None),
        discovery: RefCell::new(Some(rx)),
        revokers: RefCell::new(Vec::new()),
        rendering: RefCell::new(None),
        dialog: RefCell::new(None),
        profile_menu: RefCell::new(None),
    });
    SHELL.with(|s| *s.borrow_mut() = Some(shell.clone()));

    window::extend_into_title_bar(&window, &ui(&shell.ui.drag)?)?;
    if let Err(e) = window::silence_alt_chords(&window) {
        log(&format!("silence_alt_chords: {e}"));
    }
    shell.apply_glass()?;
    shell.set_tooltips();
    shell.attach_bar()?;
    shell.attach_window()?;
    shell.attach_keyboard()?;
    shell.sync_appearance_controls();

    window.SetContent(&root)?;
    let appw = app_window(&window)?;
    appw.MoveAndResize(RectInt32 { x: 120, y: 80, width: 1500, height: 960 })?;
    window.Activate()?;

    let problems = parsed.problems.len() + built.problems.len();
    let loaded_shell = shell.clone();
    let loaded = fe(&root)?.Loaded(cb!("root loaded", |_, _| loaded_shell.on_loaded(problems, load_error.clone())))?;
    shell.revokers.borrow_mut().push(loaded);
    log("shell built");
    Ok(())
}

impl Shell {
    // ---------------------------------------------------------------------------------------------
    // Start-up
    // ---------------------------------------------------------------------------------------------

    fn on_loaded(self: &Rc<Self>, settings_problems: usize, load_error: Option<String>) {
        let scale = self.scale();
        let (left, right) = window::caption_insets(&self.window, scale);
        let _ = self.ui.left_inset.SetWidth(GridLength { value: left, grid_unit_type: GridUnitType::Pixel });
        let _ = self.ui.right_inset.SetWidth(GridLength { value: right, grid_unit_type: GridUnitType::Pixel });
        self.layout_tabs();

        // Discovery has been running since launch; it is normally done by now.
        let found = self
            .discovery
            .borrow_mut()
            .take()
            .and_then(|rx| rx.recv_timeout(Duration::from_secs(6)).ok())
            .unwrap_or_default();
        self.set_profiles(&found);

        if let Some(error) = load_error {
            self.show_message(&tf(Text::StatusSettingsSaveFailed, &[&error]), InfoBarSeverity::Warning);
        } else if settings_problems > 0 {
            self.show_message(&tf(Text::StatusSettingsProblems, &[&settings_problems]), InfoBarSeverity::Warning);
        } else if nekominal_pty::profiles::inherited_no_color() {
            self.show_message(t(Text::InfobarNoColor), InfoBarSeverity::Informational);
        } else if nekominal_pty::conpty::backend()
            .is_ok_and(|backend| backend.kind != nekominal_pty::conpty::Backend::BUNDLED)
        {
            self.show_message(t(Text::InfobarWidthModel), InfoBarSeverity::Informational);
        }

        let profile = self.default_profile();
        match profile {
            Some(profile) => {
                if let Err(e) = self.add_tab(&profile) {
                    log(&format!("first tab failed: {e}"));
                    self.show_message(&tf(Text::StatusSpawnFailed, &[&profile.name, &e]), InfoBarSeverity::Error);
                }
            }
            None => self.show_message(t(Text::StatusNoProfile), InfoBarSeverity::Error),
        }

        let this = self.clone();
        let rendering = CompositionTarget::Rendering(cb!("rendering", |_, _| this.tick()));
        match rendering {
            Ok(revoker) => *self.rendering.borrow_mut() = Some(revoker),
            Err(e) => log(&format!("CompositionTarget.Rendering: {e}")),
        }
    }

    fn scale(&self) -> f64 {
        self.root.xaml_root().and_then(|root| root.RasterizationScale()).unwrap_or(1.0)
    }

    pub fn lang(&self) -> Lang {
        i18n::lang()
    }

    fn set_profiles(self: &Rc<Self>, found: &[nekominal_pty::profiles::Profile]) {
        let discovered: Vec<DiscoveredProfile> = found.iter().map(discovered).collect();
        *self.profiles.borrow_mut() = profiles::merge(&discovered, &self.settings.borrow().profiles);
        if let Err(e) = self.build_profile_menu() {
            log(&format!("profile menu: {e}"));
        }
    }

    fn default_profile(&self) -> Option<Profile> {
        profiles::resolve_default(&self.profiles.borrow(), &self.settings.borrow().default_profile).cloned()
    }

    fn profile_by_id(&self, id: &str) -> Option<Profile> {
        profiles::find(&self.profiles.borrow(), id).cloned()
    }

    // ---------------------------------------------------------------------------------------------
    // Glass: backdrop, colours, opacity
    // ---------------------------------------------------------------------------------------------

    fn palette_brush(&self, key: &str) -> Result<SolidColorBrush> {
        let map: windows_collections::IMap<windows_core::IInspectable, windows_core::IInspectable> =
            self.resources.cast()?;
        map.Lookup(&IReference::from(key).cast::<windows_core::IInspectable>()?)?.cast()
    }

    /// Recolours the whole chrome from the colour scheme and applies backdrop and opacity.
    pub fn apply_glass(self: &Rc<Self>) -> Result<()> {
        let (scheme_name, backdrop, setting_opacity) = {
            let s = self.settings.borrow();
            (s.theme.clone(), s.appearance.backdrop, s.appearance.opacity)
        };
        let scheme = themes::scheme(&scheme_name);
        let glass = Glass::from_scheme(scheme);
        self.glass.set(glass);
        for (key, color) in [
            ("NkText", glass.text()),
            ("NkTextSecondary", glass.text_secondary()),
            ("NkHover", glass.hover()),
            ("NkPressed", glass.pressed()),
            ("NkCluster", glass.cluster()),
            ("NkDivider", glass.divider()),
            ("NkAccent", glass.accent),
            ("NkInk", themes::Color { a: 255, ..glass.background }),
            ("NkPanel", glass.panel()),
        ] {
            self.palette_brush(key)?.SetColor(xaml_color(color))?;
        }
        if self.backdrop_applied.get() != Some(backdrop) {
            window::set_system_backdrop(&self.window, backdrop)?;
            self.backdrop_applied.set(Some(backdrop));
        }
        window::apply_root_background(&self.root, backdrop, &glass)?;
        window::apply_theme(&self.window, &self.root, &glass)?;
        let opacity = glass::terminal_opacity(backdrop, setting_opacity);
        self.apply_opacity(scheme, &glass, opacity)?;
        for tab in self.tabs.borrow().iter() {
            tab.set_glass(glass);
        }
        self.restyle_strip_fades(&glass)?;
        if let Some(page) = self.settings_page.borrow().as_ref() {
            page.restyle(&glass);
        }
        log(&format!(
            "glass: scheme '{}' light={} backdrop={backdrop:?} opacity={opacity:.2} bar tint alpha {:.2}",
            scheme.name,
            glass.light,
            glass::bar_tint_alpha(opacity)
        ));
        Ok(())
    }

    /// What follows the opacity alone: the tint of the bar and the background of the terminals.
    fn apply_opacity(&self, scheme: &themes::Scheme, glass: &Glass, opacity: f32) -> Result<()> {
        let tint = themes::Color { a: (glass::bar_tint_alpha(opacity) * 255.0).round() as u8, ..glass.background };
        self.ui.bar_tint.SetBackground(&solid_brush(tint)?)?;
        *self.look.theme.borrow_mut() = glass::render_theme(scheme, opacity);
        for tab in self.tabs.borrow().iter() {
            for pane in tab.panes.borrow().iter() {
                pane.apply_theme();
            }
        }
        Ok(())
    }

    /// Settings changed from somewhere: apply them and schedule the write.
    pub fn settings_changed(self: &Rc<Self>) {
        if let Err(e) = self.apply_glass() {
            log(&format!("apply_glass: {e}"));
        }
        self.sync_appearance_controls();
        self.save_due.set(Some(Instant::now() + SAVE_DELAY));
    }

    pub fn set_backdrop(self: &Rc<Self>, backdrop: Backdrop) {
        self.settings.borrow_mut().appearance.backdrop = backdrop;
        self.settings_changed();
    }

    /// Asks for a background opacity; the next frame applies it, however many requests a drag makes before that.
    pub fn set_opacity(&self, opacity: f64) {
        // Without a backdrop nothing shows through: the slider is off and the saved value stays.
        if self.settings.borrow().appearance.backdrop != Backdrop::None {
            self.pending_opacity.borrow_mut().request(opacity);
        }
    }

    /// The opacity the controls show: the latest request, or the one in effect.
    pub fn opacity_latest(&self) -> f64 {
        self.pending_opacity.borrow().latest(self.settings.borrow().appearance.opacity)
    }

    /// Applies the opacity asked for since the last frame, once. A failure while dragging is only logged; the
    /// next change applies again.
    fn apply_pending_opacity(self: &Rc<Self>) {
        let applied = self.settings.borrow().appearance.opacity;
        let Some(opacity) = self.pending_opacity.borrow_mut().take(applied) else { return };
        let (scheme_name, backdrop) = {
            let mut s = self.settings.borrow_mut();
            s.appearance.opacity = opacity;
            (s.theme.clone(), s.appearance.backdrop)
        };
        let effective = glass::terminal_opacity(backdrop, opacity);
        if let Err(e) = self.apply_opacity(themes::scheme(&scheme_name), &self.glass.get(), effective) {
            log(&format!("apply_opacity: {e}"));
        }
        self.sync_appearance_controls();
        self.save_due.set(Some(Instant::now() + SAVE_DELAY));
        log(&format!("opacity applied {opacity:.2}"));
    }

    pub fn set_theme(self: &Rc<Self>, name: &str) {
        if !themes::is_known(name) {
            return;
        }
        self.settings.borrow_mut().theme = name.to_owned();
        self.settings_changed();
    }

    pub fn set_font_size(self: &Rc<Self>, size: f64) {
        self.settings.borrow_mut().font.size = size.clamp(settings::FONT_SIZE_RANGE.0, settings::FONT_SIZE_RANGE.1);
        self.zoom_steps.set(0);
        self.apply_font();
        self.save_due.set(Some(Instant::now() + SAVE_DELAY));
    }

    fn apply_font(&self) {
        let size = self.settings.borrow().font.size_after_zoom(self.zoom_steps.get());
        self.look.font_pt.set(size as f32);
        for tab in self.tabs.borrow().iter() {
            for pane in tab.panes.borrow().iter() {
                pane.apply_look(false);
            }
        }
    }

    /// Brings the quick appearance flyout in line with the settings (without feeding the change back).
    pub fn sync_appearance_controls(&self) {
        let backdrop = self.settings.borrow().appearance.backdrop;
        let (percent, enabled) = glass::opacity_control(backdrop, self.opacity_latest());
        self.syncing_controls.set(true);
        if self.ui.backdrop_choice.SelectedIndex().ok() != Some(backdrop.index()) {
            let _ = self.ui.backdrop_choice.SetSelectedIndex(backdrop.index());
        }
        if let Ok(range) = self.ui.opacity_slider.cast::<RangeBase>()
            && (range.Value().unwrap_or(0.0) - percent).abs() > 0.4
        {
            let _ = range.SetValue(percent);
        }
        let _ = self.ui.opacity_slider.cast::<Control>().and_then(|slider| slider.SetIsEnabled(enabled));
        let _ = self.ui.opacity_value.SetText(&format!("{percent:.0}%"));
        self.syncing_controls.set(false);
        if let Some(page) = self.settings_page.borrow().as_ref() {
            page.sync();
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Window, bar and tool wiring
    // ---------------------------------------------------------------------------------------------

    fn set_tooltips(&self) {
        let keymap = self.keymap.borrow();
        let lang = self.lang();
        let tip = |button: &Button, text: String| {
            let _ = ToolTipService::SetToolTip(button, &IReference::from(text.as_str()));
            let _ = AutomationProperties::SetName(button, &text);
        };
        tip(&self.ui.new_tab, keymap.tooltip(Command::TabNew, lang));
        tip(&self.ui.new_tab_menu, keymap.tooltip(Command::TabProfiles, lang));
        tip(&self.ui.tab_plate_button, t(Text::TabAll).to_owned());
        tip(&self.ui.tool_split_right, keymap.tooltip(Command::PaneSplitRight, lang));
        tip(&self.ui.tool_split_down, keymap.tooltip(Command::PaneSplitDown, lang));
        tip(&self.ui.tool_search, keymap.tooltip(Command::EditFind, lang));
        tip(&self.ui.tool_palette, keymap.tooltip(Command::AppPalette, lang));
        tip(&self.ui.tool_appearance, t(Text::SettingsAppearance).to_owned());
        tip(&self.ui.tool_settings, keymap.tooltip(Command::AppSettings, lang));
    }

    fn attach_bar(self: &Rc<Self>) -> Result<()> {
        let mut revokers = Vec::new();
        let buttons: [(&Button, Command); 6] = [
            (&self.ui.new_tab, Command::TabNew),
            (&self.ui.tool_split_right, Command::PaneSplitRight),
            (&self.ui.tool_split_down, Command::PaneSplitDown),
            (&self.ui.tool_search, Command::EditFind),
            (&self.ui.tool_palette, Command::AppPalette),
            (&self.ui.tool_settings, Command::AppSettings),
        ];
        for (button, command) in buttons {
            let this = self.clone();
            revokers.push(button.on_click(cb!("tool button", |_, _| this.run(command)))?);
        }
        let this = self.clone();
        revokers.push(self.ui.new_tab_menu.on_click(cb!("profile menu", |_, _| this.show_profile_menu()))?);
        let this = self.clone();
        revokers.push(self.ui.tab_plate_button.on_click(cb!("tab list", |_, _| this.toggle_overlay(Overlay::Tabs)))?);

        // Quick appearance flyout: changes apply while the control moves.
        let this = self.clone();
        revokers.push(self.ui.backdrop_choice.SelectionChanged(cb!("backdrop choice", |_, _| {
            // While the selection moves from one choice to the next the list reports no selection: not a choice.
            let chosen = Backdrop::from_index(this.ui.backdrop_choice.SelectedIndex().unwrap_or(-1));
            if let Some(backdrop) = chosen
                && !this.syncing_controls.get()
                && backdrop != this.settings.borrow().appearance.backdrop
            {
                this.set_backdrop(backdrop);
            }
        }))?);
        let this = self.clone();
        revokers.push(self.ui.opacity_slider.cast::<RangeBase>()?.ValueChanged(cb!("opacity slider", |_, args| {
            let Some(value) = args.as_ref().and_then(|a| a.NewValue().ok()) else { return };
            if !this.syncing_controls.get() {
                this.set_opacity(value / 100.0);
            }
        }))?);
        let this = self.clone();
        revokers.push(
            self.ui
                .appearance_flyout
                .cast::<FlyoutBase>()?
                .Closed(cb!("appearance flyout closed", |_, _| this.focus_active_pane()))?,
        );

        // The strip's width follows the bar; the wheel scrolls it sideways.
        let this = self.clone();
        revokers.push(fe(&self.ui.top_bar)?.SizeChanged(cb!("bar size", |_, _| this.layout_tabs()))?);
        let this = self.clone();
        revokers.push(fe(&self.ui.tool_cluster)?.SizeChanged(cb!("cluster size", |_, _| this.layout_tabs()))?);
        let this = self.clone();
        revokers.push(self.ui.tab_scroll.ViewChanged(cb!("strip scrolled", |_, _| this.update_strip_edges(None)))?);
        let scroll = self.ui.tab_scroll.clone();
        revokers.push(ui(&self.ui.tab_scroll)?.PointerWheelChanged(cb!("strip wheel", |_, args| {
            let Some(args) = args.as_ref() else { return };
            let delta = args
                .GetCurrentPoint(&scroll)
                .and_then(|p| p.Properties())
                .and_then(|p| p.MouseWheelDelta())
                .unwrap_or(0);
            let _ = scroll.ChangeView(Some(scroll.HorizontalOffset().unwrap_or(0.0) - f64::from(delta)), None, None);
            let _ = args.SetHandled(true);
        }))?);
        self.revokers.borrow_mut().extend(revokers);
        Ok(())
    }

    /// The width the strip and the position plate may share: the bar without the app mark, the new-tab buttons,
    /// the tool cluster, the caption buttons and the smallest drag area.
    fn strip_budget(&self) -> f64 {
        let bar = fe(&self.ui.top_bar).and_then(|b| b.ActualWidth()).unwrap_or(0.0);
        let cluster = fe(&self.ui.tool_cluster).and_then(|b| b.ActualWidth()).unwrap_or(0.0);
        let insets = self.ui.left_inset.ActualWidth().unwrap_or(0.0) + self.ui.right_inset.ActualWidth().unwrap_or(0.0);
        (bar - insets - cluster - BAR_FIXED_WIDTH).max(STRIP_MIN_WIDTH)
    }

    /// Sizes the whole tab strip: the position plate, the width of every chip, the scroll position that keeps the
    /// selected chip in view and the counts at the cut edges. Runs whenever the tabs, their titles, the selection or
    /// the window change.
    fn layout_tabs(&self) {
        let tabs = self.tabs.borrow().clone();
        let selected = self.selected.get();
        let budget = self.strip_budget();
        let strip = (budget - self.update_plate(tabs.len(), selected, budget)).max(STRIP_MIN_WIDTH);
        let _ = fe(&self.ui.tab_scroll).and_then(|scroll| scroll.SetMaxWidth(strip));
        let preferred: Vec<f64> = tabs.iter().map(|tab| tab.chip.natural_width()).collect();
        let widths = logic::tab_widths(&preferred, selected, strip - STRIP_PADDING, CHIP_GAP);
        for (index, (tab, width)) in tabs.iter().zip(&widths).enumerate() {
            tab.chip.set_width(*width);
            tab.chip.set_position(index, tabs.len());
        }
        let extent = logic::strip_extent(&widths, CHIP_GAP, STRIP_PADDING);
        *self.strip_widths.borrow_mut() = widths;
        self.strip_viewport.set(strip.min(extent));
        self.reveal_pending.set(true);
        self.update_strip_edges(None);
    }

    /// Shows `current / total` (or the total alone on a narrow strip) and returns the width the plate takes.
    fn update_plate(&self, count: usize, selected: usize, budget: f64) -> f64 {
        let text = match logic::plate(selected, count, budget) {
            None => {
                let _ = ui(&self.ui.tab_plate).and_then(|plate| plate.SetVisibility(Visibility::Collapsed));
                return 0.0;
            }
            Some(Plate::Position { current, total }) => tf(Text::FindCount, &[&current, &total]),
            Some(Plate::Total(total)) => total.to_string(),
        };
        let _ = self.ui.tab_plate_text.SetText(&text);
        let plate = ui(&self.ui.tab_plate);
        let _ = plate.as_ref().map(|plate| plate.SetVisibility(Visibility::Visible));
        let measured = plate.and_then(|plate| {
            plate.Measure(Size { width: f32::INFINITY, height: f32::INFINITY })?;
            plate.DesiredSize()
        });
        // The plate and the 4 DIP in front of it.
        measured.map_or(0.0, |size| f64::from(size.width)) + 4.0
    }

    /// Scrolls the strip so the selected chip is in view with a margin; returns the offset it scrolls to.
    fn reveal_selected(&self) -> Option<f64> {
        let widths = self.strip_widths.borrow();
        let selected = self.selected.get();
        let width = *widths.get(selected)?;
        let left = STRIP_PADDING / 2.0 + widths[..selected].iter().sum::<f64>() + CHIP_GAP * selected as f64;
        let extent = logic::strip_extent(&widths, CHIP_GAP, STRIP_PADDING);
        let offset = self.ui.tab_scroll.HorizontalOffset().unwrap_or(0.0);
        let viewport = self.strip_viewport.get();
        let target = logic::reveal_offset((left, width), offset, viewport, extent, REVEAL_MARGIN, COUNT_BLOCK_WIDTH);
        if (target - offset).abs() > 0.5 {
            // The viewer must know the new extent before it can scroll that far.
            let _ = ui(&self.ui.tab_scroll).and_then(|scroll| scroll.UpdateLayout());
            let _ = self.ui.tab_scroll.ChangeViewWithOptionalAnimation(Some(target), None, None, true);
            log(&format!("strip scrolled from {offset:.0} to {target:.0} for tab {}", selected + 1));
        }
        Some(target)
    }

    /// Shows how many chips are hidden at each cut edge. `offset` is where a layout is about to scroll the strip
    /// to; without it the viewer's own position is read (the user scrolled).
    fn update_strip_edges(&self, offset: Option<f64>) {
        let widths = self.strip_widths.borrow();
        let offset = offset.unwrap_or_else(|| self.ui.tab_scroll.HorizontalOffset().unwrap_or(0.0));
        let (left, right) = logic::hidden_counts(&widths, CHIP_GAP, STRIP_PADDING, offset, self.strip_viewport.get());
        // Only the chips in the visible part of the strip spin.
        let tabs = self.tabs.borrow();
        for (index, tab) in tabs.iter().enumerate() {
            tab.chip.set_in_view(index >= left && index + right < tabs.len());
        }
        for (count, glyph_first, block, text, fade) in [
            (left, true, &self.ui.more_left, &self.ui.more_left_text, &self.ui.fade_left),
            (right, false, &self.ui.more_right, &self.ui.more_right_text, &self.ui.fade_right),
        ] {
            let label = if glyph_first { format!("‹ {count}") } else { format!("{count} ›") };
            let _ = text.SetText(&label);
            let _ = ui(block).and_then(|block| block.SetVisibility(visibility(count > 0)));
            let _ = ui(fade).and_then(|fade| fade.SetVisibility(visibility(count > 0)));
        }
    }

    /// The fades at the cut edges of the strip go from the colour of the count blocks to clear.
    fn restyle_strip_fades(&self, glass: &Glass) -> Result<()> {
        let solid = hex_color(glass.panel());
        let clear = hex_color(themes::Color { a: 0, ..glass.panel() });
        for (fade, start, end) in [(&self.ui.fade_left, "0,0.5", "1,0.5"), (&self.ui.fade_right, "1,0.5", "0,0.5")] {
            let brush: Brush = XamlReader::Load(&format!(
                r#"<LinearGradientBrush xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" StartPoint="{start}" EndPoint="{end}">
                     <GradientStop Color="{solid}" Offset="0"/><GradientStop Color="{clear}" Offset="1"/>
                   </LinearGradientBrush>"#
            ))?
            .cast()?;
            fade.SetBackground(&brush)?;
        }
        Ok(())
    }

    fn attach_window(self: &Rc<Self>) -> Result<()> {
        let mut revokers = Vec::new();
        let this = self.clone();
        revokers.push(self.window.Closed(cb!("window closed", |_, _| this.on_closed()))?);
        let this = self.clone();
        revokers.push(self.window.Activated(cb!("window activated", |_, args| {
            let active =
                args.as_ref().and_then(|a| a.WindowActivationState().ok()) != Some(WindowActivationState::Deactivated);
            this.window_active.set(active);
            log(&format!("window active = {active}"));
            if active {
                // Coming back, the user sees the tab that was selected.
                if let Some(tab) = this.selected_tab() {
                    this.mark_seen(&tab);
                }
            } else {
                this.mru.borrow_mut().commit();
                // Losing the focus ends a drag: apply the last opacity and write the settings now.
                this.apply_pending_opacity();
                this.flush_settings();
            }
        }))?);
        self.revokers.borrow_mut().extend(revokers);
        Ok(())
    }

    /// Every session ends here, while the process can still start the threads that close them (critique.md C14).
    fn on_closed(self: &Rc<Self>) {
        log("window closed: ending every session");
        if let Err(e) = window::restore_alt_chords(&self.window) {
            log(&format!("restore_alt_chords: {e}"));
        }
        self.apply_pending_opacity();
        self.flush_settings();
        *self.rendering.borrow_mut() = None;
        for tab in self.tabs.borrow().iter() {
            for pane in tab.panes.borrow().iter() {
                pane.shutdown();
            }
        }
    }

    fn flush_settings(&self) {
        if self.save_due.take().is_none() {
            return;
        }
        let Some(path) = store::settings_path() else { return };
        match store::save(&path, &settings::serialize(&self.settings.borrow())) {
            Ok(()) => log("settings saved"),
            Err(e) => log(&format!("settings save failed: {e}")),
        }
    }

    // ---------------------------------------------------------------------------------------------
    // The render tick
    // ---------------------------------------------------------------------------------------------

    fn tick(self: &Rc<Self>) {
        self.apply_pending_opacity();
        if self.reveal_pending.take() {
            let offset = self.reveal_selected();
            self.update_strip_edges(offset);
        }
        if self.save_due.get().is_some_and(|due| Instant::now() >= due) {
            self.flush_settings();
        }
        let tabs: Vec<Rc<Tab>> = self.tabs.borrow().clone();
        let mut closed_panes = Vec::new();
        for tab in &tabs {
            for pane in tab.panes.borrow().iter() {
                for event in pane.poll_events() {
                    self.on_pane_event(tab, pane, event, &mut closed_panes);
                }
            }
            for pane in tab.drawn_panes() {
                pane.tick();
            }
        }
        for (tab_id, pane_id) in closed_panes {
            self.close_pane_in(tab_id, pane_id);
        }
    }

    fn on_pane_event(&self, tab: &Rc<Tab>, pane: &Rc<PaneView>, event: UiEvent, closed: &mut Vec<(TabId, PaneId)>) {
        match event {
            UiEvent::Title(title) => {
                *pane.title.borrow_mut() = logic::clean_title(&title);
                self.refresh_titles();
            }
            UiEvent::Cwd(path) => {
                *pane.cwd.borrow_mut() = Some(path.into());
                self.refresh_titles();
            }
            UiEvent::Bell => self.on_bell(tab, pane),
            UiEvent::Exit(code) => {
                pane.exited.set(Some(code));
                let closes =
                    self.profile_by_id(&pane.profile_id).is_none_or(|profile| profile.kind.closes_pane_on_clean_exit());
                if code == 0 && closes {
                    closed.push((tab.id, pane.id));
                } else if code == 0 {
                    // A program profile (an agent): its last screen is what the session was for.
                    let line = format!("\r\n{}\r\n", tf(Text::StatusExited, &[&code]));
                    pane.pane.term.lock().feed(line.as_bytes());
                } else {
                    pane.attention.set(Some(Attention::Exit));
                    let line = format!("\r\n{}\r\n", tf(Text::StatusExited, &[&code]));
                    pane.pane.term.lock().feed(line.as_bytes());
                }
                self.refresh_status(tab);
            }
            UiEvent::Agent(_) | UiEvent::AgentRecordsDropped => {
                // What a report says is not latched: the chip shows what the emulator holds now. A report in the
                // pane the user is looking at is seen at once.
                if self.in_view(tab, pane) {
                    pane.pane.dismiss_finished();
                }
                self.refresh_status(tab);
                self.refresh_titles();
            }
            UiEvent::Notification(notification) => {
                if !self.in_view(tab, pane) {
                    pane.attention.set(Some(Attention::Notification));
                    let text = [notification.title.as_str(), notification.body.as_str()];
                    *pane.note.borrow_mut() =
                        Some(text.iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join(": "));
                    self.refresh_status(tab);
                }
            }
            UiEvent::Progress(progress) => {
                pane.progress.set(progress.map(|progress| match progress {
                    Progress::Value { value, .. } => Reported::Percent(value.round().clamp(0.0, 100.0) as u8),
                    Progress::Indeterminate => Reported::Indeterminate,
                }));
                self.refresh_status(tab);
            }
            UiEvent::PromptMark(_) | UiEvent::Other(_) => {}
        }
    }

    /// Gives every tab the title its rules make (`logic::default_title`): what a program or an agent set in the
    /// focused pane unless that is generic, else the working directory, else the profile; the parent folder is
    /// added where two tabs would otherwise be named alike. A tab the user named keeps its name.
    fn refresh_titles(&self) {
        let tabs = self.tabs.borrow().clone();
        let home = std::env::var("USERPROFILE").ok();
        let parts: Vec<TitleParts> = tabs
            .iter()
            .map(|tab| {
                let pane = tab.focused_pane();
                let record = pane.as_ref().and_then(|pane| pane.pane.agent_summary());
                TitleParts {
                    user: tab.renamed.get().then(|| tab.title.borrow().clone()),
                    program: pane.as_ref().map(|pane| pane.title.borrow().clone()).unwrap_or_default(),
                    agent_title: record.as_ref().and_then(|record| record.title.clone()),
                    agent_app: record.as_ref().and_then(|record| record.app.clone()),
                    cwd: pane
                        .as_ref()
                        .and_then(|pane| pane.cwd.borrow().as_ref().map(|cwd| cwd.to_string_lossy().into_owned())),
                    profile: pane
                        .as_ref()
                        .and_then(|pane| self.profile_by_id(&pane.profile_id))
                        .map(|profile| profile.name)
                        .unwrap_or_default(),
                    shell: pane.as_ref().map(|pane| pane.shell_name.clone()).unwrap_or_default(),
                    home: home.clone(),
                    untitled: t(Text::TabUntitled).to_owned(),
                }
            })
            .collect();
        let first: Vec<(String, TitleSource)> = parts.iter().map(|parts| logic::default_title(parts, false)).collect();
        let folders: Vec<Option<String>> =
            first.iter().map(|(title, source)| (*source == TitleSource::Folder).then(|| title.clone())).collect();
        let collide = logic::colliding_folders(&folders);
        for (((tab, parts), (title, _)), with_parent) in tabs.iter().zip(&parts).zip(first).zip(collide) {
            if tab.renamed.get() {
                continue;
            }
            let title = if with_parent { logic::default_title(parts, true).0 } else { title };
            if *tab.title.borrow() != title {
                *tab.title.borrow_mut() = title.clone();
                tab.chip.set_title(&title);
            }
        }
        self.layout_tabs();
    }

    /// The pane is on screen, has the focus and the window is active: the user is looking at it.
    fn in_view(&self, tab: &Rc<Tab>, pane: &Rc<PaneView>) -> bool {
        self.window_active.get() && self.pane_shown(tab, pane)
    }

    /// The pane is on screen and has the focus (its tab is selected, no zoomed sibling hides it).
    fn pane_shown(&self, tab: &Rc<Tab>, pane: &Rc<PaneView>) -> bool {
        tab.is_visible() && tab.focused.get() == pane.id && tab.zoomed.get().is_none_or(|zoomed| zoomed == pane.id)
    }

    /// A bell: the settings and the place it rang decide what happens (`bell_effects`).
    fn on_bell(&self, tab: &Rc<Tab>, pane: &Rc<PaneView>) {
        let context = BellContext {
            window_active: self.window_active.get(),
            pane_in_view: self.pane_shown(tab, pane),
            since_last_alert: self.last_alert.get().map(|at| at.elapsed()),
            show_sounds: window::show_sounds(),
        };
        // No profile carries bell settings of its own yet.
        let effects = bell_effects(&self.settings.borrow().bell, None, &context);
        if effects.chip {
            pane.attention.set(Some(Attention::Bell));
            self.refresh_status(tab);
        }
        if effects.taskbar_flash
            && let Err(e) = window::flash_taskbar(&self.window)
        {
            log(&format!("taskbar flash: {e}"));
        }
        if effects.sound {
            crate::sound::bell();
        }
        if effects.taskbar_flash || effects.sound {
            self.last_alert.set(Some(Instant::now()));
        }
        log(&format!("bell in pane {}: {effects:?}", pane.id));
    }

    /// What the tab shows: that of its most urgent pane, read from the live state of every pane.
    fn refresh_status(&self, tab: &Rc<Tab>) {
        let signals: Vec<PaneSignal> = tab
            .panes
            .borrow()
            .iter()
            .map(|pane| PaneSignal {
                agent: pane.pane.agent_summary(),
                progress: pane.progress.get(),
                attention: pane.attention.get(),
                note: pane.note.borrow().clone(),
            })
            .collect();
        let status = logic::tab_status(&signals);
        if tab.chip.state() != status.state {
            log(&format!("tab {} status {:?}", tab.id, status.state));
        }
        tab.chip.set_status(status);
    }

    /// The user has seen the tab: bells, notifications and what a finished or failed agent reported are
    /// acknowledged. What still waits (a blocked or working agent) stays.
    fn mark_seen(&self, tab: &Rc<Tab>) {
        for pane in tab.panes.borrow().iter() {
            pane.attention.set(None);
            *pane.note.borrow_mut() = None;
            pane.pane.dismiss_finished();
        }
        self.refresh_status(tab);
    }

    // ---------------------------------------------------------------------------------------------
    // Tabs and panes
    // ---------------------------------------------------------------------------------------------

    fn find_pane(&self, id: PaneId) -> Option<Rc<PaneView>> {
        self.tabs.borrow().iter().find_map(|tab| tab.pane(id))
    }

    fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.borrow().iter().position(|tab| tab.id == id)
    }

    /// The open tabs in strip order, for the "go to tab" entries of the palette.
    pub fn palette_tabs(&self) -> Vec<nekominal_model::palette::PaletteTab> {
        let tabs = self.tabs.borrow();
        tabs.iter()
            .map(|tab| nekominal_model::palette::PaletteTab { id: tab.id, title: tab.title.borrow().clone() })
            .collect()
    }

    /// One row per tab for the list of every tab.
    pub fn tab_rows(&self) -> Vec<TabRow> {
        let tabs = self.tabs.borrow();
        let keymap = self.keymap.borrow();
        let home = std::env::var("USERPROFILE").ok();
        tabs.iter()
            .enumerate()
            .map(|(index, tab)| {
                let title = tab.title.borrow().clone();
                let cwd = tab.focused_pane().and_then(|pane| pane.cwd.borrow().clone());
                let folder =
                    cwd.map(|cwd| logic::folder_label(&cwd.to_string_lossy(), home.as_deref())).unwrap_or_default();
                TabRow {
                    id: tab.id,
                    ordinal: index + 1,
                    search: format!("{title} {folder}"),
                    title,
                    folder,
                    panes: tab.pane_count(),
                    state: tab.chip.state(),
                    shortcut: Command::tab_goto_number(index, tabs.len())
                        .map(|number| keymap.shortcut_text(Command::TabGoto(number)))
                        .unwrap_or_default(),
                }
            })
            .collect()
    }

    pub fn selected_tab(&self) -> Option<Rc<Tab>> {
        self.tabs.borrow().get(self.selected.get()).cloned()
    }

    /// The session of a profile: its program and arguments, directory and environment.
    fn spawn_config(&self, profile: &Profile, cwd: Option<&std::path::Path>) -> SpawnConfig {
        let settings = self.settings.borrow();
        let mut env = nekominal_pty::profiles::default_environment();
        if settings.agent_environment {
            env.extend(nekominal_pty::profiles::agent_environment());
        }
        let cwd = cwd
            .or(profile.cwd.as_deref().map(std::path::Path::new))
            .filter(|dir| dir.is_dir())
            .map(std::path::Path::to_path_buf);
        let program = nekominal_pty::profiles::resolve_program(&profile.program)
            .unwrap_or_else(|| profile.program.clone().into());
        SpawnConfig {
            program,
            args: profiles::launch_args(profile, settings.shell_integration),
            cwd,
            env,
            cols: 80,
            rows: 24,
            width_model: WidthModel::default(),
        }
    }

    fn new_pane(self: &Rc<Self>, profile: &Profile, cwd: Option<&std::path::Path>) -> Result<Rc<PaneView>> {
        let id = self.next_pane.get();
        self.next_pane.set(id + 1);
        let weak = Rc::downgrade(self);
        let on_focus: Rc<dyn Fn(PaneId, bool)> = Rc::new(move |pane, focused| {
            if let Some(shell) = weak.upgrade() {
                shell.on_pane_focus(pane, focused);
            }
        });
        let config = self.spawn_config(profile, cwd);
        PaneView::new(id, config, profile.id.clone(), self.look.clone(), &self.window, &self.root, on_focus)
    }

    fn on_pane_focus(&self, pane: PaneId, focused: bool) {
        self.terminal_focus.set(focused);
        if !focused {
            return;
        }
        let tabs = self.tabs.borrow().clone();
        if let Some(tab) = tabs.iter().find(|tab| tab.pane(pane).is_some()) {
            if tab.focused.get() != pane {
                tab.focused.set(pane);
                tab.update_focus_look();
                tab.relayout();
            }
            self.refresh_titles();
            self.mark_seen(tab);
        }
    }

    pub fn add_tab(self: &Rc<Self>, profile: &Profile) -> Result<Rc<Tab>> {
        let cwd = self.selected_tab().and_then(|tab| tab.focused_pane()).and_then(|pane| pane.cwd.borrow().clone());
        let pane = self.new_pane(profile, cwd.as_deref())?;
        let id = self.next_tab.get();
        self.next_tab.set(id + 1);
        // The rules of `refresh_titles` give it its real title once it is in the strip.
        let title = profile.name.clone();
        let weak = Rc::downgrade(self);
        let on_event: Rc<dyn Fn(TabEvent)> = Rc::new(move |event| {
            if let Some(shell) = weak.upgrade() {
                shell.on_tab_event(event);
            }
        });
        let chip = Chip::new(id, &title, self.glass.get(), &self.ui.tab_strip, on_event)?;
        let tab = Tab::new(id, chip.clone(), pane, title, self.glass.get())?;
        self.ui.tab_strip.cast::<Panel>()?.Children()?.Append(&chip.root)?;
        self.ui.terminal_area.cast::<Panel>()?.Children()?.Append(&tab.area)?;
        ui(&tab.area)?.SetVisibility(Visibility::Collapsed)?;
        self.attach_chip_menu(&tab)?;
        self.tabs.borrow_mut().push(tab.clone());
        self.refresh_titles();
        self.select_index(self.tabs.borrow().len() - 1);
        Ok(tab)
    }

    fn on_tab_event(self: &Rc<Self>, event: TabEvent) {
        match event {
            TabEvent::Select(id) => {
                if let Some(index) = self.tab_index(id) {
                    self.select_index(index);
                }
            }
            TabEvent::Close(id) => self.request_close_tab(id),
            TabEvent::Move { id, to } => self.move_tab(id, to),
        }
    }

    /// Selects a tab by a click, the palette, a number or a command: it becomes the most recently used.
    pub fn select_index(self: &Rc<Self>, index: usize) {
        if let Some(tab) = self.tabs.borrow().get(index) {
            self.mru.borrow_mut().touch(tab.id);
        }
        self.show_index(index);
    }

    /// Shows the tab at `index` without touching the order of use (the next/previous keys do that when they come up).
    fn show_index(self: &Rc<Self>, index: usize) {
        let tabs = self.tabs.borrow().clone();
        let Some(selected) = tabs.get(index) else { return };
        self.selected.set(index);
        for tab in &tabs {
            let is_selected = Rc::ptr_eq(tab, selected);
            tab.chip.set_selected(is_selected);
            tab.set_visible(is_selected);
        }
        if self.window_active.get() {
            self.mark_seen(selected);
        }
        selected.update_focus_look();
        self.layout_tabs();
        selected.relayout();
        if self.overlay.get() == Overlay::None {
            self.focus_active_pane();
        }
        log(&format!("selected tab {} ('{}')", index + 1, selected.title.borrow()));
    }

    pub fn focus_active_pane(&self) {
        if self.overlay.get() != Overlay::None {
            return;
        }
        if let Some(pane) = self.selected_tab().and_then(|tab| tab.focused_pane()) {
            pane.focus();
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected.get()
    }

    fn move_tab(self: &Rc<Self>, id: TabId, to: usize) {
        let Some(from) = self.tab_index(id) else { return };
        let to = to.min(self.tabs.borrow().len().saturating_sub(1));
        if from == to {
            return;
        }
        let selected_id = self.selected_tab().map(|tab| tab.id);
        let tab = self.tabs.borrow_mut().remove(from);
        self.tabs.borrow_mut().insert(to, tab.clone());
        if let Ok(children) = self.ui.tab_strip.cast::<Panel>().and_then(|p| p.Children()) {
            let _ = children.RemoveAt(from as u32);
            let _ = children.InsertAt(to as u32, &tab.chip.root);
        }
        if let Some(selected_id) = selected_id
            && let Some(index) = self.tab_index(selected_id)
        {
            self.selected.set(index);
        }
        self.layout_tabs();
        self.focus_active_pane();
        log(&format!("tab {from} moved to {to}"));
    }

    /// Closes after asking when running sessions would end.
    pub fn request_close_tab(self: &Rc<Self>, id: TabId) {
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == id).cloned() else { return };
        let running = tab.busy_sessions();
        if self.settings.borrow().should_confirm_close(running) {
            let this = self.clone();
            self.confirm_close(tf(Text::DialogCloseTab, &[&tab.title.borrow()]), move || this.close_tab(id));
        } else {
            self.close_tab(id);
        }
    }

    pub fn close_tab(self: &Rc<Self>, id: TabId) {
        let Some(index) = self.tab_index(id) else { return };
        let current = self.selected_tab().map(|tab| tab.id);
        let tab = self.tabs.borrow_mut().remove(index);
        self.mru.borrow_mut().remove(id);
        if let Ok(children) = self.ui.tab_strip.cast::<Panel>().and_then(|p| p.Children()) {
            let _ = children.RemoveAt(index as u32);
        }
        if let Ok(children) = self.ui.terminal_area.cast::<Panel>().and_then(|p| p.Children()) {
            let mut at = 0u32;
            if let Ok(true) = children.IndexOf(&tab.area, &mut at) {
                let _ = children.RemoveAt(at);
            }
        }
        for pane in tab.panes.borrow().iter() {
            pane.shutdown();
        }
        log(&format!("closed tab '{}', {} left", tab.title.borrow(), self.tabs.borrow().len()));
        let remaining = self.tabs.borrow().len();
        if remaining == 0 {
            let _ = self.window.Close();
            return;
        }
        // The selected tab stays; closing it hands over to the tab used most recently before.
        let next = if current == Some(id) { self.mru.borrow().most_recent() } else { current };
        self.refresh_titles();
        self.show_index(next.and_then(|next| self.tab_index(next)).unwrap_or(0));
    }

    pub fn close_pane_in(self: &Rc<Self>, tab_id: TabId, pane_id: PaneId) {
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == tab_id).cloned() else { return };
        if !tab.remove_pane(pane_id) {
            self.close_tab(tab_id);
            return;
        }
        self.refresh_status(&tab);
        self.refresh_titles();
        if tab.is_visible() {
            self.focus_active_pane();
        }
    }

    fn request_close_pane(self: &Rc<Self>) {
        let Some(tab) = self.selected_tab() else { return };
        let pane_id = tab.focused.get();
        if tab.pane_count() <= 1 {
            self.request_close_tab(tab.id);
            return;
        }
        let running = tab.pane(pane_id).map_or(0, |pane| u32::from(pane.is_busy()));
        if self.settings.borrow().should_confirm_close(running) {
            let this = self.clone();
            let tab_id = tab.id;
            self.confirm_close(tf(Text::DialogCloseSessions, &[&running]), move || this.close_pane_in(tab_id, pane_id));
        } else {
            self.close_pane_in(tab.id, pane_id);
        }
    }

    fn split_focused(self: &Rc<Self>, axis: Axis) -> Result<()> {
        let Some(tab) = self.selected_tab() else { return Ok(()) };
        let Some(source) = tab.focused_pane() else { return Ok(()) };
        let profile = self.profile_by_id(&source.profile_id).or_else(|| self.default_profile());
        let Some(profile) = profile else { return Ok(()) };
        let cwd = source.cwd.borrow().clone();
        let pane = self.new_pane(&profile, cwd.as_deref())?;
        let id = pane.id;
        tab.split(source.id, axis, pane)?;
        if let Some(new) = tab.pane(id) {
            new.focus();
        }
        Ok(())
    }

    fn close_other_tabs(self: &Rc<Self>, keep: TabId, only_right: bool) {
        let ids: Vec<TabId> = {
            let tabs = self.tabs.borrow();
            let keep_index = tabs.iter().position(|tab| tab.id == keep).unwrap_or(0);
            tabs.iter()
                .enumerate()
                .filter(|(i, tab)| tab.id != keep && (!only_right || *i > keep_index))
                .map(|(_, tab)| tab.id)
                .collect()
        };
        for id in ids {
            self.close_tab(id);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Menus
    // ---------------------------------------------------------------------------------------------

    fn menu_item(&self, label: &str, on_click: impl Fn() + 'static) -> Result<MenuFlyoutItem> {
        let item = MenuFlyoutItem::new()?;
        item.SetText(label)?;
        let revoker = item.Click(cb!("menu item", |_, _| on_click()))?;
        self.revokers.borrow_mut().push(revoker);
        Ok(item)
    }

    fn build_profile_menu(self: &Rc<Self>) -> Result<()> {
        let menu = MenuFlyout::new()?;
        let items = menu.Items()?;
        for profile in self.profiles.borrow().iter() {
            let this = self.clone();
            let id = profile.id.clone();
            let item = self.menu_item(&profile.name, move || {
                if let Some(profile) = this.profile_by_id(&id)
                    && let Err(e) = this.add_tab(&profile)
                {
                    this.show_message(&tf(Text::StatusSpawnFailed, &[&profile.name, &e]), InfoBarSeverity::Error);
                }
            })?;
            items.Append(&item)?;
        }
        items.Append(&MenuFlyoutSeparator::new()?)?;
        for command in [Command::AppSettings, Command::AppPalette] {
            let this = self.clone();
            let label = self.keymap.borrow().tooltip(command, self.lang());
            let item = self.menu_item(&label, move || this.run(command))?;
            items.Append(&item)?;
        }
        *self.profile_menu.borrow_mut() = Some(menu);
        Ok(())
    }

    fn show_profile_menu(&self) {
        if let Some(menu) = self.profile_menu.borrow().as_ref() {
            let _ = menu.cast::<FlyoutBase>().and_then(|flyout| flyout.ShowAt(&self.ui.new_tab_menu));
        }
    }

    fn attach_chip_menu(self: &Rc<Self>, tab: &Rc<Tab>) -> Result<()> {
        let flyout = MenuFlyout::new()?;
        let items = flyout.Items()?;
        let id = tab.id;
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabRename), move || this.rename_tab(id))?)?;
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabDuplicate), move || {
            let profile = this
                .tabs
                .borrow()
                .iter()
                .find(|tab| tab.id == id)
                .and_then(|tab| tab.focused_pane())
                .and_then(|pane| this.profile_by_id(&pane.profile_id));
            if let Some(profile) = profile {
                let _ = this.add_tab(&profile);
            }
        })?)?;
        items.Append(&MenuFlyoutSeparator::new()?)?;
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabCloseOthers), move || this.close_other_tabs(id, false))?)?;
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabCloseRight), move || this.close_other_tabs(id, true))?)?;
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabClose), move || this.request_close_tab(id))?)?;
        ui(&tab.chip.root)?.SetContextFlyout(&flyout)?;
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // Messages and dialogs
    // ---------------------------------------------------------------------------------------------

    pub fn show_message(&self, message: &str, severity: InfoBarSeverity) {
        let bar = &self.ui.message_bar;
        let _ = bar.SetSeverity(severity);
        let _ = bar.SetMessage(message);
        let _ = bar.SetIsOpen(true);
    }

    /// Asks before ending work; `title` names what closes, the buttons answer it.
    fn confirm_close(self: &Rc<Self>, title: String, on_confirm: impl Fn() + 'static) {
        let result = (|| -> Result<()> {
            let dialog = ContentDialog::new()?;
            dialog.SetTitle(&IReference::from(title.as_str()))?;
            dialog.SetPrimaryButtonText(t(Text::CommonClose))?;
            dialog.SetCloseButtonText(t(Text::CommonCancel))?;
            dialog.SetDefaultButton(ContentDialogButton::Close)?;
            ui(&dialog)?.SetXamlRoot(&self.root.xaml_root()?)?;
            let revoker = dialog.PrimaryButtonClick(cb!("close confirmed", |_, _| on_confirm()))?;
            self.revokers.borrow_mut().push(revoker);
            let this = self.clone();
            let revoker = dialog.Closed(cb!("dialog closed", |_, _| this.focus_active_pane()))?;
            self.revokers.borrow_mut().push(revoker);
            *self.dialog.borrow_mut() = Some(dialog.ShowAsync()?);
            Ok(())
        })();
        if let Err(e) = result {
            log(&format!("confirm dialog: {e}"));
        }
    }

    fn rename_tab(self: &Rc<Self>, id: TabId) {
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == id).cloned() else { return };
        let result = (|| -> Result<()> {
            let input = TextBox::new()?;
            input.SetText(&tab.title.borrow())?;
            input.SelectAll()?;
            let dialog = ContentDialog::new()?;
            dialog.SetTitle(&IReference::from(t(Text::TabRename)))?;
            dialog.cast::<ContentControl>()?.SetContent(&input)?;
            dialog.SetPrimaryButtonText(t(Text::TabRename))?;
            dialog.SetCloseButtonText(t(Text::CommonCancel))?;
            dialog.SetDefaultButton(ContentDialogButton::Primary)?;
            ui(&dialog)?.SetXamlRoot(&self.root.xaml_root()?)?;
            let tab = tab.clone();
            let this_layout = self.clone();
            let revoker = dialog.PrimaryButtonClick(cb!("rename confirmed", |_, _| {
                let name = logic::clean_title(&input.Text().unwrap_or_default());
                if name.is_empty() {
                    tab.renamed.set(false);
                } else {
                    tab.renamed.set(true);
                    *tab.title.borrow_mut() = name.clone();
                    tab.chip.set_title(&name);
                }
                this_layout.refresh_titles();
            }))?;
            self.revokers.borrow_mut().push(revoker);
            let this = self.clone();
            let revoker = dialog.Closed(cb!("dialog closed", |_, _| this.focus_active_pane()))?;
            self.revokers.borrow_mut().push(revoker);
            *self.dialog.borrow_mut() = Some(dialog.ShowAsync()?);
            Ok(())
        })();
        if let Err(e) = result {
            log(&format!("rename dialog: {e}"));
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Overlays
    // ---------------------------------------------------------------------------------------------

    pub fn set_overlay(self: &Rc<Self>, which: Overlay) {
        let previous = self.overlay.replace(which);
        if previous == which {
            return;
        }
        if let Some(p) = self.palette.borrow().as_ref() {
            p.show(matches!(which, Overlay::Palette | Overlay::Tabs));
        }
        if let Some(f) = self.find.borrow().as_ref() {
            f.show(which == Overlay::Find);
        }
        if let Some(s) = self.settings_page.borrow().as_ref() {
            s.show(which == Overlay::Settings);
        }
        // The settings page takes the place of the terminals; the others float above them.
        let _ = ui(&self.ui.terminal_area).and_then(|area| area.SetVisibility(visibility(which != Overlay::Settings)));
        log(&format!("overlay {previous:?} -> {which:?}"));
        match which {
            Overlay::None => self.focus_active_pane(),
            Overlay::Palette => {
                if let Some(p) = self.palette.borrow().as_ref() {
                    p.open(self);
                }
            }
            Overlay::Tabs => {
                if let Some(p) = self.palette.borrow().as_ref() {
                    p.open_tabs(self);
                }
            }
            Overlay::Find => {
                if let Some(f) = self.find.borrow().as_ref() {
                    f.focus();
                }
            }
            Overlay::Settings => {
                if let Some(s) = self.settings_page.borrow().as_ref() {
                    s.focus();
                }
            }
        }
    }

    fn toggle_overlay(self: &Rc<Self>, which: Overlay) {
        self.ensure_overlays();
        self.set_overlay(if self.overlay.get() == which { Overlay::None } else { which });
    }

    /// The overlays need a live `XamlRoot`, so they are built the first time one opens.
    fn ensure_overlays(self: &Rc<Self>) {
        let build = || -> Result<()> {
            if self.palette.borrow().is_none() {
                let palette = palette::Palette::build(self)?;
                self.ui.body.cast::<Panel>()?.Children()?.Append(&palette.root)?;
                *self.palette.borrow_mut() = Some(palette);
            }
            if self.find.borrow().is_none() {
                let bar = findbar::FindBar::build(self)?;
                self.ui.body.cast::<Panel>()?.Children()?.Append(&bar.root)?;
                *self.find.borrow_mut() = Some(bar);
            }
            if self.settings_page.borrow().is_none() {
                let page = settings_page::SettingsPage::build(self)?;
                self.ui.body.cast::<Panel>()?.Children()?.Append(&page.root)?;
                *self.settings_page.borrow_mut() = Some(page);
            }
            Ok(())
        };
        if let Err(e) = build() {
            log(&format!("overlays: {e}"));
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Commands
    // ---------------------------------------------------------------------------------------------

    pub fn run(self: &Rc<Self>, command: Command) {
        log(&format!("command {}", command.id()));
        let tab = self.selected_tab();
        let result: Result<()> = (|| {
            match command {
                Command::TabNew => {
                    if let Some(profile) = self.default_profile() {
                        self.add_tab(&profile)?;
                    }
                }
                Command::TabClose => {
                    if let Some(tab) = &tab {
                        self.request_close_tab(tab.id);
                    }
                }
                Command::TabNext | Command::TabPrevious => {
                    let step = if command == Command::TabNext { 1 } else { -1 };
                    let shown = self.mru.borrow_mut().step(step);
                    if let Some(index) = shown.and_then(|id| self.tab_index(id)) {
                        self.show_index(index);
                    }
                }
                Command::TabGoto(number) => {
                    if let Some(index) = Command::tab_goto_index(number, self.tabs.borrow().len()) {
                        self.select_index(index);
                    }
                }
                Command::TabMoveLeft | Command::TabMoveRight => {
                    if let Some(tab) = &tab {
                        let index = self.selected.get();
                        let to = if command == Command::TabMoveLeft { index.saturating_sub(1) } else { index + 1 };
                        self.move_tab(tab.id, to);
                    }
                }
                Command::TabProfiles => self.show_profile_menu(),
                Command::PaneSplitRight => self.split_focused(Axis::Row)?,
                Command::PaneSplitDown => self.split_focused(Axis::Column)?,
                Command::PaneClose => self.request_close_pane(),
                Command::PaneZoom => {
                    if let Some(tab) = &tab {
                        tab.toggle_zoom();
                        self.focus_active_pane();
                    }
                }
                Command::PaneFocus(direction) => {
                    if let Some(tab) = &tab
                        && let Some(id) = tab.focus_direction(direction)
                        && let Some(pane) = tab.pane(id)
                    {
                        pane.focus();
                        tab.relayout();
                    }
                }
                Command::PaneResize(direction) => {
                    if let Some(tab) = &tab {
                        tab.resize_focused(direction);
                    }
                }
                Command::EditPaste => {
                    if let (Some(text), Some(pane)) =
                        (clipboard::read_text(), tab.as_ref().and_then(|t| t.focused_pane()))
                    {
                        pane.pane.paste(&text);
                    }
                }
                Command::EditFind => self.toggle_overlay(Overlay::Find),
                Command::AppPalette => self.toggle_overlay(Overlay::Palette),
                Command::AppSettings => self.toggle_overlay(Overlay::Settings),
                Command::ViewFullscreen => {
                    let appw = app_window(&self.window)?;
                    let fullscreen = appw.Presenter()?.Kind()? == AppWindowPresenterKind::FullScreen;
                    appw.SetPresenterByKind(if fullscreen {
                        AppWindowPresenterKind::Default
                    } else {
                        AppWindowPresenterKind::FullScreen
                    })?;
                }
                Command::ViewZoomIn | Command::ViewZoomOut | Command::ViewZoomReset => {
                    let steps = match command {
                        Command::ViewZoomIn => self.zoom_steps.get() + 1,
                        Command::ViewZoomOut => self.zoom_steps.get() - 1,
                        _ => 0,
                    };
                    self.zoom_steps.set(steps);
                    self.apply_font();
                }
                Command::AppearanceBackdropNext => {
                    let next = self.settings.borrow().appearance.backdrop.next();
                    self.set_backdrop(next);
                }
                Command::AppearanceOpacityUp | Command::AppearanceOpacityDown => {
                    let steps = if command == Command::AppearanceOpacityUp { 1 } else { -1 };
                    // A step starts from what is in effect, not from a request that is still waiting.
                    self.apply_pending_opacity();
                    let opacity = self.settings.borrow().appearance.opacity_after(steps);
                    self.set_opacity(opacity);
                }
                // Selection, scrollback navigation, prompt marks, restart and the settings file belong to
                // the terminal features that are not wired yet.
                Command::EditCopy
                | Command::EditSmartCopy
                | Command::EditSelectAll
                | Command::EditClear
                | Command::ScrollPageUp
                | Command::ScrollPageDown
                | Command::ScrollTop
                | Command::ScrollBottom
                | Command::PromptPrevious
                | Command::PromptNext
                | Command::PaneRestart
                | Command::AppOpenSettingsFile
                | Command::AppReloadSettings => log(&format!("command {} is not wired yet", command.id())),
            }
            Ok(())
        })();
        if let Err(e) = result {
            log(&format!("command {} failed: {e}", command.id()));
        }
    }

    pub fn run_palette_action(self: &Rc<Self>, action: nekominal_model::palette::PaletteAction) {
        use nekominal_model::palette::PaletteAction;
        match action {
            PaletteAction::Run(command) => self.run(command),
            PaletteAction::GotoTab(id) => {
                if let Some(index) = self.tab_index(id) {
                    self.select_index(index);
                }
            }
            PaletteAction::NewTab(id) => {
                if let Some(profile) = self.profile_by_id(&id) {
                    let _ = self.add_tab(&profile);
                }
            }
            PaletteAction::SetColorScheme(name) => self.set_theme(&name),
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Keyboard layer
    // ---------------------------------------------------------------------------------------------

    /// `PreviewKeyDown` of the window content tunnels first: shortcuts are matched by physical key.
    fn attach_keyboard(self: &Rc<Self>) -> Result<()> {
        let this = self.clone();
        let revoker = ui(&self.root)?.PreviewKeyDown(cb!("shortcut", |_, args| {
            let Some(args) = args.as_ref() else { return };
            let code = args.Key().map_or(0, |key| key.0);
            let status = args.KeyStatus().unwrap_or_default();
            let held = keyboard::held_modifiers();
            let physical = keyboard::physical_code(code, &status);
            let command = physical.and_then(|physical| this.keymap.borrow().find(held, physical));
            let Some(command) = command else {
                // Bare modifier presses are not chords; everything else that matched nothing is worth a line.
                let is_modifier = matches!(code, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
                if (held.ctrl || held.alt) && !is_modifier {
                    log(&format!(
                        "chord vk=0x{code:02X} scan=0x{:02X} key={physical:?} {held:?}: no command",
                        status.scan_code
                    ));
                }
                return;
            };
            // Commands that act on the focused terminal (paste, copy, scrolling) must not take the keys of
            // a text box in an overlay, and without a selection Ctrl+C is the program's interrupt.
            let in_terminal = this.terminal_focus.get() && this.overlay.get() == Overlay::None;
            if command == Command::EditSmartCopy || (!in_terminal && !command.works_outside_terminal()) {
                return;
            }
            let _ = args.SetHandled(true);
            // A held key repeats; creating, closing and toggling must act once per press.
            if status.was_key_down && command.once_per_press() {
                return;
            }
            this.run(command);
        }))?;
        self.revokers.borrow_mut().push(revoker);

        // The keys of next/previous tab are up: the tab shown last becomes the most recently used.
        let this = self.clone();
        let revoker = ui(&self.root)?.PreviewKeyUp(cb!("shortcut released", |_, args| {
            let code = args.as_ref().and_then(|args| args.Key().ok()).map_or(0, |key| key.0);
            let is_modifier = matches!(code, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
            if is_modifier && keyboard::held_modifiers() == Default::default() {
                this.mru.borrow_mut().commit();
            }
        }))?;
        self.revokers.borrow_mut().push(revoker);
        Ok(())
    }
}
