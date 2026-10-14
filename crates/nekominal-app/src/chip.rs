//! The tab chip: a rounded pill with a status dot, the title and a close glyph, in a horizontal strip.
//!
//! It is drawn and driven here (hover, selection, middle-click, drag to reorder) instead of using `TabView`,
//! which would bring Windows Terminal's look and its focus-stealing pointer handling. The shell owns the tabs;
//! a chip only reports what the user did through [`TabEvent`].

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use crate::glass::Glass;
use crate::logic::reorder_target;
use crate::util::{find, load_xaml, solid_brush, ui, xml_escape};

pub type TabId = u32;

/// Space between two chips (`StackPanel.Spacing` of the strip in `shell.xaml`).
pub const CHIP_GAP: f64 = 4.0;

/// Pointer travel before a press on a chip becomes a drag.
const DRAG_THRESHOLD: f64 = 5.0;

const CHIP_XAML: &str = r#"
<Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        x:Name="Chip" Height="32" MinWidth="76" MaxWidth="216" CornerRadius="{StaticResource NkCornerRadius}"
        BorderThickness="1" Background="Transparent" BorderBrush="Transparent" Padding="12,0,3,0">
  <Border.BackgroundTransition>
    <BrushTransition Duration="0:0:0.14"/>
  </Border.BackgroundTransition>
  <Grid ColumnSpacing="8">
    <Grid.ColumnDefinitions>
      <ColumnDefinition Width="Auto"/>
      <ColumnDefinition Width="*"/>
      <ColumnDefinition Width="Auto"/>
    </Grid.ColumnDefinitions>
    <Ellipse x:Name="Dot" Width="8" Height="8" VerticalAlignment="Center"/>
    <TextBlock x:Name="Title" Grid.Column="1" VerticalAlignment="Center" FontFamily="Segoe UI Variable Text"
               FontSize="12.5" TextTrimming="CharacterEllipsis" TextWrapping="NoWrap" Text="{title}"/>
    <Border x:Name="Close" Grid.Column="2" Width="24" Height="24" CornerRadius="{StaticResource NkCornerRadius}" Background="Transparent"
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
  </Grid>
</Border>"#;

/// What a tab reports. The dot takes the scheme colours: accent while selected, warning for attention and
/// danger for a program that ended with an error. States an agent reports (working, blocked, done) join once the
/// pane forwards those events.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabState {
    Idle,
    Attention,
    Error,
}

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
    title: TextBlock,
    close: Border,
    close_glyph: TextBlock,
    selected: Cell<bool>,
    hovered: Cell<bool>,
    close_hot: Cell<bool>,
    close_pressed: Cell<bool>,
    state: Cell<TabState>,
    glass: Cell<Glass>,
    drag: RefCell<Option<Drag>>,
    on_event: Rc<dyn Fn(TabEvent)>,
    revokers: RefCell<Vec<EventRevoker>>,
}

fn fade(color: nekominal_model::themes::Color) -> nekominal_model::themes::Color {
    nekominal_model::themes::Color { a: 0, ..color }
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
            title: find(&element, "Title")?,
            close: find(&element, "Close")?,
            close_glyph: find(&element, "CloseGlyph")?,
            selected: Cell::new(false),
            hovered: Cell::new(false),
            close_hot: Cell::new(false),
            close_pressed: Cell::new(false),
            state: Cell::new(TabState::Idle),
            glass: Cell::new(glass),
            drag: RefCell::new(None),
            on_event,
            revokers: RefCell::new(Vec::new()),
        });
        AutomationProperties::SetName(&chip.root, title)?;
        ToolTipService::SetToolTip(&chip.root, &IReference::from(title))?;
        chip.attach()?;
        chip.refresh();
        Ok(chip)
    }

    pub fn set_title(&self, title: &str) {
        let _ = self.title.SetText(title);
        let _ = AutomationProperties::SetName(&self.root, title);
        let _ = ToolTipService::SetToolTip(&self.root, &IReference::from(title));
    }

    pub fn set_selected(&self, selected: bool) {
        self.selected.set(selected);
        self.refresh();
    }

    pub fn set_state(&self, state: TabState) {
        self.state.set(state);
        self.refresh();
    }

    pub fn set_glass(&self, glass: Glass) {
        self.glass.set(glass);
        self.refresh();
    }

    fn dot_color(&self) -> nekominal_model::themes::Color {
        let glass = self.glass.get();
        match self.state.get() {
            TabState::Idle if self.selected.get() => glass.accent,
            TabState::Idle => glass.dot_idle(),
            TabState::Attention => glass.warning,
            TabState::Error => glass.danger,
        }
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
            self.dot.cast::<Shape>()?.SetFill(&solid_brush(self.dot_color())?)?;
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
            (self.on_event)(TabEvent::Close(self.id));
            let _ = args.SetHandled(true);
            return;
        }
        // The right button opens the context flyout, which XAML does by itself.
        if !properties.IsLeftButtonPressed().unwrap_or(false) {
            return;
        }
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

    fn chip_widths(&self) -> Vec<f64> {
        let Ok(children) = self.strip.Children() else { return Vec::new() };
        (0..children.Size().unwrap_or(0))
            .map(|i| {
                children
                    .GetAt(i)
                    .ok()
                    .and_then(|child| child.cast::<FrameworkElement>().ok())
                    .and_then(|fe| fe.ActualWidth().ok())
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
