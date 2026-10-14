//! Pane host: one ConPTY session, one `TerminalCore` and the single writer thread.
//!
//! Seed of the future `nekominal-pane` crate (from emulator-src/tests/common/pane.rs). Rules it encodes:
//! * the PTY forwarder thread calls the sink; the sink feeds the core under a `FairMutex`;
//! * replies, keyboard bytes and paste all go through one channel to one writer thread, because
//!   `Session::write` is a blocking `write_all` and must never run on the forwarder;
//! * the ack happens after `feed`, which is the back-pressure.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU8, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread,
};

use nekominal_input::{InputModes, KeyInput, encode_focus, encode_key, encode_text, prepare_paste};
use nekominal_pty::{
    conpty::{SpawnConfig, WidthModel},
    session::{Event, Session, Sessions, Sink},
};
use nekominal_term::{
    CoreConfig, CoreEvent, TerminalCore,
    agent_status::{AgentRecord, AgentState, Report},
    alacritty_terminal::{grid::Scroll, sync::FairMutex},
    osc::{Notification, Progress, PromptMark},
};

#[derive(Debug)]
pub enum UiEvent {
    Title(String),
    /// The shell reported its working directory (OSC 7).
    Cwd(String),
    Bell,
    Exit(u32),
    /// An OSC 7501 report changed the agent records of the pane. The state to show is the live one,
    /// [`Pane::agent_summary`], not what this report says.
    Agent(Report),
    /// The agent records changed for another reason: the next prompt, a terminal reset, the program ended.
    AgentRecordsDropped,
    /// OSC 9 or OSC 777: a one-shot message. Control and bidirectional characters are already removed.
    Notification(Notification),
    /// OSC 9;4 progress; `None` clears it.
    Progress(Option<Progress>),
    /// OSC 133: the shell marked a prompt, a command or its end.
    PromptMark(PromptMark),
    /// Anything else the emulator reported, as debug text.
    Other(String),
}

/// Whether a command runs in the pane, as far as the prompt marks of the shell (OSC 133) and the input sent to it
/// tell. The integration of PowerShell marks the prompt and the end of a command but not its start, so a command
/// starts when a line is submitted at the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// No mark has come yet: a shell without integration, or one that has not shown its first prompt.
    Unknown,
    /// The shell waits at its prompt.
    AtPrompt,
    /// A line was submitted and no prompt has come back since.
    Running,
}

impl Activity {
    fn code(self) -> u8 {
        self as u8
    }

    fn from_code(code: u8) -> Activity {
        match code {
            1 => Activity::AtPrompt,
            2 => Activity::Running,
            _ => Activity::Unknown,
        }
    }

    /// A prompt mark came: the shell is at its prompt, or runs the command whose output has just begun.
    pub fn after_mark(mark: PromptMark) -> Activity {
        match mark {
            PromptMark::PromptStart | PromptMark::CommandStart | PromptMark::CommandEnd { .. } => Activity::AtPrompt,
            PromptMark::OutputStart => Activity::Running,
        }
    }

    /// A line went to the program: a shell at its prompt starts a command. A program that gave no marks stays
    /// unknown, and a running one keeps running.
    pub fn after_submit(self) -> Activity {
        match self {
            Activity::AtPrompt => Activity::Running,
            other => other,
        }
    }
}

/// The event the shell gets for what the emulator reported. `PtyWrite` is not one: it goes to the program.
fn ui_event(event: CoreEvent) -> UiEvent {
    match event {
        CoreEvent::Title(title) => UiEvent::Title(title),
        CoreEvent::Cwd(path) => UiEvent::Cwd(path),
        CoreEvent::Bell => UiEvent::Bell,
        CoreEvent::Agent(report) => UiEvent::Agent(report),
        CoreEvent::AgentRecordsDropped => UiEvent::AgentRecordsDropped,
        CoreEvent::Notification(notification) => UiEvent::Notification(notification),
        CoreEvent::Progress(progress) => UiEvent::Progress(progress),
        CoreEvent::PromptMark { mark, .. } => UiEvent::PromptMark(mark),
        other => UiEvent::Other(format!("{other:?}")),
    }
}

pub struct Pane {
    pub term: Arc<FairMutex<TerminalCore>>,
    pub events: Receiver<UiEvent>,
    input: Sender<Vec<u8>>,
    sessions: Sessions,
    id: u32,
    /// An [`Activity`] as its code: the reader thread writes it, the UI thread reads it.
    activity: Arc<AtomicU8>,
}

impl Pane {
    pub fn spawn(program: PathBuf, args: Vec<String>, cols: u16, rows: u16) -> std::io::Result<Pane> {
        let mut env = BTreeMap::new();
        env.insert("TERM_PROGRAM".to_owned(), "Nekominal".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        // Wcswidth (0x10) is the mode that agrees with alacritty_terminal's per-code-point width model
        // (emulator blueprint, section 11.5).
        Pane::spawn_with(SpawnConfig { program, args, cwd: None, env, cols, rows, width_model: WidthModel::Wcswidth })
    }

    /// Starts the session described by `config` (a profile's program, arguments, directory and environment).
    pub fn spawn_with(config: SpawnConfig) -> std::io::Result<Pane> {
        let (cols, rows) = (config.cols, config.rows);
        let term = Arc::new(FairMutex::new(TerminalCore::new(cols as usize, rows as usize, CoreConfig::default())));
        let (input_tx, input_rx) = channel::<Vec<u8>>();
        let (ui_tx, ui_rx) = channel::<UiEvent>();
        let session_cell: Arc<OnceLock<Arc<Session>>> = Arc::new(OnceLock::new());
        let pending_ack = Arc::new(AtomicUsize::new(0));
        let activity = Arc::new(AtomicU8::new(Activity::Unknown.code()));

        let sink: Sink = {
            let (term, input_tx, session_cell, pending_ack, activity) =
                (term.clone(), input_tx.clone(), session_cell.clone(), pending_ack.clone(), activity.clone());
            Box::new(move |event| match event {
                Event::Data(bytes) => {
                    let events = {
                        let mut core = term.lock();
                        core.feed(&bytes);
                        core.take_events()
                    };
                    for event in events {
                        match event {
                            CoreEvent::PtyWrite(reply) => {
                                if input_tx.send(reply).is_err() {
                                    return false;
                                }
                            }
                            other => {
                                if let CoreEvent::PromptMark { mark, .. } = &other {
                                    activity.store(Activity::after_mark(*mark).code(), Ordering::Relaxed);
                                }
                                let _ = ui_tx.send(ui_event(other));
                            }
                        }
                    }
                    // Ack after the emulator consumed the frame: this is the back-pressure.
                    match session_cell.get() {
                        Some(session) => session.ack(bytes.len() + pending_ack.swap(0, Ordering::Relaxed)),
                        None => {
                            pending_ack.fetch_add(bytes.len(), Ordering::Relaxed);
                        }
                    }
                    true
                }
                Event::Exit(code) => {
                    term.lock().on_child_exit();
                    let _ = ui_tx.send(UiEvent::Exit(code));
                    true
                }
            })
        };

        let sessions = Sessions::default();
        let (id, _pid) = sessions.spawn(&config, sink)?;
        let session = sessions.get(id)?;
        let _ = session_cell.set(session.clone());
        let early = pending_ack.swap(0, Ordering::Relaxed);
        if early > 0 {
            session.ack(early);
        }

        thread::Builder::new().name("pty-writer".into()).spawn(move || {
            for bytes in input_rx {
                if session.write(&bytes).is_err() {
                    break;
                }
            }
        })?;
        Ok(Pane { term, events: ui_rx, input: input_tx, sessions, id, activity })
    }

    /// Keyboard, paste and IME bytes from the UI thread.
    pub fn send(&self, bytes: Vec<u8>) {
        if bytes.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            let _ = self.activity.try_update(Ordering::Relaxed, Ordering::Relaxed, |code| {
                Some(Activity::from_code(code).after_submit().code())
            });
        }
        let _ = self.input.send(bytes);
    }

    pub fn activity(&self) -> Activity {
        Activity::from_code(self.activity.load(Ordering::Relaxed))
    }

    /// Closing the pane would end work: a command runs, an agent works or waits for an answer, or nothing tells
    /// (a shell that gives no prompt marks counts as busy).
    pub fn is_busy(&self) -> bool {
        self.activity() != Activity::AtPrompt
            || self
                .agent_summary()
                .is_some_and(|record| matches!(record.state, AgentState::Working | AgentState::Blocked))
    }

    /// The most urgent agent record of the pane right now (OSC 7501), if any. Live: it follows the records the
    /// emulator keeps, so there is nothing to latch on the UI side.
    pub fn agent_summary(&self) -> Option<AgentRecord> {
        self.term.lock().agent_summary().cloned()
    }

    /// The user has seen what a finished or failed agent reported: those records go away.
    pub fn dismiss_finished(&self) -> bool {
        self.term.lock().agent_mut().dismiss_finished()
    }

    /// Input modes as the program set them, read under the terminal lock for every event.
    pub fn input_modes(&self) -> InputModes {
        InputModes::from_term_mode(self.term.lock().mode())
    }

    /// Encodes a key event for the program's current modes and queues it. False when nothing is sent.
    pub fn send_key(&self, input: &KeyInput) -> bool {
        match encode_key(input, &self.input_modes()) {
            Some(bytes) => {
                self.send(bytes);
                true
            }
            None => false,
        }
    }

    /// IME commits and other composed text (raw UTF-8 on the bundled ConPTY).
    pub fn send_text(&self, text: &str) {
        self.send(encode_text(text));
    }

    pub fn paste(&self, text: &str) {
        let bracketed = self.input_modes().bracketed_paste;
        self.send(prepare_paste(text, bracketed));
    }

    /// Closes the session (kills the process tree) while the process can still create threads. Drop only does it as a fallback:
    /// at process exit the main thread's TLS destructors run after thread creation is already denied, and
    /// Sessions::close spawns a thread (found by the vertical slice: panic ailed to spawn thread: Access is denied).
    pub fn shutdown(&self) {
        self.sessions.close(self.id);
    }

    /// A handle other threads can use to feed the writer thread (probe scripts).
    pub fn sender(&self) -> Sender<Vec<u8>> {
        self.input.clone()
    }

    /// Term and PTY are resized together under the terminal lock so a snapshot never sees half a resize.
    pub fn resize(&self, cols: u16, rows: u16) {
        let mut core = self.term.lock();
        core.resize(cols as usize, rows as usize);
        if let Ok(session) = self.sessions.get(self.id) {
            let _ = session.resize(cols, rows);
        }
    }

    /// Scrolls the viewport through the scrollback (positive: toward older lines). Clamped to the history, so a
    /// program on the alternate screen, which has none, does not move.
    pub fn scroll_lines(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }
    /// Keyboard focus moved into or out of the pane: the cursor shape follows it and a program that asked
    /// for focus reports (DECSET 1004) is told.
    pub fn set_focused(&self, focused: bool) {
        let modes = {
            let mut core = self.term.lock();
            core.set_focused(focused);
            InputModes::from_term_mode(core.mode())
        };
        if let Some(report) = encode_focus(focused, &modes) {
            self.send(report.to_vec());
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.sessions.close(self.id);
    }
}

#[cfg(test)]
mod tests {
    use nekominal_term::{agent_status::AgentState, osc::ProgressState};

    use super::*;

    /// What the shell is told when `bytes` arrive from the program.
    fn events_for(bytes: &[u8]) -> Vec<UiEvent> {
        let mut core = TerminalCore::new(80, 24, CoreConfig::default());
        core.feed(bytes);
        core.take_events().into_iter().filter(|e| !matches!(e, CoreEvent::PtyWrite(_))).map(ui_event).collect()
    }

    #[test]
    fn a_shell_is_busy_from_a_submitted_line_until_the_next_prompt() {
        use Activity::*;
        assert_eq!(Unknown.after_submit(), Unknown, "no marks, no claim");
        assert_eq!(AtPrompt.after_submit(), Running);
        assert_eq!(Running.after_submit(), Running);
        for mark in [PromptMark::PromptStart, PromptMark::CommandStart, PromptMark::CommandEnd { exit_code: Some(0) }] {
            assert_eq!(Activity::after_mark(mark), AtPrompt, "{mark:?}");
        }
        assert_eq!(Activity::after_mark(PromptMark::OutputStart), Running);
        // A command with the prompt marks of PowerShell: end of the last one, prompt, input; Enter; end, prompt again.
        let mut state = Unknown;
        for step in [
            Some(PromptMark::CommandEnd { exit_code: Some(0) }),
            Some(PromptMark::PromptStart),
            Some(PromptMark::CommandStart),
            None,
        ] {
            state = match step {
                Some(mark) => Activity::after_mark(mark),
                None => state.after_submit(),
            };
        }
        assert_eq!(state, Running);
        assert_eq!(Activity::after_mark(PromptMark::CommandEnd { exit_code: Some(0) }), AtPrompt);
    }

    #[test]
    fn an_agent_report_reaches_the_shell_as_a_typed_event() {
        let events = events_for(b"\x1b]7501;state=blocked:kind=permission:id=a:app=claude\x07");
        let [UiEvent::Agent(Report::Set(record))] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(
            (record.state, record.id.as_str(), record.app.as_deref()),
            (AgentState::Blocked, "a", Some("claude"))
        );
    }

    #[test]
    fn working_ends_at_the_next_prompt_and_the_shell_hears_both() {
        let mut core = TerminalCore::new(80, 24, CoreConfig::default());
        core.feed(b"\x1b]7501;state=working:id=a\x07");
        assert_eq!(core.agent_summary().map(|r| r.state), Some(AgentState::Working));
        core.take_events();
        core.feed(b"\x1b]133;A\x07");
        let events: Vec<UiEvent> = core.take_events().into_iter().map(ui_event).collect();
        assert!(
            matches!(events.as_slice(), [UiEvent::AgentRecordsDropped, UiEvent::PromptMark(PromptMark::PromptStart)]),
            "{events:?}"
        );
        assert!(core.agent_summary().is_none());
    }

    #[test]
    fn notifications_and_progress_are_typed_and_cleaned() {
        let events = events_for(b"\x1b]777;notify;Claude;needs\xE2\x80\xAE input\x07\x1b]9;Build finished\x07\x1b]9;4;1;40\x07\x1b]9;4;0\x07");
        let [
            UiEvent::Notification(first),
            UiEvent::Notification(second),
            UiEvent::Progress(Some(Progress::Value { state: ProgressState::Normal, value })),
            UiEvent::Progress(None),
        ] = events.as_slice()
        else {
            panic!("{events:?}")
        };
        assert_eq!((first.title.as_str(), first.body.as_str()), ("Claude", "needs input"), "the bidi override is gone");
        assert_eq!((second.title.as_str(), second.body.as_str()), ("", "Build finished"));
        assert_eq!(*value, 40.0);
    }

    #[test]
    fn title_directory_and_bell_keep_their_events() {
        let events = events_for(b"\x1b]0;build\x07\x1b]7;file://pc/C:/work\x07\x07");
        assert!(events.iter().any(|e| matches!(e, UiEvent::Title(title) if title == "build")), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, UiEvent::Cwd(path) if path == "C:\\work")), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, UiEvent::Bell)), "{events:?}");
        assert!(events.iter().all(|e| !matches!(e, UiEvent::Other(_))), "{events:?}");
    }
}
