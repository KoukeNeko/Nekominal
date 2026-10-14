//! `Probe`: one program in one ConPTY session feeding one `TerminalCore`, driven like the pane host does it.
//!
//! Threads, as in `nekominal_pane::host::Pane`:
//! * the PTY forwarder thread calls the sink; the sink feeds the emulator under a `FairMutex`, then acks the
//!   bytes (the back-pressure);
//! * every byte sent to the program (keys, text, paste, mouse, focus and the emulator's own replies) goes
//!   through one channel to one writer thread, because `Session::write` blocks;
//! * a ticker thread plays the UI loop's part for DEC 2026: it flushes a synchronized update whose 150 ms
//!   timeout has passed.
//!
//! Safety: by default a probe refuses any input that could submit a prompt (see [`Guard`]).

use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use nekominal_input::{
    InputModes, KeyAssembler, KeyInput, KeyOutput, KeyboardEvent, MouseEncoding, MouseInput, MouseMode, MouseState,
    encode_focus, encode_key, encode_mouse, encode_text, prepare_paste,
};
use nekominal_pty::{
    conpty::{SpawnConfig, WidthModel},
    session::{Event, Session, Sessions, Sink},
};
use nekominal_term::{
    CoreConfig, CoreEvent, Snapshot, TerminalCore,
    agent_status::Report,
    alacritty_terminal::{grid::Scroll, sync::FairMutex, term::TermMode},
    osc::{Progress, PromptMark},
};

use crate::{
    env,
    inventory::Inventory,
    keys::{KeySpec, events_for},
    screen::Screen,
    scrub::scrub_screen,
};

// ---- errors ----------------------------------------------------------------------------------------

pub enum ProbeError {
    /// A wait ran out. `screen` is already scrubbed.
    Timeout {
        what: String,
        waited: Duration,
        exit: Option<u32>,
        screen: String,
    },
    /// The program exited while the probe waited for something else.
    Exited {
        what: String,
        code: u32,
        screen: String,
    },
    /// The no-model guard refused an input.
    Refused(String),
    Spawn(io::Error),
    Other(String),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeError::Timeout { what, waited, exit, screen } => {
                let state = match exit {
                    Some(code) => format!("the program had exited with code {code}"),
                    None => "the program was still running".to_owned(),
                };
                write!(f, "timed out after {waited:.1?} waiting for {what}; {state}\n--- screen ---\n{screen}")
            }
            ProbeError::Exited { what, code, screen } => {
                write!(f, "the program exited with code {code} while waiting for {what}\n--- screen ---\n{screen}")
            }
            ProbeError::Refused(reason) => write!(f, "input refused by the safety guard: {reason}"),
            ProbeError::Spawn(error) => write!(f, "could not start the program: {error}"),
            ProbeError::Other(message) => f.write_str(message),
        }
    }
}

impl fmt::Debug for ProbeError {
    // `unwrap()` and `expect()` print Debug: make that the readable message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for ProbeError {}

pub type Result<T> = std::result::Result<T, ProbeError>;

// ---- options ---------------------------------------------------------------------------------------

/// What a probe may send that could make an agent CLI call its model.
#[derive(Clone, Copy)]
pub enum Guard {
    /// No restriction. Refused for a program whose name contains "claude".
    Off,
    /// Anything that can submit a prompt (CR, LF, Enter and its variants, a multi-line paste without bracketed
    /// paste) is refused unless the input box holds an allow-listed local slash command.
    NoModelCalls(EnterPolicy),
}

/// Which input box content makes Enter safe.
#[derive(Clone, Copy)]
pub struct EnterPolicy {
    /// Local slash commands (with the leading `/`).
    pub allowed_commands: &'static [&'static str],
    /// The text of the input box on this screen, if one is drawn.
    pub read_input_box: fn(&Screen) -> Option<String>,
}

impl EnterPolicy {
    /// True when `text` is exactly an allow-listed command (whitespace collapsed). Arguments only count when
    /// the list names them, so nothing can hide a prompt behind a command word.
    pub fn allows(&self, text: &str) -> bool {
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        normalized.starts_with('/') && self.allowed_commands.contains(&normalized.as_str())
    }
}

/// How key chords are turned into bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRouting {
    /// `KeyAssembler`: kitty bytes when the program pushed keyboard flags, win32-input-mode records otherwise.
    ConptyRoute,
    /// What `nekominal_pane::Pane::send_key` does today: `encode_key` only (legacy or kitty), never win32 records.
    PaneHost,
}

pub struct SpawnOptions {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    /// Variables applied on top of the allow-listed environment (the terminal's contract, scenario extras).
    pub env: BTreeMap<String, String>,
    /// Names the files under `target/agent-probe/`.
    pub scenario: String,
    pub guard: Guard,
    pub routing: KeyRouting,
    pub core: CoreConfig,
    /// Where taps and logs go; default `<workspace>/target/agent-probe`.
    pub tap_dir: PathBuf,
}

/// `<workspace>/target/agent-probe`, or `AGENT_PROBE_DIR`.
pub fn default_tap_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AGENT_PROBE_DIR") {
        return PathBuf::from(dir);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join("agent-probe")
}

impl SpawnOptions {
    /// A program that is not an agent CLI (self-tests): no guard, default terminal environment.
    pub fn new(program: impl Into<PathBuf>, scenario: &str) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            cols: 120,
            rows: 30,
            env: nekominal_pty::profiles::default_environment(),
            scenario: scenario.to_owned(),
            guard: Guard::Off,
            routing: KeyRouting::ConptyRoute,
            core: CoreConfig::default(),
            tap_dir: default_tap_dir(),
        }
    }

    pub fn args(mut self, args: &[&str]) -> Self {
        self.args = args.iter().map(|a| (*a).to_owned()).collect();
        self
    }

    pub fn size(mut self, cols: u16, rows: u16) -> Self {
        self.cols = cols;
        self.rows = rows;
        self
    }

    pub fn env(mut self, name: &str, value: &str) -> Self {
        self.env.insert(name.to_owned(), value.to_owned());
        self
    }

    pub fn agent_environment(mut self) -> Self {
        self.env.extend(nekominal_pty::profiles::agent_environment());
        self
    }

    pub fn guard(mut self, guard: Guard) -> Self {
        self.guard = guard;
        self
    }

    pub fn routing(mut self, routing: KeyRouting) -> Self {
        self.routing = routing;
        self
    }
}

// ---- observed events -------------------------------------------------------------------------------

/// What the program told the host besides drawing, in a form tests can compare.
#[derive(Clone, Debug, PartialEq)]
pub enum Observed {
    Title(String),
    ResetTitle,
    Bell,
    ClipboardStore(String),
    ClipboardLoad,
    Cwd(String),
    Notification { title: String, body: String },
    Progress(Option<Progress>),
    PromptMark(PromptMark),
    Agent(Report),
    AgentRecordsDropped,
    Unsupported(String),
    Win32InputMode(bool),
    EmulatorPanic(String),
    LineIdsReset,
    CursorBlinkingChange,
    MouseCursorDirty,
}

fn observe(event: CoreEvent) -> Option<Observed> {
    Some(match event {
        CoreEvent::PtyWrite(_) => return None,
        CoreEvent::Title(title) => Observed::Title(title),
        CoreEvent::ResetTitle => Observed::ResetTitle,
        CoreEvent::Bell => Observed::Bell,
        CoreEvent::ClipboardStore { text, .. } => Observed::ClipboardStore(text),
        CoreEvent::ClipboardLoad { .. } => Observed::ClipboardLoad,
        CoreEvent::CursorBlinkingChange => Observed::CursorBlinkingChange,
        CoreEvent::MouseCursorDirty => Observed::MouseCursorDirty,
        CoreEvent::Cwd(path) => Observed::Cwd(path),
        CoreEvent::Notification(n) => Observed::Notification { title: n.title, body: n.body },
        CoreEvent::Progress(p) => Observed::Progress(p),
        CoreEvent::PromptMark { mark, .. } => Observed::PromptMark(mark),
        CoreEvent::LineIdsReset => Observed::LineIdsReset,
        CoreEvent::Agent(report) => Observed::Agent(report),
        CoreEvent::AgentRecordsDropped => Observed::AgentRecordsDropped,
        CoreEvent::Unsupported(u) => Observed::Unsupported(format!("{u:?}")),
        CoreEvent::Win32InputMode(on) => Observed::Win32InputMode(on),
        CoreEvent::EmulatorPanic(message) => Observed::EmulatorPanic(message),
    })
}

// ---- modes -----------------------------------------------------------------------------------------

/// The terminal modes the program has set, in the terms the harness reports and asserts on.
#[derive(Clone, Debug)]
pub struct Modes {
    pub term: TermMode,
    pub alt_screen: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    pub show_cursor: bool,
    pub alternate_scroll: bool,
    pub mouse_mode: MouseMode,
    pub mouse_encoding: MouseEncoding,
    /// Active kitty keyboard flags (1 disambiguate, 2 events, 4 alternates, 8 all keys, 16 text).
    pub kitty_flags: u8,
    /// A DEC 2026 synchronized update is open.
    pub sync_pending: bool,
    /// win32-input-mode (DECSET 9001) as the program or ConPTY last set it.
    pub win32_input_mode: bool,
}

impl Modes {
    pub fn mouse_tracking(&self) -> bool {
        self.mouse_mode != MouseMode::None
    }

    pub fn summary(&self) -> String {
        format!(
            "alt_screen={} mouse={:?}/{:?} bracketed_paste={} focus={} app_cursor={} app_keypad={} kitty={:#07b} sync_open={} win32_input={} cursor_visible={} alt_scroll={}",
            self.alt_screen,
            self.mouse_mode,
            self.mouse_encoding,
            self.bracketed_paste,
            self.focus_reporting,
            self.app_cursor,
            self.app_keypad,
            self.kitty_flags,
            self.sync_pending,
            self.win32_input_mode,
            self.show_cursor,
            self.alternate_scroll
        )
    }
}

fn read_modes(core: &TerminalCore) -> (Modes, InputModes) {
    let term = core.mode();
    let mut input = InputModes::from_term_mode(term);
    // `from_term_mode` documents that the sniffer-tracked mouse modes are the host's to add.
    let extensions = core.mouse_extensions();
    if extensions.x10 {
        input.mouse_mode = MouseMode::X10;
    }
    if extensions.sgr_pixels {
        input.mouse_encoding = MouseEncoding::SgrPixels;
    } else if extensions.urxvt {
        input.mouse_encoding = MouseEncoding::Urxvt;
    }
    let modes = Modes {
        term,
        alt_screen: input.alt_screen,
        app_cursor: input.app_cursor,
        app_keypad: input.app_keypad,
        bracketed_paste: input.bracketed_paste,
        focus_reporting: input.focus_reporting,
        show_cursor: term.contains(TermMode::SHOW_CURSOR),
        alternate_scroll: input.alternate_scroll,
        mouse_mode: input.mouse_mode,
        mouse_encoding: input.mouse_encoding,
        kitty_flags: core.kitty_flags(),
        sync_pending: core.is_sync_pending(),
        win32_input_mode: core.win32_input_mode(),
    };
    (modes, input)
}

// ---- shared state ----------------------------------------------------------------------------------

struct State {
    inventory: Inventory,
    events: Vec<(Duration, Observed)>,
    taken: usize,
    exit: Option<(Duration, u32)>,
    bytes_in: u64,
    bytes_out: u64,
    chunks: u64,
    replies: u64,
    last_output: Instant,
    /// Longest gap between two output chunks while the program ran, for storm and stall checks.
    tap: Option<BufWriter<File>>,
    log: Option<BufWriter<File>>,
}

struct Shared {
    term: Arc<FairMutex<TerminalCore>>,
    state: Mutex<State>,
    changed: Condvar,
    started: Instant,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn log(&self, line: &str) {
        let at = self.started.elapsed();
        let mut state = self.state();
        if let Some(log) = state.log.as_mut() {
            let _ = writeln!(log, "[{:>8.3}] {line}", at.as_secs_f64());
            let _ = log.flush();
        }
    }
}

/// Bytes as a readable line: ESC as `ESC`, controls as `\xNN`, valid UTF-8 text as it is.
pub fn escape_bytes(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\x1b' => out.push_str("ESC"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Handles what `feed` produced: replies go to the writer in order, everything else is recorded.
/// Returns false when the writer is gone.
fn handle_events(shared: &Shared, writer: &Sender<Outgoing>, events: Vec<CoreEvent>) -> bool {
    let at = shared.started.elapsed();
    let mut alive = true;
    for event in events {
        match event {
            CoreEvent::PtyWrite(reply) => {
                {
                    let mut state = shared.state();
                    state.inventory.note_reply(&reply, at);
                    state.replies += 1;
                }
                alive &= writer.send(Outgoing { bytes: reply, label: "reply" }).is_ok();
            }
            other => {
                if let Some(observed) = observe(other) {
                    shared.log(&format!("EVENT {observed:?}"));
                    shared.state().events.push((at, observed));
                }
            }
        }
    }
    shared.changed.notify_all();
    alive
}

struct Outgoing {
    bytes: Vec<u8>,
    label: &'static str,
}

// ---- guard statistics ------------------------------------------------------------------------------

/// What the guard let through and what it refused.
#[derive(Clone, Debug, Default)]
pub struct GuardStats {
    pub keys_sent: u64,
    pub text_sent: u64,
    pub pastes_sent: u64,
    /// Inputs that could submit a prompt and were sent because the box held an allow-listed command.
    pub submits_sent: Vec<String>,
    pub refusals: Vec<String>,
}

/// Counters of the data flow.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub chunks: u64,
    pub replies: u64,
}

// ---- the probe -------------------------------------------------------------------------------------

pub struct Probe {
    shared: Arc<Shared>,
    writer: Sender<Outgoing>,
    sessions: Sessions,
    id: u32,
    session: Arc<Session>,
    pid: u32,
    assembler: Mutex<KeyAssembler>,
    mouse: Mutex<MouseState>,
    routing: Mutex<KeyRouting>,
    guard: Guard,
    stats: Mutex<GuardStats>,
    tap_paths: (PathBuf, PathBuf),
    stop_ticker: Arc<AtomicBool>,
    closed: AtomicBool,
}

fn looks_like_agent(program: &Path) -> bool {
    program.file_stem().is_some_and(|stem| stem.to_string_lossy().to_ascii_lowercase().contains("claude"))
}

fn file_name_for(scenario: &str) -> String {
    scenario.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

impl Probe {
    pub fn spawn(options: SpawnOptions) -> Result<Probe> {
        if matches!(options.guard, Guard::Off) && looks_like_agent(&options.program) {
            return Err(ProbeError::Refused("the guard cannot be turned off for an agent CLI".into()));
        }
        let started = Instant::now();
        fs::create_dir_all(&options.tap_dir).map_err(ProbeError::Spawn)?;
        let name = file_name_for(&options.scenario);
        let tap_path = options.tap_dir.join(format!("{name}.bin"));
        let log_path = options.tap_dir.join(format!("{name}.log"));
        let tap = BufWriter::new(File::create(&tap_path).map_err(ProbeError::Spawn)?);
        let mut log = BufWriter::new(File::create(&log_path).map_err(ProbeError::Spawn)?);
        let _ = writeln!(
            log,
            "scenario {} program {} args {:?} size {}x{} routing {:?}",
            options.scenario,
            options.program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            options.args,
            options.cols,
            options.rows,
            options.routing
        );

        let term = Arc::new(FairMutex::new(TerminalCore::new(
            options.cols as usize,
            options.rows as usize,
            options.core.clone(),
        )));
        let shared = Arc::new(Shared {
            term: term.clone(),
            state: Mutex::new(State {
                inventory: Inventory::new(),
                events: Vec::new(),
                taken: 0,
                exit: None,
                bytes_in: 0,
                bytes_out: 0,
                chunks: 0,
                replies: 0,
                last_output: Instant::now(),
                tap: Some(tap),
                log: Some(log),
            }),
            changed: Condvar::new(),
            started,
        });

        let (writer_tx, writer_rx) = channel::<Outgoing>();
        let session_cell: Arc<OnceLock<Arc<Session>>> = Arc::new(OnceLock::new());
        let pending_ack = Arc::new(AtomicUsize::new(0));

        let sink: Sink = {
            let (shared, writer, session_cell, pending_ack) =
                (shared.clone(), writer_tx.clone(), session_cell.clone(), pending_ack.clone());
            Box::new(move |event| match event {
                Event::Data(bytes) => {
                    let at = shared.started.elapsed();
                    {
                        let mut state = shared.state();
                        if let Some(tap) = state.tap.as_mut() {
                            let _ = tap.write_all(&bytes);
                            let _ = tap.flush();
                        }
                        state.inventory.feed(&bytes, at);
                        state.bytes_in += bytes.len() as u64;
                        state.chunks += 1;
                        state.last_output = Instant::now();
                    }
                    let events = {
                        let mut core = shared.term.lock();
                        core.feed(&bytes);
                        core.take_events()
                    };
                    let alive = handle_events(&shared, &writer, events);
                    // Ack after the emulator consumed the frame: this is the back-pressure.
                    match session_cell.get() {
                        Some(session) => session.ack(bytes.len() + pending_ack.swap(0, Ordering::Relaxed)),
                        None => {
                            pending_ack.fetch_add(bytes.len(), Ordering::Relaxed);
                        }
                    }
                    alive
                }
                Event::Exit(code) => {
                    shared.term.lock().on_child_exit();
                    let at = shared.started.elapsed();
                    shared.log(&format!("EXIT code {code}"));
                    shared.state().exit = Some((at, code));
                    shared.changed.notify_all();
                    true
                }
            })
        };

        let config = SpawnConfig {
            program: options.program.clone(),
            args: options.args.clone(),
            cwd: options.cwd.clone(),
            env: env::child_environment(&options.env).map_err(ProbeError::Other)?,
            cols: options.cols,
            rows: options.rows,
            // The mode that agrees with alacritty_terminal's per-code-point width model, as Pane::spawn uses.
            width_model: WidthModel::Wcswidth,
        };
        let sessions = Sessions::default();
        let spawned = env::with_process_environment(&config.env.clone(), || sessions.spawn(&config, sink));
        let (id, pid) = spawned.map_err(ProbeError::Spawn)?;
        let session = sessions.get(id).map_err(ProbeError::Spawn)?;
        let _ = session_cell.set(session.clone());
        let early = pending_ack.swap(0, Ordering::Relaxed);
        if early > 0 {
            session.ack(early);
        }
        shared.log(&format!("SPAWNED pid {pid}"));

        {
            let (session, shared) = (session.clone(), shared.clone());
            thread::Builder::new()
                .name("probe-writer".into())
                .spawn(move || {
                    for outgoing in writer_rx {
                        shared.log(&format!("OUT {:<6} {}", outgoing.label, escape_bytes(&outgoing.bytes)));
                        shared.state().bytes_out += outgoing.bytes.len() as u64;
                        if session.write(&outgoing.bytes).is_err() {
                            break;
                        }
                    }
                })
                .map_err(ProbeError::Spawn)?;
        }

        let stop_ticker = Arc::new(AtomicBool::new(false));
        {
            let (shared, writer, stop) = (shared.clone(), writer_tx.clone(), stop_ticker.clone());
            thread::Builder::new()
                .name("probe-sync-ticker".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(8));
                        let events = {
                            let mut core = shared.term.lock();
                            if core.flush_expired_sync(Instant::now()) { Some(core.take_events()) } else { None }
                        };
                        if let Some(events) = events {
                            shared.log("SYNC deadline passed, frame flushed");
                            handle_events(&shared, &writer, events);
                        }
                    }
                })
                .map_err(ProbeError::Spawn)?;
        }

        Ok(Probe {
            shared,
            writer: writer_tx,
            sessions,
            id,
            session,
            pid,
            assembler: Mutex::new(KeyAssembler::new()),
            mouse: Mutex::new(MouseState::default()),
            routing: Mutex::new(options.routing),
            guard: options.guard,
            stats: Mutex::new(GuardStats::default()),
            tap_paths: (tap_path, log_path),
            stop_ticker,
            closed: AtomicBool::new(false),
        })
    }

    // ---- observation --------------------------------------------------------------------------

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// `(byte tap, log)` paths under `target/agent-probe`.
    pub fn paths(&self) -> (&Path, &Path) {
        (&self.tap_paths.0, &self.tap_paths.1)
    }

    pub fn elapsed(&self) -> Duration {
        self.shared.started.elapsed()
    }

    /// Appends a line to the scenario log.
    pub fn note(&self, line: &str) {
        self.shared.log(&format!("NOTE {line}"));
    }

    /// Copy of the visible grid.
    pub fn screen(&self) -> Screen {
        let mut core = self.shared.term.lock();
        let mut snapshot = Snapshot::default();
        core.snapshot(&mut snapshot);
        let alt = core.mode().contains(TermMode::ALT_SCREEN);
        Screen::from_snapshot(&snapshot, alt)
    }

    pub fn screen_text(&self) -> String {
        self.screen().text()
    }

    /// Cursor `(row, column)` in the viewport, `None` when scrolled out of view.
    pub fn cursor(&self) -> Option<(usize, usize)> {
        self.screen().cursor
    }

    pub fn modes(&self) -> Modes {
        read_modes(&self.shared.term.lock()).0
    }

    fn input_modes(&self) -> InputModes {
        read_modes(&self.shared.term.lock()).1
    }

    pub fn history_size(&self) -> usize {
        self.shared.term.lock().history_size()
    }

    pub fn display_offset(&self) -> usize {
        self.shared.term.lock().display_offset()
    }

    /// The target of the OSC 8 hyperlink on a visible cell, if the cell has one.
    pub fn hyperlink_at(&self, row: usize, column: usize) -> Option<String> {
        self.shared.term.lock().hyperlink_at(row, column).map(|link| link.uri().to_owned())
    }

    /// The scrollback, oldest line first, trailing blanks dropped.
    pub fn history_lines(&self) -> Vec<String> {
        let core = self.shared.term.lock();
        let last_column = core.columns().saturating_sub(1);
        (-(core.history_size() as i32)..0)
            .map(|line| core.text_between((line, 0), (line, last_column)).trim_end().to_owned())
            .collect()
    }

    pub fn size(&self) -> (usize, usize) {
        let core = self.shared.term.lock();
        (core.columns(), core.rows())
    }

    /// Scrolls the viewport through the scrollback (positive: toward older lines).
    pub fn scroll_view(&self, lines: i32) {
        self.shared.term.lock().scroll_display(Scroll::Delta(lines));
    }

    /// Events (title, bell, notifications, OSC 7501, cwd, progress, clipboard, ...) since the last call.
    pub fn take_events(&self) -> Vec<Observed> {
        let mut state = self.shared.state();
        let new: Vec<Observed> = state.events[state.taken..].iter().map(|(_, e)| e.clone()).collect();
        state.taken = state.events.len();
        new
    }

    /// Every event so far with its time.
    pub fn all_events(&self) -> Vec<(Duration, Observed)> {
        self.shared.state().events.clone()
    }

    pub fn counters(&self) -> Counters {
        let state = self.shared.state();
        Counters { bytes_in: state.bytes_in, bytes_out: state.bytes_out, chunks: state.chunks, replies: state.replies }
    }

    /// Runs `f` on the inventory of sequences seen so far.
    pub fn with_inventory<R>(&self, f: impl FnOnce(&Inventory) -> R) -> R {
        f(&self.shared.state().inventory)
    }

    pub fn inventory_report(&self) -> String {
        self.with_inventory(Inventory::report)
    }

    pub fn exit_code(&self) -> Option<u32> {
        self.shared.state().exit.map(|(_, code)| code)
    }

    /// Text attributes the program left set when it exited: writes one character at the cursor and reports
    /// the colours and attributes it got, `None` when they are the defaults. Only valid after the exit,
    /// because it writes into the emulator.
    pub fn leftover_attributes(&self) -> Result<Option<String>> {
        if self.exit_code().is_none() {
            return Err(ProbeError::Other("leftover_attributes needs the program to have exited".into()));
        }
        let mut core = self.shared.term.lock();
        let (row, column) = core.cursor_cell();
        core.feed(b"Z");
        let mut snapshot = Snapshot::default();
        core.snapshot(&mut snapshot);
        let screen = Screen::from_snapshot(&snapshot, core.mode().contains(TermMode::ALT_SCREEN));
        let cell = screen.cell(row, column);
        Ok(if cell.ch == 'Z' && Screen::has_default_pen(cell) { None } else { Some(Screen::describe(cell)) })
    }

    pub fn guard_stats(&self) -> GuardStats {
        self.stats.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Time since the program last produced output.
    pub fn quiet_for(&self) -> Duration {
        self.shared.state().last_output.elapsed()
    }

    // ---- waiting ------------------------------------------------------------------------------

    fn timeout_error(&self, what: &str, waited: Duration) -> ProbeError {
        let screen = self.screen();
        self.write_failure_dump(what, &screen);
        ProbeError::Timeout { what: what.to_owned(), waited, exit: self.exit_code(), screen: screen.dump_scrubbed() }
    }

    fn write_failure_dump(&self, what: &str, screen: &Screen) {
        let path = self.tap_paths.1.with_extension("failure.txt");
        let _ = fs::write(path, format!("waiting for {what}\n{}", screen.dump()));
    }

    /// Waits until `predicate` holds for the screen. Evaluated when output arrives and at least every 25 ms.
    pub fn wait_until(
        &self,
        what: &str,
        timeout: Duration,
        mut predicate: impl FnMut(&Screen) -> bool,
    ) -> Result<Screen> {
        let deadline = Instant::now() + timeout;
        loop {
            let screen = self.screen();
            if predicate(&screen) {
                return Ok(screen);
            }
            if let Some(code) = self.exit_code() {
                // One more look: the final frame arrives before the exit is reported, but be sure.
                let screen = self.screen();
                if predicate(&screen) {
                    return Ok(screen);
                }
                self.write_failure_dump(what, &screen);
                return Err(ProbeError::Exited { what: what.to_owned(), code, screen: screen.dump_scrubbed() });
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(self.timeout_error(what, timeout));
            }
            let state = self.shared.state();
            let wait = (deadline - now).min(Duration::from_millis(25));
            let _ = self.shared.changed.wait_timeout(state, wait);
        }
    }

    pub fn wait_for_text(&self, needle: &str, timeout: Duration) -> Result<Screen> {
        self.wait_until(&format!("text {needle:?}"), timeout, |screen| screen.contains(needle))
    }

    pub fn wait_for_regex(&self, pattern: &str, timeout: Duration) -> Result<Screen> {
        let regex =
            regex::Regex::new(pattern).map_err(|e| ProbeError::Other(format!("bad pattern {pattern:?}: {e}")))?;
        self.wait_until(&format!("regex /{pattern}/"), timeout, |screen| regex.is_match(&screen.text()))
    }

    pub fn wait_for_gone(&self, needle: &str, timeout: Duration) -> Result<Screen> {
        self.wait_until(&format!("text {needle:?} to disappear"), timeout, |screen| !screen.contains(needle))
    }

    /// Waits until the program wrote nothing for `quiet`.
    pub fn wait_quiet(&self, quiet: Duration, timeout: Duration) -> Result<Screen> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.quiet_for() >= quiet || self.exit_code().is_some() {
                return Ok(self.screen());
            }
            if Instant::now() >= deadline {
                return Err(self.timeout_error(&format!("{quiet:?} of silence"), timeout));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits for the program to exit and returns its exit code.
    pub fn wait_exit(&self, timeout: Duration) -> Result<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(code) = self.exit_code() {
                return Ok(code);
            }
            if Instant::now() >= deadline {
                return Err(self.timeout_error("the program to exit", timeout));
            }
            let state = self.shared.state();
            let _ = self.shared.changed.wait_timeout(state, Duration::from_millis(25));
        }
    }

    // ---- sending ------------------------------------------------------------------------------

    fn send_bytes(&self, bytes: Vec<u8>, label: &'static str) {
        if !bytes.is_empty() {
            let _ = self.writer.send(Outgoing { bytes, label });
        }
    }

    fn stats(&self) -> MutexGuard<'_, GuardStats> {
        self.stats.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn refuse(&self, reason: String) -> ProbeError {
        self.shared.log(&format!("REFUSED {reason}"));
        self.stats().refusals.push(reason.clone());
        ProbeError::Refused(reason)
    }

    /// Lets an input that can submit a prompt through only when the input box holds an allow-listed local
    /// slash command.
    fn permit_submit(&self, what: &str) -> Result<()> {
        let Guard::NoModelCalls(policy) = self.guard else {
            return Ok(());
        };
        let screen = self.screen();
        match (policy.read_input_box)(&screen) {
            Some(text) if policy.allows(&text) => {
                self.shared.log(&format!("SUBMIT-ALLOWED {what} with input {text:?}"));
                self.stats().submits_sent.push(format!("{what}: {}", text.trim()));
                Ok(())
            }
            Some(text) => Err(self.refuse(format!(
                "{what} needs an allow-listed slash command in the input box, found {:?}",
                scrub_screen(text.trim())
            ))),
            None => Err(self.refuse(format!(
                "{what} needs a readable input box holding an allow-listed slash command; none was found"
            ))),
        }
    }

    /// Whether bytes carry a Return, as CR, LF or a win32-input-mode record of VK_RETURN or a CR/LF character.
    pub fn bytes_can_submit(bytes: &[u8]) -> bool {
        if bytes.iter().any(|b| matches!(b, b'\r' | b'\n')) {
            return true;
        }
        // win32-input-mode records: CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _
        let mut rest = bytes;
        while let Some(start) = rest.windows(2).position(|w| w == b"\x1b[") {
            rest = &rest[start + 2..];
            let Some(end) = rest.iter().position(|b| (0x40..=0x7e).contains(b)) else { break };
            if rest[end] == b'_' {
                let fields: Vec<&[u8]> = rest[..end].split(|b| *b == b';').collect();
                let number = |i: usize| {
                    fields.get(i).and_then(|f| std::str::from_utf8(f).ok()).and_then(|s| s.parse::<u32>().ok())
                };
                if number(0) == Some(13) || matches!(number(2), Some(10 | 13)) {
                    return true;
                }
            }
            // `CSI 13 u` is plain Enter under the kitty "report all keys" flag.
            if rest[end] == b'u' {
                let code = rest[..end]
                    .split(|b| *b == b';' || *b == b':')
                    .next()
                    .and_then(|f| std::str::from_utf8(f).ok())
                    .and_then(|s| s.parse::<u32>().ok());
                if code == Some(13) {
                    return true;
                }
            }
            rest = &rest[end + 1..];
        }
        false
    }

    /// Types characters as the application does for text keys and the IME: raw UTF-8. Refuses control
    /// characters (Enter, Tab, ...): those are keys and go through [`Probe::send_key`].
    pub fn send_text(&self, text: &str) -> Result<()> {
        if let Some(control) = text.chars().find(|c| c.is_control()) {
            return Err(self.refuse(format!("send_text got a control character U+{:04X}", control as u32)));
        }
        self.send_bytes(encode_text(text), "text");
        self.stats().text_sent += 1;
        Ok(())
    }

    fn assemble(
        &self,
        assembler: &mut KeyAssembler,
        spec: &KeySpec,
        routing: KeyRouting,
        modes: &InputModes,
    ) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        match routing {
            KeyRouting::ConptyRoute => {
                for event in events_for(spec).map_err(ProbeError::Other)? {
                    let KeyOutput { bytes: out, .. } = assembler.handle(event, modes);
                    bytes.extend(out);
                }
            }
            KeyRouting::PaneHost => {
                // pane_view.rs: named keys and Ctrl/Alt chords reach `Pane::send_key`; text keys are left to
                // CharacterReceived and `send_text`.
                let chord = spec.mods.intersects(nekominal_input::Mods::CTRL | nekominal_input::Mods::ALT);
                let text_key = matches!(spec.key, nekominal_input::Key::Char(_)) && !chord;
                if text_key {
                    for event in events_for(spec).map_err(ProbeError::Other)? {
                        if let KeyboardEvent::CharacterReceived(unit) = event
                            && let Some(c) = char::from_u32(u32::from(unit))
                        {
                            bytes.extend(encode_text(&c.to_string()));
                        }
                    }
                } else if let Some(out) = encode_key(&KeyInput::new(spec.key).with_mods(spec.mods), modes) {
                    bytes = out;
                }
            }
        }
        Ok(bytes)
    }

    /// The bytes `send_key` would write for the current modes and routing, without sending anything.
    pub fn predict_key(&self, spec: &KeySpec) -> Result<Vec<u8>> {
        let routing = *self.routing.lock().unwrap_or_else(|p| p.into_inner());
        self.assemble(&mut KeyAssembler::new(), spec, routing, &self.input_modes())
    }

    /// Same as [`Probe::predict_key`] for a route chosen by the caller.
    pub fn predict_key_with(&self, spec: &KeySpec, routing: KeyRouting) -> Result<Vec<u8>> {
        self.assemble(&mut KeyAssembler::new(), spec, routing, &self.input_modes())
    }

    pub fn set_routing(&self, routing: KeyRouting) {
        *self.routing.lock().unwrap_or_else(|p| p.into_inner()) = routing;
    }

    /// Presses and releases a key chord through the key assembler (or the pane host's encoder, see
    /// [`KeyRouting`]). Returns the bytes written. Enter, its variants and anything whose bytes could submit a
    /// prompt are guarded.
    pub fn send_key(&self, spec: &KeySpec) -> Result<Vec<u8>> {
        let routing = *self.routing.lock().unwrap_or_else(|p| p.into_inner());
        let modes = self.input_modes();
        let predicted = self.assemble(&mut KeyAssembler::new(), spec, routing, &modes)?;
        if spec.is_enter_like() || Self::bytes_can_submit(&predicted) {
            self.permit_submit(&format!("key {:?}+{:?} ({})", spec.mods, spec.key, escape_bytes(&predicted)))?;
        }
        let bytes = {
            let mut assembler = self.assembler.lock().unwrap_or_else(|p| p.into_inner());
            self.assemble(&mut assembler, spec, routing, &modes)?
        };
        self.send_bytes(bytes.clone(), "key");
        self.stats().keys_sent += 1;
        Ok(bytes)
    }

    /// [`Probe::send_key`] from a chord string such as `"ctrl+w"`.
    pub fn key(&self, chord: &str) -> Result<Vec<u8>> {
        self.send_key(&KeySpec::parse(chord).map_err(ProbeError::Other)?)
    }

    /// Presses Enter. Guarded: refused unless the input box holds an allow-listed local slash command.
    pub fn send_enter(&self) -> Result<Vec<u8>> {
        self.key("enter")
    }

    /// Mouse event through `encode_mouse` with the program's modes. `Ok(None)` when the program has not asked
    /// for mouse reports (nothing is sent).
    pub fn send_mouse(&self, input: &MouseInput) -> Result<Option<Vec<u8>>> {
        let modes = self.input_modes();
        let bytes = {
            let mut state = self.mouse.lock().unwrap_or_else(|p| p.into_inner());
            encode_mouse(input, &modes, &mut state)
        };
        if let Some(bytes) = &bytes {
            self.send_bytes(bytes.clone(), "mouse");
        }
        Ok(bytes)
    }

    /// Paste, bracketed when the program enabled DECSET 2004. A multi-line paste to a program that did not is
    /// refused: its line breaks would be Enter presses.
    pub fn send_paste(&self, text: &str) -> Result<Vec<u8>> {
        let bracketed = self.input_modes().bracketed_paste;
        if !bracketed && text.contains(['\r', '\n']) {
            if let Guard::NoModelCalls(_) = self.guard {
                return Err(self.refuse("a multi-line paste without bracketed paste mode would press Enter".into()));
            }
        }
        let bytes = prepare_paste(text, bracketed);
        self.send_bytes(bytes.clone(), "paste");
        self.stats().pastes_sent += 1;
        Ok(bytes)
    }

    /// Focus change, as the pane host reports it (`CSI I` / `CSI O` while the program asked for 1004).
    pub fn send_focus(&self, focused: bool) -> Option<Vec<u8>> {
        let modes = {
            let mut core = self.shared.term.lock();
            core.set_focused(focused);
            read_modes(&core).1
        };
        let report = encode_focus(focused, &modes)?.to_vec();
        self.send_bytes(report.clone(), "focus");
        Some(report)
    }

    /// Writes bytes without any check. Only for probes whose guard is `Off` (self-tests with scripts).
    pub fn send_raw(&self, bytes: &[u8]) -> Result<()> {
        if !matches!(self.guard, Guard::Off) {
            return Err(self.refuse("send_raw is only available without the guard".into()));
        }
        self.send_bytes(bytes.to_vec(), "raw");
        Ok(())
    }

    /// Resizes the emulator and the PTY together under the terminal lock, as `Pane::resize` does.
    pub fn resize(&self, cols: u16, rows: u16) {
        let mut core = self.shared.term.lock();
        core.resize(cols as usize, rows as usize);
        let _ = self.session.resize(cols, rows);
        drop(core);
        self.shared.log(&format!("RESIZE {cols}x{rows}"));
    }

    // ---- teardown -----------------------------------------------------------------------------

    /// Kills the process tree and stops the helper threads. Safe to call twice.
    pub fn shutdown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stop_ticker.store(true, Ordering::Relaxed);
        self.sessions.close(self.id);
        self.shared.log("SHUTDOWN");
        let mut state = self.shared.state();
        if let Some(tap) = state.tap.as_mut() {
            let _ = tap.flush();
        }
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_in_any_encoding_can_submit() {
        for bytes in [
            &b"\r"[..],
            b"\n",
            b"text\rmore",
            // win32-input-mode records: VK_RETURN, and a CR / LF character with another virtual key
            b"\x1b[13;28;13;1;0;1_",
            b"\x1b[0;0;13;1;0;1_",
            b"\x1b[74;36;10;1;8;1_",
            // kitty "report all keys": plain Enter is CSI 13 u, and so are its modified forms
            b"\x1b[13u",
            b"\x1b[13;2u",
        ] {
            assert!(Probe::bytes_can_submit(bytes), "{}", escape_bytes(bytes));
        }
    }

    #[test]
    fn text_navigation_and_chords_cannot_submit() {
        for bytes in [
            &b"hello"[..],
            "你好ㄅ😀".as_bytes(),
            b"\x1b[A",
            b"\x1b[5~",
            b"\x1b[117;5u",
            b"\x1b[27u",
            b"\x1b[65;30;97;1;0;1_",
            b"\x7f",
            b"\x03",
        ] {
            assert!(!Probe::bytes_can_submit(bytes), "{}", escape_bytes(bytes));
        }
    }

    #[test]
    fn the_guard_cannot_be_turned_off_for_an_agent_cli() {
        for name in ["claude.exe", "C:\\tools\\Claude.exe", "claude-code.exe"] {
            let result = Probe::spawn(SpawnOptions::new(name, "guard-off"));
            assert!(matches!(result, Err(ProbeError::Refused(_))), "{name}");
        }
    }

    #[test]
    fn only_exact_allow_listed_commands_pass_the_policy() {
        fn no_box(_: &Screen) -> Option<String> {
            None
        }
        let policy = EnterPolicy { allowed_commands: &["/help"], read_input_box: no_box };
        assert!(policy.allows("/help"));
        assert!(policy.allows("  /help \n"));
        for refused in ["/help me", "help", "/helpme", "", "/help\nsend this", "!dir"] {
            assert!(!policy.allows(refused), "{refused:?}");
        }
    }

    #[test]
    fn escape_bytes_is_readable_and_never_raw() {
        assert_eq!(escape_bytes(b"\x1b[13;2u\r\n\t\x03"), "ESC[13;2u\\r\\n\\t\\x03");
        assert_eq!(escape_bytes("你好".as_bytes()), "你好");
    }

    #[test]
    fn scenario_names_become_safe_file_names() {
        assert_eq!(file_name_for("fullscreen: resize/storm #1"), "fullscreen__resize_storm__1");
    }
}
