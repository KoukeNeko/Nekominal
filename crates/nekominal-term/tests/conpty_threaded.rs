//! ConPTY experiments, part 5: the recommended threading design under load.
//! Run with: `cargo test --release --test conpty_threaded -- --ignored --nocapture --test-threads=1`

mod common;

use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use common::{pane::*, *};
use nekominal_term::Snapshot;

fn drive(pane: &Pane, max: Duration) -> (usize, Duration, Option<u32>, Vec<String>) {
    let mut snapshot = Snapshot::default();
    let mut frames = 0;
    let mut worst_wait = Duration::ZERO;
    let mut exit = None;
    let mut seen = Vec::new();
    let deadline = Instant::now() + max;
    while Instant::now() < deadline {
        if let Some(waited) = pane.redraw(&mut snapshot, Duration::from_millis(100)) {
            frames += 1;
            worst_wait = worst_wait.max(waited);
        }
        while let Ok(event) = pane.ui_events.try_recv() {
            if let UiEvent::Exit(code) = event {
                exit = Some(code);
            }
            seen.push(format!("{event:?}"));
        }
        if exit.is_some() {
            // one more pass picks up output that raced with the exit event
            pane.redraw(&mut snapshot, Duration::from_millis(50));
            break;
        }
    }
    (frames, worst_wait, exit, seen)
}

#[test]
#[ignore = "spawns ConPTY"]
fn burst_through_the_pane_host() {
    let args = vec!["-NoLogo".into(), "-NoProfile".into(), "-File".into(), script("burst.ps1"), "3000".into()];
    let pane = Pane::spawn(pwsh(), args, 100, 30, false);
    let started = Instant::now();
    let (frames, worst_wait, exit, seen) = drive(&pane, Duration::from_secs(60));
    let elapsed = started.elapsed();
    println!("exit {exit:?} after {elapsed:.2?}; UI frames {frames}; worst wait for the terminal lock {worst_wait:.2?}");
    println!(
        "fed {} KiB in {} PTY frames; feed time total {:.1} ms, worst frame {:.2} ms; replies written {}",
        pane.stats.bytes_fed.load(Ordering::Relaxed) / 1024,
        pane.stats.frames_fed.load(Ordering::Relaxed),
        pane.stats.feed_nanos_total.load(Ordering::Relaxed) as f64 / 1e6,
        pane.stats.feed_nanos_max.load(Ordering::Relaxed) as f64 / 1e6,
        pane.stats.replies_written.load(Ordering::Relaxed),
    );
    let mut core = pane.term.lock();
    let mut snapshot = Snapshot::default();
    core.snapshot(&mut snapshot);
    println!("screen after exit:\n{}", snapshot.text());
    let found = core.search_all("line 3000 ", 5).unwrap();
    println!("history {} lines; 'line 3000' found {} time(s); agent summary {:?}", core.history_size(), found.len(), core.agent_summary());
    drop(core);
    println!("ui events: {seen:?}");
    assert_eq!(exit, Some(0));
    assert!(snapshot.text().contains("BURST-DONE"));
    assert!(!found.is_empty());
}

#[test]
#[ignore = "spawns ConPTY, writes a 64 MiB file"]
fn bulk_output_throughput_through_conpty() {
    // 64 MiB of coloured text, `type`d by cmd.exe through ConPTY.
    let path = scratch("bulk.txt");
    let mut data = Vec::new();
    let word = "\x1b[38;5;196mred\x1b[0m \x1b[32mgreen\x1b[0m 終端機 plain text for the terminal emulator throughput test 0123456789\r\n";
    while data.len() < 64 * 1024 * 1024 {
        data.extend_from_slice(word.as_bytes());
    }
    std::fs::write(&path, &data).unwrap();

    let args = vec!["/d".into(), "/c".into(), "type".into(), path.to_string_lossy().into_owned()];
    let cmd = nekominal_pty::profiles::resolve_program("cmd").unwrap();
    let pane = Pane::spawn(cmd, args, 120, 40, false);
    let started = Instant::now();
    let (frames, worst_wait, exit, _) = drive(&pane, Duration::from_secs(120));
    let elapsed = started.elapsed();
    let bytes = pane.stats.bytes_fed.load(Ordering::Relaxed);
    let feed_secs = pane.stats.feed_nanos_total.load(Ordering::Relaxed) as f64 / 1e9;
    println!(
        "exit {exit:?}: {:.1} MiB reached the emulator in {elapsed:.2?} = {:.1} MiB/s end to end; emulator busy {:.2} s ({:.0} MiB/s while feeding); UI frames {frames}, worst lock wait {worst_wait:.2?}, worst feed {:.2} ms",
        bytes as f64 / 1048576.0,
        bytes as f64 / 1048576.0 / elapsed.as_secs_f64(),
        feed_secs,
        bytes as f64 / 1048576.0 / feed_secs,
        pane.stats.feed_nanos_max.load(Ordering::Relaxed) as f64 / 1e6,
    );
    assert_eq!(exit, Some(0));
}

#[test]
#[ignore = "spawns ConPTY, writes files"]
fn conpty_alone_is_the_throughput_limit() {
    use common::raw_conpty::*;
    // Same `type` workloads, but the host does nothing with the bytes: this is ConPTY's own ceiling.
    let colored = "\x1b[38;5;196mred\x1b[0m \x1b[32mgreen\x1b[0m 終端機 plain text for the terminal emulator throughput test 0123456789\r\n";
    let plain = "plain ascii text for the terminal emulator throughput test 0123456789 abcdefghijklmnopqrstuvwxyz\r\n";
    for (name, line) in [("coloured+CJK lines", colored), ("plain ASCII lines", plain)] {
        let path = scratch("drain.txt");
        let mut data = Vec::new();
        while data.len() < 16 * 1024 * 1024 {
            data.extend_from_slice(line.as_bytes());
        }
        std::fs::write(&path, &data).unwrap();
        for (mode, flags) in [("0x10", GLYPH_WIDTH_WCSWIDTH), ("0x08", GLYPH_WIDTH_GRAPHEMES)] {
            let cmd = nekominal_pty::profiles::resolve_program("cmd").unwrap();
            let args = vec!["/d".into(), "/c".into(), "type".into(), path.to_string_lossy().into_owned()];
            let mut pty = RawPty::spawn(&cmd, &args, 120, 40, flags).unwrap();
            let started = Instant::now();
            let mut bytes = 0usize;
            let mut answered = false;
            pty.run(Duration::from_secs(120), |chunk| {
                bytes += chunk.len();
                if !answered {
                    answered = true;
                    return b"\x1b[?6c".to_vec();
                }
                Vec::new()
            });
            let elapsed = started.elapsed();
            println!(
                "{name:20} flags {mode}: input {:.1} MiB -> {:.1} MiB of ConPTY output in {elapsed:.2?} = {:.1} MiB/s (no emulator)",
                data.len() as f64 / 1048576.0,
                bytes as f64 / 1048576.0,
                bytes as f64 / 1048576.0 / elapsed.as_secs_f64()
            );
        }
    }
}

#[test]
#[ignore = "spawns ConPTY"]
fn conpty_alone_with_large_writes() {
    use common::raw_conpty::*;
    for chunk_chars in [4096usize, 32768, 262144] {
        let args = vec![
            "-NoLogo".into(), "-NoProfile".into(), "-File".into(), script("bigwrite.ps1"),
            "16".into(), chunk_chars.to_string(),
        ];
        let mut pty = RawPty::spawn(&pwsh(), &args, 120, 40, GLYPH_WIDTH_WCSWIDTH).unwrap();
        let started = Instant::now();
        let mut bytes = 0usize;
        let mut answered = false;
        pty.run(Duration::from_secs(120), |chunk| {
            bytes += chunk.len();
            if !answered {
                answered = true;
                return b"\x1b[?6c".to_vec();
            }
            Vec::new()
        });
        let elapsed = started.elapsed();
        println!(
            "pwsh writing 16 MiB in {chunk_chars}-char WriteConsole calls: {:.1} MiB of output in {elapsed:.2?} = {:.1} MiB/s (includes ~0.3 s pwsh start-up, no emulator)",
            bytes as f64 / 1048576.0,
            bytes as f64 / 1048576.0 / elapsed.as_secs_f64()
        );
    }
}
