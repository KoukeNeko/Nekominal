//! The window: glass, top bar, tabs, overlays, commands and the keyboard layer.
//!
//! One `Shell` per process (single window for now). Everything it owns lives on the UI thread; event closures
//! hold `Weak<Shell>` so closing a tab really frees its panes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nekominal_model::{
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
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use crate::chip::{Chip, TabEvent, TabId, TabState};
use crate::design;
use crate::glass::{self, Glass};
use crate::i18n::{self, localize, t, tf};
use crate::keyboard;
use crate::logic;
use crate::pane_view::{Look, PaneView};
use crate::tab::Tab;
use crate::util::*;
use crate::window;
use crate::{clipboard, findbar, palette, settings_page};

const SHELL_XAML: &str = include_str!("xaml/shell.xaml");

/// How long a change of settings waits for further changes before it is written to disk.
const SAVE_DELAY: Duration = Duration::from_millis(600);

/// Space on the bar that is not the tab strip: app mark, new-tab buttons, tool cluster margins, the smallest
/// useful drag area. Together with the cluster and the caption buttons this bounds the width of the strip.
const BAR_FIXED_WIDTH: f64 = 46.0 + 4.0 + 54.0 + 18.0 + 40.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    None,
    Palette,
    Find,
    Settings,
}

struct Ui {
    top_bar: Grid,
    bar_tint: Border,
    tab_scroll: ScrollViewer,
    tab_strip: StackPanel,
    new_tab: Button,
    new_tab_menu: Button,
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
    next_tab: Cell<TabId>,
    next_pane: Cell<PaneId>,
    pub overlay: Cell<Overlay>,
    terminal_focus: Cell<bool>,
    zoom_steps: Cell<i32>,
    save_due: Cell<Option<Instant>>,
    window_active: Cell<bool>,
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
        new_tab: find(&root, "NewTabButton")?,
        new_tab_menu: find(&root, "NewTabMenuButton")?,
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
        next_tab: Cell::new(1),
        next_pane: Cell::new(1),
        overlay: Cell::new(Overlay::None),
        terminal_focus: Cell::new(false),
        zoom_steps: Cell::new(0),
        save_due: Cell::new(None),
        window_active: Cell::new(true),
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
        self.update_tab_strip_width();

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
        ] {
            self.palette_brush(key)?.SetColor(xaml_color(color))?;
        }
        let opacity = glass::terminal_opacity(backdrop, setting_opacity);
        let bar_alpha = glass::bar_tint_alpha(opacity);
        let tint = themes::Color { a: (bar_alpha * 255.0).round() as u8, ..glass.background };
        self.ui.bar_tint.SetBackground(&solid_brush(tint)?)?;
        window::apply_backdrop(&self.window, &self.root, backdrop, &glass)?;
        window::apply_theme(&self.window, &self.root, &glass)?;
        *self.look.theme.borrow_mut() = glass::render_theme(scheme, opacity);
        for tab in self.tabs.borrow().iter() {
            tab.set_glass(glass);
            for pane in tab.panes.borrow().iter() {
                pane.apply_look(false);
            }
        }
        if let Some(page) = self.settings_page.borrow().as_ref() {
            page.restyle(&glass);
        }
        log(&format!(
            "glass: scheme '{}' light={} backdrop={backdrop:?} opacity={opacity:.2} bar tint alpha {bar_alpha:.2}",
            scheme.name, glass.light
        ));
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

    pub fn set_opacity(self: &Rc<Self>, opacity: f64) {
        self.settings.borrow_mut().appearance.opacity = (opacity * 100.0).round() / 100.0;
        self.settings_changed();
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
        let (backdrop, opacity) = {
            let s = self.settings.borrow();
            (s.appearance.backdrop, s.appearance.opacity)
        };
        let index = match backdrop {
            Backdrop::Acrylic => 0,
            Backdrop::Mica => 1,
            Backdrop::MicaAlt => 2,
            Backdrop::None => 3,
        };
        let percent = (opacity * 100.0).round();
        if self.ui.backdrop_choice.SelectedIndex().ok() != Some(index) {
            let _ = self.ui.backdrop_choice.SetSelectedIndex(index);
        }
        if let Ok(range) = self.ui.opacity_slider.cast::<RangeBase>()
            && (range.Value().unwrap_or(0.0) - percent).abs() > 0.4
        {
            let _ = range.SetValue(percent);
        }
        let _ = self.ui.opacity_value.SetText(&format!("{percent:.0}%"));
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

        // Quick appearance flyout: changes apply while the control moves.
        let this = self.clone();
        revokers.push(self.ui.backdrop_choice.SelectionChanged(cb!("backdrop choice", |_, _| {
            let index = this.ui.backdrop_choice.SelectedIndex().unwrap_or(0);
            let backdrop = [Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt, Backdrop::None]
                .get(usize::try_from(index).unwrap_or(0))
                .copied();
            if let Some(backdrop) = backdrop
                && backdrop != this.settings.borrow().appearance.backdrop
            {
                this.set_backdrop(backdrop);
            }
        }))?);
        let this = self.clone();
        revokers.push(self.ui.opacity_slider.cast::<RangeBase>()?.ValueChanged(cb!("opacity slider", |_, args| {
            let Some(value) = args.as_ref().and_then(|a| a.NewValue().ok()) else { return };
            if (value / 100.0 - this.settings.borrow().appearance.opacity).abs() > 0.004 {
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
        revokers.push(fe(&self.ui.top_bar)?.SizeChanged(cb!("bar size", |_, _| this.update_tab_strip_width()))?);
        let this = self.clone();
        revokers
            .push(fe(&self.ui.tool_cluster)?.SizeChanged(cb!("cluster size", |_, _| this.update_tab_strip_width()))?);
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

    fn update_tab_strip_width(&self) {
        let bar = fe(&self.ui.top_bar).and_then(|b| b.ActualWidth()).unwrap_or(0.0);
        let cluster = fe(&self.ui.tool_cluster).and_then(|b| b.ActualWidth()).unwrap_or(0.0);
        let insets = self.ui.left_inset.ActualWidth().unwrap_or(0.0) + self.ui.right_inset.ActualWidth().unwrap_or(0.0);
        let available = (bar - insets - cluster - BAR_FIXED_WIDTH).max(120.0);
        let _ = fe(&self.ui.tab_scroll).and_then(|s| s.SetMaxWidth(available));
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
        }))?);
        self.revokers.borrow_mut().extend(revokers);
        Ok(())
    }

    /// Every session ends here, while the process can still start the threads that close them (critique.md C14).
    fn on_closed(&self) {
        log("window closed: ending every session");
        if let Err(e) = window::restore_alt_chords(&self.window) {
            log(&format!("restore_alt_chords: {e}"));
        }
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
        if let Err(e) = store::save(&path, &settings::serialize(&self.settings.borrow())) {
            log(&format!("settings save failed: {e}"));
        }
    }

    // ---------------------------------------------------------------------------------------------
    // The render tick
    // ---------------------------------------------------------------------------------------------

    fn tick(self: &Rc<Self>) {
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
                let fallback = pane.title.borrow().clone();
                let title = logic::tab_title(&title, &fallback);
                *pane.title.borrow_mut() = title.clone();
                if tab.focused.get() == pane.id && !tab.renamed.get() {
                    *tab.title.borrow_mut() = title.clone();
                    tab.chip.set_title(&title);
                }
            }
            UiEvent::Cwd(path) => *pane.cwd.borrow_mut() = Some(path.into()),
            UiEvent::Bell => {
                if self.settings.borrow().bell.visual && !tab.is_visible() {
                    tab.chip.set_state(TabState::Attention);
                }
            }
            UiEvent::Exit(code) => {
                pane.exited.set(Some(code));
                if code == 0 {
                    closed.push((tab.id, pane.id));
                } else {
                    tab.chip.set_state(TabState::Error);
                    let line = format!("\r\n{}\r\n", tf(Text::StatusExited, &[&code]));
                    pane.pane.term.lock().feed(line.as_bytes());
                }
            }
            UiEvent::Other(_) => {}
        }
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
            if !tab.renamed.get()
                && let Some(view) = tab.pane(pane)
            {
                let title = view.title.borrow().clone();
                *tab.title.borrow_mut() = title.clone();
                tab.chip.set_title(&title);
            }
            tab.chip.set_state(TabState::Idle);
        }
    }

    pub fn add_tab(self: &Rc<Self>, profile: &Profile) -> Result<Rc<Tab>> {
        let cwd = self.selected_tab().and_then(|tab| tab.focused_pane()).and_then(|pane| pane.cwd.borrow().clone());
        let pane = self.new_pane(profile, cwd.as_deref())?;
        let id = self.next_tab.get();
        self.next_tab.set(id + 1);
        let title = pane.title.borrow().clone();
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

    pub fn select_index(self: &Rc<Self>, index: usize) {
        let tabs = self.tabs.borrow().clone();
        let Some(selected) = tabs.get(index) else { return };
        self.selected.set(index);
        for tab in &tabs {
            let is_selected = Rc::ptr_eq(tab, selected);
            tab.chip.set_selected(is_selected);
            tab.set_visible(is_selected);
        }
        selected.chip.set_state(TabState::Idle);
        selected.update_focus_look();
        let _ = ui(&selected.chip.root).and_then(|chip| chip.StartBringIntoView());
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
        self.focus_active_pane();
        log(&format!("tab {from} moved to {to}"));
    }

    /// Closes after asking when running sessions would end.
    pub fn request_close_tab(self: &Rc<Self>, id: TabId) {
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == id).cloned() else { return };
        let running = tab.running_sessions();
        if self.settings.borrow().should_confirm_close(running) {
            let this = self.clone();
            self.confirm_close(running, move || this.close_tab(id));
        } else {
            self.close_tab(id);
        }
    }

    pub fn close_tab(self: &Rc<Self>, id: TabId) {
        let Some(index) = self.tab_index(id) else { return };
        let tab = self.tabs.borrow_mut().remove(index);
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
        let next = if index < remaining { index } else { remaining - 1 };
        self.select_index(if self.selected.get() > index { self.selected.get() - 1 } else { next.min(remaining - 1) });
    }

    pub fn close_pane_in(self: &Rc<Self>, tab_id: TabId, pane_id: PaneId) {
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == tab_id).cloned() else { return };
        if !tab.remove_pane(pane_id) {
            self.close_tab(tab_id);
            return;
        }
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
        let running = tab.pane(pane_id).map_or(0, |pane| u32::from(pane.exited.get().is_none()));
        if self.settings.borrow().should_confirm_close(running) {
            let this = self.clone();
            let tab_id = tab.id;
            self.confirm_close(running, move || this.close_pane_in(tab_id, pane_id));
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

    fn confirm_close(self: &Rc<Self>, running: u32, on_confirm: impl Fn() + 'static) {
        let result = (|| -> Result<()> {
            let dialog = ContentDialog::new()?;
            dialog.SetTitle(&IReference::from(tf(Text::DialogCloseSessions, &[&running]).as_str()))?;
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
            let revoker = dialog.PrimaryButtonClick(cb!("rename confirmed", |_, _| {
                let name = input.Text().unwrap_or_default().trim().to_owned();
                if name.is_empty() {
                    tab.renamed.set(false);
                } else {
                    tab.renamed.set(true);
                    *tab.title.borrow_mut() = name.clone();
                    tab.chip.set_title(&name);
                }
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
            p.show(which == Overlay::Palette);
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
                    let count = self.tabs.borrow().len();
                    if count > 1 {
                        let current = self.selected.get();
                        let next = if command == Command::TabNext {
                            (current + 1) % count
                        } else {
                            (current + count - 1) % count
                        };
                        self.select_index(next);
                    }
                }
                Command::TabGoto(number) => {
                    let index = usize::from(number).saturating_sub(1);
                    if index < self.tabs.borrow().len() {
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
        Ok(())
    }
}
