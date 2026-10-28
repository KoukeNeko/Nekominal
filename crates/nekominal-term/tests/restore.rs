//! The text of a pane saved at exit and shown again at the next start: the SGR serializer round trips, the bounds,
//! the alternate screen, and what replaying it may and may not do.

use nekominal_term::{
    CoreConfig, SnapCell, Snapshot, TerminalCore,
    alacritty_terminal::term::TermMode,
    restore::{SAVED_BYTES, SAVED_ROWS, sanitize},
};

fn feed(core: &mut TerminalCore, text: &str) {
    core.feed(text.as_bytes());
}

fn snap(core: &mut TerminalCore) -> Snapshot {
    let mut snapshot = Snapshot::default();
    core.snapshot(&mut snapshot);
    snapshot
}

fn rows(core: &mut TerminalCore) -> Vec<Vec<SnapCell>> {
    let snapshot = snap(core);
    snapshot.cells.chunks(snapshot.columns).map(<[SnapCell]>::to_vec).collect()
}

/// `text` fed into a fresh terminal, saved, and the save fed into another fresh terminal of the same size.
fn round_trip(columns: usize, lines: usize, text: &str) -> (TerminalCore, TerminalCore, String) {
    let mut original = TerminalCore::new(columns, lines, CoreConfig::default());
    feed(&mut original, text);
    let saved = original.saved_text();
    let mut replayed = TerminalCore::new(columns, lines, CoreConfig::default());
    feed(&mut replayed, &saved);
    (original, replayed, saved)
}

#[test]
fn colours_attributes_wide_characters_and_combining_marks_come_back_cell_for_cell() {
    let text = concat!(
        "\x1b[1;31mbold red\x1b[0m \x1b[3;4mitalic underline\x1b[0m\r\n",
        "\x1b[38;2;10;20;30;48;5;200mrgb on 200\x1b[0m \x1b[2;7;9;8mdim inverse strike hidden\x1b[0m\r\n",
        "\x1b[95;104mbright\x1b[0m \x1b[4:3;58;2;1;2;3mcurl\x1b[0m \x1b[4:2mdouble\x1b[0m \x1b[4:4mdots\x1b[4:5mdashes\x1b[0m\r\n",
        "中文寬字 e\u{301} a\u{20dd} 👍🏽\r\n",
        // A line longer than the screen is wide wraps, with a wide character that does not fit at the edge.
        "0123456789012345678901234567890123456789012345678中文 wrapped\r\n",
        "\x1b[44mblue to the end\x1b[K\x1b[0m\r\n",
        "  indented, then blanks   \r\n",
        "last",
    );
    let (mut original, mut replayed, saved) = round_trip(50, 12, text);
    for sgr in ["\x1b[0;1;31m", "38;2;10;20;30;48;5;200m", "\x1b[0;2;7;8;9m", "\x1b[0;95;104m", "4:3;58;2;1;2;3m", "\x1b[0;4:2m", "\x1b[0;44m"] {
        assert!(saved.contains(sgr), "{sgr:?} in {saved:?}");
    }
    let (before, after) = (rows(&mut original), rows(&mut replayed));
    assert!(before[3][0].ch == '中' && before[3].iter().any(|cell| cell.zerowidth.is_some()), "the test has what it means to test");
    for (row, (before, after)) in before.iter().zip(&after).enumerate() {
        assert_eq!(before, after, "row {row}");
    }
}

#[test]
fn a_saved_line_is_plain_at_its_end_so_lines_can_be_dropped_from_the_front() {
    let (_, _, saved) = round_trip(40, 6, "\x1b[1;32mgreen\r\nstill green\x1b[0m\r\nplain");
    let lines: Vec<&str> = saved.split_inclusive("\r\n").collect();
    assert_eq!(lines.len(), 3, "{saved:?}");
    assert!(lines.iter().all(|line| line.ends_with("\r\n")), "{saved:?}");
    assert!(lines[1].starts_with("\x1b[0;1;32m"), "each line sets its own style: {saved:?}");
    assert!(lines[1].ends_with("\x1b[0m\r\n"), "and resets it at its end: {saved:?}");
}

#[test]
fn a_tab_is_saved_as_the_blanks_it_left() {
    let (_, mut replayed, saved) = round_trip(40, 4, "a\tb\r\n");
    assert!(!saved.contains('\t'), "{saved:?}");
    assert_eq!(snap(&mut replayed).row_text(0), "a       b");
}

#[test]
fn an_empty_terminal_saves_nothing() {
    let (_, _, saved) = round_trip(40, 4, "");
    assert_eq!(saved, "");
    let (_, _, saved) = round_trip(40, 4, "\x1b[2J\x1b[H\r\n\r\n");
    assert_eq!(saved, "");
}

#[test]
fn the_scrollback_is_saved_with_the_screen() {
    let text: String = (0..100).map(|n| format!("line {n}\r\n")).collect();
    let (_, replayed, _) = round_trip(40, 10, &text);
    let all = replayed.all_text();
    assert!(all.starts_with("line 0\n"), "{all:?}");
    assert!(all.ends_with("line 99\n"), "{all:?}");
}

#[test]
fn a_full_screen_program_is_not_saved_but_the_shell_under_it_is_and_keeps_running() {
    let mut core = TerminalCore::new(40, 6, CoreConfig::default());
    feed(&mut core, "PS C:\\> vim notes.txt\r\n\x1b[?1049h\x1b[Hfull screen app\x1b[3;5H");
    let saved = core.saved_text();
    assert!(saved.contains("vim notes.txt"), "{saved:?}");
    assert!(!saved.contains("full screen app"), "{saved:?}");

    let snapshot = snap(&mut core);
    assert_eq!(snapshot.row_text(0).trim_end(), "full screen app", "the program's screen is back");
    assert_eq!(core.cursor_cell(), (2, 4), "and its cursor");
    feed(&mut core, "\x1b[?1049l");
    assert_eq!(snap(&mut core).row_text(0).trim_end(), "PS C:\\> vim notes.txt", "leaving it shows the shell");
}

#[test]
fn a_long_history_is_cut_to_the_newest_rows_and_bytes() {
    let config = CoreConfig { scrollback_lines: SAVED_ROWS * 2, scrollback_bytes: usize::MAX / 2, ..CoreConfig::default() };
    let mut core = TerminalCore::new(300, 20, config.clone());
    let text: String = (0..SAVED_ROWS + 500).map(|n| format!("{n:06} {}\r\n", "x".repeat(280))).collect();
    feed(&mut core, &text);
    let saved = core.saved_text();
    assert!(saved.len() <= SAVED_BYTES, "{} bytes", saved.len());
    assert!(saved.ends_with(&format!("{:06} {}\r\n", SAVED_ROWS + 499, "x".repeat(280))), "the newest line is kept");
    let first = saved.lines().next().unwrap();
    assert_eq!(first.len(), 287, "the cut falls between lines: {:?}", &first[..20]);

    let short: String = (0..SAVED_ROWS + 500).map(|n| format!("{n}\r\n")).collect();
    let mut core = TerminalCore::new(40, 20, config);
    feed(&mut core, &short);
    let saved = core.saved_text();
    assert_eq!(saved.lines().count(), SAVED_ROWS);
    assert!(saved.starts_with("500\r\n"), "{:?}", &saved[..20]);
}

#[test]
fn restored_text_lands_in_the_scrollback_above_the_separator_and_the_screen_is_left_empty() {
    let mut core = TerminalCore::new(40, 6, CoreConfig::default());
    core.show_restored("\x1b[31mold output\x1b[0m\r\nPS C:\\> exit\r\n", "Restored 2026-10-25 22:54");
    let snapshot = snap(&mut core);
    assert!((0..6).all(|row| snapshot.row_text(row).trim().is_empty()), "the new program starts on an empty screen");
    assert_eq!(core.cursor_cell(), (0, 0));
    assert_eq!(core.all_text().trim_end(), "old output\nPS C:\\> exit\n── Restored 2026-10-25 22:54 ──");

    feed(&mut core, "PS C:\\> ");
    let all = core.all_text();
    assert!(all.ends_with("──\nPS C:\\>\n"), "the new prompt is below the separator: {all:?}");
}

#[test]
fn restored_text_is_only_shown_never_acted_on() {
    let mut core = TerminalCore::new(40, 6, CoreConfig::default());
    let hostile = concat!(
        "before\x1b]52;c;aGVsbG8=\x07\x1b]0;evil title\x07\x1b]8;;https://example.com\x07link\x1b]8;;\x07",
        "\x1b[6n\x1b[c\x1bP+q544e\x1b\\\x1b[?1000h\x1b[?2004h\x1b]133;A\x07\x1b]9;4;3\x07after\r\n",
    );
    // `show_restored` drops what the feed reports; the sanitized text alone must report nothing to drop.
    let mut probe = TerminalCore::new(40, 6, CoreConfig::default());
    feed(&mut probe, &sanitize(hostile));
    let events = probe.take_events();
    assert!(events.is_empty(), "no reply, clipboard, title or mark: {events:?}");

    core.show_restored(hostile, "Restored");
    assert!(core.all_text().starts_with("beforelinkafter\n"), "{:?}", core.all_text());
    assert!(!core.mode().intersects(TermMode::MOUSE_MODE | TermMode::BRACKETED_PASTE), "no mode the old program set");
}

/// `text` fed into a fresh terminal, handed over (`handoff_screen`), and shown in another fresh terminal of the same size.
fn hand_over(columns: usize, lines: usize, text: &str) -> (TerminalCore, TerminalCore) {
    let mut original = TerminalCore::new(columns, lines, CoreConfig::default());
    feed(&mut original, text);
    let screen = original.handoff_screen();
    let mut taken = TerminalCore::new(columns, lines, CoreConfig::default());
    taken.show_handoff(&screen);
    (original, taken)
}

#[test]
fn a_handed_over_pane_shows_the_same_screen_scrollback_and_cursor_so_output_goes_on_where_it_left_off() {
    let history: String = (0..30).map(|n| format!("\x1b[3{}mline {n}\x1b[0m\r\n", n % 8)).collect();
    // A prompt in the middle of the screen, with blank rows below it, and the cursor after it.
    let text = format!("{history}\x1b[2J\x1b[Hfirst\r\n\r\n\x1b[1mPS C:\\>\x1b[0m echo hi\x1b[5;3Hx\x1b[3;9H");
    let (mut original, mut taken) = hand_over(40, 8, &text);
    assert_eq!(rows(&mut taken), rows(&mut original), "the screen, cell for cell");
    assert_eq!(taken.cursor_cell(), original.cursor_cell());
    assert_eq!(taken.all_text(), original.all_text(), "the scrollback above it");
    // What the program writes next lands in the same cells in both.
    feed(&mut original, "\x1b[Kmore\r\n\x1b[2;1Hdrawn");
    feed(&mut taken, "\x1b[Kmore\r\n\x1b[2;1Hdrawn");
    assert_eq!(rows(&mut taken), rows(&mut original));
}

#[test]
fn a_handed_over_full_screen_program_keeps_its_screen_and_modes_and_the_shell_under_it() {
    let text = concat!(
        "PS C:\\> claude\r\n",
        "\x1b[?2004h\x1b[?1004h\x1b[?1002h\x1b[?1006h\x1b[?9001h\x1b[?25l",
        "\x1b[?1049h\x1b[>5u\x1b[H\x1b[7m status \x1b[0m\x1b[4;2Hfull screen\x1b[6;10H",
    );
    let (mut original, mut taken) = hand_over(40, 8, text);
    assert_eq!(rows(&mut taken), rows(&mut original), "the alternate screen");
    assert_eq!(taken.cursor_cell(), (5, 9));
    let modes = TermMode::ALT_SCREEN | TermMode::BRACKETED_PASTE | TermMode::FOCUS_IN_OUT | TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE | TermMode::SHOW_CURSOR;
    assert_eq!(taken.mode() & modes, original.mode() & modes);
    assert!(!taken.mode().contains(TermMode::SHOW_CURSOR), "the hidden cursor stays hidden");
    assert_eq!((original.kitty_flags(), taken.kitty_flags()), (5, 5), "the program's kitty keyboard flags");
    assert!(taken.win32_input_mode(), "ConPTY's win32-input-mode");
    // Leaving the program shows the shell under it, as in the original.
    feed(&mut original, "\x1b[?1049l");
    feed(&mut taken, "\x1b[?1049l");
    assert_eq!(snap(&mut taken).row_text(0).trim_end(), "PS C:\\> claude");
    assert_eq!(taken.cursor_cell(), original.cursor_cell());
}
