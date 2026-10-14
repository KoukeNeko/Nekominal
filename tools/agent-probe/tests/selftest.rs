//! Self-tests of the harness with programs that are not agent CLIs: they start in a ConPTY (like the ones in
//! `nekominal-pty`), need no network and no installed CLI, and show that what the Claude Code scenarios rely
//! on works: the environment allow-list, the guard, replies, modes and screen reads.

#![cfg(windows)]

use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use agent_probe::{
    claude,
    inventory::Group,
    probe::{Guard, Probe, ProbeError, SpawnOptions},
};
use nekominal_input::{MouseButton, MouseInput, MouseKind};

/// The harness swaps this process's environment around every spawn, so spawns take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn system_exe(relative: &str) -> PathBuf {
    let root = std::env::var("SystemRoot").expect("SystemRoot");
    PathBuf::from(root).join(relative)
}

fn cmd() -> PathBuf {
    system_exe("System32\\cmd.exe")
}

fn powershell() -> PathBuf {
    system_exe("System32\\WindowsPowerShell\\v1.0\\powershell.exe")
}

const SECOND: Duration = Duration::from_secs(1);

fn raw_output(probe: &Probe) -> String {
    String::from_utf8_lossy(&std::fs::read(probe.paths().0).expect("byte tap")).into_owned()
}

#[test]
fn the_child_gets_the_allow_list_and_the_session_variables_stay_out() {
    let _turn = turn();
    // Names of the kind the host session leaks, with values that are not secrets.
    let leaked = [
        "CLAUDE_CODE_MESSAGING_TOKEN",
        "CLAUDE_CODE_MESSAGING_SOCKET",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDECODE",
        "ANTHROPIC_API_KEY",
        "MCP_PROBE_TEST",
        "NO_COLOR",
        "GITHUB_PROBE_TOKEN",
    ];
    // SAFETY: documented as always safe on Windows; this test owns the environment while it holds `turn()`.
    unsafe {
        for name in leaked {
            std::env::set_var(name, "leaked-value-for-the-test");
        }
        std::env::set_var("AGENT_PROBE_KEPT_BY_HOST", "1");
    }
    let path_before = std::env::var("PATH");

    let probe = Probe::spawn(SpawnOptions::new(cmd(), "selftest-env").args(&["/d", "/c", "set"])).expect("spawn cmd");
    probe.wait_exit(10 * SECOND).expect("cmd /c set exits");
    probe.wait_quiet(Duration::from_millis(300), 5 * SECOND).expect("output settles");
    let output = raw_output(&probe);
    probe.shutdown();

    for name in leaked {
        assert!(!output.contains(name), "{name} reached the child");
        // SAFETY: as above.
        unsafe { std::env::remove_var(name) };
    }
    assert!(!output.contains("leaked-value-for-the-test"));
    assert!(!output.contains("AGENT_PROBE_KEPT_BY_HOST"), "only allow-listed names pass");
    for expected in [
        "DISABLE_AUTOUPDATER=1",
        "TERM=xterm-256color",
        "COLORTERM=truecolor",
        "TERM_PROGRAM=Nekominal",
        "SystemRoot=",
        "PATH=",
    ] {
        assert!(output.contains(expected), "the child lacks {expected}:\n{output}");
    }
    assert_eq!(std::env::var("AGENT_PROBE_KEPT_BY_HOST").as_deref(), Ok("1"), "the host environment is restored");
    assert_eq!(std::env::var("PATH"), path_before);
    // SAFETY: as above.
    unsafe { std::env::remove_var("AGENT_PROBE_KEPT_BY_HOST") };
}

#[test]
fn an_override_with_a_session_variable_name_is_refused() {
    let _turn = turn();
    let result = Probe::spawn(
        SpawnOptions::new(cmd(), "selftest-override").env("CLAUDE_CODE_MESSAGING_TOKEN", "x").args(&["/c", "exit"]),
    );
    assert!(matches!(result, Err(ProbeError::Other(_))), "{:?}", result.err());
}

#[test]
fn the_guard_refuses_what_could_submit_and_lets_plain_input_through() {
    let _turn = turn();
    let options =
        SpawnOptions::new(cmd(), "selftest-guard").args(&["/d", "/k"]).guard(Guard::NoModelCalls(claude::policy()));
    let probe = Probe::spawn(options).expect("spawn cmd");
    probe.wait_for_text(">", 10 * SECOND).expect("cmd prompt");

    probe.send_text("echo plain text").expect("text is not a submit");
    probe.send_paste("single line").expect("a single-line paste is not a submit");
    probe.key("left").expect("navigation is not a submit");
    for chord in ["enter", "shift+enter", "alt+enter", "ctrl+j", "ctrl+m"] {
        assert!(matches!(probe.key(chord), Err(ProbeError::Refused(_))), "{chord}");
    }
    assert!(matches!(probe.send_text("a\rb"), Err(ProbeError::Refused(_))), "a control character in text");
    assert!(
        matches!(probe.send_paste("one\ntwo"), Err(ProbeError::Refused(_))),
        "a multi-line paste without bracketed paste"
    );
    assert!(matches!(probe.send_raw(b"\r"), Err(ProbeError::Refused(_))));

    let stats = probe.guard_stats();
    assert_eq!(stats.refusals.len(), 8, "{:?}", stats.refusals);
    assert!(stats.submits_sent.is_empty());
    // Nothing that was refused reached the program: the typed text is still on the command line.
    probe.wait_for_text("echo plain textsingle line", 5 * SECOND).expect("only the allowed input arrived");
    probe.shutdown();
}

#[test]
fn a_scripted_program_exercises_replies_modes_events_and_screen_reads() {
    let _turn = turn();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("scripts").join("sequences.ps1");
    let args = ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script.to_str().unwrap()];
    let probe = Probe::spawn(SpawnOptions::new(powershell(), "selftest-script").args(&args).size(100, 30))
        .expect("spawn powershell");

    probe.wait_for_text("PHASE-1", 20 * SECOND).expect("phase 1");
    // Modes the script set.
    let modes = probe.modes();
    assert!(modes.alt_screen, "{}", modes.summary());
    assert!(modes.bracketed_paste && modes.focus_reporting, "{}", modes.summary());
    assert_eq!(format!("{:?}/{:?}", modes.mouse_mode, modes.mouse_encoding), "Button/Sgr", "{}", modes.summary());
    assert_eq!(modes.kitty_flags & 1, 1, "kitty flags pushed: {}", modes.summary());
    // Text, wide characters and the cursor.
    let screen = probe.wait_for_text("宽字 end", 5 * SECOND).expect("wide text");
    let row = screen.row_containing("宽字 end", 0).unwrap();
    assert_eq!(screen.column_of(row, "end"), Some(5), "two wide glyphs take four cells and a space");
    assert!(screen.wide_cell_problems().is_empty(), "{:?}", screen.wide_cell_problems());
    // Queries were answered, in order, and counted.
    probe.wait_until("replies recorded", 5 * SECOND, |_| probe.counters().replies >= 3).expect("replies");
    probe.with_inventory(|inventory| {
        for group in [Group::Da1, Group::Xtversion, Group::KittyKeyboard, Group::DecrqmPrivate(2026)] {
            assert!(
                inventory.answer_for(group).label().starts_with("answered"),
                "{}: {}",
                group.name(),
                inventory.report()
            );
        }
        assert!(inventory.replies_follow_query_order(), "{}", inventory.report());
        assert_eq!(inventory.count("DECSET ?2026"), inventory.count("DECRST ?2026"), "{}", inventory.report());
    });
    // Title and program status reports reached the host as events.
    let events = probe.take_events();
    assert!(
        events.iter().any(|e| matches!(e, agent_probe::probe::Observed::Title(t) if t == "selftest-title")),
        "{events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, agent_probe::probe::Observed::Agent(nekominal_term::agent_status::Report::Set(r)) if r.app.as_deref() == Some("selftest"))),
        "{events:?}"
    );
    // A mouse click is encoded for the modes the script chose.
    let click = MouseInput::new(MouseKind::Press(MouseButton::Left), 4, 2);
    assert_eq!(probe.send_mouse(&click).unwrap(), Some(b"\x1b[<0;5;3M".to_vec()));

    probe.wait_for_text("PHASE-2", 20 * SECOND).expect("phase 2");
    let modes = probe.modes();
    assert!(
        !modes.alt_screen && !modes.mouse_tracking() && !modes.bracketed_paste,
        "the script reset them: {}",
        modes.summary()
    );
    assert_eq!(modes.kitty_flags, 0, "{}", modes.summary());
    assert!(probe.screen().contains("PHASE-2"));
    assert_eq!(probe.wait_exit(20 * SECOND).unwrap(), 0);
    assert_eq!(probe.leftover_attributes().unwrap(), None, "the script ended with SGR 0");
    probe.shutdown();
}

#[test]
fn attributes_the_program_leaves_set_are_reported() {
    let _turn = turn();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("scripts").join("leftover.ps1");
    let args = ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script.to_str().unwrap()];
    let probe =
        Probe::spawn(SpawnOptions::new(powershell(), "selftest-leftover").args(&args)).expect("spawn powershell");
    assert_eq!(probe.wait_exit(20 * SECOND).unwrap(), 0);
    let leftover = probe.leftover_attributes().unwrap().expect("bold red was never reset");
    assert!(leftover.contains("BOLD") && leftover.contains("idx1"), "{leftover}");
    probe.shutdown();
}

#[test]
fn leftover_attributes_need_an_exited_program() {
    let _turn = turn();
    let probe =
        Probe::spawn(SpawnOptions::new(cmd(), "selftest-leftover-early").args(&["/d", "/k"])).expect("spawn cmd");
    assert!(matches!(probe.leftover_attributes(), Err(ProbeError::Other(_))));
    probe.shutdown();
}

#[test]
fn shutdown_ends_the_program_and_is_safe_to_repeat() {
    let _turn = turn();
    let probe = Probe::spawn(SpawnOptions::new(cmd(), "selftest-shutdown").args(&["/d", "/k"])).expect("spawn cmd");
    probe.wait_for_text(">", 10 * SECOND).expect("cmd prompt");
    let pid = probe.pid();
    assert!(agent_probe::sys::is_running(pid));
    probe.shutdown();
    probe.shutdown();
    let deadline = std::time::Instant::now() + 5 * SECOND;
    while agent_probe::sys::is_running(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!agent_probe::sys::is_running(pid), "the process tree is killed with the session");
}
