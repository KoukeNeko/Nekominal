//! One tab: its chip, the area its panes live in, and the pane tree.
//!
//! The shape of the split lives only in `nekominal_model::layout` (critique.md C12). Every layout pass derives
//! the position of each pane and divider from `layout_rects` / `splitters` and places them on a `Canvas`,
//! snapped to physical pixels; dragging a divider edits the tree and runs the pass again.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nekominal_model::layout::{Axis, Direction, Node, PaneId, Splitter, UNIT};
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};

use crate::chip::{Chip, TabId};
use crate::glass::Glass;
use crate::logic::{PxRect, divider_px, pane_px_rect, ratio_at, splitter_px_rect};
use crate::pane_view::PaneView;
use crate::util::{Ext as _, find, load_xaml, solid_brush, ui, visibility};

/// Width of the strip around a divider that grabs the pointer, in DIP.
const DIVIDER_HIT: f64 = 8.0;
/// Height of the accent line on the focused pane, in DIP.
const FOCUS_LINE: f64 = 2.0;
/// One step of the resize-pane command, as a fraction of the tab area.
pub const RESIZE_STEP: f64 = 0.05;

const DIVIDER_XAML: &str = r##"
<Grid xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
      xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Background="#01808080">
  <Border x:Name="Line" Background="Transparent"/>
</Grid>"##;

struct Divider {
    root: Grid,
    line: Border,
    /// Where the split sits in the tree; changes when the tree does.
    path: RefCell<Vec<u8>>,
    axis: Cell<Axis>,
    hot: Cell<bool>,
    dragging: Cell<bool>,
    glass: Cell<Glass>,
    revokers: RefCell<Vec<EventRevoker>>,
}

pub struct Tab {
    pub id: TabId,
    pub chip: Rc<Chip>,
    pub area: Canvas,
    pub layout: RefCell<Node>,
    pub panes: RefCell<Vec<Rc<PaneView>>>,
    pub focused: Cell<PaneId>,
    pub zoomed: Cell<Option<PaneId>>,
    pub title: RefCell<String>,
    /// The user named the tab: programs no longer change its title.
    pub renamed: Cell<bool>,
    visible: Cell<bool>,
    glass: Cell<Glass>,
    dividers: RefCell<Vec<Rc<Divider>>>,
    focus_line: Border,
    revokers: RefCell<Vec<EventRevoker>>,
}

fn rasterization_scale(element: &impl Interface) -> f64 {
    element.xaml_root().and_then(|root| root.RasterizationScale()).unwrap_or(1.0)
}

impl Tab {
    pub fn new(id: TabId, chip: Rc<Chip>, first: Rc<PaneView>, title: String, glass: Glass) -> Result<Rc<Tab>> {
        let area = Canvas::new()?;
        let fe: FrameworkElement = area.cast()?;
        fe.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
        fe.SetVerticalAlignment(VerticalAlignment::Stretch)?;
        let focus_line = Border::new()?;
        focus_line.cast::<FrameworkElement>()?.SetHeight(FOCUS_LINE)?;
        ui(&focus_line)?.SetIsHitTestVisible(false)?;
        ui(&focus_line)?.SetVisibility(Visibility::Collapsed)?;
        let first_id = first.id;
        let tab = Rc::new(Tab {
            id,
            chip,
            area,
            layout: RefCell::new(Node::Leaf(first_id)),
            panes: RefCell::new(vec![first.clone()]),
            focused: Cell::new(first_id),
            zoomed: Cell::new(None),
            title: RefCell::new(title),
            renamed: Cell::new(false),
            visible: Cell::new(false),
            glass: Cell::new(glass),
            dividers: RefCell::new(Vec::new()),
            focus_line,
            revokers: RefCell::new(Vec::new()),
        });
        let children = tab.area.cast::<Panel>()?.Children()?;
        children.Append(&first.host)?;
        children.Append(&tab.focus_line)?;
        let weak = Rc::downgrade(&tab);
        let size_changed = fe.SizeChanged(cb!("tab size", |_, _| {
            if let Some(tab) = weak.upgrade() {
                tab.relayout();
            }
        }))?;
        tab.revokers.borrow_mut().push(size_changed);
        Ok(tab)
    }

    pub fn pane(&self, id: PaneId) -> Option<Rc<PaneView>> {
        self.panes.borrow().iter().find(|pane| pane.id == id).cloned()
    }

    pub fn focused_pane(&self) -> Option<Rc<PaneView>> {
        self.pane(self.focused.get())
    }

    pub fn pane_count(&self) -> usize {
        self.panes.borrow().len()
    }

    /// Panes whose program is still running.
    pub fn running_sessions(&self) -> u32 {
        self.panes.borrow().iter().filter(|pane| pane.exited.get().is_none()).count() as u32
    }

    /// Whether this tab is the one on screen. A hidden tab gives its swap chains back.
    pub fn set_visible(&self, visible: bool) {
        self.visible.set(visible);
        let _ = ui(&self.area).and_then(|area| area.SetVisibility(visibility(visible)));
        if !visible {
            for pane in self.panes.borrow().iter() {
                pane.suspend();
            }
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible.get()
    }

    /// The panes that are drawn now: all of them, or only the zoomed one.
    pub fn drawn_panes(&self) -> Vec<Rc<PaneView>> {
        if !self.visible.get() {
            return Vec::new();
        }
        let zoomed = self.zoomed.get();
        self.panes.borrow().iter().filter(|pane| zoomed.is_none_or(|id| id == pane.id)).cloned().collect()
    }

    pub fn set_glass(self: &Rc<Self>, glass: Glass) {
        self.glass.set(glass);
        self.chip.set_glass(glass);
        self.relayout();
    }

    /// Dim every pane but the focused one while the tab has several.
    pub fn update_focus_look(&self) {
        let several = self.pane_count() > 1;
        for pane in self.panes.borrow().iter() {
            pane.set_dimmed(several && pane.id != self.focused.get());
        }
    }

    /// Adds `pane` next to `target` and gives it the focus.
    pub fn split(self: &Rc<Self>, target: PaneId, axis: Axis, pane: Rc<PaneView>) -> Result<()> {
        let id = pane.id;
        self.zoomed.set(None);
        {
            let mut layout = self.layout.borrow_mut();
            *layout = layout.split_pane(target, axis, id);
        }
        self.area.cast::<Panel>()?.Children()?.InsertAt(0, &pane.host)?;
        self.panes.borrow_mut().push(pane);
        self.focused.set(id);
        self.relayout();
        self.update_focus_look();
        Ok(())
    }

    /// Removes a pane and stops its session. Returns false when it was the last one.
    pub fn remove_pane(self: &Rc<Self>, id: PaneId) -> bool {
        let Some(pane) = self.pane(id) else { return true };
        let remaining = self.layout.borrow().remove_pane(id);
        let Some(remaining) = remaining else { return false };
        if let Ok(children) = self.area.cast::<Panel>().and_then(|panel| panel.Children()) {
            let mut index = 0u32;
            if let Ok(true) = children.IndexOf(&pane.host, &mut index) {
                let _ = children.RemoveAt(index);
            }
        }
        // The sibling that takes the freed space is the natural next focus; fall back to the first pane.
        let next = self
            .layout
            .borrow()
            .neighbor(id, Direction::Left)
            .or_else(|| self.layout.borrow().neighbor(id, Direction::Up))
            .or_else(|| remaining.pane_ids().first().copied());
        *self.layout.borrow_mut() = remaining;
        self.panes.borrow_mut().retain(|p| p.id != id);
        pane.shutdown();
        if self.zoomed.get() == Some(id) {
            self.zoomed.set(None);
        }
        if self.focused.get() == id
            && let Some(next) =
                next.filter(|next| self.pane(*next).is_some()).or_else(|| self.panes.borrow().first().map(|p| p.id))
        {
            self.focused.set(next);
        }
        self.relayout();
        self.update_focus_look();
        true
    }

    pub fn toggle_zoom(self: &Rc<Self>) {
        if self.zoomed.get().is_some() {
            self.zoomed.set(None);
        } else if self.pane_count() > 1 {
            self.zoomed.set(Some(self.focused.get()));
        }
        self.relayout();
    }

    /// Moves focus to the neighbouring pane in `direction`, if there is one.
    pub fn focus_direction(&self, direction: Direction) -> Option<PaneId> {
        if self.zoomed.get().is_some() {
            return None;
        }
        let target = self.layout.borrow().neighbor(self.focused.get(), direction)?;
        self.focused.set(target);
        self.update_focus_look();
        Some(target)
    }

    pub fn resize_focused(self: &Rc<Self>, direction: Direction) {
        let resized = self.layout.borrow().resize_pane(self.focused.get(), direction, RESIZE_STEP);
        *self.layout.borrow_mut() = resized;
        self.relayout();
    }

    /// The layout pass: positions of the panes, dividers and the focus line from the pane tree.
    pub fn relayout(self: &Rc<Self>) {
        let Ok(fe) = self.area.cast::<FrameworkElement>() else { return };
        let (width, height) = (fe.ActualWidth().unwrap_or(0.0), fe.ActualHeight().unwrap_or(0.0));
        if width < 1.0 || height < 1.0 {
            return;
        }
        let scale = rasterization_scale(&self.area);
        let area_px = ((width * scale).round() as i32, (height * scale).round() as i32);
        let divider = divider_px(scale);
        let layout = self.layout.borrow().clone();
        let zoomed = self.zoomed.get();
        let mut focused_rect = None;
        for (id, rect) in layout.layout_rects(UNIT) {
            let Some(pane) = self.pane(id) else { continue };
            let shown = zoomed.is_none_or(|z| z == id);
            let _ = ui(&pane.host).and_then(|host| host.SetVisibility(visibility(shown)));
            if !shown {
                pane.suspend();
                continue;
            }
            let px = pane_px_rect(if zoomed.is_some() { UNIT } else { rect }, area_px, divider);
            place(&pane.host, px, scale);
            if id == self.focused.get() {
                focused_rect = Some(px);
            }
        }
        let splitters = if zoomed.is_some() { Vec::new() } else { layout.splitters(UNIT) };
        self.sync_dividers(&splitters, area_px, divider, scale);
        self.place_focus_line(focused_rect, scale);
    }

    fn place_focus_line(&self, rect: Option<PxRect>, scale: f64) {
        let show = rect.is_some() && self.pane_count() > 1 && self.zoomed.get().is_none();
        let _ = ui(&self.focus_line).and_then(|line| line.SetVisibility(visibility(show)));
        let Some(px) = rect.filter(|_| show) else { return };
        if let Ok(brush) = solid_brush(self.glass.get().accent) {
            let _ = self.focus_line.SetBackground(&brush);
        }
        let _ = Canvas::SetLeft(&self.focus_line, f64::from(px.x) / scale);
        let _ = Canvas::SetTop(&self.focus_line, f64::from(px.y) / scale);
        let _ = self.focus_line.cast::<FrameworkElement>().and_then(|line| line.SetWidth(f64::from(px.w) / scale));
        // Above the panes and the dividers.
        let _ = Canvas::SetZIndex(&self.focus_line, 20);
    }

    fn sync_dividers(self: &Rc<Self>, splitters: &[Splitter], area_px: (i32, i32), divider: i32, scale: f64) {
        let Ok(children) = self.area.cast::<Panel>().and_then(|panel| panel.Children()) else { return };
        let mut dividers = self.dividers.borrow_mut();
        while dividers.len() > splitters.len() {
            if let Some(extra) = dividers.pop() {
                let mut index = 0u32;
                if let Ok(true) = children.IndexOf(&extra.root, &mut index) {
                    let _ = children.RemoveAt(index);
                }
            }
        }
        while dividers.len() < splitters.len() {
            match Divider::new(self) {
                Ok(new) => {
                    let _ = children.Append(&new.root);
                    dividers.push(new);
                }
                Err(e) => {
                    crate::util::log(&format!("divider: {e}"));
                    return;
                }
            }
        }
        let glass = self.glass.get();
        for (divider_view, splitter) in dividers.iter().zip(splitters) {
            *divider_view.path.borrow_mut() = splitter.path.clone();
            divider_view.axis.set(splitter.axis);
            let gap = splitter_px_rect(splitter, area_px, divider);
            divider_view.place(gap, scale, glass);
        }
    }

    fn drag_divider(self: &Rc<Self>, path: &[u8], axis: Axis, position_dip: (f64, f64)) {
        let Ok(fe) = self.area.cast::<FrameworkElement>() else { return };
        let (width, height) = (fe.ActualWidth().unwrap_or(0.0), fe.ActualHeight().unwrap_or(0.0));
        if width < 1.0 || height < 1.0 {
            return;
        }
        let layout = self.layout.borrow().clone();
        let Some(splitter) = layout.splitters(UNIT).into_iter().find(|s| s.path == path) else { return };
        let fraction = match axis {
            Axis::Row => position_dip.0 / width,
            Axis::Column => position_dip.1 / height,
        };
        let updated = layout.set_ratio_at(path, ratio_at(&splitter, fraction));
        *self.layout.borrow_mut() = updated;
        self.relayout();
    }
}

fn place(element: &impl Interface, px: PxRect, scale: f64) {
    let Ok(positioned) = element.cast::<UIElement>() else { return };
    let _ = Canvas::SetLeft(&positioned, f64::from(px.x) / scale);
    let _ = Canvas::SetTop(&positioned, f64::from(px.y) / scale);
    if let Ok(fe) = element.cast::<FrameworkElement>() {
        let _ = fe.SetWidth(f64::from(px.w) / scale);
        let _ = fe.SetHeight(f64::from(px.h) / scale);
    }
}

impl Divider {
    fn new(tab: &Rc<Tab>) -> Result<Rc<Divider>> {
        let element = load_xaml(DIVIDER_XAML)?;
        let divider = Rc::new(Divider {
            root: element.cast()?,
            line: find(&element, "Line")?,
            path: RefCell::new(Vec::new()),
            axis: Cell::new(Axis::Row),
            hot: Cell::new(false),
            dragging: Cell::new(false),
            glass: Cell::new(tab.glass.get()),
            revokers: RefCell::new(Vec::new()),
        });
        divider.attach(tab)?;
        Ok(divider)
    }

    fn attach(self: &Rc<Self>, tab: &Rc<Tab>) -> Result<()> {
        let root = ui(&self.root)?;
        let mut revokers = Vec::new();
        let (weak_divider, weak_tab): (Weak<Divider>, Weak<Tab>) = (Rc::downgrade(self), Rc::downgrade(tab));

        revokers.push(root.PointerEntered({
            let weak = weak_divider.clone();
            cb!("divider enter", |_, _| {
                if let Some(divider) = weak.upgrade() {
                    divider.hot.set(true);
                    divider.paint();
                }
            })
        })?);
        revokers.push(root.PointerMoved({
            let (weak, weak_tab) = (weak_divider.clone(), weak_tab.clone());
            cb!("divider moved", |_, args| {
                let (Some(divider), Some(tab), Some(args)) = (weak.upgrade(), weak_tab.upgrade(), args.as_ref()) else {
                    return;
                };
                let shape = if divider.axis.get() == Axis::Row {
                    InputSystemCursorShape::SizeWestEast
                } else {
                    InputSystemCursorShape::SizeNorthSouth
                };
                // XAML puts the arrow back on every move, so the resize cursor is set on every move too.
                if let Ok(root) = ui(&divider.root) {
                    let _ = crate::cursor::set_cursor(&root, shape);
                }
                if divider.dragging.get()
                    && let Ok(point) = args.GetCurrentPoint(&tab.area).and_then(|p| p.Position())
                {
                    let path = divider.path.borrow().clone();
                    tab.drag_divider(&path, divider.axis.get(), (f64::from(point.x), f64::from(point.y)));
                }
            })
        })?);
        revokers.push(root.PointerExited({
            let weak = weak_divider.clone();
            cb!("divider exit", |_, _| {
                if let Some(divider) = weak.upgrade() {
                    divider.hot.set(false);
                    divider.paint();
                    if let Ok(root) = ui(&divider.root) {
                        let _ = crate::cursor::set_cursor(&root, InputSystemCursorShape::Arrow);
                    }
                }
            })
        })?);
        revokers.push(root.PointerPressed({
            let weak = weak_divider.clone();
            cb!("divider pressed", |_, args| {
                let (Some(divider), Some(args)) = (weak.upgrade(), args.as_ref()) else { return };
                divider.dragging.set(true);
                divider.paint();
                if let Ok(pointer) = args.Pointer() {
                    let _ = ui(&divider.root).and_then(|root| root.CapturePointer(&pointer));
                }
                let _ = args.SetHandled(true);
            })
        })?);
        revokers.push(root.PointerReleased({
            let weak = weak_divider.clone();
            cb!("divider released", |_, args| {
                let Some(divider) = weak.upgrade() else { return };
                divider.dragging.set(false);
                divider.paint();
                if let Some(pointer) = args.as_ref().and_then(|a| a.Pointer().ok()) {
                    let _ = ui(&divider.root).and_then(|root| root.ReleasePointerCapture(&pointer));
                }
            })
        })?);
        revokers.push(root.PointerCaptureLost({
            let weak = weak_divider;
            cb!("divider capture lost", |_, _| {
                if let Some(divider) = weak.upgrade() {
                    divider.dragging.set(false);
                    divider.paint();
                }
            })
        })?);
        *self.revokers.borrow_mut() = revokers;
        Ok(())
    }

    /// Puts the divider into the gap `gap` (pixels) with its grab strip centred on it.
    fn place(&self, gap: PxRect, scale: f64, glass: Glass) {
        let hit_px = (DIVIDER_HIT * scale).round() as i32;
        let (x, y, w, h, line_w, line_h) = match self.axis.get() {
            Axis::Row => {
                let centre = gap.x + gap.w / 2;
                (centre - hit_px / 2, gap.y, hit_px, gap.h, f64::from(gap.w) / scale, f64::NAN)
            }
            Axis::Column => {
                let centre = gap.y + gap.h / 2;
                (gap.x, centre - hit_px / 2, gap.w, hit_px, f64::NAN, f64::from(gap.h) / scale)
            }
        };
        place(&self.root, PxRect { x, y, w, h }, scale);
        self.glass.set(glass);
        if let Ok(line) = self.line.cast::<FrameworkElement>() {
            let _ = line.SetWidth(line_w);
            let _ = line.SetHeight(line_h);
            let (horizontal, vertical) = match self.axis.get() {
                Axis::Row => (HorizontalAlignment::Center, VerticalAlignment::Stretch),
                Axis::Column => (HorizontalAlignment::Stretch, VerticalAlignment::Center),
            };
            let _ = line.SetHorizontalAlignment(horizontal);
            let _ = line.SetVerticalAlignment(vertical);
        }
        let _ = Canvas::SetZIndex(&self.root, 10);
        self.paint();
    }

    /// The line is quiet at rest and takes the accent while the pointer is on it or drags it.
    fn paint(&self) {
        let glass = self.glass.get();
        let color = if self.hot.get() || self.dragging.get() { glass.accent } else { glass.divider() };
        if let Ok(brush) = solid_brush(color) {
            let _ = self.line.SetBackground(&brush);
        }
    }
}
