//! One tab: the panes of its layout tree, the one that has the keyboard, and where each of them is in the window.

use nekominal_model::layout::{Axis, Direction, Node, PaneId, Splitter, UNIT};
use nekominal_pane::{Pane, PaneRenderer};

use crate::logic::{self, PxRect};

/// How far one keyboard step moves the border of a pane, as a share of the tab area.
const RESIZE_STEP: f64 = 0.05;

pub struct PaneEntry {
    pub id: PaneId,
    pub pane: Pane,
    pub view: PaneRenderer,
    /// Cells the pane has now.
    pub grid: (u16, u16),
    /// Where the pane is in the window, in pixels; set by [`Tab::place`].
    pub rect: PxRect,
    /// The title the program set.
    pub title: String,
    pub exited: bool,
}

pub struct Tab {
    pub tree: Node,
    pub panes: Vec<PaneEntry>,
    pub focused: PaneId,
    /// Only the focused pane is shown, over the whole area.
    pub zoomed: bool,
    next: PaneId,
}

impl Tab {
    /// A tab with one pane; its id is the first one this tab hands out.
    pub fn new(make: impl FnOnce(PaneId) -> Result<PaneEntry, String>) -> Result<Tab, String> {
        let entry = make(1)?;
        Ok(Tab { tree: Node::Leaf(1), focused: 1, panes: vec![entry], zoomed: false, next: 2 })
    }

    pub fn next_id(&mut self) -> PaneId {
        let id = self.next;
        self.next += 1;
        id
    }

    pub fn entry(&self, id: PaneId) -> Option<&PaneEntry> {
        self.panes.iter().find(|p| p.id == id)
    }

    pub fn entry_mut(&mut self, id: PaneId) -> Option<&mut PaneEntry> {
        self.panes.iter_mut().find(|p| p.id == id)
    }

    pub fn focused_entry(&self) -> Option<&PaneEntry> {
        self.entry(self.focused)
    }

    pub fn focused_entry_mut(&mut self) -> Option<&mut PaneEntry> {
        let id = self.focused;
        self.entry_mut(id)
    }

    /// The title of the tab: what the focused pane's program set.
    pub fn title(&self) -> &str {
        self.focused_entry().map_or("", |p| p.title.as_str())
    }

    /// Adds a pane next to the focused one (below it for a column, right of it for a row) and focuses it.
    pub fn split(&mut self, axis: Axis, entry: PaneEntry) {
        self.tree = self.tree.split_pane(self.focused, axis, entry.id);
        self.focused = entry.id;
        self.zoomed = false;
        self.panes.push(entry);
    }

    /// Removes a pane; the tab is empty (`true`) when it was the last one. The focus goes to a neighbour.
    pub fn remove(&mut self, id: PaneId) -> bool {
        let neighbour = [Direction::Left, Direction::Up, Direction::Right, Direction::Down]
            .into_iter()
            .find_map(|d| self.tree.neighbor(id, d));
        match self.tree.remove_pane(id) {
            None => {
                self.panes.retain(|p| p.id != id);
                true
            }
            Some(tree) => {
                self.tree = tree;
                if let Some(entry) = self.panes.iter().position(|p| p.id == id).map(|i| self.panes.remove(i)) {
                    entry.pane.shutdown();
                }
                if self.focused == id {
                    self.focused = neighbour.or_else(|| self.panes.first().map(|p| p.id)).unwrap_or(self.focused);
                }
                self.zoomed = false;
                false
            }
        }
    }

    /// Computes the pixel rectangle of every shown pane inside `area` (window pixels); `divider` is the gap between panes.
    pub fn place(&mut self, area: PxRect, divider: i32) {
        let rects: Vec<(PaneId, PxRect)> = if self.zoomed {
            vec![(self.focused, area)]
        } else {
            self.tree
                .layout_rects(UNIT)
                .into_iter()
                .map(|(id, r)| {
                    let p = logic::pane_px_rect(r, (area.w, area.h), divider);
                    (id, PxRect { x: area.x + p.x, y: area.y + p.y, ..p })
                })
                .collect()
        };
        for (id, rect) in rects {
            if let Some(entry) = self.entry_mut(id) {
                entry.rect = rect;
            }
        }
    }

    /// The panes that are on the screen now.
    pub fn shown(&self) -> Vec<PaneId> {
        if self.zoomed { vec![self.focused] } else { self.tree.pane_ids() }
    }

    /// The dividers between panes, with their rectangles in window pixels (none while zoomed).
    pub fn splitters(&self, area: PxRect, divider: i32) -> Vec<(Splitter, PxRect)> {
        if self.zoomed {
            return Vec::new();
        }
        self.tree
            .splitters(UNIT)
            .into_iter()
            .map(|s| {
                let p = logic::splitter_px_rect(&s, (area.w, area.h), divider);
                (s, PxRect { x: area.x + p.x, y: area.y + p.y, ..p })
            })
            .collect()
    }

    /// The divider within `slop` pixels of a window pixel.
    pub fn splitter_at(&self, area: PxRect, divider: i32, x: i32, y: i32, slop: i32) -> Option<Splitter> {
        self.splitters(area, divider)
            .into_iter()
            .find(|(_, r)| x >= r.x - slop && x < r.x + r.w + slop && y >= r.y - slop && y < r.y + r.h + slop)
            .map(|(s, _)| s)
    }

    /// Moves the divider at `path` to a window pixel.
    pub fn drag_splitter(&mut self, area: PxRect, path: &[u8], x: i32, y: i32) {
        let Some(splitter) = self.tree.splitters(UNIT).into_iter().find(|s| s.path == path) else { return };
        let position = match splitter.axis {
            Axis::Row => f64::from(x - area.x) / f64::from(area.w.max(1)),
            Axis::Column => f64::from(y - area.y) / f64::from(area.h.max(1)),
        };
        self.tree = self.tree.set_ratio_at(path, logic::ratio_at(&splitter, position));
    }

    pub fn focus(&mut self, id: PaneId) -> bool {
        let changed = self.focused != id && self.entry(id).is_some();
        if changed {
            self.focused = id;
        }
        changed
    }

    pub fn focus_direction(&mut self, direction: Direction) -> bool {
        self.tree.neighbor(self.focused, direction).is_some_and(|id| self.focus(id))
    }

    pub fn resize_focused(&mut self, direction: Direction) {
        self.tree = self.tree.resize_pane(self.focused, direction, RESIZE_STEP);
    }

    /// The shown pane under a window pixel.
    pub fn pane_at(&self, x: i32, y: i32) -> Option<PaneId> {
        self.shown().into_iter().find(|id| {
            self.entry(*id).is_some_and(|p| x >= p.rect.x && y >= p.rect.y && x < p.rect.x + p.rect.w && y < p.rect.y + p.rect.h)
        })
    }
}
