//! The settings page: a NavigationView on the left, cards on the right. WinUI 3 has no SettingsCard (that is
//! CommunityToolkit), so a card is the `xaml/card.xaml` template with a real control in its right slot.
//! Appearance is live: backdrop, opacity, colour scheme and font size apply as the control changes.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nekominal_model::{
    commands::Command,
    settings::{Backdrop, FONT_SIZE_RANGE, Language},
    strings::Text,
    themes,
};
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, HSTRING, Interface, Result};
use windows_reference::IReference;

use crate::glass::{self, Glass};
use crate::i18n::{localize, t, tf};
use crate::shell::Shell;
use crate::util::*;

const SETTINGS_XAML: &str = include_str!("xaml/settings.xaml");
const CARD_XAML: &str = include_str!("xaml/card.xaml");

struct CardInfo {
    page: &'static str,
    title: String,
    element: FrameworkElement,
}

struct Controls {
    theme: ComboBox,
    backdrop: ComboBox,
    opacity: Slider,
    opacity_text: TextBlock,
    font_size: NumberBox,
    profile: ComboBox,
    language: ComboBox,
    confirm_close: ToggleSwitch,
}

pub struct SettingsPage {
    pub root: FrameworkElement,
    shell: Weak<Shell>,
    nav: NavigationView,
    page: StackPanel,
    header: TextBlock,
    cards: RefCell<Vec<CardInfo>>,
    controls: RefCell<Option<Controls>>,
    /// Set while the controls are being brought in line with the settings, so that does not count as a change.
    syncing: Cell<bool>,
    revokers: RefCell<Vec<EventRevoker>>,
}

fn card(title: &str, control: &impl Interface) -> Result<FrameworkElement> {
    let root = load_xaml(CARD_XAML)?;
    find::<TextBlock>(&root, "CardTitle")?.SetText(title)?;
    let slot: ContentControl = find(&root, "CardSlot")?;
    slot.SetContent(&control.cast::<windows_core::IInspectable>()?)?;
    Ok(root)
}

fn combo(items: &[&str]) -> Result<ComboBox> {
    let combo = ComboBox::new()?;
    for item in items {
        combo.items()?.Append(&IReference::from(*item))?;
    }
    fe(&combo)?.SetMinWidth(200.0)?;
    Ok(combo)
}

fn toggle() -> Result<ToggleSwitch> {
    let toggle = ToggleSwitch::new()?;
    // The state text is part of the template; empty on/off content keeps the card compact.
    toggle.SetOnContent(&IReference::from(""))?;
    toggle.SetOffContent(&IReference::from(""))?;
    Ok(toggle)
}

fn tag_of(item: &impl Interface) -> String {
    item.cast::<FrameworkElement>()
        .and_then(|f| f.Tag())
        .and_then(|tag| tag.cast::<IReference<HSTRING>>())
        .and_then(|r| r.Value())
        .map(|h| h.to_string_lossy())
        .unwrap_or_default()
}

fn text_chip(text: &str) -> Result<FrameworkElement> {
    const CHIP: &str = r#"<Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" CornerRadius="{StaticResource NkCornerRadius}" Padding="9,3"
                   Background="{ThemeResource ControlFillColorSecondaryBrush}"><TextBlock Text="@TEXT@"/></Border>"#;
    load_xaml(&CHIP.replace("@TEXT@", &xml_escape(text)))
}

impl SettingsPage {
    pub fn build(shell: &Rc<Shell>) -> Result<Rc<Self>> {
        let root = load_xaml(&localize(SETTINGS_XAML))?;
        let nav: NavigationView = find(&root, "SettingsNav")?;
        let search: AutoSuggestBox = find(&root, "SettingsSearch")?;
        let page = Rc::new(Self {
            nav: nav.clone(),
            page: find(&root, "SettingsPage")?,
            header: find(&root, "SettingsHeader")?,
            root: root.clone(),
            shell: Rc::downgrade(shell),
            cards: RefCell::new(Vec::new()),
            controls: RefCell::new(None),
            syncing: Cell::new(false),
            revokers: RefCell::new(Vec::new()),
        });
        let mut revokers = Vec::new();
        page.build_cards(shell, &mut revokers)?;

        let this = page.clone();
        revokers.push(nav.SelectionChanged(cb!("settings nav", |_, args| {
            let Some(args) = args.as_ref() else { return };
            let Ok(item) =
                args.cast::<INavigationViewSelectionChangedEventArgs2>().and_then(|a| a.SelectedItemContainer())
            else {
                return;
            };
            this.show_page(&tag_of(&item));
        }))?);
        let this = page.clone();
        revokers.push(search.TextChanged(cb!("settings search", |sender, _| {
            let query = sender.as_ref().and_then(|b| b.Text().ok()).unwrap_or_default();
            if query.is_empty() {
                this.show_page(&this.current_page());
            } else {
                this.show_results(&query.to_lowercase());
            }
        }))?);

        let first: NavigationViewItem = find(&root, "NavGeneral")?;
        nav.SetSelectedItem(&first)?;
        *page.revokers.borrow_mut() = revokers;
        page.restyle(&shell.glass.get());
        page.sync();
        Ok(page)
    }

    pub fn show(&self, shown: bool) {
        let _ = ui(&self.root).and_then(|root| root.SetVisibility(visibility(shown)));
    }

    /// Moves the keyboard focus off the terminal and onto the page.
    pub fn focus(&self) {
        // The page was collapsed a moment ago: it must be laid out before it can take the focus.
        let _ = ui(&self.root).and_then(|root| root.UpdateLayout());
        let target = self.nav.SelectedItem().ok().and_then(|item| item.cast::<UIElement>().ok());
        let _ = match target {
            Some(target) => target.Focus(FocusState::Programmatic),
            None => ui(&self.nav).and_then(|nav| nav.Focus(FocusState::Programmatic)),
        };
    }

    pub fn restyle(&self, _glass: &Glass) {
        // Cards use theme brushes, which follow the window theme the shell already switched.
    }

    fn current_page(&self) -> String {
        self.nav
            .SelectedItem()
            .ok()
            .map(|item| tag_of(&item))
            .filter(|tag| !tag.is_empty())
            .unwrap_or_else(|| "general".into())
    }

    fn show_page(&self, page: &str) {
        let Ok(children) = self.page.cast::<Panel>().and_then(|p| p.Children()) else { return };
        let _ = children.Clear();
        let title = match page {
            "general" => Text::SettingsGeneral,
            "appearance" => Text::SettingsAppearance,
            "profiles" => Text::SettingsProfiles,
            "keys" => Text::SettingsKeys,
            _ => Text::SettingsAbout,
        };
        let _ = self.header.SetText(t(title));
        for card in self.cards.borrow().iter().filter(|c| c.page == page) {
            let _ = children.Append(&card.element);
        }
    }

    fn show_results(&self, query: &str) {
        let Ok(children) = self.page.cast::<Panel>().and_then(|p| p.Children()) else { return };
        let _ = children.Clear();
        let _ = self.header.SetText(t(Text::AppSettings));
        for card in self.cards.borrow().iter().filter(|c| c.title.to_lowercase().contains(query)) {
            let _ = children.Append(&card.element);
        }
    }

    fn push(&self, page: &'static str, title: &str, control: &impl Interface) -> Result<()> {
        let element = card(title, control)?;
        self.cards.borrow_mut().push(CardInfo { page, title: title.to_owned(), element });
        Ok(())
    }

    /// Brings every control in line with the settings.
    pub fn sync(&self) {
        let Some(shell) = self.shell.upgrade() else { return };
        let controls = self.controls.borrow();
        let Some(c) = controls.as_ref() else { return };
        self.syncing.set(true);
        let settings = shell.settings.borrow();
        let _ = c
            .theme
            .set_selected_index(themes::theme_names().position(|name| name == settings.theme).map_or(-1, |i| i as i32));
        let _ = c.backdrop.set_selected_index(settings.appearance.backdrop.index());
        let (percent, enabled) = glass::opacity_control(settings.appearance.backdrop, shell.opacity_latest());
        let _ = c.opacity.cast::<RangeBase>().and_then(|range| range.SetValue(percent));
        let _ = c.opacity.cast::<Control>().and_then(|slider| slider.SetIsEnabled(enabled));
        let _ = c.opacity_text.SetText(&format!("{percent:.0}%"));
        let _ = c.font_size.SetValue(settings.font.size);
        let profile = shell.profiles.borrow().iter().position(|p| p.id == settings.default_profile).unwrap_or(0);
        let _ = c.profile.set_selected_index(profile as i32);
        let _ = c.language.set_selected_index(match settings.language {
            Language::Auto => 0,
            Language::ZhTw => 1,
            Language::EnUs => 2,
        });
        let _ = c.confirm_close.SetIsOn(settings.confirm_close);
        self.syncing.set(false);
    }

    fn build_cards(self: &Rc<Self>, shell: &Rc<Shell>, revokers: &mut Vec<EventRevoker>) -> Result<()> {
        // General
        let profile_names: Vec<String> = shell.profiles.borrow().iter().map(|p| p.name.clone()).collect();
        let profile = combo(&profile_names.iter().map(String::as_str).collect::<Vec<_>>())?;
        let this = self.clone();
        revokers.push(profile.cast::<Selector>()?.SelectionChanged(cb!("default profile", |sender, _| {
            let (Some(shell), false) = (this.shell.upgrade(), this.syncing.get()) else { return };
            let index = sender.as_ref().and_then(|c| c.selected_index().ok()).unwrap_or(-1);
            let id = usize::try_from(index).ok().and_then(|i| shell.profiles.borrow().get(i).map(|p| p.id.clone()));
            if let Some(id) = id {
                shell.settings.borrow_mut().default_profile = id;
                shell.settings_changed();
            }
        }))?);
        self.push("general", t(Text::SettingsDefaultProfile), &profile)?;

        let language = combo(&[t(Language::Auto.text()), t(Language::ZhTw.text()), t(Language::EnUs.text())])?;
        let this = self.clone();
        revokers.push(language.cast::<Selector>()?.SelectionChanged(cb!("language", |sender, _| {
            let (Some(shell), false) = (this.shell.upgrade(), this.syncing.get()) else { return };
            let index = sender.as_ref().and_then(|c| c.selected_index().ok()).unwrap_or(0);
            // The interface language is read at start-up.
            shell.settings.borrow_mut().language =
                [Language::Auto, Language::ZhTw, Language::EnUs][usize::try_from(index).unwrap_or(0).min(2)];
            shell.settings_changed();
        }))?);
        self.push("general", t(Text::SettingsLanguage), &language)?;

        let confirm_close = toggle()?;
        let this = self.clone();
        revokers.push(confirm_close.Toggled(cb!("confirm close", |sender, _| {
            let (Some(shell), false) = (this.shell.upgrade(), this.syncing.get()) else { return };
            let on =
                sender.as_ref().and_then(|s| s.cast::<ToggleSwitch>().ok()).and_then(|s| s.IsOn().ok()).unwrap_or(true);
            shell.settings.borrow_mut().confirm_close = on;
            shell.settings_changed();
        }))?);
        self.push("general", t(Text::SettingsConfirmClose), &confirm_close)?;

        // Appearance: every control applies as it changes.
        let theme_names: Vec<&str> = themes::theme_names().collect();
        let theme = combo(&theme_names)?;
        let this = self.clone();
        revokers.push(theme.cast::<Selector>()?.SelectionChanged(cb!("theme", |sender, _| {
            let (Some(shell), false) = (this.shell.upgrade(), this.syncing.get()) else { return };
            let index = sender.as_ref().and_then(|c| c.selected_index().ok()).unwrap_or(-1);
            if let Some(name) = usize::try_from(index).ok().and_then(|i| themes::theme_names().nth(i)) {
                shell.set_theme(name);
            }
        }))?);
        self.push("appearance", t(Text::SettingsTheme), &theme)?;

        let backdrop = combo(&Backdrop::ALL.map(|choice| t(choice.text())))?;
        let this = self.clone();
        revokers.push(backdrop.cast::<Selector>()?.SelectionChanged(cb!("backdrop", |sender, _| {
            let (Some(shell), false) = (this.shell.upgrade(), this.syncing.get()) else { return };
            let index = sender.as_ref().and_then(|c| c.selected_index().ok()).unwrap_or(-1);
            if let Some(choice) = Backdrop::from_index(index) {
                shell.set_backdrop(choice);
            }
        }))?);
        self.push("appearance", t(Text::SettingsBackdrop), &backdrop)?;

        let row = load_xaml(
            r#"<StackPanel xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                          xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Orientation="Horizontal" Spacing="12">
                 <Slider x:Name="Opacity" Width="190" Minimum="20" Maximum="100" StepFrequency="1" SmallChange="1" LargeChange="5"
                         IsThumbToolTipEnabled="False"/>
                 <TextBlock x:Name="OpacityText" MinWidth="40" VerticalAlignment="Center"/>
               </StackPanel>"#,
        )?;
        let opacity: Slider = find(&row, "Opacity")?;
        let opacity_text: TextBlock = find(&row, "OpacityText")?;
        let range: RangeBase = opacity.cast()?;
        let this = self.clone();
        revokers.push(range.ValueChanged(cb!("opacity", |_, args| {
            let (Some(shell), false) = (this.shell.upgrade(), this.syncing.get()) else { return };
            let Some(value) = args.as_ref().and_then(|a| a.NewValue().ok()) else { return };
            shell.set_opacity(value / 100.0);
        }))?);
        self.push("appearance", t(Text::SettingsOpacity), &row)?;

        let font_size = NumberBox::new()?;
        font_size.SetMinimum(FONT_SIZE_RANGE.0)?;
        font_size.SetMaximum(FONT_SIZE_RANGE.1)?;
        font_size.SetSpinButtonPlacementMode(NumberBoxSpinButtonPlacementMode::Inline)?;
        fe(&font_size)?.SetMinWidth(140.0)?;
        let this = self.clone();
        revokers.push(font_size.ValueChanged(cb!("font size", |sender, _| {
            let (Some(shell), false) = (this.shell.upgrade(), this.syncing.get()) else { return };
            let value = sender.as_ref().and_then(|b| b.Value().ok()).unwrap_or(f64::NAN);
            if value.is_finite() && (value - shell.settings.borrow().font.size).abs() > 0.01 {
                shell.set_font_size(value);
            }
        }))?);
        self.push("appearance", t(Text::SettingsFontSize), &font_size)?;

        // Profiles: what a new tab can open.
        for profile in shell.profiles.borrow().iter() {
            self.push("profiles", &profile.name, &text_chip(&profile.program)?)?;
        }

        // Keys: the shortcuts in effect, from the same table the keyboard layer and the palette use.
        let lang = shell.lang();
        for command in Command::ALL.iter().copied().filter(|c| c.in_palette()) {
            let shortcut = shell.keymap.borrow().shortcut_text(command);
            if !shortcut.is_empty() {
                self.push("keys", &command.label(lang), &text_chip(&shortcut)?)?;
            }
        }

        // About
        self.push("about", t(Text::SettingsVersion), &text_chip(env!("CARGO_PKG_VERSION"))?)?;
        if let Ok(backend) = nekominal_pty::conpty::backend() {
            let conpty = if backend.kind == nekominal_pty::conpty::Backend::BUNDLED {
                tf(Text::ConptyBundled, &[&backend.version])
            } else {
                t(Text::ConptySystem).to_owned()
            };
            self.push("about", t(Text::SettingsConpty), &text_chip(&conpty)?)?;
        }
        if let Some(path) = nekominal_model::store::settings_path() {
            self.push("about", t(Text::SettingsFile), &text_chip(&path.display().to_string())?)?;
        }

        *self.controls.borrow_mut() =
            Some(Controls { theme, backdrop, opacity, opacity_text, font_size, profile, language, confirm_close });
        Ok(())
    }
}
