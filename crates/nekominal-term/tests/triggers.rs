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

#[test]
fn without_triggers_nothing_is_matched() {
    let mut core = TerminalCore::new(80, 10, CoreConfig::default());
    core.feed(b"error\r\n");
    assert!(hits(&mut core).is_empty());
}
