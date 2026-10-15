//! How the tab chips move: toward the place the layout gives them, a little further every frame, so that a closed tab
//! makes the others slide over, a narrower strip shrinks the chips, and a dragged chip makes room as it passes.
//!
//! There is no timer. A frame that finds a chip short of its place asks for the next one, and the frame that finds every
//! chip in place asks for none, so a window at rest draws nothing. Pure: the window code gives the time between two frames.

use nekominal_ui::Rect;

/// The time one slide takes to cover all but a third of its way, in seconds. It is the pace of the whole slide: the chip
/// is within a pixel of its place after about five times as much.
pub const TIME_CONSTANT: f32 = 0.055;

/// A frame longer than this counts as this long: a window that was not drawn for a while does not jump to the end.
pub const LONGEST_FRAME: f32 = 0.05;

/// Closer than this (in DIP) the chip is in its place.
const REST: f32 = 0.4;

/// `current` moved toward `target` by what `seconds` of the slide cover.
pub fn approach(current: Rect, target: Rect, seconds: f32) -> Rect {
    let share = 1.0 - (-seconds.clamp(0.0, LONGEST_FRAME) / TIME_CONSTANT).exp();
    let toward = |from: f32, to: f32| from + (to - from) * share;
    let moved = Rect::new(toward(current.x, target.x), toward(current.y, target.y), toward(current.w, target.w), toward(current.h, target.h));
    if at_rest(moved, target) { target } else { moved }
}

pub fn at_rest(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < REST && (a.y - b.y).abs() < REST && (a.w - b.w).abs() < REST && (a.h - b.h).abs() < REST
}

/// The place (the index of a slot of the strip) tab `tab` takes while the tab `from` is dragged to the place `to`: the
/// tabs between the two move one place toward where the dragged tab came from.
pub fn slot_of(tab: usize, from: usize, to: usize) -> usize {
    if tab == from {
        to
    } else if from < to && tab > from && tab <= to {
        tab - 1
    } else if to < from && tab >= to && tab < from {
        tab + 1
    } else {
        tab
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_gets_closer_every_frame_and_is_in_its_place_in_the_end() {
        let target = Rect::new(200.0, 8.0, 120.0, 32.0);
        let mut chip = Rect::new(20.0, 8.0, 200.0, 32.0);
        let mut last = (chip.x - target.x).abs();
        let mut frames = 0;
        while chip != target {
            chip = approach(chip, target, 1.0 / 60.0);
            let now = (chip.x - target.x).abs();
            assert!(now <= last, "the chip moved away");
            last = now;
            frames += 1;
            assert!(frames < 60, "still moving after a second");
        }
        assert!(frames > 4, "it arrived in {frames} frames: that is a jump, not a slide");
    }

    #[test]
    fn a_long_frame_does_not_jump_to_the_end_and_a_zero_frame_does_not_move() {
        let (from, to) = (Rect::new(0.0, 0.0, 100.0, 32.0), Rect::new(300.0, 0.0, 100.0, 32.0));
        assert_eq!(approach(from, to, 0.0), from);
        let long = approach(from, to, 5.0);
        assert!(long.x > 100.0 && long.x < 300.0, "{}", long.x);
    }

    #[test]
    fn the_tabs_between_the_dragged_one_and_its_new_place_make_way() {
        // Tab 1 of 5 dragged to place 3: 2 and 3 move up one place, 0 and 4 stay.
        assert_eq!([0, 1, 2, 3, 4].map(|tab| slot_of(tab, 1, 3)), [0, 3, 1, 2, 4]);
        // Tab 3 of 5 dragged to place 1: 1 and 2 move down one place.
        assert_eq!([0, 1, 2, 3, 4].map(|tab| slot_of(tab, 3, 1)), [0, 2, 3, 1, 4]);
        // Dropped where it was: nothing moves.
        assert_eq!([0, 1, 2].map(|tab| slot_of(tab, 1, 1)), [0, 1, 2]);
    }

    #[test]
    fn the_places_are_a_permutation() {
        for (from, to) in [(0, 4), (4, 0), (2, 3), (3, 2)] {
            let mut slots: Vec<usize> = (0..5).map(|tab| slot_of(tab, from, to)).collect();
            slots.sort_unstable();
            assert_eq!(slots, [0, 1, 2, 3, 4], "{from} -> {to}");
        }
    }
}
