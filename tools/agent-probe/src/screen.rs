//! A copy of the visible grid with the queries scenarios need: text, positions, regions, comparison.

use nekominal_term::{CellColor, SnapCell, Snapshot, alacritty_terminal::term::cell::Flags};

use crate::scrub::scrub_screen;

#[derive(Clone, Debug)]
pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    /// `rows * cols`, row-major. A wide character occupies two cells: the glyph and a spacer.
    pub cells: Vec<SnapCell>,
    /// Row text without trailing blanks; wide-character spacers are skipped.
    pub lines: Vec<String>,
    /// Viewport position of the cursor, `None` when it is scrolled out of view.
    pub cursor: Option<(usize, usize)>,
    pub cursor_visible: bool,
    pub history_size: usize,
    pub display_offset: usize,
    pub alt_screen: bool,
}

/// A difference between two screens.
#[derive(Clone, Debug, PartialEq)]
pub enum Diff {
    /// The grids have different sizes.
    Size { left: (usize, usize), right: (usize, usize) },
    /// The text of a row differs (after normalisation).
    Row { row: usize, left: String, right: String },
    /// Same text, different colour or attribute in one cell.
    Cell { row: usize, col: usize, left: String, right: String },
}

/// Attributes that are visible when a row is compared.
const VISIBLE_FLAGS: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::UNDERLINE)
    .union(Flags::INVERSE)
    .union(Flags::STRIKEOUT)
    .union(Flags::HIDDEN);

fn is_spacer(cell: &SnapCell) -> bool {
    cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

/// Attributes that make the foreground of a blank cell visible: a line through it, or swapped colours.
const BLANK_VISIBLE_FLAGS: Flags = Flags::UNDERLINE.union(Flags::STRIKEOUT).union(Flags::INVERSE);

/// Whether two cells look the same. The foreground colour of a blank cell is invisible unless the cell is
/// underlined, struck through or inverted, and a program may or may not set it, so it does not count.
fn same_appearance(a: &SnapCell, b: &SnapCell) -> bool {
    if a.ch != b.ch
        || a.zerowidth != b.zerowidth
        || a.bg_raw != b.bg_raw
        || (a.flags & VISIBLE_FLAGS) != (b.flags & VISIBLE_FLAGS)
    {
        return false;
    }
    let blank = a.ch == ' ' && a.zerowidth.is_none() && !a.flags.intersects(BLANK_VISIBLE_FLAGS);
    blank || a.fg_raw == b.fg_raw
}

fn describe_cell(cell: &SnapCell) -> String {
    let color = |c: CellColor| match c {
        CellColor::Default => "default".to_owned(),
        CellColor::Indexed(i) => format!("idx{i}"),
        CellColor::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    };
    format!("{:?} fg={} bg={} attrs={:?}", cell.ch, color(cell.fg_raw), color(cell.bg_raw), cell.flags & VISIBLE_FLAGS)
}

impl Screen {
    /// Colours and visible attributes of `cell` as text.
    pub fn describe(cell: &SnapCell) -> String {
        describe_cell(cell)
    }

    /// The cell has the default foreground and background and no visible attribute.
    pub fn has_default_pen(cell: &SnapCell) -> bool {
        cell.fg_raw == CellColor::Default
            && cell.bg_raw == CellColor::Default
            && (cell.flags & VISIBLE_FLAGS).is_empty()
    }

    pub fn from_snapshot(snapshot: &Snapshot, alt_screen: bool) -> Screen {
        let lines = (0..snapshot.rows).map(|row| snapshot.row_text(row)).collect();
        Screen {
            cols: snapshot.columns,
            rows: snapshot.rows,
            cells: snapshot.cells.clone(),
            lines,
            cursor: snapshot.cursor.position,
            cursor_visible: !matches!(
                snapshot.cursor.shape,
                nekominal_term::alacritty_terminal::vte::ansi::CursorShape::Hidden
            ),
            history_size: snapshot.history_size,
            display_offset: snapshot.display_offset,
            alt_screen,
        }
    }

    /// Builds a screen from text rows, for tests of code that reads screens.
    pub fn from_lines(lines: &[&str], cols: usize) -> Screen {
        let rows = lines.len();
        let mut cells = vec![SnapCell::default(); rows * cols];
        for (row, line) in lines.iter().enumerate() {
            let mut col = 0;
            for ch in line.chars() {
                if col >= cols {
                    break;
                }
                cells[row * cols + col].ch = ch;
                col += 1;
            }
        }
        Screen {
            cols,
            rows,
            cells,
            lines: lines.iter().map(|l| l.trim_end().to_owned()).collect(),
            cursor: None,
            cursor_visible: true,
            history_size: 0,
            display_offset: 0,
            alt_screen: false,
        }
    }

    pub fn cell(&self, row: usize, col: usize) -> &SnapCell {
        &self.cells[row * self.cols + col]
    }

    pub fn line(&self, row: usize) -> &str {
        self.lines.get(row).map_or("", String::as_str)
    }

    /// All rows joined by newlines, trailing blank rows removed.
    pub fn text(&self) -> String {
        let end = self.lines.iter().rposition(|line| !line.is_empty()).map_or(0, |last| last + 1);
        self.lines[..end].join("\n")
    }

    /// Rows `from..to` (clamped) joined by newlines.
    pub fn region(&self, from: usize, to: usize) -> String {
        self.lines[from.min(self.rows)..to.min(self.rows)].join("\n")
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.lines.iter().any(|line| line.contains(needle))
    }

    /// The first row at or after `from` whose text contains `needle`.
    pub fn row_containing(&self, needle: &str, from: usize) -> Option<usize> {
        (from..self.rows).find(|&row| self.lines[row].contains(needle))
    }

    pub fn last_row_containing(&self, needle: &str) -> Option<usize> {
        (0..self.rows).rev().find(|&row| self.lines[row].contains(needle))
    }

    /// The cell column where `needle` starts on `row`.
    pub fn column_of(&self, row: usize, needle: &str) -> Option<usize> {
        let mut text = String::new();
        let mut columns = Vec::new();
        for col in 0..self.cols {
            let cell = self.cell(row, col);
            if is_spacer(cell) {
                continue;
            }
            columns.push((text.len(), col));
            text.push(cell.ch);
            if let Some(marks) = &cell.zerowidth {
                text.extend(marks.iter());
            }
        }
        let at = text.find(needle)?;
        columns.iter().rev().find(|(offset, _)| *offset <= at).map(|(_, col)| *col)
    }

    pub fn row_is_blank(&self, row: usize) -> bool {
        self.lines[row].is_empty()
    }

    /// Violations of the wide-character layout: a wide glyph is followed by exactly one spacer, a spacer
    /// follows exactly one wide glyph, and a leading spacer (the blank a wide glyph that did not fit leaves)
    /// is only ever the last cell of a row. Empty when the grid is consistent.
    pub fn wide_cell_problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let cell = self.cell(row, col);
                if cell.flags.contains(Flags::WIDE_CHAR) {
                    let spacer = (col + 1 < self.cols).then(|| self.cell(row, col + 1));
                    if !spacer.is_some_and(|next| next.flags.contains(Flags::WIDE_CHAR_SPACER)) {
                        problems.push(format!("row {row} col {col}: wide glyph {:?} has no spacer after it", cell.ch));
                    }
                }
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                    && (col == 0 || !self.cell(row, col - 1).flags.contains(Flags::WIDE_CHAR))
                {
                    problems.push(format!("row {row} col {col}: spacer without a wide glyph before it"));
                }
                if cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) && col + 1 != self.cols {
                    problems.push(format!("row {row} col {col}: leading spacer is not in the last column"));
                }
            }
        }
        problems
    }

    /// Dump with row numbers and the cursor marked, for failure messages and files.
    pub fn dump(&self) -> String {
        let mut out = format!(
            "{}x{} cursor={:?}{} alt_screen={} history={} offset={}\n",
            self.cols,
            self.rows,
            self.cursor,
            if self.cursor_visible { "" } else { " (hidden)" },
            self.alt_screen,
            self.history_size,
            self.display_offset
        );
        for (row, line) in self.lines.iter().enumerate() {
            out.push_str(&format!("{row:>3}|{line}\n"));
        }
        out
    }

    /// [`Screen::dump`] with account details masked: safe to print.
    pub fn dump_scrubbed(&self) -> String {
        scrub_screen(&self.dump())
    }

    /// Lines passed through `normalize` (volatile text such as clocks and spinners replaced).
    pub fn normalized_lines(&self, normalize: &dyn Fn(&str) -> String) -> Vec<String> {
        self.lines.iter().map(|line| normalize(line).trim_end().to_owned()).collect()
    }

    /// Compares two screens cell by cell. Rows whose normalised text differs are reported once as a row
    /// difference; rows with equal text are then compared for colour and attributes per cell.
    pub fn diff(&self, other: &Screen, normalize: &dyn Fn(&str) -> String) -> Vec<Diff> {
        if (self.cols, self.rows) != (other.cols, other.rows) {
            return vec![Diff::Size { left: (self.cols, self.rows), right: (other.cols, other.rows) }];
        }
        let (left_lines, right_lines) = (self.normalized_lines(normalize), other.normalized_lines(normalize));
        let mut diffs = Vec::new();
        for row in 0..self.rows {
            if left_lines[row] != right_lines[row] {
                diffs.push(Diff::Row { row, left: left_lines[row].clone(), right: right_lines[row].clone() });
                continue;
            }
            // Volatile rows may legitimately differ in colour; only rows whose raw text is equal are compared.
            if self.lines[row] != other.lines[row] {
                continue;
            }
            for col in 0..self.cols {
                let (a, b) = (self.cell(row, col), other.cell(row, col));
                if !same_appearance(a, b) {
                    diffs.push(Diff::Cell { row, col, left: describe_cell(a), right: describe_cell(b) });
                }
            }
        }
        diffs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(line: &str) -> String {
        line.to_owned()
    }

    #[test]
    fn text_drops_trailing_blank_rows_and_finds_columns() {
        let screen = Screen::from_lines(&["  hello world", "", "x", "", ""], 20);
        assert_eq!(screen.text(), "  hello world\n\nx");
        assert_eq!(screen.column_of(0, "world"), Some(8));
        assert_eq!(screen.row_containing("x", 0), Some(2));
        assert_eq!(screen.last_row_containing("hello"), Some(0));
        assert!(!screen.contains("absent"));
    }

    #[test]
    fn identical_screens_have_no_diff_and_different_rows_are_reported() {
        let a = Screen::from_lines(&["one", "two"], 10);
        let b = Screen::from_lines(&["one", "tWo"], 10);
        assert!(a.diff(&a.clone(), &identity).is_empty());
        assert_eq!(a.diff(&b, &identity), vec![Diff::Row { row: 1, left: "two".into(), right: "tWo".into() }]);
        let narrow = Screen::from_lines(&["one", "two"], 8);
        assert!(matches!(a.diff(&narrow, &identity)[0], Diff::Size { .. }));
    }

    #[test]
    fn normalisation_hides_volatile_text() {
        let a = Screen::from_lines(&["clock 12:01", "static"], 20);
        let b = Screen::from_lines(&["clock 23:59", "static"], 20);
        let normalize = |line: &str| line.replace("12:01", "HH:MM").replace("23:59", "HH:MM");
        assert!(a.diff(&b, &normalize).is_empty());
        assert_eq!(a.diff(&b, &identity).len(), 1);
    }

    #[test]
    fn wide_glyphs_need_their_spacer() {
        let mut screen = Screen::from_lines(&["a你 b", "cd"], 6);
        assert_eq!(screen.wide_cell_problems(), Vec::<String>::new(), "no wide flags, nothing to check");
        // cells: a, 你 (wide), its spacer, b
        screen.cells[1].flags.insert(Flags::WIDE_CHAR);
        screen.cells[2].flags.insert(Flags::WIDE_CHAR_SPACER);
        assert_eq!(screen.wide_cell_problems(), Vec::<String>::new());
        // the glyph in the last column cannot have a spacer
        screen.cells[5].flags.insert(Flags::WIDE_CHAR);
        assert_eq!(screen.wide_cell_problems().len(), 1);
        screen.cells[5].flags.remove(Flags::WIDE_CHAR);
        // a spacer on its own
        screen.cells[7].flags.insert(Flags::WIDE_CHAR_SPACER);
        assert!(screen.wide_cell_problems()[0].contains("spacer without a wide glyph"));
        screen.cells[7].flags.remove(Flags::WIDE_CHAR_SPACER);
        // a leading spacer belongs in the last column only
        screen.cells[5].flags.insert(Flags::LEADING_WIDE_CHAR_SPACER);
        assert_eq!(screen.wide_cell_problems(), Vec::<String>::new());
        screen.cells[4].flags.insert(Flags::LEADING_WIDE_CHAR_SPACER);
        assert_eq!(screen.wide_cell_problems().len(), 1);
    }

    #[test]
    fn equal_text_with_a_different_colour_is_a_cell_diff() {
        let a = Screen::from_lines(&["ab"], 4);
        let mut b = a.clone();
        b.cells[1].fg_raw = CellColor::Indexed(1);
        assert!(matches!(a.diff(&b, &identity).as_slice(), [Diff::Cell { row: 0, col: 1, .. }]));
    }

    #[test]
    fn the_foreground_of_a_blank_cell_is_not_a_difference_unless_it_shows() {
        let a = Screen::from_lines(&["a b"], 4);
        let mut b = a.clone();
        b.cells[1].fg_raw = CellColor::Indexed(1);
        assert!(a.diff(&b, &identity).is_empty(), "a blank cell has no visible foreground");
        b.cells[1].flags.insert(Flags::UNDERLINE);
        assert!(!a.diff(&b, &identity).is_empty(), "an underline is visible");
        let mut c = a.clone();
        c.cells[1].bg_raw = CellColor::Indexed(4);
        assert!(!a.diff(&c, &identity).is_empty(), "a background is visible");
    }
}
