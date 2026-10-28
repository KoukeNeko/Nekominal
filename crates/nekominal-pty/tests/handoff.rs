//! A session handed to another process keeps running there: the program is never stopped, what it wrote while nobody read
//! comes through, and input, resize and exit work on the side that took it.
//!
//! As in an update, the process that gives the session exits without closing anything (`TerminateProcess` on itself:
//! no destructor, no `ClosePseudoConsole`), after duplicating the handles into the process that takes it. That needs two
//! processes, so this test has no harness: it runs itself again as the child that starts the session and gives it away.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use nekominal_pty::{
    conpty::{SpawnConfig, WidthModel},
    handoff::{self, Parts, Process},
    session::{Event, Session, Sessions, Sink},
};

const CHILD_MARKER: &str = "NEKOMINAL_PTY_HANDOFF_PARENT";

fn main() -> ExitCode {
    let outcome = match std::env::var(CHILD_MARKER) {
        Ok(parent) => child(parent.parse().expect("the parent's pid")),
        Err(_) => parent(),
    };
    match outcome {
        Ok(()) => {
            println!("handoff ... ok");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("handoff ... FAILED: {message}");
            ExitCode::FAILURE
        }
    }
}

/// A session and the events of its output.
fn started(sessions: &Sessions, start: impl FnOnce(&Sessions, Sink) -> std::io::Result<(u32, u32)>) -> Result<(std::sync::Arc<Session>, Receiver<Event>), String> {
    let (tx, rx) = mpsc::channel();
    let sink: Sink = Box::new(move |event| tx.send(event).is_ok());
    let (id, _) = start(sessions, sink).map_err(|e| format!("start: {e}"))?;
    Ok((sessions.get(id).map_err(|e| e.to_string())?, rx))
}

/// Plays the terminal (answers ConPTY's DA1 query, acknowledges output) until `done` holds for the text read since the
/// call, or `timeout` passes. The text, or the exit code when the program ended first.
fn read_until(session: &Session, rx: &Receiver<Event>, timeout: Duration, done: impl Fn(&str) -> bool) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut text = Vec::new();
    while !done(&String::from_utf8_lossy(&text)) {
        let left = deadline.checked_duration_since(Instant::now()).ok_or_else(|| format!("timed out; read {:?}", String::from_utf8_lossy(&text)))?;
        match rx.recv_timeout(left) {
            Ok(Event::Data(bytes)) => {
                if bytes.windows(3).any(|w| w == b"\x1b[c") {
                    session.write(b"\x1b[?1;2c").map_err(|e| e.to_string())?;
                }
                session.ack(bytes.len());
                text.extend(bytes);
            }
            Ok(Event::Exit(code)) => return Err(format!("the program exited with {code}; read {:?}", String::from_utf8_lossy(&text))),
            Err(_) => return Err(format!("timed out; read {:?}", String::from_utf8_lossy(&text))),
        }
    }
    Ok(String::from_utf8_lossy(&text).into_owned())
}

/// Lines of ping's replies: `TTL=` in every language Windows speaks.
fn replies(text: &str) -> usize {
    text.matches("TTL=").count()
}

/// Starts `ping -t` under `cmd /k`, gives it to `parent` and exits on the spot.
fn child(parent: u32) -> Result<(), String> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let config = SpawnConfig {
        program: PathBuf::from(system_root).join("System32").join("cmd.exe"),
        args: ["/d", "/k", "ping -t 127.0.0.1"].map(String::from).to_vec(),
        cwd: None,
        env: BTreeMap::new(),
        cols: 100,
        rows: 30,
        width_model: WidthModel::default(),
        inherit_environment: true,
    };
    let sessions = Sessions::default();
    let (session, rx) = started(&sessions, |sessions, sink| sessions.spawn(&config, sink))?;
    read_until(&session, &rx, Duration::from_secs(20), |text| replies(text) >= 2)?;
    let running = session.job_process_ids();
    if !session.pause(Duration::from_secs(5)) {
        return Err("the session did not pause".into());
    }
    let parts = session.parts().map_err(|e| format!("parts: {e}"))?;
    let target = Process::open(parent).map_err(|e| e.to_string())?;
    let given = handoff::duplicate_into(&target, &parts).map_err(|e| e.to_string())?;
    let line = serde_json::json!({ "parts": given, "running": running });
    let mut out = std::io::stdout();
    writeln!(out, "{line}").and_then(|()| out.flush()).map_err(|e| e.to_string())?;
    // As the Nekominal that is replaced does: no destructor runs, no pseudoconsole is closed.
    // SAFETY: ends this process.
    unsafe { windows_sys::Win32::System::Threading::TerminateProcess(windows_sys::Win32::System::Threading::GetCurrentProcess(), 0) };
    unreachable!("the process ended")
}

fn parent() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .env(CHILD_MARKER, std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot start the child: {e}"))?;
    let mut line = String::new();
    BufReader::new(child.stdout.take().ok_or("no stdout")?).read_line(&mut line).map_err(|e| e.to_string())?;
    let given: serde_json::Value = serde_json::from_str(&line).map_err(|e| format!("the child said {line:?}: {e}"))?;
    let parts: Parts = serde_json::from_value(given["parts"].clone()).map_err(|e| e.to_string())?;
    let running: Vec<u32> = serde_json::from_value(given["running"].clone()).map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("the child exited with {status}"));
    }

    // The child is gone: what it ran is still there, the ping included, though nobody reads its output.
    thread::sleep(Duration::from_secs(2));
    if running.len() < 2 {
        return Err(format!("cmd and ping were not both running in the child: {running:?}"));
    }
    for pid in &running {
        let process = Process::open(*pid).map_err(|e| format!("process {pid} of the session ended with the child: {e}"))?;
        if process.exited_within(Duration::ZERO) {
            return Err(format!("process {pid} of the session ended with the child"));
        }
    }

    let sessions = Sessions::default();
    let (session, rx) = started(&sessions, |sessions, sink| sessions.adopt(&parts, sink))?;
    if session.job_process_ids() != running {
        return Err(format!("the job holds {:?}, the child's held {running:?}", session.job_process_ids()));
    }
    // What ping wrote while nobody read, then what it writes now.
    read_until(&session, &rx, Duration::from_secs(15), |text| replies(text) >= 3)?;
    // Input reaches it (Ctrl+C stops ping, cmd /k stays), and a resize through the packed pseudoconsole reaches cmd.
    session.write(b"\x03").map_err(|e| e.to_string())?;
    session.resize(57, 21).map_err(|e| format!("resize: {e}"))?;
    session.write(b"mode con\r\n").map_err(|e| e.to_string())?;
    read_until(&session, &rx, Duration::from_secs(15), |text| text.contains("57") && text.contains("21"))?;
    session.write(b"exit 5\r\n").map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let left = deadline.checked_duration_since(Instant::now()).ok_or("cmd did not exit")?;
        match rx.recv_timeout(left).map_err(|_| "cmd did not exit".to_owned())? {
            Event::Data(bytes) => session.ack(bytes.len()),
            Event::Exit(5) => break,
            Event::Exit(code) => return Err(format!("cmd exited with {code}, not 5")),
        }
    }
    Ok(())
}
