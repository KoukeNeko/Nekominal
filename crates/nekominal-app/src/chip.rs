//! The tab chip: a rounded rectangle with a status mark, the title and a close glyph, in a horizontal strip.
//!
//! It is drawn and driven here (hover, selection, middle-click, drag to reorder) instead of using `TabView`,
//! which would bring Windows Terminal's look and its focus-stealing pointer handling. The shell owns the tabs;
//! a chip only reports what the user did through [`TabEvent`].

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Instant;

use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use nekominal_model::strings::Text;

use crate::glass::Glass;
use crate::i18n::{t, tf};
use crate::keyboard::{self, VK_END, VK_F2, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE};
use crate::logic::{self, Attention, TabState, TabStatus, reorder_target, shows_ordinal};
use crate::util::{fe, find, load_xaml, solid_brush, ui, visibility, xml_escape};

pub type TabId = u32;

/// Space between two chips (`StackPanel.Spacing` of the strip in `shell.xaml`).
pub const CHIP_GAP: f64 = 4.0;

/// Pointer travel before a press on a chip becomes a drag.
const DRAG_THRESHOLD: f64 = 5.0;

/// The activity id of the notifications a chip raises, which lets a screen reader tell them from other notifications.
const ANNOUNCEMENT_ID: &str = "nekominal.tab.needs-response";

/// The host of a chip is a control only so that it can take the keyboard focus (the arrow keys move it along the strip):
/// a pointer press does not focus it, the keyboard and a screen reader do. Every chip can be focused, and the strip is
/// one stop of the Tab key (`TabFocusNavigation="Once"` in `shell.xaml`).
const CHIP_XAML: &str = r#"
<UserControl xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
             xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
             x:Name="Host" IsTabStop="True" AllowFocusOnInteraction="False" UseSystemFocusVisuals="False" Padding="0">
<Border x:Name="Chip" Height="32" MinWidth="48" MaxWidth="216" CornerRadius="{StaticResource NkCornerRadius}"
        BorderThickness="1" Background="Transparent" BorderBrush="Transparent" Padding="12,0,3,0">
  <Border.BackgroundTransition>
    <BrushTransition Duration="0:0:0.14"/>
  </Border.BackgroundTransition>
  <Grid x:Name="Content" ColumnSpacing="8">
    <Grid.ColumnDefinitions>
      <ColumnDefinition Width="Auto"/>
      <ColumnDefinition Width="*"/>
      <ColumnDefinition Width="Auto"/>
    </Grid.ColumnDefinitions>
    <!-- The status mark: a dot at rest, a ring while something works, a glyph for the other states. -->
    <Grid x:Name="Slot" Width="12" Height="12" VerticalAlignment="Center">
      <Ellipse x:Name="Dot" Width="8" Height="8" HorizontalAlignment="Center" VerticalAlignment="Center"/>
      <TextBlock x:Name="Glyph" Visibility="Collapsed" FontFamily="Segoe Fluent Icons, Segoe MDL2 Assets" FontSize="12"
                 HorizontalAlignment="Center" VerticalAlignment="Center" IsHitTestVisible="False"/>
      <ProgressRing x:Name="Ring" Visibility="Collapsed" Width="12" Height="12" MinWidth="12" MinHeight="12"
                    IsActive="False" IsHitTestVisible="False"/>
    </Grid>
    <TextBlock x:Name="Title" Grid.Column="1" VerticalAlignment="Center" FontFamily="Segoe UI Variable Text"
               FontSize="12.5" TextTrimming="CharacterEllipsis" TextWrapping="NoWrap" Text="{title}"/>
    <TextBlock x:Name="Ordinal" Grid.Column="1" VerticalAlignment="Center" FontFamily="Segoe UI Variable Text"
               FontSize="12.5" Visibility="Collapsed" IsHitTestVisible="False"/>
    <StackPanel Grid.Column="2" Orientation="Horizontal" VerticalAlignment="Center">
    <!-- Only a tab with several panes has this: the split icon (filled while a pane is zoomed) and how many. -->
    <StackPanel x:Name="PaneMark" Orientation="Horizontal" Spacing="3" Margin="0,0,4,0" VerticalAlignment="Center"
                Visibility="Collapsed" IsHitTestVisible="False">
      <Viewbox Width="11" Height="11">
        <Canvas Width="16" Height="16">
          <Path x:Name="PaneIcon" StrokeThickness="1.6" StrokeLineJoin="Round"
                Data="M3,2.75 H13 A1.25,1.25 0 0 1 14.25,4 V12 A1.25,1.25 0 0 1 13,13.25 H3 A1.25,1.25 0 0 1 1.75,12 V4 A1.25,1.25 0 0 1 3,2.75 Z M8.5,2.75 V13.25"/>
        </Canvas>
      </Viewbox>
      <TextBlock x:Name="PaneCount" FontFamily="Segoe UI Variable Text" FontSize="11.5" VerticalAlignment="Center"/>
    </StackPanel>
    <Border x:Name="Close" Width="24" Height="24" CornerRadius="{StaticResource NkCornerRadius}" Background="Transparent"
            VerticalAlignment="Center" Opacity="0">
      <Border.OpacityTransition>
        <ScalarTransition Duration="0:0:0.14"/>
      </Border.OpacityTransition>
      <Border.BackgroundTransition>
        <BrushTransition Duration="0:0:0.1"/>
      </Border.BackgroundTransition>
      <TextBlock x:Name="CloseGlyph" Text="&#xE711;" FontFamily="Segoe Fluent Icons, Segoe MDL2 Assets" FontSize="9"
                 HorizontalAlignment="Center" VerticalAlignment="Center" IsHitTestVisible="False"/>
    </Border>
    </StackPanel>
  </Grid>
</Border>
</UserControl>"#;

/// What the user did with a chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabEvent {
    Select(TabId),
    Close(TabId),
    /// F2 on the chip, or a double click on it.
    Rename(TabId),
    /// The chip was dragged to this position in the strip.
    Move {
        id: TabId,
        to: usize,
    },
}

struct Drag {
    /// Where the pointer was on the strip (DIP from its first edge, scrolled content included) when it went down.
    start_x: f64,
    started: bool,
    index: usize,
    widths: Vec<f64>,
    /// Left edge of the dragged chip, measured from the left edge of the strip's first chip.
    origin: f64,
    target: usize,
    /// Where the pointer is in the viewer of the strip (DIP from its left edge), as the last pointer event said. It
    /// decides whether the strip scrolls under a pointer that is held at an edge and sends no events.
    viewport_x: f64,
    /// When the strip last had its turn to scroll for this drag.
    last_tick: Option<Instant>,
    /// Since when the pointer has been in a zone at an end of the strip, and whether the log says the strip scrolled.
    zone_since: Option<Instant>,
    scroll_logged: bool,
}

pub struct Chip {
    pub id: TabId,
    /// What the strip holds and what takes the focus; the chip is drawn in `border`.
    pub root: UserControl,
    border: Border,
    strip: Panel,
    /// The viewer that scrolls the strip.
    scroll: ScrollViewer,
    dot: Ellipse,
    glyph: TextBlock,
    ring: ProgressRing,
    title: TextBlock,
    /// The title as a program or the user set it; tooltip and accessible name add the status to it.
    title_text: RefCell<String>,
    ordinal: TextBlock,
    pane_mark: StackPanel,
    pane_icon: Shape,
    pane_count: TextBlock,
    content: Grid,
    close: Border,
    close_glyph: TextBlock,
    selected: Cell<bool>,
    /// The chip is narrower than a title needs and shows its number.
    narrow: Cell<bool>,
    hovered: Cell<bool>,
    close_hot: Cell<bool>,
    close_pressed: Cell<bool>,
    /// The middle button went down on the chip and has not come up.
    middle_down: Cell<bool>,
    /// How many panes the tab has and whether one of them is zoomed.
    panes: Cell<usize>,
    zoomed: Cell<bool>,
    status: RefCell<TabStatus>,
    /// The chip is in the visible part of the strip: only then does a working chip spin.
    in_view: Cell<bool>,
    glass: Cell<Glass>,
    /// The chip has the keyboard focus (never from a pointer press: the chip does not take it).
    focused: Cell<bool>,
    /// Where the tab works and with which profile, for the tooltip.
    cwd: RefCell<Option<String>>,
    profile: RefCell<String>,
    drag: RefCell<Option<Drag>>,
    on_event: Rc<dyn Fn(TabEvent)>,
    revokers: RefCell<Vec<EventRevoker>>,
}

fn fade(color: nekominal_model::themes::Color) -> nekominal_model::themes::Color {
    nekominal_model::themes::Color { a: 0, ..color }
}

/// The glyph (Segoe Fluent Icons) that tells a state apart without its colour. A quiet tab has a dot and a working
/// one a ring instead.
pub fn status_glyph(state: TabState) -> Option<&'static str> {
    match state {
        TabState::Idle | TabState::Working => None,
        TabState::Blocked => Some("\u{E897}"),
        TabState::Done => Some("\u{E73E}"),
        TabState::Error => Some("\u{E7BA}"),
        TabState::Attention => Some(ATTENTION_GLYPH),
    }
}

/// The glyph of a bell or a notification that rang out of sight.
pub const ATTENTION_GLYPH: &str = "\u{EA8F}";

/// The colour of a tab's status mark: the accent while the tab is selected and quiet or works, amber for what
/// waits for the user, green for done and red for an error, all kept legible against the scheme.
pub fn status_color(glass: Glass, state: TabState, selected: bool) -> nekominal_model::themes::Color {
    match state {
        TabState::Idle if selected => glass.accent,
        TabState::Idle => glass.dot_idle(),
        TabState::Working => glass.mark(glass.accent),
        TabState::Blocked | TabState::Attention => glass.mark(glass.warning),
        TabState::Done => glass.mark(glass.success),
        TabState::Error => glass.mark(glass.danger),
    }
}

/// The string the interface has for a state; none for a tab at rest. `attention` tells a notification from a bell.
pub fn status_label(state: TabState, attention: Option<Attention>) -> Option<Text> {
    Some(match state {
        TabState::Idle => return None,
        TabState::Working => Text::AgentWorking,
        TabState::Blocked => Text::AgentBlocked,
        TabState::Done => Text::AgentDone,
        TabState::Error => Text::AgentError,
        TabState::Attention if attention == Some(Attention::Notification) => Text::AgentNotification,
        TabState::Attention => Text::AgentBell,
    })
}

/// The word of a state, the string the interface already has for it.
pub fn status_text(status: &TabStatus) -> Option<&'static str> {
    status_label(status.state, status.attention).map(t)
}

impl Chip {
    pub fn new(
        id: TabId,
        title: &str,
        glass: Glass,
        strip: &StackPanel,
        scroll: &ScrollViewer,
        on_event: Rc<dyn Fn(TabEvent)>,
    ) -> Result<Rc<Chip>> {
        let markup = CHIP_XAML.replace("{title}", &xml_escape(title));
        let element = load_xaml(&markup)?;
        let chip = Rc::new(Chip {
            id,
            root: element.cast()?,
            border: find(&element, "Chip")?,
            strip: strip.cast()?,
            scroll: scroll.clone(),
            dot: find(&element, "Dot")?,
            glyph: find(&element, "Glyph")?,
            ring: find(&element, "Ring")?,
            title: find(&element, "Title")?,
            title_text: RefCell::new(title.to_owned()),
            ordinal: find(&element, "Ordinal")?,
            pane_mark: find(&element, "PaneMark")?,
            pane_icon: find(&element, "PaneIcon")?,
            pane_count: find(&element, "PaneCount")?,
            content: find(&element, "Content")?,
            close: find(&element, "Close")?,
            close_glyph: find(&element, "CloseGlyph")?,
            selected: Cell::new(false),
            narrow: Cell::new(false),
            hovered: Cell::new(false),
            close_hot: Cell::new(false),
            close_pressed: Cell::new(false),
            middle_down: Cell::new(false),
            panes: Cell::new(1),
            zoomed: Cell::new(false),
            status: RefCell::new(TabStatus::default()),
            in_view: Cell::new(true),
            glass: Cell::new(glass),
            focused: Cell::new(false),
            cwd: RefCell::new(None),
            profile: RefCell::new(String::new()),
            drag: RefCell::new(None),
            on_event,
            revokers: RefCell::new(Vec::new()),
        });
        chip.update_labels();
        chip.attach()?;
        chip.refresh();
        Ok(chip)
    }

    pub fn set_title(&self, title: &str) {
        let _ = self.title.SetText(title);
        *self.title_text.borrow_mut() = title.to_owned();
        self.update_labels();
    }

    /// Accessible name, status and tooltip. The name is the title, then the state in words: the state is also words for
    /// assistive technology, so it never depends on colour. The tooltip has four lines at most (`logic::tab_tooltip`).
    fn update_labels(&self) {
        let title = self.title_text.borrow();
        let status = self.status.borrow();
        let word = status_text(&status);
        let name = word.map_or_else(|| title.clone(), |word| tf(Text::TabNameStatus, &[&*title, &word]));
        // The state, when the tab is not at rest: its word, the progress it reports and what it said.
        let progress = status.progress.map(|progress| format!("{progress}%"));
        let message = status.message.as_deref().map(logic::clean_title);
        let state = word.and_then(|word| logic::tip_line(&[Some(word), progress.as_deref(), message.as_deref()]));
        let cwd = self.cwd.borrow().as_deref().map(|cwd| logic::clean_text(cwd, logic::MAX_PATH_CHARS));
        // The profile, how many panes there are when there are several, and that one is zoomed.
        let profile = self.profile.borrow();
        let panes = (self.panes.get() >= 2).then(|| tf(Text::TabPaneCount, &[&self.panes.get()]));
        let zoomed = self.zoomed.get().then(|| t(Text::TabZoomed));
        let place = logic::tip_line(&[(!profile.is_empty()).then_some(profile.as_str()), panes.as_deref(), zoomed]);
        let tip = logic::tab_tooltip(&title, state, cwd.as_deref(), place);
        let _ = AutomationProperties::SetName(&self.root, &name);
        let _ = AutomationProperties::SetItemStatus(&self.root, word.unwrap_or_default());
        let _ = ToolTipService::SetToolTip(&self.root, &IReference::from(tip.as_str()));
    }

    /// Gives the chip its width in the strip (`logic::tab_width`): the same for every chip, whatever its title says, whether
    /// it is selected and which marks it shows, all of which live inside it. A chip too narrow for a title shows its
    /// number instead.
    pub fn set_width(&self, width: f64) {
        let _ = fe(&self.root).and_then(|root| root.SetWidth(width));
        let narrow = shows_ordinal(width);
        if narrow != self.narrow.replace(narrow) {
            let (padding, spacing) = if narrow {
                (Thickness { left: 6.0, top: 0.0, right: 4.0, bottom: 0.0 }, 4.0)
            } else {
                (Thickness { left: 12.0, top: 0.0, right: 3.0, bottom: 0.0 }, 8.0)
            };
            let _ = self.border.SetPadding(padding);
            let _ = self.content.SetColumnSpacing(spacing);
            let _ = ui(&self.title).and_then(|title| title.SetOpacity(if narrow { 0.0 } else { 1.0 }));
            let _ = ui(&self.ordinal).and_then(|ordinal| ordinal.SetVisibility(visibility(narrow)));
            let _ = ui(&self.close).and_then(|close| close.SetVisibility(visibility(!narrow)));
            self.show_pane_mark();
            self.refresh();
        }
    }

    /// How many panes the tab has and whether one is zoomed. A single pane shows nothing; with several the chip
    /// shows the split icon (filled while a pane is zoomed) and the count, and says so in its tooltip.
    pub fn set_panes(&self, count: usize, zoomed: bool) {
        if self.panes.replace(count) == count && self.zoomed.replace(zoomed) == zoomed {
            return;
        }
        self.zoomed.set(zoomed);
        let _ = self.pane_count.SetText(&count.to_string());
        self.show_pane_mark();
        self.update_labels();
        self.refresh();
    }

    /// The mark is there for a tab with several panes, when the chip is wide enough for a title.
    fn show_pane_mark(&self) {
        let shown = self.panes.get() >= 2 && !self.narrow.get();
        let _ = ui(&self.pane_mark).and_then(|mark| mark.SetVisibility(visibility(shown)));
    }

    /// Tells assistive technology and the chip itself where it is in the strip (zero-based `index` of `count`).
    pub fn set_position(&self, index: usize, count: usize) {
        let _ = AutomationProperties::SetPositionInSet(&self.root, i32::try_from(index + 1).unwrap_or(i32::MAX));
        let _ = AutomationProperties::SetSizeOfSet(&self.root, i32::try_from(count).unwrap_or(i32::MAX));
        let _ = self.ordinal.SetText(&(index + 1).to_string());
    }

    pub fn set_selected(&self, selected: bool) {
        self.selected.set(selected);
        self.refresh();
    }

    /// Where the tab works and with which profile, as the tooltip tells them.
    pub fn set_context(&self, cwd: Option<String>, profile: &str) {
        if *self.cwd.borrow() == cwd && *self.profile.borrow() == profile {
            return;
        }
        *self.cwd.borrow_mut() = cwd;
        *self.profile.borrow_mut() = profile.to_owned();
        self.update_labels();
    }

    pub fn state(&self) -> TabState {
        self.status.borrow().state
    }

    pub fn status(&self) -> TabStatus {
        self.status.borrow().clone()
    }

    /// Says `text` to assistive technology as a notification of this chip (UI Automation `RaiseNotificationEvent`),
    /// once: the screen reader speaks it without the user having moved to the chip. Returns whether it was raised.
    pub fn announce(&self, text: &str) -> bool {
        let raised = FrameworkElementAutomationPeer::CreatePeerForElement(&self.root).and_then(|peer| {
            peer.RaiseNotificationEvent(
                AutomationNotificationKind::Other,
                AutomationNotificationProcessing::ImportantMostRecent,
                text,
                ANNOUNCEMENT_ID,
            )
        });
        match raised {
            Ok(()) => true,
            Err(e) => {
                crate::util::log(&format!("chip announce: {e}"));
                false
            }
        }
    }

    /// What the tab shows about its panes; the shell derives it, the chip only draws it.
    pub fn set_status(&self, status: TabStatus) {
        if *self.status.borrow() == status {
            return;
        }
        *self.status.borrow_mut() = status;
        self.update_labels();
        self.refresh();
    }

    /// Whether the chip is in the visible part of the strip; a working ring spins only then.
    pub fn set_in_view(&self, in_view: bool) {
        if self.in_view.replace(in_view) != in_view {
            self.refresh();
        }
    }

    pub fn set_glass(&self, glass: Glass) {
        self.glass.set(glass);
        self.refresh();
    }

    /// Paints the chip for its current selection, hover and state.
    fn refresh(&self) {
        let glass = self.glass.get();
        let (selected, hovered) = (self.selected.get(), self.hovered.get());
        let fill = if selected {
            glass.chip_selected()
        } else if hovered {
            glass.hover()
        } else {
            fade(glass.hover())
        };
        let border = if self.focused.get() {
            // The keyboard focus: the accent, as legible as any mark of the chip.
            glass.mark(glass.accent)
        } else if selected {
            glass.chip_selected_border()
        } else {
            fade(glass.chip_selected_border())
        };
        let paint = || -> Result<()> {
            self.border.SetBackground(&solid_brush(fill)?)?;
            self.border.SetBorderBrush(&solid_brush(border)?)?;
            self.title.cast::<TextBlock>()?.SetForeground(&solid_brush(if selected {
                glass.text()
            } else {
                glass.text_secondary()
            })?)?;
            self.title.SetFontWeight(FontWeight { weight: if selected { 600 } else { 400 } })?;
            self.ordinal.SetFontWeight(FontWeight { weight: if selected { 600 } else { 400 } })?;
            self.ordinal.SetForeground(&solid_brush(if selected { glass.text() } else { glass.text_secondary() })?)?;
            self.paint_status(glass, selected)?;
            let ink = if selected { glass.text() } else { glass.text_secondary() };
            self.pane_icon.SetStroke(&solid_brush(ink)?)?;
            // Filled while a pane is zoomed.
            self.pane_icon.SetFill(&solid_brush(if self.zoomed.get() { ink } else { fade(ink) })?)?;
            self.pane_count.SetForeground(&solid_brush(ink)?)?;
            ui(&self.close)?.SetOpacity(if selected || hovered { 1.0 } else { 0.0 })?;
            let close_fill = if self.close_hot.get() { glass.pressed() } else { fade(glass.pressed()) };
            self.close.SetBackground(&solid_brush(close_fill)?)?;
            self.close_glyph.SetForeground(&solid_brush(glass.text())?)?;
            Ok(())
        };
        if let Err(e) = paint() {
            crate::util::log(&format!("chip refresh: {e}"));
        }
    }

    /// The status mark: a dot, a ring or a glyph, and a semibold title for what waits for the user.
    fn paint_status(&self, glass: Glass, selected: bool) -> Result<()> {
        let status = self.status.borrow();
        let color = solid_brush(status_color(glass, status.state, selected))?;
        let glyph = status_glyph(status.state);
        let working = status.state == TabState::Working;
        ui(&self.dot)?.SetVisibility(visibility(!working && glyph.is_none()))?;
        self.dot.cast::<Shape>()?.SetFill(&color)?;
        ui(&self.glyph)?.SetVisibility(visibility(glyph.is_some()))?;
        if let Some(glyph) = glyph {
            self.glyph.SetText(glyph)?;
            self.glyph.SetForeground(&color)?;
        }
        ui(&self.ring)?.SetVisibility(visibility(working))?;
        let ring: IProgressRing = self.ring.cast()?;
        ring.SetIsActive(working && self.in_view.get())?;
        // A ring without a progress spins; with one, or when the system turns animations off, it is a static arc.
        let spins = status.progress.is_none() && crate::window::animations_enabled();
        ring.SetIsIndeterminate(spins)?;
        ring.SetValue(f64::from(status.progress.unwrap_or(25)))?;
        self.ring.cast::<Control>()?.SetForeground(&color)?;
        if status.state == TabState::Blocked {
            self.title.SetFontWeight(FontWeight { weight: 600 })?;
        }
        Ok(())
    }

    fn attach(self: &Rc<Self>) -> Result<()> {
        let root = ui(&self.root)?;
        let weak = Rc::downgrade(self);
        let mut revokers = Vec::new();

        revokers.push(root.PointerEntered({
            let weak = weak.clone();
            cb!("chip enter", |_, _| {
                if let Some(chip) = weak.upgrade() {
                    chip.hovered.set(true);
                    chip.refresh();
                }
            })
        })?);
        revokers.push(root.PointerExited({
            let weak = weak.clone();
            cb!("chip exit", |_, _| {
                if let Some(chip) = weak.upgrade() {
                    chip.hovered.set(false);
                    chip.close_hot.set(false);
                    chip.refresh();
                }
            })
        })?);
        revokers.push(root.PointerPressed({
            let weak = weak.clone();
            cb!("chip pressed", |_, args| {
                let (Some(chip), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                chip.on_pressed(args);
            })
        })?);
        revokers.push(root.PointerMoved({
            let weak = weak.clone();
            cb!("chip moved", |_, args| {
                let (Some(chip), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                chip.on_moved(args);
            })
        })?);
        revokers.push(root.PointerReleased({
            let weak = weak.clone();
            cb!("chip released", |_, args| {
                let (Some(chip), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                chip.on_released(args);
            })
        })?);
        revokers.push(root.PointerCaptureLost({
            let weak = weak.clone();
            cb!("chip capture lost", |_, _| {
                if let Some(chip) = weak.upgrade() {
                    chip.end_drag();
                }
            })
        })?);
        // A double click renames, as F2 does on a chip that has the focus.
        revokers.push(root.DoubleTapped({
            let weak = weak.clone();
            cb!("chip double tapped", |_, _| {
                if let Some(chip) = weak.upgrade() {
                    (chip.on_event)(TabEvent::Rename(chip.id));
                }
            })
        })?);
        revokers.push(root.KeyDown({
            let weak = weak.clone();
            cb!("chip key", |_, args| {
                let (Some(chip), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                chip.on_key(args);
            })
        })?);
        revokers.push(root.GotFocus({
            let weak = weak.clone();
            cb!("chip focus", |_, _| {
                if let Some(chip) = weak.upgrade() {
                    chip.focused.set(true);
                    chip.refresh();
                }
            })
        })?);
        revokers.push(root.LostFocus({
            let weak: Weak<Chip> = weak;
            cb!("chip blur", |_, _| {
                if let Some(chip) = weak.upgrade() {
                    chip.focused.set(false);
                    chip.refresh();
                }
            })
        })?);
        *self.revokers.borrow_mut() = revokers;
        Ok(())
    }

    /// The place of this chip in the strip and how many chips it has.
    fn strip_position(&self) -> Option<(usize, usize)> {
        let children = self.strip.Children().ok()?;
        let mut at = 0u32;
        let found = ui(&self.root).and_then(|root| children.IndexOf(&root, &mut at)).ok()?;
        found.then(|| (at as usize, children.Size().unwrap_or(0) as usize))
    }

    /// Gives the keyboard focus to the chip at `index` of the strip.
    fn focus_chip(&self, index: usize) {
        let chip = self
            .strip
            .Children()
            .and_then(|children| children.GetAt(index as u32))
            .and_then(|child| child.cast::<UIElement>());
        if let Ok(chip) = chip {
            let _ = chip.Focus(FocusState::Keyboard);
        }
    }

    /// The keys of a chip that has the focus: Left and Right move the focus along the strip (Home and End to its ends),
    /// Space and Enter select the chip and F2 renames it. Ctrl and Alt chords are the commands of the window.
    fn on_key(&self, args: &KeyRoutedEventArgs) {
        let held = keyboard::held_modifiers();
        if held.ctrl || held.alt {
            return;
        }
        let Some((at, count)) = self.strip_position() else { return };
        let target = match args.Key().map_or(0, |key| key.0) {
            VK_LEFT => at.saturating_sub(1),
            VK_RIGHT => (at + 1).min(count.saturating_sub(1)),
            VK_HOME => 0,
            VK_END => count.saturating_sub(1),
            VK_SPACE | VK_RETURN => {
                let _ = args.SetHandled(true);
                (self.on_event)(TabEvent::Select(self.id));
                return;
            }
            VK_F2 => {
                let _ = args.SetHandled(true);
                (self.on_event)(TabEvent::Rename(self.id));
                return;
            }
            _ => return,
        };
        let _ = args.SetHandled(true);
        self.focus_chip(target);
    }

    /// Whether `x` (DIP, from the left edge of the chip) is over the close glyph.
    fn over_close(&self, x: f64) -> bool {
        let Ok(fe) = self.close.cast::<FrameworkElement>() else { return false };
        let Ok(origin) = ui(&self.close)
            .and_then(|c| c.TransformToVisual(&self.root))
            .and_then(|t| t.TransformPoint(Point { x: 0.0, y: 0.0 }))
        else {
            return false;
        };
        let left = f64::from(origin.x);
        x >= left && x <= left + fe.ActualWidth().unwrap_or(0.0)
    }

    fn position_in(&self, args: &PointerRoutedEventArgs, relative_to: &impl Interface) -> Option<Point> {
        args.GetCurrentPoint(&relative_to.cast::<UIElement>().ok()?).ok()?.Position().ok()
    }

    fn on_pressed(&self, args: &PointerRoutedEventArgs) {
        let Some(properties) = args.GetCurrentPoint(&self.root).and_then(|point| point.Properties()).ok() else {
            return;
        };
        if properties.IsMiddleButtonPressed().unwrap_or(false) {
            // Closing waits for the button to come up over the chip, so a press that slides away cancels it.
            self.middle_down.set(true);
            if let Ok(pointer) = args.Pointer() {
                let _ = ui(&self.root).and_then(|root| root.CapturePointer(&pointer));
            }
            let _ = args.SetHandled(true);
            return;
        }
        // The right button opens the context flyout, which XAML does by itself.
        if !properties.IsLeftButtonPressed().unwrap_or(false) {
            return;
        }
        // Handled: XAML would otherwise move the keyboard focus off the pane that the selection just gave it to.
        let _ = args.SetHandled(true);
        let Some(local) = self.position_in(args, &self.root) else { return };
        if let Ok(pointer) = args.Pointer() {
            let _ = ui(&self.root).and_then(|root| root.CapturePointer(&pointer));
        }
        if self.over_close(f64::from(local.x)) {
            self.close_pressed.set(true);
            return;
        }
        (self.on_event)(TabEvent::Select(self.id));
        let Some(in_strip) = self.position_in(args, &self.strip) else { return };
        let mut index = 0u32;
        let found = self
            .strip
            .Children()
            .and_then(|children| ui(&self.root).and_then(|root| children.IndexOf(&root, &mut index)))
            .unwrap_or(false);
        if !found {
            return;
        }
        let widths = self.chip_widths();
        let index = index as usize;
        let origin: f64 = widths[..index.min(widths.len())].iter().sum::<f64>() + CHIP_GAP * index as f64;
        let viewport_x = self.position_in(args, &self.scroll).map_or(0.0, |point| f64::from(point.x));
        *self.drag.borrow_mut() = Some(Drag {
            start_x: f64::from(in_strip.x),
            started: false,
            index,
            widths,
            origin,
            target: index,
            viewport_x,
            last_tick: None,
            zone_since: None,
            scroll_logged: false,
        });
    }

    /// The width every chip of the strip has been given. `Width`, not `ActualWidth`: the press that starts a drag also
    /// selects the chip, which re-sizes the strip, and `ActualWidth` still has the sizes of the layout before it.
    fn chip_widths(&self) -> Vec<f64> {
        let Ok(children) = self.strip.Children() else { return Vec::new() };
        (0..children.Size().unwrap_or(0))
            .map(|i| {
                children
                    .GetAt(i)
                    .ok()
                    .and_then(|child| child.cast::<FrameworkElement>().ok())
                    .and_then(|fe| fe.Width().ok().filter(|width| width.is_finite()).or_else(|| fe.ActualWidth().ok()))
                    .unwrap_or(0.0)
            })
            .collect()
    }

    fn on_moved(&self, args: &PointerRoutedEventArgs) {
        let Some(local) = self.position_in(args, &self.root) else { return };
        let hot = self.over_close(f64::from(local.x));
        if hot != self.close_hot.get() {
            self.close_hot.set(hot);
            self.refresh();
        }
        let Some(in_strip) = self.position_in(args, &self.strip) else { return };
        let mut drag = self.drag.borrow_mut();
        let Some(drag) = drag.as_mut() else { return };
        if let Some(point) = self.position_in(args, &self.scroll) {
            drag.viewport_x = f64::from(point.x);
            if logic::drag_scroll_speed(drag.viewport_x, self.viewport_width()) == 0.0 {
                drag.zone_since = None;
                drag.scroll_logged = false;
            } else {
                drag.zone_since.get_or_insert_with(Instant::now);
            }
        }
        if !drag.started {
            if (f64::from(in_strip.x) - drag.start_x).abs() < DRAG_THRESHOLD {
                return;
            }
            drag.started = true;
            drag.last_tick = Some(Instant::now());
            let _ = Canvas::SetZIndex(&self.root, 10);
        }
        self.place_drag(drag, f64::from(in_strip.x));
    }

    /// Puts the dragged chip where the pointer is, `pointer_x` DIP along the strip, and every chip it passes one place
    /// aside; the place it would be dropped in is the target.
    fn place_drag(&self, drag: &mut Drag, pointer_x: f64) {
        let width = drag.widths[drag.index];
        let total: f64 = drag.widths.iter().sum::<f64>() + CHIP_GAP * drag.widths.len().saturating_sub(1) as f64;
        let delta = (pointer_x - drag.start_x).clamp(-drag.origin, (total - width - drag.origin).max(-drag.origin));
        drag.target = reorder_target(&drag.widths, CHIP_GAP, drag.index, drag.origin + width / 2.0 + delta);
        let shift = width + CHIP_GAP;
        let Ok(children) = self.strip.Children() else { return };
        for (j, _) in drag.widths.iter().enumerate() {
            let x = if j == drag.index {
                delta
            } else if drag.index < drag.target && j > drag.index && j <= drag.target {
                -shift
            } else if drag.target < drag.index && j >= drag.target && j < drag.index {
                shift
            } else {
                0.0
            };
            if let Ok(child) = children.GetAt(j as u32) {
                let _ = child.SetTranslation(Vector3 { x: x as f32, y: 0.0, z: 0.0 });
            }
        }
    }

    /// Every frame: a dragged chip whose pointer rests in the zone at an end of the strip scrolls the strip, though the
    /// pointer sends no events, and the chip stays under it as the strip moves (`logic::drag_scroll_speed`).
    pub fn tick_drag(&self) {
        let mut guard = self.drag.borrow_mut();
        let Some(drag) = guard.as_mut().filter(|drag| drag.started) else { return };
        let now = Instant::now();
        let seconds = drag.last_tick.replace(now).map_or(0.0, |last| now.duration_since(last).as_secs_f64()).min(0.05);
        let speed = logic::drag_scroll_speed(drag.viewport_x, self.viewport_width());
        if speed == 0.0 {
            return;
        }
        let offset = self.scroll.HorizontalOffset().unwrap_or(0.0);
        let target = (offset + speed * seconds).clamp(0.0, self.scroll.ScrollableWidth().unwrap_or(0.0));
        if (target - offset).abs() < 0.01 {
            return;
        }
        let _ = self.scroll.ChangeViewWithOptionalAnimation(Some(target), None, None, true);
        if let Some(since) = drag.zone_since.filter(|_| !drag.scroll_logged) {
            drag.scroll_logged = true;
            crate::util::log(&format!(
                "drag: the strip started to scroll {} ms after the pointer entered the zone",
                since.elapsed().as_millis()
            ));
        }
        let pointer_x = drag.viewport_x + target;
        self.place_drag(drag, pointer_x);
    }

    /// The width of the part of the strip that shows, in DIP.
    fn viewport_width(&self) -> f64 {
        fe(&self.scroll).and_then(|scroll| scroll.ActualWidth()).unwrap_or(0.0)
    }

    /// Esc during a drag: the chips go back where they were and nothing moves. True when there was a drag to end.
    pub fn cancel_drag(&self) -> bool {
        let dragging = self.drag.borrow().as_ref().is_some_and(|drag| drag.started);
        if dragging {
            self.end_drag();
        }
        dragging
    }

    fn on_released(&self, args: &PointerRoutedEventArgs) {
        let local = self.position_in(args, &self.root);
        if self.middle_down.get() {
            let still_down = args
                .GetCurrentPoint(&self.root)
                .and_then(|point| point.Properties())
                .and_then(|properties| properties.IsMiddleButtonPressed())
                .unwrap_or(false);
            if !still_down {
                self.middle_down.set(false);
                if let Ok(pointer) = args.Pointer() {
                    let _ = ui(&self.root).and_then(|root| root.ReleasePointerCapture(&pointer));
                }
                let over_chip = local.is_some_and(|point| {
                    let (width, height) = fe(&self.root)
                        .map(|root| (root.ActualWidth().unwrap_or(0.0), root.ActualHeight().unwrap_or(0.0)))
                        .unwrap_or_default();
                    (0.0..=width).contains(&f64::from(point.x)) && (0.0..=height).contains(&f64::from(point.y))
                });
                if over_chip {
                    (self.on_event)(TabEvent::Close(self.id));
                }
            }
            return;
        }
        let on_close = self.close_pressed.replace(false) && local.is_some_and(|p| self.over_close(f64::from(p.x)));
        let moved = self.drag.borrow().as_ref().filter(|drag| drag.started).map(|drag| (drag.index, drag.target));
        self.end_drag();
        if let Ok(pointer) = args.Pointer() {
            let _ = ui(&self.root).and_then(|root| root.ReleasePointerCapture(&pointer));
        }
        if on_close {
            (self.on_event)(TabEvent::Close(self.id));
        } else if let Some((from, to)) = moved
            && from != to
        {
            (self.on_event)(TabEvent::Move { id: self.id, to });
        }
    }

    /// Puts every chip back where the layout says it is.
    fn end_drag(&self) {
        let was_dragging = self.drag.borrow_mut().take().is_some_and(|drag| drag.started);
        self.close_pressed.set(false);
        self.middle_down.set(false);
        if !was_dragging {
            return;
        }
        let _ = Canvas::SetZIndex(&self.root, 0);
        if let Ok(children) = self.strip.Children() {
            for i in 0..children.Size().unwrap_or(0) {
                if let Ok(child) = children.GetAt(i) {
                    let _ = child.SetTranslation(Vector3 { x: 0.0, y: 0.0, z: 0.0 });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value of `attribute` on the element of `CHIP_XAML` whose `x:Name` is `name`.
    fn attribute(name: &str, attribute: &str) -> String {
        let marker = format!("x:Name=\"{name}\"");
        let start = CHIP_XAML.find(&marker).unwrap_or_else(|| panic!("the markup has no {marker}"));
        let rest = &CHIP_XAML[start..];
        let element = &rest[..rest.find('>').expect("the element ends")];
        let key = format!("{attribute}=\"");
        let at = element.find(&key).unwrap_or_else(|| panic!("{name} has no {attribute}")) + key.len();
        element[at..].split('"').next().expect("the value ends").to_owned()
    }

    fn number(text: &str) -> f64 {
        text.parse().unwrap_or_else(|_| panic!("{text:?} is not a number"))
    }

    #[test]
    fn the_limits_of_the_markup_are_those_of_the_width_rule_and_nothing_inside_a_chip_asks_for_more() {
        use crate::logic::{CHIP_MAX_WIDTH, CHIP_MIN_WIDTH};
        // `logic::tab_width` gives every chip a width between the two; the markup must not narrow or widen it.
        assert_eq!(number(&attribute("Chip", "MinWidth")), CHIP_MIN_WIDTH);
        assert_eq!(number(&attribute("Chip", "MaxWidth")), CHIP_MAX_WIDTH);
        // The status slot and the close slot have a width of their own, and the close slot is hidden by its opacity
        // (never collapsed): hovering or selecting a chip changes how it looks and never how wide it is.
        assert_eq!(number(&attribute("Slot", "Width")), 12.0);
        assert_eq!(number(&attribute("Close", "Width")), 24.0);
        assert_eq!(number(&attribute("Close", "Opacity")), 0.0);
    }
}
