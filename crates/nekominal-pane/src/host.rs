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
        atomic::{AtomicUsize, Ordering},
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
    alacritty_terminal::{grid::Scroll, sync::FairMutex},
};

#[derive(Debug)]
pub enum UiEvent {
    Title(String),
    /// The shell reported its working directory (OSC 7).
    Cwd(String),
    Bell,
    Exit(u32),
    Other(String),
}

pub struct Pane {
    pub term: Arc<FairMutex<TerminalCore>>,
    pub events: Receiver<UiEvent>,
    input: Sender<Vec<u8>>,
    sessions: Sessions,
    id: u32,
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

        let sink: Sink = {
            let (term, input_tx, session_cell, pending_ack) =
                (term.clone(), input_tx.clone(), session_cell.clone(), pending_ack.clone());
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
                            CoreEvent::Title(title) => {
                                let _ = ui_tx.send(UiEvent::Title(title));
                            }
                            CoreEvent::Cwd(path) => {
                                let _ = ui_tx.send(UiEvent::Cwd(path));
                            }
                            CoreEvent::Bell => {
                                let _ = ui_tx.send(UiEvent::Bell);
                            }
                            other => {
                                let _ = ui_tx.send(UiEvent::Other(format!("{other:?}")));
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
        Ok(Pane { term, events: ui_rx, input: input_tx, sessions, id })
    }

    /// Keyboard, paste and IME bytes from the UI thread.
    pub fn send(&self, bytes: Vec<u8>) {
        let _ = self.input.send(bytes);
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
