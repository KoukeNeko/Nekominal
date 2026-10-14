//! Claude Code through ConPTY and the emulator, with the same wiring as the pane host.
//!
//! These tests start the installed `claude.exe` (see `claude::exe`), so they are ignored by default. They never
//! submit a prompt: the probe's guard refuses any Enter unless the input box holds a local slash command, and
//! the child gets an allow-listed environment (`agent_probe::env`). Run them with
//!
//! `cargo test -p agent-probe --test claude_code -- --ignored --nocapture --test-threads=1`
//!
//! Every check prints `RESULT <id> | <pass|FAIL|gap|info> | <evidence>` and appends it to
//! `target/agent-probe/results.txt` (emptied at the start of each run); `docs/agent-cli-compat.md` is written
//! from those lines. Raw byte taps and scenario logs are in `target/agent-probe/` as well and can hold account
//! details: they are never committed.

use std::{
    fmt::Display,
    fs::OpenOptions,
    io::Write,
    sync::{Mutex, MutexGuard, Once},
    thread::sleep,
    time::{Duration, Instant},
};

use agent_probe::{
    claude::{self, Started},
    inventory::{Answer, Group},
    probe::{Guard, KeyRouting, Observed, Probe, SpawnOptions, default_tap_dir},
    screen::Screen,
    scrub::{scrub, scrub_screen},
};
use nekominal_input::{MouseButton, MouseInput, MouseKind, WheelDirection};
use nekominal_term::agent_status::{AgentState, Report};

// ---- plumbing ----------------------------------------------------------------------------------------

/// One `claude.exe` at a time, whatever `--test-threads` says.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn exclusive() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn pause(ms: u64) {
    sleep(Duration::from_millis(ms));
}

const SECOND: Duration = Duration::from_secs(1);

/// The results of this run: the file is emptied by the first check of the process.
fn record(id: &str, status: &str, evidence: &str) {
    static FRESH: Once = Once::new();
    let line = format!("RESULT {id} | {status} | {}", scrub(evidence).replace('\n', " / "));
    println!("{line}");
    let path = default_tap_dir().join("results.txt");
    FRESH.call_once(|| {
        let _ = std::fs::write(&path, "");
    });
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}
#[derive(Default)]
struct Checks {
    failures: Vec<String>,
}

impl Checks {
    fn check(&mut self, id: &str, ok: bool, evidence: impl Display) {
        let evidence = evidence.to_string();
        record(id, if ok { "pass" } else { "FAIL" }, &evidence);
        if !ok {
            self.failures.push(format!("{id}: {evidence}"));
        }
    }

    /// A difference from what a terminal should do that is known and reported, not a regression.
    fn gap(&mut self, id: &str, evidence: impl Display) {
        record(id, "gap", &evidence.to_string());
    }

    fn info(&mut self, id: &str, evidence: impl Display) {
        record(id, "info", &evidence.to_string());
    }

    fn finish(self) {
        assert!(self.failures.is_empty(), "{} check(s) failed:\n{}", self.failures.len(), self.failures.join("\n"));
    }
}

fn fullscreen(scenario: &str) -> SpawnOptions {
    claude::fullscreen_options(scenario)
}

fn classic(scenario: &str) -> SpawnOptions {
    claude::options(scenario).env("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN", "1")
}

fn start(options: SpawnOptions) -> Started {
    claude::start(options).unwrap_or_else(|error| panic!("{error}"))
}

fn box_text(probe: &Probe) -> String {
    claude::input_box(&probe.screen()).map(|found| found.text).unwrap_or_default()
}

fn wait_box(probe: &Probe, expected: &str) {
    probe
        .wait_until(&format!("the input box to read {expected:?}"), 5 * SECOND, |s| {
            claude::input_box(s).is_some_and(|b| b.text == expected)
        })
        .unwrap_or_else(|error| panic!("{error}"));
}

/// Empties the input box the way a user does: Escape twice, again if a menu swallowed the first.
fn clear_box(probe: &Probe) {
    for _ in 0..4 {
        if claude::input_box(&probe.screen()).is_some_and(|b| b.text.is_empty()) {
            return;
        }
        probe.key("esc").unwrap();
        pause(150);
        probe.key("esc").unwrap();
        pause(450);
    }
    panic!("the input box would not clear:\n{}", probe.screen().dump_scrubbed());
}

/// The row of the rule that closes the input box.
fn box_bottom(screen: &Screen) -> Option<usize> {
    let found = claude::input_box(screen)?;
    (found.row + 1..screen.rows).find(|&row| screen.line(row).starts_with('─'))
}

/// Text that differs between two runs of the CLI without anything being wrong: the hint in an empty input box
/// is random, hooks finish in any order, the start banner shows a tip for a while.
fn normalize(line: &str) -> String {
    if line.starts_with("❯\u{a0}Try ") {
        return "❯ <hint>".to_owned();
    }
    if line.contains("[✻]") || line.contains("weekly limit") {
        return String::new();
    }
    line.replace("bash", "<hook>").replace("node", "<hook>")
}

fn restored_modes(checks: &mut Checks, id: &str, probe: &Probe) {
    let modes = probe.modes();
    checks.check(
        id,
        !modes.alt_screen
            && !modes.mouse_tracking()
            && !modes.bracketed_paste
            && modes.kitty_flags == 0
            && modes.show_cursor
            && !modes.sync_pending,
        format!("after exit: {}", modes.summary()),
    );
}

fn answered(probe: &Probe, group: Group) -> bool {
    probe.with_inventory(|inventory| matches!(inventory.answer_for(group), Answer::Answered { .. }))
}

fn unanswered_names(probe: &Probe) -> Vec<String> {
    probe.with_inventory(|inventory| inventory.unanswered().iter().map(|group| group.name()).collect())
}

/// Startup facts shared by the classic and the fullscreen scenario.
fn check_startup(checks: &mut Checks, prefix: &str, started: &Started) {
    let probe = &started.probe;
    let screen = probe.screen();
    let version = claude::banner_version(&screen);
    checks.check(
        &format!("{prefix}.main-ui"),
        version.is_some() && claude::input_box(&screen).is_some() && claude::blocking_screen(&screen).is_none(),
        format!(
            "banner Claude Code v{}, input box on row {:?}; input box after {:?}, settled after {:?}",
            version.as_deref().unwrap_or("?"),
            claude::input_box(&screen).map(|b| b.row),
            started.prompt_after,
            started.stable_after
        ),
    );
    let modes = probe.modes();
    checks.check(&format!("{prefix}.bracketed-paste"), modes.bracketed_paste, "DECSET 2004 set");
    checks.check(&format!("{prefix}.focus-reporting"), modes.focus_reporting, "DECSET 1004 set");
    checks.check(
        &format!("{prefix}.kitty-keyboard"),
        modes.kitty_flags & 1 == 1,
        format!("CSI > 5 u pushed, active flags {:#07b}", modes.kitty_flags),
    );
    checks.check(&format!("{prefix}.sync-idle"), !modes.sync_pending, "no synchronized update left open when idle");
    checks.check(
        &format!("{prefix}.cursor-visible"),
        modes.show_cursor && probe.cursor().is_some(),
        format!("cursor at {:?} while idle", probe.cursor()),
    );

    probe.with_inventory(|inventory| {
        let (set, reset) = (inventory.count("DECSET ?2026"), inventory.count("DECRST ?2026"));
        checks.check(
            &format!("{prefix}.sync-balance"),
            set == reset && set > 0,
            format!("DECSET 2026 x{set}, DECRST 2026 x{reset}"),
        );
        checks.check(
            &format!("{prefix}.reply-order"),
            inventory.replies_follow_query_order(),
            "every reply came in the order of its query".to_owned(),
        );
    });
    // CSI 16 t (cell size in pixels) is the only query Nekominal leaves unanswered by policy.
    let unanswered = unanswered_names(probe);
    checks.check(
        &format!("{prefix}.queries-answered"),
        unanswered.iter().all(|name| name == "XTWINOPS 16"),
        format!("unanswered queries: {unanswered:?}; every other query was answered"),
    );
    for (name, group) in [
        ("DA1", Group::Da1),
        ("XTVERSION", Group::Xtversion),
        ("kitty keyboard ?u", Group::KittyKeyboard),
        ("DECRQM 2026", Group::DecrqmPrivate(2026)),
        ("OSC 7501 ?", Group::Osc7501),
    ] {
        checks.check(
            &format!("{prefix}.answers.{}", name.replace(' ', "-")),
            answered(probe, group),
            format!("{name} is answered"),
        );
    }
    let never_sent: Vec<String> = [
        Group::Da2,
        Group::Da3,
        Group::Xtwinops(18),
        Group::OscColor(10),
        Group::OscColor(11),
        Group::OscColor(4),
        Group::DecrqmPrivate(2004),
        Group::DecrqmPrivate(1004),
        Group::DecrqmPrivate(2027),
        Group::ColorScheme,
    ]
    .into_iter()
    .filter(|group| probe.with_inventory(|inventory| matches!(inventory.answer_for(*group), Answer::NotAQuery)))
    .map(|group| group.name())
    .collect();
    checks.info(&format!("{prefix}.not-sent"), format!("not sent by this version: {never_sent:?}"));

    let events = probe.all_events();
    let status = events.iter().find_map(|(_, event)| match event {
        Observed::Agent(Report::Set(record)) => Some(record.clone()),
        _ => None,
    });
    checks.check(
        &format!("{prefix}.osc-7501"),
        status.as_ref().is_some_and(|r| r.state == AgentState::Idle && r.app.as_deref() == Some("claude-code")),
        format!("program status after the reply to OSC 7501 ?: {:?}", status.map(|r| (r.state, r.app))),
    );
    checks.check(
        &format!("{prefix}.no-emulator-panic"),
        !events.iter().any(|(_, e)| matches!(e, Observed::EmulatorPanic(_))),
        "the emulator never had to recover from a panic".to_owned(),
    );
}

/// Opens /help, reads the OSC 8 target under the documentation URL and closes the dialog again.
fn help_link(probe: &Probe) -> Option<String> {
    claude::run_local_command(probe, "/help").unwrap();
    probe.wait_for_text("For more help", 5 * SECOND).unwrap_or_else(|e| panic!("{e}"));
    pause(400);
    let screen = probe.screen();
    let row = screen.row_containing("code.claude.com", 0).expect("the documentation URL in the help dialog");
    let column = screen.column_of(row, "https").expect("the URL scheme") + 3;
    let link = probe.hyperlink_at(row, column);
    probe.key("esc").unwrap();
    probe
        .wait_until("the input box after the dialog", 5 * SECOND, |s| claude::input_box(s).is_some())
        .unwrap_or_else(|e| panic!("{e}"));
    link
}

fn exit_with(probe: &Probe, how: &str) {
    match how {
        "/exit" => claude::run_local_command(probe, "/exit").unwrap(),
        chord => {
            probe.key(chord).unwrap();
            pause(500);
            probe.key(chord).unwrap();
        }
    }
}

// ---- startup, modes and exit -------------------------------------------------------------------------

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn fullscreen_startup_queries_modes_and_exit() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    let started = start(fullscreen("fs-startup").size(120, 30));
    let probe = &started.probe;
    check_startup(&mut checks, "fs", &started);

    let modes = probe.modes();
    checks.check("fs.alt-screen", modes.alt_screen, "DECSET 1049 entered, the grid is the alternate screen");
    checks.check(
        "fs.mouse-capture",
        format!("{:?}/{:?}", modes.mouse_mode, modes.mouse_encoding) == "Any/Sgr",
        format!("1000, 1002, 1003 and 1006 set: {:?}/{:?}", modes.mouse_mode, modes.mouse_encoding),
    );
    let screen = probe.screen();
    let found = claude::input_box(&screen).expect("input box");
    let bottom = box_bottom(&screen).expect("closing rule");
    checks.check(
        "fs.box-pinned",
        bottom + 3 >= screen.rows && found.row + 6 >= screen.rows,
        format!("box on row {} of {}, closing rule on row {bottom}, the status line below it", found.row, screen.rows),
    );
    checks.check(
        "fs.no-scrollback",
        probe.history_size() == 0,
        format!("the alternate screen holds {} lines of scrollback", probe.history_size()),
    );
    checks.check(
        "fs.no-wide-cell-problems",
        screen.wide_cell_problems().is_empty(),
        "wide-character cells are consistent",
    );

    let before = agent_probe::sys::process(probe.pid());
    pause(3000);
    let after = agent_probe::sys::process(probe.pid());
    if let (Some(a), Some(b)) = (before, after) {
        let cpu = b.cpu.saturating_sub(a.cpu);
        checks.check(
            "fs.idle-cpu",
            cpu < 2 * SECOND,
            format!("{cpu:?} of CPU in 3 s idle, working set {} MiB", b.working_set >> 20),
        );
    }

    // The same screen must come back after Ctrl+L (redraw).
    probe.key("ctrl+l").unwrap();
    pause(700);
    let redrawn = probe.screen();
    let diffs = screen.diff(&redrawn, &normalize);
    checks.check(
        "fs.ctrl-l-redraw",
        diffs.is_empty(),
        format!("{} differences after Ctrl+L, e.g. {:?}", diffs.len(), diffs.first()),
    );

    let link = help_link(probe);
    checks.info(
        "fs.osc-8-default",
        format!("documentation URL in /help: OSC 8 target {link:?}, OSC 8 sequences {}; the CLI does not emit them for TERM_PROGRAM=Nekominal without FORCE_HYPERLINK=1", probe.with_inventory(|i| i.count("OSC 8"))),
    );

    exit_with(probe, "/exit");
    let code = probe.wait_exit(10 * SECOND).map_err(|e| e.to_string());
    checks.check("fs.exit", code == Ok(0), format!("/exit ended the program with {code:?}"));
    restored_modes(&mut checks, "fs.modes-restored", probe);
    checks.check(
        "fs.pen-restored",
        matches!(probe.leftover_attributes(), Ok(None)),
        format!("attributes left after exit: {:?}", probe.leftover_attributes()),
    );
    let pid = probe.pid();
    probe.shutdown();
    pause(500);
    checks.check("fs.process-gone", !agent_probe::sys::is_running(pid), "the process is gone after shutdown");
    probe.with_inventory(|inventory| {
        let (pushed, popped) = (inventory.count("CSI > 5 u"), inventory.count("CSI < u"));
        checks.check("fs.kitty-balance", popped >= pushed, format!("kitty flags pushed x{pushed}, popped x{popped}"));
    });
    let states: Vec<String> = probe
        .all_events()
        .iter()
        .filter_map(|(_, event)| match event {
            Observed::Agent(Report::Set(record)) => Some(format!("{:?}", record.state)),
            Observed::Agent(Report::Clear { .. }) => Some("clear".to_owned()),
            Observed::AgentRecordsDropped => Some("records dropped at exit".to_owned()),
            _ => None,
        })
        .collect();
    checks.info("fs.osc-7501-sequence", format!("program status reports over the session: {states:?}"));
    let mut kinds: Vec<String> = probe
        .all_events()
        .iter()
        .map(|(_, e)| format!("{e:?}").split(['(', ' ']).next().unwrap_or("").to_owned())
        .collect();
    kinds.sort();
    kinds.dedup();
    checks.info("fs.host-events", format!("kinds of events the program caused: {kinds:?}"));
    checks.info("fs.guard", format!("{:?}", probe.guard_stats()));
    println!("{}", probe.inventory_report());
    checks.finish();
}

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn classic_startup_modes_and_exit() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    let started = start(classic("cl-startup").size(120, 30));
    let probe = &started.probe;
    check_startup(&mut checks, "cl", &started);
    let modes = probe.modes();
    checks.check(
        "cl.main-screen",
        !modes.alt_screen,
        "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 stays on the main screen",
    );
    checks.check("cl.no-mouse-capture", !modes.mouse_tracking(), format!("mouse mode {:?}", modes.mouse_mode));
    exit_with(probe, "/exit");
    let code = probe.wait_exit(10 * SECOND).map_err(|e| e.to_string());
    checks.check("cl.exit", code == Ok(0), format!("/exit ended the program with {code:?}"));
    restored_modes(&mut checks, "cl.modes-restored", probe);
    checks.check(
        "cl.pen-restored",
        matches!(probe.leftover_attributes(), Ok(None)),
        format!("{:?}", probe.leftover_attributes()),
    );
    probe.shutdown();
    checks.finish();
}

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn classic_scrollback_belongs_to_the_terminal() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    // Eight rows: the start banner and the box do not fit, so part of the banner is in the scrollback.
    let started = start(classic("cl-scrollback").size(100, 8));
    let probe = &started.probe;
    let bottom = probe.screen();
    checks.check(
        "cl.scrollback.history",
        probe.history_size() > 0,
        format!("{} lines in the scrollback after the start", probe.history_size()),
    );
    probe.scroll_view(2);
    let older = probe.screen();
    checks.check(
        "cl.scrollback.view",
        probe.display_offset() == 2 && older.lines != bottom.lines,
        format!("scrolling the view by two lines shows older text (offset {})", probe.display_offset()),
    );
    // No tracking mode, main screen: the wheel is the terminal's, the program never sees it.
    let wheel = probe.send_mouse(&MouseInput::new(MouseKind::Wheel(WheelDirection::Up), 5, 3)).unwrap();
    checks.check(
        "cl.scrollback.wheel-is-the-terminals",
        wheel.is_none(),
        format!("wheel report for the program: {wheel:?}"),
    );
    probe.scroll_view(-100);
    checks.check(
        "cl.scrollback.back-to-bottom",
        probe.display_offset() == 0 && probe.screen().lines == bottom.lines,
        "scrolling back down shows the live screen again",
    );
    probe.shutdown();
    checks.finish();
}

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn ctrl_c_and_ctrl_d_exit_after_a_second_press() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    for (id, chord, hint) in
        [("ctrl-c", "ctrl+c", "Press Ctrl-C again to exit"), ("ctrl-d", "ctrl+d", "Press Ctrl-D again to exit")]
    {
        let started = start(fullscreen(&format!("fs-exit-{id}")).size(120, 30));
        let probe = &started.probe;
        probe.key(chord).unwrap();
        let armed = probe.wait_for_text(hint, 5 * SECOND).is_ok();
        checks.check(
            &format!("exit.{id}.armed"),
            armed && probe.exit_code().is_none(),
            format!("the first {chord} shows {hint:?} and keeps running"),
        );
        probe.key(chord).unwrap();
        let code = probe.wait_exit(10 * SECOND).map_err(|e| e.to_string());
        checks.check(
            &format!("exit.{id}.exits"),
            code == Ok(0),
            format!("the second {chord} ends the program with {code:?}"),
        );
        restored_modes(&mut checks, &format!("exit.{id}.modes-restored"), probe);
        probe.shutdown();
    }
    checks.finish();
}

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn fullscreen_switches_that_change_mouse_cursor_and_repaint() {
    let _turn = exclusive();
    let mut checks = Checks::default();

    // CLAUDE_CODE_DISABLE_MOUSE=1: flicker-free rendering without mouse capture.
    let started = start(fullscreen("fs-no-mouse").env("CLAUDE_CODE_DISABLE_MOUSE", "1"));
    let modes = started.probe.modes();
    checks.check(
        "switch.disable-mouse",
        modes.alt_screen && !modes.mouse_tracking(),
        format!("alt screen kept, no mouse tracking: {}", modes.summary()),
    );
    // With no tracking mode the wheel on the alternate screen turns into cursor keys (DECSET 1007), the
    // pane host's rule; the CLI then sees arrows, not wheel reports.
    let wheel = started.probe.send_mouse(&MouseInput::new(MouseKind::Wheel(WheelDirection::Up), 10, 5)).unwrap();
    checks.check(
        "switch.wheel-as-arrows",
        wheel.as_deref() == Some(b"\x1b[A"),
        format!("wheel up becomes {:?}", wheel),
    );
    started.probe.shutdown();
    drop(started);

    // The terminal's own cursor at the input caret: what Nekominal enables for IME composition.
    let started = start(fullscreen("fs-native-cursor").agent_environment());
    let probe = &started.probe;
    probe.send_text("ab").unwrap();
    wait_box(probe, "ab");
    let found = claude::input_box(&probe.screen()).unwrap();
    checks.check(
        "switch.native-cursor",
        probe.modes().show_cursor && probe.cursor() == Some((found.row, 4)),
        format!("CLAUDE_CODE_NATIVE_CURSOR=1: cursor {:?}, caret expected at ({}, 4)", probe.cursor(), found.row),
    );
    clear_box(probe);
    let link = help_link(probe);
    checks.check(
        "switch.force-hyperlink",
        link.as_deref().is_some_and(|uri| uri.starts_with("https://code.claude.com/")),
        format!("FORCE_HYPERLINK=1: the documentation URL in /help carries the OSC 8 target {link:?}"),
    );
    probe.shutdown();
    drop(started);

    // Full repaint on every frame (the documented answer to stale fragments under ConPTY hosts).
    let started = start(fullscreen("fs-full-repaint").env("CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT", "1"));
    let probe = &started.probe;
    probe.send_text("full repaint").unwrap();
    wait_box(probe, "full repaint");
    let screen = probe.screen();
    checks.check(
        "switch.full-repaint",
        screen.wide_cell_problems().is_empty() && box_bottom(&screen).is_some() && probe.modes().alt_screen,
        "CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT=1 draws a consistent screen",
    );
    probe.shutdown();
    checks.finish();
}

// ---- input -------------------------------------------------------------------------------------------

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn keys_text_paste_and_cell_widths_in_the_input_box() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    let started = start(fullscreen("fs-input").size(120, 30));
    let probe = &started.probe;
    let box_row = claude::input_box(&probe.screen()).expect("input box").row;
    let caret = |columns: usize| Some((box_row, 2 + columns));

    // Plain text and the cursor at the caret.
    probe.send_text("hello").unwrap();
    wait_box(probe, "hello");
    checks.check(
        "input.text-and-caret",
        probe.cursor() == caret(5),
        format!("cursor {:?}, expected {:?}", probe.cursor(), caret(5)),
    );

    // Backspace, arrows, Home/End, Delete, Ctrl+A/E/U/Y.
    probe.key("backspace").unwrap();
    probe.key("backspace").unwrap();
    wait_box(probe, "hel");
    checks.check("input.backspace", true, "two Backspace presses removed two characters (0x7f)");
    probe.key("left").unwrap();
    probe.send_text("X").unwrap();
    wait_box(probe, "heXl");
    probe.key("home").unwrap();
    probe.send_text("A").unwrap();
    wait_box(probe, "AheXl");
    probe.key("end").unwrap();
    pause(300);
    let at_end = probe.cursor() == caret(5);
    probe.key("ctrl+a").unwrap();
    pause(300);
    let at_start = probe.cursor() == caret(0);
    probe.key("ctrl+e").unwrap();
    pause(300);
    let at_end_again = probe.cursor() == caret(5);
    probe.key("home").unwrap();
    probe.key("delete").unwrap();
    wait_box(probe, "heXl");
    checks.check(
        "input.navigation",
        at_end && at_start && at_end_again,
        format!("End {at_end}, Ctrl+A {at_start}, Ctrl+E {at_end_again}; Delete removed the first character"),
    );
    probe.key("end").unwrap();
    probe.key("ctrl+u").unwrap();
    wait_box(probe, "");
    probe.key("ctrl+y").unwrap();
    wait_box(probe, "heXl");
    checks.check("input.ctrl-u-ctrl-y", true, "Ctrl+U deleted to the line start and Ctrl+Y brought it back");
    clear_box(probe);

    // Word movement and deletion: Ctrl+Left and Ctrl+Backspace (the CLI reads a ^H byte as Ctrl+Backspace on
    // Windows, so the terminal must send the CSI u form for it and a plain 0x7f for Backspace).
    probe.send_text("alpha beta").unwrap();
    wait_box(probe, "alpha beta");
    let left_bytes = probe.key("ctrl+left").map(|b| agent_probe::probe::escape_bytes(&b)).unwrap();
    pause(400);
    let word_start = probe.cursor() == caret(6);
    checks.check(
        "input.ctrl-left",
        word_start,
        format!("Ctrl+Left sent {left_bytes}; cursor {:?}, expected {:?}", probe.cursor(), caret(6)),
    );
    probe.key("end").unwrap();
    let delete_bytes = probe.key("ctrl+backspace").map(|b| agent_probe::probe::escape_bytes(&b)).unwrap();
    let word_deleted = probe
        .wait_until("the last word deleted", 3 * SECOND, |s| claude::input_box(s).is_some_and(|b| b.text == "alpha"))
        .is_ok();
    checks.check(
        "input.ctrl-backspace",
        word_deleted,
        format!("Ctrl+Backspace sent {delete_bytes}; the box reads {:?}", box_text(probe)),
    );
    clear_box(probe);

    // Shift+Enter and Alt+Enter insert a newline (the guard lets them through only while the box holds a
    // local command, so a bare Enter that submitted would run /help, not a prompt).
    for (id, chord, bytes) in [("shift-enter", "shift+enter", "ESC[13;2u"), ("alt-enter", "alt+enter", "ESC[13;3u")] {
        claude::type_command(probe, "/help").unwrap();
        let sent = probe.key(chord).map(|b| agent_probe::probe::escape_bytes(&b)).unwrap();
        pause(500);
        let after = claude::input_box(&probe.screen()).map(|b| b.text);
        let dialog = probe.screen().contains("Shortcuts");
        checks.check(
            &format!("input.{id}"),
            sent == bytes && after.as_deref() == Some("/help\n") && !dialog,
            format!("{chord} sent {sent}; the box reads {after:?}; help dialog opened: {dialog}"),
        );
        clear_box(probe);
    }

    // Escape closes the slash menu and keeps the text; twice clears the box.
    probe.send_text("/he").unwrap();
    probe.wait_for_text("Show help", 5 * SECOND).unwrap();
    probe.key("esc").unwrap();
    let closed = probe.wait_for_gone("Show help", 3 * SECOND).is_ok();
    checks.check(
        "input.esc-closes-menu",
        closed && box_text(probe) == "/he",
        format!("menu closed: {closed}, box {:?}", box_text(probe)),
    );
    clear_box(probe);

    // Ctrl+C with text clears the box and arms the exit; nothing exits.
    probe.send_text("abc").unwrap();
    wait_box(probe, "abc");
    probe.key("ctrl+c").unwrap();
    let armed = probe.wait_for_text("Press Ctrl-C again to exit", 3 * SECOND).is_ok();
    checks.check(
        "input.ctrl-c-clears",
        armed && box_text(probe).is_empty() && probe.exit_code().is_none(),
        format!("armed {armed}, box {:?}, still running", box_text(probe)),
    );
    probe.wait_for_gone("Press Ctrl-C again to exit", 6 * SECOND).unwrap();

    // Bracketed paste: three lines stay in the box and are not submitted; twelve collapse to a placeholder.
    let bytes = probe.send_paste("alpha\nbeta\ngamma").unwrap();
    wait_box(probe, "alpha\nbeta\ngamma");
    checks.check(
        "input.paste-bracketed",
        bytes.starts_with(b"\x1b[200~") && bytes.ends_with(b"\x1b[201~"),
        format!("paste wrapped in 2004 markers: {}", agent_probe::probe::escape_bytes(&bytes)),
    );
    clear_box(probe);
    let long: String = (1..=12).map(|n| format!("line number {n}\n")).collect();
    probe.send_paste(&long).unwrap();
    wait_box(probe, "[Pasted text #1 +12 lines]");
    // The only Enter-like keys sent so far are the Shift/Alt+Enter presses with /help in the box.
    let only_newlines = probe.guard_stats().submits_sent.iter().all(|sent| sent.ends_with(": /help"));
    checks.check(
        "input.paste-collapsed",
        only_newlines,
        "a 12-line paste became [Pasted text #1 +12 lines]; a paste never sends Enter",
    );
    probe.key("ctrl+u").unwrap();
    clear_box(probe);

    // Wide, emoji and combining characters: the cell widths of the CLI and of the emulator agree.
    let mixed = "你好ㄅㄆㄇ😀e\u{301}ab";
    probe.send_text(mixed).unwrap();
    probe
        .wait_until("the mixed text in the box", 5 * SECOND, |s| {
            claude::input_box(s).is_some_and(|b| b.text.ends_with("ab"))
        })
        .unwrap_or_else(|error| panic!("{error}"));
    pause(300);
    let screen = probe.screen();
    let row = box_row;
    let ab = screen.column_of(row, "ab");
    checks.check(
        "input.cell-widths",
        ab == Some(15) && probe.cursor() == caret(15) && screen.wide_cell_problems().is_empty(),
        format!(
            "'ab' at column {ab:?} (expected 15), cursor {:?} (expected col 17), wide-cell problems {:?}",
            probe.cursor(),
            screen.wide_cell_problems()
        ),
    );
    checks.check(
        "input.combining",
        screen.line(row).contains("e\u{301}ab") || screen.line(row).contains("\u{e9}ab"),
        format!("row text {:?}", screen.line(row)),
    );
    for _ in 0..3 {
        probe.key("backspace").unwrap();
    }
    wait_box(probe, "你好ㄅㄆㄇ😀");
    checks.check(
        "input.backspace-grapheme",
        probe.cursor() == caret(12),
        format!("after three Backspaces the cursor is at {:?}, expected col 14", probe.cursor()),
    );
    probe.key("backspace").unwrap();
    wait_box(probe, "你好ㄅㄆㄇ");
    clear_box(probe);

    // Zhuyin-composed text reaches the program as plain UTF-8, one commit at a time or in one piece.
    for commit in ["你", "好", "，", "ㄋ", "嗎"] {
        probe.send_text(commit).unwrap();
    }
    wait_box(probe, "你好，ㄋ嗎");
    checks.check(
        "input.ime-commits",
        probe.cursor() == caret(10),
        format!("five commits typed, cursor {:?}, expected col 12", probe.cursor()),
    );
    clear_box(probe);

    // Focus reports.
    let (out, back) = (probe.send_focus(false), probe.send_focus(true));
    pause(300);
    checks.check(
        "input.focus-reports",
        out.as_deref() == Some(b"\x1b[O")
            && back.as_deref() == Some(b"\x1b[I")
            && claude::input_box(&probe.screen()).is_some(),
        format!(
            "focus out {:?}, focus in {:?}",
            out.map(|b| agent_probe::probe::escape_bytes(&b)),
            back.map(|b| agent_probe::probe::escape_bytes(&b))
        ),
    );

    // The key bytes of the key assembler and of what the pane host sends today must agree for the keys in use.
    let mut differing = Vec::new();
    for chord in [
        "shift+enter",
        "alt+enter",
        "backspace",
        "delete",
        "esc",
        "tab",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "pgup",
        "pgdn",
        "ctrl+a",
        "ctrl+c",
        "ctrl+d",
        "ctrl+e",
        "ctrl+l",
        "ctrl+o",
        "ctrl+u",
        "ctrl+y",
        "ctrl+end",
        "ctrl+home",
        "alt+v",
        "alt+p",
    ] {
        let spec = agent_probe::KeySpec::parse(chord).unwrap();
        let assembler = probe.predict_key_with(&spec, KeyRouting::ConptyRoute).unwrap();
        let host = probe.predict_key_with(&spec, KeyRouting::PaneHost).unwrap();
        if assembler != host {
            differing.push(format!(
                "{chord}: assembler {} vs pane host {}",
                agent_probe::probe::escape_bytes(&assembler),
                agent_probe::probe::escape_bytes(&host)
            ));
        }
    }
    checks.check(
        "input.pane-host-parity",
        differing.is_empty(),
        format!("keys whose bytes differ between the routes: {differing:?}"),
    );

    checks.info("input.guard", format!("{:?}", probe.guard_stats()));
    probe.shutdown();
    checks.finish();
}

// ---- mouse, scrolling, scrollback --------------------------------------------------------------------

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn fullscreen_wheel_click_scroll_keys_and_the_scrollback_handoff() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    // Ten rows: the start banner overflows the conversation area, so there is something to scroll.
    let started = start(fullscreen("fs-mouse").size(100, 10));
    let probe = &started.probe;
    // The CLI's own bottom of the conversation is the reference: the first frame can sit a row higher.
    probe.key("ctrl+end").unwrap();
    pause(500);
    probe.wait_quiet(Duration::from_millis(700), 10 * SECOND).unwrap();
    let home = probe.screen();
    checks.check(
        "scroll.no-terminal-scrollback",
        probe.history_size() == 0 && probe.display_offset() == 0,
        "the alternate screen has no scrollback; the CLI scrolls itself",
    );

    let up = MouseInput::new(MouseKind::Wheel(WheelDirection::Up), 20, 4).with_notches(3);
    let sent = probe.send_mouse(&up).unwrap().map(|b| agent_probe::probe::escape_bytes(&b));
    probe.wait_for_text("Jump to bottom", 5 * SECOND).unwrap();
    let scrolled = probe.screen();
    checks.check(
        "scroll.wheel-up",
        sent.as_deref() == Some("ESC[<64;21;5MESC[<64;21;5MESC[<64;21;5M")
            && !home.diff(&scrolled, &normalize).is_empty(),
        format!("wheel up sent {sent:?}; the view moved and the jump-to-bottom button appeared"),
    );
    let down = MouseInput::new(MouseKind::Wheel(WheelDirection::Down), 20, 4).with_notches(6);
    probe.send_mouse(&down).unwrap();
    probe.wait_for_gone("Jump to bottom", 5 * SECOND).unwrap();
    // The wheel reaches the end of the conversation (the button goes away); the CLI's own bottom after the
    // wheel is a row higher than after Ctrl+End, which is its own scroll arithmetic, so only the input box is compared.
    let at_bottom = probe.screen();
    checks.check(
        "scroll.wheel-down",
        claude::input_box(&at_bottom) == claude::input_box(&home) && !at_bottom.contains("Jump to bottom"),
        "six notches down made the jump-to-bottom button go away and left the input box where it was",
    );

    probe.key("pgup").unwrap();
    probe.wait_for_text("Jump to bottom", 5 * SECOND).unwrap();
    probe.key("ctrl+end").unwrap();
    probe.wait_for_gone("Jump to bottom", 5 * SECOND).unwrap();
    checks.check(
        "scroll.keys",
        home.diff(&probe.screen(), &normalize).is_empty(),
        "PgUp (ESC[5~) scrolled and Ctrl+End jumped back to the bottom",
    );

    // Click-to-position in the input box.
    probe.send_text("hello world").unwrap();
    wait_box(probe, "hello world");
    let box_row = claude::input_box(&probe.screen()).unwrap().row;
    let press = MouseInput::new(MouseKind::Press(MouseButton::Left), 8, box_row as u16);
    let release = MouseInput::new(MouseKind::Release(MouseButton::Left), 8, box_row as u16);
    let press_bytes = probe.send_mouse(&press).unwrap().map(|b| agent_probe::probe::escape_bytes(&b));
    probe.send_mouse(&release).unwrap();
    pause(400);
    probe.send_text("Z").unwrap();
    let positioned = probe
        .wait_until("Z inserted at the clicked cell", 5 * SECOND, |s| {
            claude::input_box(s).is_some_and(|b| b.text == "hello Zworld")
        })
        .is_ok();
    checks.check(
        "mouse.click-positions-caret",
        positioned,
        format!("click sent {press_bytes:?}; the box reads {:?}", box_text(probe)),
    );
    clear_box(probe);

    // Hover reports (DECSET 1003) are accepted without side effects.
    let hover = probe.send_mouse(&MouseInput::new(MouseKind::Move { held: None }, 30, 3)).unwrap();
    pause(300);
    checks.check(
        "mouse.hover",
        hover.is_some() && claude::input_box(&probe.screen()).is_some(),
        format!("motion report {:?}", hover.map(|b| agent_probe::probe::escape_bytes(&b))),
    );

    // Ctrl+O then `[` hands the conversation to the terminal's own scrollback (main screen), Esc comes back.
    probe.key("ctrl+o").unwrap();
    probe.wait_for_text("Showing detailed transcript", 5 * SECOND).unwrap();
    probe.send_text("[").unwrap();
    let left = probe
        .wait_until("the transcript on the main screen", 5 * SECOND, |s| {
            !probe.modes().alt_screen && s.contains("Showing detailed transcript")
        })
        .is_ok();
    let handoff = probe.screen();
    let banner =
        handoff.contains("Claude Code v") || probe.history_lines().iter().any(|line| line.contains("Claude Code v"));
    checks.check(
        "scroll.scrollback-handoff",
        left && banner && !probe.modes().mouse_tracking(),
        format!(
            "after Ctrl+O [: alt screen {}, mouse {:?}, {} lines of scrollback, whole conversation reachable {banner}",
            probe.modes().alt_screen,
            probe.modes().mouse_mode,
            probe.history_size()
        ),
    );
    probe.key("esc").unwrap();
    let back = probe
        .wait_until("the CLI returns to the alternate screen", 5 * SECOND, |s| {
            probe.modes().alt_screen && claude::input_box(s).is_some()
        })
        .is_ok();
    let modes = probe.modes();
    checks.check(
        "scroll.handoff-return",
        back && format!("{:?}/{:?}", modes.mouse_mode, modes.mouse_encoding) == "Any/Sgr",
        format!("after Esc: {}", modes.summary()),
    );
    probe.shutdown();
    checks.finish();
}

// ---- resize --------------------------------------------------------------------------------------------

const STORM_SIZES: [(u16, u16); 7] = [(60, 20), (140, 40), (80, 24), (100, 30), (50, 15), (120, 36), (80, 24)];
const STORM_CYCLES: usize = 4;

/// Resizes the emulator and the PTY through [`STORM_SIZES`] several times with 35 ms between steps, then
/// returns the settled screen. The final size is always 80x24.
fn storm(probe: &Probe) -> Screen {
    for _ in 0..STORM_CYCLES {
        for (cols, rows) in STORM_SIZES {
            probe.resize(cols, rows);
            pause(35);
        }
    }
    pause(500);
    probe.wait_quiet(claude::QUIET, 30 * SECOND).unwrap_or_else(|e| panic!("{e}"));
    probe.screen()
}

fn compare_with_fresh(
    checks: &mut Checks,
    prefix: &str,
    stormed: &Screen,
    probe: &Probe,
    options: SpawnOptions,
) -> (usize, Vec<String>) {
    let fresh = start(options.size(80, 24));
    let fresh_screen = fresh.probe.screen();
    let diffs = stormed.diff(&fresh_screen, &normalize);
    checks.check(
        &format!("{prefix}.matches-fresh-process"),
        diffs.is_empty(),
        format!(
            "{} differences between the stormed screen and a fresh 80x24 process; first {:?}",
            diffs.len(),
            diffs.first()
        ),
    );
    checks.check(
        &format!("{prefix}.cursor-matches-fresh-process"),
        probe.cursor() == fresh.probe.cursor(),
        format!("cursor {:?} vs {:?}", probe.cursor(), fresh.probe.cursor()),
    );
    let fresh_history = fresh.probe.history_size();
    fresh.probe.shutdown();
    (fresh_history, probe.history_lines())
}

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn fullscreen_resize_storm_leaves_no_stale_fragments() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    let started = start(fullscreen("fs-storm").size(100, 30));
    let probe = &started.probe;
    let stormed = storm(probe);
    let steps = STORM_CYCLES * STORM_SIZES.len();
    checks.check(
        "fs.storm.size",
        probe.size() == (80, 24) && stormed.cols == 80,
        format!("{steps} resizes, final grid {:?}", probe.size()),
    );
    checks.check(
        "fs.storm.still-fullscreen",
        probe.modes().alt_screen && probe.modes().mouse_tracking(),
        probe.modes().summary(),
    );
    checks.check(
        "fs.storm.frame-complete",
        !probe.modes().sync_pending && claude::input_box(&stormed).is_some() && stormed.wide_cell_problems().is_empty(),
        "input box present, no open synchronized update",
    );
    checks.check(
        "fs.storm.no-emulator-panic",
        !probe.all_events().iter().any(|(_, e)| matches!(e, Observed::EmulatorPanic(_))),
        "no emulator panic during the storm",
    );
    let (_, history) = compare_with_fresh(&mut checks, "fs.storm", &stormed, probe, fullscreen("fs-storm-fresh"));
    checks.check("fs.storm.no-scrollback", history.is_empty(), format!("{} lines in the scrollback", history.len()));
    probe.shutdown();
    checks.finish();
}

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn classic_resize_storm_compared_with_a_fresh_process() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    let started = start(classic("cl-storm").size(100, 30));
    let probe = &started.probe;
    let stormed = storm(probe);
    checks.check(
        "cl.storm.size",
        probe.size() == (80, 24),
        format!("{} resizes, final grid {:?}", STORM_CYCLES * STORM_SIZES.len(), probe.size()),
    );
    let (fresh_history, history) =
        compare_with_fresh(&mut checks, "cl.storm", &stormed, probe, classic("cl-storm-fresh"));
    // The scrollback is where the classic renderer's resize leaves stale lines: a fresh process has none.
    if history.len() > fresh_history {
        checks.gap(
            "cl.storm.stale-scrollback",
            format!(
                "{} lines in the scrollback after the storm, {} in a fresh process: {}",
                history.len(),
                fresh_history,
                history.iter().take(6).map(|line| format!("{:?}", scrub_screen(line))).collect::<Vec<_>>().join(", ")
            ),
        );
    } else {
        checks.check("cl.storm.no-stale-scrollback", true, "the scrollback matches a fresh process");
    }
    probe.shutdown();
    checks.finish();
}

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn fullscreen_tiny_sizes_keep_the_session_alive() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    let started = start(fullscreen("fs-tiny").size(100, 30));
    let probe = &started.probe;
    let mut outside = Vec::new();
    for _ in 0..3 {
        for (cols, rows) in [(40u16, 10u16), (30, 8), (20, 6), (12, 4), (80, 24)] {
            probe.resize(cols, rows);
            pause(60);
            let screen = probe.screen();
            if let Some((row, col)) = screen.cursor
                && (row >= screen.rows || col >= screen.cols)
            {
                outside.push(format!("{cols}x{rows}: cursor ({row}, {col})"));
            }
        }
    }
    probe.wait_quiet(claude::QUIET, 30 * SECOND).unwrap_or_else(|e| panic!("{e}"));
    let screen = probe.screen();
    checks.check("tiny.cursor-inside-grid", outside.is_empty(), format!("cursor outside the grid at: {outside:?}"));
    checks.check(
        "tiny.no-emulator-panic",
        !probe.all_events().iter().any(|(_, e)| matches!(e, Observed::EmulatorPanic(_))),
        "no emulator panic while shrinking to 12x4 and back",
    );
    checks.check(
        "tiny.still-running",
        probe.exit_code().is_none() && claude::input_box(&screen).is_some() && screen.wide_cell_problems().is_empty(),
        format!("running, input box on row {:?} of the 80x24 grid", claude::input_box(&screen).map(|b| b.row)),
    );
    probe.shutdown();
    checks.finish();
}

// ---- rendering sanity ----------------------------------------------------------------------------------

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn wide_characters_wrap_at_the_same_column_in_the_cli_and_the_emulator() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    // 41 columns: the box text area is 39 cells wide, so two-cell glyphs meet the right edge at odd offsets.
    let started = start(fullscreen("fs-wrap").size(41, 16));
    let probe = &started.probe;
    // Short enough for the box: it shows at most a few rows and scrolls to the end of longer text.
    let typed: String = "你好ㄅㄆ😀ab".chars().cycle().take(35).collect();
    probe.send_text(&typed).unwrap();
    probe
        .wait_until("the whole text in the box", 10 * SECOND, |s| {
            claude::input_box(s).is_some_and(|b| b.text.replace('\n', "").chars().count() >= typed.chars().count())
        })
        .unwrap_or_else(|e| panic!("{e}"));
    pause(400);
    let screen = probe.screen();
    let found = claude::input_box(&screen).expect("input box");
    let joined = found.text.replace('\n', "");
    checks.check(
        "wrap.text-intact",
        joined == typed,
        format!(
            "{} rows in the box; the rows joined are {} characters, typed {}; equal: {}",
            found.text.lines().count(),
            joined.chars().count(),
            typed.chars().count(),
            joined == typed
        ),
    );
    checks.check(
        "wrap.wide-cells",
        screen.wide_cell_problems().is_empty(),
        format!("{:?}", screen.wide_cell_problems()),
    );
    let caret = probe.cursor();
    checks.check(
        "wrap.cursor-in-box",
        caret.is_some_and(|(row, _)| row > found.row && row < found.end),
        format!("cursor {caret:?} inside the box rows {}..{}", found.row, found.end),
    );
    probe.shutdown();
    checks.finish();
}

#[test]
#[ignore = "runs the real Claude Code CLI: needs the installed claude.exe and its signed-in state"]
fn frames_are_never_torn_while_typing() {
    let _turn = exclusive();
    let mut checks = Checks::default();
    let started = start(fullscreen("fs-frames").size(100, 30));
    let probe = &started.probe;
    let alphabet: Vec<char> = "abc你好ㄅㄆ😀 def".chars().collect();
    let (mut sampled, mut torn, mut wide_problems) = (0u64, Vec::new(), 0u64);
    let burst = Instant::now();
    for i in 0..120 {
        probe.send_text(&alphabet[i % alphabet.len()].to_string()).unwrap();
        let until = Instant::now() + Duration::from_millis(8);
        while Instant::now() < until {
            let screen = probe.screen();
            sampled += 1;
            if claude::input_box(&screen).is_none()
                || box_bottom(&screen).is_none()
                || screen.cursor.is_none()
                || screen.cells.len() != screen.rows * screen.cols
            {
                torn.push(screen.dump_scrubbed());
            }
            if !screen.wide_cell_problems().is_empty() {
                wide_problems += 1;
            }
        }
    }
    let rate = sampled as f64 / burst.elapsed().as_secs_f64();
    checks.check(
        "frames.not-torn",
        sampled > 500 && torn.is_empty(),
        format!(
            "{sampled} snapshots ({rate:.0}/s) during 120 keystrokes, {} without an intact input box; first {:?}",
            torn.len(),
            torn.first()
        ),
    );
    checks.check(
        "frames.wide-cells",
        wide_problems == 0,
        format!("{wide_problems} snapshots with inconsistent wide-character cells"),
    );
    let settled = probe.wait_quiet(claude::QUIET, 20 * SECOND).unwrap();
    checks.check("frames.all-typed", settled.contains("def"), "the typed text reached the box");
    probe.shutdown();
    checks.finish();
}

#[test]
fn the_probe_refuses_to_run_without_its_guard() {
    // Not ignored: no process is started.
    let options = SpawnOptions::new(claude::exe(), "guard-check").guard(Guard::Off);
    assert!(matches!(Probe::spawn(options), Err(agent_probe::ProbeError::Refused(_))));
}
