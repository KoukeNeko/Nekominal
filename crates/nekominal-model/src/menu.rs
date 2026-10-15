//! The "…" menu of the top bar: an ordered view of the `Command` table that is a superset of the toolbar. What it
//! holds and in which order never depends on the width of the window, so an action the bar folds away is always in
//! the same place. Labels are `Command::label` and the shortcut hints come from the live keymap (the shell reads
//! both when the menu opens); only two rows are not commands: the backdrop and the opacity of the background.

use crate::commands::Command;

/// One row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuRow {
    Command(Command),
    /// A submenu of the four backdrops, one of them chosen. Not a command.
    Backdrop,
    /// Opens the quick appearance flyout (the opacity of the background and the backdrop). Not a command.
    Opacity,
    /// Between the groups: the objects, find, appearance and view, settings.
    Separator,
}

/// The menu, top to bottom.
pub const APP_MENU: &[MenuRow] = &[
    MenuRow::Command(Command::TabNew),
    MenuRow::Command(Command::PaneSplitRight),
    MenuRow::Command(Command::PaneSplitDown),
    MenuRow::Command(Command::PaneZoom),
    MenuRow::Command(Command::PaneClose),
    MenuRow::Separator,
    MenuRow::Command(Command::EditFind),
    MenuRow::Command(Command::AppPalette),
    MenuRow::Separator,
    MenuRow::Backdrop,
    MenuRow::Opacity,
    MenuRow::Command(Command::ViewFullscreen),
    MenuRow::Separator,
    MenuRow::Command(Command::AppSettings),
];

/// The commands of the right-hand group of the bar, in order. A narrow window folds them into the menu, the
/// split pair first (together) and then the palette; the menu button itself is always there. The new-tab button
/// and the profile button on the left never fold, and the new-tab button is in the menu as well.
pub const TOOLBAR: &[Command] = &[
    Command::PaneSplitRight,
    Command::PaneSplitDown,
    Command::AppPalette,
];

/// The commands of the menu in order.
pub fn commands() -> impl Iterator<Item = Command> {
    APP_MENU.iter().filter_map(|row| match row {
        MenuRow::Command(command) => Some(*command),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_is_the_documented_order_of_commands_and_two_rows_that_are_not() {
        let ids: Vec<&str> = APP_MENU
            .iter()
            .map(|row| match row {
                MenuRow::Command(command) => command.id(),
                MenuRow::Backdrop => "<backdrop>",
                MenuRow::Opacity => "<opacity>",
                MenuRow::Separator => "-",
            })
            .collect();
        assert_eq!(
            ids,
            [
                "tab.new",
                "pane.splitRight",
                "pane.splitDown",
                "pane.zoom",
                "pane.close",
                "-",
                "edit.find",
                "app.palette",
                "-",
                "<backdrop>",
                "<opacity>",
                "view.fullscreen",
                "-",
                "app.settings",
            ]
        );
    }

    #[test]
    fn groups_are_set_apart_by_one_separator_and_the_menu_neither_starts_nor_ends_with_one() {
        assert_ne!(APP_MENU.first(), Some(&MenuRow::Separator));
        assert_ne!(APP_MENU.last(), Some(&MenuRow::Separator));
        assert!(
            APP_MENU
                .windows(2)
                .all(|pair| pair != [MenuRow::Separator, MenuRow::Separator])
        );
    }

    #[test]
    fn no_command_is_listed_twice_and_each_row_that_is_not_a_command_once() {
        let mut seen = Vec::new();
        for command in commands() {
            assert!(!seen.contains(&command), "{} is listed twice", command.id());
            seen.push(command);
        }
        for row in [MenuRow::Backdrop, MenuRow::Opacity] {
            assert_eq!(
                APP_MENU.iter().filter(|other| **other == row).count(),
                1,
                "{row:?}"
            );
        }
    }

    /// An action the bar folds away must stay reachable: whatever the toolbar can show is in the menu, and whatever
    /// the menu offers is in the palette. So no width takes an action away.
    #[test]
    fn the_toolbar_is_a_subset_of_the_menu_and_the_menu_of_the_palette() {
        let in_menu: Vec<Command> = commands().collect();
        for command in TOOLBAR.iter().chain([&Command::TabNew]) {
            assert!(
                in_menu.contains(command),
                "{} is on the bar but not in the menu",
                command.id()
            );
        }
        for command in &in_menu {
            assert!(
                command.in_palette(),
                "{} is in the menu but not in the palette",
                command.id()
            );
        }
    }

    #[test]
    fn the_toolbar_is_the_split_pair_then_the_palette() {
        assert_eq!(
            TOOLBAR,
            [
                Command::PaneSplitRight,
                Command::PaneSplitDown,
                Command::AppPalette
            ]
        );
    }

    #[test]
    fn find_and_settings_are_two_steps_away_without_an_icon_of_their_own() {
        // The bar no longer has a find or a settings button: both are rows of the menu (button, then row), and keep
        // their shortcuts.
        let in_menu: Vec<Command> = commands().collect();
        for command in [Command::EditFind, Command::AppSettings] {
            assert!(
                in_menu.contains(&command) && !TOOLBAR.contains(&command),
                "{}",
                command.id()
            );
            assert!(
                !command.default_shortcuts().is_empty(),
                "{} keeps a shortcut",
                command.id()
            );
        }
    }
}
