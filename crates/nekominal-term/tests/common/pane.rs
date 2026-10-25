//! The threading design the blueprint recommends, wired to `nekominal-pty`:
//!
//! * the PTY crate's forwarder thread calls the sink; the sink feeds `TerminalCore` under a
//!   `FairMutex`, queues replies for a writer thread, acks the bytes and wakes the UI;
//! * the UI thread takes the same lock only to flush an expired DEC 2026 frame and copy a snapshot;
//! * `Session::write` is only ever called from the writer thread, so the forwarder can never block on
//!   a full input pipe while ConPTY waits for the forwarder to drain output.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use nekominal_pty::{
    conpty::{SpawnConfig, WidthModel},
    session::{Event, Session, Sessions, Sink},
};
use nekominal_term::{CoreConfig, CoreEvent, Snapshot, TerminalCore, alacritty_terminal::sync::FairMutex};

/// What the UI thread has to know about besides the grid.
#[derive(Debug)]
pub enum UiEvent {
    Title(String),
    Bell,
    Exit(u32),
    Other(String),
}

#[derive(Default)]
pub struct Stats {
    pub frames_fed: AtomicU64,
    pub bytes_fed: AtomicU64,
    pub feed_nanos_total: AtomicU64,
    pub feed_nanos_max: AtomicU64,
    pub replies_written: AtomicU64,
}

pub struct Pane {
    pub term: Arc<FairMutex<TerminalCore>>,
    pub stats: Arc<Stats>,
    pub ui_events: Receiver<UiEvent>,
    /// Set by the sink when it wants a redraw; the UI clears it when it starts drawing.
    pub redraw_requested: Arc<AtomicBool>,
    input: Sender<Vec<u8>>,
    sessions: Sessions,
    id: u32,
    wake: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl Pane {
    pub fn spawn(program: PathBuf, args: Vec<String>, cols: u16, rows: u16, graphemes: bool) -> Pane {
        let term = Arc::new(FairMutex::new(TerminalCore::new(cols as usize, rows as usize, CoreConfig::default())));
        let stats = Arc::new(Stats::default());
        let redraw_requested = Arc::new(AtomicBool::new(false));
        let wake = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (input_tx, input_rx) = channel::<Vec<u8>>();
        let (ui_tx, ui_rx) = channel::<UiEvent>();
        let session_cell: Arc<OnceLock<Arc<Session>>> = Arc::new(OnceLock::new());
        let pending_ack = Arc::new(AtomicUsize::new(0));

        let sink: Sink = {
            let (term, stats, redraw, wake) = (term.clone(), stats.clone(), redraw_requested.clone(), wake.clone());
            let (input_tx, session_cell, pending_ack) = (input_tx.clone(), session_cell.clone(), pending_ack.clone());
            Box::new(move |event| match event {
                Event::Data(bytes) => {
                    let started = Instant::now();
                    let (events, dirty) = {
                        let mut core = term.lock();
                        core.feed(&bytes);
                        (core.take_events(), core.is_dirty())
                    };
                    let took = started.elapsed().as_nanos() as u64;
                    stats.frames_fed.fetch_add(1, Ordering::Relaxed);
                    stats.bytes_fed.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    stats.feed_nanos_total.fetch_add(took, Ordering::Relaxed);
                    stats.feed_nanos_max.fetch_max(took, Ordering::Relaxed);

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
                    if dirty && !redraw.swap(true, Ordering::AcqRel) {
                        let (lock, condvar) = &*wake;
                        *lock.lock().unwrap() = true;
                        condvar.notify_one();
                    }
                    true
                }
                Event::Exit(code) => {
                    term.lock().on_child_exit();
                    let _ = ui_tx.send(UiEvent::Exit(code));
                    redraw.store(true, Ordering::Release);
                    let (lock, condvar) = &*wake;
                    *lock.lock().unwrap() = true;
                    condvar.notify_one();
                    true
                }
            })
        };

        let width_model = if graphemes { WidthModel::Graphemes } else { WidthModel::Wcswidth };
        let config = SpawnConfig { program, args, cwd: None, env: BTreeMap::new(), cols, rows, width_model, inherit_environment: true };
        let sessions = Sessions::default();
        let (id, _pid) = sessions.spawn(&config, sink).expect("spawn session");
        let session = sessions.get(id).unwrap();
        let _ = session_cell.set(session.clone());
        // Catch up on acks for frames that arrived before the session handle was published.
        let early = pending_ack.swap(0, Ordering::Relaxed);
        if early > 0 {
            session.ack(early);
        }

        // The writer thread is the only caller of Session::write.
        let writer_stats = stats.clone();
        thread::Builder::new()
            .name("pty-writer".into())
            .spawn(move || {
                for bytes in input_rx {
                    if session.write(&bytes).is_err() {
                        break;
                    }
                    writer_stats.replies_written.fetch_add(1, Ordering::Relaxed);
                }
            })
            .unwrap();

        Pane { term, stats, ui_events: ui_rx, redraw_requested, input: input_tx, sessions, id, wake }
    }

    /// Keyboard / paste bytes from the UI thread.
    pub fn send_input(&self, bytes: Vec<u8>) {
        let _ = self.input.send(bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.term.lock().resize(cols as usize, rows as usize);
        if let Ok(session) = self.sessions.get(self.id) {
            let _ = session.resize(cols, rows);
        }
    }

    /// The UI loop body: wait for a wake-up or the synchronized-update deadline, then snapshot.
    /// Returns the time spent waiting for the lock.
    pub fn redraw(&self, snapshot: &mut Snapshot, max_wait: Duration) -> Option<Duration> {
        let deadline = self.term.lock().sync_deadline();
        let timeout = deadline.map_or(max_wait, |d| d.saturating_duration_since(Instant::now()).min(max_wait));
        {
            let (lock, condvar) = &*self.wake;
            let guard = lock.lock().unwrap();
            let (mut guard, _) = condvar.wait_timeout_while(guard, timeout, |woken| !*woken).unwrap();
            *guard = false;
        }
        self.redraw_requested.store(false, Ordering::Release);
        let wait_started = Instant::now();
        let mut core = self.term.lock();
        let waited = wait_started.elapsed();
        core.flush_expired_sync(Instant::now());
        if core.take_dirty() {
            core.snapshot(snapshot);
            return Some(waited);
        }
        None
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.sessions.close(self.id);
    }
}
