//! PTY sessions: output coalescing, ack-based backpressure and exit delivery.
//!
//! Three threads serve each session. The reader drains the ConPTY output pipe, the waiter blocks
//! on the child process, and the forwarder merges both into ordered events for the front end.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU32, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use crate::conpty::{Pty, SpawnConfig};

const READ_BUFFER_SIZE: usize = 32 * 1024;
/// Output frames are merged up to this size so the IPC layer sees few large payloads.
const FRAME_MAX: usize = 64 * 1024;
const FRAME_WINDOW: Duration = Duration::from_millis(2);
/// The child's output stops being read once this many bytes await acknowledgement.
const HIGH_WATERMARK: usize = 256 * 1024;
/// ConPTY may emit trailing output shortly after the child exits.
const EXIT_QUIET_PERIOD: Duration = Duration::from_millis(120);

pub enum Event {
    Data(Vec<u8>),
    Exit(u32),
}

/// Receives events in order. Returns `false` once the consumer is gone.
pub type Sink = Box<dyn FnMut(Event) -> bool + Send>;

enum Message {
    Data(Vec<u8>),
    Exit(u32),
    Eof,
}

#[derive(Default)]
struct FlowState {
    unacked: usize,
    closed: bool,
}

#[derive(Default)]
struct Flow {
    state: Mutex<FlowState>,
    changed: Condvar,
}

impl Flow {
    /// Blocks while too much output is unacknowledged. Returns `false` once the session is closed.
    fn wait_for_room(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        while state.unacked >= HIGH_WATERMARK && !state.closed {
            state = self.changed.wait(state).unwrap();
        }
        !state.closed
    }

    fn add(&self, bytes: usize) {
        self.state.lock().unwrap().unacked += bytes;
    }

    fn ack(&self, bytes: usize) {
        let mut state = self.state.lock().unwrap();
        state.unacked = state.unacked.saturating_sub(bytes);
        self.changed.notify_all();
    }

    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.changed.notify_all();
    }
}

pub struct Session {
    pty: Arc<Pty>,
    flow: Arc<Flow>,
}

impl Session {
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.pty.write(bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.pty.resize(cols, rows)
    }

    pub fn ack(&self, bytes: usize) {
        self.flow.ack(bytes);
    }

    /// The program the session is running now, with its path; see [`Pty::job`].
    pub fn job(&self) -> Option<crate::conpty::Job> {
        self.pty.job()
    }

    /// The program the session is running now, from processes listed once for many sessions; see [`Pty::job_with`].
    pub fn job_with(&self, names: &[(u32, String)]) -> Option<crate::conpty::Job> {
        self.pty.job_with(names)
    }

    /// Every process of the session's job; see [`Pty::job_process_ids`].
    pub fn job_process_ids(&self) -> Vec<u32> {
        self.pty.job_process_ids()
    }
}

#[derive(Default)]
pub struct Sessions {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<Session>>>,
}

impl Sessions {
    /// Starts a child process. Returns the session id and the child's pid.
    pub fn spawn(&self, config: &SpawnConfig, sink: Sink) -> io::Result<(u32, u32)> {
        let (pty, output) = Pty::spawn(config)?;
        let pid = pty.pid;
        let pty = Arc::new(pty);
        let flow = Arc::new(Flow::default());
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;

        let (tx, rx) = mpsc::channel();
        {
            let (flow, tx) = (flow.clone(), tx.clone());
            thread::Builder::new()
                .name(format!("pty-{id}-read"))
                .spawn(move || read_loop(output, &flow, &tx))?;
        }
        {
            let pty = pty.clone();
            thread::Builder::new()
                .name(format!("pty-{id}-wait"))
                .spawn(move || {
                    let _ = tx.send(Message::Exit(pty.wait()));
                })?;
        }
        {
            let (pty, flow) = (pty.clone(), flow.clone());
            thread::Builder::new()
                .name(format!("pty-{id}-forward"))
                .spawn(move || forward_loop(&rx, sink, &pty, &flow))?;
        }

        self.sessions
            .lock()
            .unwrap()
            .insert(id, Arc::new(Session { pty, flow }));
        Ok((id, pid))
    }

    pub fn get(&self, id: u32) -> io::Result<Arc<Session>> {
        self.sessions
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no session {id}")))
    }

    /// Terminates the child tree and forgets the session. Must not panic: a pane dropped by a
    /// thread-local destructor calls this while the process exits.
    pub fn close(&self, id: u32) {
        let Some(session) = self.sessions.lock().unwrap().remove(&id) else {
            return;
        };
        session.flow.close();
        session.pty.kill();
        // Closing the pseudoconsole can block until the output pipe is drained, so it normally
        // runs off the caller's thread. During process exit the OS refuses new threads
        // (ERROR_ACCESS_DENIED), which makes `thread::spawn` panic; close inline then.
        let worker = session.pty.clone();
        let spawned = thread::Builder::new()
            .name(format!("pty-{id}-close"))
            .spawn(move || worker.close_console());
        if spawned.is_err() {
            session.pty.close_console_if_nonblocking();
        }
    }
}

fn read_loop(mut output: File, flow: &Flow, tx: &Sender<Message>) {
    let mut buffer = vec![0u8; READ_BUFFER_SIZE];
    let mut forwarding = true;
    loop {
        let open = flow.wait_for_room();
        let read = match output.read(&mut buffer) {
            Ok(0) | Err(_) => break, // a broken pipe is the normal end of the stream
            Ok(read) => read,
        };
        // After close the stream is still drained so ClosePseudoConsole cannot block on it.
        if open && forwarding {
            flow.add(read);
            forwarding = tx.send(Message::Data(buffer[..read].to_vec())).is_ok();
        }
    }
    let _ = tx.send(Message::Eof);
}

#[derive(Default)]
struct Frame {
    data: Vec<u8>,
    exit: Option<u32>,
    eof: bool,
}

impl Frame {
    fn apply(&mut self, message: Message) {
        match message {
            Message::Data(bytes) => self.data.extend_from_slice(&bytes),
            Message::Exit(code) => self.exit = Some(code),
            Message::Eof => self.eof = true,
        }
    }
}

fn forward_loop(rx: &Receiver<Message>, mut sink: Sink, pty: &Pty, flow: &Flow) {
    let mut frame = Frame::default();
    let mut consumer_alive = true;
    let exit_code = loop {
        let received = match frame.exit {
            Some(_) => rx.recv_timeout(EXIT_QUIET_PERIOD),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let mut finished = false;
        match received {
            Ok(message) => frame.apply(message),
            // Silence after the exit, or every sender gone, means the stream is complete.
            Err(_) => finished = true,
        }

        // Merge whatever else arrives within the window into the same frame.
        let deadline = Instant::now() + FRAME_WINDOW;
        while !finished && frame.data.len() < FRAME_MAX {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            match rx.recv_timeout(remaining) {
                Ok(message) => frame.apply(message),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => finished = true,
            }
        }

        if !frame.data.is_empty() {
            let data = std::mem::take(&mut frame.data);
            consumer_alive &= sink(Event::Data(data));
        }
        if !consumer_alive {
            flow.close();
            pty.kill();
        }
        if let Some(code) = frame.exit {
            if finished || frame.eof {
                break code;
            }
        } else if finished {
            break u32::MAX;
        }
    };
    sink(Event::Exit(exit_code));
    pty.close_console();
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, sync::mpsc::channel};

    use super::*;
    use crate::conpty::{WidthModel, test_support::ProcessWatch};

    /// ConPTY asks the hosting terminal for its attributes (DA1) at startup. Neither the bundled
    /// nor the in-box ConPTY holds the child's output back until the reply arrives
    /// (`first_output_arrives_without_a_da1_reply`), but a terminal always answers: programs send
    /// DA1 after a feature probe such as `CSI ? u` and treat its reply as the end of the probe.
    const DA1_QUERY: &[u8] = b"\x1b[c";
    const DA1_REPLY: &[u8] = b"\x1b[?1;2c";

    fn cmd_config(args: &[&str]) -> SpawnConfig {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        SpawnConfig {
            program: PathBuf::from(system_root).join("System32").join("cmd.exe"),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: None,
            env: BTreeMap::new(),
            cols: 100,
            rows: 30,
            width_model: WidthModel::default(),
            inherit_environment: true,
        }
    }

    fn start(sessions: &Sessions, config: &SpawnConfig) -> (u32, Arc<Session>, Receiver<Event>) {
        let (tx, rx) = channel();
        let sink: Sink = Box::new(move |event| tx.send(event).is_ok());
        let (id, _) = sessions.spawn(config, sink).expect("spawn");
        (id, sessions.get(id).unwrap(), rx)
    }

    fn answer_da1(session: &Session, bytes: &[u8]) {
        if bytes
            .windows(DA1_QUERY.len())
            .any(|window| window == DA1_QUERY)
        {
            session.write(DA1_REPLY).unwrap();
        }
    }

    /// Plays the terminal's role until the child exits: answers DA1 and acknowledges output.
    fn drive(session: &Session, rx: &Receiver<Event>, timeout: Duration) -> (String, u32) {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("timed out waiting for exit");
            match rx.recv_timeout(left).expect("event") {
                Event::Data(bytes) => {
                    answer_da1(session, &bytes);
                    session.ack(bytes.len());
                    output.extend(bytes);
                }
                Event::Exit(code) => return (String::from_utf8_lossy(&output).into_owned(), code),
            }
        }
    }

    #[test]
    fn runs_a_command_and_reports_its_exit_code() {
        let sessions = Sessions::default();
        let (_, session, rx) = start(
            &sessions,
            &cmd_config(&["/d", "/c", "echo hello-nekominal & exit 7"]),
        );
        let (output, code) = drive(&session, &rx, Duration::from_secs(20));
        assert!(output.contains("hello-nekominal"), "output was {output:?}");
        assert_eq!(code, 7);
    }

    #[test]
    fn writes_input_and_exits_when_the_shell_exits() {
        let sessions = Sessions::default();
        let (_, session, rx) = start(&sessions, &cmd_config(&["/d"]));
        session.write(b"echo ping-pong-%OS%\r\n").unwrap();
        session.write(b"exit\r\n").unwrap();
        let (output, code) = drive(&session, &rx, Duration::from_secs(20));
        assert!(
            output.contains("ping-pong-Windows_NT"),
            "output was {output:?}"
        );
        assert_eq!(code, 0);
    }

    #[test]
    fn resize_reaches_the_child() {
        let sessions = Sessions::default();
        let (_, session, rx) = start(&sessions, &cmd_config(&["/d"]));
        session.resize(57, 21).unwrap();
        session.write(b"mode con\r\n").unwrap();
        session.write(b"exit\r\n").unwrap();
        let (output, _) = drive(&session, &rx, Duration::from_secs(20));
        assert!(output.contains("57"), "output was {output:?}");
        assert!(output.contains("21"), "output was {output:?}");
    }

    #[test]
    fn closing_a_session_kills_the_process_tree() {
        let sessions = Sessions::default();
        let (id, session, rx) = start(
            &sessions,
            &cmd_config(&["/d", "/c", "ping -n 60 127.0.0.1 >nul"]),
        );
        std::thread::sleep(Duration::from_millis(800));
        sessions.close(id);
        let (_, code) = drive(&session, &rx, Duration::from_secs(10));
        assert_eq!(code, 1);
    }

    #[test]
    fn closing_a_session_kills_grandchildren_through_the_job_object() {
        let sessions = Sessions::default();
        // cmd (root) -> cmd -> ping: ping is a grandchild of the process the session started.
        let config = cmd_config(&["/d", "/c", "cmd /d /c ping -n 60 127.0.0.1 >nul"]);
        let (id, session, _rx) = start(&sessions, &config);

        let deadline = Instant::now() + Duration::from_secs(10);
        let watches: Vec<ProcessWatch> = loop {
            let pids = session.pty.job_process_ids();
            if pids.len() >= 3 {
                assert!(
                    pids.contains(&session.pty.pid),
                    "root {} not in {pids:?}",
                    session.pty.pid
                );
                break pids.into_iter().filter_map(ProcessWatch::open).collect();
            }
            assert!(
                Instant::now() < deadline,
                "process tree never grew to 3: {pids:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(watches.len() >= 3);

        // The session stays referenced here, so the kill-on-close of the job handle cannot be what
        // ends them: closing has to terminate the job itself.
        sessions.close(id);
        for (index, watch) in watches.iter().enumerate() {
            assert!(
                watch.exited_within(Duration::from_secs(5)),
                "process {index} of the tree survived the close"
            );
        }
    }

    #[test]
    fn the_job_is_the_program_that_started_last_and_never_a_helper_of_the_console() {
        let sessions = Sessions::default();
        // cmd (root) -> cmd -> ping: the program the user would say is running is ping.
        let config = cmd_config(&["/d", "/c", "cmd /d /c ping -n 60 127.0.0.1 >nul"]);
        let (id, session, _rx) = start(&sessions, &config);

        let deadline = Instant::now() + Duration::from_secs(10);
        let job = loop {
            match session.job() {
                Some(job) if job.name.eq_ignore_ascii_case("ping.exe") => break job,
                other => assert!(Instant::now() < deadline, "the job was {other:?}"),
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let path = job.path.expect("the path of the program, for its icon");
        assert!(path.is_absolute() && path.file_name().is_some_and(|name| name.eq_ignore_ascii_case("ping.exe")), "{path:?}");
        assert!(!job.root, "ping runs, the shell is not alone");
        sessions.close(id);
    }

    #[test]
    fn a_shell_with_nothing_running_is_alone_in_its_job() {
        let sessions = Sessions::default();
        let config = cmd_config(&["/d", "/k"]);
        let (id, session, _rx) = start(&sessions, &config);
        let deadline = Instant::now() + Duration::from_secs(10);
        let job = loop {
            match session.job() {
                Some(job) if job.name.eq_ignore_ascii_case("cmd.exe") => break job,
                other => assert!(Instant::now() < deadline, "the job was {other:?}"),
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(job.root, "cmd at its prompt is the root of the job");
        sessions.close(id);
    }

    #[test]
    fn closing_while_output_is_flowing_does_not_deadlock() {
        let sessions = Sessions::default();
        let config = cmd_config(&[
            "/d",
            "/c",
            "for /L %i in (1,1,2000000) do @echo flood-%i-padding-padding-padding-padding",
        ]);
        let (id, session, rx) = start(&sessions, &config);

        // Let the flood back up to the watermark; nothing is acknowledged.
        let mut delivered = 0usize;
        let started = Instant::now();
        while delivered < HIGH_WATERMARK / 2 {
            if let Ok(Event::Data(bytes)) = rx.recv_timeout(Duration::from_millis(100)) {
                answer_da1(&session, &bytes);
                delivered += bytes.len();
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "only {delivered} bytes arrived"
            );
        }

        let closing = Instant::now();
        sessions.close(id);
        assert!(
            closing.elapsed() < Duration::from_secs(2),
            "close blocked for {:?}",
            closing.elapsed()
        );

        // The reader keeps draining for the console to close and the exit is reported, with no
        // acknowledgement from the consumer.
        let deadline = Instant::now() + Duration::from_secs(15);
        let code = loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("no exit after close");
            if let Event::Exit(code) = rx.recv_timeout(left).expect("event after close") {
                break code;
            }
        };
        assert_eq!(code, 1);
        while !session.pty.console_closed() {
            assert!(
                Instant::now() < deadline,
                "the pseudoconsole was never closed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn exit_is_reported_once_with_the_childs_exit_code() {
        let sessions = Sessions::default();
        let (_, session, rx) = start(&sessions, &cmd_config(&["/d", "/c", "exit 42"]));
        let (_, code) = drive(&session, &rx, Duration::from_secs(20));
        assert_eq!(code, 42);
        // The forwarder ends right after Exit and drops the sink, which disconnects the channel.
        // Any further event, or silence, would mean a second report or a stuck forwarder.
        assert!(
            matches!(
                rx.recv_timeout(Duration::from_secs(3)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ),
            "the stream continued after Exit"
        );
    }

    #[test]
    fn closing_reports_exit_once() {
        let sessions = Sessions::default();
        let (id, session, rx) = start(
            &sessions,
            &cmd_config(&["/d", "/c", "ping -n 60 127.0.0.1 >nul"]),
        );
        std::thread::sleep(Duration::from_millis(500));
        sessions.close(id);
        sessions.close(id);
        let (_, code) = drive(&session, &rx, Duration::from_secs(10));
        assert_eq!(code, 1);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(3)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn closing_from_a_thread_local_destructor_does_not_panic() {
        struct CloseOnExit {
            sessions: Arc<Sessions>,
            id: u32,
        }
        impl Drop for CloseOnExit {
            fn drop(&mut self) {
                self.sessions.close(self.id);
            }
        }
        thread_local! {
            static GUARD: RefCell<Option<CloseOnExit>> = const { RefCell::new(None) };
        }

        let sessions = Arc::new(Sessions::default());
        let (id, session, rx) = start(
            &sessions,
            &cmd_config(&["/d", "/c", "ping -n 60 127.0.0.1 >nul"]),
        );
        let worker = {
            let sessions = sessions.clone();
            std::thread::spawn(move || {
                GUARD.with(|guard| *guard.borrow_mut() = Some(CloseOnExit { sessions, id }));
            })
        };
        worker.join().expect("the destructor panicked");
        assert!(sessions.get(id).is_err(), "the session survived its guard");
        let (_, code) = drive(&session, &rx, Duration::from_secs(10));
        assert_eq!(code, 1);
        // The same call during real process exit is covered by tests/close_at_exit.rs.
    }

    #[test]
    fn starts_in_the_requested_directory() {
        let dir = std::env::temp_dir().join("nekominal-pty-cwd-test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = cmd_config(&["/d", "/c", "cd"]);
        config.cwd = Some(dir.clone());
        let sessions = Sessions::default();
        let (_, session, rx) = start(&sessions, &config);
        let (output, _) = drive(&session, &rx, Duration::from_secs(20));
        let expected = dir.to_string_lossy().to_lowercase();
        assert!(
            output.to_lowercase().contains(&expected),
            "output was {output:?}"
        );
    }

    #[test]
    fn an_invalid_or_missing_cwd_starts_in_the_home_directory() {
        let home = std::env::var("USERPROFILE")
            .expect("USERPROFILE")
            .to_lowercase();
        let missing = std::env::temp_dir().join("nekominal-pty-no-such-directory");
        for cwd in [Some(missing), None] {
            let mut config = cmd_config(&["/d", "/c", "cd"]);
            config.cwd = cwd.clone();
            let sessions = Sessions::default();
            let (_, session, rx) = start(&sessions, &config);
            let (output, _) = drive(&session, &rx, Duration::from_secs(20));
            assert!(
                output.to_lowercase().contains(&home),
                "cwd {cwd:?}: output was {output:?}"
            );
        }
    }

    #[test]
    fn the_store_powershell_alias_runs_inside_the_job() {
        // Skipped on machines without PowerShell 7.
        let Some(pwsh) = crate::profiles::resolve_program("pwsh") else {
            return;
        };
        let mut config = cmd_config(&[]);
        config.program = pwsh.clone();
        config.args = ["-NoLogo", "-NoProfile", "-Command", "Start-Sleep 30"]
            .map(String::from)
            .to_vec();
        let sessions = Sessions::default();
        let (id, session, rx) = start(&sessions, &config);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        while !seen.contains(&session.pty.pid) {
            assert!(
                Instant::now() < deadline,
                "{} (pid {}) never joined the job: {seen:?}",
                pwsh.display(),
                session.pty.pid
            );
            std::thread::sleep(Duration::from_millis(50));
            seen = session.pty.job_process_ids();
        }
        sessions.close(id);
        let (_, code) = drive(&session, &rx, Duration::from_secs(10));
        assert_eq!(code, 1);
    }

    #[test]
    fn a_job_of_more_processes_than_the_first_list_holds_lists_them_all() {
        // 70 pings at once, as a parallel build starts its compilers: more than the 64 the first query has room for.
        let sessions = Sessions::default();
        let (id, session, rx) = start(&sessions, &cmd_config(&["/d", "/c", "for /L %i in (1,1,70) do @start /b ping -n 20 127.0.0.1 >nul"]));
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut most = 0;
        while most <= 64 && Instant::now() < deadline {
            most = most.max(session.pty.job_process_ids().len());
            std::thread::sleep(Duration::from_millis(100));
        }
        sessions.close(id);
        let _ = drive(&session, &rx, Duration::from_secs(10));
        assert!(most > 64, "at most {most} processes were listed");
    }

    /// Startup handshake, kept manual like the emulator's `conpty_startup`: rerun it whenever the
    /// bundled ConPTY changes (`cargo test -p nekominal-pty -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn first_output_arrives_without_a_da1_reply() {
        let sessions = Sessions::default();
        let (_, session, rx) = start(&sessions, &cmd_config(&["/d", "/c", "echo startup-probe"]));
        let started = Instant::now();
        let mut first_output = None;
        let mut output = Vec::new();
        let deadline = started + Duration::from_secs(10);
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("no exit in time");
            match rx.recv_timeout(left).expect("event") {
                // Acknowledged, but the DA1 query is deliberately never answered.
                Event::Data(bytes) => {
                    first_output.get_or_insert(started.elapsed());
                    session.ack(bytes.len());
                    output.extend(bytes);
                }
                Event::Exit(_) => break,
            }
        }
        println!(
            "{:?}: first output after {first_output:?}, {} bytes in total",
            crate::conpty::backend(),
            output.len()
        );
        assert!(
            output.windows(DA1_QUERY.len()).any(|w| w == DA1_QUERY),
            "ConPTY sent no DA1 query"
        );
        assert!(String::from_utf8_lossy(&output).contains("startup-probe"));
        assert!(
            first_output.unwrap() < Duration::from_secs(2),
            "first output after {first_output:?}"
        );
    }

    #[test]
    fn unacknowledged_output_pauses_the_reader() {
        let sessions = Sessions::default();
        let config = cmd_config(&[
            "/d",
            "/c",
            "for /L %i in (1,1,60000) do @echo line-%i-padding-padding-padding",
        ]);
        let (_, session, rx) = start(&sessions, &config);

        // Without acknowledgements the reader must stall near the high watermark.
        let mut delivered = 0usize;
        let window = Instant::now();
        while window.elapsed() < Duration::from_secs(4) {
            if let Ok(Event::Data(bytes)) = rx.recv_timeout(Duration::from_millis(100)) {
                answer_da1(&session, &bytes);
                delivered += bytes.len();
            }
        }
        assert!(
            delivered >= HIGH_WATERMARK / 2,
            "delivered only {delivered}"
        );
        assert!(
            delivered <= HIGH_WATERMARK + READ_BUFFER_SIZE + 1024,
            "delivered {delivered}"
        );

        // Acknowledging what was received lets the command finish.
        session.ack(delivered);
        let (output, code) = drive(&session, &rx, Duration::from_secs(60));
        assert_eq!(code, 0);
        assert!(output.contains("line-60000-"), "tail missing");
    }

    #[test]
    fn starts_powershell_7_including_the_store_alias() {
        // Skipped on machines without PowerShell 7.
        let Some(pwsh) = crate::profiles::resolve_program("pwsh") else {
            return;
        };
        let mut config = cmd_config(&[]);
        config.program = pwsh;
        config.args = vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-Command".into(),
            "Write-Output pwsh-ready; exit 3".into(),
        ];
        let sessions = Sessions::default();
        let (_, session, rx) = start(&sessions, &config);
        let (output, code) = drive(&session, &rx, Duration::from_secs(30));
        assert!(output.contains("pwsh-ready"), "output was {output:?}");
        assert_eq!(code, 3);
    }
}
