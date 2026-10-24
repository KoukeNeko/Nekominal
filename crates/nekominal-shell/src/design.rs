//! The measures the chrome is built from. The bar, the pop-ups and the settings take theirs from here, so that they look made
//! by one hand: square but for a hint of a corner, and every space on a grid of four. A control that is round by nature (the
//! switch, the track and knob of a slider, the radio) is round and not rounded by `RADIUS`.

/// The corner of every plate, tab, field, tile and pop-up, in DIP.
pub const RADIUS: f32 = 4.0;
/// Between two neighbours: the rows of a list, the buttons of a row, the plates of a pop-up and the edge of it.
pub const GAP: f32 = 4.0;
/// Between the edge of a tab, a button or a row of a menu and what is in it.
pub const INSET: f32 = 12.0;
/// Between the edge of a card (a row of the settings, the question of a dialog) and what is in it.
pub const MARGIN: f32 = 16.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_measure_is_on_the_grid_of_four_and_they_grow_in_order() {
        for (name, value) in [("RADIUS", RADIUS), ("GAP", GAP), ("INSET", INSET), ("MARGIN", MARGIN)] {
            assert_eq!(value % 4.0, 0.0, "{name} is {value}");
        }
        assert!(RADIUS <= GAP && GAP < INSET && INSET < MARGIN);
    }
}
