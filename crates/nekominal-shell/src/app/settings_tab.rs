//! The settings as a tab of the window, as Windows Terminal has them: one tab a window at most, with a gear and the title of
//! the settings on its chip, and the page (`settings_page`, drawn by `App::paint_page`) in the place of the panes while it
//! is selected. The tab has no pane, so what looks through the panes of the tabs (agents, notifications, the search of every
//! tab, the next tab that waits) finds nothing in it. It is not kept in the session or in a layout, and it is not torn off
//! into a window of its own: it can be moved among the other tabs of its strip only.

use super::*;

/// What acts on the panes of the selected tab, or on the tab as a terminal (its name, a copy of it): on the settings tab
/// it does nothing.
pub(super) fn needs_pane(command: Command) -> bool {
    matches!(
        command,
        Command::PaneSplitRight
            | Command::PaneSplitDown
            | Command::PaneClose
            | Command::PaneZoom
            | Command::PaneRestart
            | Command::PaneFocus(_)
            | Command::PaneResize(_)
            | Command::PaneSwap(_)
            | Command::PaneToTab
            | Command::PaneBroadcast
            | Command::EditExport
            | Command::EditMarkMode
            | Command::EditCopyFormatted
            | Command::EditSelectOutput
            | Command::EditCopyOutput
            | Command::EditHints
            | Command::EditSuggestions
            | Command::EditCopy
            | Command::EditSmartCopy
            | Command::EditSelectAll
            | Command::EditFind
            | Command::EditClear
            | Command::ScrollPageUp
            | Command::ScrollPageDown
            | Command::ScrollTop
            | Command::ScrollBottom
            | Command::PromptPrevious
            | Command::PromptNext
            | Command::TabRename
            | Command::TabDuplicate
            | Command::TabNewWorktree
    )
}

/// The shortcuts that still work while the settings tab has the keyboard: those that open, switch, move or close tabs, the
/// palette, and those of the window itself. The others are the terminal's, and a key of the page is not taken from it by
/// them (the page has the arrows, Tab, Enter, and the letters of its fields).
pub(super) fn works_on_page(command: Command) -> bool {
    matches!(
        command,
        Command::TabNew
            | Command::TabClose
            | Command::TabNext
            | Command::TabPrevious
            | Command::TabGoto(_)
            | Command::TabSelectLeft
            | Command::TabSelectRight
            | Command::TabNextNeedsResponse
            | Command::TabMoveLeft
            | Command::TabMoveRight
            | Command::TabProfiles
            | Command::AppPalette
            | Command::AppSettings
            | Command::AppNewWindow
            | Command::ViewFullscreen
            | Command::ViewFocusMode
            | Command::ViewSidebar
    )
}

/// Whether a press or a release of the pointer goes to the page: one over the area of the panes, or any while a slider of
/// the page is held, but not the release of a press that began on the chrome (a chip, a button of the bar).
pub(super) fn button_to_page(hit: Hit, chrome_pressed: bool, slider_held: bool) -> bool {
    !chrome_pressed && (slider_held || hit == Hit::Pane)
}

/// The place of the settings tab among `tabs`: the one there is, or a new one after the others.
pub(super) fn settings_tab(tabs: &mut Vec<Tab>) -> usize {
    if let Some(index) = tabs.iter().position(|tab| tab.settings) {
        return index;
    }
    tabs.push(Tab::settings());
    tabs.len() - 1
}

/// The tabs the session and the layouts keep (not the settings), and the place among them of the one in view: with the
/// settings in view, the tab after them, or the last when they are last.
pub(super) fn kept(tabs: &[Tab], selected: usize) -> (Vec<&Tab>, usize) {
    let kept: Vec<&Tab> = tabs.iter().filter(|tab| !tab.settings).collect();
    let before = tabs.iter().take(selected).filter(|tab| !tab.settings).count();
    let active = before.min(kept.len().saturating_sub(1));
    (kept, active)
}

impl App {
    /// The settings tab is the tab in view, and the page is there to show in it.
    pub(super) fn page_shown(&self) -> bool {
        self.page.is_some() && self.tab().is_some_and(|tab| tab.settings)
    }

    /// The input method composes into a line of the page: the system draws the composition, under the caret of the line.
    pub fn ime_in_page(&self) -> bool {
        self.page_shown() && !self.popup_keeps_owner_active()
    }

    /// The area of the page: that of the panes, in DIP.
    pub(super) fn page_area(&self) -> Rect {
        let area = self.pane_area();
        let s = self.scale;
        Rect::new(area.x as f32 / s, area.y as f32 / s, area.w as f32 / s, area.h as f32 / s)
    }

    /// Asks for a frame with the page in it.
    pub fn page_redraw(&self) {
        self.redraw();
    }

    /// The rectangles of the page for the state it is in now.
    pub(super) fn page_frame(&mut self) -> Option<settings_page::Frame> {
        let lang = self.language();
        let area = self.page_area();
        let page = self.page.as_ref()?;
        // The pane is read only while a rule's form is open: the form is all that shows it.
        let sample = if page.trigger.is_some() { self.trigger_sample() } else { Vec::new() };
        let ui = &mut self.ui;
        let update = self.update.lock().map(|state| state.0.clone()).unwrap_or(update::Check::Failed);
        let push_test = crate::push::test_status().map(|(at, test)| (at.elapsed(), test));
        let view = settings_page::View { lang, settings: &self.settings, keymap: &self.keymap, profiles: &self.profiles, found: &self.found, programs: &self.programs, claude: &self.claude, area, update, updates_itself: update::scoop_dir().is_some() || (update::msi_dir().is_some() && !super::elevated()), push_test, sample: &sample };
        Some(settings_page::build(page, &view, &mut |text, size, bold| ui.measure(text, size, bold)))
    }

    /// Opens the settings tab at the section of the row `focus`, with that row under the keyboard. A tab that is open already
    /// is selected, and kept as it was when no row is asked for.
    pub(super) fn open_settings_page(&mut self, focus: Option<&str>) {
        self.close_popup();
        // Claude Code may have been installed or connected elsewhere since the page last looked.
        self.claude = agent::claude();
        let open = self.page.is_some() && self.tabs.iter().any(|tab| tab.settings);
        let mut page = self.page.take().unwrap_or_default();
        let row = focus.and_then(settings_ui::find);
        if !open || row.is_some() {
            page.query.clear();
            page.cursor = textfield::Cursor::default();
            page.editing = None;
            page.scroll = 0.0;
            page.focus = None;
        }
        if let Some(row) = row {
            page.section = Section::ALL.iter().position(|s| *s == row.section).unwrap_or(0);
        }
        page.layouts = workspace::layout_names();
        self.page = Some(page);
        self.selected = settings_tab(&mut self.tabs);
        self.relayout();
        self.sync_focus();
        if let Some(row) = row
            && let Some(frame) = self.page_frame()
            && let Some(at) = frame.entries.iter().position(|entry| matches!(entry, settings_page::Entry::Setting(r) if r.id == row.id))
            && let Some(page) = self.page.as_mut()
        {
            page.focus = Some(at);
            page.scroll = frame.reveal(at, 0.0).min(frame.max_scroll());
        }
        self.redraw();
    }

    /// Closes the settings tab (Escape on the page with nothing to clear).
    pub(super) fn request_close_settings(&mut self) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.settings) {
            self.close_tab(index);
        }
    }

    /// Rows that were scrolled to the end of a tall page stay inside a page that has grown.
    pub(super) fn fit_page_scroll(&mut self) {
        if let Some(max) = self.page_frame().map(|frame| frame.max_scroll())
            && let Some(page) = self.page.as_mut()
        {
            page.scroll = page.scroll.min(max);
        }
    }

    /// A key went down while the settings tab is in view. `true` when the page or a shortcut that works on it took it; the
    /// other shortcuts are taken too, and do nothing: there is no terminal for them.
    pub(super) fn page_tab_key(&mut self, vk_code: i32, scan: u16, extended: bool, repeat: bool) -> bool {
        let handled = self.page_takes_key(vk_code, scan, extended, repeat);
        if handled && !repeat {
            // Its release must not reach the terminal of the tab the key may have switched to (or closed the settings for).
            self.shortcut_keys.push(scan);
        }
        handled
    }

    fn page_takes_key(&mut self, vk_code: i32, scan: u16, extended: bool, repeat: bool) -> bool {
        let input_method = vk_code == vk::VK_PROCESSKEY || vk_code == vk::VK_PACKET;
        if self.page.as_ref().is_some_and(|page| page.capturing.is_some()) && self.capture_chord(vk_code, scan, extended, repeat) {
            return true;
        }
        let held = held();
        if !input_method && (self.page_field_key(vk_code) || (!held.ctrl && !held.alt && self.page_key(vk_code, repeat))) {
            return true;
        }
        let Some(command) = nekominal_input::shortcut_key(vk_code, scan, extended).and_then(|key| self.keymap.find(held, key.name())) else { return false };
        match command {
            Command::EditPaste if !repeat => self.paste_into_page(),
            command if works_on_page(command) && (!repeat || command.repeats()) => self.run(command),
            _ => {}
        }
        true
    }

    /// Puts the first line of the clipboard into the line of text that has the keyboard: the search or the field being edited.
    fn paste_into_page(&mut self) {
        let text = clipboard::read_text().unwrap_or_default();
        let line = text.lines().next().unwrap_or_default().to_owned();
        for unit in line.encode_utf16() {
            self.page_text(unit);
        }
    }

    /// Where the caret of the line of text being typed in is, in pixels of the window: left, top and height (the candidate
    /// window of the input method hangs under it).
    pub(super) fn page_ime_anchor(&mut self) -> Option<(i32, i32, i32)> {
        let scale = self.scale;
        let page = self.page.as_ref()?;
        let (scroll, editing, env_row) = (page.scroll, page.editing.as_ref().map(|(id, _)| *id), page.env_row);
        let frame = self.page_frame()?;
        let wanted = editing.map_or(Part::Search, |id| match id {
            profile_edit::ENV_KEY_ID => Part::EnvKey(env_row),
            profile_edit::ENV_VALUE_ID => Part::EnvValue(env_row),
            settings_page::SNIPPET_NAME_ID => Part::SnippetName(env_row),
            settings_page::SNIPPET_INPUT_ID => Part::SnippetInput(env_row),
            settings_page::LAYOUT_NAME_ID => Part::LayoutName(env_row),
            settings_page::TRIGGER_NAME_ID => Part::TriggerName(env_row),
            settings_page::TRIGGER_PATTERN_ID => Part::TriggerPattern(env_row),
            settings_page::TRIGGER_PROGRAM_ID => Part::TriggerProgram(env_row),
            settings_page::TRIGGER_TEXT_ID => Part::TriggerText(env_row),
            _ => profile_edit::Field::from_id(id).map_or(Part::Field(id), Part::ProfileField),
        });
        let item = frame.items.iter().find(|item| item.part == Some(wanted))?;
        let rect = frame.on_screen(item, scroll);
        let left = rect.x + if editing.is_some() { 10.0 } else { 40.0 };
        Some(((left * scale) as i32, ((rect.y + 4.0) * scale) as i32, ((rect.h - 8.0) * scale) as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_the_settings_twice_makes_one_tab_and_selects_it_again() {
        let mut tabs = vec![Tab::empty(), Tab::empty()];
        assert_eq!(settings_tab(&mut tabs), 2, "a new tab after the others");
        assert_eq!(settings_tab(&mut tabs), 2, "the same tab again");
        assert_eq!(tabs.iter().filter(|tab| tab.settings).count(), 1);
        tabs.swap(0, 2);
        assert_eq!(settings_tab(&mut tabs), 0, "where it was moved to");
        tabs.remove(0);
        assert_eq!(settings_tab(&mut tabs), 2, "closed, it opens anew");
    }

    #[test]
    fn the_session_and_the_layouts_keep_the_other_tabs_with_the_one_in_view() {
        let tabs = vec![Tab::empty(), Tab::settings(), Tab::empty()];
        let uid = |tab: &&Tab| tab.uid;
        let (kept_tabs, active) = kept(&tabs, 2);
        assert_eq!(kept_tabs.iter().map(uid).collect::<Vec<_>>(), vec![tabs[0].uid, tabs[2].uid]);
        assert_eq!(active, 1, "the last tab is still the one in view");
        assert_eq!(kept(&tabs, 0).1, 0);
        assert_eq!(kept(&tabs, 1).1, 1, "the settings in view: the tab after them");
        let last = vec![Tab::empty(), Tab::settings()];
        assert_eq!(kept(&last, 1).1, 0, "the settings last: the tab before them");
        let alone = vec![Tab::settings()];
        let (nothing, active) = kept(&alone, 0);
        assert!(nothing.is_empty() && active == 0, "nothing to keep: the next run starts afresh");
    }

    #[test]
    fn the_settings_tab_never_waits_and_the_next_tab_that_waits_passes_over_it() {
        let settings = Tab::settings();
        assert_eq!(settings.status().state, logic::TabState::Idle);
        assert!(settings.panes.is_empty() && settings.broadcast_targets().is_empty());
        let waiting = [settings.status().state, logic::TabState::Blocked].map(logic::is_urgent);
        assert_eq!(logic::next_waiting(&waiting, 1), None, "only the tab in view waits");
        assert_eq!(logic::next_waiting(&waiting, 0), Some(1));
    }

    #[test]
    fn what_needs_a_pane_does_nothing_on_the_settings_tab_and_its_shortcuts_stay_the_terminal_s() {
        for command in [Command::PaneSplitRight, Command::PaneSplitDown, Command::PaneZoom, Command::PaneBroadcast, Command::EditFind, Command::TabRename, Command::TabDuplicate, Command::ScrollTop] {
            assert!(needs_pane(command), "{} needs a pane", command.id());
            assert!(!works_on_page(command), "{} is not taken from the page", command.id());
        }
        // A command that works on the page never needs a pane.
        for command in commands::Command::ALL.iter().copied() {
            assert!(!(works_on_page(command) && needs_pane(command)), "{}", command.id());
        }
    }

    #[test]
    fn tab_switching_closing_and_the_palette_work_from_the_page() {
        for command in [Command::TabNext, Command::TabPrevious, Command::TabGoto(3), Command::TabClose, Command::TabNew, Command::AppPalette] {
            assert!(works_on_page(command), "{}", command.id());
        }
        for command in [Command::EditPaste, Command::EditCopy, Command::EditClear, Command::ScrollPageUp] {
            assert!(!works_on_page(command), "{}", command.id());
        }
    }

    #[test]
    fn the_page_gets_the_pointer_over_the_panes_area_and_while_a_slider_is_held() {
        assert!(button_to_page(Hit::Pane, false, false));
        assert!(!button_to_page(Hit::Chip(0), false, false), "the chips are the strip's");
        assert!(!button_to_page(Hit::NewTab, false, false));
        assert!(button_to_page(Hit::Chip(0), false, true), "a slider held over the strip");
        assert!(!button_to_page(Hit::Pane, true, false), "the release of a press on a chip is the chip's");
    }
}
