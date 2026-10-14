//! ConPTY experiments, part 1: what ConPTY (bundled 1.25.2609.30003) sends at startup and whether it
//! waits for the DA1 answer. Run with:
//! `cargo test --test conpty_startup -- --ignored --nocapture --test-threads=1`

mod common;

use std::time::Duration;

use common::*;

fn cmd_exe() -> std::path::PathBuf {
    nekominal_pty::profiles::resolve_program("cmd").unwrap()
}

#[test]
#[ignore = "spawns ConPTY"]
fn startup_sequence_and_da1_dependency() {
    for (label, answer) in [("DA1 answered with CSI ? 6 c", true), ("DA1 never answered", false)] {
        let args = ["/d".to_owned(), "/c".to_owned(), "echo hello-from-cmd".to_owned()];
        let mut host = Host::spawn(cmd_exe(), &args, HostOptions { answer_replies: answer, ..HostOptions::default() });
        let marker = b"hello-from-cmd";
        let seen = host.pump(Duration::from_secs(6), |h| h.raw.windows(marker.len()).any(|w| w == marker));
        let first_data = host.frames.first().map(|f| f.0);
        let marker_at = host.frames.last().map(|f| f.0);
        println!("== {label}");
        println!("   output reached the host: {seen}; first frame at {first_data:?}; marker frame at {marker_at:?}");
        println!("   sequences: {:?}", sequence_log(&host.raw));
        println!("   replies the emulator produced: {:?}", escape(&host.replies));
        println!("   raw (first 500 bytes): {}", escape(&host.raw[..host.raw.len().min(500)]));
        host.pump_until_exit(Duration::from_secs(3));
        println!("   exit: {:?}", host.exit);
    }
}

#[test]
#[ignore = "spawns ConPTY"]
fn startup_with_pwsh() {
    for (label, answer) in [("answered", true), ("unanswered", false)] {
        let args = ["-NoLogo".to_owned(), "-NoProfile".to_owned(), "-Command".to_owned(), "Write-Output 'pwsh-ready'".to_owned()];
        let mut host = Host::spawn(pwsh(), &args, HostOptions { answer_replies: answer, ..HostOptions::default() });
        let marker = b"pwsh-ready";
        let seen = host.pump(Duration::from_secs(15), |h| h.raw.windows(marker.len()).any(|w| w == marker));
        println!("== pwsh, DA1 {label}: output reached the host: {seen}; frames: {:?}", host.frames.iter().take(6).collect::<Vec<_>>());
        println!("   sequences: {:?}", sequence_log(&host.raw));
        host.pump_until_exit(Duration::from_secs(5));
    }
}

#[test]
#[ignore = "spawns ConPTY"]
fn in_box_conpty_startup_and_da1_dependency() {
    use common::raw_conpty::*;
    // The in-box ConPTY of this Windows build (kernel32), for comparison with the bundled pair.
    for (label, answer, flags) in [
        ("in-box, DA1 answered, flags 0x10", true, GLYPH_WIDTH_WCSWIDTH),
        ("in-box, DA1 NOT answered, flags 0x10", false, GLYPH_WIDTH_WCSWIDTH),
        ("in-box, DA1 NOT answered, flags 0", false, 0),
    ] {
        let args = ["/d".to_owned(), "/c".to_owned(), "echo hello-from-cmd".to_owned()];
        let created = RawPty::spawn_system(&cmd_exe(), &args, 100, 30, flags);
        let mut pty = match created {
            Ok(pty) => pty,
            Err(error) => {
                println!("== {label}: spawn failed: {error}");
                continue;
            }
        };
        let started = std::time::Instant::now();
        let mut first = None;
        let mut marker_at = None;
        let mut seen = Vec::new();
        let raw = pty.run(Duration::from_secs(5), |chunk| {
            first.get_or_insert(started.elapsed());
            seen.extend_from_slice(chunk);
            if marker_at.is_none() && seen.windows(14).any(|w| w == b"hello-from-cmd") {
                marker_at = Some(started.elapsed());
            }
            if answer && chunk.windows(3).any(|w| w == b"\x1b[c") { b"\x1b[?6c".to_vec() } else { Vec::new() }
        });
        println!("== {label}");
        println!("   first output at {first:?}; marker at {marker_at:?}; sequences {:?}", sequence_log(&raw));
        println!("   raw: {}", escape(&raw[..raw.len().min(300)]));
    }
}
