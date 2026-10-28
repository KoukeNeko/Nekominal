//! How the tab chips move: toward the place the layout gives them, a little further every frame, so that a closed tab
//! makes the others slide over, a narrower strip shrinks the chips, and a dragged chip makes room as it passes.
//!
//! There is no timer. A frame that finds a chip short of its place asks for the next one, and the frame that finds every
//! chip in place asks for none, so a window at rest draws nothing. Pure: the window code gives the time between two frames.
//!
//! The marks of the states of the tabs move too (a spinner, a pulse); they are the one thing here a timer drives.

use nekominal_ui::Rect;

/// The time one slide takes to cover all but a third of its way, in seconds. It is the pace of the whole slide: the chip
/// is within a pixel of its place after about five times as much.
pub const TIME_CONSTANT: f32 = 0.055;

/// A frame longer than this counts as this long: a window that was not drawn for a while does not jump to the end.
pub const LONGEST_FRAME: f32 = 0.05;

/// Closer than this (in DIP) the chip is in its place.
const REST: f32 = 0.4;

/// The share of what is left of a slide that `seconds` of it cover.
fn share(seconds: f32) -> f32 {
    1.0 - (-seconds.clamp(0.0, LONGEST_FRAME) / TIME_CONSTANT).exp()
}

/// `current` moved toward `target` by what `seconds` of the slide cover.
pub fn approach(current: Rect, target: Rect, seconds: f32) -> Rect {
    let share = share(seconds);
    let toward = |from: f32, to: f32| from + (to - from) * share;
    let moved = Rect::new(toward(current.x, target.x), toward(current.y, target.y), toward(current.w, target.w), toward(current.h, target.h));
    if at_rest(moved, target) { target } else { moved }
}

/// A light (0 to 1) moved toward `target` at the pace of a slide: how the ring of a field comes on when it takes the
/// keyboard and goes when it leaves.
pub fn fade(current: f32, target: f32, seconds: f32) -> f32 {
    let next = current + (target - current) * share(seconds);
    if (next - target).abs() < 0.01 { target } else { next }
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

/// The marks of what a tab is doing move while it lasts: a spinner while it works, a slow pulse while it waits for the user.
/// Their frames come from a timer of the window at this pace, and only while one of them is on show.
pub const MARK_FRAME_MS: u32 = 33;
/// One turn of the spinner, and one breath of the pulse, in seconds.
pub const SPIN_PERIOD: f32 = 0.9;
pub const PULSE_PERIOD: f32 = 1.6;
/// The dots of the spinner, and how light the faintest of them stays.
pub const SPINNER_DOTS: usize = 8;
const SPINNER_FLOOR: f32 = 0.18;

/// How light (0 to 1) dot `dot` of the spinner is `seconds` after it started: the dot at the head of the turn is full, those
/// behind it fade toward the floor.
pub fn spinner_dot(dot: usize, seconds: f32) -> f32 {
    let count = SPINNER_DOTS as f32;
    let head = (seconds / SPIN_PERIOD).rem_euclid(1.0) * count;
    let behind = (head - dot as f32).rem_euclid(count) / count;
    SPINNER_FLOOR + (1.0 - SPINNER_FLOOR) * (1.0 - behind).powi(2)
}

/// Where dot `dot` of the spinner sits on its circle, as (x, y) from the centre for a radius of 1: the first on top, then
/// clockwise.
pub fn spinner_place(dot: usize) -> (f32, f32) {
    let angle = dot as f32 / SPINNER_DOTS as f32 * std::f32::consts::TAU;
    (angle.sin(), -angle.cos())
}

/// How opaque (0.45 to 1) the pulsing mark is `seconds` after it started: full at the start, faintest half a breath later.
pub fn pulse(seconds: f32) -> f32 {
    0.725 + 0.275 * (seconds / PULSE_PERIOD * std::f32::consts::TAU).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spinner_head_goes_round_and_the_pulse_breathes_within_its_range() {
        let brightest = |s: f32| (0..SPINNER_DOTS).max_by(|a, b| spinner_dot(*a, s).total_cmp(&spinner_dot(*b, s))).unwrap();
        let heads: Vec<usize> = (0..SPINNER_DOTS).map(|n| brightest((n as f32 + 0.5) / SPINNER_DOTS as f32 * SPIN_PERIOD)).collect();
        assert_eq!(heads, (0..SPINNER_DOTS).collect::<Vec<_>>(), "the head moves one dot at a time, clockwise");
        assert_eq!(brightest(0.25 * SPIN_PERIOD), brightest(1.25 * SPIN_PERIOD), "a turn later it is where it was");
        assert!((0..SPINNER_DOTS).all(|d| (SPINNER_FLOOR..=1.0).contains(&spinner_dot(d, 0.3))));
        let (x, y) = spinner_place(SPINNER_DOTS / 4);
        assert!((x - 1.0).abs() < 1e-5 && y.abs() < 1e-5, "a quarter of the way round is on the right");
        let samples: Vec<f32> = (0..64).map(|n| pulse(n as f32 * PULSE_PERIOD / 64.0)).collect();
        assert!(samples.iter().all(|a| (0.449..=1.0).contains(a)));
        assert!((pulse(0.0) - 1.0).abs() < 1e-5 && (pulse(PULSE_PERIOD / 2.0) - 0.45).abs() < 1e-5);
    }

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
    fn the_ring_of_a_field_comes_on_over_a_few_frames_and_goes_the_same_way() {
        let mut light = 0.0;
        let mut frames = 0;
        while light < 1.0 {
            let next = fade(light, 1.0, 1.0 / 60.0);
            assert!(next > light && next <= 1.0);
            light = next;
            frames += 1;
            assert!(frames < 60, "still fading after a second");
        }
        assert!(frames > 4, "lit in {frames} frames: that is a jump, not a fade");
        assert!(fade(1.0, 0.0, 1.0 / 60.0) < 1.0);
        assert_eq!(fade(0.3, 0.3, 0.02), 0.3);
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
