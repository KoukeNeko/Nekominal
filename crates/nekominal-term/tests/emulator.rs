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
    CellColor, CellSide, CoreConfig, CoreEvent, Damage, MouseExtensions, Palette, SelectionKind, Snapshot, TerminalCore,
    Unsupported,
};
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
fn emoji_zwj_sequence_is_three_wide_cells_with_zwj_attached() {
    let mut core = core(20, 3);
    feed(&mut core, "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}");
    let s = snap(&mut core);
    // unicode-width gives U+200D width 0, so alacritty never joins: 3 x 2 cells + ZWJ marks.
    assert_eq!(core.cursor_cell(), (0, 6));
    assert!(s.cell(0, 0).flags.contains(Flags::WIDE_CHAR));
    assert_eq!(s.cell(0, 0).zerowidth.as_deref(), Some(&['\u{200d}'][..]));
    assert_eq!(s.cell(0, 2).ch, '\u{1f469}');
    assert_eq!(s.cell(0, 4).ch, '\u{1f467}');
    assert_eq!(s.cell(0, 4).zerowidth, None);
    assert_eq!(s.row_text(0), "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}");
}

#[test]
fn variation_selector_16_is_zero_width_so_text_emoji_stay_narrow() {
    let mut core = core(20, 3);
    feed(&mut core, "\u{2764}\u{fe0f}x");
    let s = snap(&mut core);
    assert_eq!(s.cell(0, 0).ch, '\u{2764}');
    assert_eq!(s.cell(0, 0).zerowidth.as_deref(), Some(&['\u{fe0f}'][..]));
    assert_eq!(s.cell(0, 1).ch, 'x');
    // The string-level width in the same crate version says 2; the per-char path alacritty uses says 1.
    assert_eq!(unicode_width::UnicodeWidthStr::width("\u{2764}\u{fe0f}"), 2);
    assert_eq!('\u{2764}'.width(), Some(1));
    assert_eq!('\u{fe0f}'.width(), Some(0));
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
fn cursor_advance_always_equals_the_sum_of_unicode_width_per_char() {
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
        let expected: usize = sample.chars().map(|c| c.width().unwrap_or(0)).sum();
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
    let mut core = core(10, 5);
    feed(&mut core, "0123456789abcde");
    assert_eq!(snap(&mut core).text(), "0123456789\nabcde");
    core.resize(5, 5);
    // Three wrapped rows plus three blank ones do not fit in 5 lines: the top row moves into history.
    assert_eq!((core.history_size(), snap(&mut core).text()), (1, "56789\nabcde".to_owned()));
    core.scroll_display(Scroll::Top);
    assert_eq!(snap(&mut core).row_text(0), "01234");
    core.scroll_display(Scroll::Bottom);
    core.resize(20, 5);
    assert_eq!((core.history_size(), snap(&mut core).text()), (0, "0123456789abcde".to_owned()));
    // the alternate screen does not reflow
    feed(&mut core, "\x1b[?1049h\x1b[H0123456789abcdefghij");
    core.resize(10, 5);
    assert_eq!(snap(&mut core).text(), "0123456789", "alt screen content is cut, not reflowed");
}

// --- selection and search ---------------------------------------------------------------------

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
    assert_eq!(core.selection_text().as_deref(), Some("hello world\n"));

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
fn sixel_and_kitty_graphics_are_swallowed() {
    let mut core = core(40, 4);
    feed(&mut core, "a\x1bP0;0;0q\"1;1;4;4#0;2;0;0;0~-~\x1b\\b\x1b_Ga=T,f=24,s=1,v=1;AAAA\x1b\\c");
    assert_eq!(snap(&mut core).text(), "abc", "neither payload reaches the grid");
    let events = core.take_events();
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], CoreEvent::Unsupported(Unsupported::Sixel)));
}

#[test]
fn unterminated_osc_strings_are_bounded_and_text_after_them_still_prints() {
    let mut core = core(40, 4);
    core.feed(b"\x1b]2;");
    let filler = vec![b'x'; 1 << 20];
    for _ in 0..10 {
        core.feed(&filler); // 10 MiB in total, the cap is 4 MiB
    }
    core.feed(b"\x07after");
    let events = core.take_events();
    let title = events.iter().find_map(|e| if let CoreEvent::Title(t) = e { Some(t.len()) } else { None });
    assert_eq!(title, Some(4 * 1024 * 1024 - 2), "the prefix is dispatched when CAN ends the string");
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
