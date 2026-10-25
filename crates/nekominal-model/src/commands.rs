//! Every command the application can run, with its id, label, default shortcuts and where it applies.
//!
//! This is the single source for shortcuts, the command palette, tooltips and the keys page of the settings.

use crate::{
    layout::Direction,
    strings::{Lang, Text},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    TabNew,
    TabClose,
    TabNext,
    TabPrevious,
    /// One-based tab number, 1 to 9; 9 is the last tab (see [`Command::tab_goto_index`]). Other numbers are not
    /// commands: they have no id of their own.
    TabGoto(u8),
    /// The tab to the left or to the right of the selected one in the strip, wrapping at the ends. `TabNext` and
    /// `TabPrevious` follow the order of use instead.
    TabSelectLeft,
    TabSelectRight,
    /// The next tab in strip order after the selected one, wrapping, that has a pane waiting for the user.
    TabNextNeedsResponse,
    TabMoveLeft,
    TabMoveRight,
    /// The menu of profiles to open a new tab with.
    TabProfiles,
    PaneSplitRight,
    PaneSplitDown,
    PaneClose,
    PaneZoom,
    PaneRestart,
    PaneFocus(Direction),
    PaneResize(Direction),
    /// Trades places with the pane in a direction, which keeps the keyboard; moves the focused pane into a tab of its own;
    /// sends what is typed to every pane of the tab.
    PaneSwap(Direction),
    PaneToTab,
    PaneBroadcast,
    /// Writes all the text of the focused pane, the scrollback with it, to a file.
    EditExport,
    /// Selects with the keyboard: the arrows move a caret through the text, Shift selects, Enter copies.
    EditMarkMode,
    /// Copies the selection with its colours and style, as HTML beside the text.
    EditCopyFormatted,
    /// What the last command wrote, by the marks of the shell; again, the command before it.
    EditSelectOutput,
    EditCopyOutput,
    /// Labels on the URLs, paths and hashes on screen: a label copies its text, the same in capitals opens it.
    EditHints,
    /// Keeps the window over the others; hides the title bar and the sidebar; shows or hides the drop-down window.
    ViewAlwaysOnTop,
    ViewFocusMode,
    ViewQuake,
    /// Lists the snippets and the recent commands of PowerShell; the one chosen is typed at the prompt.
    EditSuggestions,
    EditCopy,
    /// Ctrl+C: copies only while text is selected, otherwise the key reaches the program as an interrupt.
    EditSmartCopy,
    EditPaste,
    EditSelectAll,
    EditFind,
    EditClear,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    PromptPrevious,
    PromptNext,
    ViewFullscreen,
    /// Hides or shows the sidebar of the tabs.
    ViewSidebar,
    /// Closes the windows, has Scoop update the app and starts it again.
    AppUpdate,
    /// Names the selected tab, gives it a colour, opens a copy of it, closes the others, closes those right of it.
    TabRename,
    TabColor,
    TabDuplicate,
    TabCloseOthers,
    TabCloseRight,
    ViewZoomIn,
    ViewZoomOut,
    ViewZoomReset,
    AppPalette,
    AppNewWindow,
    AppSettings,
    AppOpenSettingsFile,
    AppReloadSettings,
    /// Adds the colour schemes of Windows Terminal's settings that Nekominal has none of by name.
    AppImportSchemes,
    /// Adds the schemes of iTerm2 colour presets (`.itermcolors`) the user picks.
    AppImportItermColors,
    /// Makes a git worktree on a new branch beside the repository of the focused pane, and opens a tab in it.
    TabNewWorktree,
    /// Keeps the tabs of the window under a name, for the palette to open again (`workspaces`).
    AppSaveLayout,
    AppearanceBackdropNext,
    AppearanceOpacityUp,
    AppearanceOpacityDown,
}

impl Command {
    /// Every command once, in palette order.
    pub const ALL: &[Command] = &[
        Command::TabNew,
        Command::TabClose,
        Command::TabNext,
        Command::TabPrevious,
        Command::TabGoto(1),
        Command::TabGoto(2),
        Command::TabGoto(3),
        Command::TabGoto(4),
        Command::TabGoto(5),
        Command::TabGoto(6),
        Command::TabGoto(7),
        Command::TabGoto(8),
        Command::TabGoto(9),
        Command::TabSelectLeft,
        Command::TabSelectRight,
        Command::TabNextNeedsResponse,
        Command::TabMoveLeft,
        Command::TabMoveRight,
        Command::TabProfiles,
        Command::PaneSplitRight,
        Command::PaneSplitDown,
        Command::PaneClose,
        Command::PaneZoom,
        Command::PaneRestart,
        Command::PaneFocus(Direction::Left),
        Command::PaneFocus(Direction::Right),
        Command::PaneFocus(Direction::Up),
        Command::PaneFocus(Direction::Down),
        Command::PaneResize(Direction::Left),
        Command::PaneResize(Direction::Right),
        Command::PaneResize(Direction::Up),
        Command::PaneResize(Direction::Down),
        Command::PaneSwap(Direction::Left),
        Command::PaneSwap(Direction::Right),
        Command::PaneSwap(Direction::Up),
        Command::PaneSwap(Direction::Down),
        Command::PaneToTab,
        Command::PaneBroadcast,
        Command::EditExport,
        Command::EditMarkMode,
        Command::EditCopyFormatted,
        Command::EditSelectOutput,
        Command::EditCopyOutput,
        Command::EditHints,
        Command::ViewAlwaysOnTop,
        Command::ViewFocusMode,
        Command::ViewQuake,
        Command::EditSuggestions,
        Command::EditSmartCopy,
        Command::EditCopy,
        Command::EditPaste,
        Command::EditSelectAll,
        Command::EditFind,
        Command::EditClear,
        Command::ScrollPageUp,
        Command::ScrollPageDown,
        Command::ScrollTop,
        Command::ScrollBottom,
        Command::PromptPrevious,
        Command::PromptNext,
        Command::ViewFullscreen,
        Command::ViewSidebar,
        Command::AppUpdate,
        Command::TabRename,
        Command::TabColor,
        Command::TabDuplicate,
        Command::TabCloseOthers,
        Command::TabCloseRight,
        Command::ViewZoomIn,
        Command::ViewZoomOut,
        Command::ViewZoomReset,
        Command::AppPalette,
        Command::AppNewWindow,
        Command::AppSettings,
        Command::AppOpenSettingsFile,
        Command::AppReloadSettings,
        Command::AppImportSchemes,
        Command::AppImportItermColors,
        Command::TabNewWorktree,
        Command::AppSaveLayout,
        Command::AppearanceBackdropNext,
        Command::AppearanceOpacityUp,
        Command::AppearanceOpacityDown,
    ];

    /// The stable name used in `settings.json` keybindings and in the palette.
    pub fn id(self) -> &'static str {
        match self {
            Command::TabNew => "tab.new",
            Command::TabClose => "tab.close",
            Command::TabNext => "tab.next",
            Command::TabPrevious => "tab.previous",
            Command::TabGoto(1) => "tab.goto1",
            Command::TabGoto(2) => "tab.goto2",
            Command::TabGoto(3) => "tab.goto3",
            Command::TabGoto(4) => "tab.goto4",
            Command::TabGoto(5) => "tab.goto5",
            Command::TabGoto(6) => "tab.goto6",
            Command::TabGoto(7) => "tab.goto7",
            Command::TabGoto(8) => "tab.goto8",
            Command::TabGoto(9) => "tab.goto9",
            Command::TabGoto(_) => "tab.goto",
            Command::TabSelectLeft => "tab.selectLeft",
            Command::TabSelectRight => "tab.selectRight",
            Command::TabNextNeedsResponse => "tab.nextNeedsResponse",
            Command::TabMoveLeft => "tab.moveLeft",
            Command::TabMoveRight => "tab.moveRight",
            Command::TabProfiles => "tab.profiles",
            Command::PaneSplitRight => "pane.splitRight",
            Command::PaneSplitDown => "pane.splitDown",
            Command::PaneClose => "pane.close",
            Command::PaneZoom => "pane.zoom",
            Command::PaneRestart => "pane.restart",
            Command::PaneFocus(Direction::Left) => "pane.focusLeft",
            Command::PaneFocus(Direction::Right) => "pane.focusRight",
            Command::PaneFocus(Direction::Up) => "pane.focusUp",
            Command::PaneFocus(Direction::Down) => "pane.focusDown",
            Command::PaneResize(Direction::Left) => "pane.resizeLeft",
            Command::PaneResize(Direction::Right) => "pane.resizeRight",
            Command::PaneResize(Direction::Up) => "pane.resizeUp",
            Command::PaneResize(Direction::Down) => "pane.resizeDown",
            Command::PaneSwap(Direction::Left) => "pane.swapLeft",
            Command::PaneSwap(Direction::Right) => "pane.swapRight",
            Command::PaneSwap(Direction::Up) => "pane.swapUp",
            Command::PaneSwap(Direction::Down) => "pane.swapDown",
            Command::PaneToTab => "pane.toTab",
            Command::PaneBroadcast => "pane.broadcast",
            Command::EditExport => "edit.export",
            Command::EditMarkMode => "edit.markMode",
            Command::EditCopyFormatted => "edit.copyFormatted",
            Command::EditSelectOutput => "edit.selectOutput",
            Command::EditCopyOutput => "edit.copyOutput",
            Command::EditHints => "edit.hints",
            Command::ViewAlwaysOnTop => "view.alwaysOnTop",
            Command::ViewFocusMode => "view.focusMode",
            Command::ViewQuake => "view.quake",
            Command::EditSuggestions => "edit.suggestions",
            Command::EditCopy => "edit.copy",
            Command::EditSmartCopy => "edit.smartCopy",
            Command::EditPaste => "edit.paste",
            Command::EditSelectAll => "edit.selectAll",
            Command::EditFind => "edit.find",
            Command::EditClear => "edit.clear",
            Command::ScrollPageUp => "scroll.pageUp",
            Command::ScrollPageDown => "scroll.pageDown",
            Command::ScrollTop => "scroll.top",
            Command::ScrollBottom => "scroll.bottom",
            Command::PromptPrevious => "prompt.previous",
            Command::PromptNext => "prompt.next",
            Command::ViewFullscreen => "view.fullscreen",
            Command::ViewSidebar => "view.sidebar",
            Command::AppUpdate => "app.update",
            Command::TabRename => "tab.rename",
            Command::TabColor => "tab.color",
            Command::TabDuplicate => "tab.duplicate",
            Command::TabCloseOthers => "tab.closeOthers",
            Command::TabCloseRight => "tab.closeRight",
            Command::ViewZoomIn => "view.zoomIn",
            Command::ViewZoomOut => "view.zoomOut",
            Command::ViewZoomReset => "view.zoomReset",
            Command::AppPalette => "app.palette",
            Command::AppNewWindow => "app.newWindow",
            Command::AppSettings => "app.settings",
            Command::AppOpenSettingsFile => "app.openSettingsFile",
            Command::AppReloadSettings => "app.reloadSettings",
            Command::AppImportSchemes => "app.importSchemes",
            Command::AppImportItermColors => "app.importItermColors",
            Command::TabNewWorktree => "tab.newWorktree",
            Command::AppSaveLayout => "app.saveLayout",
            Command::AppearanceBackdropNext => "appearance.backdrop.next",
            Command::AppearanceOpacityUp => "appearance.opacity.up",
            Command::AppearanceOpacityDown => "appearance.opacity.down",
        }
    }

    pub fn from_id(id: &str) -> Option<Command> {
        Command::ALL
            .iter()
            .copied()
            .find(|command| command.id() == id)
    }

    /// The string of the label. `TabGoto` takes the tab number as its only argument.
    pub fn text(self) -> Text {
        match self {
            Command::TabNew => Text::TabNew,
            Command::TabClose => Text::TabClose,
            Command::TabNext => Text::TabNext,
            Command::TabPrevious => Text::TabPrevious,
            Command::TabGoto(_) => Text::TabGoto,
            Command::TabSelectLeft => Text::TabSelectLeft,
            Command::TabSelectRight => Text::TabSelectRight,
            Command::TabNextNeedsResponse => Text::TabNextNeedsResponse,
            Command::TabMoveLeft => Text::TabMoveLeft,
            Command::TabMoveRight => Text::TabMoveRight,
            Command::TabProfiles => Text::TabProfiles,
            Command::PaneSplitRight => Text::PaneSplitRight,
            Command::PaneSplitDown => Text::PaneSplitDown,
            Command::PaneClose => Text::PaneClose,
            Command::PaneZoom => Text::PaneZoom,
            Command::PaneRestart => Text::PaneRestart,
            Command::PaneFocus(Direction::Left) => Text::PaneFocusLeft,
            Command::PaneFocus(Direction::Right) => Text::PaneFocusRight,
            Command::PaneFocus(Direction::Up) => Text::PaneFocusUp,
            Command::PaneFocus(Direction::Down) => Text::PaneFocusDown,
            Command::PaneResize(Direction::Left) => Text::PaneResizeLeft,
            Command::PaneResize(Direction::Right) => Text::PaneResizeRight,
            Command::PaneResize(Direction::Up) => Text::PaneResizeUp,
            Command::PaneResize(Direction::Down) => Text::PaneResizeDown,
            Command::PaneSwap(Direction::Left) => Text::PaneSwapLeft,
            Command::PaneSwap(Direction::Right) => Text::PaneSwapRight,
            Command::PaneSwap(Direction::Up) => Text::PaneSwapUp,
            Command::PaneSwap(Direction::Down) => Text::PaneSwapDown,
            Command::PaneToTab => Text::PaneToTab,
            Command::PaneBroadcast => Text::PaneBroadcast,
            Command::EditExport => Text::EditExport,
            Command::EditMarkMode => Text::EditMarkMode,
            Command::EditCopyFormatted => Text::EditCopyFormatted,
            Command::EditSelectOutput => Text::EditSelectOutput,
            Command::EditCopyOutput => Text::EditCopyOutput,
            Command::EditHints => Text::EditHints,
            Command::ViewAlwaysOnTop => Text::ViewAlwaysOnTop,
            Command::ViewFocusMode => Text::ViewFocusMode,
            Command::ViewQuake => Text::ViewQuake,
            Command::EditSuggestions => Text::EditSuggestions,
            // Copying with and without a selection check is one action, so it keeps one label.
            Command::EditCopy | Command::EditSmartCopy => Text::EditCopy,
            Command::EditPaste => Text::EditPaste,
            Command::EditSelectAll => Text::EditSelectAll,
            Command::EditFind => Text::EditFind,
            Command::EditClear => Text::EditClear,
            Command::ScrollPageUp => Text::ScrollPageUp,
            Command::ScrollPageDown => Text::ScrollPageDown,
            Command::ScrollTop => Text::ScrollTop,
            Command::ScrollBottom => Text::ScrollBottom,
            Command::PromptPrevious => Text::PromptPrevious,
            Command::PromptNext => Text::PromptNext,
            Command::ViewFullscreen => Text::ViewFullscreen,
            Command::ViewSidebar => Text::ViewSidebar,
            Command::AppUpdate => Text::AppUpdate,
            Command::TabRename => Text::TabRename,
            Command::TabColor => Text::TabColor,
            Command::TabDuplicate => Text::TabDuplicate,
            Command::TabCloseOthers => Text::TabCloseOthers,
            Command::TabCloseRight => Text::TabCloseRight,
            Command::ViewZoomIn => Text::ViewZoomIn,
            Command::ViewZoomOut => Text::ViewZoomOut,
            Command::ViewZoomReset => Text::ViewZoomReset,
            Command::AppPalette => Text::AppPalette,
            Command::AppNewWindow => Text::AppNewWindow,
            Command::AppSettings => Text::AppSettings,
            Command::AppOpenSettingsFile => Text::AppOpenSettingsFile,
            Command::AppReloadSettings => Text::AppReloadSettings,
            Command::AppImportSchemes => Text::AppImportSchemes,
            Command::AppImportItermColors => Text::AppImportItermColors,
            Command::TabNewWorktree => Text::TabNewWorktree,
            Command::AppSaveLayout => Text::AppSaveLayout,
            Command::AppearanceBackdropNext => Text::AppearanceBackdropNext,
            Command::AppearanceOpacityUp => Text::AppearanceOpacityUp,
            Command::AppearanceOpacityDown => Text::AppearanceOpacityDown,
        }
    }

    pub fn label(self, lang: Lang) -> String {
        match self {
            Command::TabGoto(number) => self.text().format(lang, &[&number]),
            _ => self.text().get(lang).to_owned(),
        }
    }

    /// Shortcuts in effect until the user's `keybindings` replace them, in the form `Shortcut::parse` reads.
    ///
    /// The table keeps clear of what other layers claim first: no Ctrl+Alt chord on a printable key (that
    /// is AltGr on many layouts), no Alt chord on a bare letter or digit (readline and emacs Meta keys), and
    /// no command whose only binding is punctuation or Space, which an active IME swallows (the two zoom
    /// commands are the exception: they are also reachable from the palette).
    pub fn default_shortcuts(self) -> &'static [&'static str] {
        match self {
            Command::TabNew => &["Ctrl+Shift+T"],
            Command::TabNext => &["Ctrl+Tab"],
            Command::TabPrevious => &["Ctrl+Shift+Tab"],
            Command::TabGoto(1) => &["Ctrl+Shift+1"],
            Command::TabGoto(2) => &["Ctrl+Shift+2"],
            Command::TabGoto(3) => &["Ctrl+Shift+3"],
            Command::TabGoto(4) => &["Ctrl+Shift+4"],
            Command::TabGoto(5) => &["Ctrl+Shift+5"],
            Command::TabGoto(6) => &["Ctrl+Shift+6"],
            Command::TabGoto(7) => &["Ctrl+Shift+7"],
            Command::TabGoto(8) => &["Ctrl+Shift+8"],
            Command::TabGoto(9) => &["Ctrl+Shift+9"],
            Command::TabSelectLeft => &["Ctrl+PageUp"],
            Command::TabSelectRight => &["Ctrl+PageDown"],
            Command::TabNextNeedsResponse => &["Ctrl+Shift+U"],
            Command::TabProfiles => &["Ctrl+Shift+N"],
            Command::PaneSplitRight => &["Alt+Shift+D", "Alt+Shift+="],
            Command::PaneSplitDown => &["Alt+Shift+S", "Alt+Shift+-"],
            Command::PaneClose => &["Ctrl+Shift+W"],
            Command::PaneZoom => &["Ctrl+Shift+Z"],
            Command::PaneFocus(Direction::Left) => &["Alt+Left"],
            Command::PaneFocus(Direction::Right) => &["Alt+Right"],
            Command::PaneFocus(Direction::Up) => &["Alt+Up"],
            Command::PaneFocus(Direction::Down) => &["Alt+Down"],
            Command::PaneResize(Direction::Left) => &["Alt+Shift+Left"],
            Command::PaneResize(Direction::Right) => &["Alt+Shift+Right"],
            Command::PaneResize(Direction::Up) => &["Alt+Shift+Up"],
            Command::PaneResize(Direction::Down) => &["Alt+Shift+Down"],
            Command::PaneSwap(_) | Command::PaneToTab | Command::PaneBroadcast | Command::EditExport | Command::EditCopyFormatted | Command::EditSelectOutput | Command::EditCopyOutput => &[],
            Command::EditMarkMode => &["Ctrl+Shift+M"],
            Command::EditHints => &["Ctrl+Shift+E"],
            Command::ViewAlwaysOnTop | Command::ViewFocusMode | Command::ViewQuake => &[],
            Command::EditSuggestions => &["Ctrl+Shift+H"],
            Command::EditSmartCopy => &["Ctrl+C"],
            Command::EditCopy => &["Ctrl+Shift+C", "Ctrl+Insert"],
            Command::EditPaste => &["Ctrl+V", "Ctrl+Shift+V", "Shift+Insert"],
            Command::EditSelectAll => &["Ctrl+Shift+A"],
            Command::EditFind => &["Ctrl+Shift+F"],
            Command::ScrollPageUp => &["Ctrl+Shift+PageUp"],
            Command::ScrollPageDown => &["Ctrl+Shift+PageDown"],
            Command::ScrollTop => &["Ctrl+Shift+Home"],
            Command::ScrollBottom => &["Ctrl+Shift+End"],
            Command::PromptPrevious => &["Ctrl+Alt+Up"],
            Command::PromptNext => &["Ctrl+Alt+Down"],
            Command::ViewFullscreen => &["F11"],
            Command::ViewSidebar => &["Ctrl+Shift+B"],
            Command::AppUpdate => &[],
            Command::TabRename | Command::TabColor | Command::TabDuplicate | Command::TabCloseOthers | Command::TabCloseRight => &[],
            Command::ViewZoomIn => &["Ctrl+="],
            Command::ViewZoomOut => &["Ctrl+-"],
            Command::ViewZoomReset => &["Ctrl+0"],
            Command::AppPalette => &["Ctrl+Shift+P"],
            Command::AppSettings => &["Ctrl+,", "Ctrl+Shift+O"],
            Command::TabClose
            | Command::AppNewWindow
            | Command::TabGoto(_)
            | Command::TabMoveLeft
            | Command::TabMoveRight
            | Command::PaneRestart
            | Command::EditClear
            | Command::AppOpenSettingsFile
            | Command::AppReloadSettings
            | Command::AppImportSchemes
            | Command::AppImportItermColors
            | Command::TabNewWorktree
            | Command::AppSaveLayout
            | Command::AppearanceBackdropNext
            | Command::AppearanceOpacityUp
            | Command::AppearanceOpacityDown => &[],
        }
    }

    /// Acts again while its key is held, as moving and scrolling do. Everything that opens, closes or toggles acts once a
    /// press: a held Ctrl+Shift+T must not open a tab a repeat.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Command::TabNext
                | Command::TabPrevious
                | Command::TabSelectLeft
                | Command::TabSelectRight
                | Command::TabMoveLeft
                | Command::TabMoveRight
                | Command::PaneFocus(_)
                | Command::PaneResize(_)
                | Command::ScrollPageUp
                | Command::ScrollPageDown
                | Command::PromptPrevious
                | Command::PromptNext
                | Command::ViewZoomIn
                | Command::ViewZoomOut
        )
    }

    /// Listed in the command palette. Hidden are the duplicate of Copy, the nine tab numbers and the profile
    /// menu (the palette lists each profile as its own entry).
    pub fn in_palette(self) -> bool {
        !matches!(
            self,
            Command::EditSmartCopy | Command::TabGoto(_) | Command::TabProfiles
        )
    }

    /// The tab (zero-based) that `TabGoto(number)` selects among `count` tabs: 1 to 8 are the tabs of that
    /// number, 9 is the last tab whatever their count. `None` when there is no such tab.
    pub fn tab_goto_index(number: u8, count: usize) -> Option<usize> {
        match number {
            9 => count.checked_sub(1),
            1..=8 => Some(usize::from(number) - 1).filter(|index| *index < count),
            _ => None,
        }
    }

    /// The tab (zero-based) to the left or to the right of `index` among `count` tabs, wrapping at the ends. `None`
    /// when there is no other tab.
    pub fn tab_neighbour(index: usize, count: usize, right: bool) -> Option<usize> {
        (count > 1).then(|| {
            if right {
                (index + 1) % count
            } else {
                (index + count - 1) % count
            }
        })
    }

    /// The `TabGoto` number whose shortcut selects tab `index` of `count`, for showing it next to the tab.
    pub fn tab_goto_number(index: usize, count: usize) -> Option<u8> {
        if index < 8 {
            u8::try_from(index + 1).ok()
        } else {
            (index + 1 == count).then_some(9)
        }
    }

    /// Acts on the window or the workspace, so it also runs while focus is on the tab strip or in an
    /// overlay. The others read or change the focused terminal itself (selection, clipboard, scrollback).
    pub fn works_outside_terminal(self) -> bool {
        !matches!(
            self,
            Command::EditCopy
                | Command::EditSmartCopy
                | Command::EditPaste
                | Command::EditSelectAll
                | Command::EditClear
                | Command::ScrollPageUp
                | Command::ScrollPageDown
                | Command::ScrollTop
                | Command::ScrollBottom
                | Command::PromptPrevious
                | Command::PromptNext
        )
    }
}

/// The default shortcuts of every command, the `defaults` argument of `keymap::build_keymap`.
pub fn default_bindings() -> impl Iterator<Item = (Command, &'static [&'static str])> {
    Command::ALL
        .iter()
        .map(|command| (*command, command.default_shortcuts()))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::keymap::{Key, KeyClass, Modifiers, Shortcut, build_keymap};

    #[test]
    fn moving_and_scrolling_repeat_while_held_and_opening_closing_or_toggling_does_not() {
        for command in [Command::PaneResize(Direction::Left), Command::TabSelectRight, Command::ScrollPageUp, Command::ViewZoomIn] {
            assert!(command.repeats(), "{}", command.id());
        }
        for command in [Command::TabNew, Command::TabClose, Command::PaneClose, Command::PaneSplitRight, Command::EditPaste, Command::AppPalette, Command::ViewFullscreen, Command::PaneZoom, Command::ViewZoomReset] {
            assert!(!command.repeats(), "{} acts once a press", command.id());
        }
    }

    const DIRECTIONS: [Direction; 4] = [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ];

    /// One value per variant. The match has no wildcard, so adding a variant fails to compile here until
    /// `ALL` and this table are updated together; `every_variant_is_listed_in_all` then checks `ALL`.
    fn variant_index(command: Command) -> usize {
        match command {
            Command::TabNew => 0,
            Command::TabClose => 1,
            Command::TabNext => 2,
            Command::TabPrevious => 3,
            Command::TabGoto(_) => 4,
            Command::TabMoveLeft => 5,
            Command::TabMoveRight => 6,
            Command::TabProfiles => 7,
            Command::PaneSplitRight => 8,
            Command::PaneSplitDown => 9,
            Command::PaneClose => 10,
            Command::PaneZoom => 11,
            Command::PaneRestart => 12,
            Command::PaneFocus(_) => 13,
            Command::PaneResize(_) => 14,
            Command::EditCopy => 15,
            Command::EditSmartCopy => 16,
            Command::EditPaste => 17,
            Command::EditSelectAll => 18,
            Command::EditFind => 19,
            Command::EditClear => 20,
            Command::ScrollPageUp => 21,
            Command::ScrollPageDown => 22,
            Command::ScrollTop => 23,
            Command::ScrollBottom => 24,
            Command::PromptPrevious => 25,
            Command::PromptNext => 26,
            Command::ViewFullscreen => 27,
            Command::ViewZoomIn => 28,
            Command::ViewZoomOut => 29,
            Command::ViewZoomReset => 30,
            Command::AppPalette => 31,
            Command::AppSettings => 32,
            Command::AppOpenSettingsFile => 33,
            Command::AppReloadSettings => 34,
            Command::AppearanceBackdropNext => 35,
            Command::AppearanceOpacityUp => 36,
            Command::AppearanceOpacityDown => 37,
            Command::TabNextNeedsResponse => 38,
            Command::TabSelectLeft => 39,
            Command::TabSelectRight => 40,
            Command::AppNewWindow => 41,
            Command::ViewSidebar => 42,
            Command::AppUpdate => 43,
            Command::TabRename => 44,
            Command::TabColor => 45,
            Command::TabDuplicate => 46,
            Command::TabCloseOthers => 47,
            Command::TabCloseRight => 48,
            Command::PaneSwap(_) => 49,
            Command::PaneToTab => 50,
            Command::PaneBroadcast => 51,
            Command::EditExport => 52,
            Command::EditMarkMode => 53,
            Command::EditCopyFormatted => 54,
            Command::ViewAlwaysOnTop => 55,
            Command::ViewFocusMode => 56,
            Command::ViewQuake => 57,
            Command::EditSuggestions => 58,
            Command::EditSelectOutput => 59,
            Command::EditCopyOutput => 60,
            Command::EditHints => 61,
            Command::AppImportSchemes => 62,
            Command::AppImportItermColors => 63,
            Command::TabNewWorktree => 64,
            Command::AppSaveLayout => 65,
        }
    }
    const VARIANTS: usize = 66;

    fn shortcuts_of(command: Command) -> Vec<Shortcut> {
        command
            .default_shortcuts()
            .iter()
            .map(|text| {
                Shortcut::parse(text)
                    .unwrap_or_else(|| panic!("{}: {text} should parse", command.id()))
            })
            .collect()
    }

    #[test]
    fn every_variant_is_listed_in_all() {
        let listed: BTreeSet<usize> = Command::ALL
            .iter()
            .map(|command| variant_index(*command))
            .collect();
        assert_eq!(listed, (0..VARIANTS).collect::<BTreeSet<_>>());
        let numbers: Vec<u8> = Command::ALL
            .iter()
            .filter_map(|c| {
                if let Command::TabGoto(n) = c {
                    Some(*n)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(numbers, (1..=9).collect::<Vec<u8>>());
        for kind in [
            Command::PaneFocus as fn(Direction) -> Command,
            Command::PaneResize,
            Command::PaneSwap,
        ] {
            let directions: Vec<Direction> = DIRECTIONS
                .iter()
                .copied()
                .filter(|d| Command::ALL.contains(&kind(*d)))
                .collect();
            assert_eq!(directions, DIRECTIONS);
        }
    }

    #[test]
    fn ids_are_unique_and_read_back() {
        let mut seen = BTreeSet::new();
        for command in Command::ALL {
            assert!(seen.insert(command.id()), "duplicate id {}", command.id());
            assert_eq!(Command::from_id(command.id()), Some(*command));
        }
        assert_eq!(Command::from_id("tab.goto"), None);
        assert_eq!(Command::from_id("paste"), None);
        assert_eq!(Command::from_id(""), None);
        assert_eq!(Command::TabGoto(3).id(), "tab.goto3");
        assert_eq!(Command::PaneFocus(Direction::Up).id(), "pane.focusUp");
        assert_eq!(
            Command::AppearanceBackdropNext.id(),
            "appearance.backdrop.next"
        );
    }

    #[test]
    fn every_command_has_a_label_in_both_languages() {
        for command in Command::ALL {
            for lang in Lang::ALL {
                let label = command.label(lang);
                assert!(
                    !label.trim().is_empty() && !label.contains('{'),
                    "{} in {lang:?}: {label:?}",
                    command.id()
                );
            }
        }
        assert_eq!(Command::TabGoto(4).label(Lang::ZhTw), "切換到分頁 4");
        assert_eq!(Command::TabGoto(4).label(Lang::EnUs), "Go to tab 4");
        assert_eq!(Command::TabNew.label(Lang::ZhTw), "新增分頁");
    }

    #[test]
    fn palette_labels_are_unique_so_two_entries_never_read_the_same() {
        for lang in Lang::ALL {
            let mut seen = BTreeMap::new();
            for command in Command::ALL.iter().filter(|c| c.in_palette()) {
                if let Some(other) = seen.insert(command.label(lang), command.id()) {
                    panic!(
                        "{lang:?}: {} and {} are both {:?}",
                        other,
                        command.id(),
                        command.label(lang)
                    );
                }
            }
        }
    }

    #[test]
    fn default_shortcuts_parse_and_no_two_commands_share_one() {
        let mut owners: BTreeMap<String, &str> = BTreeMap::new();
        for command in Command::ALL {
            for shortcut in shortcuts_of(*command) {
                if let Some(other) = owners.insert(shortcut.to_string(), command.id()) {
                    panic!("{} is bound to both {other} and {}", shortcut, command.id());
                }
            }
        }
    }

    #[test]
    fn default_shortcuts_are_written_in_the_canonical_form() {
        for command in Command::ALL {
            for (text, shortcut) in command
                .default_shortcuts()
                .iter()
                .zip(shortcuts_of(*command))
            {
                assert_eq!(*text, shortcut.to_string(), "{}", command.id());
            }
        }
    }

    /// Why a default shortcut would collide with something that sees the key first.
    fn rule_violations(shortcut: &Shortcut) -> Vec<&'static str> {
        let (mods, key) = (shortcut.mods, shortcut.key);
        let mut found = Vec::new();
        if mods.ctrl && mods.alt && key.types_text() {
            found.push("Ctrl+Alt on a printable key is AltGr on many layouts");
        }
        if mods.alt
            && !mods.ctrl
            && !mods.shift
            && matches!(key.class(), KeyClass::Letter | KeyClass::Digit)
        {
            found.push("Alt on a bare letter or digit is a readline/emacs Meta key");
        }
        if mods.meta {
            found.push("the Windows key belongs to Windows");
        }
        if key.types_text() && !mods.ctrl && !mods.alt {
            found.push("without Ctrl or Alt the key is typing");
        }
        found
    }

    #[test]
    fn default_shortcuts_avoid_altgr_meta_keys_and_typing() {
        for command in Command::ALL {
            for shortcut in shortcuts_of(*command) {
                assert_eq!(
                    rule_violations(&shortcut),
                    Vec::<&str>::new(),
                    "{}: {shortcut}",
                    command.id()
                );
            }
        }
    }

    #[test]
    fn no_command_depends_only_on_chords_an_ime_swallows() {
        // Zoom is the exception: it is also in the palette, and Ctrl+0 resets it.
        let punctuation_only_ok = [Command::ViewZoomIn, Command::ViewZoomOut];
        for command in Command::ALL {
            let shortcuts = shortcuts_of(*command);
            if !shortcuts.is_empty() && !punctuation_only_ok.contains(command) {
                assert!(
                    shortcuts.iter().any(|s| !s.key.is_swallowed_by_ime()),
                    "{}: every binding is punctuation or Space",
                    command.id()
                );
            }
        }
    }

    #[test]
    fn the_rules_flag_the_chords_they_are_written_for() {
        for (text, expected) in [
            ("Ctrl+Alt+1", 1),
            ("Ctrl+Alt+Shift+Q", 1),
            ("Alt+O", 1),
            ("Alt+5", 1),
            ("Shift+A", 1),
            ("A", 1),
            ("Ctrl+Space", 0),
            ("Win+L", 2),
            // Allowed: Alt+Shift+letter, Ctrl+Alt on a named key, Ctrl+Shift+digit, Alt+arrow.
            ("Alt+Shift+D", 0),
            ("Ctrl+Alt+Up", 0),
            ("Ctrl+Shift+1", 0),
            ("Alt+Left", 0),
            ("F11", 0),
        ] {
            let found = rule_violations(&Shortcut::parse(text).unwrap());
            assert_eq!(found.len(), expected, "{text}: {found:?}");
        }
    }

    #[test]
    fn core_commands_have_a_binding_that_survives_an_ime() {
        for command in [
            Command::TabNew,
            Command::PaneSplitRight,
            Command::PaneSplitDown,
            Command::AppSettings,
            Command::AppPalette,
            Command::PaneClose,
        ] {
            let shortcuts = shortcuts_of(command);
            assert!(
                shortcuts.iter().any(|s| !s.key.is_swallowed_by_ime()),
                "{}",
                command.id()
            );
        }
        // Settings keeps the familiar Ctrl+, next to the IME-proof one.
        assert!(Command::AppSettings.default_shortcuts().contains(&"Ctrl+,"));
    }

    #[test]
    fn the_default_keymap_resolves_the_documented_chords() {
        let keymap = build_keymap(default_bindings(), &BTreeMap::new());
        assert!(keymap.problems.is_empty());
        let keymap = keymap.keymap;
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(keymap.find(ctrl_shift, "KeyT"), Some(Command::TabNew));
        assert_eq!(keymap.find(ctrl_shift, "Digit3"), Some(Command::TabGoto(3)));
        assert_eq!(
            keymap.find(ctrl_shift, "KeyU"),
            Some(Command::TabNextNeedsResponse)
        );
        assert_eq!(keymap.find(ctrl, "KeyC"), Some(Command::EditSmartCopy));
        assert_eq!(keymap.find(ctrl_shift, "KeyC"), Some(Command::EditCopy));
        assert_eq!(keymap.find(ctrl, "Comma"), Some(Command::AppSettings));
        assert_eq!(
            keymap.find(Modifiers::default(), "F11"),
            Some(Command::ViewFullscreen)
        );
        assert_eq!(
            keymap.find(
                Modifiers {
                    alt: true,
                    ..Default::default()
                },
                "ArrowLeft"
            ),
            Some(Command::PaneFocus(Direction::Left))
        );
        assert_eq!(
            keymap.find(Modifiers::default(), "KeyT"),
            None,
            "plain typing is never a shortcut"
        );
    }

    #[test]
    fn every_default_key_exists_in_the_key_table() {
        for command in Command::ALL {
            for shortcut in shortcuts_of(*command) {
                assert_eq!(Key::from_code(shortcut.key.code()), Some(shortcut.key));
            }
        }
    }

    #[test]
    fn only_commands_that_act_on_the_terminal_itself_need_terminal_focus() {
        for command in [
            Command::AppPalette,
            Command::AppSettings,
            Command::ViewFullscreen,
        Command::ViewSidebar,
            Command::EditFind,
            Command::TabNew,
            Command::TabGoto(2),
            Command::PaneSplitRight,
        ] {
            assert!(command.works_outside_terminal(), "{}", command.id());
        }
        for command in [
            Command::EditCopy,
            Command::EditSmartCopy,
            Command::EditPaste,
            Command::EditSelectAll,
            Command::EditClear,
            Command::ScrollTop,
            Command::PromptNext,
        ] {
            assert!(!command.works_outside_terminal(), "{}", command.id());
        }
    }

    #[test]
    fn the_palette_hides_the_duplicates_and_nothing_else() {
        let hidden: Vec<&str> = Command::ALL
            .iter()
            .filter(|c| !c.in_palette())
            .map(|c| c.id())
            .collect();
        let mut expected = vec!["edit.smartCopy", "tab.profiles"];
        expected.extend((1..=9).map(|n| Command::TabGoto(n).id()));
        expected.sort_unstable();
        let mut hidden = hidden;
        hidden.sort_unstable();
        assert_eq!(hidden, expected);
    }

    #[test]
    fn the_numbered_tab_commands_select_the_tab_of_that_number_and_nine_the_last() {
        assert_eq!(Command::tab_goto_index(1, 3), Some(0));
        assert_eq!(Command::tab_goto_index(3, 3), Some(2));
        assert_eq!(Command::tab_goto_index(4, 3), None);
        assert_eq!(Command::tab_goto_index(8, 12), Some(7));
        assert_eq!(Command::tab_goto_index(9, 12), Some(11));
        assert_eq!(Command::tab_goto_index(9, 2), Some(1));
        assert_eq!(Command::tab_goto_index(9, 0), None);
        assert_eq!(Command::tab_goto_index(0, 5), None);
        assert_eq!(Command::tab_goto_index(10, 12), None);
    }

    #[test]
    fn the_tab_to_the_left_or_the_right_wraps_at_the_ends_and_needs_another_tab() {
        assert_eq!(Command::tab_neighbour(0, 3, true), Some(1));
        assert_eq!(Command::tab_neighbour(2, 3, true), Some(0));
        assert_eq!(Command::tab_neighbour(0, 3, false), Some(2));
        assert_eq!(Command::tab_neighbour(2, 3, false), Some(1));
        assert_eq!(Command::tab_neighbour(0, 2, true), Some(1));
        assert_eq!(Command::tab_neighbour(1, 2, true), Some(0));
        assert_eq!(Command::tab_neighbour(0, 1, true), None);
        assert_eq!(Command::tab_neighbour(0, 0, false), None);
    }

    #[test]
    fn the_positional_tab_keys_are_free_of_the_commands_and_of_what_programs_need() {
        use crate::keybindings::{clash, needed_by_programs};
        let keymap = build_keymap(default_bindings(), &BTreeMap::new()).keymap;
        for (command, text) in [
            (Command::TabSelectLeft, "Ctrl+PageUp"),
            (Command::TabSelectRight, "Ctrl+PageDown"),
        ] {
            assert_eq!(command.default_shortcuts(), [text]);
            let chord = Shortcut::parse(text).expect("a shortcut");
            assert!(!needed_by_programs(chord), "{text}");
            assert_eq!(clash(&keymap, chord, command), None, "{text}");
            assert_eq!(
                keymap.find(chord.mods, chord.key.code()),
                Some(command),
                "{text}"
            );
        }
        // Ctrl+Tab and Ctrl+Shift+Tab stay the order of use, and the scrolling of Ctrl+Shift+PageUp is not touched.
        assert_eq!(Command::TabNext.default_shortcuts(), ["Ctrl+Tab"]);
        assert_eq!(
            Command::ScrollPageUp.default_shortcuts(),
            ["Ctrl+Shift+PageUp"]
        );
    }

    #[test]
    fn a_tab_shows_the_shortcut_that_selects_it() {
        for count in 1..=14 {
            for index in 0..count {
                if let Some(number) = Command::tab_goto_number(index, count) {
                    assert_eq!(
                        Command::tab_goto_index(number, count),
                        Some(index),
                        "{index} of {count}"
                    );
                }
            }
            // The first eight tabs and the last one have a shortcut; the ones in between have none.
            let with_shortcut = (0..count)
                .filter(|i| Command::tab_goto_number(*i, count).is_some())
                .count();
            assert_eq!(
                with_shortcut,
                count.min(8) + usize::from(count > 8),
                "{count} tabs"
            );
        }
    }

    #[test]
    fn the_new_appearance_commands_exist_for_the_blur_and_opacity_tools() {
        for id in [
            "appearance.backdrop.next",
            "appearance.opacity.up",
            "appearance.opacity.down",
            "tab.profiles",
            "tab.moveLeft",
            "tab.moveRight",
        ] {
            assert!(Command::from_id(id).is_some(), "{id}");
        }
    }
}
