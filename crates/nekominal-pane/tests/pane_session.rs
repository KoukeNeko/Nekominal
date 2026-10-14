//! `Pane` behaviour that needs a running session (ConPTY) but no GPU.

use std::{
    collections::BTreeMap,
    thread::sleep,
    time::{Duration, Instant},
};

use nekominal_input::{Key, KeyEvent, KeyInput};
use nekominal_pane::{Pane, UiEvent};
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_term::{CoreConfig, alacritty_terminal::term::Osc52};

fn system32(file: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    std::path::Path::new(&root).join("System32").join(file)
}

fn cmd() -> std::path::PathBuf {
    system32("cmd.exe")
}

/// Windows PowerShell 5.1, which every supported Windows has, running `script` and then waiting two seconds so
/// the session outlives the output.
fn powershell_session(script: &str, osc52: Osc52) -> Pane {
    let mut args: Vec<String> = ["-NoProfile", "-NonInteractive", "-Command"].map(String::from).into();
    args.push(format!("{script}; Start-Sleep 2"));
    let config = SpawnConfig {
        program: system32("WindowsPowerShell\\v1.0\\powershell.exe"),
        args,
        cwd: None,
        env: BTreeMap::new(),
        cols: 80,
        rows: 24,
        width_model: WidthModel::Wcswidth,
    };
    Pane::spawn_with(config, CoreConfig { osc52, ..CoreConfig::default() }).expect("powershell starts under ConPTY")
}

/// The first event `pick` accepts, within 20 seconds.
fn wait_for<T>(pane: &Pane, mut pick: impl FnMut(UiEvent) -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        while let Ok(event) = pane.events.try_recv() {
            if let Some(found) = pick(event) {
                return Some(found);
            }
        }
        sleep(Duration::from_millis(50));
    }
    None
}

const OSC52_WRITE: &str = r#"[Console]::Out.Write("$([char]27)]52;c;aGVsbG8=$([char]7)")"#;
const OSC52_READ: &str = r#"[Console]::Out.Write("$([char]27)]52;c;?$([char]7)")"#;

#[test]
fn an_osc52_write_survives_conpty_and_arrives_as_a_clipboard_event() {
    let pane = powershell_session(OSC52_WRITE, Osc52::OnlyCopy);
    let text = wait_for(&pane, |event| match event {
        UiEvent::ClipboardStore(text) => Some(text),
        _ => None,
    });
    pane.shutdown();
    assert_eq!(text.as_deref(), Some("hello"));
}

#[test]
fn an_osc52_read_arrives_only_when_the_policy_allows_it_and_can_be_answered() {
    // The write after the read is what shows the read had its chance: events come in the order of the output.
    let denied = powershell_session(&format!("{OSC52_READ}; {OSC52_WRITE}"), Osc52::OnlyCopy);
    let mut read_seen = false;
    let write_seen = wait_for(&denied, |event| match event {
        UiEvent::ClipboardLoad(_) => {
            read_seen = true;
            None
        }
        UiEvent::ClipboardStore(_) => Some(()),
        _ => None,
    });
    denied.shutdown();
    assert!(write_seen.is_some(), "the write after the read arrived");
    assert!(!read_seen, "the default policy never lets a program read the clipboard");

    let allowed = powershell_session(OSC52_READ, Osc52::CopyPaste);
    let request = wait_for(&allowed, |event| match event {
        UiEvent::ClipboardLoad(request) => Some(request),
        _ => None,
    })
    .expect("a read the policy allows reaches the pane");
    assert!(allowed.answer_clipboard(&request, "hi"));
    assert!(!allowed.answer_clipboard(&request, &"x".repeat(nekominal_pane::MAX_CLIPBOARD_TEXT + 1)));
    allowed.set_osc52(Osc52::Disabled);
    allowed.shutdown();
}

#[test]
fn scrolling_moves_through_the_history_and_stops_at_its_ends() {
    let pane = Pane::spawn(cmd(), Vec::new(), 80, 24).expect("cmd.exe starts under ConPTY");
    let mut text = String::new();
    for line in 0..100 {
        text.push_str(&format!("line {line}\r\n"));
    }
    pane.term.lock().feed(text.as_bytes());
    assert_eq!(pane.term.lock().display_offset(), 0);

    pane.scroll_lines(5);
    assert_eq!(pane.term.lock().display_offset(), 5);
    pane.scroll_lines(-2);
    assert_eq!(pane.term.lock().display_offset(), 3);
    pane.scroll_lines(-50);
    assert_eq!(pane.term.lock().display_offset(), 0, "the bottom is the live screen");
    pane.scroll_lines(10_000);
    let history = pane.term.lock().history_size();
    assert_eq!(pane.term.lock().display_offset(), history, "the top is the oldest line");
    pane.shutdown();
}

#[test]
fn typing_pasting_and_composing_dismiss_the_selection_and_return_to_the_live_screen_but_output_does_not() {
    let pane = Pane::spawn(cmd(), Vec::new(), 80, 24).expect("cmd.exe starts under ConPTY");
    let mut text = String::new();
    for line in 0..60 {
        text.push_str(&format!("line {line}\r\n"));
    }
    pane.term.lock().feed(text.as_bytes());
    let key = KeyInput::new(Key::Escape);
    let release = KeyInput::new(Key::Escape).with_event(KeyEvent::Release);
    let actions: [(&str, &dyn Fn()); 3] = [
        ("a key", &|| assert!(pane.send_key(&key))),
        ("composed text", &|| pane.send_text("你")),
        ("a paste", &|| pane.paste("x")),
    ];
    for (name, act) in actions {
        pane.scroll_lines(10);
        pane.select_all();
        assert!(pane.selection_text().is_some() && pane.term.lock().display_offset() == 10, "{name}: set up");
        // Output arrives: the view the user scrolled back to stays, and so does the selection.
        pane.term.lock().feed(b"more output\r\n");
        assert!(pane.selection_text().is_some(), "{name}: output keeps the selection");
        assert_eq!(pane.term.lock().display_offset(), 11, "{name}: and does not move the view");
        act();
        assert!(pane.selection_text().is_none(), "{name} dismissed the selection");
        assert_eq!(pane.term.lock().display_offset(), 0, "{name} returned to the live screen");
    }
    // Letting go of a key is not typing.
    pane.scroll_lines(5);
    pane.select_all();
    pane.send_key(&release);
    assert!(pane.selection_text().is_some());
    pane.shutdown();
}

#[test]
fn focus_changes_reach_the_emulator_and_survive_a_closed_session() {
    let pane = Pane::spawn(cmd(), Vec::new(), 80, 24).expect("cmd.exe starts under ConPTY");
    pane.set_focused(false);
    pane.set_focused(true);
    pane.shutdown();
    // Writing after shutdown must not panic: the writer thread has ended and the send is dropped.
    sleep(Duration::from_millis(200));
    pane.set_focused(false);
}
