//! Selecting with the keyboard (mark mode): a caret that the arrows move through the text of the pane, the scrollback
//! with it, and the cell it started on. With Shift the selection runs from that cell to the caret; without, both move
//! together and the caret is the one cell selected. Pure: the window gives the keys and the size of the text.

/// A place in the text: the line (below 0 is scrollback) and the column.
pub type Point = (i32, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub anchor: Point,
    pub caret: Point,
}

/// How far the caret can go: the first and the last line of the text, the last column, and the lines of a page.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub top: i32,
    pub bottom: i32,
    pub last_column: usize,
    pub page: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// The move a key makes in mark mode: the arrows, Home and End (with Ctrl the start and the end of all the text), and
/// Page Up and Down. None for any other key.
pub fn key_move(vk: i32, ctrl: bool) -> Option<Move> {
    Some(match vk {
        0x25 => Move::Left,
        0x27 => Move::Right,
        0x26 => Move::Up,
        0x28 => Move::Down,
        0x24 if ctrl => Move::Top,
        0x23 if ctrl => Move::Bottom,
        0x24 => Move::LineStart,
        0x23 => Move::LineEnd,
        0x21 => Move::PageUp,
        0x22 => Move::PageDown,
        _ => return None,
    })
}

impl Mark {
    /// Mark mode starting at `point`: the caret and the anchor on the same cell.
    pub fn at(point: Point) -> Mark {
        Mark { anchor: point, caret: point }
    }

    /// The caret moved by `step`, the anchor staying when `extend` (Shift) and following it when not. Left and Right go
    /// on to the line before and after at the ends of a line; nothing goes past the text.
    pub fn moved(self, step: Move, extend: bool, bounds: Bounds) -> Mark {
        let (line, column) = self.caret;
        let clamp_line = |l: i32| l.clamp(bounds.top, bounds.bottom);
        let caret = match step {
            Move::Left if column > 0 => (line, column - 1),
            Move::Left if line > bounds.top => (line - 1, bounds.last_column),
            Move::Left => (line, 0),
            Move::Right if column < bounds.last_column => (line, column + 1),
            Move::Right if line < bounds.bottom => (line + 1, 0),
            Move::Right => (line, bounds.last_column),
            Move::Up => (clamp_line(line - 1), column),
            Move::Down => (clamp_line(line + 1), column),
            Move::LineStart => (line, 0),
            Move::LineEnd => (line, bounds.last_column),
            Move::PageUp => (clamp_line(line - bounds.page.max(1)), column),
            Move::PageDown => (clamp_line(line + bounds.page.max(1)), column),
            Move::Top => (bounds.top, 0),
            Move::Bottom => (bounds.bottom, bounds.last_column),
        };
        Mark { anchor: if extend { self.anchor } else { caret }, caret }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Bounds = Bounds { top: -10, bottom: 4, last_column: 79, page: 5 };

    #[test]
    fn without_shift_the_caret_moves_alone_and_with_it_the_selection_grows_from_where_it_started() {
        let start = Mark::at((2, 10));
        let moved = start.moved(Move::Right, false, B);
        assert_eq!(moved, Mark::at((2, 11)), "one cell, anchor and caret together");
        let grown = start.moved(Move::Right, true, B).moved(Move::Down, true, B);
        assert_eq!(grown, Mark { anchor: (2, 10), caret: (3, 11) });
    }

    #[test]
    fn the_caret_wraps_at_the_ends_of_a_line_and_stops_at_the_ends_of_the_text() {
        assert_eq!(Mark::at((2, 0)).moved(Move::Left, false, B).caret, (1, 79));
        assert_eq!(Mark::at((2, 79)).moved(Move::Right, false, B).caret, (3, 0));
        assert_eq!(Mark::at((-10, 0)).moved(Move::Left, false, B).caret, (-10, 0), "the first cell of all");
        assert_eq!(Mark::at((4, 79)).moved(Move::Right, false, B).caret, (4, 79), "the last cell of all");
        assert_eq!(Mark::at((-8, 5)).moved(Move::PageUp, false, B).caret, (-10, 5), "a page up stops at the top");
        assert_eq!(Mark::at((0, 5)).moved(Move::Top, true, B), Mark { anchor: (0, 5), caret: (-10, 0) });
        assert_eq!(Mark::at((0, 5)).moved(Move::Bottom, false, B).caret, (4, 79));
    }

    #[test]
    fn the_keys_of_mark_mode_are_the_arrows_home_end_and_the_page_keys() {
        assert_eq!(key_move(0x25, false), Some(Move::Left));
        assert_eq!(key_move(0x24, false), Some(Move::LineStart));
        assert_eq!(key_move(0x24, true), Some(Move::Top));
        assert_eq!(key_move(0x22, false), Some(Move::PageDown));
        assert_eq!(key_move(0x41, false), None, "a letter is not a move");
    }
}
