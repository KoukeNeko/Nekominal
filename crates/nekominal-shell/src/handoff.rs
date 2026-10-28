//! The shell's side of an update that keeps the tabs running (`docs/updater-design.md`); the side that holds the tabs
//! meanwhile, the updater, is this program too, a copy of it in the hold mode (`hold`).
//!
//! Giving: Update and restart copies this program out of the app's folder and starts the copy with `--hold`
//! ([`start_holder`]); then every window of this process is handed to it ([`give`]): each pane stops reading its output,
//! its session's handles are duplicated into the updater, and the windows go over the updater's pipe with what each pane
//! shows; once the updater has them, this process ends on the spot, closing nothing (closing a pseudoconsole would end
//! its shell). Another Nekominal is asked to do the same through its automation pipe (`hand-off`).
//!
//! Taking: the updater starts the new Nekominal with `--adopt <pipe>`; it takes the windows ([`take`]), opens each with its
//! panes on the sessions handed over (`App::open_handed_over`), and tells the updater which it adopted ([`confirm`]). A
//! pane that cannot be adopted shows its text and starts a fresh shell, as a restored session does.

use std::{
    cell::RefCell,
    collections::HashSet,
    fs::File,
    io::BufReader,
    path::Path,
    sync::OnceLock,
    time::Duration,
};

use nekominal_pty::handoff::{self as wire, Bundle, Message, PROTOCOL, Process, WindowState};

/// The pipe of the updater this process was started to take the windows from (`--adopt`).
static ADOPT: OnceLock<String> = OnceLock::new();

thread_local! {
    /// The window being made, which `App::new` opens instead of a tab.
    static PENDING: RefCell<Option<WindowState>> = const { RefCell::new(None) };
    /// The sessions of the window being made that panes adopted, and those whose handles a pane took (adopted, or
    /// closed when adopting failed): the rest are closed after it.
    static ADOPTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CONSUMED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// The pipe of the updater this window's Update started, for the hand-off posted to run outside the window's borrow.
    static GIVE_TO: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `args` without `--adopt <pipe>`, which is kept for [`adopt_pipe`].
pub fn strip_adopt(args: Vec<String>) -> Vec<String> {
    match args.iter().position(|arg| arg == "--adopt") {
        Some(at) if at + 1 < args.len() => {
            let _ = ADOPT.set(args[at + 1].clone());
            args.into_iter().enumerate().filter(|(i, _)| *i != at && *i != at + 1).map(|(_, arg)| arg).collect()
        }
        _ => args,
    }
}

pub fn adopt_pipe() -> Option<&'static str> {
    ADOPT.get().map(String::as_str)
}

pub fn set_pending(window: WindowState) {
    ADOPTED.with(|adopted| adopted.borrow_mut().clear());
    CONSUMED.with(|consumed| consumed.borrow_mut().clear());
    PENDING.with(|pending| *pending.borrow_mut() = Some(window));
}

pub fn take_pending() -> Option<WindowState> {
    PENDING.with(|pending| pending.borrow_mut().take())
}

/// A pane took the handles of `session`, adopting it when `adopted`.
pub fn consumed(session: &str, adopted: bool) {
    CONSUMED.with(|consumed| consumed.borrow_mut().insert(session.to_owned()));
    if adopted {
        ADOPTED.with(|list| list.borrow_mut().push(session.to_owned()));
    }
}

/// After the window of `window` was made (or not, `made`): the sessions it adopted. The handles of the panes it did not
/// take are closed: the updater ends those sessions.
pub fn window_done(window: &WindowState, made: bool) -> Vec<String> {
    let consumed = CONSUMED.with(|consumed| std::mem::take(&mut *consumed.borrow_mut()));
    for pane in &window.panes {
        if let Some(parts) = &pane.parts
            && !consumed.contains(&pane.session)
        {
            wire::close(parts);
        }
    }
    let adopted = ADOPTED.with(|adopted| std::mem::take(&mut *adopted.borrow_mut()));
    // A window that failed dropped its panes, and their sessions with them.
    if made { adopted } else { Vec::new() }
}

/// The place of pane `id` in the tree `layout` of its tab: what names a handed-over pane on both sides. Its id does not:
/// a tab numbers its panes from 1, while reading the session again numbers them across the window (the first pane of
/// the second tab is 2), so by id only the panes of the first tab were found, and every other came back a fresh shell.
pub fn place(layout: &nekominal_model::layout::Node, id: nekominal_model::layout::PaneId) -> Option<u32> {
    layout.pane_ids().iter().position(|pane| *pane == id).map(|at| at as u32)
}

/// Where among `panes` the pane handed over as pane `id` of tab `tab` is (`id` and `layout` as the session read again
/// has them).
pub fn find(panes: &[wire::PaneState], tab: usize, layout: &nekominal_model::layout::Node, id: nekominal_model::layout::PaneId) -> Option<usize> {
    let place = place(layout, id)?;
    panes.iter().position(|state| state.tab == tab && state.pane == place)
}

/// Copies this program out of the app's folder (`hold::copy_of`) and starts the copy in the hold mode for the Scoop install
/// in `app`: the name of the pipe it serves.
pub fn start_holder(app: &Path) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Out of the folder Scoop replaces (Scoop refuses to update while a program runs from it), under another name.
    let copy = crate::hold::copy_of(&exe).map_err(|e| format!("hold: {e}"))?;
    let sid = wire::user_sid().map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |at| at.as_millis());
    let pipe = format!(r"\\.\pipe\nekominal-update-{sid}-{}-{stamp}", std::process::id());
    let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let args = ["--hold", pipe.as_str(), "--app", &app.to_string_lossy(), "--version", crate::update::CURRENT];
    let start = |flags: u32| std::process::Command::new(&copy).args(args).current_dir(&home).creation_flags(flags).spawn();
    // Out of any job this process runs in (a terminal it was started from), so that ending that job does not end the update.
    start(CREATE_BREAKAWAY_FROM_JOB).or_else(|_| start(0)).map_err(|e| format!("hold: {e}"))?;
    Ok(pipe)
}

/// Keeps `pipe` for the hand-off that the window posts itself.
pub fn give_later(pipe: String) {
    GIVE_TO.with(|give| *give.borrow_mut() = Some(pipe));
}

pub fn take_give_to() -> Option<String> {
    GIVE_TO.with(|give| give.borrow_mut().take())
}

/// Hands every window of this process to the updater serving `pipe`, and ends the process. Returns only when that could
/// not be done: what went wrong, with every pane reading its output again and nothing handed.
pub fn give(pipe: &str) -> String {
    let error = match try_give(pipe) {
        Ok(()) => {
            crate::util::log("update: the windows were handed to the updater");
            crate::api::forget_instance();
            // No destructor, no pseudoconsole closed: the shells live on in the updater's handles.
            // SAFETY: ends this process.
            unsafe {
                let _ = windows::Win32::System::Threading::TerminateProcess(windows::Win32::System::Threading::GetCurrentProcess(), 0);
            }
            unreachable!("the process ended")
        }
        Err(e) => e,
    };
    crate::util::log(&format!("update: hand-off: {error}"));
    error
}

fn try_give(pipe: &str) -> Result<(), String> {
    let connection = wire::connect(pipe, Duration::from_secs(15)).map_err(|e| format!("{pipe}: {e}"))?;
    let server = wire::peer_pid(&connection, false).map_err(|e| e.to_string())?;
    let updater = Process::open(server).map_err(|e| e.to_string())?;
    // The handles of the shells go to whoever serves the pipe: only to a copy of this very program, holding them.
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if !updater.image().is_some_and(|image| crate::hold::is_copy_of(&image, &exe)) {
        return Err(format!("process {server} that serves {pipe} is not a copy of this Nekominal holding the tabs"));
    }
    let apps = crate::win::apps();
    // The session file too, as when the last window closes: what the next start restores if the update goes wrong.
    if let Some(app) = apps.first().and_then(|(_, app)| app.try_borrow().ok()) {
        app.save_session();
    }
    let mut windows = Vec::new();
    let mut handed = Vec::new();
    for (_, app) in &apps {
        let Ok(app) = app.try_borrow() else {
            undo(&apps, &updater, &handed);
            return Err("a window is busy".into());
        };
        windows.push(app.hand_off_window(&updater, &mut handed));
    }
    let bundle = Bundle {
        pid: std::process::id(),
        exe: std::env::current_exe().map(|exe| exe.display().to_string()).unwrap_or_default(),
        version: crate::update::CURRENT.to_owned(),
        windows,
    };
    let answer = wire::write_message(&connection, &Message::Give { v: PROTOCOL, bundle }).and_then(|()| wire::read_message(&mut BufReader::new(&connection)));
    match answer {
        Ok(Message::Ok) => Ok(()),
        other => {
            undo(&apps, &updater, &handed);
            Err(match other {
                Ok(Message::Error { message }) => message,
                Ok(other) => format!("the updater answered {other:?}"),
                Err(e) => e.to_string(),
            })
        }
    }
}

/// Every pane reads again, and the handles duplicated into the updater are closed there.
fn undo(apps: &[(windows::Win32::Foundation::HWND, std::rc::Rc<RefCell<crate::app::App>>)], updater: &Process, handed: &[wire::Parts]) {
    for parts in handed {
        wire::close_in(updater, parts);
    }
    for (_, app) in apps {
        if let Ok(app) = app.try_borrow() {
            app.resume_panes();
        }
    }
}

/// Takes the windows from the updater serving `pipe`: the connection, to confirm on, and the windows, their handles this
/// process's. A version of the messages this one does not speak gives the windows with their text only.
pub fn take(pipe: &str) -> Option<(File, Vec<WindowState>)> {
    let taken = (|| -> std::io::Result<(File, Vec<WindowState>)> {
        let connection = wire::connect(pipe, Duration::from_secs(10))?;
        wire::write_message(&connection, &Message::Take { v: PROTOCOL })?;
        match wire::read_message(&mut BufReader::new(&connection))? {
            Message::Windows { v, windows } if v == PROTOCOL => Ok((connection, windows)),
            // Its handles may mean something else: left alone (the updater ends those sessions), the text kept.
            Message::Windows { v, mut windows } => {
                crate::util::log(&format!("update: the updater speaks version {v} of the hand-off, this Nekominal {PROTOCOL}"));
                windows.iter_mut().flat_map(|window| &mut window.panes).for_each(|pane| pane.parts = None);
                Ok((connection, windows))
            }
            other => Err(std::io::Error::other(format!("the updater answered {other:?}"))),
        }
    })();
    taken.map_err(|e| crate::util::log(&format!("update: adopt from {pipe}: {e}"))).ok()
}

/// Tells the updater which sessions were adopted; it ends the others and goes.
pub fn confirm(connection: File, sessions: Vec<String>) {
    let said = wire::write_message(&connection, &Message::Adopted { sessions }).and_then(|()| wire::read_message(&mut BufReader::new(&connection)));
    if let Err(e) = said {
        crate::util::log(&format!("update: confirm: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adopt_and_its_pipe_are_taken_out_of_the_arguments() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(strip_adopt(args(&["new-tab", "-d", "C:\\"])), args(&["new-tab", "-d", "C:\\"]));
        assert_eq!(strip_adopt(args(&["--adopt"])), args(&["--adopt"]), "no pipe: left for the command line to refuse");
        assert_eq!(strip_adopt(args(&["--adopt", r"\\.\pipe\nekominal-update-x"])), Vec::<String>::new());
        assert_eq!(adopt_pipe(), Some(r"\\.\pipe\nekominal-update-x"));
    }

    #[test]
    fn every_pane_of_every_tab_is_found_again_though_reading_the_session_numbers_the_panes_across_the_window() {
        use std::collections::BTreeMap;

        use nekominal_model::{
            layout::{Axis, Node, Split},
            session::{PaneRecord, Session, TabRecord},
        };
        // The window of the sandbox run: four tabs of one pane (cmd, then three PowerShells), each pane 1 of its tab as
        // `Tab::new` numbers it, and a fifth tab split twice, whose ids are not in tree order after closing and splitting.
        let tab = |layout: Node| TabRecord {
            panes: layout.pane_ids().into_iter().map(|id| (id, PaneRecord::default())).collect::<BTreeMap<_, _>>(),
            active: layout.pane_ids()[0],
            layout,
            zoomed: None,
            name: None,
            color: None,
        };
        let split = |first, second| Node::Split(Box::new(Split { axis: Axis::Row, ratio: 0.5, first, second }));
        let given = vec![tab(Node::Leaf(1)), tab(Node::Leaf(1)), tab(Node::Leaf(1)), tab(Node::Leaf(1)), tab(split(Node::Leaf(3), split(Node::Leaf(1), Node::Leaf(4))))];
        // As `hand_off_window` names them, the session's name telling them apart.
        let panes: Vec<wire::PaneState> = given
            .iter()
            .enumerate()
            .flat_map(|(index, record)| {
                record.layout.pane_ids().into_iter().map(move |id| wire::PaneState {
                    tab: index,
                    pane: place(&record.layout, id).unwrap(),
                    session: format!("{index}-{id}"),
                    title: String::new(),
                    cols: 80,
                    rows: 24,
                    screen: String::new(),
                    parts: None,
                })
            })
            .collect();
        let value = Session { window: None, tabs: given.clone(), active_tab: 0 }.to_value();

        // As `open_handed_over` reads them back.
        let read = Session::parse(&value.to_string()).session;
        assert_eq!(read.tabs[1].layout, Node::Leaf(2), "the second tab's pane is 2 once read: by id it was not found");
        let mut found = Vec::new();
        for (index, record) in read.tabs.iter().enumerate() {
            for (id, given_id) in record.layout.pane_ids().into_iter().zip(given[index].layout.pane_ids()) {
                let at = find(&panes, index, &record.layout, id).unwrap_or_else(|| panic!("tab {index} pane {id} not found"));
                assert_eq!(panes[at].session, format!("{index}-{given_id}"));
                found.push(at);
            }
        }
        found.sort();
        assert_eq!(found, (0..panes.len()).collect::<Vec<_>>(), "each pane once");
        assert_eq!(find(&panes, 5, &Node::Leaf(1), 1), None, "a tab that was not handed over");
    }

    #[test]
    fn the_handles_of_panes_a_window_did_not_take_are_closed_and_only_a_window_made_adopts() {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::Threading::CreateEventW;
        // SAFETY: plain calls; every event is closed by the code under test or below.
        let event = || unsafe { CreateEventW(None, true, false, None).unwrap().0 as u64 };
        let parts = || wire::Parts { job: event(), process: event(), input: event(), output: event(), signal: event(), reference: event(), console: event(), pid: 1 };
        let pane = |session: &str, parts| wire::PaneState { tab: 0, pane: 1, session: session.into(), title: String::new(), cols: 80, rows: 24, screen: String::new(), parts };
        let (taken, left) = (parts(), parts());
        let window = WindowState { session: serde_json::json!({}), panes: vec![pane("1-1", Some(taken)), pane("1-2", Some(left)), pane("1-3", None)] };
        set_pending(window.clone());
        assert!(take_pending().is_some() && take_pending().is_none(), "taken once");
        consumed("1-1", true);
        assert_eq!(window_done(&window, true), ["1-1"]);
        // SAFETY: the events of `taken` are still open (consumed means a pane owns them); those of `left` were closed.
        unsafe {
            assert!(CloseHandle(HANDLE(taken.job as *mut _)).is_ok(), "a pane's handles are the pane's");
            assert!(CloseHandle(HANDLE(left.job as *mut _)).is_err(), "the handles nobody took are closed");
        }
        let window = WindowState { session: serde_json::json!({}), panes: vec![pane("1-1", Some(taken))] };
        set_pending(window.clone());
        consumed("1-1", true);
        assert!(window_done(&window, false).is_empty(), "a window that was not made adopted nothing");
    }
}
