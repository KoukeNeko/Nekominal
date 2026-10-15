//! The rows of the settings drawer: one `SettingRow` per entry of `nekominal_model::settings_ui::TABLE`, built with the
//! control its `Control` names, and the read-only rows of the pages that list things (the shortcuts, the version).
//!
//! A row is a label, the control, and a reset button that is there (not hidden, so nothing moves) only while the row
//! differs from its default; a modified row also has a 2 DIP bar of the accent at the left edge of the sheet. Every
//! control applies as it changes: it calls `Drawer::commit`, which hands the value to the shell and brings the sheet
//! back in line with the settings.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nekominal_model::settings::Settings;
use nekominal_model::settings_ui::{self, Control as RowControl, Descriptor, Range, Value};
use nekominal_model::strings::Text;
use nekominal_model::themes;
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use crate::drawer::Drawer;
use crate::drawer_pages::Extras;
use crate::i18n::{t, tf};
use crate::keyboard::{self, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_PAGE_DOWN, VK_PAGE_UP, VK_RETURN, VK_RIGHT, VK_UP};
use crate::logic::{self, Arrow};
use crate::shell::Shell;
use crate::util::*;

/// A row of one line: label, control, reset. A wide row (the colour schemes, the profiles) has its control under the
/// label across the whole width.
const ROW_XAML: &str = r#"
<Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        MinHeight="40" BorderThickness="0,0,0,1" BorderBrush="{StaticResource NkDivider}">
  <Grid ColumnSpacing="8">
    <Grid.ColumnDefinitions>
      <ColumnDefinition Width="*"/>
      <ColumnDefinition Width="Auto"/>
      <ColumnDefinition Width="32"/>
    </Grid.ColumnDefinitions>
    <Grid.RowDefinitions>
      <RowDefinition Height="40"/>
      <RowDefinition Height="Auto"/>
    </Grid.RowDefinitions>
    <Border x:Name="Bar" Grid.RowSpan="2" Width="2" Margin="-15,8,0,8" HorizontalAlignment="Left" Opacity="0"
            IsHitTestVisible="False" Background="{StaticResource NkPanelAccent}"/>
    <Grid ColumnSpacing="8">
      <Grid.ColumnDefinitions>
        <ColumnDefinition Width="*"/>
        <ColumnDefinition Width="Auto"/>
        <ColumnDefinition Width="Auto"/>
      </Grid.ColumnDefinitions>
      <TextBlock x:Name="Label" FontSize="13" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"
                 Foreground="{StaticResource NkPanelText}"/>
      <!-- The section of the row, said while the sheet shows what a search found and not a section. -->
      <TextBlock x:Name="SectionTag" Grid.Column="1" FontSize="12" VerticalAlignment="Center" Visibility="Collapsed"
                 Foreground="{StaticResource NkPanelTextSecondary}"/>
      <!-- Said of a row whose value in the settings file was not valid. -->
      <TextBlock x:Name="Tag" Grid.Column="2" FontSize="12" VerticalAlignment="Center" Visibility="Collapsed"
                 Foreground="{StaticResource NkPanelWarning}"/>
    </Grid>
    <Border x:Name="Slot" @SLOT@/>
    <Button x:Name="Reset" Grid.Column="2" Style="{StaticResource NkIconButton}" Opacity="0" IsEnabled="False">
      <FontIcon Glyph="&#xE7A7;" FontSize="12"/>
    </Button>
  </Grid>
</Border>"#;

const SLOT_LINE: &str = r#"Grid.Column="1" VerticalAlignment="Center" HorizontalAlignment="Right""#;
const SLOT_WIDE: &str = r#"Grid.Row="1" Grid.ColumnSpan="3" HorizontalAlignment="Stretch" Margin="0,0,0,10""#;

/// Everything about a row that decides what its control shows, kept to see what changed.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    value: Value,
    enabled: bool,
    text: String,
    /// The profiles of the list, as (id, name and program).
    items: Vec<(String, String)>,
    modified: bool,
    /// The settings file has a value for the row that is not valid.
    invalid: bool,
}

enum Kind {
    /// An off/on switch or a few named options: one segment each.
    Choices(Vec<ToggleButton>),
    Slider {
        slider: Slider,
        value: TextBlock,
    },
    Number {
        field: TextBox,
    },
    Tiles(Vec<ToggleButton>),
    Profiles(Rc<ProfileList>),
    Text {
        field: TextBox,
    },
}

/// The profiles of the default profile row: one button each, built again when the profiles change (discovery
/// finishes after start-up).
struct ProfileList {
    list: StackPanel,
    buttons: RefCell<Vec<ToggleButton>>,
    ids: RefCell<Vec<String>>,
    focused: Cell<usize>,
    /// The handlers of the buttons; they go when the buttons do.
    revokers: RefCell<Vec<EventRevoker>>,
}

pub struct SettingRow {
    pub descriptor: &'static Descriptor,
    pub root: FrameworkElement,
    bar: UIElement,
    reset: Button,
    tag: TextBlock,
    /// Takes the focus when a search or the palette brings the user to the row.
    pub focus: UIElement,
    kind: Kind,
    last: RefCell<Option<Shown>>,
    owner: Weak<Drawer>,
    _revokers: Vec<EventRevoker>,
}

impl SettingRow {
    /// Moves the focus to the tile of the colour scheme `name`, when this is the row of the colour schemes; true when
    /// it was.
    pub fn focus_tile(&self, name: &str) -> bool {
        let Kind::Tiles(tiles) = &self.kind else { return false };
        let Some(at) = themes::theme_names().position(|candidate| candidate == name) else { return false };
        focus_item(tiles, at);
        true
    }
}

/// A toggle button of the style `style` (`resources.xaml`) with a word on it.
pub fn toggle_button(style: &str, content: &str) -> Result<ToggleButton> {
    let markup = format!(
        r#"<ToggleButton xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Style="{{StaticResource {style}}}" Content="{}"/>"#,
        xml_escape(content)
    );
    load_xaml(&markup)?.cast()
}

/// The value a control hands over when the option at `index` is picked.
fn picked(control: RowControl, index: usize) -> Value {
    match control {
        RowControl::Toggle => Value::Bool(index == 1),
        _ => Value::Index(index),
    }
}

/// The position of the option a value stands for.
fn option_of(value: &Value) -> Option<usize> {
    match value {
        Value::Bool(on) => Some(usize::from(*on)),
        Value::Index(index) => Some(*index),
        _ => None,
    }
}

fn arrow_of(key: i32) -> Option<Arrow> {
    Some(match key {
        VK_LEFT => Arrow::Left,
        VK_RIGHT => Arrow::Right,
        VK_UP => Arrow::Up,
        VK_DOWN => Arrow::Down,
        VK_HOME => Arrow::Home,
        VK_END => Arrow::End,
        _ => return None,
    })
}

pub(crate) fn stack(horizontal: bool, spacing: f64) -> Result<StackPanel> {
    let markup = format!(
        r#"<StackPanel xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Orientation="{}" Spacing="{spacing}"/>"#,
        if horizontal { "Horizontal" } else { "Vertical" }
    );
    load_xaml(&markup)?.cast()
}

impl Drawer {
    /// A control changed a row: the shell puts the value in effect and the sheet follows the settings.
    pub(crate) fn commit(&self, row: &'static Descriptor, value: Value) {
        if self.syncing.get() {
            return;
        }
        let Some(shell) = self.shell.upgrade() else { return };
        shell.change_setting(row, value);
        self.sync();
    }

    /// Writes into a field what the settings hold for the row, whatever the field says now.
    fn show_setting(&self, row: &'static Descriptor, field: &TextBox) {
        let Some(shell) = self.shell.upgrade() else { return };
        let shown = match (row.get)(&shell.settings.borrow()) {
            Value::Number(n) => logic::number_text(n),
            Value::Text(text) => text,
            _ => return,
        };
        let _ = field.SetText(&shown);
    }

    /// Arrow keys among `buttons`, which are one stop of the Tab key: the key moves the focus (and the stop) and
    /// `pick` is told where it went. Returns the handlers, which live as long as the row.
    fn roving(
        self: &Rc<Self>,
        container: &UIElement,
        buttons: &[ToggleButton],
        columns: usize,
        pick: impl Fn(usize) + 'static,
    ) -> Result<Vec<EventRevoker>> {
        let focused = Rc::new(Cell::new(0usize));
        let (all, seen) = (buttons.to_vec(), focused.clone());
        let mut revokers = vec![container.PreviewKeyDown(cb!("drawer roving", |_, args| {
            let Some(args) = args.as_ref() else { return };
            let Some(arrow) = arrow_of(args.Key().map_or(0, |key| key.0)) else { return };
            // Ctrl and Alt with an arrow are commands of the window (Alt+Left moves between panes).
            let held = keyboard::held_modifiers();
            if held.ctrl || held.alt {
                return;
            }
            if let Some(target) = logic::roving_target(seen.get(), all.len(), columns, arrow) {
                let _ = args.SetHandled(true);
                focus_item(&all, target);
                pick(target);
            }
        }))?];
        for (index, button) in buttons.iter().enumerate() {
            let (all, focused) = (buttons.to_vec(), focused.clone());
            revokers.push(ui(button)?.GotFocus(cb!("drawer roving focus", |_, _| {
                focused.set(index);
                set_tab_stop(&all, index);
            }))?);
        }
        Ok(revokers)
    }
}

/// Makes `at` the one button of the group that the Tab key stops at.
fn set_tab_stop(buttons: &[ToggleButton], at: usize) {
    for (index, button) in buttons.iter().enumerate() {
        let _ = ui(button).and_then(|button| button.SetIsTabStop(index == at));
    }
}

/// Gives the focus to one button of a group that is a single stop of the Tab key. A button that is not that stop cannot
/// take the focus, so it becomes the stop first.
pub fn focus_item(buttons: &[ToggleButton], at: usize) {
    set_tab_stop(buttons, at);
    if let Some(button) = buttons.get(at) {
        let _ = ui(button).and_then(|button| button.Focus(FocusState::Keyboard));
    }
}
/// Builds every row of the table and the pages that list things.
pub fn build(drawer: &Rc<Drawer>, shell: &Rc<Shell>) -> Result<()> {
    let mut rows = Vec::new();
    for descriptor in settings_ui::TABLE {
        match build_row(drawer, shell, descriptor) {
            Ok(row) => rows.push(Rc::new(row)),
            Err(e) => log(&format!("drawer row {}: {e}", descriptor.id)),
        }
    }
    *drawer.rows.borrow_mut() = rows;
    *drawer.extras.borrow_mut() = Extras::build(shell);
    Ok(())
}

/// Says the section of a row after its label (`Some`), or nothing (`None`): a search finds rows of every section, and
/// each says where it is; a section shows its own rows and needs no tag.
pub(crate) fn set_section_tag(row: &FrameworkElement, section: Option<&str>) {
    if let Ok(tag) = find::<TextBlock>(row, "SectionTag") {
        let _ = tag.SetText(section.unwrap_or_default());
        let _ = ui(&tag).and_then(|tag| tag.SetVisibility(visibility(section.is_some())));
    }
}

pub(crate) fn row_frame(label: &str, wide: bool) -> Result<(FrameworkElement, Border, UIElement, Button)> {
    let markup = ROW_XAML.replace("@SLOT@", if wide { SLOT_WIDE } else { SLOT_LINE });
    let root = load_xaml(&markup)?;
    find::<TextBlock>(&root, "Label")?.SetText(label)?;
    let bar: UIElement = find(&root, "Bar")?;
    let reset: Button = find(&root, "Reset")?;
    let slot: Border = find(&root, "Slot")?;
    Ok((root, slot, bar, reset))
}

fn build_row(drawer: &Rc<Drawer>, shell: &Rc<Shell>, descriptor: &'static Descriptor) -> Result<SettingRow> {
    let label = descriptor.label(shell.lang());
    let wide = matches!(descriptor.control, RowControl::Themes | RowControl::Profiles);
    let (root, slot, bar, reset) = row_frame(label, wide)?;
    let tag: TextBlock = find(&root, "Tag")?;
    tag.SetText(t(Text::SettingInvalid))?;
    let mut revokers = Vec::new();

    let (content, focus, kind): (UIElement, UIElement, Kind) = match descriptor.control {
        RowControl::Toggle => {
            choices(drawer, descriptor, label, &[t(Text::ToggleOff), t(Text::ToggleOn)], &mut revokers)?
        }
        RowControl::Choice(options) => {
            let labels: Vec<&str> = options.iter().map(|option| t(*option)).collect();
            choices(drawer, descriptor, label, &labels, &mut revokers)?
        }
        RowControl::Slider(range) => slider(drawer, descriptor, range, label, &mut revokers)?,
        RowControl::Number(range) => number(drawer, descriptor, range, label, &mut revokers)?,
        RowControl::Themes => tiles(drawer, descriptor, &mut revokers)?,
        RowControl::Profiles => profiles(drawer, descriptor, &mut revokers)?,
        RowControl::Text => text(drawer, descriptor, label, &mut revokers)?,
    };
    slot.SetChild(&content)?;

    let name = tf(Text::LabelValue, &[&t(Text::CommonRestoreDefault), &label]);
    AutomationProperties::SetName(&reset, &name)?;
    ToolTipService::SetToolTip(&reset, &IReference::from(t(Text::CommonRestoreDefault)))?;
    let this = drawer.clone();
    revokers.push(reset.on_click(cb!("drawer reset", |_, _| {
        this.commit(descriptor, (descriptor.get)(&Settings::default()));
    }))?);

    Ok(SettingRow {
        descriptor,
        root,
        bar,
        reset,
        tag,
        focus,
        kind,
        last: RefCell::new(None),
        owner: Rc::downgrade(drawer),
        _revokers: revokers,
    })
}

/// Off/on and the few named options: a row of segments, one tab stop, the arrow keys pick as they move.
fn choices(
    drawer: &Rc<Drawer>,
    descriptor: &'static Descriptor,
    row_label: &str,
    labels: &[&str],
    revokers: &mut Vec<EventRevoker>,
) -> Result<(UIElement, UIElement, Kind)> {
    let panel = stack(true, 4.0)?;
    let mut buttons = Vec::new();
    for label in labels {
        let button = toggle_button("NkSegment", label)?;
        // A reader says which row a choice belongs to ("Cursor blink: On").
        AutomationProperties::SetName(&button, &tf(Text::LabelValue, &[&row_label, label]))?;
        panel.cast::<Panel>()?.Children()?.Append(&button)?;
        buttons.push(button);
    }
    let this = drawer.clone();
    let keys = drawer.roving(&ui(&panel)?, &buttons, buttons.len(), move |target| {
        this.commit(descriptor, picked(descriptor.control, target));
    })?;
    revokers.extend(keys);
    for (index, button) in buttons.iter().enumerate() {
        let this = drawer.clone();
        revokers.push(button.on_click(cb!("drawer choice", |_, _| {
            this.commit(descriptor, picked(descriptor.control, index));
            // A click on the choice that is on has just switched it off.
            this.sync();
        }))?);
    }
    let first = ui(&buttons[0])?;
    Ok((ui(&panel)?, first, Kind::Choices(buttons)))
}

fn slider(
    drawer: &Rc<Drawer>,
    descriptor: &'static Descriptor,
    range: Range,
    label: &str,
    revokers: &mut Vec<EventRevoker>,
) -> Result<(UIElement, UIElement, Kind)> {
    let markup = format!(
        r#"<Grid xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" ColumnSpacing="8">
             <Grid.ColumnDefinitions><ColumnDefinition Width="200"/><ColumnDefinition Width="48"/></Grid.ColumnDefinitions>
             <Slider x:Name="Track" Style="{{StaticResource NkSlider}}" Minimum="{}" Maximum="{}" StepFrequency="{}" SmallChange="{}"
                     LargeChange="{}" VerticalAlignment="Center"/>
             <TextBlock x:Name="Value" Grid.Column="1" FontSize="13" FontFamily="Segoe UI Variable Text" TextAlignment="Right"
                        VerticalAlignment="Center" Foreground="{{StaticResource NkPanelText}}"/>
           </Grid>"#,
        range.min, range.max, range.step, range.step, range.large
    );
    let root = load_xaml(&markup)?;
    let slider: Slider = find(&root, "Track")?;
    AutomationProperties::SetName(&slider, label)?;
    let this = drawer.clone();
    revokers.push(slider.cast::<RangeBase>()?.ValueChanged(cb!("drawer slider", |_, args| {
        let Some(value) = args.as_ref().and_then(|args| args.NewValue().ok()) else { return };
        this.commit(descriptor, Value::Number(value));
    }))?);
    // The slider moves by one step with the arrow keys and goes to the ends with Home and End; Page Up and Page Down,
    // which it does not know, move it by the large step.
    let (this, track) = (drawer.clone(), slider.clone());
    revokers.push(ui(&slider)?.PreviewKeyDown(cb!("drawer slider key", |_, args| {
        let Some(args) = args.as_ref() else { return };
        let direction = match args.Key().map_or(0, |key| key.0) {
            VK_PAGE_UP => 1.0,
            VK_PAGE_DOWN => -1.0,
            _ => return,
        };
        let _ = args.SetHandled(true);
        let now = track.cast::<RangeBase>().and_then(|range| range.Value()).unwrap_or(range.min);
        this.commit(descriptor, Value::Number((now + direction * range.large).clamp(range.min, range.max)));
    }))?);
    let focus = ui(&slider)?;
    Ok((ui(&root)?, focus, Kind::Slider { slider, value: find(&root, "Value")? }))
}

fn number(
    drawer: &Rc<Drawer>,
    descriptor: &'static Descriptor,
    range: Range,
    label: &str,
    revokers: &mut Vec<EventRevoker>,
) -> Result<(UIElement, UIElement, Kind)> {
    let markup = r#"<StackPanel xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Orientation="Horizontal" Spacing="4">
          <Button x:Name="Down" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Style="{StaticResource NkIconButton}"><FontIcon Glyph="&#xE738;" FontSize="11"/></Button>
          <TextBox x:Name="Field" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Width="84" Height="32" TextAlignment="Center"
                   CornerRadius="{StaticResource NkCornerRadius}"/>
          <Button x:Name="Up" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Style="{StaticResource NkIconButton}"><FontIcon Glyph="&#xE710;" FontSize="11"/></Button>
        </StackPanel>"#;
    let root = load_xaml(markup)?;
    let field: TextBox = find(&root, "Field")?;
    AutomationProperties::SetName(&field, label)?;
    let (down, up): (Button, Button) = (find(&root, "Down")?, find(&root, "Up")?);
    AutomationProperties::SetName(&down, &tf(Text::LabelValue, &[&label, &t(Text::CommonDecrease)]))?;
    AutomationProperties::SetName(&up, &tf(Text::LabelValue, &[&label, &t(Text::CommonIncrease)]))?;

    // The value in the field, or the one of the settings when the text is not a number.
    let current = {
        let (this, field) = (drawer.clone(), field.clone());
        move || -> f64 {
            let typed = logic::parse_number(&field.Text().unwrap_or_default());
            typed
                .or_else(|| match this.shell.upgrade().map(|shell| (descriptor.get)(&shell.settings.borrow())) {
                    Some(Value::Number(n)) => Some(n),
                    _ => None,
                })
                .unwrap_or(range.min)
        }
    };
    let step = {
        let (this, current) = (drawer.clone(), current.clone());
        move |steps: i32| this.commit(descriptor, Value::Number(logic::step_number(current(), steps, range)))
    };
    let (by, minus) = (step.clone(), step.clone());
    revokers.push(up.on_click(cb!("drawer number up", |_, _| by(1)))?);
    revokers.push(down.on_click(cb!("drawer number down", |_, _| minus(-1)))?);

    // Enter and leaving the field apply what was typed; the arrow keys step it and Page Up and Down take big steps.
    let apply = {
        let (this, field) = (drawer.clone(), field.clone());
        move || {
            if let Some(typed) = logic::parse_number(&field.Text().unwrap_or_default()) {
                this.commit(descriptor, Value::Number(typed));
            }
            // What was typed that is not a number, or was kept inside the range, becomes what the settings hold.
            this.show_setting(descriptor, &field);
        }
    };
    let (on_enter, on_leave) = (apply.clone(), apply);
    let large = (range.large / range.step).round() as i32;
    revokers.push(ui(&field)?.PreviewKeyDown(cb!("drawer number key", |_, args| {
        let Some(args) = args.as_ref() else { return };
        let moved = match args.Key().map_or(0, |key| key.0) {
            VK_UP => Some(1),
            VK_DOWN => Some(-1),
            VK_PAGE_UP => Some(large),
            VK_PAGE_DOWN => Some(-large),
            VK_RETURN => {
                let _ = args.SetHandled(true);
                on_enter();
                return;
            }
            _ => None,
        };
        if let Some(steps) = moved {
            let _ = args.SetHandled(true);
            step(steps);
        }
    }))?);
    revokers.push(ui(&field)?.LostFocus(cb!("drawer number leave", |_, _| on_leave()))?);
    let focus = ui(&field)?;
    Ok((ui(&root)?, focus, Kind::Number { field }))
}

fn text(
    drawer: &Rc<Drawer>,
    descriptor: &'static Descriptor,
    label: &str,
    revokers: &mut Vec<EventRevoker>,
) -> Result<(UIElement, UIElement, Kind)> {
    let field: TextBox = load_xaml(
        r#"<TextBox xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Width="236" Height="32"
                    CornerRadius="{StaticResource NkCornerRadius}"/>"#,
    )?
    .cast()?;
    AutomationProperties::SetName(&field, label)?;
    let apply = {
        let (this, field) = (drawer.clone(), field.clone());
        move || {
            this.commit(descriptor, Value::Text(field.Text().unwrap_or_default()));
            this.show_setting(descriptor, &field);
        }
    };
    let (on_enter, on_leave) = (apply.clone(), apply);
    revokers.push(ui(&field)?.PreviewKeyDown(cb!("drawer text key", |_, args| {
        let Some(args) = args.as_ref() else { return };
        if args.Key().map_or(0, |key| key.0) == VK_RETURN {
            let _ = args.SetHandled(true);
            on_enter();
        }
    }))?);
    revokers.push(ui(&field)?.LostFocus(cb!("drawer text leave", |_, _| on_leave()))?);
    let focus = ui(&field)?;
    Ok((ui(&field)?, focus, Kind::Text { field }))
}

/// The profiles a tab can open, one to choose as the default. The buttons are made when the row is synced, because the
/// shells are found after the window is up; Up and Down walk the list and pick as they go.
fn profiles(
    drawer: &Rc<Drawer>,
    descriptor: &'static Descriptor,
    revokers: &mut Vec<EventRevoker>,
) -> Result<(UIElement, UIElement, Kind)> {
    let list = Rc::new(ProfileList {
        list: stack(false, 4.0)?,
        buttons: RefCell::new(Vec::new()),
        ids: RefCell::new(Vec::new()),
        focused: Cell::new(0),
        revokers: RefCell::new(Vec::new()),
    });
    let (this, state) = (drawer.clone(), list.clone());
    revokers.push(ui(&list.list)?.PreviewKeyDown(cb!("drawer profiles key", |_, args| {
        let Some(args) = args.as_ref() else { return };
        let Some(arrow) = arrow_of(args.Key().map_or(0, |key| key.0)) else { return };
        let held = keyboard::held_modifiers();
        if held.ctrl || held.alt {
            return;
        }
        let count = state.buttons.borrow().len();
        if let Some(target) = logic::roving_target(state.focused.get(), count, 1, arrow) {
            let _ = args.SetHandled(true);
            if let Some(id) = state.ids.borrow().get(target).cloned() {
                this.commit(descriptor, Value::Text(id));
            }
            focus_item(&state.buttons.borrow(), target);
        }
    }))?);
    let content = ui(&list.list)?;
    Ok((content.clone(), content, Kind::Profiles(list)))
}

/// The colour schemes as tiles in five columns. Pointing at one (or moving the focus to it) previews the scheme; a
/// click, Enter or Space keeps it. Leaving the grid or Esc puts the kept one back.
fn tiles(
    drawer: &Rc<Drawer>,
    descriptor: &'static Descriptor,
    revokers: &mut Vec<EventRevoker>,
) -> Result<(UIElement, UIElement, Kind)> {
    const COLUMNS: usize = 5;
    let grid = stack(false, 6.0)?;
    grid.cast::<Panel>()?.SetBackground(&solid_brush(nekominal_model::themes::Color { a: 0, r: 0, g: 0, b: 0 })?)?;
    let names: Vec<&'static str> = themes::theme_names().collect();
    let mut buttons: Vec<ToggleButton> = Vec::new();
    let mut line: Option<StackPanel> = None;
    for (index, name) in names.iter().enumerate() {
        if index % COLUMNS == 0 {
            let row = stack(true, 8.0)?;
            grid.cast::<Panel>()?.Children()?.Append(&row)?;
            line = Some(row);
        }
        let scheme = themes::scheme(name);
        let cell = stack(false, 3.0)?;
        let tile = tile_button(scheme)?;
        AutomationProperties::SetName(&tile, name)?;
        ToolTipService::SetToolTip(&tile, &IReference::from(*name))?;
        let caption: TextBlock = load_xaml(
            r#"<TextBlock xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Width="80" FontSize="11"
                          TextTrimming="CharacterEllipsis" Foreground="{StaticResource NkPanelTextSecondary}"/>"#,
        )?
        .cast()?;
        caption.SetText(name)?;
        cell.cast::<Panel>()?.Children()?.Append(&tile)?;
        cell.cast::<Panel>()?.Children()?.Append(&caption)?;
        if let Some(line) = &line {
            line.cast::<Panel>()?.Children()?.Append(&cell)?;
        }
        buttons.push(tile);
    }

    // The arrow keys only move the focus (which previews); keeping the scheme is a click, Enter or Space.
    revokers.extend(drawer.roving(&ui(&grid)?, &buttons, COLUMNS, |_| {})?);
    for (index, tile) in buttons.iter().enumerate() {
        let (this, name) = (drawer.clone(), names[index]);
        revokers.push(tile.on_click(cb!("drawer tile", |_, _| {
            this.peek.borrow_mut().clear();
            this.commit(descriptor, Value::Text(name.to_owned()));
            this.sync();
        }))?);
        let this = drawer.clone();
        revokers
            .push(ui(tile)?.GotFocus(cb!("drawer tile focus", |_, _| this.peek.borrow_mut().focused(Some(index))))?);
        let this = drawer.clone();
        revokers.push(ui(tile)?.LostFocus(cb!("drawer tile blur", |_, _| this.peek.borrow_mut().focused(None)))?);
        let this = drawer.clone();
        revokers.push(
            ui(tile)?.PointerEntered(cb!("drawer tile hover", |_, _| this.peek.borrow_mut().hovered(Some(index))))?,
        );
    }
    // Leaving the grid (the gaps between tiles belong to it) ends the preview.
    let this = drawer.clone();
    revokers.push(ui(&grid)?.PointerExited(cb!("drawer tiles leave", |_, _| this.peek.borrow_mut().hovered(None)))?);

    let first = ui(&buttons[0])?;
    Ok((ui(&grid)?, first, Kind::Tiles(buttons)))
}

/// A tile in the colours of a scheme: its background, two lines of text, six of the colours of the terminal.
fn tile_button(scheme: &themes::Scheme) -> Result<ToggleButton> {
    let text = hex_color(scheme.foreground);
    let faint = hex_color(themes::Color { a: 150, ..scheme.foreground });
    let dots: String = (1..=6)
        .map(|slot| format!(r#"<Rectangle Width="8" Height="8" Fill="{}"/>"#, hex_color(scheme.ansi[slot])))
        .collect();
    let markup = format!(
        r#"<ToggleButton xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Style="{{StaticResource NkTile}}"
                         Background="{}">
             <Grid Margin="9,9,9,8">
               <Rectangle Width="38" Height="4" Fill="{text}" HorizontalAlignment="Left" VerticalAlignment="Top"/>
               <Rectangle Width="24" Height="4" Fill="{faint}" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="0,9,0,0"/>
               <StackPanel Orientation="Horizontal" Spacing="2" HorizontalAlignment="Left" VerticalAlignment="Bottom">{dots}</StackPanel>
             </Grid>
           </ToggleButton>"#,
        hex_color(themes::Color { a: 255, ..scheme.background })
    );
    load_xaml(&markup)?.cast()
}

/// Brings the controls of every row in line with the settings. A row whose state did not change is left alone.
pub fn sync(drawer: &Drawer, shell: &Rc<Shell>) {
    let settings = shell.settings.borrow().clone();
    for row in drawer.rows.borrow().iter() {
        if let Err(e) = row.sync(&settings, shell) {
            log(&format!("drawer sync {}: {e}", row.descriptor.id));
        }
    }
    drawer.extras.borrow().sync(shell);
}

impl SettingRow {
    fn sync(&self, settings: &Settings, shell: &Rc<Shell>) -> Result<()> {
        let value = (self.descriptor.get)(settings);
        let mut shown = Shown {
            value,
            enabled: true,
            text: String::new(),
            items: Vec::new(),
            modified: self.descriptor.is_modified(settings),
            invalid: shell.problems.borrow().iter().any(|problem| problem.key == self.descriptor.id),
        };
        if self.descriptor.id == "appearance.opacity" {
            // The slider shows what is asked for, reads 100% and is off without a backdrop, and says so when the
            // opacity could not be put in effect.
            let (percent, enabled, text) = shell.opacity_display();
            shown.value = Value::Number(percent);
            shown.enabled = enabled;
            shown.text = text;
        }
        if matches!(self.kind, Kind::Profiles { .. }) {
            shown.items =
                shell.profiles.borrow().iter().map(|p| (p.id.clone(), format!("{}\n{}", p.name, p.program))).collect();
            let chosen =
                nekominal_model::profiles::resolve_default(&shell.profiles.borrow(), &settings.default_profile)
                    .map(|p| p.id.clone());
            shown.value = Value::Text(chosen.unwrap_or_default());
        }
        if self.last.borrow().as_ref() == Some(&shown) {
            return Ok(());
        }
        let focused_field =
            |field: &TextBox| ui(field).and_then(|f| f.FocusState()).is_ok_and(|state| state != FocusState::Unfocused);
        match (&self.kind, &shown.value) {
            (Kind::Choices(buttons), value) => {
                let on = option_of(value);
                for (index, button) in buttons.iter().enumerate() {
                    button.SetIsChecked(Some(Some(index) == on))?;
                }
                set_tab_stop(buttons, on.unwrap_or(0));
            }
            (Kind::Slider { slider, value: label }, Value::Number(percent)) => {
                slider.cast::<RangeBase>()?.SetValue(*percent)?;
                slider.cast::<Control>()?.SetIsEnabled(shown.enabled)?;
                label.SetText(&shown.text)?;
            }
            (Kind::Number { field }, Value::Number(n)) => {
                if !focused_field(field) {
                    field.SetText(&logic::number_text(*n))?;
                }
            }
            (Kind::Text { field }, Value::Text(text)) => {
                if !focused_field(field) {
                    field.SetText(text)?;
                }
            }
            (Kind::Tiles(tiles), Value::Text(name)) => {
                let on = themes::theme_names().position(|candidate| candidate == name);
                for (index, tile) in tiles.iter().enumerate() {
                    tile.SetIsChecked(Some(Some(index) == on))?;
                }
                set_tab_stop(tiles, on.unwrap_or(0));
            }
            (Kind::Profiles(profiles), Value::Text(chosen)) => {
                self.sync_profiles(profiles, &shown.items, chosen)?;
            }
            _ => {}
        }
        ui(&self.tag)?.SetVisibility(visibility(shown.invalid))?;
        // The reset button keeps its room while it is not needed (nothing moves); it is off, and so out of the tab order.
        ui(&self.bar)?.SetOpacity(if shown.modified { 1.0 } else { 0.0 })?;
        ui(&self.reset)?.SetOpacity(if shown.modified { 1.0 } else { 0.0 })?;
        self.reset.cast::<Control>()?.SetIsEnabled(shown.modified)?;
        *self.last.borrow_mut() = Some(shown);
        Ok(())
    }

    /// The list of profiles: one button each, made again when the profiles changed.
    fn sync_profiles(&self, profiles: &Rc<ProfileList>, items: &[(String, String)], chosen: &str) -> Result<()> {
        let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
        if *profiles.ids.borrow() != ids {
            let children = profiles.list.cast::<Panel>()?.Children()?;
            children.Clear()?;
            profiles.buttons.borrow_mut().clear();
            profiles.revokers.borrow_mut().clear();
            for (index, (id, text)) in items.iter().enumerate() {
                let (name, program) = text.split_once('\n').unwrap_or((text, ""));
                let button = toggle_button("NkSegment", name)?;
                fe(&button)?.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
                button.cast::<ContentControl>()?.SetContent(&profile_content(name, program)?)?;
                AutomationProperties::SetName(&button, name)?;
                let (owner, descriptor, id) = (self.owner.clone(), self.descriptor, id.clone());
                let click = button.on_click(cb!("drawer profile", |_, _| {
                    if let Some(drawer) = owner.upgrade() {
                        drawer.commit(descriptor, Value::Text(id.clone()));
                        drawer.sync();
                    }
                }))?;
                let state = profiles.clone();
                let focus = ui(&button)?.GotFocus(cb!("drawer profile focus", |_, _| {
                    state.focused.set(index);
                    set_tab_stop(&state.buttons.borrow(), index);
                }))?;
                profiles.revokers.borrow_mut().extend([click, focus]);
                children.Append(&button)?;
                profiles.buttons.borrow_mut().push(button);
            }
            *profiles.ids.borrow_mut() = ids;
        }
        let known = profiles.ids.borrow().iter().any(|id| id == chosen);
        for (index, (id, button)) in profiles.ids.borrow().iter().zip(profiles.buttons.borrow().iter()).enumerate() {
            button.SetIsChecked(Some(id == chosen))?;
            ui(button)?.SetIsTabStop(id == chosen || (index == 0 && !known))?;
        }
        Ok(())
    }
}

/// A profile in the list: its name, then the program it starts.
fn profile_content(name: &str, program: &str) -> Result<UIElement> {
    let panel: StackPanel = load_xaml(
        r#"<StackPanel xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Orientation="Horizontal" Spacing="12">
             <TextBlock x:Name="Name" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" FontSize="13"
                        Foreground="{StaticResource NkPanelText}" VerticalAlignment="Center"/>
             <TextBlock x:Name="Program" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" FontSize="12"
                        Foreground="{StaticResource NkPanelTextSecondary}" TextTrimming="CharacterEllipsis" MaxWidth="260"
                        VerticalAlignment="Center"/>
           </StackPanel>"#,
    )?
    .cast()?;
    let root = fe(&panel)?;
    find::<TextBlock>(&root, "Name")?.SetText(name)?;
    find::<TextBlock>(&root, "Program")?.SetText(program)?;
    ui(&panel)
}
