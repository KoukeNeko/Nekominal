//! ConPTY experiments, part 3: queries a child sends, answered by the emulator, delivered back through
//! ConPTY's input side; plus how host-injected key sequences arrive at the child.
//! Run with: `cargo test --test conpty_roundtrip -- --ignored --nocapture --test-threads=1`

mod common;

use std::time::Duration;

use common::*;

fn queries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("kitty-push+query", "\x1b[>1u\x1b[?u"),
        ("kitty-query-then-da1", "\x1b[?u\x1b[c"),
        ("da1", "\x1b[c"),
        ("da2", "\x1b[>c"),
        ("xtversion", "\x1b[>q"),
        ("decrqm-2004", "\x1b[?2004$p"),
        ("decrqm-2026", "\x1b[?2026$p"),
        ("decrqm-2027", "\x1b[?2027$p"),
        ("decrqm-9001", "\x1b[?9001$p"),
        ("osc10-query", "\x1b]10;?\x07"),
        ("osc11-query-st", "\x1b]11;?\x1b\\"),
        ("osc7501-query-then-da1", "\x1b]7501;?\x1b\\\x1b[c"),
        ("dsr-5", "\x1b[5n"),
        ("dsr-6-cpr", "\x1b[6n"),
        ("xtwinops-14t", "\x1b[14t"),
        ("xtwinops-18t", "\x1b[18t"),
        ("color-scheme-996", "\x1b[?996n"),
        ("decrqss-sgr", "\x1bP$qm\x1b\\"),
        ("clipboard-read-osc52", "\x1b]52;c;?\x07"),
    ]
}

fn hex(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

#[test]
#[ignore = "spawns ConPTY"]
fn query_round_trip_and_input_delivery() {
    let probe_file = scratch("roundtrip-probes.tsv");
    let out_file = scratch("roundtrip-out.tsv");
    let _ = std::fs::remove_file(&out_file);
    let tsv: String = queries().iter().map(|(tag, seq)| format!("{tag}\t{}\n", hex(seq))).collect();
    std::fs::write(&probe_file, tsv).unwrap();

    let args = vec![
        "-NoLogo".into(),
        "-NoProfile".into(),
        "-File".into(),
        script("roundtrip.ps1"),
        probe_file.to_string_lossy().into_owned(),
        out_file.to_string_lossy().into_owned(),
    ];
    let mut host = Host::spawn(pwsh(), &args, HostOptions::default());
    let ready = host.pump(Duration::from_secs(60), |h| h.raw.windows(15).any(|w| w == b"<<INPUT-READY>>"));
    println!("child reached the input phase: {ready}");

    // Keys the front end would send, one chunk each.
    let injections: [(&str, &str); 12] = [
        ("plain a", "a"),
        ("kitty shift+enter CSI 13;2u", "\x1b[13;2u"),
        ("kitty ctrl+[ CSI 91;5u", "\x1b[91;5u"),
        ("bracketed paste", "\x1b[200~hi there\x1b[201~"),
        ("arrow up", "\x1b[A"),
        ("ctrl+arrow CSI 1;5A", "\x1b[1;5A"),
        ("focus in", "\x1b[I"),
        ("focus out", "\x1b[O"),
        ("utf-8 text", "你好"),
        ("alt+b as ESC b", "\x1bb"),
        ("SGR mouse press", "\x1b[<0;10;5M"),
        ("end marker", "q"),
    ];
    for (_, bytes) in injections {
        host.pump(Duration::from_millis(250), |_| false);
        host.session.write(bytes.as_bytes()).unwrap();
    }
    host.pump_until_exit(Duration::from_secs(15));
    println!("exit: {:?}", host.exit);

    println!("\n-- child -> host raw output sequences (queries as ConPTY forwarded them)");
    for entry in sequence_log(&host.raw) {
        println!("   {entry}");
    }
    println!("\n-- what the emulator wrote back (in order): {}", escape(&host.replies));

    println!("\n-- what the child read from stdin after each query (VT input mode)");
    let log = std::fs::read_to_string(&out_file).unwrap_or_default();
    for line in log.lines() {
        println!("   {line}");
    }
    println!("\n-- injections, in order:");
    for (name, bytes) in injections {
        println!("   {name:32} sent {}", escape(bytes.as_bytes()));
    }
    println!("\n-- emulator events: {:?}", host.events);
}
