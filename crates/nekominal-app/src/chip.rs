//! The tab chip: a rounded rectangle with a status mark, the title and a close glyph, in a horizontal strip.
//!
//! It is drawn and driven here (hover, selection, middle-click, drag to reorder) instead of using `TabView`,
//! which would bring Windows Terminal's look and its focus-stealing pointer handling. The shell owns the tabs;
//! a chip only reports what the user did through [`TabEvent`].

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use nekominal_model::strings::Text;

use crate::glass::Glass;
use crate::i18n::{lang, t, tf};
use crate::logic::{Attention, CHIP_MAX_WIDTH, TabState, TabStatus, reorder_target, shows_ordinal};
use crate::util::{fe, find, load_xaml, solid_brush, ui, visibility, xml_escape};

pub type TabId = u32;

/// Space between two chips (`StackPanel.Spacing` of the strip in `shell.xaml`).
pub const CHIP_GAP: f64 = 4.0;

/// Width of a chip around its title: the padding, the border, the status slot, the two column gaps and the close
/// slot (`CHIP_XAML`; a test keeps the two in step).
const CHROME_WIDTH: f64 = 12.0 + 3.0 + 2.0 + 12.0 + 8.0 + 8.0 + 24.0;

/// Pointer travel before a press on a chip becomes a drag.
const DRAG_THRESHOLD: f64 = 5.0;

const CHIP_XAML: &str = r#"
<Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        x:Name="Chip" Height="32" MinWidth="48" MaxWidth="216" CornerRadius="{StaticResource NkCornerRadius}"
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
</Border>"#;

/// What the user did with a chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabEvent {
    Select(TabId),
    Close(TabId),
    /// The chip was dragged to this position in the strip.
    Move {
        id: TabId,
        to: usize,
    },
}

struct Drag {
    start_x: f64,
    started: bool,
    index: usize,
    widths: Vec<f64>,
    /// Left edge of the dragged chip, measured from the left edge of the strip's first chip.
    origin: f64,
    target: usize,
}

pub struct Chip {
    pub id: TabId,
    pub root: Border,
    strip: Panel,
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
        TabState::Attention => Some("\u{EA8F}"),
    }
}

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

/// The word of a state, the string the interface already has for it.
pub fn status_text(status: &TabStatus) -> Option<&'static str> {
    Some(t(match status.state {
        TabState::Idle => return None,
        TabState::Working => Text::AgentWorking,
        TabState::Blocked => Text::AgentBlocked,
        TabState::Done => Text::AgentDone,
        TabState::Error => Text::AgentError,
        TabState::Attention if status.attention == Some(Attention::Notification) => Text::AgentNotification,
        TabState::Attention => Text::AgentBell,
    }))
}

impl Chip {
    pub fn new(
        id: TabId,
        title: &str,
        glass: Glass,
        strip: &StackPanel,
        on_event: Rc<dyn Fn(TabEvent)>,
    ) -> Result<Rc<Chip>> {
        let markup = CHIP_XAML.replace("{title}", &xml_escape(title));
        let element = load_xaml(&markup)?;
        let chip = Rc::new(Chip {
            id,
            root: element.cast()?,
            strip: strip.cast()?,
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

    /// Accessible name, status and tooltip: the title, then the state in words and what the agent said. The state is
    /// also words for assistive technology, so it never depends on colour.
    fn update_labels(&self) {
        let title = self.title_text.borrow();
        let status = self.status.borrow();
        let word = status_text(&status);
        let separator = if lang() == nekominal_model::strings::Lang::ZhTw { "，" } else { ", " };
        let name = word.map_or_else(|| title.clone(), |word| format!("{title}{separator}{word}"));
        let mut tip = title.clone();
        let panes = (self.panes.get() >= 2).then(|| tf(Text::TabPaneCount, &[&self.panes.get()]));
        let zoomed = self.zoomed.get().then(|| t(Text::TabZoomed).to_owned());
        let lines = word.map(str::to_owned).into_iter().chain(status.message.clone()).chain(panes).chain(zoomed);
        for line in lines {
            tip.push('\n');
            tip.push_str(&line);
        }
        let _ = AutomationProperties::SetName(&self.root, &name);
        let _ = AutomationProperties::SetItemStatus(&self.root, word.unwrap_or_default());
        let _ = ToolTipService::SetToolTip(&self.root, &IReference::from(tip.as_str()));
    }

    /// The width the chip wants for its title and chrome, at most `CHIP_MAX_WIDTH`.
    pub fn natural_width(&self) -> f64 {
        let unbounded = Size { width: f32::INFINITY, height: f32::INFINITY };
        let title = ui(&self.title).and_then(|title| title.Measure(unbounded).and_then(|()| title.DesiredSize()));
        let mark = if self.panes.get() >= 2 {
            ui(&self.pane_mark).and_then(|mark| mark.Measure(unbounded).and_then(|()| mark.DesiredSize())).ok()
        } else {
            None
        };
        let chrome = CHROME_WIDTH + mark.map_or(0.0, |size| f64::from(size.width));
        (chrome + title.map_or(0.0, |size| f64::from(size.width))).min(CHIP_MAX_WIDTH)
    }

    /// Gives the chip its width in the strip; a chip too narrow for a title shows its number instead.
    pub fn set_width(&self, width: f64) {
        let _ = fe(&self.root).and_then(|root| root.SetWidth(width));
        let narrow = shows_ordinal(width);
        if narrow != self.narrow.replace(narrow) {
            let (padding, spacing) = if narrow {
                (Thickness { left: 6.0, top: 0.0, right: 4.0, bottom: 0.0 }, 4.0)
            } else {
                (Thickness { left: 12.0, top: 0.0, right: 3.0, bottom: 0.0 }, 8.0)
            };
            let _ = self.root.SetPadding(padding);
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

    pub fn state(&self) -> TabState {
        self.status.borrow().state
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
        let border = if selected { glass.chip_selected_border() } else { fade(glass.chip_selected_border()) };
        let paint = || -> Result<()> {
            self.root.SetBackground(&solid_brush(fill)?)?;
            self.root.SetBorderBrush(&solid_brush(border)?)?;
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
            let weak: Weak<Chip> = weak;
            cb!("chip capture lost", |_, _| {
                if let Some(chip) = weak.upgrade() {
                    chip.end_drag();
                }
            })
        })?);
        *self.revokers.borrow_mut() = revokers;
        Ok(())
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
        *self.drag.borrow_mut() =
            Some(Drag { start_x: f64::from(in_strip.x), started: false, index, widths, origin, target: index });
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
        let delta = f64::from(in_strip.x) - drag.start_x;
        if !drag.started {
            if delta.abs() < DRAG_THRESHOLD {
                return;
            }
            drag.started = true;
            let _ = Canvas::SetZIndex(&self.root, 10);
        }
        let width = drag.widths[drag.index];
        let total: f64 = drag.widths.iter().sum::<f64>() + CHIP_GAP * drag.widths.len().saturating_sub(1) as f64;
        let delta = delta.clamp(-drag.origin, (total - width - drag.origin).max(-drag.origin));
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

    #[test]
    fn the_chrome_width_adds_up_the_numbers_of_the_markup() {
        for needle in [
            "BorderThickness=\"1\"",
            "Padding=\"12,0,3,0\"",
            "ColumnSpacing=\"8\"",
            "<Grid x:Name=\"Slot\" Width=\"12\"",
            "Width=\"24\" Height=\"24\"",
        ] {
            assert!(CHIP_XAML.contains(needle), "the markup no longer has {needle}");
        }
        // padding 12 + 3, border 2, status slot 12, two gaps of 8, close slot 24
        assert_eq!(CHROME_WIDTH, 69.0);
    }
}
