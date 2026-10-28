//! Scrollback budget, stable line ids for OSC 133 prompt navigation, and resizing under load.

use nekominal_term::{
    CoreConfig, CoreEvent, DEFAULT_SCROLLBACK_BYTES, Damage, LineId, Snapshot, TerminalCore, scrollback_bytes_for,
    alacritty_terminal::{
        grid::{Row, Scroll},
        term::cell::Cell,
    },
    osc::PromptMark,
};

fn core_with(columns: usize, rows: usize, config: CoreConfig) -> TerminalCore {
    TerminalCore::new(columns, rows, config)
}

fn feed(core: &mut TerminalCore, text: &str) {
    core.feed(text.as_bytes());
}

fn snap(core: &mut TerminalCore) -> Snapshot {
    let mut snapshot = Snapshot::default();
    core.snapshot(&mut snapshot);
    snapshot
}

fn row_bytes(columns: usize) -> usize {
    columns * std::mem::size_of::<Cell>() + std::mem::size_of::<Row<Cell>>()
}

/// Ids of the prompt-start marks in `events`, in order.
fn prompt_ids(events: &[CoreEvent]) -> Vec<LineId> {
    events
        .iter()
        .filter_map(|e| match e {
            CoreEvent::PromptMark { mark: PromptMark::PromptStart, cursor } => cursor.and_then(|c| c.line_id),
            _ => None,
        })
        .collect()
}

/// The text of the grid line `line`: scrollback lines are scrolled to the top of the view first.
fn line_text(core: &mut TerminalCore, line: i32) -> String {
    if line < 0 {
        core.scroll_to_line(line);
        let text = snap(core).row_text(0);
        core.scroll_display(Scroll::Bottom);
        text
    } else {
        core.scroll_display(Scroll::Bottom);
        snap(core).row_text(line as usize)
    }
}

// --- scrollback budget -------------------------------------------------------------------------

#[test]
fn the_default_budget_is_64_mib_and_lines_are_the_tighter_limit_on_ordinary_widths() {
    assert_eq!(DEFAULT_SCROLLBACK_BYTES, 64 * 1024 * 1024);
    let config = CoreConfig::default();
    assert_eq!(config.scrollback_bytes, DEFAULT_SCROLLBACK_BYTES);
    let core = core_with(120, 30, config);
    assert_eq!(core.scrollback_limit(), 10_000, "120 columns: 10,000 rows cost about 30 MiB");
}

#[test]
fn the_byte_budget_limits_lines_in_proportion_to_the_width() {
    for columns in [20usize, 80, 200, 400] {
        let budget = 2 * 1024 * 1024;
        let config = CoreConfig { scrollback_lines: 1_000_000, scrollback_bytes: budget, ..CoreConfig::default() };
        let mut core = core_with(columns, 10, config);
        let expected = budget / row_bytes(columns);
        assert_eq!(core.scrollback_limit(), expected, "{columns} columns");
        // Flood well past the limit; every line is distinct so nothing could be merged away.
        let mut flood = String::new();
        for i in 0..expected + 3000 {
            flood.push_str(&format!("{i:08}\r\n"));
        }
        feed(&mut core, &flood);
        assert_eq!(core.history_size(), expected, "{columns} columns: the budget decides, not the line count");
        assert!(core.history_size() * row_bytes(columns) <= budget);
    }
}

#[test]
fn the_line_limit_still_applies_when_the_budget_allows_more() {
    let config = CoreConfig { scrollback_lines: 123, scrollback_bytes: 1 << 30, ..CoreConfig::default() };
    let mut core = core_with(40, 5, config);
    for i in 0..1000 {
        feed(&mut core, &format!("l{i}\r\n"));
    }
    assert_eq!(core.history_size(), 123);
    assert_eq!(core.scrollback_limit(), 123);
}

#[test]
fn the_default_budget_bounds_an_unbounded_line_setting_at_80_columns() {
    // 64 MiB at 80 columns is 34,000+ rows: "unlimited" scrollback cannot grow past that.
    let config = CoreConfig { scrollback_lines: usize::MAX / 4, ..CoreConfig::default() };
    let mut core = core_with(80, 24, config);
    let limit = DEFAULT_SCROLLBACK_BYTES / row_bytes(80);
    assert_eq!(core.scrollback_limit(), limit);
    let mut flood = String::new();
    for i in 0..limit + 5000 {
        flood.push_str(&format!("{i}\r\n"));
    }
    feed(&mut core, &flood);
    assert_eq!(core.history_size(), limit);
}

#[test]
fn growing_wider_shrinks_the_scrollback_to_the_new_budget_and_shrinking_keeps_it() {
    let config = CoreConfig { scrollback_lines: 1_000_000, scrollback_bytes: 1 << 20, ..CoreConfig::default() };
    let mut core = core_with(40, 10, config);
    let narrow = core.scrollback_limit();
    let mut flood = String::new();
    for i in 0..narrow + 500 {
        flood.push_str(&format!("{i}\r\n"));
    }
    feed(&mut core, &flood);
    assert_eq!(core.history_size(), narrow);
    core.take_events();

    core.resize(200, 10);
    let wide = core.scrollback_limit();
    assert!(wide < narrow);
    assert_eq!(core.history_size(), wide, "trimmed to what the budget buys at 200 columns");
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::LineIdsReset)));

    core.resize(40, 10);
    assert_eq!(core.scrollback_limit(), narrow);
    assert_eq!(core.history_size(), wide, "history that is gone does not come back");
}

#[test]
fn a_line_setting_is_kept_whole_in_a_wide_pane_when_its_budget_follows_it() {
    // 30,000 lines at 300 columns are far past the default budget (about 9,000 rows there).
    let config = CoreConfig { scrollback_lines: 30_000, scrollback_bytes: scrollback_bytes_for(30_000), ..CoreConfig::default() };
    let mut core = core_with(300, 5, config);
    assert_eq!(core.scrollback_limit(), 30_000);
    // Raised on a running pane, from the default, the setting is kept whole too.
    let mut running = core_with(300, 5, CoreConfig::default());
    assert!(running.scrollback_limit() < 10_000, "the default budget cuts a wide pane short");
    running.set_scrollback_lines(50_000);
    assert_eq!(running.scrollback_limit(), 50_000);
    core.set_scrollback_lines(0);
    assert_eq!(core.scrollback_limit(), 0);
}

#[test]
fn the_line_limit_of_a_running_terminal_can_change_and_line_ids_stay_exact() {
    let mut core = core_with(30, 5, CoreConfig { scrollback_lines: 100, ..CoreConfig::default() });
    for i in 0..400 {
        feed(&mut core, &format!("l{i}\r\n"));
    }
    assert_eq!(core.history_size(), 100);
    // A smaller limit drops the oldest lines at once; the lines that stay keep their ids.
    core.set_scrollback_lines(40);
    assert_eq!((core.scrollback_limit(), core.history_size()), (40, 40));
    assert_eq!(core.oldest_line_id().map(|id| id.index), Some(356));
    assert_eq!(line_text(&mut core, -40), "l356");
    // A larger one lets the history grow again.
    core.set_scrollback_lines(1000);
    assert_eq!(core.scrollback_limit(), 1000);
    for i in 400..600 {
        feed(&mut core, &format!("l{i}\r\n"));
    }
    assert_eq!(core.history_size(), 240);
    assert_eq!(core.oldest_line_id().map(|id| id.index), Some(356));
    core.set_scrollback_lines(0);
    assert_eq!(core.history_size(), 0);
}

// --- line ids ----------------------------------------------------------------------------------

#[test]
fn line_ids_are_absolute_counts_from_the_first_line() {
    let mut core = core_with(30, 5, CoreConfig { scrollback_lines: 100, ..CoreConfig::default() });
    let mut ids = Vec::new();
    for i in 0..400 {
        // A prompt-start mark at the start of lines 0, 3, 50, 99, 250 and 399.
        let mark = if [0, 3, 50, 99, 250, 399].contains(&i) { "\x1b]133;A\x07" } else { "" };
        feed(&mut core, &format!("{mark}l{i}\r\n"));
        ids.extend(prompt_ids(&core.take_events()).into_iter().map(|id| (i, id)));
    }
    assert_eq!(ids.len(), 6);
    for (line_number, id) in &ids {
        assert_eq!(id.index, *line_number as u64, "the id of 'l{line_number}' is its absolute line number");
        assert_eq!(id.epoch, 0);
    }
    assert!(ids.windows(2).all(|w| w[0].1 < w[1].1), "later lines have larger ids");

    // 400 lines in a 5-row terminal with 100 lines of scrollback: the screen shows l396..l399 and the cursor
    // row, history holds l296..l395, and l0..l295 were trimmed.
    assert_eq!(core.history_size(), 100);
    for (line_number, id) in &ids {
        let line = core.line_of(*id);
        assert_eq!(
            line.is_some(),
            *line_number >= 296,
            "l{line_number} {}",
            if line.is_some() { "resolves" } else { "is gone" }
        );
        if let Some(line) = line {
            assert_eq!(line_text(&mut core, line), format!("l{line_number}"), "id of l{line_number}");
        }
    }
    let oldest = core.oldest_line_id().expect("buffer");
    assert_eq!(oldest.index, 296, "the oldest surviving line is l296");
    assert_eq!(core.line_of(oldest), Some(-100));
    assert_eq!(line_text(&mut core, -100), "l296");
}

#[test]
fn ids_stay_exact_when_a_single_feed_trims_many_times() {
    // 20,000 lines in one `feed` call cross the internal chunk boundary and the trim about 19 times.
    let mut core = core_with(30, 5, CoreConfig { scrollback_lines: 100, ..CoreConfig::default() });
    let mut flood = String::new();
    for i in 0..20_000 {
        let mark = if i == 19_000 || i == 19_990 { "\x1b]133;A\x07" } else { "" };
        flood.push_str(&format!("{mark}l{i}\r\n"));
    }
    feed(&mut core, &flood);
    let ids = prompt_ids(&core.take_events());
    assert_eq!(ids.iter().map(|id| id.index).collect::<Vec<_>>(), [19_000, 19_990]);
    assert_eq!(core.history_size(), 100);
    // The screen shows l19996..l19999 and the cursor row, history l19896..l19995.
    assert_eq!(core.oldest_line_id().map(|id| id.index), Some(19_896));
    assert_eq!(core.line_of(ids[0]), None, "l19000 was trimmed long ago");
    let line = core.line_of(ids[1]).expect("l19990 is in the history");
    assert_eq!(line_text(&mut core, line), "l19990");
}

#[test]
fn an_overflow_past_the_slack_resets_ids_instead_of_guessing() {
    let mut core = core_with(30, 100, CoreConfig { scrollback_lines: 100, ..CoreConfig::default() });
    feed(&mut core, "\x1b]133;A\x07marked\r\n");
    let id = prompt_ids(&core.take_events())[0];
    // `CSI 100 S` scrolls a whole screen in six bytes: 40 of them add 4,000 lines inside one chunk, more than
    // the 2,048 lines the scrollback may overshoot, so lines are dropped that nobody counted.
    feed(&mut core, &"\x1b[100S".repeat(40));
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::LineIdsReset)));
    assert_eq!(core.line_of(id), None);
    assert!(core.history_size() <= core.scrollback_limit());
}

#[test]
fn a_trimmed_line_stops_resolving_exactly_when_it_leaves_the_scrollback() {
    let mut core = core_with(30, 4, CoreConfig { scrollback_lines: 20, ..CoreConfig::default() });
    feed(&mut core, "\x1b]133;A\x07first\r\n");
    let id = prompt_ids(&core.take_events())[0];
    assert_eq!(core.line_of(id), Some(0));
    // The 4-row screen holds 'first', n0, n1 and the cursor row; from n2 on every line scrolls one line into
    // the 20-line scrollback, so after n<k> 'first' sits k - 1 lines above the screen.
    let mut resolves = Vec::new();
    for i in 0..30 {
        feed(&mut core, &format!("n{i}\r\n"));
        resolves.push(core.line_of(id));
    }
    assert_eq!((resolves[0], resolves[1]), (Some(0), Some(0)), "still on screen");
    assert_eq!(resolves[2], Some(-1), "after n2 'first' is the newest scrollback line");
    assert_eq!(resolves[21], Some(-20), "the 20th line of history is the oldest allowed");
    assert_eq!(resolves[22], None, "one more line pushes it out");
    assert!(resolves[22..].iter().all(Option::is_none), "and it never comes back");
}

#[test]
fn line_ids_survive_screen_height_changes() {
    let mut core = core_with(30, 6, CoreConfig::default());
    let mut ids = Vec::new();
    for i in 0..40 {
        feed(&mut core, &format!("\x1b]133;A\x07line{i}\r\n"));
        ids.extend(prompt_ids(&core.take_events()));
    }
    let check = |core: &mut TerminalCore, ids: &[LineId], when: &str| {
        for (i, id) in ids.iter().enumerate() {
            let line = core.line_of(*id).unwrap_or_else(|| panic!("{when}: line{i} lost its id"));
            assert_eq!(line_text(core, line), format!("line{i}"), "{when}");
        }
    };
    check(&mut core, &ids, "before");
    for rows in [10, 3, 12, 6, 2, 20, 6] {
        core.resize(30, rows);
        check(&mut core, &ids, &format!("after resizing to {rows} rows"));
        assert!(core.take_events().iter().all(|e| !matches!(e, CoreEvent::LineIdsReset)), "a height change keeps ids");
    }
}

#[test]
fn ids_reset_when_the_buffer_is_rewritten() {
    let mut core = core_with(30, 5, CoreConfig::default());
    let mark = |core: &mut TerminalCore| {
        feed(core, "\x1b]133;A\x07x\r\n");
        let id = prompt_ids(&core.take_events())[0];
        assert!(core.line_of(id).is_some());
        id
    };

    // reflow after a column change
    let id = mark(&mut core);
    core.resize(20, 5);
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::LineIdsReset)));
    assert_eq!(core.line_of(id), None);
    let fresh = mark(&mut core);
    assert_eq!(fresh.epoch, id.epoch + 1);

    // ED 3 clears the scrollback
    feed(&mut core, "\x1b[3J");
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::LineIdsReset)));
    assert_eq!(core.line_of(fresh), None);

    // RIS
    let id = mark(&mut core);
    feed(&mut core, "\x1bc");
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::LineIdsReset)));
    assert_eq!(core.line_of(id), None);
    let after = mark(&mut core);
    assert_eq!(after.index, 0, "a fresh buffer counts from zero again");
}

#[test]
fn ed3_on_the_alternate_screen_does_not_touch_the_main_scrollback_ids() {
    let mut core = core_with(30, 4, CoreConfig::default());
    feed(&mut core, "\x1b]133;A\x07main\r\n");
    let id = prompt_ids(&core.take_events())[0];
    feed(&mut core, "\x1b[?1049h\x1b[3Jalt");
    assert!(core.take_events().iter().all(|e| !matches!(e, CoreEvent::LineIdsReset)));
    assert_eq!(core.line_of(id), None, "no ids are resolved while the alternate screen is shown");
    assert_eq!(core.line_id(0), None);
    assert_eq!(snap(&mut core).top_line, None);
    feed(&mut core, "\x1b[?1049l");
    assert_eq!(core.line_of(id), Some(0), "back on the main screen the id resolves again");
}

#[test]
fn marks_on_the_alternate_screen_carry_no_id() {
    let mut core = core_with(30, 4, CoreConfig::default());
    feed(&mut core, "\x1b[?1049h\x1b]133;A\x07");
    let events = core.take_events();
    let cursor = events.iter().find_map(|e| match e {
        CoreEvent::PromptMark { cursor, .. } => *cursor,
        _ => None,
    });
    assert_eq!(cursor.map(|c| c.line_id), Some(None));
}

#[test]
fn scroll_to_line_and_top_line_map_between_ids_and_viewport_rows() {
    let mut core = core_with(30, 5, CoreConfig::default());
    let mut ids = Vec::new();
    for i in 0..60 {
        feed(&mut core, &format!("\x1b]133;A\x07L{i}\r\n"));
        ids.extend(prompt_ids(&core.take_events()));
    }
    // Jump to the prompt of L20: it becomes the top row.
    let line = core.line_of(ids[20]).expect("in the buffer");
    core.scroll_to_line(line);
    let s = snap(&mut core);
    assert_eq!(s.row_text(0), "L20");
    assert_eq!(s.row_text(3), "L23");
    let top = s.top_line.expect("main screen");
    assert_eq!(top, ids[20]);
    for row in 0..s.rows {
        assert_eq!(Some(top.offset(row as i64)), core.line_id(row as i32 - s.display_offset as i32), "row {row}");
    }
    assert_eq!(top.offset(5), ids[25], "the id of a row below is the id of its prompt");

    // Beyond the oldest line it stops at the top of the buffer, beyond the newest at the live screen.
    core.scroll_to_line(-100_000);
    assert_eq!(core.display_offset(), core.history_size());
    core.scroll_to_line(1000);
    assert_eq!(core.display_offset(), 0);
}

// --- prompt navigation and clearing --------------------------------------------------------------

/// A shell session: `count` prompts, each followed by one line of output, so prompt `i` is on line `2 * i`.
fn session(columns: usize, rows: usize, config: CoreConfig, count: usize) -> TerminalCore {
    let mut core = core_with(columns, rows, config);
    for i in 0..count {
        feed(&mut core, &format!("\x1b]133;A\x07p{i}> cmd\r\nout {i}\r\n"));
    }
    core
}

fn top_row(core: &mut TerminalCore) -> String {
    snap(core).row_text(0)
}

#[test]
fn the_core_keeps_the_prompts_the_shell_marked_on_their_lines() {
    let mut core = session(30, 5, CoreConfig::default(), 30);
    let lines = core.prompt_lines();
    assert_eq!(lines.len(), 30);
    assert!(lines.windows(2).all(|pair| pair[1] - pair[0] == 2), "one prompt every second line: {lines:?}");
    assert_eq!(lines[0], -56, "60 lines and the cursor row in a 5-row screen: 56 are in the scrollback");
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(line_text(&mut core, *line), format!("p{i}> cmd"), "prompt {i}");
    }
    // Only the start of a prompt is kept, not the other marks, and the same line twice is one prompt.
    feed(&mut core, "\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;0\x07");
    assert_eq!(core.prompt_lines(), lines);
    feed(&mut core, "\x1b]133;A\x07\x1b]133;A\x07");
    assert_eq!(core.prompt_lines().len(), 31);
}

#[test]
fn previous_and_next_prompt_put_the_prompt_on_the_top_row() {
    let mut core = session(30, 5, CoreConfig::default(), 30);
    // The live screen holds p28, its output, p29, its output and the cursor row.
    assert_eq!(core.display_offset(), 0);
    assert!(core.scroll_to_prompt(false), "from the live screen to the prompt above its top row");
    assert_eq!(top_row(&mut core), "p27> cmd");
    assert!(core.scroll_to_prompt(false));
    assert_eq!(top_row(&mut core), "p26> cmd");
    assert!(core.scroll_to_prompt(true));
    assert_eq!(top_row(&mut core), "p27> cmd");
    assert!(core.scroll_to_prompt(true), "p28 is the top row of the live screen");
    assert_eq!((core.display_offset(), top_row(&mut core)), (0, "p28> cmd".to_owned()));
    assert!(!core.scroll_to_prompt(true), "p29 is on the live screen already: nothing to scroll");
    assert_eq!(core.display_offset(), 0);

    core.scroll_display(Scroll::Top);
    assert_eq!(top_row(&mut core), "p0> cmd");
    assert!(!core.scroll_to_prompt(false), "nothing above the oldest prompt");
    assert!(core.scroll_to_prompt(true));
    assert_eq!(top_row(&mut core), "p1> cmd");
}

#[test]
fn next_prompt_with_none_below_returns_to_the_live_screen_and_a_view_between_prompts_finds_its_neighbours() {
    let mut core = session(30, 5, CoreConfig::default(), 30);
    core.scroll_display(Scroll::Delta(5));
    assert_eq!(top_row(&mut core), "out 25", "between p25 and p26");
    assert!(core.scroll_to_prompt(false));
    assert_eq!(top_row(&mut core), "p25> cmd", "the nearest prompt above the top row");
    core.scroll_display(Scroll::Delta(-1));
    assert_eq!(top_row(&mut core), "out 25");
    assert!(core.scroll_to_prompt(true));
    assert_eq!(top_row(&mut core), "p26> cmd", "the nearest prompt below the top row");
    // No prompt after the last one on the screen: the end of the buffer is the live screen.
    let mut core = core_with(30, 5, CoreConfig::default());
    for i in 0..10 {
        feed(&mut core, &format!("\x1b]133;A\x07p{i}\r\n"));
    }
    for i in 0..10 {
        feed(&mut core, &format!("plain {i}\r\n"));
    }
    core.scroll_display(Scroll::Delta(3));
    assert!(core.scroll_to_prompt(true), "no prompt below: the live screen");
    assert_eq!(core.display_offset(), 0);
    assert!(!core.scroll_to_prompt(true), "and then it stays");
}

#[test]
fn prompts_leave_with_their_lines_when_the_scrollback_trims_them_or_the_buffer_is_rewritten() {
    let mut core = session(30, 5, CoreConfig { scrollback_lines: 20, ..CoreConfig::default() }, 30);
    let lines = core.prompt_lines();
    assert!(!lines.is_empty() && lines.len() < 30, "the trimmed prompts are gone: {lines:?}");
    assert!(lines.iter().all(|line| (-20..5).contains(line)), "{lines:?}");
    let oldest = lines[0];
    assert!(line_text(&mut core, oldest).starts_with('p'), "the oldest survivor is still a prompt line");

    // A taller screen keeps them (the ids do), more columns reflow the buffer and lose them.
    let mut core = session(30, 5, CoreConfig::default(), 10);
    let before = core.prompt_lines().len();
    core.resize(30, 9);
    assert_eq!(core.prompt_lines().len(), before);
    core.resize(24, 9);
    assert!(core.prompt_lines().is_empty(), "reflow rewrote the rows");
    assert!(!core.scroll_to_prompt(false));

    let mut core = session(30, 5, CoreConfig::default(), 10);
    feed(&mut core, "\x1b[3J");
    assert!(core.prompt_lines().is_empty(), "ED 3");
    let mut core = session(30, 5, CoreConfig::default(), 10);
    feed(&mut core, "\x1bc");
    assert!(core.prompt_lines().is_empty(), "RIS");
}

#[test]
fn the_alternate_screen_has_no_prompts_and_a_mark_during_a_synchronized_update_has_no_line() {
    let mut core = session(30, 5, CoreConfig::default(), 5);
    let kept = core.prompt_lines();
    feed(&mut core, "\x1b[?1049h\x1b]133;A\x07");
    assert!(core.prompt_lines().is_empty(), "no line is resolved while the alternate screen is shown");
    assert!(!core.scroll_to_prompt(false) && !core.scroll_to_prompt(true));
    feed(&mut core, "\x1b[?1049l");
    assert_eq!(core.prompt_lines(), kept, "the main screen's prompts are back, and the one from the alternate screen is not");
    feed(&mut core, "\x1b[?2026h\x1b]133;A\x07\x1b[?2026l");
    assert_eq!(core.prompt_lines(), kept, "the emulator has not applied the bytes of an open update: its cursor says nothing");
}

#[test]
fn clearing_the_buffer_empties_the_screen_the_scrollback_and_the_prompts() {
    let mut core = session(30, 5, CoreConfig::default(), 30);
    core.scroll_display(Scroll::Delta(10));
    assert!(core.history_size() > 0);
    assert!(core.clear_buffer());
    assert_eq!((core.history_size(), core.display_offset()), (0, 0));
    assert_eq!(snap(&mut core).text(), "");
    assert_eq!(core.cursor_cell(), (0, 0));
    assert!(core.prompt_lines().is_empty());
    assert!(core.take_events().iter().any(|e| matches!(e, CoreEvent::LineIdsReset)));
    // Output goes on from the top; new prompts are kept again.
    feed(&mut core, "\x1b]133;A\x07fresh> \r\n");
    assert_eq!(snap(&mut core).text(), "fresh>");
    assert_eq!(core.prompt_lines(), [0]);
}

#[test]
fn the_alternate_screen_is_not_cleared_because_a_full_screen_program_owns_it() {
    let mut core = session(30, 5, CoreConfig::default(), 5);
    feed(&mut core, "\x1b[?1049h\x1b[HFULL SCREEN");
    assert!(!core.clear_buffer());
    assert_eq!(snap(&mut core).row_text(0), "FULL SCREEN");
    feed(&mut core, "\x1b[?1049l");
    assert!(snap(&mut core).text().contains("p4> cmd"), "and the main screen was not touched either");
}

// --- resizing during heavy output --------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Output of the kind that stresses a resize: long wrapping lines, wide characters, colours, cursor jumps,
/// scroll regions, the alternate screen, prompt marks.
fn burst(rng: &mut Rng) -> Vec<u8> {
    let mut out = String::new();
    for _ in 0..1 + rng.below(40) {
        match rng.below(9) {
            0 => out.push_str(&"x".repeat(1 + rng.below(150))),
            1 => out.push_str("終端機模擬器：你好世界"),
            2 => out.push_str(&format!("\x1b[{};{}m", 30 + rng.below(8), 40 + rng.below(8))),
            3 => out.push_str("\x1b[0m"),
            4 => out.push_str(&format!("\x1b[{};{}H", 1 + rng.below(30), 1 + rng.below(100))),
            5 => out.push_str("\x1b]133;A\x07"),
            6 => out.push_str("\x1b[?1049h"),
            7 => out.push_str("\x1b[?1049l"),
            _ => out.push_str("\r\n"),
        }
    }
    out.into_bytes()
}

#[test]
fn resizing_under_load_keeps_the_cursor_the_damage_and_the_ids_consistent() {
    for seed in 1..=5u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let config = CoreConfig { scrollback_lines: 300, ..CoreConfig::default() };
        let mut core = core_with(60, 20, config);
        let mut snapshot = Snapshot::default();
        let mut resizes = 0;
        let mut marks: Vec<LineId> = Vec::new();

        for step in 0..1500 {
            core.feed(&burst(&mut rng));
            if rng.below(4) == 0 {
                let (columns, rows) = (10 + rng.below(110), 2 + rng.below(40));
                core.resize(columns, rows);
                resizes += 1;
                core.snapshot(&mut snapshot);
                assert_eq!(snapshot.damage, Damage::Full, "seed {seed} step {step}: a resize damages everything");
            }
            for event in core.take_events() {
                match event {
                    CoreEvent::EmulatorPanic(message) => panic!("seed {seed} step {step}: {message}"),
                    CoreEvent::LineIdsReset => marks.clear(),
                    CoreEvent::PromptMark { cursor: Some(c), .. } => {
                        marks.extend(c.line_id);
                    }
                    _ => {}
                }
            }
            core.snapshot(&mut snapshot);
            let (columns, rows) = (core.columns(), core.rows());
            assert_eq!((snapshot.columns, snapshot.rows), (columns, rows), "seed {seed} step {step}");
            assert_eq!(snapshot.cells.len(), columns * rows);
            // The cursor is inside the grid (or hidden because the view is scrolled away: not here).
            let (row, column) = snapshot.cursor.position.expect("not scrolled back");
            assert!(
                row < rows && column < columns,
                "seed {seed} step {step}: cursor ({row},{column}) in {columns}x{rows}"
            );
            // Partial damage never points outside the grid.
            if let Damage::Lines(lines) = &snapshot.damage {
                for l in lines {
                    assert!(l.row < rows && l.left <= l.right && l.right < columns, "seed {seed} step {step}: {l:?}");
                }
            }
            // The scrollback respects the cap whatever the output and the resizes did.
            assert!(core.history_size() <= core.scrollback_limit(), "seed {seed} step {step}");
            // Every id that still resolves round-trips.
            for id in &marks {
                if let Some(line) = core.line_of(*id) {
                    assert_eq!(core.line_id(line), Some(*id), "seed {seed} step {step}");
                }
            }
            // The core's own list of prompts is the marks it announced that still resolve.
            let mut expected: Vec<i32> = marks.iter().filter_map(|id| core.line_of(*id)).collect();
            expected.sort_unstable();
            expected.dedup();
            assert_eq!(core.prompt_lines(), expected, "seed {seed} step {step}");
            // The alternate screen is exited every so often so that main-screen ids are testable.
            if step % 97 == 0 {
                core.feed(b"\x1b[?1049l");
            }
        }
        assert!(resizes > 200, "the storm must actually resize");
        core.feed(b"\x1b[?1049l");
        core.feed(b"after the storm\r\n");
        let s = snap(&mut core);
        assert!(s.text().contains("after the storm"), "seed {seed}: the core still works");
    }
}
