//! Where things are, in DIP: the title bar with its tab chips and the new-tab button, and the area below it that belongs
//! to the terminal. The caption buttons are the system's: DWM draws them at the top right, so the layout only keeps
//! the room they take. Pure: the window code gives the size and the number of tabs and gets rectangles and hit tests
//! back.

use nekominal_ui::Rect;

use crate::logic;

/// Height of the title bar: the tab strip and the caption buttons live in it.
pub const TITLE_HEIGHT: f32 = 48.0;
pub const CHIP_HEIGHT: f32 = 32.0;
pub const CHIP_GAP: f32 = 4.0;
/// Space left of the first chip.
pub const STRIP_LEFT: f32 = 12.0;
/// Width and height of the new-tab button.
pub const BUTTON: f32 = 32.0;
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
    Pane,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub size: (f32, f32),
    pub chips: Vec<Rect>,
    pub new_tab: Rect,
    pub pane: Rect,
}

impl Layout {
    /// `caption` is the width of the system caption buttons at the right edge.
    pub fn new(width: f32, height: f32, tabs: usize, caption: f32) -> Layout {
        let strip_end = width - caption - DRAG_MIN;
        // The room for the chips: what is between the strip start and the new-tab button at the end of the strip.
        let available = (strip_end - STRIP_LEFT - BUTTON - CHIP_GAP).max(0.0);
        let chip_width = logic::tab_width(tabs, f64::from(available), f64::from(CHIP_GAP)) as f32;
        let top = (TITLE_HEIGHT - CHIP_HEIGHT) / 2.0;
        let chips: Vec<Rect> = (0..tabs)
            .map(|i| Rect::new(STRIP_LEFT + i as f32 * (chip_width + CHIP_GAP), top, chip_width, CHIP_HEIGHT))
            .collect();
        let after = chips.last().map_or(STRIP_LEFT, |c| c.x + c.w + CHIP_GAP);
        let new_tab = Rect::new(after.min(strip_end - BUTTON).max(STRIP_LEFT), top, BUTTON, CHIP_HEIGHT);
        Layout {
            size: (width, height),
            chips,
            new_tab,
            pane: Rect::new(0.0, TITLE_HEIGHT, width, (height - TITLE_HEIGHT).max(0.0)),
        }
    }

    /// The close button inside a chip, `None` when the chip is too narrow for one.
    pub fn close_button(chip: Rect) -> Option<Rect> {
        (chip.w >= CLOSE_FROM)
            .then(|| Rect::new(chip.x + chip.w - 8.0 - CLOSE_SIZE, chip.y + (chip.h - CLOSE_SIZE) / 2.0, CLOSE_SIZE, CLOSE_SIZE))
    }

    pub fn hit(&self, x: f32, y: f32) -> Hit {
        if self.new_tab.contains(x, y) {
            return Hit::NewTab;
        }
        for (i, chip) in self.chips.iter().enumerate() {
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

    const CAPTION: f32 = 138.0;

    #[test]
    fn every_chip_has_the_same_width_and_the_strip_leaves_room_to_drag() {
        for tabs in [1, 3, 12, 30] {
            let layout = Layout::new(1200.0, 700.0, tabs, CAPTION);
            let first = layout.chips[0].w;
            assert!(layout.chips.iter().all(|c| c.w == first), "{tabs} tabs");
            let strip_end = layout.chips.last().unwrap().x + first;
            // the chips themselves stay left of the free drag area, until the floor width makes the strip overflow
            if first > logic::CHIP_MIN_WIDTH as f32 {
                assert!(strip_end + BUTTON + CHIP_GAP + DRAG_MIN <= 1200.0 - CAPTION + 0.01, "{tabs} tabs");
            }
        }
    }

    #[test]
    fn the_hit_test_names_what_is_under_the_pointer() {
        let layout = Layout::new(1000.0, 600.0, 3, CAPTION);
        let chip = layout.chips[1];
        assert_eq!(layout.hit(chip.x + 10.0, chip.y + 10.0), Hit::Chip(1));
        let close = Layout::close_button(chip).expect("a wide chip has a close button");
        assert_eq!(layout.hit(close.x + 2.0, close.y + 2.0), Hit::ChipClose(1));
        assert_eq!(layout.hit(layout.new_tab.x + 4.0, layout.new_tab.y + 4.0), Hit::NewTab);
        assert_eq!(layout.hit(900.0, 10.0), Hit::Drag);
        assert_eq!(layout.hit(chip.x + 10.0, 2.0), Hit::Drag, "above the chips is still the title bar");
        assert_eq!(layout.hit(600.0, 300.0), Hit::Pane);
        assert_eq!(layout.hit(-1.0, 300.0), Hit::None);
    }

    #[test]
    fn a_narrow_chip_has_no_close_button() {
        let layout = Layout::new(700.0, 500.0, 30, CAPTION);
        assert!(layout.chips.iter().all(|c| Layout::close_button(*c).is_none()));
    }
}
