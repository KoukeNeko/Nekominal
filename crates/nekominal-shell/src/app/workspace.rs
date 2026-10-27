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
                let (tabs, active_tab) = self.tab_records(|_| None);
                let layout = Session { window: self.window_placement(), tabs, active_tab };
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
            Err(e) => {
                util::log(&format!("worktree {branch} in {folder}: {e:?}"));
                return self.inform(self.worktree_error(Text::WorktreeFailed, e));
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
                // Typed before the shell is up: the console keeps it until the shell reads its first line.
                if let (Some(setup), Some(entry)) = (crate::worktree::setup_command(std::path::Path::new(&path)), tab.focused_entry()) {
                    entry.pane.send_text(&format!("{setup}\r"));
                }
                self.tabs.push(tab);
                self.selected = self.tabs.len() - 1;
                self.relayout();
                self.sync_focus();
                self.redraw();
            }
            Err(e) => util::log(&format!("worktree tab: {e}")),
        }
    }

    /// The panes of the window whose folder is in the worktree at `path`, by the uid of their tab.
    fn panes_in(&self, path: &std::path::Path) -> Vec<(u64, PaneId)> {
        let inside = |entry: &&PaneEntry| entry.cwd.as_deref().is_some_and(|cwd| crate::worktree::contains(path, cwd));
        self.tabs.iter().flat_map(|tab| tab.panes.iter().filter(inside).map(|entry| (tab.uid, entry.id))).collect()
    }

    /// "Remove this worktree": the dialog of what the worktree of the focused pane holds and how to remove it, or why it
    /// is not removed.
    pub(super) fn ask_remove_worktree(&mut self) {
        match self.focused().and_then(|entry| entry.cwd.clone()) {
            Some(cwd) => self.ask_remove_worktree_in(std::path::Path::new(&cwd)),
            None => self.inform(Text::WorktreeNotRepo.get(self.language()).to_owned()),
        }
    }

    /// The dialog of Remove this worktree for the worktree `dir` is in.
    // shortcut: git runs on the thread of the window, a few short commands; move `inspect` to a thread if it is felt.
    fn ask_remove_worktree_in(&mut self, dir: &std::path::Path) {
        let lang = self.language();
        let status = match crate::worktree::inspect(dir) {
            Ok(status) => status,
            Err(e) => return self.inform(self.worktree_error(Text::WorktreeRemoveFailed, e)),
        };
        let mut tabs: Vec<u64> = self.panes_in(&status.path).into_iter().map(|(uid, _)| uid).collect();
        tabs.dedup();
        let agent = self
            .tabs
            .iter()
            .flat_map(|tab| &tab.panes)
            .filter(|entry| entry.cwd.as_deref().is_some_and(|cwd| crate::worktree::contains(&status.path, cwd)) && Self::runs_agent(entry))
            .find_map(|entry| {
                let named = entry.job.as_deref().and_then(crate::hooks::agent_of_program).map(crate::hooks::agent_name);
                // The status line of Claude Code is what hands over the usage.
                named.or_else(|| entry.usage.is_some().then(|| "Claude Code".to_owned()))
            });
        let pr = self.tabs.get(self.selected).and_then(|tab| self.badges.pull_request.get(&tab.uid)).map(|pr| pr.number);
        let facts = crate::remove_dialog::Facts { pr, tabs: tabs.len(), agent };
        let dialog = match crate::remove_dialog::model(&status, &facts, lang) {
            Ok(dialog) => dialog,
            Err(refusal) => return self.inform(self.worktree_error(Text::WorktreeRemoveFailed, crate::worktree::Error::Refused(refusal))),
        };
        let (accept, action) = match dialog.primary {
            crate::remove_dialog::Primary::Remove => (Text::WorktreeRemove, Confirmation::RemoveWorktree { path: status.path, delete_branch: dialog.delete_branch() }),
            crate::remove_dialog::Primary::ShowTab => (Text::WorktreeShowTab, Confirmation::ShowFolder(status.path)),
        };
        let row = |label: Text, action| popup::Row { label: label.get(lang).into(), hint: String::new(), action: Some(action), current: false, mark: None, program: None };
        let rows = vec![row(Text::CommonCancel, popup::Action::Cancel), row(accept, popup::Action::Confirm)];
        self.open_dialog(dialog.title.clone(), rows, action, Some(dialog));
    }

    /// Shows the tab of a pane in the folder `path`, the focused one first.
    pub(super) fn show_folder(&mut self, path: &std::path::Path) {
        let focused = self.tabs.get(self.selected).map(|tab| (tab.uid, tab.focused));
        let panes = self.panes_in(path);
        let Some(&(uid, id)) = panes.iter().find(|pane| Some(**pane) == focused).or(panes.first()) else { return };
        let Some(index) = self.tabs.iter().position(|tab| tab.uid == uid) else { return };
        self.select(index);
        self.tabs[index].focus(id);
        self.sync_focus();
        self.redraw();
    }

    /// Removes the worktree at `path` (asked by `ask_remove_worktree`), with its branch when `delete_branch`: inspected
    /// again, as it may have changed since (the dialog comes back when it can no longer go), then the panes in it close,
    /// the folder goes, and the branch with it or not.
    pub(super) fn remove_worktree(&mut self, path: &std::path::Path, delete_branch: bool) {
        let lang = self.language();
        let status = match crate::worktree::inspect(path) {
            Ok(status) => status,
            Err(e) => return self.inform(self.worktree_error(Text::WorktreeRemoveFailed, e)),
        };
        match crate::remove_dialog::model(&status, &Default::default(), lang) {
            Ok(dialog) if dialog.primary == crate::remove_dialog::Primary::Remove => {}
            Ok(_) => return self.ask_remove_worktree_in(path),
            Err(refusal) => return self.inform(self.worktree_error(Text::WorktreeRemoveFailed, crate::worktree::Error::Refused(refusal))),
        }
        let panes = self.panes_in(&status.path);
        for (uid, id) in &panes {
            if let Some(index) = self.tabs.iter().position(|tab| tab.uid == *uid) {
                self.close_pane(index, *id);
            }
        }
        if !panes.is_empty() {
            // shortcut: the shells of the closed panes end on another thread, and Windows keeps a folder that is a process's
            // working directory; a fixed wait on the thread of the window, until the UI removes the folder after they exit.
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let removed = if delete_branch { crate::worktree::remove(&status, false, true) } else { crate::worktree::archive(&status).map(|()| None) };
        match removed {
            Ok(None) => util::log(&format!("worktree removed: {}", status.path.display())),
            Ok(Some(said)) => self.inform(Text::WorktreeBranchKept.format(lang, &[&status.branch.unwrap_or_default(), &said])),
            Err(e) => {
                util::log(&format!("worktree {}: {e:?}", status.path.display()));
                self.inform(self.worktree_error(Text::WorktreeRemoveFailed, e));
            }
        }
    }

    /// What to tell the user of `error`; `failed` holds what git said.
    fn worktree_error(&self, failed: Text, error: crate::worktree::Error) -> String {
        use crate::worktree::{Error, Refusal};
        let lang = self.language();
        match error {
            Error::NotRepo => Text::WorktreeNotRepo.get(lang).to_owned(),
            Error::Git(said) => failed.format(lang, &[&said]),
            Error::Refused(Refusal::Main) => Text::WorktreeIsMain.get(lang).to_owned(),
            Error::Refused(Refusal::Dirty { changed, untracked }) => Text::WorktreeDirty.format(lang, &[&changed, &untracked]),
            Error::Refused(Refusal::Detached) => Text::WorktreeDetached.get(lang).to_owned(),
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

    /// Opens the tabs of `session` after the ones the window has and selects its active one; how many opened here.
    ///
    /// A tab whose focused pane ran as administrator comes back in an elevated window of its own, as it was opened; declined,
    /// it comes back here without elevation (the elevated window gets one pane: its splits are not kept). Windows' prompt
    /// holds up the thread until it is answered, so it is asked once this window shows, unless it has nothing else to show.
    pub(super) fn open_session(&mut self, session: &Session) -> usize {
        let (elevated, here): (Vec<&TabRecord>, Vec<&TabRecord>) = session.tabs.iter().partition(|record| self.elevated_profile(record).is_some());
        if here.is_empty() {
            let declined: Vec<TabRecord> = elevated.into_iter().filter(|record| !self.hand_off(record)).cloned().collect();
            return self.open_tabs(&declined, session.active_tab);
        }
        // The window asks for them once it shows (`elevates_later`); a window shown already asks now, after its own tabs.
        self.elevate_later.extend(elevated.into_iter().cloned());
        let opened = self.open_tabs(here, session.active_tab);
        if self.shown() {
            self.elevate_later();
        }
        opened
    }

    /// Whether tabs are waiting to be handed to elevated windows once the window shows.
    pub(crate) fn elevates_later(&mut self) -> bool {
        !self.elevate_later.is_empty()
    }

    /// Whether the window is on screen: a layout opened from the palette, not the session of a window being made.
    fn shown(&self) -> bool {
        // SAFETY: a plain query of a window of this thread.
        unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.hwnd) }.as_bool()
    }

    /// The tabs `open_session` left for the window to show first: each gets its elevated window, or opens here when declined.
    pub(crate) fn elevate_later(&mut self) {
        let records = std::mem::take(&mut self.elevate_later);
        let declined: Vec<TabRecord> = records.into_iter().filter(|record| !self.hand_off(record)).collect();
        if !declined.is_empty() {
            let selected = self.selected;
            self.open_tabs(&declined, 0);
            // The tab in view stays the one the session had in view.
            self.selected = selected;
            self.relayout();
            self.sync_focus();
            self.redraw();
        }
    }

    /// The profile a tab's focused pane ran as administrator with, when this window is not elevated.
    fn elevated_profile(&self, record: &TabRecord) -> Option<Profile> {
        let pane = record.panes.get(&record.active)?;
        self.profiles.iter().find(|p| p.id == pane.profile).filter(|p| p.options.elevate && !elevated()).cloned()
    }

    /// Opens the tab in an elevated window; false when that did not happen (declined, or there is no such profile).
    fn hand_off(&self, record: &TabRecord) -> bool {
        let Some(profile) = self.elevated_profile(record) else { return false };
        self.opens_elevated(&profile, record.panes.get(&record.active).and_then(|pane| pane.cwd.as_deref()))
    }

    /// Opens `records` as tabs after the ones the window has and selects the one at `active`; how many opened.
    fn open_tabs<'a>(&mut self, records: impl IntoIterator<Item = &'a TabRecord>, active: usize) -> usize {
        let area = self.pane_area();
        let size = (area.w.max(1) as u32, area.h.max(1) as u32);
        let before = self.tabs.len();
        for record in records {
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
            self.selected = before + active.min(added - 1);
            self.relayout();
            self.sync_focus();
        }
        added
    }

    /// The tabs of the window as the session and the layouts keep them (not the settings), and the place of the one in view
    /// among them; `buffer` names the saved text of a pane, if any.
    pub(super) fn tab_records(&self, mut buffer: impl FnMut(&PaneEntry) -> Option<String>) -> (Vec<TabRecord>, usize) {
        let (tabs, active) = settings_tab::kept(&self.tabs, self.selected);
        let records = tabs
            .into_iter()
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
            .collect();
        (records, active)
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
