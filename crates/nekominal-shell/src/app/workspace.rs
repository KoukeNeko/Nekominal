//! The palette as a line for a name that is not a tab's (the branch of a new worktree, the name of a saved layout), the tab
//! in a new git worktree, and the saved layouts: saving the tabs of the window, opening them here or in a window of their
//! own, renaming and deleting them. Apart from `app.rs`, which only hands over to it.

use std::cell::RefCell;

use nekominal_model::{palette::LayoutAction, workspaces};

use super::*;

/// What the line of the open palette is typed for.
pub(super) enum Entry {
    /// The new branch of a worktree of the repository this folder is in.
    Branch(String),
    /// The name to keep the tabs of the window under.
    SaveLayout,
    /// The new name of the saved layout of this name.
    RenameLayout(String),
}

thread_local! {
    /// The layout a window being made opens instead of a tab: every window lives on this thread, and the window that asks
    /// for one posts the message that makes the next.
    static PENDING: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// The layout a window opened from the palette is to show, once.
pub(super) fn take_pending() -> Option<Session> {
    PENDING.with(|pending| pending.borrow_mut().take())
}

/// The saved layouts, or none when the file is missing or unreadable; what cannot be read is told in the log.
fn read_layouts() -> workspaces::Layouts {
    let text = workspaces::path().and_then(|path| store::load(&path).map_err(|e| util::log(&format!("layouts: {e}"))).ok().flatten());
    let (layouts, problems) = text.as_deref().map(workspaces::parse).unwrap_or_default();
    for problem in problems {
        util::log(&format!("layouts: {problem}"));
    }
    layouts
}

/// The names of the saved layouts, in the order `App::layout_action` counts them in.
pub(super) fn layout_names() -> Vec<String> {
    read_layouts().into_keys().collect()
}

/// Changes the saved layouts with `change` and writes them. A file a newer version wrote is left as it is.
fn write_layouts(change: impl FnOnce(&mut workspaces::Layouts)) -> Result<(), String> {
    let path = workspaces::path().ok_or("APPDATA is not set")?;
    if store::load(&path).ok().flatten().is_some_and(|text| session::written_by_newer(&text)) {
        return Err(format!("{} was written by a newer version", path.display()));
    }
    let mut layouts = read_layouts();
    change(&mut layouts);
    store::save(&path, &workspaces::to_json(&layouts)).map_err(|e| e.to_string())
}

impl App {
    /// The palette as a line for `entry`, holding `text`, all of it selected.
    fn open_entry(&mut self, entry: Entry, text: String) {
        let anchor = self.screen_point((self.layout.size.0 + popup::PALETTE_WIDTH) / 2.0 * self.scale, (layout::TITLE_HEIGHT + 16.0) * self.scale);
        self.open_popup(Vec::new(), popup::Kind::Palette, anchor);
        let Some(popup) = self.popup.as_mut() else { return };
        popup.query = Some(text.clone());
        textfield::select_all(&text, &mut popup.cursor);
        self.entry = Some(entry);
        self.query_changed();
    }

    /// The row under the line: what Enter does with it, or what to type while there is nothing it can do.
    pub(super) fn entry_rows(&self, query: &str) -> Vec<popup::Row> {
        let lang = self.language();
        let typed = query.trim();
        let (label, action) = match &self.entry {
            Some(Entry::Branch(_)) if typed.is_empty() => (Text::WorktreeTypeBranch.get(lang).to_owned(), popup::Action::Cancel),
            Some(Entry::Branch(_)) if !crate::worktree::valid_branch(typed) => (Text::WorktreeBadBranch.format(lang, &[&typed]), popup::Action::Cancel),
            Some(Entry::Branch(_)) => (Text::WorktreeCreate.format(lang, &[&typed]), popup::Action::Entered),
            Some(_) if typed.is_empty() => (Text::LayoutTypeName.get(lang).to_owned(), popup::Action::Cancel),
            Some(Entry::RenameLayout(_)) => (Text::LayoutRenameTo.format(lang, &[&typed]), popup::Action::Entered),
            _ => (Text::LayoutSaveAs.format(lang, &[&typed]), popup::Action::Entered),
        };
        vec![popup::Row { label, hint: String::new(), action: Some(action), current: false, mark: None, program: None }]
    }

    /// Enter on the line of `entry`, which held `text`.
    pub(super) fn entered(&mut self, entry: Entry, text: &str) {
        let typed = text.trim();
        match entry {
            Entry::Branch(folder) => self.open_worktree_tab(&folder, typed),
            Entry::SaveLayout => {
                let Some(name) = workspaces::name(typed) else { return };
                let layout = Session { window: self.window_placement(), tabs: self.tab_records(|_| None), active_tab: self.selected };
                if let Err(e) = write_layouts(|layouts| { layouts.insert(name, layout); }) {
                    self.layout_failed(e);
                }
            }
            Entry::RenameLayout(old) => {
                let Some(name) = workspaces::name(typed) else { return };
                let renamed = write_layouts(|layouts| {
                    if let Some(layout) = layouts.remove(&old) {
                        layouts.insert(name, layout);
                    }
                });
                if let Err(e) = renamed {
                    self.layout_failed(e);
                }
            }
        }
    }

    fn layout_failed(&mut self, error: String) {
        util::log(&format!("layouts: {error}"));
        self.inform(Text::LayoutFailed.format(self.language(), &[&error]));
    }

    /// Asks for the branch of a new worktree of the repository the focused pane is in.
    pub(super) fn open_worktree_entry(&mut self) {
        match self.focused().and_then(|entry| entry.cwd.clone()) {
            Some(folder) => self.open_entry(Entry::Branch(folder), String::new()),
            None => self.inform(Text::WorktreeNotRepo.get(self.language()).to_owned()),
        }
    }

    /// Makes the worktree of `branch` for the repository `folder` is in, and a tab of the default profile there named after
    /// the branch.
    fn open_worktree_tab(&mut self, folder: &str, branch: &str) {
        let path = match crate::worktree::create(std::path::Path::new(folder), branch) {
            Ok(path) => path.display().to_string(),
            Err(crate::worktree::Error::NotRepo) => return self.inform(Text::WorktreeNotRepo.get(self.language()).to_owned()),
            Err(crate::worktree::Error::Git(said)) => {
                util::log(&format!("worktree {branch} in {folder}: {said}"));
                return self.inform(Text::WorktreeFailed.format(self.language(), &[&said]));
            }
        };
        util::log(&format!("worktree {branch}: {path}"));
        let profile = self.profile.clone();
        if self.opens_elevated(&profile, Some(&path)) {
            return;
        }
        let area = self.pane_area();
        let size = (area.w.max(1) as u32, area.h.max(1) as u32);
        match Tab::new(|id| self.make_entry_with(id, size, &profile, Some(&path))) {
            Ok(mut tab) => {
                tab.name = Some(branch.to_owned());
                self.tabs.push(tab);
                self.selected = self.tabs.len() - 1;
                self.relayout();
                self.sync_focus();
                self.redraw();
            }
            Err(e) => util::log(&format!("worktree tab: {e}")),
        }
    }

    /// Asks for the name to keep the tabs of the window under.
    pub(super) fn open_layout_entry(&mut self) {
        self.open_entry(Entry::SaveLayout, String::new());
    }

    /// An entry of a saved layout was chosen: `index` is its place among the names, read again from the file.
    pub(super) fn layout_action(&mut self, action: LayoutAction, index: usize) {
        let Some((name, layout)) = read_layouts().into_iter().nth(index) else { return };
        match action {
            LayoutAction::Open => {
                self.open_session(&layout);
                self.redraw();
            }
            LayoutAction::OpenInNewWindow => {
                PENDING.with(|pending| *pending.borrow_mut() = Some(layout));
                // SAFETY: posting to our own window.
                if let Err(e) = unsafe { PostMessageW(Some(self.hwnd), crate::win::WM_NEW_WINDOW, WPARAM(0), LPARAM(0)) } {
                    util::log(&format!("new window: {e}"));
                }
            }
            LayoutAction::Rename => self.open_entry(Entry::RenameLayout(name.clone()), name),
            LayoutAction::Delete => {
                if let Err(e) = write_layouts(|layouts| { layouts.remove(&name); }) {
                    self.layout_failed(e);
                }
            }
        }
    }

    /// Opens the saved layout `name` here (the command line's `--layout`); told in the log when there is none.
    pub(super) fn open_layout_named(&mut self, name: &str) {
        match read_layouts().remove(name) {
            Some(layout) => {
                self.open_session(&layout);
                self.placement = layout.window;
            }
            None => util::log(&format!("layouts: there is no layout named {name:?}")),
        }
    }

    /// Opens the tabs of `session` after the ones the window has and selects its active one; how many opened.
    pub(super) fn open_session(&mut self, session: &Session) -> usize {
        let area = self.pane_area();
        let size = (area.w.max(1) as u32, area.h.max(1) as u32);
        let before = self.tabs.len();
        for record in &session.tabs {
            // A tab whose focused pane ran as administrator comes back in an elevated window of its own, as it was opened;
            // declined, it comes back here without elevation. The elevated window gets one pane: its splits are not kept.
            let active = record.panes.get(&record.active);
            if let Some(profile) = active.and_then(|pane| self.profiles.iter().find(|p| p.id == pane.profile)).cloned() {
                if self.opens_elevated(&profile, active.and_then(|pane| pane.cwd.as_deref())) {
                    continue;
                }
            }
            let restored = Tab::restore(record.layout.clone(), record.active, record.zoomed.is_some(), |id| {
                let pane = record.panes.get(&id);
                let profile = pane.and_then(|pane| self.profiles.iter().find(|p| p.id == pane.profile)).unwrap_or(&self.profile);
                let restored = pane.and_then(|pane| pane.buffer.as_deref()).and_then(buffers::take).map(|(text, written)| {
                    (text, Text::PaneRestored.format(self.language(), &[&buffers::local_time(written)]))
                });
                self.make_entry_restoring(id, size, profile, pane.and_then(|pane| pane.cwd.as_deref()), restored)
            });
            if let Some(mut tab) = restored {
                (tab.name, tab.color) = (record.name.clone(), record.color.filter(|c| *c < logic::TAB_COLORS.len()));
                self.tabs.push(tab);
            }
        }
        let added = self.tabs.len() - before;
        if added > 0 {
            self.selected = before + session.active_tab.min(added - 1);
            self.relayout();
            self.sync_focus();
        }
        added
    }

    /// The tabs of the window as the session and the layouts keep them; `buffer` names the saved text of a pane, if any.
    pub(super) fn tab_records(&self, mut buffer: impl FnMut(&PaneEntry) -> Option<String>) -> Vec<TabRecord> {
        self.tabs
            .iter()
            .map(|tab| TabRecord {
                layout: tab.tree.clone(),
                panes: tab
                    .panes
                    .iter()
                    .map(|entry| (entry.id, PaneRecord { profile: entry.profile_id.clone(), cwd: entry.cwd.clone(), buffer: buffer(entry) }))
                    .collect(),
                active: tab.focused,
                zoomed: tab.zoomed.then_some(tab.focused),
                name: tab.name.clone(),
                color: tab.color,
            })
            .collect()
    }

    /// Where the window is when it is not maximized or minimized, and whether it is maximized.
    pub(super) fn window_placement(&self) -> Option<Placement> {
        let mut placement = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        // SAFETY: a window of this thread and a structure that lives through the call.
        unsafe { GetWindowPlacement(self.hwnd, &mut placement) }.ok().map(|()| {
            let rect = placement.rcNormalPosition;
            Placement {
                rect: PixelRect { x: rect.left, y: rect.top, width: (rect.right - rect.left).max(0) as u32, height: (rect.bottom - rect.top).max(0) as u32 },
                maximized: placement.showCmd == SW_SHOWMAXIMIZED.0 as u32,
            }
        })
    }
}
