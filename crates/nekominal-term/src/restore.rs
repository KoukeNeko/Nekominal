//! The text of a pane kept between runs, as Windows Terminal does since 1.21: the primary buffer is written as text
//! with SGR (colours and attributes, nothing else), and on the next start shown above a separator line before the
//! restored shell writes anything.
//!
//! The saved file is outside the program's control once written, so what is fed back is sanitized: printable text,
//! CR, LF, tab and SGR survive; every other escape sequence (OSC 52 clipboard writes, OSC 8 links, titles, DCS, mode
//! changes, replies the terminal would send to the program) is dropped whole, so nothing of the old session can act again.

use std::fmt::Write as _;

use alacritty_terminal::{
    grid::{Dimensions, Grid, GridCell},
    index::{Column, Line},
    term::cell::{Cell, Flags},
};

use crate::terminal::{CellColor, cell_color};

/// Most rows of a pane that are saved, the newest kept. Windows Terminal saves the whole buffer; a cap keeps a
/// session of many panes from writing (and replaying at start) megabytes per pane.
pub const SAVED_ROWS: usize = 5_000;
/// Most bytes of a pane's saved text; whole lines are dropped from the oldest end to fit.
pub const SAVED_BYTES: usize = 1 << 20;

/// What a cell is drawn with, the part SGR can say.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Style {
    fg: CellColor,
    bg: CellColor,
    flags: Flags,
    underline: Option<CellColor>,
}

const STYLE_FLAGS: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

const PLAIN: Style = Style { fg: CellColor::Default, bg: CellColor::Default, flags: Flags::empty(), underline: None };

fn style_of(cell: &Cell) -> Style {
    Style {
        fg: cell_color(cell.fg),
        bg: cell_color(cell.bg),
        flags: cell.flags & STYLE_FLAGS,
        underline: cell.underline_color().map(cell_color),
    }
}

fn push_color(out: &mut String, color: CellColor, base: u8, bright: u8, extended: u8) {
    match color {
        CellColor::Default => {}
        CellColor::Indexed(i @ 0..=7) => _ = write!(out, ";{}", base + i),
        CellColor::Indexed(i @ 8..=15) => _ = write!(out, ";{}", bright + i - 8),
        CellColor::Indexed(i) => _ = write!(out, ";{extended};5;{i}"),
        CellColor::Rgb(r, g, b) => _ = write!(out, ";{extended};2;{r};{g};{b}"),
    }
}

/// A whole SGR that sets `style` from any state: a reset and then what differs from plain.
fn push_sgr(out: &mut String, style: Style) {
    out.push_str("\x1b[0");
    for (flag, code) in [
        (Flags::BOLD, "1"),
        (Flags::DIM, "2"),
        (Flags::ITALIC, "3"),
        (Flags::UNDERLINE, "4"),
        (Flags::DOUBLE_UNDERLINE, "4:2"),
        (Flags::UNDERCURL, "4:3"),
        (Flags::DOTTED_UNDERLINE, "4:4"),
        (Flags::DASHED_UNDERLINE, "4:5"),
        (Flags::INVERSE, "7"),
        (Flags::HIDDEN, "8"),
        (Flags::STRIKEOUT, "9"),
    ] {
        if style.flags.contains(flag) {
            out.push(';');
            out.push_str(code);
        }
    }
    push_color(out, style.fg, 30, 90, 38);
    push_color(out, style.bg, 40, 100, 48);
    if let Some(color) = style.underline {
        // 58 has no short forms.
        match color {
            CellColor::Default => {}
            CellColor::Indexed(i) => _ = write!(out, ";58;5;{i}"),
            CellColor::Rgb(r, g, b) => _ = write!(out, ";58;2;{r};{g};{b}"),
        }
    }
    out.push('m');
}

/// The rows of `grid` (scrollback and screen) as text with SGR, down to the last row that holds something: at most
/// `max_rows` rows and `max_bytes` bytes, the newest kept. Every line ends in CR LF in the plain style, so lines can be
/// dropped from the front; a row that wrapped continues without a line break, so the text re-wraps at the width it is
/// shown in.
pub(crate) fn grid_text(grid: &Grid<Cell>, max_rows: usize, max_bytes: usize) -> String {
    let top = -(grid.history_size() as i32);
    let Some(bottom) = (top..grid.screen_lines() as i32).rev().find(|line| grid[Line(*line)].into_iter().any(|cell| !cell.is_empty())) else {
        return String::new();
    };
    rows_text(grid, top, bottom, max_rows, max_bytes)
}

/// The rows of `grid` as [`grid_text`] writes them, the screen kept whole, its blank rows included, and no line break
/// after its last row: fed to a terminal of the same size, the screen ends up as it was, row for row (the scrollback
/// above it, at most `max_rows` rows and `max_bytes` bytes of it). With `history` false, the screen alone.
pub(crate) fn screen_text(grid: &Grid<Cell>, history: bool, max_rows: usize, max_bytes: usize) -> String {
    let top = if history { -(grid.history_size() as i32) } else { 0 };
    let screen = grid.screen_lines();
    let mut text = rows_text(grid, top, screen as i32 - 1, max_rows.max(screen), max_bytes);
    if text.ends_with("\r\n") {
        text.truncate(text.len() - 2);
    }
    text
}

fn rows_text(grid: &Grid<Cell>, top: i32, bottom: i32, max_rows: usize, max_bytes: usize) -> String {
    let first = (bottom + 1 - max_rows as i32).max(top);
    let columns = grid.columns();
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut style = PLAIN;
    for index in first..=bottom {
        let row = &grid[Line(index)];
        let wraps = row[Column(columns - 1)].flags.contains(Flags::WRAPLINE);
        // The blanks after the text are not text, unless the row wraps: then they are spaces between words.
        let end = if wraps { columns } else { (0..columns).rev().find(|c| !row[Column(*c)].is_empty()).map_or(0, |c| c + 1) };
        for cell in row.into_iter().take(end) {
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let next = style_of(cell);
            if next != style {
                push_sgr(&mut line, next);
                style = next;
            }
            // A tab left its character in the cell it started from and blanks in the others: replayed, it would move
            // the cursor past those blanks and push the rest of the line right.
            line.push(if cell.c == '\t' { ' ' } else { cell.c });
            if let Some(rest) = cell.zerowidth() {
                line.extend(rest);
            }
        }
        if !wraps {
            if style != PLAIN {
                line.push_str("\x1b[0m");
                style = PLAIN;
            }
            line.push_str("\r\n");
            lines.push(std::mem::take(&mut line));
        }
    }
    if !line.is_empty() {
        line.push_str("\x1b[0m\r\n");
        lines.push(line);
    }
    let mut size = 0;
    let kept = lines.iter().rev().take_while(|line| {
        size += line.len();
        size <= max_bytes
    });
    let start = lines.len() - kept.count();
    lines[start..].concat()
}

/// `text` with only what may be replayed into a terminal: printable characters, CR, LF, tab and SGR (`CSI` digits,
/// `;` and `:` then `m`). Any other escape sequence is dropped whole, its payload included, and so are the other
/// control characters; a sequence the text ends inside of is dropped too.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let introducer = match c {
            '\x1b' => match chars.next() {
                Some('[') => '\u{9b}',
                Some(']') => '\u{9d}',
                Some('P') => '\u{90}',
                Some('X') => '\u{98}',
                Some('^') => '\u{9e}',
                Some('_') => '\u{9f}',
                // ESC, its intermediates and its final byte (ESC c, ESC 7, ESC ( 0 ...).
                Some(mut next) => {
                    while ('\x20'..='\x2f').contains(&next) {
                        match chars.next() {
                            Some(more) => next = more,
                            None => break,
                        }
                    }
                    continue;
                }
                None => continue,
            },
            '\r' | '\n' | '\t' => {
                out.push(c);
                continue;
            }
            c if c.is_control() => c,
            c => {
                out.push(c);
                continue;
            }
        };
        match introducer {
            '\u{9b}' => {
                let mut sequence = String::from("\x1b[");
                let mut sgr = true;
                // Anything else ends a broken sequence, which is not kept, and is read on its own.
                while let Some(&byte) = chars.peek() {
                    match byte {
                        '\x30'..='\x3f' => {
                            sgr &= byte.is_ascii_digit() || byte == ';' || byte == ':';
                            sequence.push(byte);
                        }
                        '\x20'..='\x2f' => sgr = false,
                        '\x40'..='\x7e' => {
                            chars.next();
                            if sgr && byte == 'm' {
                                sequence.push('m');
                                out.push_str(&sequence);
                            }
                            break;
                        }
                        _ => break,
                    }
                    chars.next();
                }
            }
            // OSC, DCS, SOS, PM, APC: a string up to ST (ESC \ or U+009C), BEL ending an OSC too.
            '\u{9d}' | '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => {
                while let Some(byte) = chars.next() {
                    if byte == '\u{9c}' || (byte == '\x07' && introducer == '\u{9d}') {
                        break;
                    }
                    if byte == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Any other control character goes alone.
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_text_and_line_breaks_survive_sanitizing() {
        let text = "\x1b[0;1;38;2;1;2;3;4:3;58;5;9mred\x1b[0m plain\r\n中文 e\u{301}\ttab\n";
        assert_eq!(sanitize(text), text);
    }

    #[test]
    fn every_other_escape_sequence_is_dropped_with_its_payload() {
        for (input, expected) in [
            ("a\x1b]52;c;aGVsbG8=\x07b", "ab"),
            ("a\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\b", "alinkb"),
            ("a\x1b]0;title\x1b\\b", "ab"),
            ("a\x1bP1$qm\x1b\\b", "ab"),
            ("a\x1b_payload\x1b\\b\x1b^pm\x1b\\c\x1bXsos\x1b\\d", "abcd"),
            ("a\x1b[2Jb\x1b[3Jc\x1b[?1049hd\x1b[6ne\x1b[>0cf", "abcdef"),
            ("a\x1b[1 qb\x1b[=5m\x1b[?5mc", "abc"),
            ("a\x1bcb\x1b7c\x1b(0d\x1b#8e", "abcde"),
            ("a\u{9d}52;c;aGk=\u{9c}b\u{9b}2Jc\u{90}q\u{9c}d", "abcd"),
            ("a\x07b\x08c\x00d\x7fe\x05f", "abcdef"),
            ("unterminated \x1b]52;c;aGk=", "unterminated "),
            ("broken \x1b[12\x1b]0;x\x07after", "broken after"),
        ] {
            assert_eq!(sanitize(input), expected, "{input:?}");
        }
    }
}
