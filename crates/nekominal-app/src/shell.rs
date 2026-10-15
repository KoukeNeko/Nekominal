//! The window: glass, top bar, tabs, overlays, commands and the keyboard layer.
//!
//! One `Shell` per process (single window for now). Everything it owns lives on the UI thread; event closures
//! hold `Weak<Shell>` so closing a tab really frees its panes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nekominal_model::{
    bell::{BellContext, bell_effects, flash_for_response},
    commands::{self, Command},
    keymap::{self, Keymap},
    layout::{Axis, PaneId},
    menu::{APP_MENU, MenuRow},
    profiles::{self, DiscoveredProfile, Profile},
    settings::{self, Backdrop, Problem, Settings},
    settings_ui::{Descriptor, Value},
    store,
    strings::{APP_NAME, Lang, Text},
    themes::{self},
};
use nekominal_pane::host::Activity;
use nekominal_pane::{MAX_CLIPBOARD_TEXT, UiEvent};
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_term::LinkKind;
use nekominal_term::agent_status::Report;
use nekominal_term::alacritty_terminal::grid::Scroll;
use nekominal_term::osc::Progress;
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use crate::chip::{CHIP_GAP, Chip, TabEvent, TabId, status_text};
use crate::design;
use crate::glass::{self, Glass, Material};
use crate::i18n::{self, localize, t, tf};
use crate::keyboard;
use crate::logic::{
    self, AgentLabel, AlertKind, Attention, BarTier, COUNT_BLOCK_WIDTH, Need, PaneSignal, Plate, REVEAL_MARGIN,
    Reported, TabAttention, TabStatus, TitleParts, TitleSource,
};
use crate::palette::TabRow;
use crate::pane_view::{Look, PaneRequest, PaneView};
use crate::tab::Tab;
use crate::util::*;
use crate::window;
use crate::{clipboard, drawer, findbar, palette};

const SHELL_XAML: &str = include_str!("xaml/shell.xaml");

/// How long a change of settings waits for further changes before it is written to disk.
const SAVE_DELAY: Duration = Duration::from_millis(600);

/// Padding of the strip inside its scroll viewer, both sides together (`shell.xaml`).
const STRIP_PADDING: f64 = 4.0;

/// How often the transparency effects of Windows are asked about.
const EFFECTS_CHECK_EVERY: Duration = Duration::from_secs(1);

/// Width of the box in which a tab is renamed, in DIP.
const RENAME_WIDTH: f64 = 240.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    None,
    Palette,
    /// The palette as the list of every tab.
    Tabs,
    Find,
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
    /// `!2` after the position: the tabs that are blocked or failed, not shown, with a response nobody looked at.
    tab_plate_attention: TextBlock,
    drag: Grid,
    tool_cluster: Border,
    tool_split_right: Button,
    tool_split_down: Button,
    tool_divider: Border,
    tool_palette: Button,
    /// The button of the menu of everything the bar folds away; the appearance flyout hangs on it.
    tool_menu: Button,
    appearance_flyout: Flyout,
    backdrop_choice: RadioButtons,
    opacity_slider: Slider,
    opacity_value: TextBlock,
    left_inset: ColumnDefinition,
    right_inset: ColumnDefinition,
    terminal_area: Grid,
    body: Grid,
    message_bar: InfoBar,
    /// The button of the message bar, for a message that has an action (the settings file has problems).
    message_action: Button,
}

pub struct Shell {
    pub window: Window,
    pub root: FrameworkElement,
    /// The palette brushes and the button style (`resources.xaml`), merged into the application resources.
    resources: ResourceDictionary,
    ui: Ui,
    pub settings: RefCell<Settings>,
    /// What is wrong with the settings file: the values of `settings.json` and the shortcuts that were not valid.
    pub problems: RefCell<Vec<Problem>>,
    pub keymap: RefCell<Keymap>,
    pub profiles: RefCell<Vec<Profile>>,
    /// The shells found on this machine, which the profiles of the settings are merged with.
    discovered: RefCell<Vec<DiscoveredProfile>>,
    pub glass: Cell<Glass>,
    look: Rc<Look>,
    tabs: RefCell<Vec<Rc<Tab>>>,
    selected: Cell<usize>,
    /// Tabs by most recent use, and the walk of the next/previous tab keys through them.
    mru: RefCell<logic::Mru<TabId>>,
    /// The width of every chip as the last layout of the strip set it.
    strip_widths: RefCell<Vec<f64>>,
    /// How much of the right-hand group of the bar the width of the window leaves room for (None before the first layout).
    bar_tier: Cell<Option<BarTier>>,
    /// The width of the visible part of the strip as the last layout set it (the viewer reports it late).
    strip_viewport: Cell<f64>,
    /// The selected chip must be scrolled into view once the viewer has laid out at its new size (it clamps a
    /// scroll to the size it had).
    reveal_pending: Cell<bool>,
    next_tab: Cell<TabId>,
    next_pane: Cell<PaneId>,
    pub overlay: Cell<Overlay>,
    terminal_focus: Cell<bool>,
    /// The shortcuts page of the drawer is waiting for a chord: the keyboard layer does not run commands meanwhile.
    pub recording_keys: Cell<bool>,
    zoom_steps: Cell<i32>,
    save_due: Cell<Option<Instant>>,
    /// The backdrop the window has now: it is replaced only when the kind changes.
    backdrop_applied: Cell<Option<Backdrop>>,
    /// The opacity the controls asked for, applied by the next frame.
    pending_opacity: RefCell<logic::PendingOpacity>,
    /// Whether the last apply of the opacity failed, for the row of the opacity.
    opacity_apply: RefCell<logic::OpacityApply>,
    /// The quick flyout is being brought in line with the settings: its events are not user input.
    syncing_controls: Cell<bool>,
    window_active: Cell<bool>,
    /// Whether the transparency effects of Windows are on, and when that was last asked (it is asked once a second).
    effects_on: Cell<bool>,
    effects_checked: Cell<Instant>,
    /// When the window last flashed its taskbar button or played a sound: those happen at most once a second.
    last_alert: Cell<Option<Instant>>,
    /// The count after the position on the plate as the last layout of the strip drew it, and the title the window has.
    plate_attention: Cell<usize>,
    window_title: RefCell<String>,
    /// The window was closed: its title can no longer be set.
    closed: Cell<bool>,
    palette: RefCell<Option<Rc<palette::Palette>>>,
    find: RefCell<Option<Rc<findbar::FindBar>>>,
    /// The settings drawer. It is not an `Overlay`: it floats over the terminal without taking the keyboard from it.
    drawer: RefCell<Option<Rc<drawer::Drawer>>>,
    discovery: RefCell<Option<mpsc::Receiver<Vec<nekominal_pty::profiles::Profile>>>>,
    revokers: RefCell<Vec<EventRevoker>>,
    rendering: RefCell<Option<EventRevoker>>,
    dialog: RefCell<Option<windows_future::IAsyncOperation<ContentDialogResult>>>,
    profile_menu: RefCell<Option<MenuFlyout>>,
    /// The menu of the right button of a pane and its Copy item, which is enabled while a selection exists.
    pane_menu: RefCell<Option<(MenuFlyout, MenuFlyoutItem)>>,
    /// The menu of the ... button as it was last opened, with the handlers of its rows. It is built again every time it opens: its labels, shortcuts and checks are read then.
    app_menu: RefCell<Option<(MenuFlyout, Vec<EventRevoker>)>>,
    /// The row Background opacity... of that menu was chosen: the appearance flyout opens when the menu has closed.
    appearance_requested: Cell<bool>,
    /// The row Rename of the menu of a chip was chosen: the flyout of the name opens when the menu has closed.
    rename_requested: Cell<Option<TabId>>,
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

/// A menu row that runs `on_click`; its handler goes into `revokers`, which are dropped with the menu.
fn plain_menu_item(label: &str, revokers: &mut Vec<EventRevoker>, on_click: impl Fn() + 'static) -> Result<MenuFlyoutItem> {
    let item = MenuFlyoutItem::new()?;
    item.SetText(label)?;
    revokers.push(item.Click(cb!("menu item", |_, _| on_click()))?);
    Ok(item)
}

/// A menu row with a check mark that is shown while `checked`.
fn checked_menu_item(
    label: &str,
    checked: bool,
    revokers: &mut Vec<EventRevoker>,
    on_click: impl Fn() + 'static,
) -> Result<ToggleMenuFlyoutItem> {
    let item = ToggleMenuFlyoutItem::new()?;
    item.cast::<MenuFlyoutItem>()?.SetText(label)?;
    item.SetIsChecked(checked)?;
    revokers.push(item.cast::<MenuFlyoutItem>()?.Click(cb!("menu item", |_, _| on_click()))?);
    Ok(item)
}

/// A row of a menu that is one of several choices, marked with a dot while `checked`. `RadioMenuFlyoutItem` has no
/// binding here, so it is created from markup that holds only its group and its state; the label is set after.
fn radio_menu_item(
    label: &str,
    checked: bool,
    revokers: &mut Vec<EventRevoker>,
    on_click: impl Fn() + 'static,
) -> Result<MenuFlyoutItem> {
    let markup = format!(
        r#"<RadioMenuFlyoutItem xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" GroupName="Backdrop" IsChecked="{}"/>"#,
        if checked { "True" } else { "False" }
    );
    let item: MenuFlyoutItem = load_xaml(&markup)?.cast()?;
    item.SetText(label)?;
    revokers.push(item.Click(cb!("menu item", |_, _| on_click()))?);
    Ok(item)
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
    window.SetTitle(APP_NAME)?;
    let resources: ResourceDictionary = XamlReader::Load(&design::resources_xaml())?.cast()?;
    Application::Current()?.Resources()?.MergedDictionaries()?.Append(&resources)?;
    let root = load_xaml(&localize(SHELL_XAML))?;

    let scheme = themes::scheme(&settings.theme);
    let glass = Glass::from_scheme(scheme);
    let material = Material { window_active: true, effects_on: window::transparency_effects() };
    let opacity = glass::terminal_opacity(settings.appearance.backdrop, settings.appearance.opacity, material);
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
        tab_plate_attention: find(&root, "TabPlateAttention")?,
        drag: find(&root, "DragRegion")?,
        tool_cluster: find(&root, "ToolCluster")?,
        tool_split_right: find(&root, "ToolSplitRight")?,
        tool_split_down: find(&root, "ToolSplitDown")?,
        tool_divider: find(&root, "ToolDivider")?,
        tool_palette: find(&root, "ToolPalette")?,
        tool_menu: find(&root, "ToolMenu")?,
        appearance_flyout: find(&root, "AppearanceFlyout")?,
        backdrop_choice: find(&root, "BackdropChoice")?,
        opacity_slider: find(&root, "OpacitySlider")?,
        opacity_value: find(&root, "OpacityValue")?,
        left_inset: find(&root, "LeftInsetColumn")?,
        right_inset: find(&root, "RightInsetColumn")?,
        terminal_area: find(&root, "TerminalArea")?,
        body: find(&root, "Body")?,
        message_bar: find(&root, "MessageBar")?,
        message_action: find(&root, "MessageAction")?,
    };

    let shell = Rc::new(Shell {
        window: window.clone(),
        root: root.clone(),
        resources,
        ui: elements,
        settings: RefCell::new(settings),
        problems: RefCell::new(parsed.problems.iter().chain(&built.problems).cloned().collect()),
        keymap: RefCell::new(built.keymap),
        profiles: RefCell::new(Vec::new()),
        discovered: RefCell::new(Vec::new()),
        glass: Cell::new(glass),
        look,
        tabs: RefCell::new(Vec::new()),
        selected: Cell::new(0),
        mru: RefCell::new(logic::Mru::default()),
        strip_widths: RefCell::new(Vec::new()),
        bar_tier: Cell::new(None),
        strip_viewport: Cell::new(0.0),
        reveal_pending: Cell::new(false),
        next_tab: Cell::new(1),
        next_pane: Cell::new(1),
        overlay: Cell::new(Overlay::None),
        terminal_focus: Cell::new(false),
        recording_keys: Cell::new(false),
        zoom_steps: Cell::new(0),
        save_due: Cell::new(None),
        backdrop_applied: Cell::new(None),
        pending_opacity: RefCell::new(logic::PendingOpacity::default()),
        opacity_apply: RefCell::new(logic::OpacityApply::default()),
        syncing_controls: Cell::new(false),
        window_active: Cell::new(true),
        effects_on: Cell::new(material.effects_on),
        effects_checked: Cell::new(Instant::now()),
        last_alert: Cell::new(None),
        plate_attention: Cell::new(0),
        window_title: RefCell::new(APP_NAME.to_owned()),
        closed: Cell::new(false),
        palette: RefCell::new(None),
        find: RefCell::new(None),
        drawer: RefCell::new(None),
        discovery: RefCell::new(Some(rx)),
        revokers: RefCell::new(Vec::new()),
        rendering: RefCell::new(None),
        dialog: RefCell::new(None),
        profile_menu: RefCell::new(None),
        pane_menu: RefCell::new(None),
        app_menu: RefCell::new(None),
        appearance_requested: Cell::new(false),
        rename_requested: Cell::new(None),
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

    let loaded_shell = shell.clone();
    let loaded = fe(&root)?.Loaded(cb!("root loaded", |_, _| loaded_shell.on_loaded(load_error.clone())))?;
    shell.revokers.borrow_mut().push(loaded);
    log("shell built");
    Ok(())
}

impl Shell {
    // ---------------------------------------------------------------------------------------------
    // Start-up
    // ---------------------------------------------------------------------------------------------

    fn on_loaded(self: &Rc<Self>, load_error: Option<String>) {
        let scale = self.scale();
        let (left, right) = window::caption_insets(&self.window, scale);
        let _ = self.ui.left_inset.SetWidth(GridLength { value: left, grid_unit_type: GridUnitType::Pixel });
        let _ = self.ui.right_inset.SetWidth(GridLength { value: right, grid_unit_type: GridUnitType::Pixel });
        self.apply_bar_tier();
        self.layout_tabs(true);

        // Discovery has been running since launch; it is normally done by now.
        let found = self
            .discovery
            .borrow_mut()
            .take()
            .and_then(|rx| rx.recv_timeout(Duration::from_secs(6)).ok())
            .unwrap_or_default();
        self.set_profiles(&found);

        if let Some(error) = load_error {
            self.show_message(&tf(Text::StatusSettingsLoadFailed, &[&error]), InfoBarSeverity::Warning);
        } else if !self.problems.borrow().is_empty() {
            self.show_settings_problems();
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

    /// What the system does to the backdrop now: the window is active or not, the transparency effects are on or not.
    fn material(&self) -> Material {
        Material { window_active: self.window_active.get(), effects_on: self.effects_on.get() }
    }

    fn scale(&self) -> f64 {
        self.root.xaml_root().and_then(|root| root.RasterizationScale()).unwrap_or(1.0)
    }

    pub fn lang(&self) -> Lang {
        i18n::lang()
    }

    /// The grid under the top bar that holds the terminals and, above them, the overlays and the drawer.
    pub fn body(&self) -> &Grid {
        &self.ui.body
    }

    fn set_profiles(self: &Rc<Self>, found: &[nekominal_pty::profiles::Profile]) {
        *self.discovered.borrow_mut() = found.iter().map(discovered).collect();
        self.merge_profiles();
    }

    /// The profiles are what was discovered plus what the settings add; the menu of new tabs follows them.
    fn merge_profiles(self: &Rc<Self>) {
        *self.profiles.borrow_mut() = profiles::merge(&self.discovered.borrow(), &self.settings.borrow().profiles);
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

    /// Recolours the whole chrome from the colour scheme of the settings and applies backdrop and opacity.
    pub fn apply_glass(self: &Rc<Self>) -> Result<()> {
        let scheme_name = self.settings.borrow().theme.clone();
        self.apply_scheme(&scheme_name)
    }

    /// The same with the colour scheme `scheme_name`, which is what the drawer shows while a tile is pointed at.
    fn apply_scheme(self: &Rc<Self>, scheme_name: &str) -> Result<()> {
        let (backdrop, setting_opacity) = {
            let s = self.settings.borrow();
            (s.appearance.backdrop, s.appearance.opacity)
        };
        let scheme = themes::scheme(scheme_name);
        // A window that is not active has quieter chrome; what asks for the user keeps its strength (`Glass::calm`).
        let glass =
            if self.window_active.get() { Glass::from_scheme(scheme) } else { Glass::from_scheme(scheme).calm() };
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
            ("NkPanelText", glass.panel_text()),
            ("NkPanelTextSecondary", glass.panel_text_secondary()),
            ("NkPanelOutline", glass.panel_outline()),
            ("NkPanelAccent", glass.panel_accent()),
            ("NkPanelWarning", glass.panel_warning()),
            ("NkPanelBorder", glass.panel_border()),
            ("NkSelected", glass.chip_selected()),
            ("NkSelectedBorder", glass.chip_selected_border()),
        ] {
            self.palette_brush(key)?.SetColor(xaml_color(color))?;
        }
        // The count of the plate is amber, the colour of what waits for the user.
        self.ui.tab_plate_attention.SetForeground(&solid_brush(glass.mark(glass.warning))?)?;
        if self.backdrop_applied.get() != Some(backdrop) {
            // The rest of the glass is applied either way; what stays as it was is said once, in the InfoBar.
            match window::set_system_backdrop(&self.window, backdrop) {
                Ok(()) => self.backdrop_applied.set(Some(backdrop)),
                Err(e) => {
                    log(&format!("system backdrop {backdrop:?}: {e}"));
                    let what = tf(Text::LabelValue, &[&t(Text::SettingsBackdrop), &t(Text::SettingNotApplied)]);
                    self.show_message(&what, InfoBarSeverity::Warning);
                }
            }
        }
        let flat = self.material().is_flat(backdrop);
        window::apply_root_background(&self.root, flat, &glass)?;
        window::apply_theme(&self.window, &self.root, &glass)?;
        let opacity = glass::terminal_opacity(backdrop, setting_opacity, self.material());
        self.apply_opacity(scheme, &glass, opacity)?;
        for tab in self.tabs.borrow().iter() {
            tab.set_glass(glass);
        }
        self.restyle_strip_fades(&glass)?;
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
            self.opacity_apply.borrow_mut().requested();
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
        let effective = glass::terminal_opacity(backdrop, opacity, self.material());
        match self.apply_opacity(themes::scheme(&scheme_name), &self.glass.get(), effective) {
            Ok(()) => self.opacity_apply.borrow_mut().applied(),
            Err(e) => {
                log(&format!("apply_opacity: {e}"));
                self.opacity_apply.borrow_mut().failed();
            }
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

    /// Shows the colour scheme `name` without keeping it (a tile of the drawer is pointed at), or, with `None`, puts the
    /// scheme of the settings back.
    pub fn peek_theme(self: &Rc<Self>, name: Option<&str>) {
        let applied = match name {
            Some(name) => self.apply_scheme(name),
            None => self.apply_glass(),
        };
        if let Err(e) = applied {
            log(&format!("peek {name:?}: {e}"));
        }
    }

    /// A row of the drawer changed: the value goes into the settings, what it changes is put in effect, and the save is
    /// scheduled. The opacity takes the way of the sliders (applied by the next frame, saved once it rests).
    pub fn change_setting(self: &Rc<Self>, row: &Descriptor, value: Value) {
        if row.id == "appearance.opacity" {
            if let Value::Number(percent) = value {
                self.set_opacity(percent / 100.0);
            }
            return;
        }
        let before = self.settings.borrow().clone();
        (row.set)(&mut self.settings.borrow_mut(), value);
        let now = self.settings.borrow().clone();
        if now == before {
            return;
        }
        log(&format!("setting {} changed", row.id));
        let effects = logic::effects(&before, &now);
        if effects.glass {
            self.settings_changed();
        } else {
            self.save_due.set(Some(Instant::now() + SAVE_DELAY));
        }
        if effects.font {
            self.apply_font_settings(&before);
        }
        if effects.osc52 {
            self.apply_osc52();
        }
        if effects.titles {
            self.refresh_titles();
        }
        self.apply_terminal_settings(&effects);
        if !effects.glass {
            self.sync_drawer();
        }
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
        let (percent, enabled, value) = self.opacity_display();
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
        let _ = self.ui.opacity_value.SetText(&value);
        self.syncing_controls.set(false);
        self.sync_drawer();
    }

    /// What the controls show of the opacity: the percent, whether the slider can be moved, and its label.
    pub fn opacity_display(&self) -> (f64, bool, String) {
        let backdrop = self.settings.borrow().appearance.backdrop;
        let (percent, enabled) = glass::opacity_control(backdrop, self.opacity_latest());
        let label = if self.opacity_apply.borrow().shows_label() {
            t(Text::SettingNotApplied).to_owned()
        } else {
            format!("{percent:.0}%")
        };
        (percent, enabled, label)
    }

    /// Brings the drawer, when there is one, in line with the settings.
    fn sync_drawer(&self) {
        let drawer = self.drawer.borrow().clone();
        if let Some(drawer) = drawer {
            drawer.sync();
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
        tip(&self.ui.tool_palette, keymap.tooltip(Command::AppPalette, lang));
        tip(&self.ui.tool_menu, t(Text::MenuApp).to_owned());
    }

    fn attach_bar(self: &Rc<Self>) -> Result<()> {
        let mut revokers = Vec::new();
        let buttons: [(&Button, Command); 4] = [
            (&self.ui.new_tab, Command::TabNew),
            (&self.ui.tool_split_right, Command::PaneSplitRight),
            (&self.ui.tool_split_down, Command::PaneSplitDown),
            (&self.ui.tool_palette, Command::AppPalette),
        ];
        for (button, command) in buttons {
            let this = self.clone();
            revokers.push(button.on_click(cb!("tool button", |_, _| this.run(command)))?);
        }
        let this = self.clone();
        revokers.push(self.ui.new_tab_menu.on_click(cb!("profile menu", |_, _| this.show_profile_menu()))?);
        let this = self.clone();
        revokers.push(self.ui.tool_menu.on_click(cb!("app menu", |_, _| this.show_app_menu()))?);
        let this = self.clone();
        revokers.push(self.ui.tab_plate_button.on_click(cb!("tab list", |_, _| this.toggle_overlay(Overlay::Tabs)))?);
        let this = self.clone();
        revokers.push(self.ui.message_action.on_click(cb!("message action", |_, _| this.open_settings_file()))?);

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
        revokers.push(fe(&self.ui.top_bar)?.SizeChanged(cb!("bar size", |_, _| {
            this.apply_bar_tier();
            this.layout_tabs(true);
        }))?);
        let this = self.clone();
        revokers.push(fe(&self.ui.tool_cluster)?.SizeChanged(cb!("cluster size", |_, _| this.layout_tabs(true)))?);
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

    /// The tier the width of the window puts the bar in.
    fn tier(&self) -> BarTier {
        self.bar_tier.get().unwrap_or(BarTier::Wide)
    }

    /// Shows the controls on the right of the bar that the width of the window leaves room for (`BarTier::at`): the
    /// rest is in the menu. Returns whether the tier changed.
    fn apply_bar_tier(&self) -> bool {
        let width = fe(&self.ui.top_bar).and_then(|bar| bar.ActualWidth()).unwrap_or(0.0);
        let tier = BarTier::at(width, self.bar_tier.get());
        if self.bar_tier.replace(Some(tier)) == Some(tier) {
            return false;
        }
        for (button, command) in [
            (&self.ui.tool_split_right, Command::PaneSplitRight),
            (&self.ui.tool_split_down, Command::PaneSplitDown),
            (&self.ui.tool_palette, Command::AppPalette),
        ] {
            let _ = ui(button).and_then(|button| button.SetVisibility(visibility(tier.shows(command))));
        }
        let _ = ui(&self.ui.tool_divider).and_then(|divider| divider.SetVisibility(visibility(tier.shows_divider())));
        log(&format!("bar {tier:?} at {width:.0} DIP"));
        true
    }

    /// The width the strip and the position plate may share (`logic::strip_budget`).
    fn strip_budget(&self) -> f64 {
        let bar = fe(&self.ui.top_bar).and_then(|b| b.ActualWidth()).unwrap_or(0.0);
        let insets = self.ui.left_inset.ActualWidth().unwrap_or(0.0) + self.ui.right_inset.ActualWidth().unwrap_or(0.0);
        logic::strip_budget(bar, insets, self.tier())
    }

    /// Sizes the whole tab strip: the position plate, the width of every chip and the counts at the cut edges. Runs
    /// whenever the tabs, the selection or the window change. `reveal` also scrolls the selected chip into view; only a
    /// change of the selection, of the set or order of the tabs or of the viewport asks for it, so that a strip the user
    /// scrolled away is not snapped back by what a program does. Nothing a program or the selection does changes the
    /// width of a chip (`logic::tab_width`), so a title is not a reason to lay the strip out again.
    fn layout_tabs(&self, reveal: bool) {
        let tabs = self.tabs.borrow().clone();
        let budget = self.strip_budget();
        let strip = (budget - self.update_plate(tabs.len(), self.selected.get(), budget)).max(logic::STRIP_MIN_WIDTH);
        let _ = fe(&self.ui.tab_scroll).and_then(|scroll| scroll.SetMaxWidth(strip));
        let width = logic::tab_width(tabs.len(), strip - STRIP_PADDING, CHIP_GAP);
        for (index, tab) in tabs.iter().enumerate() {
            tab.chip.set_width(width);
            tab.chip.set_position(index, tabs.len());
        }
        let widths = vec![width; tabs.len()];
        let extent = logic::strip_extent(&widths, CHIP_GAP, STRIP_PADDING);
        *self.strip_widths.borrow_mut() = widths;
        self.strip_viewport.set(strip.min(extent));
        if reveal {
            self.reveal_pending.set(true);
        }
        self.update_strip_edges(None);
    }

    /// Shows `current / total` (or the total alone on a narrow strip), then the `!N` of the tabs that wait for the user,
    /// and returns the room the strip leaves it. The room is that of the plate at its widest (the position of the last
    /// tab, the count of every tab but one) whatever it says now: selecting a tab or a tab needing a response changes the
    /// plate, and must not change the room, because the width of the chips comes from it. The count is shown on a narrow
    /// strip too: it is what the plate is for.
    fn update_plate(&self, count: usize, selected: usize, budget: f64) -> f64 {
        let unread = logic::plate_count(&self.attention_tabs());
        self.plate_attention.set(unread);
        let Some(shown) = logic::plate(selected, count, budget, self.tier()) else {
            let _ = ui(&self.ui.tab_plate).and_then(|plate| plate.SetVisibility(Visibility::Collapsed));
            return 0.0;
        };
        let text = |plate: Plate| match plate {
            Plate::Position { current, total } => tf(Text::FindCount, &[&current, &total]),
            Plate::Total(total) => total.to_string(),
        };
        let widest = match shown {
            Plate::Position { total, .. } => Plate::Position { current: total, total },
            Plate::Total(total) => Plate::Total(total),
        };
        let plate = ui(&self.ui.tab_plate).ok();
        if let Some(plate) = &plate {
            let _ = plate.SetVisibility(Visibility::Visible);
        }
        // Says `position` and `attention` on the plate and measures it.
        let say = |position: &str, attention: Option<String>| -> f64 {
            let _ = self.ui.tab_plate_text.SetText(position);
            let _ = self.ui.tab_plate_attention.SetText(attention.as_deref().unwrap_or_default());
            let _ =
                ui(&self.ui.tab_plate_attention).and_then(|mark| mark.SetVisibility(visibility(attention.is_some())));
            let measured = plate.as_ref().and_then(|plate| {
                plate.Measure(Size { width: f32::INFINITY, height: f32::INFINITY }).ok()?;
                plate.DesiredSize().ok()
            });
            measured.map_or(0.0, |size| f64::from(size.width))
        };
        let room = say(&text(widest), logic::attention_text(count - 1));
        say(&text(shown), logic::attention_text(unread));
        // The room and the 4 DIP in front of it.
        room + 4.0
    }
    /// Scrolls the strip so the selected chip is in view with a margin; returns the offset it scrolls to.
    fn reveal_selected(&self) -> Option<f64> {
        let selected = self.selected.get();
        // Copied out: `UpdateLayout` below raises SizeChanged handlers that write the widths.
        let (left, width, extent) = {
            let widths = self.strip_widths.borrow();
            let width = *widths.get(selected)?;
            let left = STRIP_PADDING / 2.0 + widths[..selected].iter().sum::<f64>() + CHIP_GAP * selected as f64;
            (left, width, logic::strip_extent(&widths, CHIP_GAP, STRIP_PADDING))
        };
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
        let offset = offset.unwrap_or_else(|| self.ui.tab_scroll.HorizontalOffset().unwrap_or(0.0));
        let (left, right) =
            logic::hidden_counts(&self.strip_widths.borrow(), CHIP_GAP, STRIP_PADDING, offset, self.strip_viewport.get());
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
            // The system makes the backdrop flat while the window is not active: the chrome follows (`Material`).
            if !this.closed.get()
                && let Err(e) = this.apply_glass()
            {
                log(&format!("apply_glass: {e}"));
            }
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
            // The title of an inactive window carries the count of the tabs that wait for the user.
            this.update_attention();
        }))?);
        self.revokers.borrow_mut().extend(revokers);
        Ok(())
    }

    /// Every session ends here, while the process can still start the threads that close them (critique.md C14).
    fn on_closed(self: &Rc<Self>) {
        log("window closed: ending every session");
        self.closed.set(true);
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
        match store::save_keeping_original(&path, &settings::serialize(&self.settings.borrow())) {
            Ok(()) => log("settings saved"),
            Err(e) => log(&format!("settings save failed: {e}")),
        }
    }

    // ---------------------------------------------------------------------------------------------
    // The render tick
    // ---------------------------------------------------------------------------------------------

    fn tick(self: &Rc<Self>) {
        let drawer = self.drawer.borrow().clone();
        if let Some(drawer) = drawer {
            drawer.tick();
        }
        self.apply_pending_opacity();
        self.check_transparency_effects();
        if self.reveal_pending.take() {
            let offset = self.reveal_selected();
            self.update_strip_edges(offset);
        }
        if self.save_due.get().is_some_and(|due| Instant::now() >= due) {
            self.flush_settings();
            // The slider has rested: an opacity that did not apply is said in its row now.
            self.opacity_apply.borrow_mut().rested();
            self.sync_appearance_controls();
        }
        let tabs: Vec<Rc<Tab>> = self.tabs.borrow().clone();
        let mut closed_panes = Vec::new();
        for tab in &tabs {
            // A chip being dragged at an end of the strip scrolls it.
            tab.chip.tick_drag();
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

    /// Asks once a second whether the user turned the transparency effects of Windows on or off, and gives the chrome the
    /// look of a flat backdrop (or takes it away) when they did.
    fn check_transparency_effects(self: &Rc<Self>) {
        if self.effects_checked.get().elapsed() < EFFECTS_CHECK_EVERY {
            return;
        }
        self.effects_checked.set(Instant::now());
        let on = window::transparency_effects();
        if on != self.effects_on.replace(on) {
            log(&format!("transparency effects = {on}"));
            if let Err(e) = self.apply_glass() {
                log(&format!("apply_glass: {e}"));
            }
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
                // A program that ended mid-progress never sent the report that clears it.
                pane.progress.set(None);
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
            UiEvent::Agent(report) => {
                if let Report::Set(record) = &report {
                    pane.agent_label.borrow_mut().remember(record);
                }
                self.on_agent_change(tab, pane);
            }
            UiEvent::AgentRecordsDropped => self.on_agent_change(tab, pane),
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
            UiEvent::ClipboardStore(text) => {
                let written = clipboard::write_text(&text);
                log(&format!("pane {} copied {} bytes to the clipboard (osc 52): {written}", pane.id, text.len()));
            }
            UiEvent::ClipboardLoad(request) => {
                // Only a policy that allows reads lets the request get here. No text on the clipboard is an empty answer.
                let text = clipboard::read_text().unwrap_or_default();
                let answered = pane.pane.answer_clipboard(&request, &text);
                log(&format!("pane {} read {} bytes of the clipboard (osc 52): answered {answered}", pane.id, text.len()));
            }
            UiEvent::ClipboardRefused { bytes } => {
                log(&format!("pane {} osc 52 write of {bytes} bytes refused: over {} bytes", pane.id, MAX_CLIPBOARD_TEXT));
            }
            UiEvent::PromptMark(mark) if Activity::after_mark(mark) == Activity::AtPrompt => {
                // A shell back at its prompt has no program at work: a command interrupted mid-progress never
                // cleared its progress, and an agent that ended takes the name it gave the tab with it.
                let was_working = pane.progress.take().is_some();
                let was_named = std::mem::take(&mut *pane.agent_label.borrow_mut()) != AgentLabel::default();
                if was_working {
                    self.refresh_status(tab);
                }
                if was_named {
                    self.refresh_titles();
                }
            }
            UiEvent::PromptMark(_) | UiEvent::Other(_) => {}
        }
    }

    /// The agent records of a pane changed. What a report says is not latched: the chip shows what the emulator holds
    /// now. A report in the pane the user is looking at is seen at once.
    fn on_agent_change(&self, tab: &Rc<Tab>, pane: &Rc<PaneView>) {
        if self.in_view(tab, pane) {
            pane.pane.dismiss_finished();
        }
        self.refresh_status(tab);
        self.refresh_titles();
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
                let label = pane.as_ref().map(|pane| pane.agent_label.borrow().clone()).unwrap_or_default();
                TitleParts {
                    user: tab.renamed.get().then(|| tab.title.borrow().clone()),
                    program: pane.as_ref().map(|pane| pane.title.borrow().clone()).unwrap_or_default(),
                    agent_title: label.title,
                    agent_app: label.app,
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
        let format = self.settings.borrow().tabs.title.parts.clone();
        let first: Vec<(String, TitleSource)> =
            parts.iter().map(|parts| logic::default_title(parts, &format, false)).collect();
        let folders: Vec<Option<String>> = first
            .iter()
            .map(|(title, source)| (*source == TitleSource::Parts { folder: true }).then(|| title.clone()))
            .collect();
        let collide = logic::colliding_folders(&folders);
        for (((tab, parts), (title, _)), with_parent) in tabs.iter().zip(&parts).zip(first).zip(collide) {
            tab.chip.set_context(parts.cwd.clone(), &parts.profile);
            if tab.renamed.get() {
                continue;
            }
            let title = if with_parent { logic::default_title(parts, &format, true).0 } else { title };
            if *tab.title.borrow() != title {
                *tab.title.borrow_mut() = title.clone();
                tab.chip.set_title(&title);
            }
        }
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
        let effects = bell_effects(&self.settings.borrow().bell, &context);
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
        let panes = tab.panes.borrow().clone();
        let signals: Vec<PaneSignal> = panes
            .iter()
            .map(|pane| PaneSignal {
                agent: pane.pane.agent_summary(),
                progress: pane.progress.get(),
                attention: pane.attention.get(),
                note: pane.note.borrow().clone(),
            })
            .collect();
        for (pane, signal) in panes.iter().zip(&signals) {
            pane.needs_response.set(logic::needs_response(signal));
        }
        tab.update_attention_lines();
        // A zoom hides the other panes and their warning lines: the corner label of the zoomed pane shows what the
        // ones that wait for the user ask for.
        let zoomed = tab.zoomed.get().and_then(|zoomed| panes.iter().position(|pane| pane.id == zoomed));
        tab.set_badge_alert(logic::hidden_alert(&signals, zoomed));
        let status = logic::tab_status(&signals);
        if tab.chip.state() != status.state {
            log(&format!("tab {} status {:?}", tab.id, status.state));
        }
        // Whether the response was looked at is a flag of its own (`logic::Watch`): the chip shows what the panes say
        // now, and the flag says only that the user was not there when it began.
        let watched = self.window_active.get() && tab.is_visible();
        let other_alerts =
            self.tabs.borrow().iter().filter(|other| other.id != tab.id && other.watch.borrow().alerted()).count();
        let raised = tab.watch.borrow_mut().observe(Need::of(&signals, &status), watched, other_alerts);
        tab.chip.set_status(status.clone());
        if let Some(kind) = raised {
            self.raise_alert(tab, kind, &status);
        }
        self.update_attention();
    }

    /// A tab began to need a response out of sight: the screen reader is told once, and the taskbar button flashes while
    /// the window is not active (at most once a second, with the bell and the sound; `bell.taskbar` is the setting).
    /// Neither happens when two alerts are outstanding already (`logic::MAX_ALERTS`); the tab has its mark all the same.
    fn raise_alert(&self, tab: &Rc<Tab>, kind: AlertKind, status: &TabStatus) {
        let flash = flash_for_response(
            self.settings.borrow().bell.taskbar,
            self.window_active.get(),
            self.last_alert.get().map(|at| at.elapsed()),
        );
        if flash {
            match window::flash_taskbar(&self.window) {
                Ok(()) => self.last_alert.set(Some(Instant::now())),
                Err(e) => log(&format!("taskbar flash: {e}")),
            }
        }
        let name = match status_text(status) {
            Some(word) => tf(Text::TabNameStatus, &[&*tab.title.borrow(), &word]),
            None => tab.title.borrow().clone(),
        };
        let announced = tab.chip.announce(&name);
        log(&format!("alert tab {} {kind:?}: announced {announced}, taskbar flash {flash}", tab.id));
    }

    /// Every tab as the counts see it.
    fn attention_tabs(&self) -> Vec<TabAttention> {
        let selected = self.selected.get();
        self.tabs
            .borrow()
            .iter()
            .enumerate()
            .map(|(index, tab)| TabAttention {
                state: tab.chip.state(),
                unread: tab.watch.borrow().unread(),
                selected: index == selected,
                waiting: tab.needs_response(),
            })
            .collect()
    }

    /// The `!N` of the plate (the strip is laid out again when its width changes) and the title of the window follow the
    /// tabs. Runs whenever what a tab needs, or whether the window is active, changes.
    fn update_attention(&self) {
        if self.closed.get() {
            return;
        }
        let tabs = self.attention_tabs();
        if logic::plate_count(&tabs) != self.plate_attention.get() {
            self.layout_tabs(false);
        }
        let title = logic::window_title(APP_NAME, logic::waiting_count(&tabs), self.window_active.get());
        if *self.window_title.borrow() != title {
            if let Err(e) = self.window.SetTitle(&title) {
                log(&format!("window title: {e}"));
            }
            log(&format!("window title '{title}'"));
            *self.window_title.borrow_mut() = title;
        }
    }

    /// The user has seen the focused pane of the tab: its bells, notifications and what a finished or failed agent
    /// reported are acknowledged. What still waits (a blocked or working agent) stays, and so does what another
    /// pane of the tab reported: that one has its warning line until it is focused.
    fn mark_seen(&self, tab: &Rc<Tab>) {
        if let Some(pane) = tab.focused_pane() {
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
                let waiting: Vec<bool> = tab.waiting_panes().into_iter().map(|(_, waits)| waits).collect();
                TabRow {
                    id: tab.id,
                    ordinal: index + 1,
                    search: format!("{title} {folder}"),
                    title,
                    folder,
                    panes: tab.pane_count(),
                    status: tab.chip.status(),
                    unread: tab.watch.borrow().unread(),
                    waiting_pane: logic::waiting_pane(&waiting),
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
        let weak = Rc::downgrade(self);
        let on_request: Rc<dyn Fn(PaneId, PaneRequest)> = Rc::new(move |pane, request| {
            if let Some(shell) = weak.upgrade() {
                shell.on_pane_request(pane, request);
            }
        });
        let config = self.spawn_config(profile, cwd);
        let core = logic::core_config(&self.settings.borrow());
        let look = self.look.clone();
        PaneView::new(id, config, core, profile.id.clone(), look, &self.window, &self.root, on_focus, on_request)
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
        let chip = Chip::new(id, &title, self.glass.get(), &self.ui.tab_strip, &self.ui.tab_scroll, on_event)?;
        let on_unzoom: Rc<dyn Fn()> = {
            let weak = Rc::downgrade(self);
            Rc::new(move || {
                if let Some(shell) = weak.upgrade() {
                    shell.run(Command::PaneZoom);
                }
            })
        };
        let tab = Tab::new(id, chip.clone(), pane, title, self.glass.get(), on_unzoom)?;
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
            TabEvent::Rename(id) => self.rename_tab(id),
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

    /// Selects the tab at `index` the way the list of every tab and the jump to the next response do: where a pane of the
    /// tab waits for the user, that pane is the one that gets the focus.
    fn go_to_tab(self: &Rc<Self>, index: usize) {
        if let Some(tab) = self.tabs.borrow().get(index).cloned() {
            tab.focus_waiting_pane();
        }
        self.select_index(index);
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
        self.layout_tabs(true);
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
        self.layout_tabs(true);
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
        let next = self.mru.borrow_mut().close(id, current);
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

    /// Opens the menu of the `...` button: `menu::APP_MENU`, the same rows at every width of the window. It is built
    /// again each time, so its labels, shortcuts and checks are those of the moment, and the handlers of the menu
    /// before it go with it.
    fn show_app_menu(self: &Rc<Self>) {
        match self.build_app_menu() {
            Ok((menu, revokers)) => {
                let shown = menu.cast::<FlyoutBase>().and_then(|flyout| {
                    flyout.SetPlacement(FlyoutPlacementMode::BottomEdgeAlignedRight)?;
                    flyout.ShowAt(&self.ui.tool_menu)
                });
                if let Err(e) = shown {
                    log(&format!("app menu: {e}"));
                }
                *self.app_menu.borrow_mut() = Some((menu, revokers));
            }
            Err(e) => log(&format!("app menu: {e}")),
        }
    }

    fn build_app_menu(self: &Rc<Self>) -> Result<(MenuFlyout, Vec<EventRevoker>)> {
        let menu = MenuFlyout::new()?;
        let items = menu.Items()?;
        let mut revokers = Vec::new();
        let keymap = self.keymap.borrow();
        let lang = self.lang();
        let (panes, zoomed) =
            self.selected_tab().map_or((0, false), |tab| (tab.pane_count(), tab.zoomed.get().is_some()));
        let fullscreen = app_window(&self.window)?.Presenter()?.Kind()? == AppWindowPresenterKind::FullScreen;
        let backdrop = self.settings.borrow().appearance.backdrop;
        for row in APP_MENU {
            match *row {
                MenuRow::Separator => items.Append(&MenuFlyoutSeparator::new()?)?,
                MenuRow::Command(command) => {
                    let label = keymap.tooltip(command, lang);
                    let this = self.clone();
                    let on_click = move || this.run(command);
                    match command {
                        Command::PaneZoom => {
                            let item = checked_menu_item(&label, zoomed, &mut revokers, on_click)?;
                            item.cast::<Control>()?.SetIsEnabled(panes >= 2)?;
                            items.Append(&item)?;
                        }
                        Command::ViewFullscreen => {
                            items.Append(&checked_menu_item(&label, fullscreen, &mut revokers, on_click)?)?;
                        }
                        _ => items.Append(&plain_menu_item(&label, &mut revokers, on_click)?)?,
                    }
                }
                MenuRow::Backdrop => {
                    let submenu = MenuFlyoutSubItem::new()?;
                    submenu.SetText(t(Text::SettingsBackdrop))?;
                    let choices = submenu.Items()?;
                    for choice in Backdrop::ALL {
                        let this = self.clone();
                        let item = radio_menu_item(t(choice.text()), choice == backdrop, &mut revokers, move || {
                            this.set_backdrop(choice);
                        })?;
                        choices.Append(&item)?;
                    }
                    items.Append(&submenu)?;
                }
                MenuRow::Opacity => {
                    let this = self.clone();
                    items.Append(&plain_menu_item(t(Text::MenuOpacity), &mut revokers, move || {
                        this.appearance_requested.set(true);
                    })?)?;
                }
            }
        }
        let this = self.clone();
        let closed = menu.cast::<FlyoutBase>()?.Closed(cb!("app menu closed", |_, _| {
            // The row of the opacity opens the appearance flyout once the menu is gone (a flyout opened from the click
            // is closed with the menu); the flyout gives the focus back to the pane when it closes.
            if this.appearance_requested.replace(false) {
                this.show_appearance_flyout();
            } else {
                this.focus_active_pane();
            }
        }))?;
        revokers.push(closed);
        Ok((menu, revokers))
    }

    /// Shows the quick appearance flyout (background material and opacity) under the menu button.
    fn show_appearance_flyout(&self) {
        let shown = self.ui.appearance_flyout.cast::<FlyoutBase>().and_then(|flyout| flyout.ShowAt(&self.ui.tool_menu));
        if let Err(e) = shown {
            log(&format!("appearance flyout: {e}"));
        }
    }

    fn attach_chip_menu(self: &Rc<Self>, tab: &Rc<Tab>) -> Result<()> {
        let flyout = MenuFlyout::new()?;
        let items = flyout.Items()?;
        let id = tab.id;
        // The flyout of the name opens when this menu has closed: closing it gives the focus back to the pane.
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabRename), move || this.rename_requested.set(Some(id)))?)?;
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
        // The pane of this tab that has the focus is the one that splits, whichever tab is shown.
        for command in [Command::PaneSplitRight, Command::PaneSplitDown] {
            let this = self.clone();
            items.Append(&self.menu_item(&command.label(self.lang()), move || {
                if let Some(index) = this.tab_index(id) {
                    this.select_index(index);
                    this.run(command);
                }
            })?)?;
        }
        items.Append(&MenuFlyoutSeparator::new()?)?;
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabCloseOthers), move || this.close_other_tabs(id, false))?)?;
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabCloseRight), move || this.close_other_tabs(id, true))?)?;
        let this = self.clone();
        items.Append(&self.menu_item(t(Text::TabClose), move || this.request_close_tab(id))?)?;
        let this = self.clone();
        let closed = flyout.cast::<FlyoutBase>()?.Closed(cb!("tab menu closed", |_, _| {
            if let Some(id) = this.rename_requested.take() {
                this.rename_tab(id);
            }
        }))?;
        self.revokers.borrow_mut().push(closed);
        ui(&tab.chip.root)?.SetContextFlyout(&flyout)?;
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // Selection, clipboard and the pane menu
    // ---------------------------------------------------------------------------------------------

    /// What a pane asked of the shell (`PaneRequest`).
    fn on_pane_request(self: &Rc<Self>, id: PaneId, request: PaneRequest) {
        let Some(pane) = self.find_pane(id) else { return };
        match request {
            PaneRequest::Selected => {
                if self.settings.borrow().copy_on_select {
                    self.copy_selection(&pane, false);
                }
            }
            PaneRequest::OpenLink(uri) => self.open_link(&uri),
            PaneRequest::RightClick { x, y } => match self.settings.borrow().right_click {
                // Copy what is selected, else paste: the selection is spent, like Copy.
                settings::RightClick::Smart => {
                    if pane.pane.selection_text().is_some() {
                        self.copy_selection(&pane, true);
                    } else {
                        self.paste_into(&pane);
                    }
                }
                settings::RightClick::Menu => {
                    if let Err(e) = self.show_pane_menu(&pane, x, y) {
                        log(&format!("pane menu: {e}"));
                    }
                }
            },
        }
    }

    /// Opens a link the user Ctrl+clicked. `open_target` is the gate: nothing it refuses is opened, and the target is
    /// logged before the shell is asked.
    fn open_link(&self, uri: &str) {
        let target = match nekominal_term::links::open_target(uri) {
            Ok(target) => target,
            Err(why) => return log(&format!("link refused ({why:?}): {uri}")),
        };
        if target.kind == LinkKind::File && !std::path::Path::new(&target.target).exists() {
            return log(&format!("link refused (no such file): {}", target.target));
        }
        log(&format!("opening {:?} link: {}", target.kind, target.target));
        if let Err(e) = crate::open::shell_open(&target.target) {
            log(&format!("opening {} failed: {e}", target.target));
        }
    }

    fn focused_has_selection(&self) -> bool {
        self.selected_tab().and_then(|tab| tab.focused_pane()).is_some_and(|pane| pane.pane.selection_text().is_some())
    }

    /// Puts the selection of the pane on the clipboard; `dismiss` then clears it (Copy does, copy on select does
    /// not: the selection stays to be extended).
    fn copy_selection(&self, pane: &Rc<PaneView>, dismiss: bool) {
        let Some(text) = pane.pane.selection_text() else { return };
        if clipboard::write_text(&text) {
            log(&format!("pane {} copied {} characters of its selection", pane.id, text.chars().count()));
            if dismiss {
                pane.pane.clear_selection();
            }
        }
    }

    /// Pastes the clipboard into the pane, asking first when the settings say the text deserves it.
    fn paste_into(self: &Rc<Self>, pane: &Rc<PaneView>) {
        let Some(text) = clipboard::read_text() else { return };
        let bracketed = pane.pane.input_modes().bracketed_paste;
        let warning = logic::paste_warning(&text, bracketed, &self.settings.borrow().paste);
        let title = match warning {
            None => return pane.pane.paste(&text),
            Some(logic::PasteWarning::Lines(lines)) => tf(Text::DialogPasteLines, &[&lines]),
            Some(logic::PasteWarning::Bytes(bytes)) => tf(Text::DialogPasteSize, &[&logic::size_label(bytes)]),
        };
        let id = pane.id;
        self.confirm(title, Text::EditPaste, move || with_pane(id, |pane| pane.pane.paste(&text)));
    }

    /// The menu of the right button (`rightClick: menu`) at a position in DIP from the top-left corner of the pane.
    /// It is built when first needed and reused: its commands act on the focused pane, which the press that opened
    /// it has just made the pane under the pointer.
    fn show_pane_menu(self: &Rc<Self>, pane: &Rc<PaneView>, x: f64, y: f64) -> Result<()> {
        if self.pane_menu.borrow().is_none() {
            let menu = MenuFlyout::new()?;
            let items = menu.Items()?;
            let lang = self.lang();
            let mut copy = None;
            for command in [Command::EditCopy, Command::EditPaste, Command::EditSelectAll] {
                let this = self.clone();
                let item = self.menu_item(&self.keymap.borrow().tooltip(command, lang), move || this.run(command))?;
                items.Append(&item)?;
                if command == Command::EditCopy {
                    copy = Some(item);
                }
            }
            let this = self.clone();
            let closed = menu.cast::<FlyoutBase>()?.Closed(cb!("pane menu closed", |_, _| this.focus_active_pane()))?;
            self.revokers.borrow_mut().push(closed);
            *self.pane_menu.borrow_mut() = copy.map(|copy| (menu, copy));
        }
        let menu = self.pane_menu.borrow();
        let Some((menu, copy)) = menu.as_ref() else { return Ok(()) };
        copy.cast::<Control>()?.SetIsEnabled(pane.pane.selection_text().is_some())?;
        menu.ShowAt(&ui(&pane.host)?, Point { x: x as f32, y: y as f32 })
    }
    // ---------------------------------------------------------------------------------------------
    // Restarting a session, the settings file
    // ---------------------------------------------------------------------------------------------

    /// Restarts the program of a pane. A program that already ended is started again without asking; one that still
    /// does work asks, as closing it would.
    fn request_restart(self: &Rc<Self>, pane: &Rc<PaneView>) {
        let id = pane.id;
        if self.settings.borrow().should_confirm_close(u32::from(pane.is_busy())) {
            let title = self.selected_tab().map(|tab| tab.title.borrow().clone()).unwrap_or_default();
            let this = self.clone();
            self.confirm(tf(Text::DialogRestartSession, &[&title]), Text::PaneRestart, move || this.restart_pane(id));
        } else {
            self.restart_pane(id);
        }
    }

    /// Starts the profile of the pane again in the same pane, in the directory it last reported.
    fn restart_pane(self: &Rc<Self>, id: PaneId) {
        let Some((tab, pane)) = self.tabs.borrow().iter().find_map(|tab| tab.pane(id).map(|pane| (tab.clone(), pane)))
        else {
            return;
        };
        let Some(profile) = self.profile_by_id(&pane.profile_id).or_else(|| self.default_profile()) else { return };
        let cwd = pane.cwd.borrow().clone();
        match pane.pane.restart(self.spawn_config(&profile, cwd.as_deref())) {
            Ok(()) => {
                pane.exited.set(None);
                pane.attention.set(None);
                pane.progress.set(None);
                *pane.note.borrow_mut() = None;
                *pane.title.borrow_mut() = String::new();
                *pane.agent_label.borrow_mut() = AgentLabel::default();
                log(&format!("pane {id} restarted as '{}'", profile.name));
                self.refresh_status(&tab);
                self.refresh_titles();
            }
            Err(e) => self.show_message(&tf(Text::StatusSpawnFailed, &[&profile.name, &e]), InfoBarSeverity::Error),
        }
    }

    /// Opens `settings.json` with the program Windows has for it. A file that does not exist yet is written first, from
    /// the settings in effect, so there is something to edit.
    fn open_settings_file(self: &Rc<Self>) {
        let Some(path) = store::settings_path() else { return };
        self.flush_settings();
        if !path.exists()
            && let Err(e) = store::save(&path, &settings::serialize(&self.settings.borrow()))
        {
            return self.show_message(&tf(Text::StatusSettingsSaveFailed, &[&e]), InfoBarSeverity::Warning);
        }
        log(&format!("opening the settings file {}", path.display()));
        if let Err(e) = crate::open::shell_open(&path.to_string_lossy()) {
            self.show_message(&tf(Text::StatusOpenFailed, &[&path.display(), &e]), InfoBarSeverity::Warning);
        }
    }

    /// Reads `settings.json` again and applies it to the running window. What the file gets wrong is reported the way
    /// it is at start-up, and a file that is fine takes the report away. The language is read at start-up only.
    fn reload_settings(self: &Rc<Self>) {
        let Some(path) = store::settings_path() else { return };
        // A change made in the window a moment ago is still waiting to be written: it must not be read as old text.
        self.flush_settings();
        let text = match store::load(&path) {
            Ok(text) => text,
            Err(e) => {
                return self.show_message(&tf(Text::StatusSettingsLoadFailed, &[&e]), InfoBarSeverity::Warning);
            }
        };
        let parsed = settings::parse(text.as_deref());
        let built = keymap::build_keymap(commands::default_bindings(), &parsed.settings.keybindings);
        let problems: Vec<Problem> = parsed.problems.iter().chain(&built.problems).cloned().collect();
        let count = problems.len();
        *self.problems.borrow_mut() = problems;
        let before = self.settings.replace(parsed.settings);
        *self.keymap.borrow_mut() = built.keymap;
        self.apply_loaded_settings(&before);
        if count > 0 {
            self.show_settings_problems();
        } else {
            let _ = self.ui.message_bar.SetIsOpen(false);
        }
        log(&format!("settings reloaded from {}: {count} problems", path.display()));
    }

    /// The font of the settings reaches every pane: the renderer takes a new size at once, a new family or line
    /// height needs a new renderer. `before` is what the settings were.
    fn apply_font_settings(&self, before: &Settings) {
        let now = self.settings.borrow().clone();
        self.zoom_steps.set(0);
        self.look.font_pt.set(now.font.size as f32);
        self.look.line_height.set(now.font.line_height as f32);
        *self.look.families.borrow_mut() = now.font.family.clone();
        let new_renderer = now.font.family != before.font.family || now.font.line_height != before.font.line_height;
        for tab in self.tabs.borrow().iter() {
            for pane in tab.panes.borrow().iter() {
                pane.apply_look(new_renderer);
            }
        }
    }

    /// What a program may do with the clipboard (OSC 52) reaches the panes that are running.
    fn apply_osc52(&self) {
        let policy = logic::osc52_for(self.settings.borrow().osc52);
        for tab in self.tabs.borrow().iter() {
            for pane in tab.panes.borrow().iter() {
                pane.pane.set_osc52(policy);
            }
        }
    }

    /// The cursor of a program that chose none and the lines of scrollback reach the panes that are running.
    fn apply_terminal_settings(&self, effects: &logic::Effects) {
        let (style, lines) = {
            let settings = self.settings.borrow();
            (logic::cursor_style_for(&settings.cursor), settings.scrollback as usize)
        };
        for tab in self.tabs.borrow().iter() {
            for pane in tab.panes.borrow().iter() {
                if effects.cursor {
                    pane.pane.set_cursor_style(style);
                }
                if effects.scrollback {
                    pane.pane.set_scrollback_lines(lines);
                }
            }
        }
    }

    /// Brings what is on screen in line with `self.settings`, which were `before`.
    fn apply_loaded_settings(self: &Rc<Self>, before: &Settings) {
        // The colours, the backdrop and the opacity: what a slider asked for and was not applied yet is dropped.
        *self.pending_opacity.borrow_mut() = logic::PendingOpacity::default();
        if let Err(e) = self.apply_glass() {
            log(&format!("apply_glass: {e}"));
        }
        self.sync_appearance_controls();
        self.apply_font_settings(before);
        self.apply_osc52();
        let now = self.settings.borrow().clone();
        self.apply_terminal_settings(&logic::effects(before, &now));
        self.refresh_titles();
        // The shortcuts show in tooltips and menus, and the profiles of the file may differ.
        self.set_tooltips();
        *self.pane_menu.borrow_mut() = None;
        self.merge_profiles();
    }

    // ---------------------------------------------------------------------------------------------
    // Messages and dialogs
    // ---------------------------------------------------------------------------------------------

    pub fn show_message(&self, message: &str, severity: InfoBarSeverity) {
        let bar = &self.ui.message_bar;
        let _ = bar.SetSeverity(severity);
        let _ = bar.SetMessage(message);
        let _ = ui(&self.ui.message_action).and_then(|action| action.SetVisibility(Visibility::Collapsed));
        let _ = bar.SetIsOpen(true);
    }

    /// The shortcuts changed in the drawer: `edit` changes the bindings of the settings and is given the keymap that was
    /// in effect; the keymap is built again, tooltips and menus show the new shortcuts, and the save is scheduled.
    pub fn change_keybindings(self: &Rc<Self>, edit: impl FnOnce(&mut std::collections::BTreeMap<String, Vec<String>>, &Keymap)) {
        {
            let in_effect = self.keymap.borrow();
            edit(&mut self.settings.borrow_mut().keybindings, &in_effect);
        }
        let built = keymap::build_keymap(commands::default_bindings(), &self.settings.borrow().keybindings);
        *self.keymap.borrow_mut() = built.keymap;
        self.set_tooltips();
        *self.pane_menu.borrow_mut() = None;
        self.save_due.set(Some(Instant::now() + SAVE_DELAY));
        self.sync_drawer();
    }

    /// Keeps the message bar clear of the drawer: `width` DIP at the right of the body are not its to use.
    pub fn reserve_at_right(&self, width: f64) {
        let margin = Thickness { left: 16.0, top: 12.0, right: 16.0 + width, bottom: 0.0 };
        let _ = fe(&self.ui.message_bar).and_then(|bar| bar.SetMargin(margin));
    }

    /// The message of a settings file that has problems, with the action that opens the file; the drawer lists them.
    fn show_settings_problems(&self) {
        let count = self.problems.borrow().len();
        self.show_message(&tf(Text::StatusSettingsProblems, &[&count]), InfoBarSeverity::Warning);
        let action = &self.ui.message_action;
        let label = IReference::from(t(Text::AppOpenSettingsFile)).cast::<windows_core::IInspectable>();
        let _ = label.and_then(|label| action.cast::<ContentControl>()?.SetContent(&label));
        let _ = ui(action).and_then(|action| action.SetVisibility(Visibility::Visible));
        self.sync_drawer();
    }

    /// Asks before ending work; `title` names what closes, the buttons answer it.
    fn confirm_close(self: &Rc<Self>, title: String, on_confirm: impl Fn() + 'static) {
        self.confirm(title, Text::CommonClose, on_confirm);
    }

    /// Asks before doing something. `title` is the question; the button that answers it is labelled `action`, the
    /// same label the action has everywhere else, and Cancel is the other one.
    pub fn confirm(self: &Rc<Self>, title: String, action: Text, on_confirm: impl Fn() + 'static) {
        if let Err(e) = self.try_confirm(title, action, on_confirm) {
            log(&format!("confirm dialog: {e}"));
        }
    }

    fn try_confirm(self: &Rc<Self>, title: String, action: Text, on_confirm: impl Fn() + 'static) -> Result<()> {
        let dialog = ContentDialog::new()?;
        dialog.SetTitle(&IReference::from(title.as_str()))?;
        dialog.SetPrimaryButtonText(t(action))?;
        dialog.SetCloseButtonText(t(Text::CommonCancel))?;
        dialog.SetDefaultButton(ContentDialogButton::Close)?;
        ui(&dialog)?.SetXamlRoot(&self.root.xaml_root()?)?;
        let revoker = dialog.PrimaryButtonClick(cb!("dialog confirmed", |_, _| on_confirm()))?;
        self.revokers.borrow_mut().push(revoker);
        let this = self.clone();
        let revoker = dialog.Closed(cb!("dialog closed", |_, _| this.focus_active_pane()))?;
        self.revokers.borrow_mut().push(revoker);
        *self.dialog.borrow_mut() = Some(dialog.ShowAsync()?);
        Ok(())
    }

    /// The window was asked to close (Alt+F4, the caption button). Closing ends every session, so running ones are
    /// asked about first, as closing a tab does; the close goes on when the question is answered. True while the close
    /// is held back for the answer.
    pub fn ask_before_closing_window(self: &Rc<Self>) -> bool {
        let running: u32 = self.tabs.borrow().iter().map(|tab| tab.busy_sessions()).sum();
        if !self.settings.borrow().should_confirm_close(running) {
            return false;
        }
        let this = self.clone();
        let shown = self.try_confirm(tf(Text::DialogCloseSessions, &[&running]), Text::CommonClose, move || {
            window::allow_close();
            let _ = this.window.Close();
        });
        // A question that cannot be asked must not keep the window open.
        shown.map_err(|e| log(&format!("close confirmation: {e}"))).is_ok()
    }

    /// Asks for the name of a tab in a flyout under its chip, which keeps the chip as wide as it is. Enter, and a click
    /// elsewhere, put the name in effect; Esc leaves it as it was.
    fn rename_tab(self: &Rc<Self>, id: TabId) {
        let Some(index) = self.tab_index(id) else { return };
        self.select_index(index);
        let Some(tab) = self.tabs.borrow().get(index).cloned() else { return };
        let result = (|| -> Result<()> {
            let input = TextBox::new()?;
            input.SetText(&tab.title.borrow())?;
            fe(&input)?.SetWidth(RENAME_WIDTH)?;
            let flyout = Flyout::new()?;
            flyout.SetContent(&ui(&input)?)?;
            let base = flyout.cast::<FlyoutBase>()?;
            base.SetPlacement(FlyoutPlacementMode::Bottom)?;
            // The name is put in effect once, whichever of Enter and the closing of the flyout comes first.
            let done = Rc::new(Cell::new(false));
            let finish = {
                let (this, tab, input, done) = (self.clone(), tab.clone(), input.clone(), done.clone());
                move |keep: bool| {
                    if !done.replace(true) && keep {
                        this.apply_rename(&tab, &input.Text().unwrap_or_default());
                    }
                }
            };
            let (hide, ends) = (base.clone(), finish.clone());
            let keys = ui(&input)?.PreviewKeyDown(cb!("rename key", |_, args| {
                let Some(args) = args.as_ref() else { return };
                let key = args.Key().map_or(0, |key| key.0);
                if key == keyboard::VK_RETURN || key == keyboard::VK_ESCAPE {
                    let _ = args.SetHandled(true);
                    ends(key == keyboard::VK_RETURN);
                    let _ = hide.Hide();
                }
            }))?;
            let opened = {
                let input = input.clone();
                base.Opened(cb!("rename opened", |_, _| {
                    let _ = ui(&input).and_then(|input| input.Focus(FocusState::Programmatic));
                    let _ = input.SelectAll();
                }))?
            };
            let this = self.clone();
            let closed = base.Closed(cb!("rename closed", |_, _| {
                finish(true);
                this.focus_active_pane();
            }))?;
            self.revokers.borrow_mut().extend([keys, opened, closed]);
            base.ShowAt(&tab.chip.root)
        })();
        if let Err(e) = result {
            log(&format!("rename flyout: {e}"));
        }
    }

    /// The name the user typed for a tab: cleaned like any title, an empty one gives the title back to the rules, and
    /// one that is only what the rules made already changes nothing (the tab keeps following them).
    fn apply_rename(self: &Rc<Self>, tab: &Rc<Tab>, text: &str) {
        let name = logic::clean_title(text);
        if name.is_empty() {
            tab.renamed.set(false);
        } else if tab.renamed.get() || name != *tab.title.borrow() {
            tab.renamed.set(true);
            *tab.title.borrow_mut() = name.clone();
            tab.chip.set_title(&name);
        }
        self.refresh_titles();
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
        }
    }

    /// The settings drawer: made the first time it is wanted, because it needs a live `XamlRoot`.
    fn ensure_drawer(self: &Rc<Self>) -> Option<Rc<drawer::Drawer>> {
        self.ensure_overlays();
        self.drawer.borrow().clone()
    }

    /// Opens the drawer, at the row `target` names when there is one. Open already, it takes the focus.
    pub fn open_settings(self: &Rc<Self>, target: Option<&str>) {
        if let Some(drawer) = self.ensure_drawer() {
            drawer.open(target);
        }
    }

    /// The command and the menu row: closed, the drawer opens; open and focused, it closes; open and not focused (the
    /// focus is in a terminal), the focus goes to it.
    fn toggle_settings(self: &Rc<Self>) {
        let Some(drawer) = self.ensure_drawer() else { return };
        if drawer.is_open() && drawer.has_focus() {
            drawer.close();
        } else {
            drawer.open(None);
        }
    }

    fn toggle_overlay(self: &Rc<Self>, which: Overlay) {
        self.ensure_overlays();
        self.set_overlay(if self.overlay.get() == which { Overlay::None } else { which });
    }

    /// The overlays need a live `XamlRoot`, so they are built the first time one opens.
    fn ensure_overlays(self: &Rc<Self>) {
        let build = || -> Result<()> {
            // The drawer goes first: the palette and the find bar are above it.
            if self.drawer.borrow().is_none() {
                let drawer = drawer::Drawer::build(self)?;
                self.ui.body.cast::<Panel>()?.Children()?.Append(&drawer.root)?;
                *self.drawer.borrow_mut() = Some(drawer);
            }
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
        let focused = tab.as_ref().and_then(|tab| tab.focused_pane());
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
                Command::TabSelectLeft | Command::TabSelectRight => {
                    let count = self.tabs.borrow().len();
                    if let Some(index) =
                        Command::tab_neighbour(self.selected.get(), count, command == Command::TabSelectRight)
                    {
                        self.select_index(index);
                    }
                }
                Command::TabNextNeedsResponse => {
                    let waiting: Vec<bool> = self.tabs.borrow().iter().map(|tab| tab.needs_response()).collect();
                    if let Some(index) = logic::next_waiting(&waiting, self.selected.get()) {
                        self.go_to_tab(index);
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
                        self.refresh_status(tab);
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
                    if let Some(pane) = &focused {
                        self.paste_into(pane);
                    }
                }
                // Without a selection the key of smart copy never gets here: it reaches the program.
                Command::EditCopy | Command::EditSmartCopy => {
                    if let Some(pane) = &focused {
                        self.copy_selection(pane, true);
                    }
                }
                Command::EditSelectAll => {
                    if let Some(pane) = &focused {
                        pane.pane.select_all();
                    }
                }
                Command::EditClear => {
                    if let Some(pane) = &focused {
                        pane.pane.clear_buffer();
                    }
                }
                Command::ScrollPageUp | Command::ScrollPageDown | Command::ScrollTop | Command::ScrollBottom => {
                    if let Some(pane) = &focused {
                        pane.pane.scroll(match command {
                            Command::ScrollPageUp => Scroll::PageUp,
                            Command::ScrollPageDown => Scroll::PageDown,
                            Command::ScrollTop => Scroll::Top,
                            _ => Scroll::Bottom,
                        });
                    }
                }
                Command::PromptPrevious | Command::PromptNext => {
                    if let Some(pane) = &focused {
                        pane.pane.scroll_to_prompt(command == Command::PromptNext);
                    }
                }
                Command::EditFind => self.toggle_overlay(Overlay::Find),
                Command::AppPalette => self.toggle_overlay(Overlay::Palette),
                Command::AppSettings => self.toggle_settings(),
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
                Command::PaneRestart => {
                    if let Some(pane) = &focused {
                        self.request_restart(pane);
                    }
                }
                Command::AppOpenSettingsFile => self.open_settings_file(),
                Command::AppReloadSettings => self.reload_settings(),
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
                    self.go_to_tab(index);
                }
            }
            PaletteAction::NewTab(id) => {
                if let Some(profile) = self.profile_by_id(&id) {
                    let _ = self.add_tab(&profile);
                }
            }
            PaletteAction::SetColorScheme(name) => self.set_theme(&name),
            PaletteAction::OpenSetting(id) => self.open_settings(Some(&id)),
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
            // Esc during the drag of a chip puts the chips back where they were.
            if args.Key().map_or(0, |key| key.0) == keyboard::VK_ESCAPE
                && this.tabs.borrow().iter().any(|tab| tab.chip.cancel_drag())
            {
                let _ = args.SetHandled(true);
                return;
            }
            // The shortcuts page is waiting for a chord to give to a command: the chord is not a command now.
            if this.recording_keys.get() {
                return;
            }
            let code = args.Key().map_or(0, |key| key.0);
            let status = args.KeyStatus().unwrap_or_default();
            let held = keyboard::held_modifiers();
            let physical = keyboard::physical_code(code, &status);
            let command = physical.and_then(|physical| this.keymap.borrow().find(held, physical));
            let Some(command) = command else {
                // Bare modifier presses are not chords; everything else that matched nothing is worth a line.
                if (held.ctrl || held.alt) && !keyboard::is_modifier_key(code) {
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
            // Smart copy is Ctrl+C only while something is selected; otherwise the key is the program's interrupt.
            let nothing_to_copy = command == Command::EditSmartCopy && !(in_terminal && this.focused_has_selection());
            if nothing_to_copy || (!in_terminal && !command.works_outside_terminal()) {
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
            if keyboard::is_modifier_key(code) && keyboard::held_modifiers() == Default::default() {
                this.mru.borrow_mut().commit();
            }
        }))?;
        self.revokers.borrow_mut().push(revoker);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value of `attribute` on the element of `markup` whose `x:Name` is `name`.
    fn attribute(markup: &str, name: &str, attribute: &str) -> String {
        let marker = format!("x:Name=\"{name}\"");
        let start = markup.find(&marker).unwrap_or_else(|| panic!("the markup has no {marker}"));
        let rest = &markup[start..];
        let element = &rest[..rest.find('>').expect("the element ends")];
        let key = format!(" {attribute}=\"");
        let at = element.find(&key).unwrap_or_else(|| panic!("{name} has no {attribute}")) + key.len();
        element[at..].split('"').next().expect("the value ends").to_owned()
    }

    fn number(text: &str) -> f64 {
        text.parse().unwrap_or_else(|_| panic!("{text:?} is not a number"))
    }

    fn numbers(text: &str) -> Vec<f64> {
        text.split(',').map(|part| number(part.trim())).collect()
    }

    /// The width every tool button of the bar gets from its style.
    fn button_width() -> f64 {
        let style = include_str!("xaml/resources.xaml").split("x:Key=\"NkIconButton\"").nth(1).expect("the button style");
        let key = "Property=\"Width\" Value=\"";
        let at = style.find(key).expect("the style sets a width") + key.len();
        number(style[at..].split('"').next().expect("the value ends"))
    }

    #[test]
    fn the_numbers_the_bar_adds_up_are_the_ones_of_the_markup() {
        let bar = SHELL_XAML;
        // Margins are left,top,right,bottom.
        let mark = numbers(&attribute(bar, "AppMark", "Margin"));
        let group = numbers(&attribute(bar, "NewTabGroup", "Margin"));
        let cluster = numbers(&attribute(bar, "ToolCluster", "Margin"));
        let drag = number(&attribute(bar, "DragColumn", "MinWidth"));
        let fixed = mark[0]
            + number(&attribute(bar, "AppMark", "Width"))
            + mark[2]
            + group[0]
            + button_width()
            + number(&attribute(bar, "NewTabMenuButton", "Width"))
            + cluster[0]
            + cluster[2]
            + drag;
        assert_eq!(logic::BAR_FIXED_WIDTH, fixed);
        assert_eq!(logic::MIN_DRAG_WIDTH, drag);
        assert_eq!(logic::TOOL_BUTTON_WIDTH, button_width());
        // The padding of the group is one number: the same on both sides.
        assert_eq!(logic::CLUSTER_PADDING, 2.0 * number(&attribute(bar, "ToolCluster", "Padding")));
        let divider = numbers(&attribute(bar, "ToolDivider", "Margin"));
        assert_eq!(logic::DIVIDER_WIDTH, number(&attribute(bar, "ToolDivider", "Width")) + 2.0 * divider[0]);
    }

    #[test]
    fn the_bar_has_a_button_for_every_control_of_a_tier_and_none_for_find_appearance_or_settings() {
        for name in ["ToolSplitRight", "ToolSplitDown", "ToolPalette", "ToolMenu", "ToolDivider"] {
            assert!(SHELL_XAML.contains(&format!("x:Name=\"{name}\"")), "{name}");
        }
        for gone in ["ToolSearch", "ToolAppearance", "ToolSettings"] {
            assert!(!SHELL_XAML.contains(gone), "{gone} has a row in the menu now");
        }
    }
}