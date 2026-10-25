//! Test harness: a ConPTY session (through `nekominal-pty`) whose output is fed to `TerminalCore`
//! and whose replies are written back, like the real front end will do.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, mpsc::Receiver, mpsc::channel},
    time::{Duration, Instant},
};

use nekominal_pty::{
    conpty::{SpawnConfig, WidthModel},
    profiles::resolve_program,
    session::{Event, Session, Sessions, Sink},
};
use nekominal_term::{CoreConfig, CoreEvent, TerminalCore};

pub fn pwsh() -> PathBuf {
    resolve_program("pwsh").expect("PowerShell 7 (pwsh) is required for these experiments")
}

pub fn script(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("scripts").join(name).to_string_lossy().into_owned()
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("nekominal-term");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Printable rendering of a byte stream: ESC as `\e`, other controls as `\xNN`.
pub fn escape(bytes: &[u8]) -> String {
    let mut out = String::new();
    for ch in String::from_utf8_lossy(bytes).chars() {
        match ch {
            '\x1b' => out.push_str("\\e"),
            '\x07' => out.push_str("\\a"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 || c == '\x7f' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone, Copy)]
pub struct HostOptions {
    pub answer_replies: bool,
    /// Feed the emulator one byte at a time so events can be tied to the exact cursor position.
    pub bytewise: bool,
    pub graphemes: bool,
    pub cols: u16,
    pub rows: u16,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self { answer_replies: true, bytewise: false, graphemes: false, cols: 100, rows: 30 }
    }
}

pub struct Host {
    sessions: Sessions,
    id: u32,
    pub session: Arc<Session>,
    rx: Receiver<Event>,
    pub core: TerminalCore,
    pub raw: Vec<u8>,
    /// (time since spawn, frame length)
    pub frames: Vec<(Duration, usize)>,
    pub started: Instant,
    pub exit: Option<u32>,
    pub replies: Vec<u8>,
    pub events: Vec<String>,
    /// Title events with the emulator cursor `(row, column)` at the moment the title arrived.
    pub titles: Vec<(String, (usize, usize))>,
    pub options: HostOptions,
}

impl Host {
    pub fn spawn(program: PathBuf, args: &[String], options: HostOptions) -> Host {
        let config = SpawnConfig {
            program,
            args: args.to_vec(),
            cwd: None,
            env: BTreeMap::new(),
            cols: options.cols,
            rows: options.rows,
            width_model: if options.graphemes { WidthModel::Graphemes } else { WidthModel::Wcswidth },
            inherit_environment: true,
        };
        let (tx, rx) = channel();
        let sink: Sink = Box::new(move |event| tx.send(event).is_ok());
        let sessions = Sessions::default();
        let (id, _pid) = sessions.spawn(&config, sink).expect("spawn ConPTY session");
        let session = sessions.get(id).unwrap();
        let core = TerminalCore::new(options.cols as usize, options.rows as usize, CoreConfig::default());
        Host {
            sessions,
            id,
            session,
            rx,
            core,
            raw: Vec::new(),
            frames: Vec::new(),
            started: Instant::now(),
            exit: None,
            replies: Vec::new(),
            events: Vec::new(),
            titles: Vec::new(),
            options,
        }
    }

    fn drain_core(&mut self) {
        for event in self.core.take_events() {
            match event {
                CoreEvent::PtyWrite(bytes) => {
                    self.replies.extend_from_slice(&bytes);
                    if self.options.answer_replies {
                        self.session.write(&bytes).expect("write reply");
                    }
                }
                CoreEvent::Title(title) => {
                    let cursor = self.core.cursor_cell();
                    self.titles.push((title.clone(), cursor));
                    self.events.push(format!("Title({title:?})"));
                }
                other => self.events.push(format!("{other:?}")),
            }
        }
    }

    /// Pumps session output until `done` returns true, the child exits (and output goes quiet) or
    /// `max` elapses. Returns whether `done` was satisfied.
    pub fn pump(&mut self, max: Duration, done: impl Fn(&Host) -> bool) -> bool {
        let deadline = Instant::now() + max;
        loop {
            if done(self) {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            match self.rx.recv_timeout(left.min(Duration::from_millis(25))) {
                Ok(Event::Data(bytes)) => {
                    self.frames.push((self.started.elapsed(), bytes.len()));
                    self.raw.extend_from_slice(&bytes);
                    if self.options.bytewise {
                        for byte in &bytes {
                            self.core.feed(std::slice::from_ref(byte));
                            self.drain_core();
                        }
                    } else {
                        self.core.feed(&bytes);
                        self.drain_core();
                    }
                    self.session.ack(bytes.len());
                }
                Ok(Event::Exit(code)) => {
                    self.exit = Some(code);
                    self.core.on_child_exit();
                    self.drain_core();
                    return done(self);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return done(self),
            }
        }
    }

    pub fn pump_until_exit(&mut self, max: Duration) -> bool {
        self.pump(max, |host| host.exit.is_some())
    }

    pub fn text(&mut self) -> String {
        let mut snapshot = nekominal_term::Snapshot::default();
        self.core.snapshot(&mut snapshot);
        snapshot.text()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.sessions.close(self.id);
    }
}

/// Every control sequence in a stream, one string per dispatch (`CSI ? 2026 h`, `OSC 7;...`, ...).
pub fn sequence_log(bytes: &[u8]) -> Vec<String> {
    use nekominal_term::alacritty_terminal::vte::{Params, Parser, Perform};

    #[derive(Default)]
    struct Log(Vec<String>);

    impl Perform for Log {
        fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
            let params: Vec<String> =
                params.iter().map(|p| p.iter().map(u16::to_string).collect::<Vec<_>>().join(":")).collect();
            let prefix: String = intermediates.iter().map(|b| *b as char).collect();
            self.0.push(format!("CSI {prefix} {} {action}", params.join(";")));
        }

        fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
            let text: Vec<String> = params.iter().map(|p| String::from_utf8_lossy(p).into_owned()).collect();
            self.0.push(format!("OSC {} {}", text.join(";"), if bell_terminated { "BEL" } else { "ST" }));
        }

        fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
            let prefix: String = intermediates.iter().map(|b| *b as char).collect();
            self.0.push(format!("ESC {prefix}{}", byte as char));
        }

        fn hook(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
            let prefix: String = intermediates.iter().map(|b| *b as char).collect();
            let params: Vec<String> =
                params.iter().map(|p| p.iter().map(u16::to_string).collect::<Vec<_>>().join(":")).collect();
            self.0.push(format!("DCS {prefix} {} {action}", params.join(";")));
        }
    }

    let mut parser = Parser::new();
    let mut log = Log::default();
    parser.advance(&mut log, bytes);
    log.0
}

pub mod pane;
pub mod raw_conpty;
