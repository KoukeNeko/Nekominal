//! Handing the sessions of the panes from one process to another, for an update that keeps them running (see
//! `docs/updater-design.md`): the Nekominal that is replaced gives its sessions to a copy of itself in the hold mode (the
//! updater), which holds them while Scoop updates the app and gives them to the new Nekominal.
//!
//! A session is its handles ([`Parts`]): the kill-on-close job and the child, the two ends of the ConPTY pipes this side
//! keeps, and the three handles inside the pseudoconsole. A handle is given by duplicating it into the other process
//! ([`duplicate_into`]) and sending its value there; the session lives while any process holds its job. The one that
//! gives must therefore never close its pseudoconsole (that tells the console host to end): it exits, which closes only
//! its own copies.
//!
//! The processes talk over a named pipe open to the user alone, a line of JSON per [`Message`], each carrying the
//! version of the protocol: a side that reads another version uses the text of the panes and starts fresh shells.

use std::{
    fs::File,
    io::{self, BufRead, Write},
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::PathBuf,
    ptr::null_mut,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{CloseHandle, DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, DuplicateHandle, GetLastError, HANDLE, LocalFree},
    Security::{
        Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
    System::{
        JobObjects::TerminateJobObject,
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
        Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_DUP_HANDLE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE},
    },
};

/// The version of the messages. A change that an older side would misread takes a new number.
pub const PROTOCOL: u32 = 1;

/// The handles of one session, as values in one process (which one is said by where they travel), and the child's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parts {
    pub job: u64,
    pub process: u64,
    /// The write end of the input pipe and the read end of the output pipe.
    pub input: u64,
    pub output: u64,
    /// The three handles of the pseudoconsole: its signal pipe, its reference, the console host process.
    pub signal: u64,
    pub reference: u64,
    pub console: u64,
    pub pid: u32,
}

impl Parts {
    fn handles(&self) -> [u64; 7] {
        [self.job, self.process, self.input, self.output, self.signal, self.reference, self.console]
    }

    fn with_handles(&self, [job, process, input, output, signal, reference, console]: [u64; 7]) -> Parts {
        Parts { job, process, input, output, signal, reference, console, pid: self.pid }
    }
}

/// One pane of a window that is handed over.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneState {
    /// The place of its tab among the tabs of the window's session, and its place among the panes of that tab in the
    /// order of its tree (not its id: reading the session numbers the panes afresh across the window).
    pub tab: usize,
    pub pane: u32,
    /// The name of the pane (`NEKOMINAL_SESSION` in its shell), which the files of its agent are named by.
    pub session: String,
    pub title: String,
    /// The size of its terminal, and what it showed: the text, the screen and the modes, as VT to replay.
    pub cols: u16,
    pub rows: u16,
    pub screen: String,
    /// The session itself; none when it could not be handed over (it ended, or the console is not the bundled one).
    pub parts: Option<Parts>,
}

/// One window: its session as the session file keeps it (tabs, layout, placement), and its panes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub session: serde_json::Value,
    pub panes: Vec<PaneState>,
    /// The settings tab, when the window had it open, which the session leaves out. A side that does not know the field
    /// leaves the settings out, as before it was added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<SettingsTab>,
}

/// Where the settings tab was: its place among all the tabs of the window, whether it was in view, and the section of
/// the page it showed (an index of the sections in their order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsTab {
    pub at: usize,
    pub selected: bool,
    pub section: usize,
}

/// What a process hands over: its windows, and which program and version it was, to start again if the new one fails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    pub pid: u32,
    pub exe: String,
    pub version: String,
    pub windows: Vec<WindowState>,
}

/// A line on the pipe of the updater.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Message {
    /// A Nekominal gives its windows; the handles are the updater's. Answered `Ok`, after which it exits.
    Give { v: u32, bundle: Bundle },
    /// The new Nekominal asks for the windows; answered `Windows`, the handles duplicated into it.
    Take { v: u32 },
    Windows { v: u32, windows: Vec<WindowState> },
    /// The new Nekominal took these sessions (by name); the updater ends the others. Answered `Ok`.
    Adopted { sessions: Vec<String> },
    Ok,
    Error { message: String },
}

/// The most a line may hold: a few windows of panes of a megabyte of text each.
const MAX_LINE: u64 = 512 << 20;

pub fn write_message(mut pipe: &File, message: &Message) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    pipe.write_all(&line)
}

pub fn read_message(reader: &mut impl BufRead) -> io::Result<Message> {
    let mut line = Vec::new();
    io::Read::take(reader, MAX_LINE).read_until(b'\n', &mut line)?;
    if line.is_empty() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the other side closed the pipe"));
    }
    serde_json::from_slice(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// --- handles ---------------------------------------------------------------------------------------------------------

fn last_error(context: &str) -> io::Error {
    // SAFETY: plain query.
    let os = io::Error::from_raw_os_error(unsafe { GetLastError() } as i32);
    io::Error::new(os.kind(), format!("{context}: {os}"))
}

/// A process handle, closed when dropped.
pub struct Process(HANDLE);

// SAFETY: a kernel handle, usable from any thread.
unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Process {
    /// Opens process `pid` to duplicate handles into it and wait for it.
    pub fn open(pid: u32) -> io::Result<Process> {
        // SAFETY: plain call; a null result is the failure.
        let handle = unsafe { OpenProcess(PROCESS_DUP_HANDLE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() { Err(last_error(&format!("OpenProcess {pid}"))) } else { Ok(Process(handle)) }
    }

    pub fn raw(&self) -> HANDLE {
        self.0
    }

    /// Whether the process ended within `timeout`.
    pub fn exited_within(&self, timeout: Duration) -> bool {
        use windows_sys::Win32::{Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject};
        // SAFETY: a handle of this struct.
        unsafe { WaitForSingleObject(self.0, timeout.as_millis().min(u32::MAX as u128 - 1) as u32) == WAIT_OBJECT_0 }
    }

    /// The path of its program.
    pub fn image(&self) -> Option<PathBuf> {
        use windows_sys::Win32::System::Threading::{PROCESS_NAME_WIN32, QueryFullProcessImageNameW};
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        // SAFETY: the buffer is as long as the length given.
        let ok = unsafe { QueryFullProcessImageNameW(self.0, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len) };
        (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize])))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: owned and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// Duplicates the handles of `parts` (values of this process) into `target`: the values there. The handles here stay.
/// When one cannot be duplicated, those already made in `target` are closed again and nothing is handed.
pub fn duplicate_into(target: &Process, parts: &Parts) -> io::Result<Parts> {
    let target = target.raw();
    let mut made = Vec::with_capacity(7);
    for handle in parts.handles() {
        let mut out: HANDLE = null_mut();
        // SAFETY: a handle of this process into a process opened with PROCESS_DUP_HANDLE.
        let ok = unsafe { DuplicateHandle(GetCurrentProcess(), handle as HANDLE, target, &mut out, 0, 0, DUPLICATE_SAME_ACCESS) };
        if ok == 0 {
            let e = last_error("DuplicateHandle");
            for &value in &made {
                close_remote(target, value);
            }
            return Err(e);
        }
        made.push(out as u64);
    }
    Ok(parts.with_handles(made.try_into().expect("seven handles")))
}

/// Closes a handle of another process (one duplicated into it that it never got).
fn close_remote(process: HANDLE, value: u64) {
    // SAFETY: DUPLICATE_CLOSE_SOURCE with no target closes the handle in `process`, which was opened with DUP_HANDLE.
    unsafe { DuplicateHandle(process, value as HANDLE, null_mut(), null_mut(), 0, 0, DUPLICATE_CLOSE_SOURCE) };
}

/// Closes the handles of `parts` that were duplicated into `process` and never used there.
pub fn close_in(process: &Process, parts: &Parts) {
    let process = process.raw();
    for value in parts.handles() {
        close_remote(process, value);
    }
}

/// Closes the handles of `parts` in this process. The session ends when no process holds its job any more.
pub fn close(parts: &Parts) {
    for value in parts.handles() {
        // SAFETY: handles this process owns, closed once.
        unsafe { CloseHandle(value as HANDLE) };
    }
}

/// Ends the session of `parts` (its job: the child and all it started) and closes the handles: a session nobody took.
pub fn kill(parts: &Parts) {
    // SAFETY: a job handle this process owns.
    unsafe { TerminateJobObject(parts.job as HANDLE, 1) };
    close(parts);
}

// --- the pipe --------------------------------------------------------------------------------------------------------

/// The SID of the user of this process, as text.
pub fn user_sid() -> io::Result<String> {
    // SAFETY: the token is closed below; the buffer is as large as the first call says and aligned for TOKEN_USER; the
    // string the conversion allocates is freed after it is copied.
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(last_error("OpenProcessToken"));
        }
        let mut size = 0u32;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let read = GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), size, &mut size);
        CloseHandle(token);
        if read == 0 {
            return Err(last_error("GetTokenInformation"));
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text: *mut u16 = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(last_error("ConvertSidToStringSidW"));
        }
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        Ok(sid)
    }
}

/// The pipe of the updater with process id `pid`, for the user `sid`.
pub fn pipe_name(sid: &str, pid: u32) -> String {
    format!(r"\\.\pipe\nekominal-update-{sid}-{pid}")
}

/// A pipe instance of `name` open to the user alone (the descriptor of the automation API), waiting for a client. The
/// first one claims the name: a process that made it first (to listen in) makes this fail.
pub fn listen(name: &str, first: bool) -> io::Result<File> {
    let sid = user_sid()?;
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})").encode_utf16().chain(Some(0)).collect();
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the descriptor the conversion allocates is freed after the call that uses it; the handle is owned by the File.
    unsafe {
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, null_mut()) == 0 {
            return Err(last_error("security descriptor"));
        }
        let attributes = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        let pipe = CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 },
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            1 << 16,
            1 << 16,
            0,
            &attributes,
        );
        LocalFree(descriptor);
        if pipe == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(last_error(&format!("pipe {name}")));
        }
        Ok(File::from_raw_handle(pipe))
    }
}

/// Waits for a client on an instance made by [`listen`].
pub fn accept(pipe: &File) -> io::Result<()> {
    const ERROR_PIPE_CONNECTED: u32 = 535;
    // SAFETY: a pipe instance in blocking mode.
    if unsafe { ConnectNamedPipe(pipe.as_raw_handle(), null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
        Ok(())
    } else {
        Err(last_error("ConnectNamedPipe"))
    }
}

/// Opens the pipe `name`, waiting up to `timeout` for it to be made or for an instance to be free.
pub fn connect(name: &str, timeout: Duration) -> io::Result<File> {
    let deadline = Instant::now() + timeout;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(name) {
            Ok(pipe) => return Ok(pipe),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// The process at the other end of a pipe: the client of an instance this side serves, or the server of one it opened.
pub fn peer_pid(pipe: &File, server_side: bool) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: a pipe handle of the File.
    let ok = unsafe {
        if server_side { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) } else { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) }
    };
    if ok == 0 { Err(last_error("the process of the pipe")) } else { Ok(pid) }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn parts(base: u64) -> Parts {
        Parts { job: base, process: base + 1, input: base + 2, output: base + 3, signal: base + 4, reference: base + 5, console: base + 6, pid: 42 }
    }

    #[test]
    fn a_message_is_one_line_of_json_that_reads_back_the_same() {
        let bundle = Bundle {
            pid: 7,
            exe: r"C:\Users\a\scoop\apps\nekominal\current\Nekominal.exe".into(),
            version: "0.2.3".into(),
            windows: vec![WindowState { settings: None,
                session: serde_json::json!({ "version": 1, "tabs": [] }),
                panes: vec![PaneState {
                    tab: 0,
                    pane: 1,
                    session: "7-1".into(),
                    title: "pwsh".into(),
                    cols: 80,
                    rows: 24,
                    screen: "\x1b[0;31mred\x1b[0m\r\nline\n".into(),
                    parts: Some(parts(100)),
                }],
            }],
        };
        for message in [
            Message::Give { v: PROTOCOL, bundle: bundle.clone() },
            Message::Take { v: PROTOCOL },
            Message::Windows { v: PROTOCOL, windows: bundle.windows.clone() },
            Message::Adopted { sessions: vec!["7-1".into()] },
            Message::Ok,
            Message::Error { message: "no".into() },
        ] {
            let line = serde_json::to_vec(&message).unwrap();
            assert!(!line.contains(&b'\n'), "a line break in the text is escaped, so a message is one line");
            let mut reader = io::BufReader::new(&line[..]);
            assert_eq!(read_message(&mut reader).unwrap(), message);
        }
        let mut empty = io::BufReader::new(&b""[..]);
        assert_eq!(read_message(&mut empty).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn the_settings_tab_of_a_window_reads_back_and_a_window_of_an_older_version_has_none() {
        let older: WindowState = serde_json::from_str(r#"{ "session": {}, "panes": [] }"#).unwrap();
        assert_eq!(older.settings, None);
        let window = WindowState { session: serde_json::json!({}), panes: Vec::new(), settings: Some(SettingsTab { at: 1, selected: true, section: 7 }) };
        let line = serde_json::to_string(&window).unwrap();
        assert_eq!(serde_json::from_str::<WindowState>(&line).unwrap(), window);
        let none = serde_json::to_string(&WindowState { settings: None, ..window }).unwrap();
        assert!(!none.contains("settings"), "a window without them says nothing an older version would trip on: {none}");
    }

    #[test]
    fn a_message_names_its_kind_and_version_so_another_version_can_tell() {
        let line = serde_json::to_string(&Message::Take { v: PROTOCOL }).unwrap();
        assert_eq!(line, r#"{"kind":"take","v":1}"#);
        let newer: Message = serde_json::from_str(r#"{"kind":"windows","v":99,"windows":[],"later":"ignored"}"#).unwrap();
        assert_eq!(newer, Message::Windows { v: 99, windows: vec![] }, "a member it does not know is skipped, the version read");
        assert!(serde_json::from_str::<Message>(r#"{"kind":"teleport"}"#).is_err(), "a kind it does not know is an error");
    }

    #[test]
    fn parts_keep_their_order_and_the_pid_when_their_handles_change() {
        let before = parts(10);
        let after = before.with_handles([1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(after.handles(), [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!((after.pid, before.handles()), (42, [10, 11, 12, 13, 14, 15, 16]));
    }

    #[test]
    fn handles_duplicated_into_a_process_are_closed_there_when_one_of_them_fails() {
        use windows_sys::Win32::System::Threading::CreateEventW;
        use std::ptr::null;
        // SAFETY: plain calls; the events are closed below.
        let event = || unsafe { CreateEventW(null(), 1, 0, null()) } as u64;
        let mut good = parts(0).with_handles([event(), event(), event(), event(), event(), event(), event()]);
        let target = Process::open(std::process::id()).unwrap();
        let copy = duplicate_into(&target, &good).expect("duplicated into this very process");
        assert!(copy.handles().iter().zip(good.handles()).all(|(a, b)| *a != b), "new values");
        close(&copy);
        good.console = 0x7777_7770;
        assert!(duplicate_into(&target, &good).is_err(), "a value that is no handle fails the whole");
        good.console = event();
        close(&good);
    }

    #[test]
    fn the_pipe_is_open_to_this_user_and_names_the_process_at_each_end() {
        let name = format!(r"\\.\pipe\nekominal-update-test-{}-{}", std::process::id(), line!());
        let server = listen(&name, true).unwrap();
        assert!(listen(&name, true).is_err(), "the name is taken once");
        let waiter = std::thread::spawn(move || {
            accept(&server).unwrap();
            let pid = peer_pid(&server, true).unwrap();
            let mut reader = io::BufReader::new(&server);
            let message = read_message(&mut reader).unwrap();
            write_message(&server, &Message::Ok).unwrap();
            (pid, message)
        });
        let client = connect(&name, Duration::from_secs(5)).unwrap();
        assert_eq!(peer_pid(&client, false).unwrap(), std::process::id());
        write_message(&client, &Message::Take { v: PROTOCOL }).unwrap();
        assert_eq!(read_message(&mut io::BufReader::new(&client)).unwrap(), Message::Ok);
        assert_eq!(waiter.join().unwrap(), (std::process::id(), Message::Take { v: PROTOCOL }));
        assert!(user_sid().unwrap().starts_with("S-1-"));
        assert!(pipe_name("S-1-5-21-1", 9).ends_with("nekominal-update-S-1-5-21-1-9"));
    }
}
