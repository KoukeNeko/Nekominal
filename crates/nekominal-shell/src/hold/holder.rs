//! The sessions while no Nekominal runs: every Nekominal that hands over gives its windows here (`Give`), the handles
//! duplicated into this process, and the Nekominal the updater starts takes them all (`Take`), duplicated again into
//! it, then says which it adopted (`Adopted`). Only the process the updater started may take them. Until a taker has
//! confirmed, this process keeps its own copies: a Nekominal that fails to start (or to adopt) loses nothing, and the
//! next one gets them all again. [`Holder::finish`] then closes the copies, and ends the sessions nobody adopted.

use std::{
    fs::File,
    io::{self, BufReader},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use nekominal_pty::handoff::{self, Bundle, Message, PROTOCOL, Process, WindowState};

#[derive(Default)]
struct State {
    bundles: Vec<Bundle>,
    /// The process allowed to take the windows: the Nekominal this updater started last.
    taker: Option<u32>,
    /// The sessions the taker adopted, once it said so.
    adopted: Option<Vec<String>>,
    /// No more windows are taken: the new Nekominal is being started, or the update gave up. One that comes now would
    /// not reach it and would end with this process, so it is refused and keeps its own tabs.
    closed: bool,
}

pub struct Holder {
    pub pipe: String,
    state: Mutex<State>,
    changed: Condvar,
}

impl Holder {
    /// Serves the pipe `pipe` on a thread of its own, a thread per client. Fails when the name is taken.
    pub fn serve(pipe: &str) -> io::Result<Arc<Holder>> {
        let holder = Arc::new(Holder { pipe: pipe.to_owned(), state: Mutex::default(), changed: Condvar::new() });
        let mut waiting = handoff::listen(pipe, true)?;
        let server = holder.clone();
        std::thread::Builder::new().name("hold pipe".into()).spawn(move || loop {
            let connected = handoff::accept(&waiting);
            let next = loop {
                match handoff::listen(&server.pipe, false) {
                    Ok(next) => break next,
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            };
            let client = std::mem::replace(&mut waiting, next);
            if connected.is_ok() {
                let server = server.clone();
                let _ = std::thread::Builder::new().name("hold client".into()).spawn(move || server.client(client));
            }
        })?;
        Ok(holder)
    }

    fn client(&self, pipe: File) {
        let mut reader = BufReader::new(&pipe);
        let reply = match handoff::read_message(&mut reader) {
            Ok(Message::Give { v, bundle }) if v == PROTOCOL => {
                let mut state = self.state.lock().unwrap();
                if state.closed {
                    Message::Error { message: "the update no longer takes windows".into() }
                } else {
                    state.bundles.push(bundle);
                    drop(state);
                    self.changed.notify_all();
                    Message::Ok
                }
            }
            Ok(Message::Give { v, .. }) | Ok(Message::Take { v }) if v != PROTOCOL => Message::Error { message: format!("this updater speaks version {PROTOCOL}, not {v}") },
            Ok(Message::Take { .. }) => match self.give_to(&pipe) {
                Ok(pid) => match handoff::read_message(&mut reader) {
                    // Taken as the taker's word only while it is still the taker: one that answered as its time ran out
                    // was ended, and its list is not the next one's.
                    Ok(Message::Adopted { sessions }) => {
                        let mut state = self.state.lock().unwrap();
                        if state.taker != Some(pid) {
                            return;
                        }
                        state.adopted = Some(sessions);
                        drop(state);
                        self.changed.notify_all();
                        Message::Ok
                    }
                    _ => return,
                },
                Err(e) => Message::Error { message: e.to_string() },
            },
            Ok(other) => Message::Error { message: format!("unexpected {other:?}") },
            Err(_) => return,
        };
        let _ = handoff::write_message(&pipe, &reply);
    }

    /// Answers `Take`: the windows of every bundle, their handles duplicated into the client, which must be the taker.
    /// The client's pid.
    fn give_to(&self, pipe: &File) -> io::Result<u32> {
        let pid = handoff::peer_pid(pipe, true)?;
        let bundles = {
            let state = self.state.lock().unwrap();
            if state.taker != Some(pid) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("process {pid} was not started by the updater")));
            }
            state.bundles.clone()
        };
        let target = Process::open(pid)?;
        let windows: Vec<WindowState> = bundles
            .into_iter()
            .flat_map(|bundle| bundle.windows)
            .map(|mut window| {
                for pane in &mut window.panes {
                    // A session that cannot be duplicated goes as text: the new Nekominal starts a fresh shell there.
                    pane.parts = pane.parts.and_then(|parts| handoff::duplicate_into(&target, &parts).ok());
                }
                window
            })
            .collect();
        handoff::write_message(pipe, &Message::Windows { v: PROTOCOL, windows })?;
        Ok(pid)
    }

    /// From now on a window that hands over is refused (it keeps its tabs): the new Nekominal is about to be started.
    pub fn close(&self) {
        self.state.lock().unwrap().closed = true;
    }

    /// Waits until `count` bundles came, or `timeout` passed; how many came.
    pub fn wait_for_bundles(&self, count: usize, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        while state.bundles.len() < count {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        state.bundles.len()
    }

    /// Who handed over: the pid, program and version of each.
    pub fn givers(&self) -> Vec<(u32, String, String)> {
        self.state.lock().unwrap().bundles.iter().map(|b| (b.pid, b.exe.clone(), b.version.clone())).collect()
    }

    /// How many panes are held, and how many of them with their session.
    pub fn panes(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap();
        let panes = state.bundles.iter().flat_map(|b| &b.windows).flat_map(|w| &w.panes);
        panes.fold((0, 0), |(all, live), pane| (all + 1, live + usize::from(pane.parts.is_some())))
    }

    /// From now on only `pid` may take the windows; what an earlier taker said is forgotten.
    pub fn expect_taker(&self, pid: u32) {
        let mut state = self.state.lock().unwrap();
        state.taker = Some(pid);
        state.adopted = None;
    }

    /// The sessions the taker adopted, once it says so; None when `timeout` passed or `gone` says it ended first.
    pub fn wait_adopted(&self, timeout: Duration, mut gone: impl FnMut() -> bool) -> Option<Vec<String>> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(adopted) = &state.adopted {
                return Some(adopted.clone());
            }
            if Instant::now() >= deadline || gone() {
                return None;
            }
            state = self.changed.wait_timeout(state, Duration::from_millis(200)).unwrap().0;
        }
    }

    /// Lets go of every session: those adopted live on in the taker, the others end. The counts of both.
    pub fn finish(&self, adopted: &[String]) -> (usize, usize) {
        let bundles = std::mem::take(&mut self.state.lock().unwrap().bundles);
        let mut counts = (0, 0);
        for pane in bundles.iter().flat_map(|b| &b.windows).flat_map(|w| &w.panes) {
            let Some(parts) = &pane.parts else { continue };
            if adopted.contains(&pane.session) {
                handoff::close(parts);
                counts.0 += 1;
            } else {
                handoff::kill(parts);
                counts.1 += 1;
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::mpsc::{self, Receiver},
    };

    use nekominal_pty::{
        conpty::{SpawnConfig, WidthModel},
        handoff::PaneState,
        session::{Event, Session, Sessions, Sink},
    };

    use super::*;

    fn ping() -> SpawnConfig {
        SpawnConfig {
            program: PathBuf::from(std::env::var("SystemRoot").unwrap()).join(r"System32\cmd.exe"),
            args: ["/d", "/k", "ping -t 127.0.0.1"].map(String::from).to_vec(),
            cwd: None,
            env: BTreeMap::new(),
            cols: 100,
            rows: 30,
            width_model: WidthModel::default(),
            inherit_environment: true,
        }
    }

    fn sink() -> (Sink, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        (Box::new(move |event| tx.send(event).is_ok()), rx)
    }

    /// Acknowledges output (answering ConPTY's DA1) until `replies` more of ping's replies came.
    fn replies(session: &Session, rx: &Receiver<Event>, replies: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut text = Vec::new();
        while String::from_utf8_lossy(&text).matches("TTL=").count() < replies {
            let left = deadline.checked_duration_since(Instant::now()).expect("no replies in time");
            match rx.recv_timeout(left).expect("output") {
                Event::Data(bytes) => {
                    if bytes.windows(3).any(|w| w == b"\x1b[c") {
                        session.write(b"\x1b[?1;2c").unwrap();
                    }
                    session.ack(bytes.len());
                    text.extend(bytes);
                }
                Event::Exit(code) => panic!("ping ended with {code}"),
            }
        }
    }

    /// A running ping, paused, and its parts duplicated into this process as a Nekominal gives them to the updater.
    fn given(sessions: &Sessions) -> (Parts, Vec<u32>) {
        let (sink, rx) = sink();
        let (id, _) = sessions.spawn(&ping(), sink).unwrap();
        let session = sessions.get(id).unwrap();
        replies(&session, &rx, 1);
        // As a pane does, what comes is taken in: a pause waits for that.
        let consumer = session.clone();
        std::thread::spawn(move || {
            while let Ok(event) = rx.recv() {
                if let Event::Data(bytes) = event {
                    consumer.ack(bytes.len());
                }
            }
        });
        assert!(session.pause(Duration::from_secs(5)));
        let me = Process::open(std::process::id()).unwrap();
        (handoff::duplicate_into(&me, &session.parts().unwrap()).unwrap(), session.job_process_ids())
    }

    use nekominal_pty::handoff::Parts;

    fn pane(session: &str, parts: Option<Parts>) -> PaneState {
        PaneState { tab: 0, pane: 1, session: session.into(), title: String::new(), cols: 100, rows: 30, screen: "text".into(), parts }
    }

    fn ask(pipe: &str, message: &Message) -> Message {
        let client = handoff::connect(pipe, Duration::from_secs(5)).unwrap();
        handoff::write_message(&client, message).unwrap();
        handoff::read_message(&mut BufReader::new(&client)).unwrap()
    }

    #[test]
    fn the_holder_keeps_the_sessions_given_hands_them_to_its_taker_alone_and_ends_those_not_adopted() {
        let pipe = format!(r"\\.\pipe\nekominal-update-test-{}", std::process::id());
        let holder = Holder::serve(&pipe).unwrap();
        // The giver's own sessions are never closed by it: it exits instead, which this test cannot, so they are leaked.
        let giver = Box::leak(Box::new(Sessions::default()));
        let (kept, kept_pids) = given(giver);
        let (dropped, dropped_pids) = given(giver);
        let bundle = Bundle {
            pid: 1,
            exe: r"C:\x\Nekominal.exe".into(),
            version: "0.2.3".into(),
            windows: vec![WindowState { settings: None, session: serde_json::json!({}), panes: vec![pane("1-1", Some(kept)), pane("1-2", None), pane("1-3", Some(dropped))] }],
        };
        assert_eq!(ask(&pipe, &Message::Give { v: PROTOCOL + 1, bundle: bundle.clone() }), Message::Error { message: format!("this updater speaks version {PROTOCOL}, not {}", PROTOCOL + 1) });
        assert_eq!(ask(&pipe, &Message::Give { v: PROTOCOL, bundle }), Message::Ok);
        assert_eq!(holder.wait_for_bundles(1, Duration::from_secs(5)), 1);
        assert_eq!(holder.panes(), (3, 2));
        assert_eq!(holder.givers(), [(1, r"C:\x\Nekominal.exe".to_owned(), "0.2.3".to_owned())]);

        // Not the process the updater started: refused.
        assert!(matches!(ask(&pipe, &Message::Take { v: PROTOCOL }), Message::Error { .. }));
        holder.expect_taker(std::process::id());
        let client = handoff::connect(&pipe, Duration::from_secs(5)).unwrap();
        handoff::write_message(&client, &Message::Take { v: PROTOCOL }).unwrap();
        let mut reader = BufReader::new(&client);
        let Message::Windows { v, windows } = handoff::read_message(&mut reader).unwrap() else { panic!("no windows") };
        assert_eq!((v, windows.len(), windows[0].panes.len()), (PROTOCOL, 1, 3));
        let taken = windows[0].panes[0].parts.expect("the running session");
        assert_ne!(taken.job, kept.job, "duplicated again, for the taker");
        assert!(windows[0].panes[1].parts.is_none(), "a pane given as text stays text");

        let taker = Sessions::default();
        let (sink, rx) = sink();
        let (id, _) = taker.adopt(&taken, sink).unwrap();
        let session = taker.get(id).unwrap();
        handoff::close(&windows[0].panes[2].parts.unwrap());
        handoff::write_message(&client, &Message::Adopted { sessions: vec!["1-1".into()] }).unwrap();
        assert_eq!(handoff::read_message(&mut reader).unwrap(), Message::Ok);
        assert_eq!(holder.wait_adopted(Duration::from_secs(5), || false), Some(vec!["1-1".to_owned()]));

        // A window that hands over once the new Nekominal is starting is refused, and keeps its tabs.
        holder.close();
        let late = Bundle { pid: 2, exe: r"C:\x\Nekominal.exe".into(), version: "0.2.3".into(), windows: Vec::new() };
        assert!(matches!(ask(&pipe, &Message::Give { v: PROTOCOL, bundle: late }), Message::Error { .. }));
        assert_eq!(holder.givers().len(), 1);

        assert_eq!(holder.finish(&["1-1".to_owned()]), (1, 1));
        for pid in dropped_pids {
            if let Ok(process) = Process::open(pid) {
                assert!(process.exited_within(Duration::from_secs(5)), "{pid} of the session nobody adopted still runs");
            }
        }
        // The adopted ping runs on, the updater's copies closed.
        replies(&session, &rx, 2);
        assert_eq!(session.job_process_ids(), kept_pids);
        taker.close(id);
    }
}
