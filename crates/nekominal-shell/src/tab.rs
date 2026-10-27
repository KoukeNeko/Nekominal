//! One tab: the panes of its layout tree, the one that has the keyboard, and where each of them is in the window.

use nekominal_model::layout::{Axis, Direction, Node, PaneId, Splitter, UNIT};
use nekominal_pane::{Pane, PaneRenderer};

use crate::logic::{self, Attention, PaneSignal, PxRect, Reported, TabStatus};

/// How far one keyboard step moves the border of a pane, as a share of the tab area.
const RESIZE_STEP: f64 = 0.05;

/// Where a pane's reader thread sends its "there is output" message: the window that shows the pane now. A tab moved
/// to another window points its panes at that window, so the reader threads keep working through the move.
pub struct WakeTarget {
    pub hwnd: std::sync::atomic::AtomicIsize,
    /// One posted message per burst of output: the handler clears it when it draws.
    pub pending: std::sync::atomic::AtomicBool,
}

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
    /// The working directory the shell reported (OSC 7).
    pub cwd: Option<String>,
    /// The profile the session was started with (its name and its id), and the name of its program without the extension.
    pub profile: String,
    pub profile_id: String,
    pub shell: String,
    pub exited: bool,
    /// The file name of the program the pane runs now (the shell at a prompt), as the last look at its processes found it.
    pub job: Option<String>,
    /// The file of that program, for its icon.
    pub job_path: Option<std::path::PathBuf>,
    /// An agent has run in the pane since its shell was last back at its prompt alone: the status bar is the agent's.
    pub agent_seen: bool,
    /// When the pane was first seen running an agent by the name of its program, for the run time of the agents panel.
    pub agent_since: Option<std::time::Instant>,
    /// Ticks in a row the shell has been seen alone: the agent is taken for gone only after a few, as one look at the
    /// processes can miss one that runs.
    pub alone_ticks: u8,
    /// A bell, a notification or a failed exit nobody has looked at.
    pub attention: Option<Attention>,
    /// What the notification said.
    pub note: Option<String>,
    /// Progress the program reports (OSC 9;4).
    pub progress: Option<Reported>,
    pub wake: std::sync::Arc<WakeTarget>,
    /// The name of the pane for the status line command of Claude Code (`agent::SESSION_VAR`), and what that command
    /// last handed over, with when it did.
    pub session: String,
    pub usage: Option<crate::agent::Usage>,
    pub usage_stamp: Option<std::time::SystemTime>,
    /// When the hook of the pane's agent (`hooks.rs`) last wrote its file, as last read.
    pub hook_stamp: Option<std::time::SystemTime>,
    /// The Git branch of the folder the pane is in.
    pub branch: Option<String>,
    /// The scheme and the font size of its profile, when the profile sets its own.
    pub scheme: Option<String>,
    pub font_size: Option<f64>,
    /// What the send triggers of the pane sent lately (`triggers.rs`).
    pub sends: nekominal_model::triggers::SendGuard,
}

impl Drop for PaneEntry {
    fn drop(&mut self) {
        crate::agent::forget(&self.session);
    }
}

/// Hands out the number that tells one tab from another for as long as the process lives, whichever window shows it.
static NEXT_TAB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub struct Tab {
    /// Stays with the tab when it moves to another window.
    pub uid: u64,
    pub tree: Node,
    pub panes: Vec<PaneEntry>,
    pub focused: PaneId,
    /// Only the focused pane is shown, over the whole area.
    pub zoomed: bool,
    /// The name the user gave the tab, shown instead of what its program says; none for the automatic title.
    pub name: Option<String>,
    /// The colour the user gave the tab, an index into `logic::TAB_COLORS`.
    pub color: Option<usize>,
    /// What is typed goes to every pane of the tab, not only the focused one.
    pub broadcast: bool,
    next: PaneId,
}

impl Tab {
    /// A tab with one pane; its id is the first one this tab hands out.
    pub fn new(make: impl FnOnce(PaneId) -> Result<PaneEntry, String>) -> Result<Tab, String> {
        let entry = make(1)?;
        Ok(Tab { uid: NEXT_TAB.fetch_add(1, std::sync::atomic::Ordering::Relaxed), tree: Node::Leaf(1), focused: 1, panes: vec![entry], zoomed: false, name: None, color: None, broadcast: false, next: 2 })
    }

    /// A tab as a saved session had it: the layout with its ratios, a pane for every leaf (make starts it), the pane that
    /// had the keyboard and whether it was zoomed. None when no pane could be started.
    pub fn restore(
        layout: nekominal_model::layout::Node,
        active: PaneId,
        zoomed: bool,
        mut make: impl FnMut(PaneId) -> Result<PaneEntry, String>,
    ) -> Option<Tab> {
        let mut layout = layout;
        let mut panes = Vec::new();
        for id in layout.pane_ids() {
            match make(id) {
                Ok(entry) => panes.push(entry),
                Err(e) => {
                    crate::util::log(&format!("restore: {e}"));
                    // The panes that started keep their tab; the one that could not leaves the layout, as a closed one
                    // does. A tab none of whose panes start is not restored.
                    layout = layout.remove_pane(id)?;
                }
            }
        }
        if panes.is_empty() {
            return None;
        }
        let ids = layout.pane_ids();
        let focused = if ids.contains(&active) { active } else { ids[0] };
        let next = ids.iter().max().map_or(1, |highest| highest + 1);
        Some(Tab { uid: NEXT_TAB.fetch_add(1, std::sync::atomic::Ordering::Relaxed), tree: layout, focused, panes, zoomed: zoomed && ids.len() > 1, name: None, color: None, broadcast: false, next })
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

    /// The program the tab runs: what the focused pane runs.
    pub fn job(&self) -> Option<&str> {
        self.focused_entry().and_then(|p| p.job.as_deref())
    }

    /// The file of the program the tab runs, for its icon.
    pub fn job_path(&self) -> Option<&std::path::Path> {
        self.focused_entry().and_then(|p| p.job_path.as_deref())
    }

    /// What the tab shows: the status of its most urgent pane.
    pub fn status(&self) -> TabStatus {
        let signals: Vec<PaneSignal> = self
            .panes
            .iter()
            .map(|p| PaneSignal { agent: p.pane.agent_summary(), progress: p.progress, attention: p.attention, note: p.note.clone() })
            .collect();
        logic::tab_status(&signals)
    }

    /// The user is looking at the tab: its bells and notifications are seen. A failed exit stays.
    pub fn mark_seen(&mut self) {
        for entry in &mut self.panes {
            if entry.attention != Some(Attention::Exit) {
                entry.attention = None;
                entry.note = None;
            }
        }
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

    /// Takes a pane out of a tab of more than one, running, for another tab; the focus goes to a neighbour. None for the
    /// last pane of the tab, which stays.
    pub fn take(&mut self, id: PaneId) -> Option<PaneEntry> {
        if self.panes.len() < 2 {
            return None;
        }
        let neighbour = [Direction::Left, Direction::Up, Direction::Right, Direction::Down].into_iter().find_map(|d| self.tree.neighbor(id, d));
        self.tree = self.tree.remove_pane(id)?;
        let entry = self.panes.iter().position(|p| p.id == id).map(|i| self.panes.remove(i))?;
        if self.focused == id {
            self.focused = neighbour.or_else(|| self.panes.first().map(|p| p.id)).unwrap_or(self.focused);
        }
        self.zoomed = false;
        Some(entry)
    }

    /// A tab of one pane taken out of another (see `take`): the pane keeps its id and its session.
    pub fn from_entry(entry: PaneEntry) -> Tab {
        let id = entry.id;
        Tab { uid: NEXT_TAB.fetch_add(1, std::sync::atomic::Ordering::Relaxed), tree: Node::Leaf(id), focused: id, panes: vec![entry], zoomed: false, name: None, color: None, broadcast: false, next: id + 1 }
    }

    /// The focused pane trades places with its neighbour in `direction`, and keeps the keyboard. False when there is none.
    pub fn swap_focused(&mut self, direction: Direction) -> bool {
        let Some(other) = self.tree.neighbor(self.focused, direction) else { return false };
        self.tree = self.tree.swap_panes(self.focused, other);
        true
    }

    /// The panes what is typed goes to besides the focused one: the others of the tab while it broadcasts.
    pub fn broadcast_targets(&self) -> Vec<PaneId> {
        if !self.broadcast {
            return Vec::new();
        }
        self.panes.iter().filter(|p| p.id != self.focused && !p.exited).map(|p| p.id).collect()
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
