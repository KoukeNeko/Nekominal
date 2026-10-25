//! Closing a session from a thread-local destructor while the process exits must neither panic
//! nor hang.
//!
//! During exit the OS refuses to create threads (`ERROR_ACCESS_DENIED`). `Sessions::close` used to
//! `thread::spawn` and panicked when a pane was dropped by a main-thread TLS destructor. That
//! needs a real process exit, so this test has no harness: it runs itself again as a child that
//! returns from `main` with a live session held by a TLS destructor.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::Read,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use nekominal_pty::{
    conpty::{SpawnConfig, WidthModel},
    session::{Event, Sessions, Sink},
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

const CHILD_MARKER: &str = "NEKOMINAL_PTY_CLOSE_AT_EXIT_CHILD";
const CHILD_TIMEOUT: Duration = Duration::from_secs(30);

struct CloseOnDrop {
    sessions: Sessions,
    id: u32,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        // Reported so the log shows whether the exit-time condition that matters was reached.
        let spawn_refused = thread::Builder::new().spawn(|| {}).is_err();
        eprintln!("destructor: thread spawn refused = {spawn_refused}");
        self.sessions.close(self.id);
        eprintln!("destructor: close returned");
    }
}

thread_local! {
    static GUARD: RefCell<Option<CloseOnDrop>> = const { RefCell::new(None) };
}

fn main() -> ExitCode {
    if std::env::var_os(CHILD_MARKER).is_some() {
        child()
    } else {
        parent()
    }
}

fn child() -> ExitCode {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let config = SpawnConfig {
        program: PathBuf::from(system_root).join("System32").join("cmd.exe"),
        args: ["/d", "/c", "ping -n 60 127.0.0.1 >nul"]
            .map(String::from)
            .to_vec(),
        cwd: None,
        env: BTreeMap::new(),
        cols: 80,
        rows: 24,
        width_model: WidthModel::default(),
        inherit_environment: true,
    };
    let sink: Sink = Box::new(|event| matches!(event, Event::Data(_) | Event::Exit(_)));
    let sessions = Sessions::default();
    let (id, pid) = match sessions.spawn(&config, sink) {
        Ok(started) => started,
        Err(error) => {
            eprintln!("spawn failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("root pid = {pid}");
    GUARD.with(|guard| *guard.borrow_mut() = Some(CloseOnDrop { sessions, id }));
    // Returning from main starts the process exit; the destructor runs inside it.
    ExitCode::SUCCESS
}

fn parent() -> ExitCode {
    match run_parent() {
        Ok(()) => {
            println!("close_at_exit ... ok");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("close_at_exit ... FAILED: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run_parent() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .env(CHILD_MARKER, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start the child: {e}"))?;
    let mut stderr = child.stderr.take().ok_or("no child stderr")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        let _ = tx.send(text);
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > CHILD_TIMEOUT {
            let _ = child.kill();
            return Err("the child hung while exiting".into());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let log = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|e| format!("no child log: {e}"))?;
    println!("child log:\n{log}");

    if log.contains("panicked") {
        return Err("the child panicked".into());
    }
    if !status.success() {
        return Err(format!("the child exited with {status}"));
    }
    if !log.contains("destructor: close returned") {
        return Err("the destructor did not run to the end".into());
    }

    // The child's whole tree must be gone once it has exited.
    let pid: u32 = log
        .lines()
        .find_map(|line| line.strip_prefix("root pid = "))
        .and_then(|pid| pid.trim().parse().ok())
        .ok_or("the child did not report its session pid")?;
    // SAFETY: plain Win32 calls; a null handle means the process no longer exists.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !handle.is_null() && WaitForSingleObject(handle, 5_000) != 0 {
            return Err(format!("session process {pid} survived the exit"));
        }
    }
    Ok(())
}
