//! A pane that becomes an administrator's shell in a window that is not elevated.
//!
//! The window cannot be elevated (a window of one integrity level holds nothing of another), and a running shell cannot
//! either, so an elevated copy of this program (`--elevate-pane`, no window) starts the new shell on a pseudoconsole of its
//! own and hands the session to the window as an update does (`nekominal_pty::handoff`): the handles of the session are
//! duplicated into the window, which adopts them as the pane's session. Windows asks the user once, when the copy starts.
//!
//! The two sides talk over a pipe open to the user alone, with a name nobody guesses, and each checks who is on the other
//! end: the window that the program that connected is the one it started, the copy that the window is this program.

use std::{
    fs::File,
    hash::{BuildHasher, RandomState},
    io::BufReader,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use nekominal_pty::{
    conpty::{Pty, SpawnConfig},
    handoff::{self, Message, PROTOCOL, Parts, Process},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, WPARAM},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            Threading::GetProcessId,
        },
        UI::{
            Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::{PostMessageW, SW_HIDE},
        },
    },
    core::{HRESULT, HSTRING, PCWSTR, w},
};

/// The argument that makes this program the elevated copy.
const FLAG: &str = "--elevate-pane";
/// `ERROR_CANCELLED`: the user said no to the prompt of Windows.
const ERROR_CANCELLED: u32 = 1223;
/// How long a window waits for the copy to connect: the prompt waits for the user.
const CONNECT_WITHIN: Duration = Duration::from_secs(300);
/// How long the window has to adopt the session before the copy is told no.
const ADOPT_WITHIN: Duration = Duration::from_secs(60);

/// What the window is told, once the elevated copy has answered (or not).
pub enum Outcome {
    /// The session, its handles values of this process: the window adopts it and says how that went through `reply`.
    Ready { parts: Parts, reply: mpsc::Sender<Reply> },
    /// The user said no to the prompt of Windows: nothing to tell.
    Declined,
    Failed(String),
}

/// How the window used a session it was given.
pub enum Reply {
    Adopted,
    /// It tried, and the handles are closed by that.
    AdoptFailed,
    /// It did not try (the pane is gone): the handles are still here, and closed.
    NotUsed,
}

static OUTCOMES: Mutex<Vec<(u64, Outcome)>> = Mutex::new(Vec::new());

/// The outcome posted to the window as `WM_ELEVATED` with this `id`.
pub fn take(id: u64) -> Option<Outcome> {
    let mut outcomes = OUTCOMES.lock().ok()?;
    let at = outcomes.iter().position(|(of, _)| *of == id)?;
    Some(outcomes.swap_remove(at).1)
}

// --- the window --------------------------------------------------------------------------------------------------------

/// Starts the elevated copy for the session of `config`, and talks to it on a thread; the outcome is posted to `hwnd`.
pub fn start(hwnd: isize, id: u64, config: SpawnConfig) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let sid = handoff::user_sid().map_err(|e| e.to_string())?;
    let name = format!(r"\\.\pipe\nekominal-elevate-{sid}-{}-{:016x}", std::process::id(), RandomState::new().hash_one(id));
    let pipe = handoff::listen(&name, true).map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("elevate".into())
        .spawn(move || talk(hwnd, id, &name, &pipe, &exe, &config))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

enum Failure {
    Declined,
    Other(String),
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Failure {
        Failure::Other(e.to_string())
    }
}

fn talk(hwnd: isize, id: u64, name: &str, pipe: &File, exe: &Path, config: &SpawnConfig) {
    let post = |outcome: Outcome| {
        if let Ok(mut outcomes) = OUTCOMES.lock() {
            outcomes.push((id, outcome));
        }
        // SAFETY: posting to a window handle from another thread is allowed; a dead window drops it.
        unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), crate::win::WM_ELEVATED, WPARAM(0), LPARAM(id as isize)).is_ok() }
    };
    let parts = match negotiate(name, pipe, exe, config) {
        Ok(parts) => parts,
        Err(Failure::Declined) => {
            post(Outcome::Declined);
            return;
        }
        Err(Failure::Other(e)) => {
            crate::util::log(&format!("elevate: {e}"));
            post(Outcome::Failed(e));
            return;
        }
    };
    let (reply, answer) = mpsc::channel();
    let posted = post(Outcome::Ready { parts, reply });
    let reply = if posted {
        answer.recv_timeout(ADOPT_WITHIN).unwrap_or(Reply::NotUsed)
    } else {
        // No window to tell: the outcome is not taken by anyone.
        let _ = take(id);
        Reply::NotUsed
    };
    let told = match reply {
        Reply::Adopted => Message::Ok,
        Reply::AdoptFailed => Message::Error { message: "the window could not adopt the session".into() },
        Reply::NotUsed => {
            handoff::kill(&parts);
            Message::Error { message: "the window did not take the session".into() }
        }
    };
    let _ = handoff::write_message(pipe, &told);
}

/// Starts the copy, waits for it to connect, and takes the session it gives.
fn negotiate(name: &str, pipe: &File, exe: &Path, config: &SpawnConfig) -> Result<Parts, Failure> {
    let pid = launch(exe, name)?;
    let helper = Arc::new(Process::open_to_query(pid)?);
    // `accept` waits for ever: a copy that ends before it connects (elevated as another user, whom the pipe is closed to)
    // or never does is wakened by a connection of this process, which is then not the one started.
    let waiting = Arc::new(AtomicBool::new(true));
    {
        let (helper, waiting, name) = (helper.clone(), waiting.clone(), name.to_owned());
        std::thread::spawn(move || {
            let deadline = Instant::now() + CONNECT_WITHIN;
            while waiting.load(Ordering::Acquire) {
                if helper.exited_within(Duration::from_millis(500)) || Instant::now() > deadline {
                    let _ = handoff::connect(&name, Duration::from_secs(1));
                    return;
                }
            }
        });
    }
    let accepted = handoff::accept(pipe);
    waiting.store(false, Ordering::Release);
    accepted?;
    if handoff::peer_pid(pipe, true)? != pid || !same_program(&helper) {
        return Err(Failure::Other("the elevated program did not connect (elevating as another user is not supported here)".into()));
    }
    handoff::write_message(pipe, &Message::Elevate { v: PROTOCOL, config: config.clone() })?;
    match handoff::read_message(&mut BufReader::new(pipe))? {
        Message::Elevated { v: PROTOCOL, parts } => Ok(parts),
        Message::Error { message } => Err(Failure::Other(message)),
        _ => Err(Failure::Other("the elevated program answered what was not asked".into())),
    }
}

/// Starts this program elevated with the pipe, and gives its process id; Windows asks the user.
fn launch(exe: &Path, pipe: &str) -> Result<u32, Failure> {
    let file = HSTRING::from(exe.as_os_str());
    let parameters = HSTRING::from(format!("{FLAG} \"{pipe}\""));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the strings live through the call; the process handle is closed once its id is read.
    unsafe {
        // The thread is a new one: the shell wants an apartment. (Already one: this only counts it again.)
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Err(e) = ShellExecuteExW(&mut info) {
            return Err(if e.code() == HRESULT::from_win32(ERROR_CANCELLED) { Failure::Declined } else { Failure::Other(e.to_string()) });
        }
        if info.hProcess.is_invalid() {
            return Err(Failure::Other("Windows gave no process for the elevated program".into()));
        }
        let pid = GetProcessId(info.hProcess);
        let _ = CloseHandle(info.hProcess);
        Ok(pid)
    }
}

/// Whether `process` runs the same program file as this one.
fn same_program(process: &Process) -> bool {
    let (Some(theirs), Ok(ours)) = (process.image(), std::env::current_exe()) else { return false };
    let text = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().to_lowercase();
    text(&theirs) == text(&ours)
}

// --- the elevated copy -------------------------------------------------------------------------------------------------

/// The elevated copy: gives the window the session it asks for and ends. Whether this process was started so.
pub fn helper_main() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [flag, pipe] = args.as_slice() else { return false };
    if flag != FLAG {
        return false;
    }
    if let Err(e) = helper(pipe) {
        crate::util::log(&format!("elevate (copy): {e}"));
    }
    true
}

fn helper(name: &str) -> std::io::Result<()> {
    let pipe = handoff::connect(name, Duration::from_secs(10))?;
    // The window is a program of lower integrity that may have started this copy by its own pipe: only this program is
    // given an administrator's shell.
    let window = Process::open(handoff::peer_pid(&pipe, false)?)?;
    if !same_program(&window) {
        return Err(std::io::Error::other("the other end of the pipe is not this program"));
    }
    let Message::Elevate { v: PROTOCOL, config } = handoff::read_message(&mut BufReader::new(&pipe))? else {
        return Err(std::io::Error::other("the window asked for what is not this"));
    };
    let (pty, output) = Pty::spawn(&config)?;
    let parts = pty.parts()?;
    let copy = handoff::duplicate_into(&window, &parts)?;
    if let Err(e) = handoff::write_message(&pipe, &Message::Elevated { v: PROTOCOL, parts: copy }) {
        handoff::close_in(&window, &copy);
        return Err(e);
    }
    // Whatever the window answers, the handles it was given are its own to close (it adopts them, or closes them).
    if let Ok(Message::Ok) = handoff::read_message(&mut BufReader::new(&pipe)) {
        // The window holds the session. Closing the pseudoconsole would end it: this process only ends, which closes its
        // own handles.
        std::mem::forget(pty);
        drop(output);
        return Ok(());
    }
    handoff::kill(&parts);
    std::mem::forget((pty, output));
    Err(std::io::Error::other("the window did not take the session"))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, io::Read, path::PathBuf, sync::mpsc};

    use nekominal_pty::conpty::WidthModel;

    use super::*;

    /// The exchange, with this process as the window and as the copy (not elevated: only the elevation is not tried).
    #[test]
    fn the_copy_gives_the_session_it_was_asked_for_and_the_window_adopts_it() {
        let name = format!(r"\\.\pipe\nekominal-elevate-test-{}", std::process::id());
        let pipe = handoff::listen(&name, true).unwrap();
        let copy = std::thread::spawn({
            let name = name.clone();
            move || helper(&name)
        });
        handoff::accept(&pipe).unwrap();
        assert_eq!(handoff::peer_pid(&pipe, true).unwrap(), std::process::id());
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let config = SpawnConfig {
            program: PathBuf::from(system_root).join("System32").join("cmd.exe"),
            args: ["/d", "/c", "echo elevated-ok & ping -n 4 127.0.0.1 >nul"].map(String::from).to_vec(),
            cwd: None,
            env: BTreeMap::new(),
            cols: 80,
            rows: 24,
            width_model: WidthModel::default(),
            inherit_environment: true,
        };
        handoff::write_message(&pipe, &Message::Elevate { v: PROTOCOL, config }).unwrap();
        let Message::Elevated { v: PROTOCOL, parts } = handoff::read_message(&mut BufReader::new(&pipe)).unwrap() else { panic!("the copy gave no session") };
        let (pty, mut output) = Pty::adopt(&parts).expect("the window adopts what the copy gave");
        handoff::write_message(&pipe, &Message::Ok).unwrap();
        copy.join().unwrap().expect("the copy ends well once the window took the session");

        // What the shell wrote before anyone read comes through.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok(n @ 1..) = output.read(&mut buffer) {
                if tx.send(buffer[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let (mut text, deadline) = (String::new(), Instant::now() + Duration::from_secs(15));
        while !text.contains("elevated-ok") {
            let left = deadline.checked_duration_since(Instant::now()).unwrap_or_default();
            text.push_str(&String::from_utf8_lossy(&rx.recv_timeout(left).unwrap_or_else(|_| panic!("timed out; read {text:?}"))));
        }
        drop(pty);
    }

    #[test]
    fn a_program_is_the_same_as_this_one_by_its_file_and_a_window_that_was_not_asked_for_is_refused() {
        let me = Process::open(std::process::id()).unwrap();
        assert!(same_program(&me));
        // The shell of Windows is a program of the user too, and not this one.
        let explorer = nekominal_pty::conpty::process_names().unwrap_or_default().into_iter().find(|(_, _, name)| name.eq_ignore_ascii_case("explorer.exe"));
        if let Some((pid, ..)) = explorer
            && let Ok(other) = Process::open_to_query(pid)
        {
            assert!(!same_program(&other));
        }
    }
}
