//! ConPTY input experiment rig: spawn a probe child inside a pseudoconsole (through
//! `nekominal-pty`), write candidate byte sequences to the session input, and read back what the
//! child received.
//!
//! Shared by `tests/conpty_input.rs` (bundled ConPTY: the test binary sits next to the staged
//! conpty.dll + OpenConsole.exe) and `examples/conpty_input_system.rs` (the example binary does
//! not, so `nekominal-pty` falls back to the in-box kernel32 ConPTY).

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{mpsc::channel, mpsc::Receiver, Arc},
    time::{Duration, Instant},
};

use nekominal_pty::{
    conpty::SpawnConfig,
    session::{Event, Session, Sessions, Sink},
};

pub const NODE: &str = "C:/Users/doeshing/scoop/apps/nodejs-lts/current/node.exe";

/// Printable rendering of a byte stream: ESC as `\e`, CR `\r`, LF `\n`, TAB `\t`, other controls
/// as `\xNN`, bytes that are not valid UTF-8 as `<NN>`.
pub fn show(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                push_text(&mut out, text);
                break;
            }
            Err(error) => {
                let (valid, after) = rest.split_at(error.valid_up_to());
                push_text(&mut out, std::str::from_utf8(valid).unwrap());
                let bad = error.error_len().unwrap_or(after.len());
                for byte in &after[..bad] {
                    out.push_str(&format!("<{byte:02x}>"));
                }
                rest = &after[bad..];
            }
        }
    }
    out
}

fn push_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '\x1b' => out.push_str("\\e"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\x7f' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2).map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Child {
    /// `node probe.js`: libuv raw-mode stdin.
    Node,
    /// `winprobe win`: ReadConsoleInputW, no VT input (cmd.exe / PSReadLine style).
    Win32,
    /// `winprobe vt`: ENABLE_VIRTUAL_TERMINAL_INPUT.
    VtInput,
    /// Like `Win32` / `VtInput` but logging only the delivered character stream (large inputs).
    Win32Quiet,
    VtInputQuiet,
}

impl Child {
    pub fn name(self) -> &'static str {
        match self {
            Child::Node => "node",
            Child::Win32 => "win32",
            Child::VtInput => "vt-input",
            Child::Win32Quiet => "win32-quiet",
            Child::VtInputQuiet => "vt-input-quiet",
        }
    }
}

fn target_debug_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    for ancestor in exe.ancestors() {
        if ancestor.join("winprobe.exe").is_file() {
            return ancestor.to_path_buf();
        }
    }
    panic!("winprobe.exe not found next to {exe:?}; build probes/winprobe first");
}

pub fn probe_command(child: Child, log: &Path, setup: &[u8]) -> (PathBuf, Vec<String>) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let log = log.to_string_lossy().into_owned();
    let setup_hex = hex(setup);
    match child {
        Child::Node => {
            let script = manifest.join("probes").join("node").join("probe.js");
            let mut args = vec![script.to_string_lossy().into_owned(), log];
            if !setup.is_empty() {
                args.push(setup_hex);
            }
            (PathBuf::from(NODE), args)
        }
        Child::Win32 | Child::VtInput | Child::Win32Quiet | Child::VtInputQuiet => {
            let mode = match child {
                Child::Win32 => "win",
                Child::VtInput => "vt",
                Child::Win32Quiet => "win-quiet",
                _ => "vt-quiet",
            };
            let mut args = vec![log, mode.to_string()];
            if !setup.is_empty() {
                args.push(setup_hex);
            }
            (target_debug_dir().join("winprobe.exe"), args)
        }
    }
}

pub struct Rig {
    sessions: Sessions,
    id: u32,
    pub session: Arc<Session>,
    rx: Receiver<Event>,
    /// Everything the child (and ConPTY) wrote to the terminal.
    pub raw: Vec<u8>,
    pub log: PathBuf,
    pub exit: Option<u32>,
    pub backend: String,
}

fn scratch_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("nekominal-input-spike");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

impl Rig {
    pub fn spawn(child: Child, setup: &[u8], tag: &str) -> Rig {
        let log = scratch_file(&format!("{tag}-{}.log", child.name()));
        let (program, args) = probe_command(child, &log, setup);
        let config = SpawnConfig {
            program,
            args,
            cwd: None,
            env: BTreeMap::new(),
            cols: 120,
            rows: 30,
            width_model: nekominal_pty::conpty::WidthModel::Wcswidth,
            inherit_environment: true,
        };
        let (tx, rx) = channel();
        let sink: Sink = Box::new(move |event| tx.send(event).is_ok());
        let sessions = Sessions::default();
        let (id, _pid) = sessions.spawn(&config, sink).expect("spawn ConPTY session");
        let session = sessions.get(id).unwrap();
        let backend = nekominal_pty::conpty::backend()
            .map(|b| format!("{} {}", b.kind, b.version))
            .unwrap_or_else(|e| format!("unavailable ({e})"));
        Rig { sessions, id, session, rx, raw: Vec::new(), log, exit: None, backend }
    }

    /// Reads output for `duration`, answering the queries ConPTY and children send.
    pub fn pump(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left.min(Duration::from_millis(10))) {
                Ok(Event::Data(bytes)) => {
                    answer_queries(&self.session, &bytes);
                    self.session.ack(bytes.len());
                    self.raw.extend_from_slice(&bytes);
                }
                Ok(Event::Exit(code)) => self.exit = Some(code),
                Err(_) => {}
            }
        }
    }

    pub fn wait_for(&mut self, needle: &[u8], timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.raw.windows(needle.len()).any(|w| w == needle) {
                return true;
            }
            self.pump(Duration::from_millis(20));
        }
        self.raw.windows(needle.len()).any(|w| w == needle)
    }

    pub fn log_lines(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Writes `bytes` to the session input and returns the log lines the child produced in
    /// response: waits until the log stops growing for `quiet`, or `settle` has passed.
    pub fn send(&mut self, bytes: &[u8], settle: Duration, quiet: Duration) -> Vec<String> {
        let before = self.log_lines().len();
        self.session.write(bytes).expect("write to session input");
        let started = Instant::now();
        let mut length = before;
        let mut last_growth: Option<Instant> = None;
        loop {
            self.pump(Duration::from_millis(10));
            let now_length = self.log_lines().len();
            if now_length != length {
                length = now_length;
                last_growth = Some(Instant::now());
            }
            if let Some(at) = last_growth
                && at.elapsed() >= quiet
            {
                break;
            }
            if started.elapsed() >= settle {
                break;
            }
        }
        self.log_lines().split_off(before)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.sessions.close(self.id);
    }
}

/// DA1 and cursor-position queries from ConPTY or the child.
fn answer_queries(session: &Session, frame: &[u8]) {
    let has = |needle: &[u8]| frame.windows(needle.len()).any(|w| w == needle);
    if has(b"\x1b[c") || has(b"\x1b[0c") {
        let _ = session.write(b"\x1b[?1;2c");
    }
    if has(b"\x1b[6n") {
        let _ = session.write(b"\x1b[1;1R");
    }
}

/// One thing to send and what came back.
#[derive(Clone, Debug)]
pub struct StepResult {
    pub label: String,
    pub sent: Vec<u8>,
    pub lines: Vec<String>,
}

impl StepResult {
    /// The byte chunks a Node child received.
    pub fn node_chunks(&self) -> Vec<Vec<u8>> {
        self.lines
            .iter()
            .filter_map(|l| l.strip_prefix("DATA "))
            .map(|rest| unhex(rest.split(' ').nth(1).unwrap_or("")))
            .collect()
    }

    /// The character stream a winprobe child's key-down records delivered (`S` lines).
    pub fn stream(&self) -> Vec<Vec<u8>> {
        self.lines.iter().filter_map(|l| l.strip_prefix("S ")).map(unhex).collect()
    }

    /// Key records `(down, vk, scan, unicode, control_state)` of a winprobe child.
    pub fn key_records(&self) -> Vec<(bool, u16, u16, u16, u32)> {
        self.lines
            .iter()
            .filter_map(|l| l.strip_prefix("K "))
            .map(|rest| {
                let field = |name: &str| -> u32 {
                    let start = rest.find(name).unwrap() + name.len();
                    let value: String = rest[start..].chars().take_while(|c| !c.is_whitespace()).collect();
                    match value.strip_prefix("0x") {
                        Some(h) => u32::from_str_radix(h, 16).unwrap(),
                        None => value.parse::<u32>().unwrap_or(u32::from(value == "true")),
                    }
                };
                (field("down=") != 0, field("vk=") as u16, field("sc=") as u16, field("uc=") as u16, field("cs="))
            })
            .collect()
    }

    pub fn mouse_records(&self) -> Vec<String> {
        self.lines.iter().filter(|l| l.starts_with("M ")).cloned().collect()
    }

    /// Compact rendering for the report.
    pub fn render(&self) -> String {
        if self.lines.is_empty() {
            return "(nothing)".to_string();
        }
        let mut parts = Vec::new();
        for line in &self.lines {
            if let Some(rest) = line.strip_prefix("DATA ") {
                parts.push(format!("\"{}\"", show(&unhex(rest.split(' ').nth(1).unwrap_or("")))));
            } else if let Some(rest) = line.strip_prefix("S ") {
                parts.push(format!("S\"{}\"", show(&unhex(rest))));
            } else if line.starts_with("K ") {
                let tmp = StepResult { label: String::new(), sent: Vec::new(), lines: vec![line.clone()] };
                let (down, vk, sc, uc, cs) = tmp.key_records()[0];
                parts.push(format!(
                    "{}vk{:02x}/sc{:02x}/u{:04x}/cs{:03x}",
                    if down { "v" } else { "^" },
                    vk,
                    sc,
                    uc,
                    cs
                ));
            } else {
                parts.push(line.clone());
            }
        }
        parts.join("  ")
    }
}

pub struct ScenarioResult {
    pub child: Child,
    pub setup: Vec<u8>,
    pub backend: String,
    pub ready: bool,
    /// The child's own mode request reached the terminal side of the pipe (ConPTY forwarded it).
    pub setup_forwarded: bool,
    pub info: Vec<String>,
    /// What the child and ConPTY wrote to the terminal before the first step.
    pub startup_output: Vec<u8>,
    pub steps: Vec<StepResult>,
}

impl ScenarioResult {
    pub fn step(&self, label: &str) -> &StepResult {
        self.steps.iter().find(|s| s.label == label).unwrap_or_else(|| panic!("no step {label}"))
    }

    pub fn report(&self, name: &str) -> String {
        let mut out = format!(
            "=== {name} | child={} setup=\"{}\" backend={} ready={} setup_forwarded={}\n",
            self.child.name(),
            show(&self.setup),
            self.backend,
            self.ready,
            self.setup_forwarded
        );
        for line in &self.info {
            out.push_str(&format!("    {line}\n"));
        }
        out.push_str(&format!("    startup output: \"{}\"\n", show(&self.startup_output)));
        for step in &self.steps {
            out.push_str(&format!("  {:<28} sent \"{}\"  ->  {}\n", step.label, show(&step.sent), step.render()));
        }
        out
    }
}

pub struct Step {
    pub label: String,
    pub bytes: Vec<u8>,
}

pub fn step(label: &str, bytes: impl Into<Vec<u8>>) -> Step {
    Step { label: label.to_string(), bytes: bytes.into() }
}

pub fn run_scenario(child: Child, setup: &[u8], tag: &str, steps: &[Step]) -> ScenarioResult {
    let mut rig = Rig::spawn(child, setup, tag);
    let ready = rig.wait_for(b"READY", Duration::from_secs(20));
    rig.pump(Duration::from_millis(150));
    let setup_forwarded = setup.is_empty() || rig.raw.windows(setup.len()).any(|w| w == setup);
    let startup_output = rig.raw.clone();
    let mut results = Vec::new();
    if ready {
        for s in steps {
            let lines = rig.send(&s.bytes, Duration::from_millis(450), Duration::from_millis(60));
            // Keep INFO lines out of the per-step results.
            let lines = lines.into_iter().filter(|l| !l.starts_with("INFO ")).collect();
            results.push(StepResult { label: s.label.clone(), sent: s.bytes.clone(), lines });
        }
    }
    let info = rig.log_lines().into_iter().filter(|l| l.starts_with("INFO ")).collect();
    ScenarioResult {
        child,
        setup: setup.to_vec(),
        backend: rig.backend.clone(),
        ready,
        setup_forwarded,
        info,
        startup_output,
        steps: results,
    }
}
