//! ConPTY experiments, part 6: what happens to wrapped text when the terminal is resized.
//! Run with: `cargo test --test conpty_resize -- --ignored --nocapture --test-threads=1`

mod common;

use std::time::Duration;

use common::*;
use nekominal_term::{alacritty_terminal::grid::Scroll, Snapshot};

fn contains(raw: &[u8], needle: &str) -> bool {
    raw.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn dump(label: &str, host: &mut Host, from: usize) {
    println!("-- {label}: ConPTY wrote {} bytes: {}", host.raw.len() - from, escape(&host.raw[from..host.raw.len().min(from + 900)]));
    let mut snapshot = Snapshot::default();
    host.core.scroll_display(Scroll::Bottom);
    host.core.snapshot(&mut snapshot);
    let history = host.core.history_size();
    println!("   emulator {}x{}, history {history} lines, visible text:", snapshot.columns, snapshot.rows);
    for row in 0..snapshot.rows {
        let text = snapshot.row_text(row);
        if !text.is_empty() {
            println!("   | {text}");
        }
    }
    if history > 0 {
        host.core.scroll_display(Scroll::Top);
        host.core.snapshot(&mut snapshot);
        println!("   scrollback (top {} rows):", snapshot.rows.min(history));
        for row in 0..snapshot.rows.min(history) {
            println!("   ^ {}", snapshot.row_text(row));
        }
        host.core.scroll_display(Scroll::Bottom);
    }
}

#[test]
#[ignore = "spawns ConPTY"]
fn resize_wrapped_output() {
    let args = vec!["-NoLogo".into(), "-NoProfile".into(), "-File".into(), script("resize.ps1")];
    let mut host = Host::spawn(pwsh(), &args, HostOptions { cols: 100, rows: 30, ..HostOptions::default() });
    assert!(host.pump(Duration::from_secs(30), |h| contains(&h.raw, "<<PHASE1>>")));
    host.pump(Duration::from_millis(200), |_| false);
    dump("before resize (100x30)", &mut host, 0);

    // Emulator first, then the PTY, like the front end does.
    let mark = host.raw.len();
    host.core.resize(60, 20);
    host.session.resize(60, 20).unwrap();
    assert!(host.pump(Duration::from_secs(10), |h| contains(&h.raw, "<<AFTER-RESIZE-1>>")));
    host.pump(Duration::from_millis(200), |_| false);
    dump("after shrinking to 60x20", &mut host, mark);

    let mark = host.raw.len();
    host.core.resize(100, 30);
    host.session.resize(100, 30).unwrap();
    assert!(host.pump(Duration::from_secs(10), |h| contains(&h.raw, "<<AFTER-RESIZE-2>>")));
    host.pump_until_exit(Duration::from_secs(5));
    dump("after growing back to 100x30", &mut host, mark);
}
