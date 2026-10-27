//! Adapter from `nekominal_term::Snapshot` to `nekominal_render::GridView` (seed of nekominal-pane::view).
//!
//! Colours stay symbolic (`CellColor`): the renderer's `Theme` is the only place that resolves them, so
//! INVERSE / HIDDEN / FAINT / bold-is-bright are applied exactly once and `Default` background cells can be
//! drawn through the single pane background rect.

use std::sync::Arc;

use nekominal_render as r;
use nekominal_term::{
    CellColor, SnapCell, Snapshot, TriggerMark,
    alacritty_terminal::{term::cell::Flags, vte::ansi::CursorShape},
};

/// The scrollbar of a pane: only while the view is scrolled up from the bottom, or while the find bar has matches to
/// mark, so that it never covers the last column at the prompt. None without a scrollback.
pub fn scrollbar(snapshot: &Snapshot, finding: bool) -> Option<r::Scrollbar> {
    let total = snapshot.history_size + snapshot.rows;
    if snapshot.history_size == 0 || total == 0 || (snapshot.display_offset == 0 && !finding) {
        return None;
    }
    let top = (snapshot.history_size - snapshot.display_offset.min(snapshot.history_size)) as f32 / total as f32;
    Some(r::Scrollbar { top, height: snapshot.rows as f32 / total as f32, hovered: false })
}

/// The marks of the scrollbar: the prompts, the commands that failed (over their prompts), then the matches of the find
/// (the current one last, over the others), each at the place of its line in the whole buffer.
pub fn scroll_marks(snapshot: &Snapshot, prompts: &[i32], failed: &[i32], matches: &[(i32, bool)]) -> Vec<r::ScrollMark> {
    let total = (snapshot.history_size + snapshot.rows).max(1) as f32;
    let at = |line: i32| (line + snapshot.history_size as i32) as f32 / total;
    let mut marks: Vec<r::ScrollMark> = prompts.iter().map(|line| r::ScrollMark { at: at(*line), kind: r::MarkKind::Prompt }).collect();
    marks.extend(failed.iter().map(|line| r::ScrollMark { at: at(*line), kind: r::MarkKind::Failed }));
    marks.extend(matches.iter().filter(|(_, current)| !current).map(|(line, _)| r::ScrollMark { at: at(*line), kind: r::MarkKind::Match }));
    marks.extend(matches.iter().filter(|(_, current)| *current).map(|(line, _)| r::ScrollMark { at: at(*line), kind: r::MarkKind::CurrentMatch }));
    marks
}

fn color(c: CellColor) -> r::Color {
    match c {
        CellColor::Default => r::Color::Default,
        CellColor::Indexed(i) => r::Color::Indexed(i),
        CellColor::Rgb(red, green, blue) => r::Color::Rgb(red, green, blue),
    }
}

fn flags(f: Flags) -> r::CellFlags {
    let mut out = r::CellFlags(0);
    for (from, to) in [
        (Flags::BOLD, r::CellFlags::BOLD),
        (Flags::ITALIC, r::CellFlags::ITALIC),
        (Flags::DIM, r::CellFlags::FAINT),
        (Flags::INVERSE, r::CellFlags::INVERSE),
        (Flags::HIDDEN, r::CellFlags::HIDDEN),
        (Flags::STRIKEOUT, r::CellFlags::STRIKEOUT),
        (Flags::WIDE_CHAR, r::CellFlags::WIDE),
        (Flags::WIDE_CHAR_SPACER, r::CellFlags::WIDE_SPACER),
    ] {
        if f.contains(from) {
            out = out.with(to);
        }
    }
    let underline = if f.contains(Flags::UNDERCURL) {
        r::Underline::Curly
    } else if f.contains(Flags::DOUBLE_UNDERLINE) {
        r::Underline::Double
    } else if f.contains(Flags::DOTTED_UNDERLINE) {
        r::Underline::Dotted
    } else if f.contains(Flags::DASHED_UNDERLINE) {
        r::Underline::Dashed
    } else if f.contains(Flags::UNDERLINE) {
        r::Underline::Single
    } else {
        r::Underline::None
    };
    out.with_underline(underline)
}

fn render_cell(cell: &SnapCell) -> r::Cell {
    let leading_spacer = cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER);
    let zerowidth = cell.zerowidth.as_deref().filter(|z| !z.is_empty());
    let underline_color = cell.underline_color.map(color);
    let extra = (zerowidth.is_some() || underline_color.is_some()).then(|| {
        Arc::new(r::CellExtra { zerowidth: zerowidth.map(<[char]>::to_vec).unwrap_or_default(), underline_color })
    });
    r::Cell { ch: if leading_spacer { ' ' } else { cell.ch }, fg: color(cell.fg_raw), bg: color(cell.bg_raw), flags: flags(cell.flags), extra }
}

/// Fills `cells` (reused between frames) with the renderer's view of the snapshot.
pub fn fill_cells(snapshot: &Snapshot, cells: &mut Vec<r::Cell>) {
    cells.clear();
    cells.extend(snapshot.cells.iter().map(render_cell));
}

/// Draws the pinned prompt line (`Snapshot::sticky_header`) over the top row of `cells`, which the caller copied so the
/// screen under it is left as it is. Its default background takes `tint`, so it reads as a header and not as a line of
/// the output under it.
pub fn pin_header(cells: &mut [r::Cell], header: &[SnapCell], tint: r::Rgb) {
    for (cell, from) in cells.iter_mut().zip(header) {
        *cell = render_cell(from);
        if matches!(cell.bg, r::Color::Default) {
            cell.bg = r::Color::Rgb(tint.0, tint.1, tint.2);
        }
    }
}

/// Paints the cells the triggers highlight (`Snapshot::trigger_marks`) on their colour, in `cells`, a copy the caller
/// made: the text keeps its own colour.
pub fn mark_triggers(cells: &mut [r::Cell], marks: &[TriggerMark], columns: usize) {
    for mark in marks {
        let row = mark.row * columns;
        for cell in cells.get_mut(row + mark.start..row + mark.end).unwrap_or_default() {
            cell.bg = r::Color::Rgb(mark.color.0, mark.color.1, mark.color.2);
        }
    }
}

/// Fills `spans` (reused between frames) with the selected columns of every visible row. The snapshot holds the
/// selection in grid coordinates, so a selection that is partly in the scrollback shows on the rows in view.
pub fn selection_spans(snapshot: &Snapshot, spans: &mut Vec<r::RowSpan>) {
    spans.clear();
    let Some(range) = snapshot.selection else { return };
    let (first, last) = (range.start.line.0, range.end.line.0);
    for row in 0..snapshot.rows {
        let line = row as i32 - snapshot.display_offset as i32;
        if line < first || line > last {
            continue;
        }
        let (start, end) = if range.is_block {
            (range.start.column.0, range.end.column.0)
        } else {
            (
                if line == first { range.start.column.0 } else { 0 },
                if line == last { range.end.column.0 } else { snapshot.columns.saturating_sub(1) },
            )
        };
        if start <= end {
            spans.push(r::RowSpan { row: row as u16, start: start as u16, end: (end + 1) as u16 });
        }
    }
}

/// Adds the rows of the viewport that a match covers: `start` and `end` are grid points (line, column) with `end`
/// included, as the search gives them; `display_offset` is how far the view is scrolled back.
pub fn match_spans(start: (i32, usize), end: (i32, usize), display_offset: usize, rows: usize, columns: usize, spans: &mut Vec<r::RowSpan>) {
    for row in 0..rows {
        let line = row as i32 - display_offset as i32;
        if line < start.0 || line > end.0 {
            continue;
        }
        let first = if line == start.0 { start.1 } else { 0 };
        let last = if line == end.0 { end.1 } else { columns.saturating_sub(1) };
        if first <= last {
            spans.push(r::RowSpan { row: row as u16, start: first as u16, end: (last + 1) as u16 });
        }
    }
}

pub fn cursor(snapshot: &Snapshot, focused: bool, visible: bool) -> Option<r::CursorView> {
    let (row, col) = snapshot.cursor.position?;
    let shape = match snapshot.cursor.shape {
        CursorShape::Hidden => return None,
        CursorShape::Block | CursorShape::HollowBlock => r::CursorShape::Block,
        CursorShape::Underline => r::CursorShape::Underline,
        CursorShape::Beam => r::CursorShape::Bar,
    };
    let wide = snapshot.cell(row, col).flags.contains(Flags::WIDE_CHAR);
    Some(r::CursorView { col: col as u16, row: row as u16, shape, visible, focused, wide })
}

/// How far an unfocused pane's text colours move toward its background.
const DIM_AMOUNT: f32 = 0.38;

fn toward(color: r::Rgb, target: r::Rgb) -> r::Rgb {
    let channel = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * DIM_AMOUNT).round() as u8;
    r::Rgb(channel(color.0, target.0), channel(color.1, target.1), channel(color.2, target.2))
}

/// The theme of an unfocused pane: foreground and the whole palette pulled toward the background. Cells with
/// their own 24-bit colour keep it, so the shape of the text (not only its colour) must still tell which
/// pane has focus: the shell also draws an accent line on the focused pane.
pub fn dimmed(theme: &r::Theme) -> r::Theme {
    let mut dimmed = theme.clone();
    dimmed.foreground = toward(theme.foreground, theme.background);
    for color in &mut dimmed.ansi {
        *color = toward(*color, theme.background);
    }
    for slot in 16..=255u8 {
        dimmed.indexed_overrides[usize::from(slot) - 16] = Some(toward(theme.indexed(slot), theme.background));
    }
    dimmed
}

/// Applies the program's OSC 4 / 10 / 11 / 12 overrides to a copy of the user's theme.
pub fn themed(base: &r::Theme, snapshot: &Snapshot) -> r::Theme {
    let mut theme = base.clone();
    for &(slot, rgb) in &snapshot.overrides {
        let value = r::Rgb(rgb.r, rgb.g, rgb.b);
        match slot {
            0..=255 => theme.set_indexed(slot as u8, value),
            256 => theme.foreground = value,
            257 => theme.background = value,
            258 => theme.cursor = value,
            _ => {}
        }
    }
    theme
}

#[cfg(test)]
mod tests {
    use nekominal_term::{CellSide, CoreConfig, SelectionKind, TerminalCore};

    use super::*;

    #[test]
    fn the_scrollbar_shows_while_scrolled_up_or_finding_and_marks_sit_where_their_lines_are() {
        let snapshot = |history: usize, offset: usize| Snapshot { history_size: history, display_offset: offset, rows: 10, ..Snapshot::default() };
        assert_eq!(scrollbar(&snapshot(0, 0), true), None, "no scrollback, no scrollbar");
        assert_eq!(scrollbar(&snapshot(90, 0), false), None, "at the bottom and not finding: out of the way");
        let up = scrollbar(&snapshot(90, 90), false).expect("scrolled to the top");
        assert_eq!((up.top, up.height), (0.0, 0.1));
        let bottom = scrollbar(&snapshot(90, 0), true).expect("the find has matches");
        assert!((bottom.top - 0.9).abs() < 1e-6);
        let marks = scroll_marks(&snapshot(90, 0), &[-90, 0], &[-80], &[(-45, false), (5, true)]);
        let places: Vec<(f32, r::MarkKind)> = marks.iter().map(|m| (m.at, m.kind)).collect();
        assert_eq!(places, [(0.0, r::MarkKind::Prompt), (0.9, r::MarkKind::Prompt), (0.1, r::MarkKind::Failed), (0.45, r::MarkKind::Match), (0.95, r::MarkKind::CurrentMatch)]);
    }

    #[test]
    fn a_trigger_mark_paints_the_background_of_its_cells_and_no_others() {
        let mut cells = vec![r::Cell::default(); 2 * 4];
        let marks = [TriggerMark { row: 1, start: 1, end: 3, color: (9, 8, 7) }, TriggerMark { row: 5, start: 0, end: 4, color: (1, 1, 1) }];
        mark_triggers(&mut cells, &marks, 4);
        let painted: Vec<usize> = (0..cells.len()).filter(|i| cells[*i].bg == r::Color::Rgb(9, 8, 7)).collect();
        assert_eq!(painted, [5, 6], "a mark off the grid is skipped");
    }

    fn distance(a: r::Rgb, b: r::Rgb) -> i32 {
        (i32::from(a.0) - i32::from(b.0)).abs()
            + (i32::from(a.1) - i32::from(b.1)).abs()
            + (i32::from(a.2) - i32::from(b.2)).abs()
    }

    #[test]
    fn dimming_pulls_text_colours_toward_the_background_and_keeps_the_background() {
        let theme = r::Theme::campbell();
        let dimmed = dimmed(&theme);
        assert_eq!(dimmed.background, theme.background);
        assert!(distance(dimmed.foreground, theme.background) < distance(theme.foreground, theme.background));
        for slot in 0..=255u8 {
            let (before, after) = (theme.indexed(slot), dimmed.indexed(slot));
            assert!(distance(after, theme.background) <= distance(before, theme.background), "slot {slot}");
        }
    }

    /// (row, start, end) of every selected span of the visible rows.
    fn spans(core: &mut TerminalCore) -> Vec<(u16, u16, u16)> {
        let mut snapshot = Snapshot::default();
        core.snapshot(&mut snapshot);
        let mut spans = Vec::new();
        selection_spans(&snapshot, &mut spans);
        spans.iter().map(|span| (span.row, span.start, span.end)).collect()
    }

    fn select(core: &mut TerminalCore, kind: SelectionKind, from: (usize, usize), to: (usize, usize)) {
        core.selection_start(kind, from.0, from.1, CellSide::Left);
        core.selection_update(to.0, to.1, CellSide::Right);
    }

    #[test]
    fn a_selection_is_drawn_from_its_start_to_the_end_of_the_first_row_whole_rows_between_and_the_end_of_the_last() {
        let mut core = TerminalCore::new(10, 5, CoreConfig::default());
        core.feed(b"aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc");
        assert_eq!(spans(&mut core), []);
        select(&mut core, SelectionKind::Simple, (0, 3), (2, 4));
        assert_eq!(spans(&mut core), [(0, 3, 10), (1, 0, 10), (2, 0, 5)]);
        select(&mut core, SelectionKind::Simple, (1, 2), (1, 6));
        assert_eq!(spans(&mut core), [(1, 2, 7)]);
        select(&mut core, SelectionKind::Block, (0, 2), (2, 4));
        assert_eq!(spans(&mut core), [(0, 2, 5), (1, 2, 5), (2, 2, 5)]);
        select(&mut core, SelectionKind::Lines, (1, 3), (1, 3));
        assert_eq!(spans(&mut core), [(1, 0, 10)]);
        core.selection_clear();
        assert_eq!(spans(&mut core), []);
    }

    #[test]
    fn a_selection_that_reaches_into_the_scrollback_shows_on_the_rows_in_view() {
        let mut core = TerminalCore::new(10, 3, CoreConfig::default());
        for i in 0..8 {
            core.feed(format!("line{i}\r\n").as_bytes());
        }
        // The live screen shows line6, line7 and the empty cursor row. Select the first two, look from further back.
        select(&mut core, SelectionKind::Simple, (0, 0), (1, 4));
        assert_eq!(spans(&mut core), [(0, 0, 10), (1, 0, 5)]);
        core.scroll_display(nekominal_term::alacritty_terminal::grid::Scroll::Delta(1));
        assert_eq!(spans(&mut core), [(1, 0, 10), (2, 0, 5)], "one row further down in the view");
        core.scroll_display(nekominal_term::alacritty_terminal::grid::Scroll::Delta(2));
        assert_eq!(spans(&mut core), [], "scrolled out of view");
        core.scroll_display(nekominal_term::alacritty_terminal::grid::Scroll::Top);
        assert_eq!(spans(&mut core), [], "the lines are not at the top of the buffer either");
    }

    #[test]
    fn the_pinned_header_replaces_the_top_row_of_a_copy_and_nothing_else() {
        let mut core = TerminalCore::new(6, 3, CoreConfig::default());
        core.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07a\r\nb\r\nc\r\nd\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ ");
        core.scroll_display(nekominal_term::alacritty_terminal::grid::Scroll::Delta(1));
        let mut snapshot = Snapshot::default();
        core.snapshot(&mut snapshot);
        let mut cells = Vec::new();
        fill_cells(&snapshot, &mut cells);
        let text = |cells: &[r::Cell], row: usize| cells[row * 6..row * 6 + 6].iter().map(|cell| cell.ch).collect::<String>();
        assert_eq!((text(&cells, 0), text(&cells, 1)), ("b     ".to_owned(), "c     ".to_owned()), "scrolled into the output");

        let tint = r::Rgb(1, 2, 3);
        let mut copy = cells.clone();
        pin_header(&mut copy, &snapshot.sticky_header, tint);
        assert_eq!(text(&copy, 0), "$ ls  ", "the prompt line is drawn over the top row");
        assert!(copy[..6].iter().all(|cell| cell.bg == r::Color::Rgb(1, 2, 3)), "on the tint");
        assert_eq!(copy[6..], cells[6..], "the rows under it are left as they are");
        assert_eq!(text(&cells, 0), "b     ", "and so is the original");
    }

    #[test]
    fn dimming_leaves_the_original_theme_untouched() {
        let theme = r::Theme::campbell();
        let before = theme.clone();
        let _ = dimmed(&theme);
        assert_eq!(theme, before);
    }

    #[test]
    fn a_match_covers_the_rows_in_view_and_a_wrapped_one_runs_to_the_edge() {
        let spans = |start, end, offset| {
            let mut spans = Vec::new();
            match_spans(start, end, offset, 10, 80, &mut spans);
            spans.into_iter().map(|s| (s.row, s.start, s.end)).collect::<Vec<_>>()
        };
        assert_eq!(spans((3, 5), (3, 8), 0), [(3, 5, 9)], "one row, end included");
        assert_eq!(spans((3, 70), (5, 4), 0), [(3, 70, 80), (4, 0, 80), (5, 0, 5)], "a match that wraps");
        assert_eq!(spans((-4, 2), (-4, 6), 6), [(2, 2, 7)], "scrolled back six lines, line -4 is row 2");
        assert!(spans((-4, 2), (-4, 6), 0).is_empty(), "a match in the scrollback is out of view");
        assert_eq!(spans((9, 0), (12, 3), 0), [(9, 0, 80)], "the part below the view is cut");
    }
}
