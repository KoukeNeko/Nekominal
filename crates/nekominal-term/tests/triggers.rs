//! Triggers on the emulator: which lines are matched, when, and how often.

use std::sync::Arc;

use nekominal_term::{
    triggers::{TriggerRule, Triggers},
    CoreConfig, CoreEvent, Snapshot, TerminalCore, TriggerMark,
};

fn rule(id: usize, pattern: &str) -> TriggerRule {
    TriggerRule { id, pattern: pattern.into(), instant: false, program: None, highlight: None }
}

fn core(columns: usize, rules: Vec<TriggerRule>) -> TerminalCore {
    let triggers = Arc::new(Triggers::new(rules).unwrap());
    TerminalCore::new(columns, 10, CoreConfig { triggers: Some(triggers), ..CoreConfig::default() })
}

/// The hits of what `core` reported since the last call: (rule, `$0`).
fn hits(core: &mut TerminalCore) -> Vec<(usize, String)> {
    core.take_events()
        .into_iter()
        .filter_map(|event| match event {
            CoreEvent::Trigger(hit) => Some((hit.rule, hit.groups[0].clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_line_is_matched_once_when_it_is_complete() {
    let mut core = core(80, vec![rule(3, r"error: \w+")]);
    core.feed(b"ok\r\nerror: di");
    assert!(hits(&mut core).is_empty(), "the line is still being written");
    core.feed(b"sk\r\n");
    assert_eq!(hits(&mut core), [(3, "error: disk".to_owned())]);
    core.feed(b"more\r\nerror: again\r\n");
    assert_eq!(hits(&mut core), [(3, "error: again".to_owned())], "the first line is not matched twice");
}

#[test]
fn a_wrapped_line_is_one_line() {
    let mut core = core(10, vec![rule(0, "abcdefghijKLM")]);
    core.feed(b"abcdefghijKLM\r\n");
    assert_eq!(hits(&mut core), [(0, "abcdefghijKLM".to_owned())]);
}

#[test]
fn an_instant_rule_matches_the_line_being_written_once() {
    let mut core = core(80, vec![TriggerRule { instant: true, ..rule(1, r"\[y/N\]") }, rule(2, r"\[y/N\]")]);
    core.feed(b"Proceed? [y/");
    assert!(hits(&mut core).is_empty());
    core.feed(b"N] ");
    assert_eq!(hits(&mut core), [(1, "[y/N]".to_owned())]);
    core.feed(b"\x1b[K");
    assert!(hits(&mut core).is_empty(), "once per line");
    core.feed(b"\r\nProceed? [y/N] ");
    assert_eq!(hits(&mut core), [(2, "[y/N]".to_owned()), (1, "[y/N]".to_owned())], "the complete line for the other rule, a new line for this one");
}

#[test]
fn nothing_on_the_alternate_screen_is_matched() {
    let mut core = core(80, vec![rule(0, "frame")]);
    core.feed(b"\x1b[?1049hframe\r\nframe\r\n\x1b[Hframe\r\n");
    assert!(hits(&mut core).is_empty());
    core.feed(b"\x1b[?1049lframe\r\n");
    assert_eq!(hits(&mut core).len(), 1, "the primary screen again");
}

#[test]
fn a_program_rewriting_lines_above_the_cursor_does_not_fire_them_again() {
    let mut core = core(80, vec![rule(0, "Allow")]);
    core.feed(b"Allow edit?\r\n");
    assert_eq!(hits(&mut core).len(), 1);
    core.feed(b"\x1b[1A\x1b[2KAllow edit?\r\n");
    assert!(hits(&mut core).is_empty());
}

#[test]
fn a_rule_of_one_program_matches_only_while_it_runs() {
    let mut core = core(80, vec![TriggerRule { program: Some("claude".into()), ..rule(0, "ask") }]);
    core.feed(b"ask\r\n");
    assert!(hits(&mut core).is_empty(), "the program is not known yet");
    core.set_program(Some("claude.exe".into()));
    core.feed(b"ask\r\n");
    assert_eq!(hits(&mut core).len(), 1);
    core.set_program(Some("pwsh.exe".into()));
    core.feed(b"ask\r\n");
    assert!(hits(&mut core).is_empty());
}

#[test]
fn a_highlight_marks_the_matched_cells_where_the_line_is_shown() {
    let mut core = core(80, vec![TriggerRule { highlight: Some((200, 0, 0)), ..rule(0, "err") }]);
    core.feed(b"one\r\nan err, an err\r\n");
    let mut snapshot = Snapshot::default();
    core.snapshot(&mut snapshot);
    let mark = |start, end| TriggerMark { row: 1, start, end, color: (200, 0, 0) };
    assert_eq!(snapshot.trigger_marks, [mark(3, 6), mark(11, 14)]);
    // The marks follow their line as it scrolls.
    core.feed(b"\r\n\r\n");
    core.snapshot(&mut snapshot);
    assert_eq!(snapshot.trigger_marks.len(), 2);
    core.feed(&b"\r\n".repeat(10));
    core.snapshot(&mut snapshot);
    assert!(snapshot.trigger_marks.is_empty(), "the line went into the scrollback");
    core.scroll_display(nekominal_term::alacritty_terminal::grid::Scroll::Top);
    core.snapshot(&mut snapshot);
    assert_eq!(snapshot.trigger_marks, [mark(3, 6), mark(11, 14)]);
    core.set_triggers(None);
    core.snapshot(&mut snapshot);
    assert!(snapshot.trigger_marks.is_empty(), "new rules, no old highlights");
}

/// The highlights `core` shows now.
fn marks(core: &mut TerminalCore) -> Vec<TriggerMark> {
    let mut snapshot = Snapshot::default();
    core.snapshot(&mut snapshot);
    snapshot.trigger_marks
}

fn lit(id: usize, pattern: &str, color: (u8, u8, u8)) -> TriggerRule {
    TriggerRule { highlight: Some(color), ..rule(id, pattern) }
}

#[test]
fn anchors_hold_for_the_whole_wrapped_line_not_each_row() {
    // "abcdefghij" fills the first row of ten, "KLM" wraps onto the second.
    let mut core = core(10, vec![rule(0, "^KLM"), rule(1, "^abc.*KLM$"), rule(2, "j$")]);
    core.feed(b"abcdefghijKLM\r\n");
    assert_eq!(hits(&mut core), [(1, "abcdefghijKLM".to_owned())]);
}

#[test]
fn case_counts_unless_the_pattern_says_otherwise() {
    let mut core = core(80, vec![rule(0, "Error"), rule(1, "(?i)error")]);
    core.feed(b"error\r\n");
    assert_eq!(hits(&mut core), [(1, "error".to_owned())]);
    core.feed(b"Error\r\n");
    assert_eq!(hits(&mut core), [(0, "Error".to_owned()), (1, "Error".to_owned())]);
}

#[test]
fn every_rule_that_matches_a_line_fires_in_the_order_of_the_rules() {
    let mut core = core(80, vec![rule(5, "b"), rule(2, "a"), rule(9, "c"), rule(1, "z")]);
    core.feed(b"abc\r\n");
    assert_eq!(hits(&mut core), [(5, "b".to_owned()), (2, "a".to_owned()), (9, "c".to_owned())]);
}

#[test]
fn overlapping_highlights_are_all_kept_in_the_order_of_the_rules() {
    let (red, blue) = ((200, 0, 0), (0, 0, 200));
    let mut core = core(80, vec![lit(0, "abcd", red), lit(1, "cdef", blue)]);
    core.feed(b"xabcdefx\r\n");
    // Drawn in this order, the later rule's colour is on top where they overlap.
    assert_eq!(marks(&mut core), [TriggerMark { row: 0, start: 1, end: 5, color: red }, TriggerMark { row: 0, start: 3, end: 7, color: blue }]);
}

#[test]
fn a_highlight_goes_with_its_line_at_the_scrollback_limit() {
    let triggers = Arc::new(Triggers::new(vec![lit(0, "err", (200, 0, 0))]).unwrap());
    let mut core = TerminalCore::new(80, 10, CoreConfig { triggers: Some(triggers), scrollback_lines: 5, ..CoreConfig::default() });
    core.feed(b"err\r\n");
    core.feed(&b"\r\n".repeat(30));
    core.feed(b"err\r\n");
    assert_eq!(marks(&mut core), [TriggerMark { row: 8, start: 0, end: 3, color: (200, 0, 0) }], "only the line still on the screen");
    core.scroll_display(nekominal_term::alacritty_terminal::grid::Scroll::Top);
    assert!(marks(&mut core).is_empty(), "the first line was trimmed, and its highlight lands on no other line");
}

#[test]
fn past_the_most_highlights_kept_the_oldest_go() {
    let mut core = core(80, vec![lit(0, "m", (200, 0, 0))]);
    core.feed(&b"m\r\n".repeat(1100));
    assert_eq!(marks(&mut core).len(), 9, "the newest lines keep theirs");
    core.scroll_display(nekominal_term::alacritty_terminal::grid::Scroll::Top);
    assert!(marks(&mut core).is_empty(), "the oldest went past the cap, though their lines are still there");
}

#[test]
fn a_resize_keeps_the_highlights_unless_the_text_is_reflowed() {
    let mut core = core(80, vec![lit(0, "err", (200, 0, 0))]);
    core.feed(b"err\r\n");
    hits(&mut core);
    core.resize(80, 12);
    assert_eq!(marks(&mut core).len(), 1, "more rows: the lines keep their ids");
    core.resize(60, 12);
    assert!(marks(&mut core).is_empty(), "rewrapped: the cells of the highlight are not known any more");
    core.feed(b"\r\n");
    assert!(hits(&mut core).is_empty(), "the lines that were matched are not matched again");
}

#[test]
fn a_cleared_buffer_drops_the_highlights_and_matches_nothing_again() {
    for clear in [&b"\x1b[3J"[..], b"\x1bc"] {
        let mut core = core(80, vec![lit(0, "err", (200, 0, 0))]);
        core.feed(b"err\r\n");
        assert_eq!(hits(&mut core).len(), 1);
        core.feed(clear);
        assert!(marks(&mut core).is_empty(), "{clear:?}");
        core.feed(b"\r\n");
        assert!(hits(&mut core).is_empty(), "{clear:?}");
    }
}

#[test]
fn an_instant_rule_is_quiet_on_the_alternate_screen_too() {
    let mut core = core(80, vec![TriggerRule { instant: true, ..rule(0, r"\[y/N\]") }]);
    core.feed(b"\x1b[?1049hProceed? [y/N] ");
    assert!(hits(&mut core).is_empty());
}

#[test]
fn a_progress_bar_redrawn_with_cr_is_one_line_matched_with_its_last_text() {
    let mut core = core(80, vec![rule(0, r"\d+%"), TriggerRule { instant: true, ..rule(1, r"5\d%") }]);
    core.feed(b"10%");
    assert!(hits(&mut core).is_empty());
    core.feed(b"\r50%");
    assert_eq!(hits(&mut core), [(1, "50%".to_owned())], "the instant rule on the text as it is now");
    core.feed(b"\r55%");
    assert!(hits(&mut core).is_empty(), "once per line, however often it is redrawn");
    core.feed(b"\r100%\r\n");
    assert_eq!(hits(&mut core), [(0, "100%".to_owned())], "the line is matched once, with its last text");
}

#[test]
fn a_line_is_matched_only_up_to_its_cap() {
    let mut core = core(80, vec![rule(0, "NEEDLE"), rule(1, "TAIL")]);
    let mut line = b"NEEDLE".to_vec();
    line.extend(std::iter::repeat_n(b'a', 3000));
    line.extend(b"TAIL\r\n");
    core.feed(&line);
    assert_eq!(hits(&mut core), [(0, "NEEDLE".to_owned())], "the text past the cap is not read");
}

#[test]
fn a_pattern_that_backtracks_elsewhere_finishes_on_a_long_line() {
    // Exponential in a backtracking engine; the regex crate is linear, so this is quick.
    let mut core = core(80, vec![rule(0, r"(a+)+$"), rule(1, r"(a|aa)+c")]);
    let mut line = vec![b'a'; 2000];
    line.extend(b"b\r\n");
    core.feed(&line);
    assert!(hits(&mut core).is_empty());
}

#[test]
fn a_pattern_that_matches_nothing_fires_on_every_line_but_blank_ones() {
    let mut core = core(80, vec![rule(0, "x*")]);
    core.feed(b"one\r\n\r\ntwo\r\n");
    assert_eq!(hits(&mut core), [(0, String::new()), (0, String::new())]);
}

#[test]
fn new_rules_match_only_new_lines() {
    let mut core = core(80, vec![rule(0, "nothing")]);
    core.feed(b"old\r\n");
    core.set_triggers(Some(Arc::new(Triggers::new(vec![rule(1, "old|new")]).unwrap())));
    core.feed(b"new\r\n");
    assert_eq!(hits(&mut core), [(1, "new".to_owned())]);
}

#[test]
fn without_triggers_nothing_is_matched() {
    let mut core = TerminalCore::new(80, 10, CoreConfig::default());
    core.feed(b"error\r\n");
    assert!(hits(&mut core).is_empty());
}
