//! Where things are, in DIP: the title bar of 32 with its tabs or the button of the sidebar, the sidebar of the tabs when
//! the tabs stand there (a list of two lines a tab, or a column of icons), and the area that belongs to the terminal. The
//! caption buttons (minimize, maximize, close) are at the top right, as tall as the bar. Pure: the window code gives the
//! size, the tabs and how much room the right of the bar needs (the waiting count and the numbers of the agent), and gets
//! rectangles and hit tests back.

use nekominal_model::settings::TabLayout;
use nekominal_ui::Rect;

use crate::logic;

/// Height of the title bar: the tabs (on top), the buttons and the caption buttons live in it.
pub const TITLE_HEIGHT: f32 = 40.0;
/// Space left of the first tab, and of the first button with the tabs in the sidebar.
pub const STRIP_LEFT: f32 = 6.0;
/// The slot of the shield of an elevated window, before the tabs.
pub const SHIELD: f32 = 28.0;
/// Space between two tabs of the strip: words with a line under the selected one need some air between them.
pub const CHIP_GAP: f32 = 4.0;
/// Width of the three caption buttons together.
pub const CAPTION: f32 = 3.0 * 40.0;
/// Width of the buttons of the bar (the sidebar, new tab, profiles, menu); they are as tall as the bar.
pub const BUTTON: f32 = 32.0;
/// The part of the title bar that stays free for dragging the window.
pub const DRAG_MIN: f32 = 48.0;
/// A tab at least this wide has room for a close button.
pub const CLOSE_FROM: f32 = 96.0;
const CLOSE_SIZE: f32 = 18.0;
/// Width of the sidebar as a list and as icons.
pub const SIDE_LIST: f32 = 220.0;
pub const SIDE_ICONS: f32 = 52.0;
/// A row of the list: the program and its state on a line, the folder and the branch under it.
pub const LIST_ROW: f32 = 44.0;
/// The heading of a group of the list (when it is grouped by state).
pub const LIST_HEADING: f32 = 26.0;
/// An icon of the column, and the space a row of the column takes.
pub const ICON_CELL: f32 = 36.0;
pub const ICON_ROW: f32 = 44.0;
const SIDE_PAD: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    None,
    /// Empty title bar: the window can be dragged by it.
    Drag,
    Chip(usize),
    ChipClose(usize),
    NewTab,
    Profiles,
    /// The position plate: opens the list of all tabs.
    Plate,
    Menu,
    /// Hides or shows the sidebar.
    Sidebar,
    /// The count of the tabs that wait for the user: goes to the next of them.
    Waiting,
    /// What of the sidebar is not a row.
    Side,
    Minimize,
    Maximize,
    Close,
    Pane,
}

/// How the tabs are shown: where (`TabLayout`), whether the sidebar is open, and whether the shield of an elevated window
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub layout: TabLayout,
    pub sidebar_open: bool,
    pub admin: bool,
}

impl Mode {
    pub const TOP: Mode = Mode { layout: TabLayout::Top, sidebar_open: true, admin: false };

    /// The width of the sidebar on show; 0 with the tabs on top or the sidebar hidden.
    pub fn side_width(self) -> f32 {
        match (self.layout, self.sidebar_open) {
            (TabLayout::List, true) => SIDE_LIST,
            (TabLayout::Icons, true) => SIDE_ICONS,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub size: (f32, f32),
    pub mode: Mode,
    /// One rectangle per tab, where it is drawn: a chip of the strip (on top) or a row of the sidebar. Only the tabs from
    /// `first` on for `shown` of them are on show; the others lie outside and are neither drawn nor hit.
    pub chips: Vec<Rect>,
    pub first: usize,
    pub shown: usize,
    /// The headings of the groups of the list, with the group each names (an index into the groups given).
    pub headings: Vec<(Rect, usize)>,
    /// The shield of a window that runs as administrator, at the left end of the bar before everything else.
    pub shield: Option<Rect>,
    /// Where the tabs (or the buttons of the sidebar) start: after the shield, when there is one.
    pub left: f32,
    /// Hides and shows the sidebar: only with the tabs in a sidebar.
    pub sidebar_button: Option<Rect>,
    pub new_tab: Rect,
    /// Opens the list of shells (profiles) a new tab can start.
    pub profiles: Rect,
    pub menu: Rect,
    /// Minimize, maximize or restore, and close: the whole height of the title bar.
    pub caption_buttons: [Rect; 3],
    /// The sidebar, when it is on show.
    pub sidebar: Option<Rect>,
    /// Where the title of the selected tab goes with the tabs in the sidebar, and where the waiting count and the numbers
    /// of the agent go at the right of the bar.
    pub title: Rect,
    pub right: Rect,
    pub pane: Rect,
}

impl Layout {
    /// `caption` is the width of the caption buttons at the right edge; `right` the room the waiting count and the numbers
    /// of the agent ask for, left of the menu. `groups` are the tabs in the order the sidebar lists them, by group (one
    /// group without a heading when the list is not grouped); the strip on top always has the tabs in their own order.
    /// When the chips at their narrowest do not all fit, the strip shows the ones that do, scrolled so that tab `selected`
    /// is the last of them or earlier.
    pub fn new(width: f32, height: f32, tabs: usize, caption: f32, selected: usize, mode: Mode, right: f32, groups: &[Vec<usize>]) -> Layout {
        let menu = Rect::new((width - caption - BUTTON).max(0.0), 0.0, BUTTON, TITLE_HEIGHT);
        let caption_buttons = [0.0, 1.0, 2.0].map(|i| Rect::new(width - caption + i * caption / 3.0, 0.0, caption / 3.0, TITLE_HEIGHT));
        let shield = mode.admin.then(|| Rect::new(STRIP_LEFT, 0.0, SHIELD, TITLE_HEIGHT));
        let left = STRIP_LEFT + if mode.admin { SHIELD } else { 0.0 };
        let right_room = right.clamp(0.0, (menu.x - left - 3.0 * BUTTON).max(0.0));
        let right = Rect::new(menu.x - right_room, 0.0, right_room, TITLE_HEIGHT);
        let side = mode.side_width();
        let pane = Rect::new(side, TITLE_HEIGHT, (width - side).max(0.0), (height - TITLE_HEIGHT).max(0.0));
        if mode.layout.is_side() {
            let button = |i: f32| Rect::new(left + i * BUTTON, 0.0, BUTTON, TITLE_HEIGHT);
            let (sidebar_button, new_tab, profiles) = (button(0.0), button(1.0), button(2.0));
            let title_x = profiles.x + BUTTON + 8.0;
            let title = Rect::new(title_x, 0.0, (right.x - DRAG_MIN / 2.0 - title_x).max(0.0), TITLE_HEIGHT);
            let sidebar = (side > 0.0).then(|| Rect::new(0.0, TITLE_HEIGHT, side, (height - TITLE_HEIGHT).max(0.0)));
            let mut chips = vec![Rect::new(0.0, 0.0, 0.0, 0.0); tabs];
            let mut headings = Vec::new();
            let mut y = TITLE_HEIGHT + SIDE_PAD;
            let list = mode.layout == TabLayout::List;
            let named = list && groups.len() > 1;
            for (g, group) in groups.iter().enumerate() {
                if named && !group.is_empty() {
                    headings.push((Rect::new(SIDE_PAD, y, side - 2.0 * SIDE_PAD, LIST_HEADING), g));
                    y += LIST_HEADING;
                }
                for &i in group.iter().filter(|i| **i < tabs) {
                    chips[i] = if list {
                        Rect::new(SIDE_PAD, y, SIDE_LIST - 2.0 * SIDE_PAD, LIST_ROW)
                    } else {
                        Rect::new((SIDE_ICONS - ICON_CELL) / 2.0, y + (ICON_ROW - ICON_CELL) / 2.0, ICON_CELL, ICON_CELL)
                    };
                    y += if list { LIST_ROW + 2.0 } else { ICON_ROW };
                }
            }
            // shortcut: the sidebar does not scroll; rows past its bottom are not shown (about 14 rows at 720 DIP).
            let shown = if sidebar.is_some() { tabs } else { 0 };
            return Layout { size: (width, height), mode, chips, first: 0, shown, headings, shield, left, sidebar_button: Some(sidebar_button), new_tab, profiles, menu, caption_buttons, sidebar, title, right, pane };
        }
        let strip_end = right.x - DRAG_MIN;
        // The room for the chips: what is between the strip start and the two buttons after the strip.
        let available = (strip_end - left - 2.0 * BUTTON).max(0.0);
        let chip_width = logic::tab_width(tabs, f64::from(available), f64::from(CHIP_GAP)) as f32;
        // The epsilon: chips that fill the strip exactly divide to 5.999... as easily as to 6.
        let fits = (((available + CHIP_GAP) / (chip_width + CHIP_GAP) + 1e-3).floor() as usize).max(1);
        let shown = tabs.min(fits);
        let first = if tabs <= shown { 0 } else { (selected + 1).saturating_sub(shown).min(tabs - shown) };
        let chips: Vec<Rect> = (0..tabs)
            .map(|i| Rect::new(left + (i as f32 - first as f32) * (chip_width + CHIP_GAP), 0.0, chip_width, TITLE_HEIGHT))
            .collect();
        let after = left + shown as f32 * (chip_width + CHIP_GAP);
        let new_tab = Rect::new(after.min(strip_end - 2.0 * BUTTON).max(left), 0.0, BUTTON, TITLE_HEIGHT);
        let profiles = Rect::new(new_tab.x + BUTTON, 0.0, BUTTON, TITLE_HEIGHT);
        Layout { size: (width, height), mode, chips, first, shown, headings: Vec::new(), shield, left, sidebar_button: None, new_tab, profiles, menu, caption_buttons, sidebar: None, title: Rect::new(0.0, 0.0, 0.0, 0.0), right, pane }
    }

    /// Whether tab `i` is on show (in the strip, or in the sidebar while it is open and above its bottom).
    pub fn is_shown(&self, i: usize) -> bool {
        if let Some(sidebar) = self.sidebar {
            return self.chips.get(i).is_some_and(|row| row.w > 0.0 && row.y + row.h <= sidebar.y + sidebar.h);
        }
        !self.mode.layout.is_side() && (self.first..self.first + self.shown).contains(&i)
    }

    /// The close button inside a chip or a row of the list, `None` when it is too narrow for one (a chip, an icon).
    pub fn close_button(chip: Rect) -> Option<Rect> {
        (chip.w >= CLOSE_FROM)
            .then(|| Rect::new(chip.x + chip.w - 6.0 - CLOSE_SIZE, chip.y + (chip.h - CLOSE_SIZE) / 2.0, CLOSE_SIZE, CLOSE_SIZE))
    }

    pub fn hit(&self, x: f32, y: f32) -> Hit {
        for (hit, button) in [Hit::Minimize, Hit::Maximize, Hit::Close].into_iter().zip(self.caption_buttons) {
            if button.contains(x, y) {
                return hit;
            }
        }
        if self.sidebar_button.is_some_and(|b| b.contains(x, y)) {
            return Hit::Sidebar;
        }
        if self.new_tab.contains(x, y) {
            return Hit::NewTab;
        }
        if self.profiles.contains(x, y) {
            return Hit::Profiles;
        }
        if self.menu.contains(x, y) {
            return Hit::Menu;
        }
        for (i, chip) in self.chips.iter().enumerate().filter(|(i, _)| self.is_shown(*i)) {
            if chip.contains(x, y) {
                return if Self::close_button(*chip).is_some_and(|close| close.contains(x, y)) { Hit::ChipClose(i) } else { Hit::Chip(i) };
            }
        }
        if y >= 0.0 && y < TITLE_HEIGHT {
            Hit::Drag
        } else if self.sidebar.is_some_and(|side| side.contains(x, y)) {
            Hit::Side
        } else if self.pane.contains(x, y) {
            Hit::Pane
        } else {
            Hit::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPTION: f32 = super::CAPTION;

    fn top(width: f32, tabs: usize, selected: usize) -> Layout {
        Layout::new(width, 700.0, tabs, CAPTION, selected, Mode::TOP, 0.0, &[(0..tabs).collect()])
    }

    fn side(layout: TabLayout, open: bool, groups: &[Vec<usize>]) -> Layout {
        let tabs = groups.iter().map(Vec::len).sum();
        Layout::new(1200.0, 720.0, tabs, CAPTION, 0, Mode { layout, sidebar_open: open, admin: false }, 200.0, groups)
    }

    #[test]
    fn every_chip_has_the_same_width_and_the_strip_leaves_room_to_drag() {
        for tabs in [1, 3, 12, 30] {
            let layout = top(1200.0, tabs, 0);
            let first = layout.chips[0].w;
            assert!(layout.chips.iter().all(|c| c.w == first), "{tabs} tabs");
            let strip_end = layout.chips.last().unwrap().x + first;
            if first > logic::CHIP_MIN_WIDTH as f32 {
                assert!(strip_end + 2.0 * BUTTON + DRAG_MIN <= 1200.0 - CAPTION - BUTTON + 0.01, "{tabs} tabs");
            }
        }
    }

    #[test]
    fn the_caption_buttons_fill_the_corner_in_the_full_height_of_the_bar() {
        let layout = top(1000.0, 3, 0);
        let [minimize, maximize, close] = layout.caption_buttons;
        assert_eq!((minimize.x, close.x + close.w), (1000.0 - CAPTION, 1000.0));
        assert!(layout.caption_buttons.iter().all(|b| b.y == 0.0 && b.h == TITLE_HEIGHT && b.w == CAPTION / 3.0));
        assert_eq!(layout.hit(minimize.x + 1.0, 1.0), Hit::Minimize);
        assert_eq!(layout.hit(maximize.x + 1.0, TITLE_HEIGHT - 1.0), Hit::Maximize);
        assert_eq!(layout.hit(999.0, 16.0), Hit::Close);
    }

    #[test]
    fn the_hit_test_names_what_is_under_the_pointer() {
        let layout = top(1000.0, 3, 0);
        let chip = layout.chips[1];
        assert_eq!(layout.hit(chip.x + 10.0, chip.y + 10.0), Hit::Chip(1));
        let close = Layout::close_button(chip).expect("a wide chip has a close button");
        assert_eq!(layout.hit(close.x + 2.0, close.y + 2.0), Hit::ChipClose(1));
        assert_eq!(layout.hit(layout.new_tab.x + 4.0, layout.new_tab.y + 4.0), Hit::NewTab);
        assert_eq!(layout.hit(layout.profiles.x + 4.0, layout.profiles.y + 4.0), Hit::Profiles);
        assert_eq!(layout.hit(layout.menu.x + 4.0, layout.menu.y + 4.0), Hit::Menu);
        assert_eq!(layout.hit(layout.menu.x - 20.0, 10.0), Hit::Drag);
        assert_eq!(layout.hit(600.0, 300.0), Hit::Pane);
        assert_eq!(layout.hit(-1.0, 300.0), Hit::None);
        assert!(layout.sidebar_button.is_none() && layout.sidebar.is_none());
    }

    #[test]
    fn chips_that_fill_the_strip_exactly_all_show() {
        for width in [900.0_f32, 1088.0, 1100.0, 1234.5] {
            assert_eq!(top(width, 6, 5).shown, 6, "width {width}");
        }
    }

    #[test]
    fn a_strip_that_overflows_shows_the_tabs_that_fit_and_scrolls_to_the_selected_one() {
        let start = top(1000.0, 30, 0);
        assert_eq!(start.first, 0);
        assert!(start.shown >= 1 && start.shown < 30, "{} tabs shown", start.shown);
        for i in 0..30 {
            let chip = start.chips[i];
            assert_eq!(start.is_shown(i), chip.x >= STRIP_LEFT && chip.x + chip.w <= start.new_tab.x + 0.01, "tab {i}");
        }
        let end = top(1000.0, 30, 29);
        assert!(end.is_shown(29) && !end.is_shown(0), "the selected tab is in the strip");
    }

    #[test]
    fn the_room_asked_for_at_the_right_of_the_bar_is_kept_free_of_tabs() {
        let layout = Layout::new(1200.0, 700.0, 4, CAPTION, 0, Mode::TOP, 300.0, &[(0..4).collect()]);
        assert_eq!(layout.right.x + layout.right.w, layout.menu.x);
        assert!(layout.profiles.x + BUTTON + DRAG_MIN <= layout.right.x + 0.01);
    }

    #[test]
    fn the_list_is_two_lines_a_tab_in_the_order_given_with_headings_only_when_grouped() {
        let plain = side(TabLayout::List, true, &[vec![2, 0, 1]]);
        assert_eq!(plain.sidebar.map(|s| s.w), Some(SIDE_LIST));
        assert_eq!(plain.pane.x, SIDE_LIST);
        assert!(plain.chips[2].y < plain.chips[0].y && plain.chips[0].y < plain.chips[1].y, "the order given");
        assert!(plain.headings.is_empty());
        assert_eq!(plain.hit(plain.chips[0].x + 4.0, plain.chips[0].y + 4.0), Hit::Chip(0));
        let close = Layout::close_button(plain.chips[1]).expect("a row of the list has a close button");
        assert_eq!(plain.hit(close.x + 2.0, close.y + 2.0), Hit::ChipClose(1));
        let grouped = side(TabLayout::List, true, &[vec![1], vec![], vec![0, 2]]);
        assert_eq!(grouped.headings.iter().map(|(_, g)| *g).collect::<Vec<_>>(), [0, 2], "empty groups have no heading");
        assert!(grouped.headings[1].0.y < grouped.chips[0].y);
    }

    #[test]
    fn an_elevated_window_has_its_shield_first_and_everything_else_moves_over_for_it() {
        for layout in [TabLayout::Top, TabLayout::List] {
            let plain = Mode { layout, sidebar_open: true, admin: false };
            let admin = Mode { admin: true, ..plain };
            let without = Layout::new(1200.0, 720.0, 3, CAPTION, 0, plain, 0.0, &[vec![0, 1, 2]]);
            let with = Layout::new(1200.0, 720.0, 3, CAPTION, 0, admin, 0.0, &[vec![0, 1, 2]]);
            assert!(without.shield.is_none());
            let shield = with.shield.expect("the shield");
            assert_eq!((shield.x, shield.w), (STRIP_LEFT, SHIELD));
            assert_eq!(with.new_tab.x - without.new_tab.x, SHIELD, "{layout:?}: the buttons move over by the shield");
            let first = if layout == TabLayout::Top { with.chips[0].x } else { with.sidebar_button.unwrap().x };
            assert!(first >= shield.x + shield.w, "{layout:?}: nothing under the shield");
        }
    }

    #[test]
    fn the_buttons_of_the_bar_stay_where_they_are_whether_the_sidebar_is_shown_or_not() {
        let open = side(TabLayout::List, true, &[vec![0, 1]]);
        let shut = side(TabLayout::List, false, &[vec![0, 1]]);
        assert_eq!((open.sidebar_button, open.new_tab), (shut.sidebar_button, shut.new_tab));
        assert_eq!(open.hit(STRIP_LEFT + 4.0, 10.0), Hit::Sidebar);
        assert!(shut.sidebar.is_none() && shut.pane.x == 0.0);
        assert!(!shut.is_shown(0) && !matches!(shut.hit(20.0, 100.0), Hit::Chip(_)), "a hidden sidebar has no rows to hit");
        let icons = side(TabLayout::Icons, true, &[vec![0, 1]]);
        assert_eq!(icons.pane.x, SIDE_ICONS);
        assert!(Layout::close_button(icons.chips[0]).is_none(), "an icon has no close button");
    }
}
