//! Behavioural tests for `TerminalCore` (alacritty_terminal 0.26.0 + sniffer).

use std::time::{Duration, Instant};

use nekominal_term::{
    alacritty_terminal::{
        grid::Scroll,
        term::{cell::Flags, Osc52, TermMode},
        vte::ansi::{CursorShape, Rgb},
    },
    agent_status::AgentState,
    osc::{Notification, PromptMark},
    CellColor, CellSide, CommandOutput, CoreConfig, CoreEvent, Damage, MouseExtensions, Palette, SelectionKind, Snapshot,
    TerminalCore, Unsupported, sticky_prompt_line,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

fn core(columns: usize, rows: usize) -> TerminalCore {
    TerminalCore::new(columns, rows, CoreConfig::default())
}

fn feed(core: &mut TerminalCore, text: &str) {
    core.feed(text.as_bytes());
}

fn snap(core: &mut TerminalCore) -> Snapshot {
    let mut snapshot = Snapshot::default();
    core.snapshot(&mut snapshot);
    snapshot
}

/// Everything the terminal wants written to the PTY, concatenated.
fn replies(events: &[CoreEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            CoreEvent::PtyWrite(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        })
        .collect()
}

fn rgb(hex: u32) -> Rgb {
    Rgb { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
}

// --- colours and attributes -------------------------------------------------------------------

// The snapshot carries colours as the program set them and flags unapplied; the renderer's `Theme` is the
// only place that resolves bold-is-bright, faint, inverse and hidden (tests in nekominal-render).

#[test]
fn sgr_attributes_leave_bold_unapplied_and_colours_symbolic() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[1;3;4;31mA\x1b[0mB");
    let s = snap(&mut core);
    let a = s.cell(0, 0);
    assert!(a.flags.contains(Flags::BOLD | Flags::ITALIC | Flags::UNDERLINE));
    assert_eq!(a.fg_raw, CellColor::Indexed(1), "bold red stays red here; bright is the renderer's decision");
    let b = s.cell(0, 1);
    assert_eq!(b.flags, Flags::empty());
    assert_eq!((b.fg_raw, b.bg_raw), (CellColor::Default, CellColor::Default));
}

#[test]
fn sixteen_color_sgr_and_bright_range() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[32;44mx\x1b[92;104my");
    let s = snap(&mut core);
    assert_eq!((s.cell(0, 0).fg_raw, s.cell(0, 0).bg_raw), (CellColor::Indexed(2), CellColor::Indexed(4)));
    assert_eq!((s.cell(0, 1).fg_raw, s.cell(0, 1).bg_raw), (CellColor::Indexed(10), CellColor::Indexed(12)));
}

#[test]
fn indexed_256_colors() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[38;5;196;48;5;21mx\x1b[0;38;5;1my\x1b[0;38;5;244mz");
    let s = snap(&mut core);
    assert_eq!((s.cell(0, 0).fg_raw, s.cell(0, 0).bg_raw), (CellColor::Indexed(196), CellColor::Indexed(21)));
    assert_eq!(s.cell(0, 1).fg_raw, CellColor::Indexed(1));
    assert_eq!(s.cell(0, 2).fg_raw, CellColor::Indexed(244));
}

#[test]
fn truecolor_in_semicolon_and_colon_forms() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[38;2;10;20;30;48;2;40;50;60mx\x1b[0;38:2::70:80:90my\x1b[0;38:2:1:2:3mz");
    let s = snap(&mut core);
    assert_eq!(
        (s.cell(0, 0).fg_raw, s.cell(0, 0).bg_raw),
        (CellColor::Rgb(10, 20, 30), CellColor::Rgb(40, 50, 60))
    );
    assert_eq!(s.cell(0, 1).fg_raw, CellColor::Rgb(70, 80, 90), "colon form with empty colour-space id");
    assert_eq!(s.cell(0, 2).fg_raw, CellColor::Rgb(1, 2, 3), "colon form without colour-space id");
}

#[test]
fn dim_inverse_hidden_are_flags_and_underline_colour_is_symbolic() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[2;31mD\x1b[0;7;31;42mI\x1b[0;8;31mH\x1b[0;4;58;2;1;2;3mU\x1b[0;4;58;5;4mV");
    let s = snap(&mut core);
    assert!(s.cell(0, 0).flags.contains(Flags::DIM));
    assert_eq!(s.cell(0, 0).fg_raw, CellColor::Indexed(1), "dim is not applied to the colour");
    assert!(s.cell(0, 1).flags.contains(Flags::INVERSE));
    assert_eq!((s.cell(0, 1).fg_raw, s.cell(0, 1).bg_raw), (CellColor::Indexed(1), CellColor::Indexed(2)), "no swap");
    assert!(s.cell(0, 2).flags.contains(Flags::HIDDEN));
    assert_eq!(s.cell(0, 2).fg_raw, CellColor::Indexed(1), "hidden text keeps its colour");
    assert_eq!(s.cell(0, 3).underline_color, Some(CellColor::Rgb(1, 2, 3)));
    assert_eq!(s.cell(0, 4).underline_color, Some(CellColor::Indexed(4)), "SGR 58;5;n is a palette reference too");
    assert_eq!(s.cell(0, 0).underline_color, None);
}

#[test]
fn osc_4_10_11_override_the_exported_table_and_queries_reply_from_the_palette() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b]4;1;rgb:ff/00/00\x07\x1b[31mx\x1b]11;#102030\x07 ");
    let s = snap(&mut core);
    assert_eq!(s.cell(0, 0).fg_raw, CellColor::Indexed(1), "cells keep referring to the slot");
    assert!(s.overrides.contains(&(1, rgb(0xff0000))));
    assert!(s.overrides.contains(&(257, rgb(0x102030))));
    core.take_events();

    feed(&mut core, "\x1b]11;?\x1b\\\x1b]10;?\x07\x1b]4;1;?\x07");
    let reply = replies(&core.take_events());
    assert_eq!(reply, "\x1b]11;rgb:1010/2020/3030\x1b\\\x1b]10;rgb:cccc/cccc/cccc\x07\x1b]4;1;rgb:ffff/0000/0000\x07");

    feed(&mut core, "\x1b]104\x07\x1b]111\x07");
    assert!(snap(&mut core).overrides.is_empty(), "OSC 104 / 111 restore the defaults");
    feed(&mut core, "\x1b]11;?\x1b\\\x1b]4;1;?\x07");
    let defaults = Palette::default();
    let channel = |v: u8| format!("{v:02x}{v:02x}");
    let (bg, red) = (defaults.background, defaults.ansi[1]);
    assert_eq!(
        replies(&core.take_events()),
        format!(
            "\x1b]11;rgb:{}/{}/{}\x1b\\\x1b]4;1;rgb:{}/{}/{}\x07",
            channel(bg.r), channel(bg.g), channel(bg.b), channel(red.r), channel(red.g), channel(red.b)
        )
    );
}

#[test]
fn queries_answer_with_the_palette_the_host_configured() {
    let mut palette = Palette { background: rgb(0x203040), ..Palette::default() };
    palette.ansi[2] = rgb(0x00aa00);
    let mut core = TerminalCore::new(20, 3, CoreConfig { palette, ..CoreConfig::default() });
    feed(&mut core, "\x1b]11;?\x07\x1b]4;2;?\x07\x1b]4;200;?\x07");
    assert_eq!(
        replies(&core.take_events()),
        "\x1b]11;rgb:2020/3030/4040\x07\x1b]4;2;rgb:0000/aaaa/0000\x07\x1b]4;200;rgb:ffff/0000/d7d7\x07"
    );
}

// --- width model ------------------------------------------------------------------------------

#[test]
fn wide_cjk_uses_two_cells_with_a_spacer() {
    let mut core = core(10, 3);
    feed(&mut core, "你好a");
    let s = snap(&mut core);
    assert!(s.cell(0, 0).flags.contains(Flags::WIDE_CHAR));
    assert!(s.cell(0, 1).flags.contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(s.cell(0, 2).ch, '好');
    assert_eq!(s.cell(0, 4).ch, 'a');
    assert_eq!(s.row_text(0), "你好a");
    assert_eq!(core.cursor_cell(), (0, 5));
}

#[test]
fn wide_char_that_does_not_fit_wraps_with_a_leading_spacer() {
    let mut core = core(5, 3);
    feed(&mut core, "abcd你");
    let s = snap(&mut core);
    assert!(s.cell(0, 4).flags.contains(Flags::LEADING_WIDE_CHAR_SPACER));
    assert_eq!(s.row_text(0), "abcd");
    assert_eq!(s.cell(1, 0).ch, '你');
    assert_eq!(core.cursor_cell(), (1, 2));
}

#[test]
fn emoji_zwj_sequence_is_one_wide_cell_with_the_rest_attached() {
    let mut core = core(20, 3);
    feed(&mut core, "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}x");
    let s = snap(&mut core);
    // Two columns, as Claude Code (string-width) and ConPTY's grapheme mode count it; alacritty alone makes six.
    assert_eq!(core.cursor_cell(), (0, 3));
    assert!(s.cell(0, 0).flags.contains(Flags::WIDE_CHAR));
    assert_eq!(s.cell(0, 0).zerowidth.as_deref(), Some(&['\u{200d}', '\u{1f469}', '\u{200d}', '\u{1f467}'][..]));
    assert_eq!(s.cell(0, 2).ch, 'x');
    assert_eq!(s.row_text(0), "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}x");
}

#[test]
fn a_cluster_that_fills_the_last_columns_does_not_wrap_the_row() {
    // A row a program draws to the right edge: had it wrapped, every row of the frame below it would move down.
    let mut core = core(10, 3);
    feed(&mut core, "aaaaaaaa\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}");
    let s = snap(&mut core);
    assert_eq!(s.row_text(0), "aaaaaaaa\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}");
    assert_eq!(s.row_text(1), "");
    feed(&mut core, "\r\n\u{1f1f9}\u{1f1fc}\u{1f44d}\u{1f3fd}|");
    assert_eq!(snap(&mut core).row_text(1), "\u{1f1f9}\u{1f1fc}\u{1f44d}\u{1f3fd}|");
    assert_eq!(core.cursor_cell(), (1, 5), "a flag and a toned thumb are two columns each");
}

#[test]
fn a_cluster_ends_when_the_program_does_anything_but_print() {
    let mut core = core(20, 3);
    // The second regional indicator after a cursor move starts its own cell instead of joining the first.
    feed(&mut core, "\u{1f1f9}\x1b[5G\u{1f1fc}");
    let s = snap(&mut core);
    assert_eq!(s.cell(0, 0).zerowidth, None);
    assert_eq!(s.cell(0, 4).ch, '\u{1f1fc}');
}

#[test]
fn variation_selector_16_makes_a_text_emoji_wide() {
    let mut core = core(20, 3);
    feed(&mut core, "\u{2764}\u{fe0f}x\u{2764}y");
    let s = snap(&mut core);
    assert_eq!(s.cell(0, 0).ch, '\u{2764}');
    assert!(s.cell(0, 0).flags.contains(Flags::WIDE_CHAR));
    assert_eq!(s.cell(0, 0).zerowidth.as_deref(), Some(&['\u{fe0f}'][..]));
    assert!(s.cell(0, 1).flags.contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(s.cell(0, 2).ch, 'x');
    assert_eq!(s.cell(0, 4).ch, 'y', "without the selector the heart stays narrow");
    // The string width the programs count; alacritty alone sums the code points to 1.
    assert_eq!(unicode_width::UnicodeWidthStr::width("\u{2764}\u{fe0f}"), 2);
    assert_eq!('\u{2764}'.width(), Some(1));
}

#[test]
fn a_flood_of_combining_marks_is_capped_and_takes_no_time() {
    let mut core = core(20, 3);
    let flood = format!("e{}x", "\u{0301}".repeat(100_000));
    let started = std::time::Instant::now();
    feed(&mut core, &flood);
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "took {:?}", started.elapsed());
    let s = snap(&mut core);
    assert_eq!(s.cell(0, 0).ch, 'e', "the cell keeps its base character");
    assert!(s.cell(0, 0).zerowidth.as_ref().is_some_and(|marks| (1..64).contains(&marks.len())));
    assert_eq!(s.cell(0, 1).ch, 'x', "the text after the marks goes on in the next cell");
}

#[test]
fn a_flood_of_combining_marks_after_an_sgr_a_carriage_return_or_a_cursor_move_is_capped_too() {
    // Each of these ends the cluster, so the marks went straight to alacritty, which stacked them all on one cell.
    for prefix in ["e\x1b[0m", "e\r", "e\x1b[D", "e\x1b[1;2H"] {
        let mut core = core(20, 3);
        let flood = format!("{prefix}{}", "\u{0301}".repeat(100_000));
        let started = std::time::Instant::now();
        feed(&mut core, &flood);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "{prefix:?} took {:?}", started.elapsed());
        let s = snap(&mut core);
        assert_eq!(s.cell(0, 0).ch, 'e', "{prefix:?}");
        let marks = s.cell(0, 0).zerowidth.as_ref().map_or(0, |marks| marks.len());
        assert!((1..=16).contains(&marks), "{prefix:?} left {marks} marks on the cell");
    }
}

#[test]
fn a_flood_of_spacing_marks_or_skin_tones_is_capped_too() {
    // These have a width, so the cap on zero-width characters alone let them join forever.
    for (base, mark) in [('\u{0915}', '\u{0903}'), ('\u{1f44d}', '\u{1f3fb}')] {
        let mut core = core(200, 70);
        let flood = format!("{base}{}", mark.to_string().repeat(100_000));
        let started = std::time::Instant::now();
        feed(&mut core, &flood);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "took {:?}", started.elapsed());
        let s = snap(&mut core);
        assert_eq!(s.cell(0, 0).ch, base);
        assert!(s.cell(0, 0).zerowidth.as_ref().is_some_and(|marks| (1..64).contains(&marks.len())));
    }
}

#[test]
fn ambiguous_width_is_always_narrow_and_combining_marks_attach() {
    let mut core = core(20, 3);
    feed(&mut core, "\u{2500}\u{00b0}e\u{0301}x");
    let s = snap(&mut core);
    assert_eq!(s.cell(0, 0).ch, '\u{2500}');
    assert_eq!(s.cell(0, 1).ch, '\u{00b0}');
    assert_eq!(s.cell(0, 2).ch, 'e');
    assert_eq!(s.cell(0, 2).zerowidth.as_deref(), Some(&['\u{0301}'][..]));
    assert_eq!(s.cell(0, 3).ch, 'x');
    assert_eq!('\u{2500}'.width(), Some(1));
    assert_eq!(UnicodeWidthChar::width_cjk('\u{2500}'), Some(2), "the CJK table would say 2");
}

#[test]
fn cursor_advance_always_equals_the_string_width_of_each_grapheme() {
    let samples = [
        "plain ascii",
        "你好，世界",
        "ｱｲｳ halfwidth",
        "０１２ fullwidth digits",
        "한국어 テスト",
        "\u{1f600}\u{1f4a9}\u{1f680}",
        "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}",
        "\u{1f44d}\u{1f3fd}",         // thumbs up + skin tone modifier
        "\u{1f1f9}\u{1f1fc}",         // regional indicators: Taiwan flag
        "1\u{fe0f}\u{20e3}",          // keycap
        "\u{2600}\u{fe0f}\u{26a1}",   // text-default emoji with VS16, emoji-default
        "e\u{0301}a\u{0308}",
        "──┌┐└┘│",
    ];
    for sample in samples {
        let mut core = core(80, 3);
        feed(&mut core, sample);
        let expected: usize = sample.graphemes(true).map(|g| unicode_width::UnicodeWidthStr::width(g).min(2)).sum();
        assert_eq!(core.cursor_cell(), (0, expected), "{sample:?}");
    }
}

// --- screens, scrollback, resize --------------------------------------------------------------

#[test]
fn alternate_screen_is_separate_and_restores_cursor() {
    let mut core = core(20, 5);
    feed(&mut core, "abc");
    feed(&mut core, "\x1b[?1049h");
    assert!(core.mode().contains(TermMode::ALT_SCREEN));
    assert_eq!(snap(&mut core).text(), "");
    feed(&mut core, "\x1b[4;9HALT");
    feed(&mut core, "\x1b[?1049l");
    assert!(!core.mode().contains(TermMode::ALT_SCREEN));
    assert_eq!(snap(&mut core).text(), "abc");
    assert_eq!(core.cursor_cell(), (0, 3));
    feed(&mut core, "\x1b[?1049h");
    assert_eq!(snap(&mut core).text(), "", "the alternate screen starts blank on every entry");
}

#[test]
fn alternate_screen_has_no_scrollback() {
    let mut core = core(20, 5);
    feed(&mut core, "\x1b[?1049h");
    for i in 0..100 {
        feed(&mut core, &format!("alt {i}\r\n"));
    }
    assert_eq!(core.history_size(), 0);
    feed(&mut core, "\x1b[?1049l");
    assert_eq!(core.history_size(), 0);
}

#[test]
fn scrollback_display_offset_and_stationary_viewport() {
    let mut core = core(20, 10);
    for i in 0..100 {
        feed(&mut core, &format!("line{i}\r\n"));
    }
    assert_eq!(core.history_size(), 91);
    assert_eq!(snap(&mut core).row_text(0), "line91");

    core.scroll_display(Scroll::Delta(5));
    let s = snap(&mut core);
    assert_eq!((s.display_offset, s.row_text(0), s.row_text(9).as_str()), (5, "line86".to_owned(), "line95"));
    assert_eq!(s.cursor.position, None, "the cursor row is below the viewport");

    // New output must not move what the user is reading.
    for _ in 0..3 {
        feed(&mut core, "more\r\n");
    }
    let s = snap(&mut core);
    assert_eq!((s.display_offset, s.row_text(0).as_str()), (8, "line86"));

    core.scroll_display(Scroll::Top);
    assert_eq!(core.display_offset(), core.history_size());
    core.scroll_display(Scroll::Bottom);
    assert_eq!(core.display_offset(), 0);
}

#[test]
fn scrollback_is_capped_by_the_config() {
    let mut core = TerminalCore::new(20, 5, CoreConfig { scrollback_lines: 50, ..CoreConfig::default() });
    for i in 0..200 {
        feed(&mut core, &format!("l{i}\r\n"));
    }
    assert_eq!(core.history_size(), 50);
    core.scroll_display(Scroll::Top);
    assert_eq!(snap(&mut core).row_text(0), "l146", "the oldest surviving line");
}

#[test]
fn resize_reflows_wrapped_lines_on_the_primary_screen() {
    // A full screen: nothing under the cursor is blank, so the rows that wrapping adds have no place to go.
    let mut core = core(10, 5);
    feed(&mut core, "0123456789abcde\r\nline3\r\nline4\r\nline5");
    assert_eq!(snap(&mut core).text(), "0123456789\nabcde\nline3\nline4\nline5");
    core.resize(5, 5);
    // Six rows do not fit in five lines: the top row moves into history.
    assert_eq!((core.history_size(), snap(&mut core).text()), (1, "56789\nabcde\nline3\nline4\nline5".to_owned()));
    core.scroll_display(Scroll::Top);
    assert_eq!(snap(&mut core).row_text(0), "01234");
    core.scroll_display(Scroll::Bottom);
    core.resize(20, 5);
    assert_eq!((core.history_size(), snap(&mut core).text()), (0, "0123456789abcde\nline3\nline4\nline5".to_owned()));
    // the alternate screen does not reflow
    feed(&mut core, "\x1b[?1049h\x1b[H0123456789abcdefghij");
    core.resize(10, 5);
    assert_eq!(snap(&mut core).text(), "0123456789", "alt screen content is cut, not reflowed");
}

/// A screen with `rows` rows holding three full-width rows (A, B, C) and a prompt, so the lines wrap when it narrows.
fn banner_and_prompt(columns: usize, rows: usize) -> TerminalCore {
    let mut core = core(columns, rows);
    let row = |ch: char| ch.to_string().repeat(columns);
    feed(&mut core, &format!("{}\r\n{}\r\n{}\r\n$ ", row('A'), row('B'), row('C')));
    core
}

#[test]
fn narrowing_gives_the_rows_that_wrapping_adds_to_the_blank_rows_under_the_cursor() {
    let mut core = banner_and_prompt(20, 10);
    assert_eq!(core.history_size(), 0);
    core.resize(10, 10);
    assert_eq!(core.history_size(), 0, "six blank rows took the three rows that wrapping added");
    assert_eq!(
        snap(&mut core).text(),
        "AAAAAAAAAA\nAAAAAAAAAA\nBBBBBBBBBB\nBBBBBBBBBB\nCCCCCCCCCC\nCCCCCCCCCC\n$"
    );
    assert_eq!(core.cursor_cell(), (6, 2), "the cursor is still after the prompt");
    // Wider again: the wrapped lines join and nothing is left in the scrollback.
    core.resize(20, 10);
    assert_eq!(core.history_size(), 0);
    assert_eq!(snap(&mut core).text(), format!("{}\n{}\n{}\n$", "A".repeat(20), "B".repeat(20), "C".repeat(20)));
}

#[test]
fn blank_rows_that_are_too_few_take_what_they_can_and_the_rest_goes_into_the_scrollback() {
    let mut core = banner_and_prompt(20, 6);
    core.resize(10, 6);
    // Seven rows of content in six lines: one row has to go, and no blank row is left.
    assert_eq!(core.history_size(), 1);
    assert_eq!(snap(&mut core).text(), "AAAAAAAAAA\nBBBBBBBBBB\nBBBBBBBBBB\nCCCCCCCCCC\nCCCCCCCCCC\n$");
    assert_eq!(core.cursor_cell(), (5, 2));
    core.scroll_display(Scroll::Top);
    assert_eq!(snap(&mut core).row_text(0), "AAAAAAAAAA", "the first row of A is what went");
}

#[test]
fn a_full_screen_has_no_blank_row_to_take_anything_so_the_top_goes_into_the_scrollback() {
    let mut core = core(20, 5);
    for ch in ['A', 'B', 'C', 'D'] {
        feed(&mut core, &format!("{}\r\n", ch.to_string().repeat(20)));
    }
    feed(&mut core, "$ ");
    core.resize(10, 5);
    assert_eq!(core.history_size(), 4, "nine rows of content in five lines");
    assert_eq!(snap(&mut core).text(), "CCCCCCCCCC\nCCCCCCCCCC\nDDDDDDDDDD\nDDDDDDDDDD\n$");
}

#[test]
fn a_row_with_a_colour_is_not_blank_and_stays_where_it_is() {
    // A painted bar on the last row: it cannot be dropped, so no blank row is under the cursor to take anything.
    let mut core = banner_and_prompt(20, 8);
    feed(&mut core, "\x1b[8;1H\x1b[44m   \x1b[0m\x1b[4;3H");
    core.resize(10, 8);
    assert_eq!(core.history_size(), 3, "the usual three rows went into the scrollback");
    assert_ne!(snap(&mut core).cell(7, 0).bg_raw, CellColor::Default, "and the bar is still on the last row");

    // The same bar three rows up: the blank rows under it are dropped, the bar ends on the last row.
    let mut core = banner_and_prompt(20, 8);
    feed(&mut core, "\x1b[5;1H\x1b[44m   \x1b[0m\x1b[4;3H");
    core.resize(10, 8);
    assert_eq!(core.history_size(), 0);
    assert_ne!(snap(&mut core).cell(7, 0).bg_raw, CellColor::Default);
    assert_eq!(snap(&mut core).row_text(0), "AAAAAAAAAA");
}
#[test]
fn changing_only_the_rows_is_what_it_was() {
    // Blank rows under the cursor are dropped and added back without touching the scrollback.
    let mut core = banner_and_prompt(20, 10);
    core.resize(20, 6);
    assert_eq!((core.history_size(), core.cursor_cell()), (0, (3, 2)));
    core.resize(20, 10);
    assert_eq!((core.history_size(), core.cursor_cell()), (0, (3, 2)));
    // With the cursor on the last row, fewer rows push lines up and more rows pull them back.
    let mut core = core_with_five_rows();
    core.resize(20, 3);
    assert_eq!(core.history_size(), 2);
    assert_eq!(snap(&mut core).text(), "CCCCCCCCCCCCCCCCCCCC\nDDDDDDDDDDDDDDDDDDDD\n$");
    core.resize(20, 5);
    assert_eq!(core.history_size(), 0);
    assert_eq!(snap(&mut core).row_text(0), "A".repeat(20));
}

fn core_with_five_rows() -> TerminalCore {
    let mut core = core(20, 5);
    for ch in ['A', 'B', 'C', 'D'] {
        feed(&mut core, &format!("{}\r\n", ch.to_string().repeat(20)));
    }
    feed(&mut core, "$ ");
    core
}

#[test]
fn the_alternate_screen_is_not_reflowed_and_the_hidden_primary_screen_is_reflowed_like_a_shown_one() {
    let mut core = banner_and_prompt(20, 8);
    feed(&mut core, "\x1b[?1049h\x1b[HABCDEFGHIJKLMNOPQRST\x1b[3;4HX");
    assert_eq!(core.cursor_cell(), (2, 4));
    core.resize(10, 8);
    assert!(core.mode().contains(TermMode::ALT_SCREEN), "still on the alternate screen");
    assert_eq!(core.history_size(), 0, "the alternate screen has no scrollback");
    let alternate = snap(&mut core);
    assert_eq!(alternate.row_text(0), "ABCDEFGHIJ", "cut, not wrapped");
    assert_eq!(alternate.row_text(2), "   X", "what the program drew where it drew it");
    assert_eq!(core.cursor_cell(), (2, 4), "and its cursor");
    // The primary screen under it was reflowed with the blank rows taking the new rows, as if it were shown.
    feed(&mut core, "\x1b[?1049l");
    assert_eq!(core.history_size(), 0);
    assert_eq!(
        snap(&mut core).text(),
        "AAAAAAAAAA\nAAAAAAAAAA\nBBBBBBBBBB\nBBBBBBBBBB\nCCCCCCCCCC\nCCCCCCCCCC\n$"
    );
    assert_eq!(core.cursor_cell(), (6, 2), "the cursor is back after the prompt, which moved with the reflow");
}

#[test]
fn growing_wider_or_resizing_to_the_same_size_leaves_the_screen_and_the_scrollback_alone() {
    let mut core = banner_and_prompt(20, 10);
    core.resize(20, 10);
    assert_eq!(snap(&mut core).text(), format!("{}\n{}\n{}\n$", "A".repeat(20), "B".repeat(20), "C".repeat(20)));
    core.resize(30, 10);
    assert_eq!(core.history_size(), 0);
    let mut core = core_with_five_rows();
    core.resize(10, 5);
    let history = core.history_size();
    assert!(history > 0);
    core.resize(10, 5);
    assert_eq!(core.history_size(), history, "the same size changes nothing");
}

/// What a program that redraws its whole screen from the top left draws at `width`: a banner, a blank row, an input
/// box between two rules as wide as the screen, a status line, and the cursor in the box.
fn classic_screen(width: usize) -> String {
    let rule = "─".repeat(width);
    format!(
        "\x1b[H\x1b[J ✻ Welcome to the agent\r\n   /help for help, /status for your current setup\r\n   cwd: C:\\work\r\n\r\n\
         {rule}\r\n> \r\n{rule}\r\n  ? for shortcuts\x1b[6;3H"
    )
}

#[test]
fn a_program_that_redraws_after_every_resize_leaves_no_old_screen_in_the_scrollback() {
    let mut core = core(100, 30);
    feed(&mut core, &classic_screen(100));
    // The last three steps narrow and shorten at once (a corner drag); a cut that spent the blank rows before they
    // could take the wrapping would leave old rows in the scrollback there.
    let steps = [(80, 24), (60, 20), (40, 30), (100, 30), (50, 15), (100, 30), (30, 12), (24, 9), (100, 30)];
    for (columns, rows) in steps {
        core.resize(columns, rows);
        feed(&mut core, &classic_screen(columns));
        let mut fresh = self::core(columns, rows);
        feed(&mut fresh, &classic_screen(columns));
        assert_eq!(core.history_size(), fresh.history_size(), "{columns}x{rows}: the scrollback of a fresh process");
        assert_eq!(snap(&mut core).text(), snap(&mut fresh).text(), "{columns}x{rows}: the same screen");
        assert_eq!(core.cursor_cell(), fresh.cursor_cell(), "{columns}x{rows}: the cursor");
    }
    assert_eq!(core.history_size(), 0);
}

/// The scrollback as text, one line per row.
fn scrollback_text(core: &TerminalCore) -> String {
    let history = core.history_size() as i32;
    core.text_between((-history, 0), (-1, core.columns() - 1))
}

#[test]
fn narrowing_a_cleared_screen_does_not_pull_scrollback_back_onto_it() {
    let mut core = core(20, 8);
    for i in 0..14 {
        feed(&mut core, &format!("L{i:02}-{}\r\n", "x".repeat(26)));
    }
    // Ctrl+L: the screen goes into the scrollback and the prompt starts at the top.
    feed(&mut core, "\x1b[H\x1b[2J$ ");
    core.resize(10, 8);
    assert_eq!(core.cursor_cell(), (0, 2), "the prompt stays where it was drawn");
    assert_eq!(snap(&mut core).text(), "$");
}

#[test]
fn narrowing_gives_back_only_the_rows_the_screen_itself_pushed() {
    // The scrollback holds "  ab" (two rows at two columns) and "cd"; the screen row "xf" does not wrap.
    let mut core = core(4, 2);
    feed(&mut core, "\x1b[2;3Hab\r\ncd\r\nef\r\n\x1b[1;1Hx");
    core.resize(2, 2);
    assert_eq!(core.cursor_cell(), (0, 1));
    assert_eq!(snap(&mut core).text(), "xf");
    assert_eq!(core.history_size(), 4, "the rows of the old scrollback wrap on their own");
}

#[test]
fn narrowing_does_not_count_the_wrapping_of_old_scrollback_as_rows_the_screen_gave_up() {
    // Scrollback lines as wide as the old screen wrap into two history rows each when it narrows.
    let mut core = core(20, 10);
    for i in 0..12 {
        feed(&mut core, &format!("{i:#>20}\r\n"));
    }
    feed(&mut core, "\x1b[H\x1b[JAAAAAAAAAAAAAAAAAAAA\r\nBBBBBBBBBBBBBBBBBBBB\r\n$ ");
    core.resize(10, 10);
    assert_eq!(snap(&mut core).text(), "AAAAAAAAAA\nAAAAAAAAAA\nBBBBBBBBBB\nBBBBBBBBBB\n$");
    assert_eq!(core.cursor_cell(), (4, 2));
}

#[test]
fn narrowing_takes_only_the_rows_wrapping_added_and_leaves_unrelated_scrollback_alone() {
    // A shell session has scrollback and the program then redraws from the top left. Giving back more blank rows than
    // wrapping added would pull old scrollback lines onto the screen, where the next redraw overwrites them.
    let mut core = core(20, 10);
    for i in 0..40 {
        feed(&mut core, &format!("h{i}\r\n"));
    }
    feed(&mut core, &format!("\x1b[H\x1b[J{}\r\n{}\r\n{}\r\n$ ", "A".repeat(20), "B".repeat(20), "C".repeat(20)));
    let history = core.history_size();
    assert!(history > 0);
    core.resize(10, 10);
    assert_eq!(core.history_size(), history, "no old line comes back onto the screen");
    assert_eq!(snap(&mut core).text(), "AAAAAAAAAA\nAAAAAAAAAA\nBBBBBBBBBB\nBBBBBBBBBB\nCCCCCCCCCC\nCCCCCCCCCC\n$");
}

#[test]
fn a_redraw_from_the_top_left_after_narrowing_keeps_the_shell_scrollback() {
    let mut core = core(100, 30);
    for i in 0..40 {
        feed(&mut core, &format!("S{i:02}-{}\r\n", "x".repeat(76)));
    }
    feed(&mut core, &classic_screen(100));
    let shell_lines = |core: &TerminalCore| scrollback_text(core).lines().filter(|line| line.starts_with('S')).count();
    let before = shell_lines(&core);
    assert!(before > 0);
    core.resize(60, 30);
    feed(&mut core, &classic_screen(60));
    assert_eq!(shell_lines(&core), before, "the redraw erases the screen, not what the shell printed before it");
    let everything = format!("{}\n{}", scrollback_text(&core), snap(&mut core).text());
    assert_eq!(everything.lines().filter(|line| line.contains("Welcome")).count(), 1, "one banner, not two");
}

#[test]
fn painted_rows_under_the_cursor_that_a_shorter_screen_cuts_do_not_push_the_top_rows_into_the_scrollback() {
    // Every row is painted, so no blank row can take anything, and the screen also gets shorter.
    let mut core = core(9, 5);
    feed(&mut core, "\x1b[44m\x1b[2J");
    core.resize(8, 4);
    assert!(core.history_size() <= 5, "no more than alacritty alone leaves");
}

#[test]
fn text_above_painted_rows_stays_on_the_screen_when_the_screen_narrows_and_shortens() {
    let mut core = core(15, 17);
    feed(&mut core, "\x1b[41m\x1b[3Sh");
    core.resize(4, 14);
    assert_eq!(core.history_size(), 3, "only the three rows the scroll pushed");
    assert_eq!(snap(&mut core).row_text(0), "h", "the text the cursor is on is not pushed away");
}

#[test]
fn the_hidden_primary_screen_ends_up_as_if_it_had_been_shown() {
    let script = format!("{}\r\n{}\r\n{}\r\n$ ", "A".repeat(60), "B".repeat(60), "C".repeat(60));
    for sizes in [vec![(30, 20)], vec![(45, 20), (20, 20)], vec![(30, 6)], vec![(10, 4), (3, 20), (50, 20)], vec![(30, 40)]] {
        let mut shown = core(60, 20);
        let mut hidden = core(60, 20);
        feed(&mut shown, &script);
        feed(&mut hidden, &script);
        feed(&mut hidden, "\x1b[?1049h\x1b[2Jfullscreen");
        for &(columns, rows) in &sizes {
            shown.resize(columns, rows);
            hidden.resize(columns, rows);
        }
        feed(&mut hidden, "\x1b[?1049l");
        assert_eq!(
            (hidden.history_size(), hidden.cursor_cell(), snap(&mut hidden).text()),
            (shown.history_size(), shown.cursor_cell(), snap(&mut shown).text()),
            "{sizes:?}"
        );
    }
}

#[test]
fn kitty_flags_set_with_equals_survive_a_narrowing_resize_on_the_alternate_screen() {
    let mut core = core(40, 10);
    feed(&mut core, "\x1b[>5u\x1b[?1049h\x1b[>1u\x1b[=8;2u");
    assert_eq!(core.kitty_flags(), 9);
    core.resize(30, 10);
    assert_eq!(core.kitty_flags(), 9, "union-set bits survive");
    feed(&mut core, "\x1b[=1;3u");
    assert_eq!(core.kitty_flags(), 8, "difference clears bit 1");
    core.resize(20, 10);
    assert_eq!(core.kitty_flags(), 8, "a cleared bit does not come back from the stack");
    feed(&mut core, "\x1b[?1049l");
    assert_eq!(core.kitty_flags(), 5, "the main screen's stack is untouched");
}

#[test]
fn a_character_cut_in_two_by_the_end_of_a_read_is_parsed_whole() {
    let text = "аб 你好😀 e\u{301}0\u{323}x\x1b[31mred\x1b[0m 世界!";
    let mut whole = core(40, 3);
    feed(&mut whole, text);
    for cut in 1..text.len() {
        let mut split = core(40, 3);
        split.feed(&text.as_bytes()[..cut]);
        split.feed(&text.as_bytes()[cut..]);
        assert_eq!(snap(&mut split).text(), snap(&mut whole).text(), "cut at byte {cut}");
        assert_eq!(split.cursor_cell(), whole.cursor_cell(), "cut at byte {cut}");
    }
    // A feed that is cut inside a character in more than two pieces, and a character at the 1024-byte chunk edge.
    let mut pieces = core(40, 3);
    for byte in "你好".as_bytes() {
        pieces.feed(&[*byte]);
    }
    assert_eq!(snap(&mut pieces).text(), "你好");
    let mut long = core(2000, 2);
    feed(&mut long, &format!("{}e\u{301}0\u{323}x", "x".repeat(1022)));
    assert_eq!(long.cursor_cell(), (0, 1025), "no character is lost where the feed is cut into chunks");
}

#[test]
fn resizing_while_scrolled_back_keeps_the_view_valid() {
    let mut core = core(40, 6);
    for i in 0..60 {
        feed(&mut core, &format!("line {i} {}\r\n", "x".repeat(30)));
    }
    core.scroll_display(Scroll::Delta(20));
    for (columns, rows) in [(20, 6), (20, 3), (60, 8), (10, 10), (40, 6)] {
        core.resize(columns, rows);
        assert!(core.display_offset() <= core.history_size(), "{columns}x{rows}");
        let s = snap(&mut core);
        assert_eq!((s.columns, s.rows, s.cells.len()), (columns, rows, columns * rows));
    }
}
// --- selection and search ---------------------------------------------------------------------

/// A prompt, a command typed at it, what it wrote and how it ended, as the shell integration marks them.
fn command(typed: &str, output: &str, exit: i64) -> String {
    format!("\x1b]133;A\x07PS> \x1b]133;B\x07{typed}\r\n\x1b]133;C\x07{output}\x1b]133;D;{exit}\x07")
}

#[test]
fn the_output_of_each_command_is_selected_and_copied_by_its_marks_newest_first() {
    let mut core = core(30, 12);
    feed(&mut core, &command("ls", "a.txt\r\nb.txt\r\n", 0));
    feed(&mut core, &command("cat nope", "not found\r\n", 1));
    feed(&mut core, &command("cls", "", 0));
    feed(&mut core, "\x1b]133;A\x07PS> \x1b]133;B\x07");
    let outputs = core.command_outputs();
    assert_eq!(outputs.len(), 2, "a command that wrote nothing has no output: {outputs:?}");
    assert_eq!((outputs[0].first, outputs[0].last, outputs[0].exit), (1, 2, Some(0)));
    assert_eq!((outputs[1].first, outputs[1].last, outputs[1].exit), (4, 4, Some(1)));
    assert_eq!(core.last_command_output().as_deref(), Some("not found"));
    feed(&mut core, &command("exit 2", "", 2));
    assert_eq!(core.failed_command_lines(), [4, 7], "a failure that printed nothing is marked too");
    // Each press goes one command further back, and the oldest stays.
    assert!(core.select_command_output());
    assert_eq!(core.selection_text().as_deref(), Some("not found"));
    assert!(core.select_command_output());
    assert_eq!(core.selection_text().as_deref(), Some("a.txt\nb.txt"));
    assert!(core.select_command_output());
    assert_eq!(core.selection_text().as_deref(), Some("a.txt\nb.txt"));
    // A command still running reaches the cursor; a shell without the marks has nothing to select.
    feed(&mut core, "sleep\r\n\x1b]133;C\x07tick 1\r\ntick 2");
    let running = *core.command_outputs().last().unwrap();
    assert!(running.running && running.exit.is_none());
    assert_eq!(core.last_command_output().as_deref(), Some("tick 1\ntick 2"));
    let mut plain = self::core(30, 5);
    feed(&mut plain, "PS> ls\r\na.txt\r\nPS> ");
    assert!(!plain.select_command_output() && plain.last_command_output().is_none());
}

#[test]
fn the_command_line_is_read_from_the_prompt_marks_when_its_output_starts() {
    let mut core = core(30, 12);
    assert_eq!(core.last_command_line(), None);
    feed(&mut core, &command("cargo build", "Compiling\r\n", 0));
    assert_eq!(core.last_command_line().as_deref(), Some("cargo build"));
    // It stays while the next prompt is up, and a line that runs on past the width is read whole.
    feed(&mut core, "\x1b]133;A\x07PS> \x1b]133;B\x07");
    assert_eq!(core.last_command_line().as_deref(), Some("cargo build"));
    feed(&mut core, "echo 0123456789012345678901234567\r\n\x1b]133;C\x07");
    assert_eq!(core.last_command_line().as_deref(), Some("echo 0123456789012345678901234567"));
    // An empty line, or an output start without a prompt before it, has none.
    feed(&mut core, &command("", "", 0));
    assert_eq!(core.last_command_line(), None);
    feed(&mut core, "\x1b]133;C\x07");
    assert_eq!(core.last_command_line(), None);
}

#[test]
fn the_command_line_loses_bidi_overrides_and_stops_at_256_characters_when_the_marks_span_the_screen() {
    // It is shown in toasts and phone pushes, where an override would turn the text around.
    let mut core = core(40, 12);
    feed(&mut core, "\x1b]133;A\x07PS> \x1b]133;B\x07rm \u{202e}txt.exe\r\n\x1b]133;C\x07");
    assert_eq!(core.last_command_line().as_deref(), Some("rm txt.exe"));
    feed(&mut core, &format!("\x1b]133;A\x07PS> \x1b]133;B\x07{}\r\n\x1b]133;C\x07", "x".repeat(400)));
    assert_eq!(core.last_command_line().map(|line| line.chars().count()), Some(256));
}

#[test]
fn the_sticky_prompt_is_the_one_of_the_command_the_top_of_the_view_is_in_while_that_prompt_is_above_it() {
    let output = |first, last| CommandOutput { first, last, exit: Some(0), running: false };
    // Prompts at -20 and -8 with outputs -18..=-15 and -6..=3, and between them a command that wrote nothing at -13.
    let outputs = [output(-18, -15), output(-6, 3)];
    let prompts = [-20, -13, -8];
    let sticky = |top| sticky_prompt_line(top, &outputs, &prompts);
    assert_eq!(sticky(-20), None, "the prompt itself is the top row: it is in sight");
    assert_eq!(sticky(-19), Some(-20), "the command line between the prompt and its output");
    assert_eq!(sticky(-17), Some(-20));
    assert_eq!(sticky(-15), Some(-20), "the last line of the output");
    assert_eq!(sticky(-14), None, "below that output");
    assert_eq!(sticky(-13), None, "the prompt of a command that wrote nothing");
    assert_eq!(sticky(-8), None, "the next command's own prompt");
    assert_eq!(sticky(-7), Some(-8));
    assert_eq!(sticky(-3), Some(-8));
    assert_eq!(sticky(5), None, "past every output");
    assert_eq!(sticky(-25), None, "above every prompt");
    assert_eq!(sticky_prompt_line(-3, &outputs, &[]), None, "a shell without prompt marks pins nothing");
    assert_eq!(sticky_prompt_line(-3, &[], &prompts), None);
}

#[test]
fn the_snapshot_pins_the_prompt_line_while_scrolled_into_its_output_and_never_on_the_alternate_screen() {
    let mut core = core(20, 4);
    let mut lines = String::new();
    for i in 0..10 {
        lines.push_str(&format!("out {i}\r\n"));
    }
    feed(&mut core, &command("ls", &lines, 0));
    feed(&mut core, "\x1b]133;A\x07PS> \x1b]133;B\x07");
    let header = |core: &mut TerminalCore| snap(core).sticky_header.iter().map(|cell| cell.ch).collect::<String>().trim_end().to_owned();
    assert_eq!(header(&mut core), "", "at the bottom the prompt in use is in sight");
    core.scroll_display(Scroll::Delta(5));
    assert_eq!(header(&mut core), "PS> ls");
    assert_eq!(snap(&mut core).sticky_header.len(), 20, "a whole row");
    core.set_sticky_header(false);
    assert_eq!(header(&mut core), "", "the setting turns it off");
    core.set_sticky_header(true);
    // A click on the header brings its prompt to the top row, where it is in sight and no longer pinned.
    assert!(core.scroll_to_sticky_prompt());
    assert_eq!(snap(&mut core).cells.iter().take(6).map(|cell| cell.ch).collect::<String>(), "PS> ls");
    assert_eq!(header(&mut core), "");
    assert!(!core.scroll_to_sticky_prompt(), "nothing pinned, nothing to scroll to");
    core.scroll_display(Scroll::Delta(-2));
    feed(&mut core, "\x1b[?1049h");
    assert_eq!(header(&mut core), "", "the alternate screen has no marks and no scrollback");
}

#[test]
fn selection_simple_block_lines_and_semantic() {
    let mut core = core(20, 5);
    feed(&mut core, "hello world\r\nsecond line");

    core.selection_start(SelectionKind::Simple, 0, 6, CellSide::Left);
    core.selection_update(1, 5, CellSide::Right);
    assert_eq!(core.selection_text().as_deref(), Some("world\nsecond"));
    assert!(snap(&mut core).is_selected(0, 6));
    assert!(!snap(&mut core).is_selected(0, 5));

    core.selection_start(SelectionKind::Block, 0, 0, CellSide::Left);
    core.selection_update(1, 3, CellSide::Right);
    assert_eq!(core.selection_text().as_deref(), Some("hell\nseco"));

    core.selection_start(SelectionKind::Lines, 0, 3, CellSide::Left);
    assert_eq!(core.selection_text().as_deref(), Some("hello world"), "no final line break: see a_line_selection_copies...");

    core.selection_start(SelectionKind::Semantic, 0, 8, CellSide::Left);
    assert_eq!(core.selection_text().as_deref(), Some("world"));

    core.selection_clear();
    assert_eq!(core.selection_text(), None);
}

#[test]
fn selection_with_cjk_text_and_fullwidth_punctuation_boundaries() {
    let mut core = core(30, 3);
    feed(&mut core, "你好，世界 abc");
    // Double click on 世 (column 6): the full-width comma ends the word on the left, the space on the right.
    core.selection_start(SelectionKind::Semantic, 0, 6, CellSide::Left);
    assert_eq!(core.selection_text().as_deref(), Some("世界"));
    core.selection_start(SelectionKind::Simple, 0, 0, CellSide::Left);
    core.selection_update(0, 9, CellSide::Right);
    assert_eq!(core.selection_text().as_deref(), Some("你好，世界"), "spacers do not duplicate or drop characters");
}

#[test]
fn selection_is_dropped_when_the_column_count_changes() {
    let mut core = core(20, 5);
    feed(&mut core, "hello");
    core.selection_start(SelectionKind::Simple, 0, 0, CellSide::Left);
    core.selection_update(0, 4, CellSide::Right);
    assert!(core.selection_text().is_some());
    core.resize(10, 5);
    assert_eq!(core.selection_text(), None);
}

/// Selects from `from` to `to` (viewport row, column; both cells included) and returns the text a copy gives.
fn copied(core: &mut TerminalCore, from: (usize, usize), to: (usize, usize)) -> Option<String> {
    core.selection_start(SelectionKind::Simple, from.0, from.1, CellSide::Left);
    core.selection_update(to.0, to.1, CellSide::Right);
    core.selection_text()
}

#[test]
fn a_soft_wrapped_line_is_copied_without_a_line_break_and_a_hard_one_with() {
    let mut core = core(10, 4);
    feed(&mut core, "0123456789abcdef\r\nsecond");
    assert_eq!(snap(&mut core).text(), "0123456789\nabcdef\nsecond");
    assert_eq!(copied(&mut core, (0, 0), (2, 5)).as_deref(), Some("0123456789abcdef\nsecond"));
    assert_eq!(copied(&mut core, (0, 7), (1, 2)).as_deref(), Some("789abc"), "across the wrap, no break inside");
    assert_eq!(copied(&mut core, (0, 0), (0, 9)).as_deref(), Some("0123456789"), "the wrapped row alone ends without one");
}

#[test]
fn trailing_blanks_are_dropped_from_every_copied_line_and_inner_blanks_stay() {
    let mut core = core(10, 4);
    feed(&mut core, "a  b   \r\n\r\nlast   ");
    assert_eq!(copied(&mut core, (0, 0), (2, 9)).as_deref(), Some("a  b\n\nlast"));
    // blanks that were painted with a background colour are still blanks
    feed(&mut core, "\x1b[2;1H\x1b[44m   \x1b[0m");
    assert_eq!(copied(&mut core, (0, 0), (2, 9)).as_deref(), Some("a  b\n\nlast"));
    core.selection_start(SelectionKind::Block, 0, 0, CellSide::Left);
    core.selection_update(2, 5, CellSide::Right);
    assert_eq!(core.selection_text().as_deref(), Some("a  b\n\nlast"), "a block is trimmed per line too");
}

#[test]
fn wide_characters_and_emoji_are_copied_once_whatever_part_of_them_is_selected() {
    let mut core = core(30, 2);
    feed(&mut core, "你好😀 a👨\u{200d}👩\u{200d}👧bcdef\tc");
    let whole = snap(&mut core).row_text(0);
    assert_eq!(copied(&mut core, (0, 0), (0, 29)).as_deref(), Some(whole.as_str()));
    assert_eq!(whole, "你好😀 a👨\u{200d}👩\u{200d}👧bcdef\tc");
    assert_eq!(copied(&mut core, (0, 0), (0, 0)).as_deref(), Some("你"), "the first half");
    assert_eq!(copied(&mut core, (0, 1), (0, 1)).as_deref(), Some("你"), "the second half (the spacer)");
    assert_eq!(copied(&mut core, (0, 1), (0, 2)).as_deref(), Some("你好"), "the end of one and the start of the next");
    assert_eq!(copied(&mut core, (0, 4), (0, 5)).as_deref(), Some("😀"));
}

#[test]
fn a_wide_character_that_wrapped_early_is_copied_once() {
    let mut core = core(5, 3);
    // The wide 世 does not fit in the last column and moves to the next row, leaving a spacer behind.
    feed(&mut core, "abcd世e");
    assert_eq!(snap(&mut core).text(), "abcd\n世e");
    assert_eq!(copied(&mut core, (0, 0), (1, 4)).as_deref(), Some("abcd世e"));
}

#[test]
fn a_line_selection_copies_the_line_without_a_final_line_break() {
    let mut core = core(20, 4);
    feed(&mut core, "hello world\r\nsecond line");
    core.selection_start(SelectionKind::Lines, 0, 3, CellSide::Left);
    assert_eq!(core.selection_text().as_deref(), Some("hello world"), "pasting it must not run it");
    core.selection_update(1, 3, CellSide::Right);
    assert_eq!(core.selection_text().as_deref(), Some("hello world\nsecond line"));
}

#[test]
fn control_and_bidi_override_characters_do_not_reach_the_clipboard() {
    let mut core = core(20, 2);
    feed(&mut core, "a\u{9b}b\u{202e}c\u{2066}d");
    assert_eq!(copied(&mut core, (0, 0), (0, 19)).as_deref(), Some("abcd"));
}

#[test]
fn select_all_takes_the_scrollback_down_to_the_last_line_with_text() {
    let mut core = core(20, 4);
    assert!(core.selection_text().is_none());
    core.select_all();
    assert!(core.selection_text().is_none(), "nothing to select in an empty buffer: {:?}", core.selection_text());
    for i in 0..10 {
        feed(&mut core, &format!("line{i}\r\n"));
    }
    assert!(core.history_size() > 0);
    core.select_all();
    let all: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
    assert_eq!(core.selection_text().as_deref(), Some(all.join("\n").as_str()), "the blank rows under the text stay out");
    assert!(core.selection_active());
    core.selection_clear();
    assert!(!core.selection_active());
}

#[test]
fn a_selection_follows_its_text_into_the_scrollback_and_the_view() {
    let mut core = core(20, 5);
    for i in 0..5 {
        feed(&mut core, &format!("line{i}\r\n"));
    }
    // Five lines and the line break after the last scroll line0 into the scrollback: the rows are line1 to line4.
    core.selection_start(SelectionKind::Simple, 1, 0, CellSide::Left);
    core.selection_update(2, 4, CellSide::Right);
    assert_eq!(core.selection_text().as_deref(), Some("line2\nline3"));
    for i in 5..12 {
        feed(&mut core, &format!("line{i}\r\n"));
    }
    assert_eq!(core.selection_text().as_deref(), Some("line2\nline3"), "new output scrolled the lines away, the selection went along");
    core.scroll_display(Scroll::Top);
    let view = snap(&mut core);
    let marked: Vec<(String, bool)> = (0..5).map(|row| (view.row_text(row), view.is_selected(row, 0))).collect();
    let expected = [("line0", false), ("line1", false), ("line2", true), ("line3", true), ("line4", false)];
    assert_eq!(marked, expected.map(|(text, selected)| (text.to_owned(), selected)));
    // Rows only: a change of height keeps the selection on its text.
    core.scroll_display(Scroll::Bottom);
    core.selection_start(SelectionKind::Simple, 3, 0, CellSide::Left);
    core.selection_update(3, 4, CellSide::Right);
    assert_eq!(core.selection_text().as_deref(), Some("line1"), "the first five characters of line11");
    core.resize(20, 8);
    assert_eq!(core.selection_text().as_deref(), Some("line1"), "a taller terminal keeps the selected text");
    core.resize(20, 3);
    assert_eq!(core.selection_text().as_deref(), Some("line1"), "so does a shorter one");
}

#[test]
fn search_finds_matches_in_scrollback_with_smart_case() {
    let mut core = core(30, 5);
    for i in 0..60 {
        feed(&mut core, &format!("Alpha {i} beta\r\n"));
    }
    feed(&mut core, "alpha lowercase\r\n");
    let lower = core.search_all("alpha", 1000).unwrap();
    assert_eq!(lower.len(), 61, "no uppercase in the pattern: case-insensitive");
    assert!(lower[0].start.0 < 0, "first hit is in the scrollback");
    let upper = core.search_all("Alpha", 1000).unwrap();
    assert_eq!(upper.len(), 60, "uppercase in the pattern: case-sensitive");
    let digits = core.search_all(r"Alpha 5\d beta", 1000).unwrap();
    assert_eq!(digits.len(), 10);
    assert_eq!(core.text_between(digits[0].start, digits[0].end), "Alpha 50 beta");
    assert!(core.search_all("(", 10).is_err());
    assert_eq!(core.search_all("alpha", 5).unwrap().len(), 5, "limit applies");
    // A flag in the pattern wins over smart case: the find bar's "match case" and its default.
    assert_eq!(core.search_all("(?-i)alpha", 1000).unwrap().len(), 1, "case counts: only the lowercase line");
    assert_eq!(core.search_all("(?i)Alpha", 1000).unwrap().len(), 61, "case ignored though the pattern has a capital");
}

#[test]
fn a_selection_gives_its_runs_of_style_and_select_between_takes_grid_points() {
    let mut core = core(20, 3);
    feed(&mut core, "ok \x1b[1;31merror\x1b[0m   \r\nnext");
    core.select_between((0, 0), (1, 3));
    let lines = core.selection_runs().expect("a selection");
    assert_eq!(lines.len(), 2);
    let first: Vec<(&str, CellColor, bool)> = lines[0].iter().map(|r| (r.text.as_str(), r.fg, r.bold)).collect();
    assert_eq!(first, [("ok ", CellColor::Default, false), ("error", CellColor::Indexed(1), true)], "the blanks after the text are left out");
    assert_eq!(lines[1].iter().map(|r| r.text.as_str()).collect::<String>(), "next");
    assert_eq!(core.selection_text().as_deref(), Some("ok error\nnext"));
    // Backwards is the same selection.
    core.select_between((1, 3), (0, 0));
    assert_eq!(core.selection_text().as_deref(), Some("ok error\nnext"));
}

#[test]
fn a_selection_s_runs_keep_combining_marks_and_whole_clusters() {
    let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
    let mut core = core(20, 3);
    feed(&mut core, &format!("e\u{0301}{family}\x1b[1me\u{0301}"));
    core.select_between((0, 0), (0, 3));
    let text: Vec<String> = core.selection_runs().expect("a selection")[0].iter().map(|r| r.text.clone()).collect();
    assert_eq!(text, [format!("e\u{0301}{family}"), "e\u{0301}".to_owned()], "a new run starts with a mark too");
}

#[test]
fn all_text_is_the_scrollback_and_the_screen_without_trailing_blank_lines() {
    let mut core = TerminalCore::new(20, 5, CoreConfig { scrollback_lines: 100, ..CoreConfig::default() });
    for i in 0..12 {
        feed(&mut core, &format!("line {i}\r\n"));
    }
    let text = core.all_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.first(), Some(&"line 0"), "from the oldest line of the scrollback");
    assert_eq!(lines.last(), Some(&"line 11"), "to the last line written, no blank rows after it");
    assert_eq!(lines.len(), 12);
}

#[test]
fn search_next_steps_through_matches_wraps_and_runs_backwards() {
    let mut core = core(20, 4);
    for i in 0..30 {
        feed(&mut core, &format!("row {i} hit\r\n"));
    }
    let mut session = core.search_start("row 1\\d").unwrap(); // row 10..19
    let first = core.search_next(&mut session, (-100, 0), true).unwrap();
    assert_eq!(core.text_between(first.start, first.end), "row 10");
    let second = core.search_next(&mut session, (first.end.0, first.end.1 + 1), true).unwrap();
    assert_eq!(core.text_between(second.start, second.end), "row 11");
    // past the last hit: wraps to the first
    let last_origin = (4, 0);
    let wrapped = core.search_next(&mut session, last_origin, true).unwrap();
    assert_eq!(core.text_between(wrapped.start, wrapped.end), "row 10");
    let back = core.search_next(&mut session, (second.start.0, second.start.1), false).unwrap();
    assert_eq!(core.text_between(back.start, back.end), "row 11", "backwards includes a match that starts at the origin");
    let mut none = core.search_start("zzz").unwrap();
    assert!(core.search_next(&mut none, (0, 0), true).is_none());
}

#[test]
fn search_regex_dialect_has_no_unicode_word_boundary() {
    let mut core = core(30, 3);
    feed(&mut core, "cat concat cat");
    let err = core.search_all(r"\bcat\b", 10).unwrap_err();
    assert!(err.contains("Unicode word boundaries"), "{err}");
    // ASCII word boundaries work; this is the only way to offer "whole word".
    let hits = core.search_all(r"(?-u:\b)cat(?-u:\b)", 10).unwrap();
    assert_eq!(hits.len(), 2);
}

#[test]
fn search_spans_wrapped_lines_and_wide_characters() {
    let mut core = core(10, 5);
    feed(&mut core, "xxxxxxxx foo bar");
    let m = core.search_all("foo bar", 10).unwrap();
    assert_eq!(m.len(), 1, "a match may cross a soft wrap");
    assert_eq!(core.text_between(m[0].start, m[0].end), "foo bar");

    let mut core = self::core(20, 3);
    feed(&mut core, "你好世界!");
    let m = core.search_all("世界", 10).unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(core.text_between(m[0].start, m[0].end), "世界");
}

// --- hyperlinks -------------------------------------------------------------------------------

#[test]
fn osc8_hyperlinks_are_stored_on_cells() {
    let mut core = core(30, 3);
    feed(&mut core, "\x1b]8;id=l1;https://example.com/a\x1b\\link\x1b]8;;\x1b\\ plain \x1b]8;;https://no-id.example\x07auto\x1b]8;;\x07");
    let s = snap(&mut core);
    for column in 0..4 {
        let link = s.cell(0, column).hyperlink.as_ref().expect("link cell");
        assert_eq!((link.id(), link.uri()), ("l1", "https://example.com/a"));
    }
    assert!(s.cell(0, 4).hyperlink.is_none(), "closing OSC 8 ends the link");
    assert!(s.cell(0, 6).hyperlink.is_none());
    let auto = s.cell(0, 11).hyperlink.as_ref().expect("link without explicit id");
    assert_eq!(auto.uri(), "https://no-id.example");
    assert!(auto.id().ends_with("_alacritty"), "alacritty invents the id: {}", auto.id());
    assert_eq!(core.hyperlink_at(0, 2).unwrap().uri(), "https://example.com/a");
}

// --- modes ------------------------------------------------------------------------------------

// --- links --------------------------------------------------------------------------------------------

#[test]
fn an_osc8_hyperlink_is_a_link_over_exactly_its_cells() {
    let mut core = core(30, 3);
    feed(&mut core, "go \x1b]8;;https://example.com/a\x07click here\x1b]8;;\x07 and more");
    let link = core.link_at(0, 5).expect("over the link text");
    assert_eq!((link.uri.as_str(), link.spans.as_slice()), ("https://example.com/a", [(0, 3, 13)].as_slice()));
    assert_eq!(core.link_at(0, 3).map(|l| l.uri), Some("https://example.com/a".to_owned()), "its first cell");
    assert_eq!(core.link_at(0, 12).map(|l| l.uri), Some("https://example.com/a".to_owned()), "its last cell");
    assert_eq!(core.link_at(0, 2), None, "the space before it");
    assert_eq!(core.link_at(0, 13), None, "the space after it");
    assert_eq!(core.link_at(1, 0), None);
    assert_eq!(core.link_at(0, 99), None, "outside the grid");
    assert_eq!(core.link_at(99, 0), None);
}

#[test]
fn an_osc8_autolink_that_runs_into_full_width_punctuation_ends_before_it() {
    let mut core = core(60, 2);
    // What Claude Code writes for `https://example.com/full・結束・`: the whole run marked as one link.
    feed(&mut core, "a \x1b]8;;https://example.com/full\u{30fb}結束\u{30fb}\x07https://example.com/full\u{30fb}結束\u{30fb}\x1b]8;;\x07");
    let link = core.link_at(0, 5).expect("over the address");
    assert_eq!((link.uri.as_str(), link.spans.as_slice()), ("https://example.com/full", [(0, 2, 26)].as_slice()));
    assert_eq!(core.link_at(0, 28), None, "結束 after the middle dot is text");
}

#[test]
fn an_osc8_link_that_wraps_is_one_link_on_every_row_and_one_id_joins_separate_runs() {
    let mut core = core(10, 4);
    feed(&mut core, "\x1b]8;;https://a.b/c\x07fifteen chars!!\x1b]8;;\x07");
    let link = core.link_at(1, 2).expect("on the second row");
    assert_eq!(link.spans, [(0, 0, 10), (1, 0, 5)]);
    let mut core = self::core(40, 3);
    feed(&mut core, "\x1b]8;id=x;https://a.b/\x07one\x1b]8;;\x07 and \x1b]8;id=x;https://a.b/\x07two\x1b]8;;\x07 and \x1b]8;;https://a.b/\x07three\x1b]8;;\x07");
    assert_eq!(core.link_at(0, 1).map(|l| l.spans), Some(vec![(0, 0, 3), (0, 8, 11)]), "the same id and address are one link");
    assert_eq!(core.link_at(0, 17).map(|l| l.spans), Some(vec![(0, 16, 21)]), "no id: its own link");
}

#[test]
fn only_a_link_that_may_be_opened_is_a_link() {
    let mut core = core(60, 4);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "\\Cargo.toml");
    let uri = format!("file:///{}", path.replace('\\', "/").replace(' ', "%20"));
    feed(&mut core, &format!("\x1b]8;;javascript:alert(1)\x07script\x1b]8;;\x07 \x1b]8;;file:///C:/Windows/System32/calc.exe\x07program\x1b]8;;\x07 \x1b]8;;{uri}\x07file\x1b]8;;\x07"));
    assert_eq!(core.link_at(0, 2), None, "a script address is text");
    assert_eq!(core.link_at(0, 10), None, "so is a program");
    let link = core.link_at(0, 18).expect("a file that exists");
    assert_eq!((link.uri.as_str(), link.target.as_str()), (uri.as_str(), path), "what is opened is the checked path");
    feed(&mut core, "\r\njavascript:alert(1) calc.exe file:///C:/Windows/System32/calc.exe http://");
    for column in [0, 10, 25, 40, 59] {
        assert_eq!(core.link_at(1, column), None, "column {column}");
    }
    // ShellExecute runs setup.exe for a name with no extension that does not exist.
    feed(&mut core, "\r\n\x1b]8;;file:///C:/nekominal-no-such-dir/setup\x07setup\x1b]8;;\x07 file:///C:/nekominal-no-such-dir/setup");
    assert_eq!(core.link_at(2, 1), None, "a file that does not exist is text");
    assert_eq!(core.link_at(2, 20), None, "in the text too");
}

#[test]
fn a_printed_path_that_starts_with_a_dot_or_two_opens_from_the_folder_the_shell_reported() {
    // `join` kept the dots, and the link checks refuse a name that is a dot, so these were never links.
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "\\src");
    let mut core = core(60, 3);
    feed(&mut core, &format!("\x1b]7;file:///{}\x07.\\lib.rs ..\\Cargo.toml", folder.replace('\\', "/").replace(' ', "%20")));
    let expected = |path: &str| std::fs::canonicalize(path).unwrap().to_str().unwrap().trim_start_matches(r"\\?\").to_owned();
    let here = core.link_at(0, 2).expect("the file in the folder");
    assert_eq!(here.target, expected(&format!("{folder}\\lib.rs")));
    let above = core.link_at(0, 12).expect("the file one folder up");
    assert_eq!(above.target, expected(concat!(env!("CARGO_MANIFEST_DIR"), "\\Cargo.toml")));
}

#[test]
fn a_url_in_the_text_is_a_link_without_the_punctuation_after_it() {
    let mut core = core(60, 3);
    feed(&mut core, "see https://example.com/path?q=1, and (http://a.b/c). end");
    let link = core.link_at(0, 10).expect("on the first URL");
    assert_eq!((link.uri.as_str(), link.spans.as_slice()), ("https://example.com/path?q=1", [(0, 4, 32)].as_slice()));
    assert_eq!(core.link_at(0, 32), None, "the comma is not part of it");
    assert_eq!(core.link_at(0, 0), None);
    assert_eq!(core.link_at(0, 40).map(|l| l.uri), Some("http://a.b/c".to_owned()), "inside the brackets");
    assert_eq!(core.link_at(0, 52), None, "the closing bracket and the full stop are not");
}

#[test]
fn a_url_that_wraps_is_found_from_any_of_its_rows_and_its_cells_are_all_marked() {
    let mut core = core(12, 5);
    feed(&mut core, "x https://example.com/very/long/path y");
    assert_eq!(snap(&mut core).text(), "x https://ex\nample.com/ve\nry/long/path\n y");
    let first = core.link_at(0, 5).expect("first row");
    assert_eq!(first.uri, "https://example.com/very/long/path");
    assert_eq!(first.spans, [(0, 2, 12), (1, 0, 12), (2, 0, 12)]);
    assert_eq!(core.link_at(1, 6), Some(first.clone()), "second row");
    assert_eq!(core.link_at(2, 11), Some(first), "the last cell of the last row");
    assert_eq!(core.link_at(3, 0), None, "the space that wrapped after it");
    assert_eq!(core.link_at(3, 1), None);
    assert_eq!(core.link_at(0, 0), None);
}

#[test]
fn a_url_after_wide_characters_is_found_at_its_cells_and_a_wide_character_ends_it() {
    let mut core = core(40, 3);
    feed(&mut core, "你好 https://a.b/x你好 end");
    assert_eq!(core.link_at(0, 0), None);
    let link = core.link_at(0, 8).expect("the URL starts after four cells and a space");
    assert_eq!((link.uri.as_str(), link.spans.as_slice()), ("https://a.b/x", [(0, 5, 18)].as_slice()));
    assert_eq!(core.link_at(0, 18), None, "the wide character after it is text");
}

#[test]
fn links_follow_the_view_into_the_scrollback() {
    let mut core = core(12, 3);
    feed(&mut core, "https://example.com/a/b/c/d\r\n1\r\n2\r\n3\r\n4\r\n");
    assert_eq!(core.link_at(0, 0), None, "the live screen shows the lines after it");
    core.scroll_display(Scroll::Top);
    assert_eq!(snap(&mut core).row_text(0), "https://exam");
    let link = core.link_at(1, 3).expect("the second row of the URL in view");
    assert_eq!(link.spans, [(0, 0, 12), (1, 0, 12), (2, 0, 3)]);
    core.scroll_display(Scroll::Delta(-1));
    assert_eq!(snap(&mut core).row_text(0), "ple.com/a/b/", "one row of the URL scrolled out above the view");
    let link = core.link_at(0, 3).expect("the URL continues above the view");
    assert_eq!(link.uri, "https://example.com/a/b/c/d");
    assert_eq!(link.spans, [(0, 0, 12), (1, 0, 3)], "only the rows in view");
}

#[test]
fn bracketed_paste_focus_cursor_keys_and_keypad_flags() {
    let mut core = core(20, 3);
    let m = core.mode();
    assert!(m.contains(TermMode::SHOW_CURSOR | TermMode::LINE_WRAP | TermMode::ALTERNATE_SCROLL));
    assert!(!m.intersects(TermMode::BRACKETED_PASTE | TermMode::FOCUS_IN_OUT | TermMode::APP_CURSOR));

    feed(&mut core, "\x1b[?2004h\x1b[?1004h\x1b[?1h\x1b=");
    let m = core.mode();
    assert!(m.contains(TermMode::BRACKETED_PASTE | TermMode::FOCUS_IN_OUT | TermMode::APP_CURSOR | TermMode::APP_KEYPAD));
    feed(&mut core, "\x1b[?2004l\x1b[?1004l\x1b[?1l\x1b>\x1b[?1007l\x1b[?25l");
    let m = core.mode();
    assert!(!m.intersects(TermMode::BRACKETED_PASTE | TermMode::FOCUS_IN_OUT | TermMode::APP_CURSOR | TermMode::APP_KEYPAD));
    assert!(!m.intersects(TermMode::ALTERNATE_SCROLL | TermMode::SHOW_CURSOR));
}

#[test]
fn mouse_reporting_modes_are_mutually_exclusive_and_encodings_are_flags() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[?1000h");
    assert!(core.mode().contains(TermMode::MOUSE_REPORT_CLICK));
    feed(&mut core, "\x1b[?1002h");
    assert!(core.mode().contains(TermMode::MOUSE_DRAG) && !core.mode().contains(TermMode::MOUSE_REPORT_CLICK));
    feed(&mut core, "\x1b[?1003h");
    assert!(core.mode().contains(TermMode::MOUSE_MOTION) && !core.mode().contains(TermMode::MOUSE_DRAG));
    assert!(core.mode().intersects(TermMode::MOUSE_MODE));

    feed(&mut core, "\x1b[?1006h");
    assert!(core.mode().contains(TermMode::SGR_MOUSE));
    feed(&mut core, "\x1b[?1005h");
    assert!(core.mode().contains(TermMode::UTF8_MOUSE) && !core.mode().contains(TermMode::SGR_MOUSE));
    feed(&mut core, "\x1b[?1006h");
    assert!(core.mode().contains(TermMode::SGR_MOUSE) && !core.mode().contains(TermMode::UTF8_MOUSE));

    feed(&mut core, "\x1b[?1003l\x1b[?1006l");
    assert!(!core.mode().intersects(TermMode::MOUSE_MODE | TermMode::SGR_MOUSE));
    // 1016 (SGR pixels), 9001 (win32-input-mode) and 2027 are unknown to TermMode (the sniffer tracks the
    // first two, see below).
    let before = core.mode();
    feed(&mut core, "\x1b[?1016h\x1b[?9001h\x1b[?2027h");
    assert_eq!(core.mode(), before);
}

#[test]
fn x10_urxvt_and_sgr_pixel_mouse_modes_are_tracked_beside_term_mode() {
    let mut core = core(20, 3);
    assert_eq!(core.mouse_extensions(), MouseExtensions::default());
    feed(&mut core, "\x1b[?1015h\x1b[?1016h\x1b[?9h");
    assert_eq!(core.mouse_extensions(), MouseExtensions { x10: true, urxvt: true, sgr_pixels: true });
    assert!(!core.mode().intersects(TermMode::MOUSE_MODE | TermMode::SGR_MOUSE | TermMode::UTF8_MOUSE));

    // The other tracking modes replace X10 reporting (xterm: the last one wins); the encodings stay.
    feed(&mut core, "\x1b[?1000h");
    assert_eq!(core.mouse_extensions(), MouseExtensions { x10: false, urxvt: true, sgr_pixels: true });
    assert!(core.mode().contains(TermMode::MOUSE_REPORT_CLICK));
    feed(&mut core, "\x1b[?9h");
    assert!(core.mouse_extensions().x10, "9 set after 1000: X10 wins in the host");
    feed(&mut core, "\x1b[?9l\x1b[?1015;1016l");
    assert_eq!(core.mouse_extensions(), MouseExtensions::default());

    // Several modes in one sequence, and ones this does not own, leave the rest alone.
    feed(&mut core, "\x1b[?25;1016;1004h");
    assert_eq!(core.mouse_extensions(), MouseExtensions { x10: false, urxvt: false, sgr_pixels: true });
}

#[test]
fn decrqm_for_the_sniffed_mouse_modes_reports_their_real_state_in_stream_order() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[?1016$p\x1b[?9$p\x1b[?1015$p");
    assert_eq!(replies(&core.take_events()), "\x1b[?1016;2$y\x1b[?9;2$y\x1b[?1015;2$y", "reset: 2, never 0");

    // set, query, reset, query in one chunk, with a DA1 reply that must stay between them.
    feed(&mut core, "\x1b[?1016h\x1b[?1016$p\x1b[c\x1b[?1016l\x1b[?1016$p");
    assert_eq!(replies(&core.take_events()), "\x1b[?1016;1$y\x1b[?6c\x1b[?1016;2$y");

    feed(&mut core, "\x1b[?9h\x1b[?9$p");
    assert_eq!(replies(&core.take_events()), "\x1b[?9;1$y");
    // Modes the sniffer does not own are still alacritty's to answer.
    feed(&mut core, "\x1b[?1006$p\x1b[?2027$p");
    assert_eq!(replies(&core.take_events()), "\x1b[?1006;2$y\x1b[?2027;0$y");
}

#[test]
fn kitty_keyboard_push_pop_set_and_query() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[?u");
    assert_eq!(replies(&core.take_events()), "\x1b[?0u");

    feed(&mut core, "\x1b[>1u");
    assert_eq!(core.kitty_flags(), 1);
    assert!(core.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
    feed(&mut core, "\x1b[>5u\x1b[?u");
    assert_eq!(core.kitty_flags(), 5);
    assert!(core.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALTERNATE_KEYS));
    assert_eq!(replies(&core.take_events()), "\x1b[?5u");

    feed(&mut core, "\x1b[<u");
    assert_eq!(core.kitty_flags(), 1, "pop restores the previous entry");
    feed(&mut core, "\x1b[=8;2u"); // union
    assert_eq!(core.kitty_flags(), 9);
    assert!(core.mode().contains(TermMode::REPORT_ALL_KEYS_AS_ESC));
    feed(&mut core, "\x1b[=1;3u"); // difference
    assert_eq!(core.kitty_flags(), 8);
    feed(&mut core, "\x1b[=18;1u"); // replace with events + associated text
    assert_eq!(core.kitty_flags(), 18);
    assert!(core.mode().contains(TermMode::REPORT_EVENT_TYPES | TermMode::REPORT_ASSOCIATED_TEXT));
    feed(&mut core, "\x1b[<99u");
    assert_eq!(core.kitty_flags(), 0, "popping more than the depth empties the stack");
}

#[test]
fn kitty_keyboard_stacks_are_per_screen_and_ris_clears_them() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[>1u");
    feed(&mut core, "\x1b[?1049h");
    assert_eq!(core.kitty_flags(), 0, "the alternate screen has its own stack");
    feed(&mut core, "\x1b[>8u");
    assert_eq!(core.kitty_flags(), 8);
    feed(&mut core, "\x1b[?1049l");
    assert_eq!(core.kitty_flags(), 1, "main screen stack is back");
    feed(&mut core, "\x1bc");
    assert_eq!(core.kitty_flags(), 0);

    feed(&mut core, "\x1b[>31u");
    assert_eq!(core.kitty_flags(), 31);
    core.reset_keyboard_modes();
    assert_eq!(core.kitty_flags(), 0, "host-side reset for programs that die without popping");
}

#[test]
fn kitty_keyboard_can_be_disabled_by_config() {
    let mut core = TerminalCore::new(20, 3, CoreConfig { kitty_keyboard: false, ..CoreConfig::default() });
    feed(&mut core, "\x1b[>1u\x1b[?u");
    assert_eq!(core.kitty_flags(), 0);
    assert_eq!(replies(&core.take_events()), "", "no flags reply: programs conclude 'unsupported' from the DA1 reply");
}

// --- replies ----------------------------------------------------------------------------------

#[test]
fn da1_da2_dsr_and_text_area_replies() {
    let mut core = core(20, 5);
    feed(&mut core, "\x1b[c");
    assert_eq!(replies(&core.take_events()), "\x1b[?6c");
    feed(&mut core, "\x1b[>c");
    let da2 = replies(&core.take_events());
    assert!(da2.starts_with("\x1b[>0;") && da2.ends_with(";1c"), "{da2:?}");
    feed(&mut core, "\x1b[5n\x1b[3;7H\x1b[6n");
    assert_eq!(replies(&core.take_events()), "\x1b[0n\x1b[3;7R");
    feed(&mut core, "\x1b[18t\x1b[14t");
    assert_eq!(replies(&core.take_events()), "\x1b[8;5;20t\x1b[4;80;160t");
}

#[test]
fn da1_reply_can_be_replaced() {
    let config = CoreConfig { da1_reply: Some("\x1b[?62;22c".into()), ..CoreConfig::default() };
    let mut core = TerminalCore::new(20, 5, config);
    feed(&mut core, "\x1b[0c");
    assert_eq!(replies(&core.take_events()), "\x1b[?62;22c");
}

#[test]
fn decrqm_reports_known_modes_and_denies_the_rest() {
    let mut core = core(20, 5);
    feed(&mut core, "\x1b[?2004$p\x1b[?2004h\x1b[?2004$p\x1b[?2026$p\x1b[?2027$p\x1b[?9001$p\x1b[?1049$p\x1b[4$p");
    assert_eq!(
        replies(&core.take_events()),
        "\x1b[?2004;2$y\x1b[?2004;1$y\x1b[?2026;2$y\x1b[?2027;0$y\x1b[?9001;0$y\x1b[?1049;2$y\x1b[4;2$y"
    );
}

#[test]
fn xtversion_and_other_probes_are_answered_in_stream_order() {
    let mut core = core(20, 5);
    // kitty query, XTVERSION, OSC 7501 query, DECRQSS, then DA1: the DA1 reply must come last.
    feed(&mut core, "\x1b[?u\x1b[>q\x1b]7501;?\x1b\\\x1bP$qm\x1b\\\x1b[c");
    assert_eq!(
        replies(&core.take_events()),
        "\x1b[?0u\x1bP>|Nekominal 0.1.0\x1b\\\x1b]7501;?\x1b\\\x1bP0$r\x1b\\\x1b[?6c"
    );
    feed(&mut core, "\x1b]7501;?\x07\x1b[?996n\x1bP+q544e\x1b\\");
    assert_eq!(replies(&core.take_events()), "\x1b]7501;?\x07\x1b[?997;1n\x1bP0+r\x1b\\");
}

#[test]
fn osc52_follows_the_configured_policy() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b]52;c;aGVsbG8=\x07\x1b]52;c;?\x07");
    let events = core.take_events();
    assert_eq!(events.len(), 1, "default policy: copy allowed, paste (read) denied: {events:?}");
    assert!(matches!(&events[0], CoreEvent::ClipboardStore { text, .. } if text == "hello"));

    let mut core = TerminalCore::new(20, 3, CoreConfig { osc52: Osc52::CopyPaste, ..CoreConfig::default() });
    feed(&mut core, "\x1b]52;c;?\x07");
    match core.take_events().pop() {
        Some(CoreEvent::ClipboardLoad { reply, .. }) => assert_eq!(reply("hi"), "\x1b]52;c;aGk=\x07"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn osc52_policy_changes_apply_to_a_running_terminal_and_the_text_stays_out_of_debug_output() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b]0;kept\x07");
    core.take_events();
    core.set_osc52(Osc52::Disabled);
    feed(&mut core, "\x1b]52;c;aGVsbG8=\x07");
    assert!(!core.take_events().iter().any(|e| matches!(e, CoreEvent::ClipboardStore { .. })), "disabled: no write");

    core.set_osc52(Osc52::CopyPaste);
    feed(&mut core, "\x1b]52;c;c2VjcmV0\x07\x1b]52;c;?\x07");
    let events = core.take_events();
    let store = events.iter().find(|e| matches!(e, CoreEvent::ClipboardStore { .. })).expect("copy is allowed again");
    assert!(events.iter().any(|e| matches!(e, CoreEvent::ClipboardLoad { .. })), "paste is allowed now");
    assert!(!format!("{store:?}").contains("secret"), "{store:?}");
    assert_eq!(snap(&mut core).history_size, 0, "changing the options leaves the buffer alone");
}

#[test]
fn a_new_session_starts_clean_and_what_the_old_one_drew_moves_into_the_scrollback() {
    let mut core = core(30, 4);
    for i in 0..6 {
        feed(&mut core, &format!("old {i}\r\n"));
    }
    let history = core.history_size();
    feed(&mut core, "\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?25l\x1b[2;3r\x1b[31m\x1b]7501;state=working:id=a\x07x");
    core.take_events();
    assert!(core.mode().contains(TermMode::ALT_SCREEN | TermMode::SGR_MOUSE | TermMode::BRACKETED_PASTE));
    assert!(core.agent_summary().is_some());

    core.reset_for_new_session();
    let mode = core.mode();
    for flag in [TermMode::ALT_SCREEN, TermMode::MOUSE_REPORT_CLICK, TermMode::SGR_MOUSE, TermMode::BRACKETED_PASTE] {
        assert!(!mode.contains(flag), "{flag:?} is still set");
    }
    assert!(mode.contains(TermMode::SHOW_CURSOR), "the cursor is back");
    assert!(core.agent_summary().is_none());
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::AgentRecordsDropped)));
    assert_eq!(snap(&mut core).text(), "", "the screen is clear for the new program");
    assert_eq!(core.cursor_cell(), (0, 0));
    assert!(core.history_size() > history, "the screen the old program drew went into the scrollback");
    let found = core.search_all("old 5", 10).expect("a valid pattern");
    assert_eq!(found.len(), 1, "and the old output can still be read there");
    // The scroll region and the colours went too: a line break at the bottom scrolls the whole screen.
    feed(&mut core, "\r\n\r\n\r\n\r\n");
    assert_eq!(snap(&mut core).cursor.position.map(|(row, _)| row), Some(3));
}

#[test]
fn a_new_session_on_the_main_screen_does_not_restore_a_saved_cursor() {
    let mut core = core(30, 4);
    feed(&mut core, "\x1b7top\r\nbottom");
    core.reset_for_new_session();
    assert_eq!((snap(&mut core).text(), core.cursor_cell()), (String::new(), (0, 0)));
    feed(&mut core, "new");
    assert_eq!(snap(&mut core).text(), "new");
    assert_eq!(core.search_all("bottom", 10).expect("pattern").len(), 1, "what was on the screen is in the scrollback");
}
#[test]
fn title_bell_and_cursor_events() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b]0;hello\x07\x1b]2;a;b\x1b\\\x1b]1;icon only\x07\x07");
    let events = core.take_events();
    let titles: Vec<_> = events.iter().filter_map(|e| if let CoreEvent::Title(t) = e { Some(t.as_str()) } else { None }).collect();
    assert_eq!(titles, ["hello", "a;b"], "OSC 1 is ignored");
    assert!(events.iter().any(|e| matches!(e, CoreEvent::Bell)));

    feed(&mut core, "\x1b[22t\x1b]2;temp\x07\x1b[23t");
    let events = core.take_events();
    assert!(matches!(events.last(), Some(CoreEvent::Title(t)) if t == "a;b"), "{events:?}");

    feed(&mut core, "\x1b[5 q");
    let s = snap(&mut core);
    assert_eq!((s.cursor.shape, s.cursor.blinking), (CursorShape::Beam, true));
    feed(&mut core, "\x1b[2 q\x1b[?25l");
    let s = snap(&mut core);
    assert_eq!(s.cursor.shape, CursorShape::Hidden);
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::CursorBlinkingChange)));
}

#[test]
fn the_default_cursor_can_change_while_the_terminal_runs_and_a_program_choice_outlasts_it() {
    use nekominal_term::alacritty_terminal::vte::ansi::CursorStyle;
    let bar = CursorStyle { shape: CursorShape::Beam, blinking: false };
    let block = CursorStyle { shape: CursorShape::Block, blinking: true };
    let mut core = TerminalCore::new(20, 3, CoreConfig { cursor_style: bar, ..CoreConfig::default() });
    let shape = |core: &mut TerminalCore| {
        let s = snap(core);
        (s.cursor.shape, s.cursor.blinking)
    };
    assert_eq!(shape(&mut core), (CursorShape::Beam, false), "the default of the settings");
    core.set_cursor_style(block);
    assert_eq!(shape(&mut core), (CursorShape::Block, true), "a setting that changed reaches a running terminal");
    // A program that sets its own cursor keeps it when the setting changes, and gets the new default back by asking.
    feed(&mut core, "\x1b[4 q");
    assert_eq!(shape(&mut core), (CursorShape::Underline, false));
    core.set_cursor_style(bar);
    assert_eq!(shape(&mut core), (CursorShape::Underline, false));
    feed(&mut core, "\x1b[0 q");
    assert_eq!(shape(&mut core), (CursorShape::Beam, false));
}

// --- synchronized output ----------------------------------------------------------------------

#[test]
fn sync_update_holds_the_screen_until_esu() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[?2026hhello");
    assert!(core.is_sync_pending());
    assert_eq!(snap(&mut core).text(), "", "nothing is applied inside the frame");
    feed(&mut core, " world");
    assert_eq!(snap(&mut core).text(), "");
    feed(&mut core, "\x1b[?2026l");
    assert!(!core.is_sync_pending());
    assert_eq!(snap(&mut core).text(), "hello world", "the whole frame appears at once");
}

#[test]
fn sync_update_times_out_after_150ms() {
    let mut core = core(20, 3);
    let started = Instant::now();
    feed(&mut core, "\x1b[?2026hstuck");
    let deadline = core.sync_deadline().expect("a deadline is armed by BSU");
    let after = deadline.duration_since(started);
    assert!(after >= Duration::from_millis(100) && after <= Duration::from_millis(400), "{after:?}");

    assert!(!core.flush_expired_sync(Instant::now()), "not expired yet");
    assert_eq!(snap(&mut core).text(), "");
    assert!(core.flush_expired_sync(deadline + Duration::from_millis(1)));
    assert!(!core.is_sync_pending());
    assert_eq!(snap(&mut core).text(), "stuck");
    assert!(core.take_dirty());
}

#[test]
fn feed_flushes_an_expired_frame_first_unlike_alacritty_event_loop() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[?2026ha");
    std::thread::sleep(Duration::from_millis(170));
    feed(&mut core, "b");
    assert!(!core.is_sync_pending());
    assert_eq!(snap(&mut core).text(), "ab");
}

#[test]
fn bsu_right_after_esu_starts_a_new_frame() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[?2026ha\x1b[?2026l\x1b[?2026hb");
    assert!(core.is_sync_pending());
    assert_eq!(snap(&mut core).text(), "a");
    feed(&mut core, "\x1b[?2026l");
    assert_eq!(snap(&mut core).text(), "ab");
}

// --- sequences the emulator ignores, surfaced by the sniffer ------------------------------------

#[test]
fn ignored_sequences_leave_the_grid_untouched_and_surface_as_events() {
    let mut core = core(40, 4);
    feed(
        &mut core,
        "\x1b]7;file://PC/C:/Users/me/%E4%BD%A0\x1b\\\x1b]9;9;\"D:\\x\"\x07\x1b]9;4;1;30\x07\x1b]777;notify;T;B\x07\x1b]9;Hello\x07",
    );
    assert_eq!(snap(&mut core).text(), "", "no text leaks into the grid");
    let events = core.take_events();
    assert!(matches!(&events[0], CoreEvent::Cwd(p) if p == "C:\\Users\\me\\你"));
    assert!(matches!(&events[1], CoreEvent::Cwd(p) if p == "D:\\x"));
    assert!(matches!(&events[2], CoreEvent::Progress(Some(_))));
    assert!(matches!(&events[3], CoreEvent::Notification(Notification { title, body }) if title == "T" && body == "B"));
    assert!(matches!(&events[4], CoreEvent::Notification(Notification { title, body }) if title.is_empty() && body == "Hello"));
}

#[test]
fn osc133_marks_carry_the_cursor_and_reset_working_agent_records() {
    let mut core = core(40, 4);
    feed(&mut core, "\x1b]7501;state=working:id=a\x07\x1b]7501;state=done:id=b\x07");
    assert_eq!(core.agent_summary().map(|r| r.state), Some(AgentState::Working));
    core.take_events();

    feed(&mut core, "out\r\n\x1b]133;A\x07$ \x1b]133;B\x07");
    let events = core.take_events();
    assert!(events.iter().any(|e| matches!(e, CoreEvent::AgentRecordsDropped)));
    let marks: Vec<_> = events
        .iter()
        .filter_map(|e| if let CoreEvent::PromptMark { mark, cursor } = e { Some((*mark, *cursor)) } else { None })
        .collect();
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[0].0, PromptMark::PromptStart);
    let at_a = marks[0].1.expect("cursor known outside a synchronized update");
    assert_eq!((at_a.line, at_a.column), (1, 0), "the emulator had applied everything before the mark");
    assert_eq!(marks[1].1.map(|c| (c.line, c.column)), Some((1, 2)));
    assert_eq!(core.agent_summary().map(|r| (r.id.as_str(), r.state)), Some(("b", AgentState::Done)));

    feed(&mut core, "\x1bc");
    assert!(core.agent_summary().is_none(), "RIS clears every record");
}

#[test]
fn agent_reports_flow_through_core_and_child_exit_drops_active_ones() {
    let mut core = core(40, 4);
    feed(&mut core, "\x1b]7501;state=blocked:kind=permission:id=x\x1b\\\x1b]7501;bogus\x07");
    let events = core.take_events();
    assert_eq!(events.iter().filter(|e| matches!(e, CoreEvent::Agent(_))).count(), 1, "malformed reports are dropped");
    assert_eq!(core.agent_summary().map(|r| r.state), Some(AgentState::Blocked));
    core.on_child_exit();
    assert!(core.agent_summary().is_none());
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::AgentRecordsDropped)));
}

#[test]
fn sixel_and_kitty_payloads_never_reach_the_grid_as_text() {
    let mut core = core(40, 4);
    feed(&mut core, "a\x1bP0;0;0q\"1;1;4;4#0;2;0;0;0~-~\x1b\\b\x1b_Ga=T,f=24,s=1,v=1,C=1;AAAA\x1b\\c");
    let snapshot = snap(&mut core);
    assert_eq!(snapshot.row_text(0), "a", "the Sixel moved the cursor below itself, to the column it started at");
    assert_eq!(snapshot.row_text(1), " bc", "C=1 leaves the cursor where the kitty picture went");
    assert_eq!(snapshot.images.len(), 2);
    assert!(core.take_events().iter().all(|e| !matches!(e, CoreEvent::Unsupported(Unsupported::KittyAnimation))));
}

#[test]
fn unterminated_osc_strings_are_bounded_and_text_after_them_still_prints() {
    let mut core = core(40, 4);
    core.feed(b"\x1b]2;");
    let filler = vec![b'x'; 1 << 20];
    for _ in 0..10 {
        core.feed(&filler); // 10 MiB in total; a title is cut at the short cap
    }
    core.feed(b"\x07after");
    let events = core.take_events();
    let title = events.iter().find_map(|e| if let CoreEvent::Title(t) = e { Some(t.len()) } else { None });
    assert_eq!(title, Some(nekominal_term::guard::MAX_SHORT_OSC - 2), "the prefix is dispatched when CAN ends the string");
    assert_eq!(snap(&mut core).text(), "after", "the dropped remainder and its BEL do not leak into the grid");
}

#[test]
fn conpty_startup_requests_are_tracked_win32_input_mode_and_focus_reporting() {
    let mut core = core(20, 3);
    // What the bundled ConPTY 1.25 writes before anything else (measured in tests/conpty_startup.rs).
    feed(&mut core, "\x1b[1t\x1b[c\x1b[?1004h\x1b[?9001h\x1b[?7l\x1b[?7h");
    assert!(core.win32_input_mode());
    assert!(core.mode().contains(TermMode::FOCUS_IN_OUT), "ConPTY itself asks for focus events");
    let events = core.take_events();
    assert_eq!(replies(&events), "\x1b[?6c");
    assert!(events.iter().any(|e| matches!(e, CoreEvent::Win32InputMode(true))));
    feed(&mut core, "\x1b[?9001l");
    assert!(!core.win32_input_mode());
    feed(&mut core, "\x1b[?9001h\x1bc");
    assert!(!core.win32_input_mode(), "RIS resets it");
}

#[test]
fn upstream_panic_on_the_4097th_kitty_push_is_contained() {
    let mut core = core(20, 3);
    feed(&mut core, "survivor");
    for _ in 0..5000 {
        core.feed(b"\x1b[>1u");
    }
    let events = core.take_events();
    let panics: Vec<_> = events.iter().filter(|e| matches!(e, CoreEvent::EmulatorPanic(_))).collect();
    assert_eq!(panics.len(), 1, "one reset, not one per push: {panics:?}");
    assert_eq!(snap(&mut core).text(), "", "the reset cleared the screen");
    feed(&mut core, "alive");
    assert_eq!(snap(&mut core).text(), "alive", "the core keeps working afterwards");
}

#[test]
fn text_between_and_search_tolerate_out_of_range_points() {
    let mut core = core(10, 3);
    feed(&mut core, "hello");
    assert_eq!(core.text_between((-50, 0), (99, 99)).trim_end(), "hello");
    assert_eq!(core.text_between((2, 5), (0, 0)), "");
    let mut session = core.search_start("hello").unwrap();
    assert!(core.search_next(&mut session, (500, 500), true).is_some());
}

#[test]
fn decstr_soft_reset_clears_modes_and_attributes_but_keeps_screen_and_cursor() {
    let mut core = core(20, 5);
    // Modes DECSTR resets, an SGR state, a scroll region, text on screen, and the modes it leaves alone.
    feed(
        &mut core,
        "keep\r\n\x1b[?2004h\x1b[?1h\x1b=\x1b[?6h\x1b[?7l\x1b[?25l\x1b[4h\x1b[31;1m\x1b[2;4r\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[>1u",
    );
    // DECOM is on and the region homed the cursor: put it somewhere recognisable inside the region.
    feed(&mut core, "\x1b[2;3Hxy");
    let before = core.cursor_cell();
    core.take_events();

    feed(&mut core, "\x1b[!pZ");

    let mode = core.mode();
    assert!(!mode.intersects(TermMode::BRACKETED_PASTE | TermMode::APP_CURSOR | TermMode::APP_KEYPAD | TermMode::INSERT));
    assert!(!mode.intersects(TermMode::ORIGIN), "origin mode is off");
    assert!(mode.contains(TermMode::LINE_WRAP | TermMode::SHOW_CURSOR), "autowrap and the cursor are back on");
    assert!(!mode.intersects(TermMode::MOUSE_MODE | TermMode::SGR_MOUSE), "program-chosen mouse modes are off");
    assert!(mode.contains(TermMode::FOCUS_IN_OUT), "host-protocol modes survive: ConPTY asked for these");
    assert_eq!(core.kitty_flags(), 1, "the kitty keyboard stack survives");

    let s = snap(&mut core);
    assert_eq!(s.row_text(0), "keep", "the screen content is untouched");
    assert_eq!(s.cell(2, 2).ch, 'x');
    // "Z" is printed after the reset: default colours, at the cursor where the reset found it.
    let z = (0..s.rows).flat_map(|r| (0..s.columns).map(move |c| (r, c))).find(|&(r, c)| s.cell(r, c).ch == 'Z');
    assert_eq!(z, Some((before.0, before.1)), "the cursor did not move");
    let z = z.map(|(r, c)| s.cell(r, c).clone()).expect("Z is on screen");
    assert_eq!(z.fg_raw, CellColor::Default, "SGR was reset");
    assert!(!z.flags.contains(Flags::BOLD));

    // The scroll region is the full screen again: a line feed on the last row scrolls the whole screen.
    feed(&mut core, "\x1b[5;1H\r\nlast");
    assert_eq!(snap(&mut core).row_text(0), "", "row 0 scrolled away with the full-screen region");
    assert_eq!(core.history_size(), 1);
}

#[test]
fn decstr_inside_a_synchronized_update_keeps_the_cursor_the_update_moved() {
    let mut core = core(20, 12);
    feed(&mut core, "\x1b[?2026h\x1b[10;10H\x1b[!p\x1b[?2026l");
    assert_eq!(core.cursor_cell(), (9, 9));
}

#[test]
fn the_bytes_a_synchronized_update_held_are_joined_into_clusters_when_applied() {
    let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
    let mut core = core(20, 3);
    feed(&mut core, &format!("\x1b[?2026h{family}\x1b[!p\x1b[?2026l"));
    assert_eq!(core.cursor_cell(), (0, 2), "DECSTR applies the update: the family is two columns, not six");
    let mut timed = self::core(20, 3);
    feed(&mut timed, &format!("\x1b[?2026h{family}"));
    let deadline = timed.sync_deadline().expect("a deadline is armed by BSU");
    assert!(timed.flush_expired_sync(deadline + Duration::from_millis(1)));
    assert_eq!(timed.cursor_cell(), (0, 2), "the timeout applies the update the same way");
}

#[test]
fn decstr_resets_the_sniffed_mouse_extensions_and_ris_does_too() {
    let mut core = core(20, 3);
    feed(&mut core, "\x1b[?9h\x1b[?1015h\x1b[?1016h");
    assert_eq!(core.mouse_extensions(), MouseExtensions { x10: true, urxvt: true, sgr_pixels: true });
    feed(&mut core, "\x1b[!p");
    assert_eq!(core.mouse_extensions(), MouseExtensions::default());
    feed(&mut core, "\x1b[?1016h\x1bc");
    assert_eq!(core.mouse_extensions(), MouseExtensions::default());
}

// --- core behaviour as a whole ------------------------------------------------------------------

#[test]
fn feeding_in_any_chunking_gives_the_same_screen_and_events() {
    let mut stream = Vec::new();
    stream.extend_from_slice("héllo 你好 \u{1f468}\u{200d}\u{1f469}\r\n".as_bytes());
    stream.extend_from_slice(b"\x1b[31mred\x1b[0m \x1b]8;;https://e.x\x1b\\link\x1b]8;;\x1b\\\r\n");
    stream.extend_from_slice(b"\x1b[?u\x1b[>q\x1b[c\x1b]7501;state=working:id=z\x07\x1b]133;A\x07");
    stream.extend_from_slice(b"\x1b[?2026hhidden\x1b[2K\x1b[?2026lvisible\x1b]2;title\x07");
    stream.extend_from_slice("\x1b]777;notify;t;你好\x07".as_bytes());

    let run = |chunks: &mut dyn Iterator<Item = &[u8]>| {
        let mut core = core(30, 8);
        for chunk in chunks {
            core.feed(chunk);
        }
        let text = snap(&mut core).text();
        let events: Vec<String> = core.take_events().iter().map(|e| format!("{e:?}")).collect();
        (text, events, core.cursor_cell(), core.agent_summary().map(|r| r.state))
    };
    let whole = run(&mut std::iter::once(&stream[..]));
    assert!(whole.0.contains("visible") && !whole.0.contains("hidden"));
    let bytewise = run(&mut stream.chunks(1));
    assert_eq!(whole, bytewise);
    for size in [2, 3, 5, 7, 11, 64] {
        assert_eq!(whole, run(&mut stream.chunks(size)), "chunk size {size}");
    }
}

#[test]
fn line_wrap_state_cursor_on_wide_cells_and_invalid_utf8() {
    let mut core = core(5, 3);
    feed(&mut core, "abcde");
    assert_eq!(core.cursor_cell(), (0, 4), "the wrap is pending, the cursor stays on the last column");
    feed(&mut core, "f");
    let s = snap(&mut core);
    assert!(s.cell(0, 4).flags.contains(Flags::WRAPLINE));
    assert_eq!(s.text(), "abcde\nf");

    let mut core = self::core(10, 3);
    feed(&mut core, "你\x1b[D");
    assert_eq!(snap(&mut core).cursor.position, Some((0, 0)), "the cursor is drawn on the wide cell, not its spacer");
    core.feed(&[0xff, b'a', 0xe4, 0xbd]); // invalid byte, then a truncated 3-byte sequence
    core.feed(&[0xa0, b'!']); // ...completed in the next chunk (U+4F60)
    assert!(snap(&mut core).text().contains("\u{fffd}a你!"));
}

#[test]
fn snapshot_damage_is_partial_for_small_updates_and_full_for_scroll_resize_and_scrollback_moves() {
    let mut core = core(20, 5);
    let mut s = Snapshot::default();
    core.snapshot(&mut s);
    assert_eq!(s.damage, Damage::Full, "first frame");
    core.snapshot(&mut s);
    assert!(matches!(&s.damage, Damage::Lines(lines) if lines.len() == 1), "idle frame: only the cursor cell: {:?}", s.damage);

    feed(&mut core, "x");
    core.snapshot(&mut s);
    match &s.damage {
        Damage::Lines(lines) => assert!(lines.iter().any(|l| l.row == 0 && l.right >= 1), "{lines:?}"),
        Damage::Full => panic!("typing one character must not damage everything"),
    }

    feed(&mut core, "\r\n\r\n\r\n\r\n\r\n");
    core.snapshot(&mut s);
    assert_eq!(s.damage, Damage::Full, "scrolling damages everything");

    core.resize(10, 5);
    core.snapshot(&mut s);
    assert_eq!(s.damage, Damage::Full);

    core.scroll_display(Scroll::Delta(1));
    core.snapshot(&mut s);
    assert_eq!(s.damage, Damage::Full, "moving through scrollback");

    // Selection is not part of the damage state: the host must track it itself.
    core.scroll_display(Scroll::Bottom);
    core.snapshot(&mut s);
    core.selection_start(SelectionKind::Simple, 0, 0, CellSide::Left);
    core.selection_update(0, 3, CellSide::Right);
    core.snapshot(&mut s);
    assert!(matches!(&s.damage, Damage::Lines(l) if l.len() == 1));
    assert!(s.selection.is_some());
}

#[derive(Default, Debug)]
struct DamageAudit {
    frames: usize,
    partial_frames: usize,
    changed_cells: usize,
    /// A changed cell on a row the damage list does not mention at all.
    line_level_misses: usize,
    /// Changed cell outside its row's damaged column range, but it is the spacer of a wide glyph whose
    /// base cell is damaged (redrawing the glyph covers it).
    spacer_only: usize,
    /// Only the combining-mark list of the cell changed.
    zero_width_only: usize,
    /// A wide glyph's base cell was destroyed by output that started on its spacer.
    wide_overwrite: usize,
    other: usize,
    first_other: String,
    first_line_miss: String,
    /// `CoreEvent::EmulatorPanic` events: alacritty_terminal panicked on the generated input.
    emulator_panics: usize,
    first_panic: String,
}

/// Differential test harness: whatever changes between two frames must lie inside the reported damage.
fn damage_audit(
    seeds: std::ops::RangeInclusive<u64>,
    steps: usize,
    allow_zero_width: bool,
    allow_scroll_region: bool,
    damage_margin: usize,
) -> DamageAudit {
    let mut audit = DamageAudit::default();
    for seed in seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let mut core = TerminalCore::new(37, 11, CoreConfig { damage_margin, ..CoreConfig::default() });
        let mut previous = Snapshot::default();
        core.snapshot(&mut previous);
        let mut recent: std::collections::VecDeque<String> = Default::default();
        for step in 0..steps {
            let op = random_op(&mut rng, allow_zero_width, allow_scroll_region);
            recent.push_back(format!("{op:?}"));
            if recent.len() > 8 {
                recent.pop_front();
            }
            core.feed(op.as_bytes());
            for event in core.take_events() {
                if let CoreEvent::EmulatorPanic(message) = event {
                    audit.emulator_panics += 1;
                    if audit.first_panic.is_empty() {
                        audit.first_panic = format!("seed {seed} step {step} after {:?}: {message}", recent);
                    }
                }
            }
            let mut current = Snapshot::default();
            core.snapshot(&mut current);
            audit.frames += 1;
            if let Damage::Lines(lines) = &current.damage {
                audit.partial_frames += 1;
                for row in 0..current.rows {
                    for column in 0..current.columns {
                        let (a, b) = (previous.cell(row, column), current.cell(row, column));
                        if pixel_equal(a, b) {
                            continue;
                        }
                        audit.changed_cells += 1;
                        let covered = |c: usize| lines.iter().any(|l| l.row == row && l.left <= c && c <= l.right);
                        if !lines.iter().any(|l| l.row == row) {
                            audit.line_level_misses += 1;
                            if audit.first_line_miss.is_empty() {
                                audit.first_line_miss = format!(
                                    "seed {seed} step {step} cell ({row},{column}) {:?} -> {:?} damage {lines:?} cursor {:?} -> {:?}\n  recent ops: {}",
                                    (a.ch, a.flags), (b.ch, b.flags), previous.cursor.position, current.cursor.position,
                                    recent.iter().cloned().collect::<Vec<_>>().join(" ")
                                );
                            }
                        }
                        if covered(column) {
                            continue;
                        }
                        let only_zero_width = SnapCellKey::of(a) == SnapCellKey::of(b);
                        let spacer = column > 0
                            && covered(column - 1)
                            && (a.flags.contains(Flags::WIDE_CHAR_SPACER) || b.flags.contains(Flags::WIDE_CHAR_SPACER));
                        let overwritten_wide = covered(column + 1)
                            && (a.flags.contains(Flags::WIDE_CHAR) || b.flags.contains(Flags::WIDE_CHAR));
                        if only_zero_width {
                            audit.zero_width_only += 1;
                        } else if spacer {
                            audit.spacer_only += 1;
                        } else if overwritten_wide {
                            audit.wide_overwrite += 1;
                        } else {
                            audit.other += 1;
                            if audit.first_other.is_empty() {
                                audit.first_other = format!(
                                    "seed {seed} step {step} cell ({row},{column}) {:?} -> {:?} damage {lines:?} cursor {:?} -> {:?}\n  recent ops: {}",
                                    (a.ch, a.flags), (b.ch, b.flags), previous.cursor.position, current.cursor.position,
                                    recent.iter().cloned().collect::<Vec<_>>().join(" ")
                                );
                            }
                        }
                    }
                }
            }
            previous = current;
        }
    }
    audit
}

/// Equal as far as drawing is concerned: WRAPLINE (copy semantics) and the leading wide-char spacer
/// marker (a blank cell either way) do not change a single pixel.
fn pixel_equal(a: &nekominal_term::SnapCell, b: &nekominal_term::SnapCell) -> bool {
    let mask = Flags::WRAPLINE | Flags::LEADING_WIDE_CHAR_SPACER;
    let strip = |c: &nekominal_term::SnapCell| {
        let mut c = c.clone();
        c.flags.remove(mask);
        c
    };
    strip(a) == strip(b)
}

/// Everything about a cell except its combining marks.
#[derive(PartialEq)]
struct SnapCellKey(char, CellColor, CellColor, Flags);

impl SnapCellKey {
    fn of(cell: &nekominal_term::SnapCell) -> Self {
        Self(cell.ch, cell.fg_raw, cell.bg_raw, cell.flags)
    }
}

#[test]
fn raw_alacritty_damage_has_gaps_around_wide_glyphs_and_combining_marks() {
    let audit = damage_audit(1..=6, 6000, true, false, 0);
    println!("damage audit, margin 0, zero-width input, no scroll regions: {audit:?}");
    assert!(audit.partial_frames > 10_000, "the test must exercise partial damage: {audit:?}");
    assert!(audit.wide_overwrite + audit.zero_width_only + audit.spacer_only > 0, "{audit:?}");
}

#[test]
fn damage_with_a_two_column_margin_is_sound_without_scroll_regions() {
    let audit = damage_audit(1..=6, 6000, true, false, 2);
    println!("damage audit, margin 2, zero-width input, no scroll regions: {audit:?}");
    assert!(audit.partial_frames > 10_000, "the test must exercise partial damage: {audit:?}");
    assert_eq!(audit.line_level_misses, 0, "{audit:?}");
    assert_eq!((audit.zero_width_only, audit.wide_overwrite, audit.spacer_only, audit.other), (0, 0, 0, 0), "{audit:?}");
    assert_eq!(audit.emulator_panics, 0, "{audit:?}");
}

#[test]
fn damage_audit_with_scroll_regions_reports_the_wrap_in_place_gap() {
    let audit = damage_audit(1..=6, 6000, true, true, 2);
    println!("damage audit, margin 2, zero-width input, random DECSTBM: {audit:?}");
    assert!(audit.partial_frames > 10_000, "the test must exercise partial damage: {audit:?}");
}

/// Writing past the last column while the cursor is below the scroll region wraps in place; alacritty
/// damages neither the cells written before the wrap nor the line as a whole.
#[test]
fn known_gap_wrap_in_place_below_the_scroll_region_loses_damage() {
    let mut core = TerminalCore::new(10, 5, CoreConfig { damage_margin: 0, ..CoreConfig::default() });
    let mut s = Snapshot::default();
    feed(&mut core, "\x1b[1;3r\x1b[5;7H"); // region rows 1-3, cursor on the last row (outside the region)
    core.snapshot(&mut s);
    feed(&mut core, "abcdefgh"); // columns 7-10, then wraps in place and continues at columns 1-4
    let before = s.cell(4, 8).clone();
    core.snapshot(&mut s);
    assert_ne!(*s.cell(4, 8), before);
    match &s.damage {
        Damage::Lines(lines) => assert!(
            !lines.iter().any(|l| l.row == 4 && l.left <= 8 && 8 <= l.right),
            "if this starts failing alacritty fixed it: {lines:?}"
        ),
        Damage::Full => panic!("expected partial damage"),
    }
}
/// `ESC [ P` on a wide glyph leaves its spacer at column 0. alacritty's `RenderableCursor::new` then
/// computes `Column(0) - 1`, which panics in debug builds and wraps to usize::MAX in release builds.
#[test]
fn orphan_spacer_at_column_zero_does_not_break_the_snapshot() {
    let mut core = core(10, 3);
    feed(&mut core, "你好\x1b[H\x1b[P");
    let s = snap(&mut core);
    assert!(s.cell(0, 0).flags.contains(Flags::WIDE_CHAR_SPACER), "the spacer is now at column 0");
    assert_eq!(s.cursor.position, Some((0, 0)));
}
#[test]
fn known_gap_combining_mark_in_a_later_frame_is_not_damaged() {
    for (margin, expect_covered) in [(0usize, false), (2, true)] {
        let mut core = TerminalCore::new(20, 3, CoreConfig { damage_margin: margin, ..CoreConfig::default() });
        let mut s = Snapshot::default();
        feed(&mut core, "e");
        core.snapshot(&mut s); // resets damage
        feed(&mut core, "\u{301}");
        let before = s.cell(0, 0).clone();
        core.snapshot(&mut s);
        assert_ne!(*s.cell(0, 0), before, "the cell did change");
        let Damage::Lines(lines) = &s.damage else { panic!("expected partial damage") };
        let covered = lines.iter().any(|l| l.row == 0 && l.left == 0);
        assert_eq!(covered, expect_covered, "margin {margin}: {lines:?} (if margin 0 starts covering it, alacritty fixed the gap)");
    }
}
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_op(rng: &mut Rng, allow_zero_width: bool, allow_scroll_region: bool) -> String {
    let n = rng.below(14) + 1;
    let (r, c) = (rng.below(13) + 1, rng.below(40) + 1);
    match rng.below(46) {
        0..=7 => (0..rng.below(24) + 1).map(|_| (b'a' + rng.below(26) as u8) as char).collect(),
        8 => "\r\n".into(),
        9 => "\n".into(),
        10 => "\r".into(),
        11 => "\x08".into(),
        12 => "\t".into(),
        13 => format!("\x1b[{n}A"),
        14 => format!("\x1b[{n}B"),
        15 => format!("\x1b[{n}C"),
        16 => format!("\x1b[{n}D"),
        17 | 18 => format!("\x1b[{r};{c}H"),
        19 => format!("\x1b[{c}G"),
        20 => format!("\x1b[{r}d"),
        21 => format!("\x1b[{}K", rng.below(3)),
        22 => format!("\x1b[{}J", rng.below(3)),
        23 => format!("\x1b[{n}X"),
        24 => format!("\x1b[{n}@"),
        25 => format!("\x1b[{n}P"),
        26 => format!("\x1b[{n}L"),
        27 => format!("\x1b[{n}M"),
        28 => format!("\x1b[{}S", n % 5 + 1),
        29 => format!("\x1b[{}T", n % 5 + 1),
        30 if allow_scroll_region => format!("\x1b[{};{}r", rng.below(5) + 1, rng.below(8) + 6),
        30 => "\x1b[r".into(),
        31 => ["\x1bM", "\x1bD", "\x1bE", "\x1b7", "\x1b8", "\x1b[s", "\x1b[u"][rng.below(7) as usize].into(),
        32..=34 if allow_zero_width => ["你好", "世界！", "\u{1f600}", "\u{1f468}\u{200d}\u{1f469}", "e\u{301}", "\u{2764}\u{fe0f}", "ｱ"][rng.below(7) as usize].into(),
        32..=34 => ["你好", "世界！", "\u{1f600}", "ｱ"][rng.below(4) as usize].into(),
        35..=37 => {
            let sgr = ["0", "1", "3", "4", "7", "31", "42", "91", "38;5;200", "48;2;10;200;30", "2", "9"];
            format!("\x1b[{}m", sgr[rng.below(sgr.len() as u64) as usize])
        }
        38 => ["\x1b[?1049h", "\x1b[?1049l"][rng.below(2) as usize].into(),
        39 => ["\x1b[4h", "\x1b[4l", "\x1b[?6h", "\x1b[?6l", "\x1b[?7h", "\x1b[?7l"][rng.below(6) as usize].into(),
        40 => format!("\x1b[{n}b"),
        41 => "\x1b#8".into(),
        42 => format!("\x1b]4;{};rgb:{:02x}/00/ff\x07", rng.below(16), rng.below(256)),
        43 => "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".into(),
        44 => format!("\x1b[{}g\x1bH\x1b[I\x1b[Z", rng.below(2) * 3),
        _ => ["\x1b[2J", "\x1b[3J", "\x1b[H"][rng.below(3) as usize].into(),
    }
}

#[test]
fn text_between_starting_on_the_spacer_of_a_wide_character_does_not_panic() {
    let mut core = core(10, 3);
    feed(&mut core, "你好ab");
    // Column 1 is the spacer of 你, column 3 the spacer of 好: a point there is inside the character.
    assert_eq!(core.text_between((0, 1), (0, 3)), "你好");
    assert_eq!(core.text_between((0, 1), (0, 1)), "你");
    assert_eq!(core.text_between((0, 0), (0, 2)), "你好");
    assert_eq!(core.text_between((0, 2), (0, 5)), "好ab");
}

#[test]
fn text_between_on_the_blank_a_wide_character_leaves_at_the_end_of_a_line_does_not_panic() {
    let mut core = core(5, 3);
    // 你 does not fit in the last column: the cell there stays a leading spacer and the character wraps.
    feed(&mut core, "abcd你好");
    for start in [(0, 4), (0, 3), (1, 0), (1, 1)] {
        for end in [(0, 4), (1, 0), (1, 1), (1, 3), (2, 0)] {
            let _ = core.text_between(start, end);
        }
    }
    assert!(core.text_between((0, 4), (1, 1)).contains('你'));
}
