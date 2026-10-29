//! Where things are, in DIP: the title bar of 32 with its tabs or the button of the sidebar, the sidebar of the tabs when
//! the tabs stand there (a list of two lines a tab, or a column of icons), and the area that belongs to the terminal. The
//! caption buttons (minimize, maximize, close) are at the top right, as tall as the bar. Pure: the window code gives the
//! size, the tabs and how much room the right of the bar needs (the waiting count and the numbers of the agent), and gets
//! rectangles and hit tests back.

use nekominal_model::settings::{TabLayout, TabStyle};
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
/// Width of the sidebar as a list (of each style) and as icons.
pub const SIDE_LIST: f32 = 220.0;
pub const SIDE_CARDS: f32 = 280.0;
pub const SIDE_REPO: f32 = 240.0;
pub const SIDE_ICONS: f32 = 52.0;
/// A row of the list: the program and its state on a line, what it last did (or where it is) under them.
pub const LIST_ROW: f32 = 44.0;
/// A card before what it holds under its two lines, and the space between two cards.
pub const CARD_ROW: f32 = 48.0;
pub const CARD_GAP: f32 = 6.0;
/// A tab under its repository: one line.
pub const REPO_ROW: f32 = 30.0;
/// The heading of a group of the list (when it is grouped by state).
pub const LIST_HEADING: f32 = 26.0;
/// An icon of the column, and the space a row of the column takes.
pub const ICON_CELL: f32 = 36.0;
pub const ICON_ROW: f32 = 44.0;
pub const SIDE_PAD: f32 = 8.0;
/// How far from the edge of the sidebar the pointer takes it, each side, in DIP.
const EDGE_GRIP: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    None,
    /// Empty title bar: the window can be dragged by it.
    Drag,
    Chip(usize),
    ChipClose(usize),
    /// A badge of a tab (its pull request, a port it listens on), by its place in the list the window drew last.
    Badge(usize),
    NewTab,
    Profiles,
    /// The position plate: opens the list of all tabs.
    Plate,
    Menu,
    /// Hides or shows the sidebar.
    Sidebar,
    /// The count of the tabs that wait for the user: goes to the next of them.
    Waiting,
    /// The context meter of the focused pane's agent: the pointer on it shows the context card.
    Meter,
    /// The context card under the meter: the pointer on it keeps it open.
    Card,
    /// The buttons of the card's Remote Control row: open the session's link, copy it.
    RemoteOpen,
    RemoteCopy,
    /// The card of a pull request under its badge (the pointer on it keeps it open), and its buttons: open the pull
    /// request, copy its link, dismiss its badge.
    PrCard,
    PrOpen,
    PrCopy,
    PrDismiss,
    /// The branch of the focused pane's folder in the bar: the pointer on it shows the branch card (`app/branch_card.rs`);
    /// and the card, which the pointer on it keeps open.
    Branch,
    BranchCard,
    /// The row of the card that names the remote: pressed, it opens the repository in the browser.
    BranchRemote,
    /// The subagents piece of the bar, its card, and a row of the card (`App::sub_rows`), which opens the drawer.
    Subagents,
    SubCard,
    SubRow(usize),
    /// A part of the subagents drawer that does something (`App::drawer_hits`), or the rest of it.
    Drawer(usize),
    DrawerBody,
    /// What of the sidebar is not a row.
    Side,
    /// The right edge of the sidebar as a list, which is dragged to make it wider or narrower.
    SideEdge,
    Minimize,
    Maximize,
    Close,
    Pane,
}

/// How the tabs are shown: where (`TabLayout`), how a row of the list looks (`TabStyle`), whether the sidebar is open,
/// and whether the shield of an elevated window
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub layout: TabLayout,
    pub style: TabStyle,
    pub sidebar_open: bool,
    pub admin: bool,
    /// The width the edge of the list was dragged to, in DIP; 0 is the width of its style.
    pub width: f32,
    /// Full screen, as on macOS: minimize and close, no maximize, and the bar does not move the window.
    pub full_screen: bool,
}

impl Mode {
    pub const TOP: Mode = Mode { layout: TabLayout::Top, style: TabStyle::Activity, sidebar_open: true, admin: false, width: 0.0, full_screen: false };

    /// The width of the sidebar on show; 0 with the tabs on top or the sidebar hidden.
    pub fn side_width(self) -> f32 {
        match (self.layout, self.sidebar_open) {
            (TabLayout::List, true) if self.width > 0.0 => self.width,
            (TabLayout::List, true) => match self.style {
                TabStyle::Activity => SIDE_LIST,
                TabStyle::Cards => SIDE_CARDS,
                TabStyle::Repo => SIDE_REPO,
            },
            (TabLayout::Icons, true) => SIDE_ICONS,
            _ => 0.0,
        }
    }

    /// A row of the list before what it adds for its own (`Layout::new`'s `tall`), and the space under it.
    fn row(self) -> (f32, f32) {
        match self.style {
            TabStyle::Activity => (LIST_ROW, 2.0),
            TabStyle::Cards => (CARD_ROW, CARD_GAP),
            TabStyle::Repo => (REPO_ROW, 2.0),
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
    /// How far the sidebar is scrolled down (`scrolled`), and where what it lists ends, as scrolled, with its padding.
    pub scroll: f32,
    end: f32,
}

impl Layout {
    /// `caption` is the width of the caption buttons at the right edge; `right` the room the waiting count and the numbers
    /// of the agent ask for, left of the menu. `groups` are the tabs in the order the sidebar lists them, by group (one
    /// group without a heading when the list is not grouped); the strip on top always has the tabs in their own order.
    /// When the chips at their narrowest do not all fit, the strip shows the ones that do, scrolled so that tab `selected`
    /// is the last of them or earlier. `tall` is what each row of the list adds under its two lines (the lines of its
    /// badges), by tab; a tab past its end adds nothing.
    pub fn new(width: f32, height: f32, tabs: usize, caption: f32, selected: usize, mode: Mode, right: f32, groups: &[Vec<usize>], tall: &[f32]) -> Layout {
        let cell = caption / 3.0;
        // In full screen there is no maximize: minimize and close, and the maximize one is nowhere (no width, never hit).
        let caption = if mode.full_screen { 2.0 * cell } else { caption };
        let menu = Rect::new((width - caption - BUTTON).max(0.0), 0.0, BUTTON, TITLE_HEIGHT);
        let caption_buttons = if mode.full_screen {
            [Rect::new(width - 2.0 * cell, 0.0, cell, TITLE_HEIGHT), Rect::new(width - cell, 0.0, 0.0, TITLE_HEIGHT), Rect::new(width - cell, 0.0, cell, TITLE_HEIGHT)]
        } else {
            [0.0, 1.0, 2.0].map(|i| Rect::new(width - caption + i * cell, 0.0, cell, TITLE_HEIGHT))
        };
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
            // The tabs under their repositories always have the names of the repositories over them.
            let named = list && (groups.len() > 1 || mode.style == TabStyle::Repo);
            let (row, gap) = mode.row();
            for (g, group) in groups.iter().enumerate() {
                if named && !group.is_empty() {
                    headings.push((Rect::new(SIDE_PAD, y, side - 2.0 * SIDE_PAD, LIST_HEADING), g));
                    y += LIST_HEADING;
                }
                for &i in group.iter().filter(|i| **i < tabs) {
                    chips[i] = if list {
                        Rect::new(SIDE_PAD, y, side - 2.0 * SIDE_PAD, row + tall.get(i).copied().unwrap_or(0.0))
                    } else {
                        Rect::new((SIDE_ICONS - ICON_CELL) / 2.0, y + (ICON_ROW - ICON_CELL) / 2.0, ICON_CELL, ICON_CELL)
                    };
                    y += if list { chips[i].h + gap } else { ICON_ROW };
                }
            }
            let shown = if sidebar.is_some() { tabs } else { 0 };
            let end = y + SIDE_PAD;
            return Layout { size: (width, height), mode, chips, first: 0, shown, headings, shield, left, sidebar_button: Some(sidebar_button), new_tab, profiles, menu, caption_buttons, sidebar, title, right, pane, scroll: 0.0, end };
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
        Layout { size: (width, height), mode, chips, first, shown, headings: Vec::new(), shield, left, sidebar_button: None, new_tab, profiles, menu, caption_buttons, sidebar: None, title: Rect::new(0.0, 0.0, 0.0, 0.0), right, pane, scroll: 0.0, end: 0.0 }
    }

    /// The sidebar scrolled down by `offset` DIP, held between its top and the point where its last row stands at its
    /// bottom: the rows and the headings move up by as much, and the ones that leave it are no longer shown.
    pub fn scrolled(mut self, offset: f32) -> Layout {
        let Some(sidebar) = self.sidebar else { return self };
        let scroll = offset.clamp(0.0, (self.end - (sidebar.y + sidebar.h)).max(0.0));
        for rect in self.chips.iter_mut().chain(self.headings.iter_mut().map(|(rect, _)| rect)) {
            rect.y -= scroll;
        }
        self.scroll = scroll;
        self.end -= scroll;
        self
    }

    /// The offset `scrolled` takes for row `i` to stand wholly in the sidebar, moving as little as it can: the offset in
    /// effect when the row is already there.
    pub fn scroll_showing(&self, i: usize) -> f32 {
        let (Some(sidebar), Some(row)) = (self.sidebar, self.chips.get(i).filter(|row| row.w > 0.0)) else { return self.scroll };
        let (top, bottom) = (sidebar.y + SIDE_PAD, sidebar.y + sidebar.h - SIDE_PAD);
        if row.y < top {
            self.scroll - (top - row.y)
        } else if row.y + row.h > bottom {
            self.scroll + (row.y + row.h - bottom)
        } else {
            self.scroll
        }
    }

    /// Whether tab `i` is on show (in the strip, or wholly in the sidebar while it is open, as scrolled).
    pub fn is_shown(&self, i: usize) -> bool {
        if let Some(sidebar) = self.sidebar {
            return self.chips.get(i).is_some_and(|row| row.w > 0.0 && row.y >= sidebar.y && row.y + row.h <= sidebar.y + sidebar.h);
        }
        !self.mode.layout.is_side() && (self.first..self.first + self.shown).contains(&i)
    }

    /// The close button inside a chip or a row of the list, `None` when it is too narrow for one (a chip, an icon). A row
    /// with badges keeps it beside its two lines.
    pub fn close_button(chip: Rect) -> Option<Rect> {
        (chip.w >= CLOSE_FROM)
            .then(|| Rect::new(chip.x + chip.w - 6.0 - CLOSE_SIZE, chip.y + (chip.h.min(LIST_ROW) - CLOSE_SIZE) / 2.0, CLOSE_SIZE, CLOSE_SIZE))
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
        // A few DIP each side of the hairline: the edge of a list, not of the narrow column of icons.
        if let Some(side) = self.sidebar.filter(|_| self.mode.layout == TabLayout::List)
            && y >= side.y
            && (x - (side.x + side.w)).abs() <= EDGE_GRIP
        {
            return Hit::SideEdge;
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
        Layout::new(width, 700.0, tabs, CAPTION, selected, Mode::TOP, 0.0, &[(0..tabs).collect()], &[])
    }

    fn side(layout: TabLayout, open: bool, groups: &[Vec<usize>]) -> Layout {
        let tabs = groups.iter().map(Vec::len).sum();
        Layout::new(1200.0, 720.0, tabs, CAPTION, 0, Mode { layout, style: TabStyle::Activity, sidebar_open: open, admin: false, width: 0.0, full_screen: false }, 200.0, groups, &[])
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
    fn full_screen_has_minimize_and_close_and_no_maximize() {
        let full = Layout::new(1000.0, 700.0, 3, CAPTION, 0, Mode { full_screen: true, ..Mode::TOP }, 0.0, &[vec![0, 1, 2]], &[]);
        let [minimize, maximize, close] = full.caption_buttons;
        assert_eq!((minimize.x, minimize.w, close.x + close.w), (1000.0 - 2.0 * CAPTION / 3.0, CAPTION / 3.0, 1000.0));
        assert_eq!(maximize.w, 0.0);
        assert_eq!(full.hit(minimize.x + 1.0, 1.0), Hit::Minimize);
        assert_eq!(full.hit(close.x + 1.0, 1.0), Hit::Close);
        assert_eq!(full.menu.x + full.menu.w, minimize.x, "the menu moves up to minimize");
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
        let layout = Layout::new(1200.0, 700.0, 4, CAPTION, 0, Mode::TOP, 300.0, &[(0..4).collect()], &[]);
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
    fn a_row_with_badges_is_taller_and_the_rows_under_it_move_down_by_as_much() {
        let mode = Mode { layout: TabLayout::List, style: TabStyle::Activity, sidebar_open: true, admin: false, width: 0.0, full_screen: false };
        let plain = Layout::new(1200.0, 720.0, 3, CAPTION, 0, mode, 0.0, &[vec![0, 1, 2]], &[]);
        let tall = Layout::new(1200.0, 720.0, 3, CAPTION, 0, mode, 0.0, &[vec![0, 1, 2]], &[0.0, 20.0]);
        assert_eq!((tall.chips[0], tall.chips[1].h), (plain.chips[0], LIST_ROW + 20.0));
        assert_eq!(tall.chips[2].y - plain.chips[2].y, 20.0);
        assert_eq!(tall.hit(tall.chips[1].x + 4.0, tall.chips[1].y + LIST_ROW + 10.0), Hit::Chip(1), "the third line is the row's");
        assert_eq!(Layout::close_button(tall.chips[1]).map(|c| c.y - tall.chips[1].y), Layout::close_button(plain.chips[1]).map(|c| c.y - plain.chips[1].y));
    }

    #[test]
    fn the_edge_of_the_list_is_taken_by_the_pointer_and_a_dragged_width_is_the_sidebars() {
        let list = Mode { layout: TabLayout::List, ..Mode::TOP };
        let plain = Layout::new(1200.0, 720.0, 2, CAPTION, 0, list, 0.0, &[vec![0, 1]], &[]);
        assert_eq!(plain.hit(SIDE_LIST, 300.0), Hit::SideEdge);
        assert_eq!(plain.hit(SIDE_LIST + 3.0, 300.0), Hit::SideEdge, "a little into the pane too");
        assert_eq!(plain.hit(SIDE_LIST + 6.0, 300.0), Hit::Pane);
        assert_eq!(plain.hit(SIDE_LIST, 20.0), Hit::Drag, "not in the title bar");
        let wide = Layout::new(1200.0, 720.0, 2, CAPTION, 0, Mode { width: 333.0, ..list }, 0.0, &[vec![0, 1]], &[]);
        assert_eq!((wide.pane.x, wide.chips[0].w), (333.0, 333.0 - 2.0 * SIDE_PAD));
        let icons = Layout::new(1200.0, 720.0, 2, CAPTION, 0, Mode { layout: TabLayout::Icons, width: 333.0, ..Mode::TOP }, 0.0, &[vec![0, 1]], &[]);
        assert_eq!(icons.pane.x, SIDE_ICONS, "the column of icons keeps its width");
        assert_ne!(icons.hit(SIDE_ICONS, 300.0), Hit::SideEdge);
    }

    #[test]
    fn each_style_of_the_list_has_its_width_its_rows_and_its_headings() {
        let groups = [vec![0, 1], vec![2]];
        let of = |style| Layout::new(1200.0, 720.0, 3, CAPTION, 0, Mode { style, ..Mode { layout: TabLayout::List, ..Mode::TOP } }, 0.0, &groups, &[0.0, 10.0]);
        let activity = of(TabStyle::Activity);
        let cards = of(TabStyle::Cards);
        let repo = of(TabStyle::Repo);
        assert_eq!([&activity, &cards, &repo].map(|l| l.pane.x), [SIDE_LIST, SIDE_CARDS, SIDE_REPO]);
        assert!([&activity, &cards, &repo].iter().all(|l| l.chips.iter().all(|c| c.x == SIDE_PAD && c.x + c.w == l.pane.x - SIDE_PAD)), "a row is as wide as its sidebar");
        assert_eq!([&activity, &cards, &repo].map(|l| l.chips[0].h), [LIST_ROW, CARD_ROW, REPO_ROW]);
        assert_eq!(cards.chips[1].h, CARD_ROW + 10.0, "what a row adds is added to each style");
        assert_eq!(cards.chips[1].y - (cards.chips[0].y + cards.chips[0].h), CARD_GAP, "cards stand apart");
        let one = Layout::new(1200.0, 720.0, 2, CAPTION, 0, Mode { style: TabStyle::Repo, ..Mode { layout: TabLayout::List, ..Mode::TOP } }, 0.0, &[vec![0, 1]], &[]);
        assert_eq!(one.headings.len(), 1, "a single repository still has its name over its tabs");
        let plain = Layout::new(1200.0, 720.0, 2, CAPTION, 0, Mode { layout: TabLayout::List, ..Mode::TOP }, 0.0, &[vec![0, 1]], &[]);
        assert!(plain.headings.is_empty());
        let icons = Mode { layout: TabLayout::Icons, style: TabStyle::Cards, ..Mode::TOP };
        assert_eq!(icons.side_width(), SIDE_ICONS, "the style is the list's only");
    }

    #[test]
    fn the_strip_and_the_icons_take_no_room_for_badges() {
        for layout in [TabLayout::Top, TabLayout::Icons] {
            let mode = Mode { layout, style: TabStyle::Activity, sidebar_open: true, admin: false, width: 0.0, full_screen: false };
            let plain = Layout::new(1200.0, 720.0, 3, CAPTION, 0, mode, 0.0, &[vec![0, 1, 2]], &[]);
            let tall = Layout::new(1200.0, 720.0, 3, CAPTION, 0, mode, 0.0, &[vec![0, 1, 2]], &[20.0, 40.0, 20.0]);
            assert_eq!(tall.chips, plain.chips, "{layout:?}");
        }
        let list = Mode { layout: TabLayout::List, style: TabStyle::Activity, sidebar_open: true, admin: false, width: 0.0, full_screen: false };
        let short = Layout::new(1200.0, 720.0, 1, CAPTION, 0, list, 0.0, &[vec![0]], &[20.0, 40.0]);
        assert_eq!(short.chips.len(), 1, "heights past the last tab are left");
    }

    #[test]
    fn an_elevated_window_has_its_shield_first_and_everything_else_moves_over_for_it() {
        for layout in [TabLayout::Top, TabLayout::List] {
            let plain = Mode { layout, style: TabStyle::Activity, sidebar_open: true, admin: false, width: 0.0, full_screen: false };
            let admin = Mode { admin: true, ..plain };
            let without = Layout::new(1200.0, 720.0, 3, CAPTION, 0, plain, 0.0, &[vec![0, 1, 2]], &[]);
            let with = Layout::new(1200.0, 720.0, 3, CAPTION, 0, admin, 0.0, &[vec![0, 1, 2]], &[]);
            assert!(without.shield.is_none());
            let shield = with.shield.expect("the shield");
            assert_eq!((shield.x, shield.w), (STRIP_LEFT, SHIELD));
            assert_eq!(with.new_tab.x - without.new_tab.x, SHIELD, "{layout:?}: the buttons move over by the shield");
            let first = if layout == TabLayout::Top { with.chips[0].x } else { with.sidebar_button.unwrap().x };
            assert!(first >= shield.x + shield.w, "{layout:?}: nothing under the shield");
        }
    }

    #[test]
    fn a_scrolled_sidebar_shows_the_rows_past_its_bottom_and_hits_them_where_they_are_drawn() {
        let groups = [(0..40).collect::<Vec<usize>>()];
        let rows = side(TabLayout::List, true, &groups);
        assert!(rows.is_shown(0) && !rows.is_shown(39), "forty rows do not fit in 720 DIP");
        let scrolled = rows.clone().scrolled(f32::MAX);
        assert!(scrolled.is_shown(39) && !scrolled.is_shown(0), "scrolled to its end, the last row shows and the first leaves");
        let sidebar = scrolled.sidebar.unwrap();
        assert!(scrolled.chips[39].y + scrolled.chips[39].h <= sidebar.y + sidebar.h && scrolled.scroll < f32::MAX, "no further than the last row");
        let last = scrolled.chips[39];
        assert_eq!(scrolled.hit(last.x + 4.0, last.y + 4.0), Hit::Chip(39));
        assert_eq!(rows.clone().scrolled(-50.0), rows, "never above the top");
        let to_last = rows.clone().scrolled(rows.scroll_showing(39));
        assert!(to_last.is_shown(39) && to_last.scroll <= scrolled.scroll);
        assert_eq!(to_last.clone().scrolled(to_last.scroll_showing(0)).scroll, 0.0, "back up to the first row");
        assert_eq!(rows.scroll_showing(3), 0.0, "a row on show moves nothing");
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
