//! Where things are, in DIP: the title bar with its tab chips and the new-tab button, and the area below it that belongs
//! to the terminal. The caption buttons (minimize, maximize, close) are drawn by the window at the top right, each as
//! tall as the bar so that their glyphs sit in its middle. Pure: the window code gives the size and the number of tabs and gets rectangles and hit tests
//! back.

use nekominal_ui::Rect;

use crate::logic;

/// Height of the title bar: the tab strip and the caption buttons live in it.
pub const TITLE_HEIGHT: f32 = 48.0;
/// Space above the tabs. A tab is as tall as the bar, so that the selected one is one piece with the terminal below it.
pub const TAB_TOP: f32 = 0.0;
/// Space between two chips: the tabs touch and a line tells them apart.
pub const CHIP_GAP: f32 = 0.0;
/// Space left of the first chip.
pub const STRIP_LEFT: f32 = 0.0;
/// Width of the three caption buttons together.
pub const CAPTION: f32 = 3.0 * 46.0;
/// Width of the new-tab, profiles and menu buttons; they are as tall as the bar.
pub const BUTTON: f32 = 32.0;
/// Space between the menu button and the caption buttons.
const MENU_RIGHT: f32 = 0.0;
/// The part of the title bar that stays free for dragging the window.
pub const DRAG_MIN: f32 = 48.0;
/// A chip at least this wide has room for a close button.
pub const CLOSE_FROM: f32 = 96.0;
const CLOSE_SIZE: f32 = 20.0;

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
    Minimize,
    Maximize,
    Close,
    Pane,
    /// The status bar at the bottom of the window.
    Status,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub size: (f32, f32),
    /// One rectangle per tab. Only the tabs from `first` on for `shown` of them are in the strip; the others lie
    /// outside it (left of the start or past the end) and are neither drawn nor hit.
    pub chips: Vec<Rect>,
    pub first: usize,
    pub shown: usize,
    pub new_tab: Rect,
    /// Opens the list of shells (profiles) a new tab can start.
    pub profiles: Rect,
    pub menu: Rect,
    /// Minimize, maximize or restore, and close: the whole height of the title bar.
    pub caption_buttons: [Rect; 3],
    pub pane: Rect,
}

impl Layout {
    /// `caption` is the width of the system caption buttons at the right edge. When the chips at their narrowest do not
    /// all fit, the strip shows the ones that do, scrolled so that tab `selected` is the last of them or earlier.
    pub fn new(width: f32, height: f32, tabs: usize, caption: f32, selected: usize) -> Layout {
        let menu = Rect::new((width - caption - MENU_RIGHT - BUTTON).max(STRIP_LEFT), 0.0, BUTTON, TITLE_HEIGHT);
        let strip_end = menu.x - DRAG_MIN;
        // The room for the chips: what is between the strip start and the two buttons at the end of the strip.
        let available = (strip_end - STRIP_LEFT - 2.0 * (BUTTON + CHIP_GAP)).max(0.0);
        let chip_width = logic::tab_width(tabs, f64::from(available), f64::from(CHIP_GAP)) as f32;
        // The epsilon: chips that fill the strip exactly divide to 5.999... as easily as to 6.
        let fits = (((available + CHIP_GAP) / (chip_width + CHIP_GAP) + 1e-3).floor() as usize).max(1);
        let shown = tabs.min(fits);
        let first = if tabs <= shown { 0 } else { (selected + 1).saturating_sub(shown).min(tabs - shown) };
        let chips: Vec<Rect> = (0..tabs)
            .map(|i| Rect::new(STRIP_LEFT + (i as f32 - first as f32) * (chip_width + CHIP_GAP), TAB_TOP, chip_width, TITLE_HEIGHT - TAB_TOP))
            .collect();
        let after = STRIP_LEFT + shown as f32 * (chip_width + CHIP_GAP);
        let new_tab = Rect::new(after.min(strip_end - 2.0 * BUTTON - CHIP_GAP).max(STRIP_LEFT), 0.0, BUTTON, TITLE_HEIGHT);
        let profiles = Rect::new(new_tab.x + BUTTON + CHIP_GAP, 0.0, BUTTON, TITLE_HEIGHT);
        Layout {
            size: (width, height),
            chips,
            first,
            shown,
            new_tab,
            profiles,
            menu,
            caption_buttons: [0.0, 1.0, 2.0].map(|i| Rect::new(width - caption + i * caption / 3.0, 0.0, caption / 3.0, TITLE_HEIGHT)),
            pane: Rect::new(0.0, TITLE_HEIGHT, width, (height - TITLE_HEIGHT).max(0.0)),
        }
    }

    /// Whether tab `i` is in the strip.
    pub fn is_shown(&self, i: usize) -> bool {
        (self.first..self.first + self.shown).contains(&i)
    }

    /// The close button inside a chip, `None` when the chip is too narrow for one.
    pub fn close_button(chip: Rect) -> Option<Rect> {
        (chip.w >= CLOSE_FROM)
            .then(|| Rect::new(chip.x + chip.w - 8.0 - CLOSE_SIZE, chip.y + (chip.h - CLOSE_SIZE) / 2.0, CLOSE_SIZE, CLOSE_SIZE))
    }

    pub fn hit(&self, x: f32, y: f32) -> Hit {
        for (hit, button) in [Hit::Minimize, Hit::Maximize, Hit::Close].into_iter().zip(self.caption_buttons) {
            if button.contains(x, y) {
                return hit;
            }
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
                return if Self::close_button(*chip).is_some_and(|close| close.contains(x, y)) {
                    Hit::ChipClose(i)
                } else {
                    Hit::Chip(i)
                };
            }
        }
        if y >= 0.0 && y < TITLE_HEIGHT {
            Hit::Drag
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

    #[test]
    fn every_chip_has_the_same_width_and_the_strip_leaves_room_to_drag() {
        for tabs in [1, 3, 12, 30] {
            let layout = Layout::new(1200.0, 700.0, tabs, CAPTION, 0);
            let first = layout.chips[0].w;
            assert!(layout.chips.iter().all(|c| c.w == first), "{tabs} tabs");
            let strip_end = layout.chips.last().unwrap().x + first;
            // the chips themselves stay left of the free drag area, until the floor width makes the strip overflow
            if first > logic::CHIP_MIN_WIDTH as f32 {
                assert!(strip_end + 2.0 * (BUTTON + CHIP_GAP) + DRAG_MIN <= 1200.0 - CAPTION + 0.01, "{tabs} tabs");
            }
        }
    }

    #[test]
    fn the_caption_buttons_fill_the_corner_in_the_full_height_of_the_bar() {
        let layout = Layout::new(1000.0, 600.0, 3, CAPTION, 0);
        let [minimize, maximize, close] = layout.caption_buttons;
        assert_eq!((minimize.x, close.x + close.w), (1000.0 - CAPTION, 1000.0));
        assert!(layout.caption_buttons.iter().all(|b| b.y == 0.0 && b.h == TITLE_HEIGHT && b.w == CAPTION / 3.0));
        assert_eq!(layout.hit(minimize.x + 1.0, 1.0), Hit::Minimize);
        assert_eq!(layout.hit(maximize.x + 1.0, TITLE_HEIGHT - 1.0), Hit::Maximize);
        assert_eq!(layout.hit(999.0, 24.0), Hit::Close);
    }

    #[test]
    fn the_hit_test_names_what_is_under_the_pointer() {
        let layout = Layout::new(1000.0, 600.0, 3, CAPTION, 0);
        let chip = layout.chips[1];
        assert_eq!(layout.hit(chip.x + 10.0, chip.y + 10.0), Hit::Chip(1));
        let close = Layout::close_button(chip).expect("a wide chip has a close button");
        assert_eq!(layout.hit(close.x + 2.0, close.y + 2.0), Hit::ChipClose(1));
        assert_eq!(layout.hit(layout.new_tab.x + 4.0, layout.new_tab.y + 4.0), Hit::NewTab);
        assert_eq!(layout.hit(layout.profiles.x + 4.0, layout.profiles.y + 4.0), Hit::Profiles);
        assert_eq!(layout.hit(layout.menu.x + 4.0, layout.menu.y + 4.0), Hit::Menu);
        assert_eq!(layout.hit(layout.menu.x - 20.0, 10.0), Hit::Drag);
        assert_eq!(layout.hit(chip.x + 10.0, 2.0), Hit::Chip(1), "a chip is as tall as the title bar");
        assert_eq!(layout.hit(chip.x + 10.0, TITLE_HEIGHT - 1.0), Hit::Chip(1));
        assert_eq!(layout.hit(600.0, 300.0), Hit::Pane);
        assert_eq!(layout.hit(-1.0, 300.0), Hit::None);
    }

    #[test]
    fn chips_that_fill_the_strip_exactly_all_show() {
        for width in [900.0_f32, 1088.0, 1100.0, 1234.5] {
            let layout = Layout::new(width, 600.0, 6, CAPTION, 5);
            assert_eq!(layout.shown, 6, "width {width}");
        }
    }

    #[test]
    fn a_strip_that_overflows_shows_the_tabs_that_fit_and_scrolls_to_the_selected_one() {
        let at = |selected| Layout::new(1000.0, 600.0, 30, CAPTION, selected);
        let start = at(0);
        assert_eq!(start.first, 0);
        assert!(start.shown >= 1 && start.shown < 30, "{} tabs shown", start.shown);
        let new_tab_left = start.new_tab.x;
        for i in 0..30 {
            let chip = start.chips[i];
            assert_eq!(start.is_shown(i), chip.x >= STRIP_LEFT && chip.x + chip.w <= new_tab_left, "tab {i}");
            if !start.is_shown(i) {
                assert!(!matches!(start.hit(chip.x + 2.0, chip.y + 2.0), Hit::Chip(_)), "tab {i} is hidden but hit");
            }
        }
        let end = at(29);
        assert!(end.is_shown(29) && !end.is_shown(0), "the selected tab is in the strip");
        assert_eq!(end.shown, start.shown);
        let middle = at(start.shown - 1);
        assert_eq!(middle.first, 0, "a selected tab that already fits does not scroll the strip");
    }

    #[test]
    fn a_narrow_chip_has_no_close_button() {
        let layout = Layout::new(700.0, 500.0, 30, CAPTION, 0);
        assert!(layout.chips.iter().all(|c| Layout::close_button(*c).is_none()));
    }
}
