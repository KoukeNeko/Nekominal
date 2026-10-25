//! Pane host: one ConPTY session, one `TerminalCore` and the single writer thread.
//!
//! Seed of the future `nekominal-pane` crate (from emulator-src/tests/common/pane.rs). Rules it encodes:
//! * the PTY forwarder thread calls the sink; the sink feeds the core under a `FairMutex`;
//! * replies, keyboard bytes and paste all go through one channel to one writer thread, because
//!   `Session::write` is a blocking `write_all` and must never run on the forwarder;
//! * the ack happens after `feed`, which is the back-pressure.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread,
};

use nekominal_input::{
    InputModes, KeyEvent, KeyInput, MouseEncoding, MouseMode, PointerEvent, PointerGrid, PointerKind, encode_focus,
    encode_key, encode_text, prepare_paste,
};
use nekominal_pty::{
    conpty::{SpawnConfig, WidthModel},
    session::{Event, Session, Sessions, Sink},
};
use crate::pointer::{Pointer, PointerEffects};
use nekominal_term::{
    ClipboardFormatter, ClipboardKind, CoreConfig, CoreEvent, Link, TerminalCore,
    agent_status::{AgentRecord, AgentState, Report},
    alacritty_terminal::{grid::Scroll, sync::FairMutex, term::Osc52, vte::ansi::CursorStyle},
    osc::{Notification, Progress, PromptMark},
};

/// The most text a program may put on the clipboard (OSC 52), and the most the clipboard may hand to one: 1 MiB.
/// That is a screenful of output ten thousand lines long, far more than anyone copies out of a terminal, and
/// little enough that a program cannot make every clipboard listener hold tens of megabytes. The emulator cuts
/// an OSC payload at 4 MiB (`guard::MAX_OSC_PAYLOAD`) and delivers what is left, about 3 MiB of text, so a copy
/// that was cut is always over this limit and is refused instead of stored half.
pub const MAX_CLIPBOARD_TEXT: usize = 1024 * 1024;

/// An OSC 52 read: the program asked for the clipboard and is waiting for the answer.
pub struct ClipboardRequest {
    reply: ClipboardFormatter,
}

impl fmt::Debug for ClipboardRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClipboardRequest")
    }
}

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
    /// OSC 52: the program copies this text to the clipboard (the emulator already applied the policy).
    ClipboardStore(String),
    /// OSC 52: the program reads the clipboard; answer with [`Pane::answer_clipboard`].
    ClipboardLoad(ClipboardRequest),
    /// An OSC 52 write was dropped for its size (over [`MAX_CLIPBOARD_TEXT`]). The text is not kept.
    ClipboardRefused { bytes: usize },
    /// Anything else the emulator reported, as debug text.
    Other(String),
}

/// Whether a command runs in the pane, as far as the prompt marks of the shell (OSC 133) and the input sent to it
/// tell. The integration of PowerShell marks the prompt and the end of a command but not its start, so a command
/// starts when a line is submitted at the prompt.
///
/// Known limit: a line typed while a command runs (type-ahead) is not counted. The shell runs it as soon as it has
/// shown its next prompt, and no mark says so, so the pane reads `AtPrompt` while that command runs and closing it
/// asks nothing. Counting the submitted lines would not fix it: a line a program reads itself (a `[Y/n]` question) or
/// a continuation line is never matched by a prompt, which would leave the pane busy until the next empty Enter.
/// Only a mark at the start of every command (OSC 133 `C`) can tell, and the PowerShell integration cannot send it
/// from `prompt`, which runs before the line is read.
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
        // Windows has one clipboard and no primary selection: a program that writes to or reads the selection
        // would otherwise overwrite or read what the user copied.
        CoreEvent::ClipboardStore { clipboard: ClipboardKind::Clipboard, text } if text.len() <= MAX_CLIPBOARD_TEXT => {
            UiEvent::ClipboardStore(text)
        }
        CoreEvent::ClipboardStore { clipboard: ClipboardKind::Clipboard, text } => {
            UiEvent::ClipboardRefused { bytes: text.len() }
        }
        CoreEvent::ClipboardLoad { clipboard: ClipboardKind::Clipboard, reply } => {
            UiEvent::ClipboardLoad(ClipboardRequest { reply })
        }
        CoreEvent::ClipboardStore { .. } | CoreEvent::ClipboardLoad { .. } => {
            UiEvent::Other("clipboard: the primary selection is not supported".to_owned())
        }
        other => UiEvent::Other(format!("{other:?}")),
    }
}

/// The input modes the program set, as the key and mouse encoders want them. `alacritty_terminal` tracks neither
/// DECSET 9 (X10 reports), 1015 (urxvt positions) nor 1016 (pixel positions): the emulator's sniffer does, and
/// those win over what `TermMode` says, as `InputModes::from_term_mode` asks of its caller.
pub fn input_modes_of(core: &TerminalCore) -> InputModes {
    let mut modes = InputModes::from_term_mode(core.mode());
    let extensions = core.mouse_extensions();
    if extensions.x10 {
        modes.mouse_mode = MouseMode::X10;
    }
    if extensions.sgr_pixels {
        modes.mouse_encoding = MouseEncoding::SgrPixels;
    } else if extensions.urxvt {
        modes.mouse_encoding = MouseEncoding::Urxvt;
    }
    modes
}

/// One running session of a pane: the channel to its writer thread and the session itself. A pane gets a new one
/// when it restarts.
struct Live {
    input: Sender<Vec<u8>>,
    sessions: Sessions,
    id: u32,
}

pub struct Pane {
    pub term: Arc<FairMutex<TerminalCore>>,
    pub events: Receiver<UiEvent>,
    live: RefCell<Live>,
    ui: Sender<UiEvent>,
    /// Which session the pane listens to: a session that was replaced by a restart still ends, and what it says
    /// then is not the new session's.
    generation: Arc<AtomicU32>,
    /// An [`Activity`] as its code: the reader thread writes it, the UI thread reads it.
    activity: Arc<AtomicU8>,
    wake: Wake,
    pointer: RefCell<Pointer>,
}

/// Called from the reader thread when the pane has something new to show.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Starts `config` as the session of generation `mine`: its output feeds `term`, what the emulator reports goes to
/// `ui` and its replies to the program. Output of a session whose generation is no longer the pane's is dropped.
fn start_session(
    config: &SpawnConfig,
    term: &Arc<FairMutex<TerminalCore>>,
    ui: &Sender<UiEvent>,
    activity: &Arc<AtomicU8>,
    generation: &Arc<AtomicU32>,
    wake: &Wake,
    mine: u32,
) -> std::io::Result<Live> {
    let (input_tx, input_rx) = channel::<Vec<u8>>();
    let session_cell: Arc<OnceLock<Arc<Session>>> = Arc::new(OnceLock::new());
    let pending_ack = Arc::new(AtomicUsize::new(0));

    let sink: Sink = {
        let (term, ui_tx, input_tx, session_cell, pending_ack, activity, generation, wake) = (
            term.clone(),
            ui.clone(),
            input_tx.clone(),
            session_cell.clone(),
            pending_ack.clone(),
            activity.clone(),
            generation.clone(),
            wake.clone(),
        );
        Box::new(move |event| {
            if generation.load(Ordering::Relaxed) != mine {
                return false;
            }
            match event {
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
                    wake();
                    true
                }
                Event::Exit(code) => {
                    term.lock().on_child_exit();
                    let _ = ui_tx.send(UiEvent::Exit(code));
                    wake();
                    true
                }
            }
        })
    };

    let sessions = Sessions::default();
    let (id, _pid) = sessions.spawn(config, sink)?;
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
    Ok(Live { input: input_tx, sessions, id })
}

impl Pane {
    pub fn spawn(program: PathBuf, args: Vec<String>, cols: u16, rows: u16) -> std::io::Result<Pane> {
        let mut env = BTreeMap::new();
        env.insert("TERM_PROGRAM".to_owned(), "Nekominal".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        // Wcswidth (0x10) is the mode that agrees with alacritty_terminal's per-code-point width model
        // (emulator blueprint, section 11.5).
        let config = SpawnConfig { program, args, cwd: None, env, cols, rows, width_model: WidthModel::Graphemes, inherit_environment: true };
        Pane::spawn_with(config, CoreConfig::default())
    }

    /// Starts the session described by `config` (a profile's program, arguments, directory and environment) with
    /// an emulator set up by `core`.
    pub fn spawn_with(config: SpawnConfig, core: CoreConfig) -> std::io::Result<Pane> {
        Pane::spawn_woken(config, core, Arc::new(|| {}))
    }

    /// Like [`Pane::spawn_with`], and `wake` is called from the reader thread after every frame of output and when
    /// the program ends, so a shell that draws on demand knows when there is something new to draw.
    pub fn spawn_woken(config: SpawnConfig, core: CoreConfig, wake: Wake) -> std::io::Result<Pane> {
        let (cols, rows) = (usize::from(config.cols), usize::from(config.rows));
        let term = Arc::new(FairMutex::new(TerminalCore::new(cols, rows, core)));
        let (ui_tx, ui_rx) = channel::<UiEvent>();
        let activity = Arc::new(AtomicU8::new(Activity::Unknown.code()));
        let generation = Arc::new(AtomicU32::new(0));
        let live = start_session(&config, &term, &ui_tx, &activity, &generation, &wake, 0)?;
        Ok(Pane {
            term,
            events: ui_rx,
            live: RefCell::new(live),
            ui: ui_tx,
            generation,
            activity,
            wake,
            pointer: RefCell::default(),
        })
    }

    /// Ends the session and starts `config` in the same pane, at the size the pane has now. What the old program left
    /// in the emulator (the alternate screen, the modes it set, the reports of an agent) goes; the screen it drew and
    /// the scrollback stay, and the new session draws below them. An error leaves the pane with no session.
    pub fn restart(&self, mut config: SpawnConfig) -> std::io::Result<()> {
        let mine = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let live = self.live.borrow();
            live.sessions.close(live.id);
        }
        {
            let mut core = self.term.lock();
            core.reset_for_new_session();
            config.cols = u16::try_from(core.columns()).unwrap_or(config.cols);
            config.rows = u16::try_from(core.rows()).unwrap_or(config.rows);
        }
        self.activity.store(Activity::Unknown.code(), Ordering::Relaxed);
        let live = start_session(&config, &self.term, &self.ui, &self.activity, &self.generation, &self.wake, mine)?;
        *self.live.borrow_mut() = live;
        Ok(())
    }
    /// Keyboard, IME and unbracketed paste bytes from the UI thread: a line break in them is Enter, which submits a
    /// line at the prompt.
    pub fn send(&self, bytes: Vec<u8>) {
        if bytes.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            let _ = self.activity.try_update(Ordering::Relaxed, Ordering::Relaxed, |code| {
                Some(Activity::from_code(code).after_submit().code())
            });
        }
        self.queue(bytes);
    }

    /// Bytes for the program that submit nothing by themselves.
    fn queue(&self, bytes: Vec<u8>) {
        let _ = self.live.borrow().input.send(bytes);
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
        input_modes_of(&self.term.lock())
    }

    /// The user typed, pasted or composed something for the program: a selection is dismissed and a view scrolled
    /// back returns to the live screen, as in Windows Terminal. Output does neither: new lines do not move a view
    /// the user scrolled back to read, and a selection follows its text into the scrollback.
    fn typed(&self) {
        let mut core = self.term.lock();
        if core.selection_active() {
            core.selection_clear();
        }
        if core.display_offset() > 0 {
            core.scroll_display(Scroll::Bottom);
        }
    }

    /// Encodes a key event for the program's current modes and queues it. False when nothing is sent.
    pub fn send_key(&self, input: &KeyInput) -> bool {
        match encode_key(input, &self.input_modes()) {
            Some(bytes) => {
                if input.event != KeyEvent::Release {
                    self.typed();
                }
                self.send(bytes);
                true
            }
            None => false,
        }
    }

    /// IME commits and other composed text (raw UTF-8 on the bundled ConPTY).
    pub fn send_text(&self, text: &str) {
        self.typed();
        self.send(encode_text(text));
    }

    pub fn paste(&self, text: &str) {
        let bracketed = self.input_modes().bracketed_paste;
        self.typed();
        let bytes = prepare_paste(text, bracketed);
        // A program that asked for bracketed paste takes the text as data: nothing runs until it sees Enter, so its
        // line breaks are not a submitted line. Without the brackets they are keys.
        if bracketed { self.queue(bytes) } else { self.send(bytes) }
    }

    /// A pointer event over the grid of the pane: the selection and the view change here, mouse reports go to the
    /// program, and what is left for the shell (copy on select, opening a link) is returned.
    pub fn pointer(&self, event: &PointerEvent, grid: &PointerGrid) -> PointerEffects {
        let mut effects = {
            let mut core = self.term.lock();
            let modes = input_modes_of(&core);
            self.pointer.borrow_mut().handle(&mut core, event, grid, &modes)
        };
        for report in effects.reports.drain(..) {
            let _ = self.live.borrow().input.send(report);
        }
        effects
    }

    /// The pointer is gone (capture lost, window deactivated): held buttons go up for the program and a drag ends.
    pub fn pointer_cancel(&self, grid: &PointerGrid) -> PointerEffects {
        self.pointer(&PointerEvent::new(PointerKind::Cancel, 0.0, 0.0), grid)
    }

    /// One step of the scrolling a drag held outside the grid asks for. True when the view moved.
    pub fn drag_scroll(&self) -> bool {
        self.pointer.borrow_mut().drag_scroll(&mut self.term.lock())
    }

    /// The link under a cell of the viewport that may be opened, and the cells it covers.
    pub fn link_at(&self, row: usize, column: usize) -> Option<Link> {
        self.term.lock().link_at(row, column)
    }

    /// The selected text as the clipboard gets it.
    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_text()
    }

    pub fn select_all(&self) {
        self.term.lock().select_all();
    }

    pub fn clear_selection(&self) {
        let mut core = self.term.lock();
        if core.selection_active() {
            core.selection_clear();
        }
    }

    /// What OSC 52 may do from now on (a settings change reaches panes that already run).
    pub fn set_osc52(&self, osc52: Osc52) {
        self.term.lock().set_osc52(osc52);
    }

    /// How many lines of scrollback the pane keeps from now on (a settings change reaches panes that already run).
    pub fn set_scrollback_lines(&self, lines: usize) {
        self.term.lock().set_scrollback_lines(lines);
    }

    /// Whether the prompt line is pinned over a view scrolled into its output from now on (a settings change).
    pub fn set_sticky_header(&self, on: bool) {
        self.term.lock().set_sticky_header(on);
    }

    /// The cursor a program that chose none gets from now on (a settings change reaches panes that already run).
    pub fn set_cursor_style(&self, style: CursorStyle) {
        self.term.lock().set_cursor_style(style);
    }

    /// Answers an OSC 52 read with the clipboard text. A clipboard over [`MAX_CLIPBOARD_TEXT`] is not sent: the
    /// program gets no answer, as for a read the policy denies. False when nothing was sent.
    pub fn answer_clipboard(&self, request: &ClipboardRequest, text: &str) -> bool {
        if text.len() > MAX_CLIPBOARD_TEXT {
            return false;
        }
        self.live.borrow().input.send((request.reply)(text).into_bytes()).is_ok()
    }

    /// Closes the session (kills the process tree) while the process can still create threads. Drop only does it as a fallback:
    /// at process exit the main thread's TLS destructors run after thread creation is already denied, and
    /// Sessions::close spawns a thread (found by the vertical slice: panic ailed to spawn thread: Access is denied).
    pub fn shutdown(&self) {
        let live = self.live.borrow();
        live.sessions.close(live.id);
    }

    /// A handle other threads can use to feed the writer thread (probe scripts).
    pub fn sender(&self) -> Sender<Vec<u8>> {
        self.live.borrow().input.clone()
    }

    /// The file name of the program the pane is running now (`vim.exe`; the shell when nothing else runs), if it can be told.
    /// The program the session runs now, with the path of its file.
    pub fn job(&self) -> Option<nekominal_pty::conpty::Job> {
        let live = self.live.borrow();
        live.sessions.get(live.id).ok()?.job()
    }

    /// Term and PTY are resized together under the terminal lock so a snapshot never sees half a resize.
    pub fn resize(&self, cols: u16, rows: u16) {
        let mut core = self.term.lock();
        core.resize(cols as usize, rows as usize);
        let live = self.live.borrow();
        if let Ok(session) = live.sessions.get(live.id) {
            let _ = session.resize(cols, rows);
        }
    }

    /// Scrolls the viewport through the scrollback (positive: toward older lines). Clamped to the history, so a
    /// program on the alternate screen, which has none, does not move.
    pub fn scroll_lines(&self, lines: i32) {
        self.scroll(Scroll::Delta(lines));
    }

    /// Scrolls the viewport by a page, to the oldest line or back to the live screen.
    pub fn scroll(&self, scroll: Scroll) {
        self.term.lock().scroll_display(scroll);
    }

    /// Scrolls to the previous or next prompt the shell marked (OSC 133). True when the view moved.
    pub fn scroll_to_prompt(&self, forward: bool) -> bool {
        self.term.lock().scroll_to_prompt(forward)
    }

    /// "Clear buffer": the screen and the scrollback go. A shell that waits at its prompt (its prompt marks say
    /// so) is asked to draw it again with Ctrl+L; a program that gives no marks is left to draw when it likes, and
    /// the screen stays blank until then.
    pub fn clear_buffer(&self) {
        let cleared = self.term.lock().clear_buffer();
        if cleared && self.activity() == Activity::AtPrompt {
            self.send(vec![0x0c]);
        }
    }

    /// Keyboard focus moved into or out of the pane: the cursor shape follows it and a program that asked
    /// for focus reports (DECSET 1004) is told.
    pub fn set_focused(&self, focused: bool) {
        let modes = {
            let mut core = self.term.lock();
            core.set_focused(focused);
            input_modes_of(&core)
        };
        if let Some(report) = encode_focus(focused, &modes) {
            self.send(report.to_vec());
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        let live = self.live.borrow();
        live.sessions.close(live.id);
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

    /// `n` copies of "QUFB", the base64 of "AAA": a payload of exactly `3 * n` bytes.
    fn osc52_write(clipboard: char, n: usize) -> Vec<u8> {
        format!("\x1b]52;{clipboard};{}\x07", "QUFB".repeat(n)).into_bytes()
    }

    #[test]
    fn an_osc52_write_reaches_the_shell_as_text_and_the_selection_target_does_not() {
        let events = events_for(&osc52_write('c', 1));
        let [UiEvent::ClipboardStore(text)] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(text, "AAA");
        assert!(matches!(events_for(&osc52_write('p', 1)).as_slice(), [UiEvent::Other(_)]));
    }

    #[test]
    fn an_osc52_write_over_the_limit_is_refused_whole_and_the_event_does_not_carry_the_text() {
        let at_limit = MAX_CLIPBOARD_TEXT / 3;
        assert_eq!(MAX_CLIPBOARD_TEXT % 3, 1, "the two payloads below are just under and just over the limit");
        let events = events_for(&osc52_write('c', at_limit));
        assert!(matches!(events.as_slice(), [UiEvent::ClipboardStore(text)] if text.len() == MAX_CLIPBOARD_TEXT - 1));
        let events = events_for(&osc52_write('c', at_limit + 1));
        assert!(
            matches!(events.as_slice(), [UiEvent::ClipboardRefused { bytes }] if *bytes == MAX_CLIPBOARD_TEXT + 2),
            "{events:?}"
        );
    }

    #[test]
    fn an_osc52_read_needs_the_policy_and_is_answered_in_the_osc52_form() {
        let query = b"\x1b]52;c;?\x07";
        assert!(events_for(query).is_empty(), "the default policy denies reads");
        let mut core = TerminalCore::new(80, 24, CoreConfig { osc52: Osc52::CopyPaste, ..CoreConfig::default() });
        core.feed(query);
        let events: Vec<UiEvent> = core.take_events().into_iter().map(ui_event).collect();
        let [UiEvent::ClipboardLoad(request)] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!((request.reply)("hi"), "\x1b]52;c;aGk=\x07");
        assert_eq!(format!("{request:?}"), "ClipboardRequest", "no clipboard text can leak through Debug");
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
