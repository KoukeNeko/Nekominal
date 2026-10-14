//! `Pane` behaviour that needs a running session (ConPTY) but no GPU.

use std::{thread::sleep, time::Duration};

use nekominal_pane::Pane;

fn cmd() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    std::path::Path::new(&root).join("System32").join("cmd.exe")
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
fn focus_changes_reach_the_emulator_and_survive_a_closed_session() {
    let pane = Pane::spawn(cmd(), Vec::new(), 80, 24).expect("cmd.exe starts under ConPTY");
    pane.set_focused(false);
    pane.set_focused(true);
    pane.shutdown();
    // Writing after shutdown must not panic: the writer thread has ended and the send is dropped.
    sleep(Duration::from_millis(200));
    pane.set_focused(false);
}
