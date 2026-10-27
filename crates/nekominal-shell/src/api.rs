//! The automation API: a named pipe a script or an agent drives the terminal through, as `wezterm cli` or `kitten @` do.
//! One pipe for the process (all its windows), `\\.\pipe\nekominal-<user SID>-<pid>`, open to the current user only and to
//! no other machine. Its name is written to `%LOCALAPPDATA%\Nekominal\api\<pid>` for `nekominal cli` to find, and handed
//! to every pane as `NEKOMINAL_PIPE`, with the pane's own id as `NEKOMINAL_PANE`, so a program talks to its own instance.
//!
//! The protocol is a line of JSON per request (`{"id": 1, "cmd": "list"}`) and a line per answer (`{"id": 1, "ok": true,
//! "result": …}` or `{"id": 1, "ok": false, "error": "…"}`). `subscribe` answers once and then streams a line per event
//! (`{"event": "agent-state", "pane": "…", …}`) until the client goes.
//!
//! The pipe has a thread of its own and a thread per client. A request is put in a queue and a message posted to a
//! message-only window of the UI thread (`WM_API`), which answers it from the windows (`app::automation`) and sends the
//! answer back over a channel. A window that is busy (the message came while one of its handlers runs, inside a modal
//! loop) is not borrowed: the request waits and is tried again a moment later.
//!
//! `send-text` is a paste, not typing: bracketed when the program asked for it, and it goes through the same check as a
//! paste of the clipboard. Text that check would ask about (several lines into a program without bracketed paste, or more
//! than the size of the setting) is not sent: the window asks the user, as for a paste, and the answer says `asked`. A
//! program of this user could already type into the window, but the question is there for text nobody looked at, which is
//! what a script sends too; `paste.warnMultiline` turns it off for both.

use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    os::windows::io::FromRawHandle,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::Duration,
};

use serde_json::{Value, json};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, HWND, LPARAM, LRESULT, LocalFree, WPARAM},
        Security::{
            Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
            GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX},
        System::{
            LibraryLoader::GetModuleHandleW,
            Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT},
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, HWND_MESSAGE, KillTimer, PostMessageW, RegisterClassW, SetTimer, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
            WM_TIMER, WNDCLASSW,
        },
    },
    core::{HSTRING, PWSTR, w},
};

/// The variables a pane gets: the pipe of its instance, and its own id there.
pub const PIPE_VAR: &str = "NEKOMINAL_PIPE";
pub const PANE_VAR: &str = "NEKOMINAL_PANE";
/// The longest line a client may send: a paste of a few megabytes, not an endless stream.
const MAX_LINE: u64 = 8 << 20;
/// How long a client waits for the windows to answer: a window held in a modal loop (a drag of its frame) answers after it.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// Posted to the message-only window when a request is queued; the timer tries again what a busy window could not take.
const WM_API: u32 = WM_APP + 0x60;
const RETRY_TIMER: usize = 1;

// --- the protocol --------------------------------------------------------------------------------------------------

/// What a subscriber can follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    /// The agent of a pane changed state (working, blocked, done, error, idle, or none).
    AgentState,
    /// The shell marked the end of a command (OSC 133 `D`), with its exit code when it said one.
    CommandFinished,
    /// A pane had output; its text is for `get-text` to read.
    Output,
}

impl Topic {
    pub const ALL: [Topic; 3] = [Topic::AgentState, Topic::CommandFinished, Topic::Output];

    pub fn name(self) -> &'static str {
        match self {
            Topic::AgentState => "agent-state",
            Topic::CommandFinished => "command-finished",
            Topic::Output => "output",
        }
    }

    fn parse(name: &str) -> Option<Topic> {
        Topic::ALL.into_iter().find(|topic| topic.name() == name)
    }
}

/// Where `split` puts the new pane: beside the pane or below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Right,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    List,
    SendText { pane: String, text: String },
    /// The last `lines` lines of the pane, the screen and the scrollback above it; the screen's rows when none.
    GetText { pane: String, lines: Option<usize> },
    NewTab { profile: Option<String>, cwd: Option<String>, command: Vec<String> },
    Split { pane: String, direction: Direction, cwd: Option<String>, command: Vec<String> },
    Focus { pane: String },
    Close { pane: String },
    /// The name of the pane's tab; empty gives the tab its automatic title back.
    SetTitle { pane: String, title: String },
    /// A notification of Windows; a click on it shows the tab of `pane`, or the selected tab of the front window.
    Notify { title: String, body: String, pane: Option<String> },
    Subscribe { topics: Vec<Topic> },
}

/// A string member: missing (or null) is `None`, anything but a string an error.
fn text(request: &Value, key: &str) -> Result<Option<String>, String> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("\"{key}\" must be a string")),
    }
}

fn required(request: &Value, key: &str) -> Result<String, String> {
    text(request, key)?.ok_or_else(|| format!("\"{key}\" is missing"))
}

/// A pane id: what `list` gives and `NEKOMINAL_PANE` holds; never empty.
fn pane(request: &Value) -> Result<String, String> {
    Some(required(request, "pane")?).filter(|pane| !pane.is_empty()).ok_or_else(|| "\"pane\" is empty".to_owned())
}

/// The program and its arguments, as an array of strings; none runs the profile's shell.
fn command(request: &Value) -> Result<Vec<String>, String> {
    match request.get("command") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(words)) => words
            .iter()
            .map(|word| word.as_str().map(str::to_owned).ok_or_else(|| "\"command\" must be an array of strings".to_owned()))
            .collect(),
        Some(_) => Err("\"command\" must be an array of strings".to_owned()),
    }
}

/// One line of a client: the id to answer with (any JSON, null when there is none) and what it asks, or why it is not a
/// request.
pub fn parse(line: &str) -> (Value, Result<Request, String>) {
    let request: Value = match serde_json::from_str(line) {
        Ok(request @ Value::Object(_)) => request,
        Ok(_) => return (Value::Null, Err("a request is a JSON object".to_owned())),
        Err(e) => return (Value::Null, Err(format!("not JSON: {e}"))),
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    (id, request_of(&request))
}

fn request_of(request: &Value) -> Result<Request, String> {
    let cmd = required(request, "cmd")?;
    Ok(match cmd.as_str() {
        "list" => Request::List,
        "send-text" => {
            let text = required(request, "text")?;
            if text.is_empty() {
                return Err("\"text\" is empty".to_owned());
            }
            Request::SendText { pane: pane(request)?, text }
        }
        "get-text" => {
            let lines = match request.get("lines") {
                None | Some(Value::Null) => None,
                Some(lines) => Some(lines.as_u64().filter(|n| *n > 0).ok_or("\"lines\" must be a whole number above 0")? as usize),
            };
            Request::GetText { pane: pane(request)?, lines }
        }
        "new-tab" => Request::NewTab { profile: text(request, "profile")?, cwd: text(request, "cwd")?, command: command(request)? },
        "split" => {
            let direction = match text(request, "direction")?.as_deref() {
                None | Some("right") => Direction::Right,
                Some("down") => Direction::Down,
                Some(other) => return Err(format!("\"direction\" is right or down, not {other}")),
            };
            Request::Split { pane: pane(request)?, direction, cwd: text(request, "cwd")?, command: command(request)? }
        }
        "focus" => Request::Focus { pane: pane(request)? },
        "close" => Request::Close { pane: pane(request)? },
        "set-title" => Request::SetTitle { pane: pane(request)?, title: required(request, "title")? },
        "notify" => Request::Notify { title: required(request, "title")?, body: text(request, "body")?.unwrap_or_default(), pane: text(request, "pane")? },
        "subscribe" => {
            let topics = match request.get("events") {
                None | Some(Value::Null) => vec![Topic::AgentState, Topic::CommandFinished],
                Some(Value::Array(names)) => names
                    .iter()
                    .map(|name| name.as_str().and_then(Topic::parse).ok_or_else(|| format!("unknown event {name}; the events are agent-state, command-finished and output")))
                    .collect::<Result<_, _>>()?,
                Some(_) => return Err("\"events\" must be an array of names".to_owned()),
            };
            Request::Subscribe { topics }
        }
        other => return Err(format!("unknown command {other}")),
    })
}

/// The line that answers the request `id`.
pub fn reply(id: &Value, answer: &Result<Value, String>) -> String {
    match answer {
        Ok(result) => json!({ "id": id, "ok": true, "result": result }),
        Err(error) => json!({ "id": id, "ok": false, "error": error }),
    }
    .to_string()
}

/// The line of an event about `pane`, with the members of `more` (an object).
pub fn event_line(topic: Topic, pane: &str, more: Value) -> String {
    let mut line = json!({ "event": topic.name(), "pane": pane });
    if let (Some(line), Value::Object(more)) = (line.as_object_mut(), more) {
        line.extend(more);
    }
    line.to_string()
}

// --- events --------------------------------------------------------------------------------------------------------

type Subscriber = (Vec<Topic>, mpsc::Sender<String>);
static SUBSCRIBERS: Mutex<Vec<Subscriber>> = Mutex::new(Vec::new());

fn subscribe(topics: Vec<Topic>) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    SUBSCRIBERS.lock().unwrap_or_else(|e| e.into_inner()).push((topics, sender));
    receiver
}

/// Someone follows `topic`: the window works out the event only then.
pub fn wants(topic: Topic) -> bool {
    SUBSCRIBERS.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(topics, _)| topics.contains(&topic))
}

/// Sends an event to everyone who follows its topic; a subscriber whose client went is dropped.
pub fn publish(topic: Topic, pane: &str, more: Value) {
    let line = event_line(topic, pane, more);
    SUBSCRIBERS.lock().unwrap_or_else(|e| e.into_inner()).retain(|(topics, sender)| !topics.contains(&topic) || sender.send(line.clone()).is_ok());
}

// --- the security of the pipe --------------------------------------------------------------------------------------

/// The SID of the user this process runs as, as text (`S-1-5-21-…`).
pub fn user_sid() -> Result<String, String> {
    // SAFETY: the token is closed below; the buffer is as large as the first call says and aligned for TOKEN_USER; the
    // string the conversion allocates is freed after it is copied.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(|e| format!("token: {e}"))?;
        let mut size = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let read = GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr().cast()), size, &mut size);
        let _ = CloseHandle(token);
        read.map_err(|e| format!("token user: {e}"))?;
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(|e| format!("sid: {e}"))?;
        let sid = text.to_string().map_err(|e| e.to_string());
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}

/// The security descriptor of the pipe, in SDDL: owned by the user, and a protected DACL (nothing inherited) with one
/// entry, all access for the user. Administrators and other users of the machine get nothing; the system's default would
/// let any of them read it.
pub fn sddl(sid: &str) -> String {
    format!("O:{sid}D:P(A;;GA;;;{sid})")
}

/// A security descriptor made from SDDL, freed when dropped.
pub struct Descriptor(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is memory of its own that nothing else points to; it is only read once made.
unsafe impl Send for Descriptor {}

impl Descriptor {
    pub fn new(sddl: &str) -> Result<Descriptor, String> {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: the system allocates the descriptor, freed by Drop.
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(sddl), SDDL_REVISION_1, &mut descriptor, None) }
            .map_err(|e| format!("security descriptor: {e}"))?;
        Ok(Descriptor(descriptor))
    }

    /// The descriptor as text again, for a test to look at what Windows made of it.
    #[cfg(test)]
    fn to_sddl(&self) -> String {
        use windows::Win32::Security::{
            Authorization::ConvertSecurityDescriptorToStringSecurityDescriptorW, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        };
        let mut text = PWSTR::null();
        // SAFETY: a descriptor of this struct; the string the system allocates is freed after it is copied.
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(self.0, SDDL_REVISION_1, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, &mut text, None).unwrap();
            let sddl = text.to_string().unwrap();
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            sddl
        }
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: allocated with LocalAlloc by the conversion.
        unsafe { let _ = LocalFree(Some(HLOCAL(self.0.0))); }
    }
}

/// The pipe of this process for the user `sid`.
pub fn pipe_name(sid: &str, pid: u32) -> String {
    format!(r"\\.\pipe\nekominal-{sid}-{pid}")
}

// --- the server ----------------------------------------------------------------------------------------------------

/// What answers a request (`subscribe` is the server's own).
pub type Handler = Arc<dyn Fn(Request) -> Result<Value, String> + Send + Sync>;

/// One instance of the pipe, waiting for a client. The first one also claims the name: another process that made a pipe
/// of this name first (to listen in) makes this fail instead of joining it.
fn instance(name: &str, descriptor: &Descriptor, first: bool) -> Result<File, String> {
    let attributes = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor.0.0, bInheritHandle: false.into() };
    let first = if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { FILE_FLAGS_AND_ATTRIBUTES(0) };
    // SAFETY: the attributes and the descriptor live through the call; the handle is owned by the File made of it.
    unsafe {
        let pipe = CreateNamedPipeW(
            &HSTRING::from(name),
            PIPE_ACCESS_DUPLEX | first,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            Some(&attributes),
        );
        if pipe.is_invalid() {
            return Err(format!("pipe {name}: {}", windows::core::Error::from_thread()));
        }
        Ok(File::from_raw_handle(pipe.0))
    }
}

/// Serves the pipe `name` on a thread of its own: each client gets a thread, its requests answered by `handler`.
/// Fails when the pipe cannot be made (the name is taken).
pub fn serve(name: &str, descriptor: Descriptor, handler: Handler) -> Result<(), String> {
    let mut waiting = instance(name, &descriptor, true)?;
    let name = name.to_owned();
    std::thread::Builder::new()
        .name("api pipe".into())
        .spawn(move || loop {
            // SAFETY: the handle of the File, a pipe instance in blocking mode.
            let connected = unsafe { ConnectNamedPipe(HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&waiting)), None) };
            // A client that came between the making of the instance and this call is connected too; after any other
            // failure (a client that came and went) the instance is closed and a new one waits.
            let ok = connected.as_ref().is_ok() || connected.as_ref().is_err_and(|e| e.code() == ERROR_PIPE_CONNECTED.to_hresult());
            let next = loop {
                match instance(&name, &descriptor, false) {
                    Ok(next) => break next,
                    Err(e) => {
                        crate::util::log(&format!("api: {e}"));
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            };
            let client = std::mem::replace(&mut waiting, next);
            if ok {
                let handler = handler.clone();
                let _ = std::thread::Builder::new().name("api client".into()).spawn(move || serve_client(&client, &handler));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Answers the lines of one client until it goes; after `subscribe` only events go to it.
fn serve_client(pipe: &File, handler: &Handler) {
    let mut reader = BufReader::new(pipe);
    let mut writer = pipe;
    loop {
        let mut bytes = Vec::new();
        match reader.by_ref().take(MAX_LINE).read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if !bytes.ends_with(b"\n") && bytes.len() as u64 >= MAX_LINE {
            let _ = writeln!(writer, "{}", reply(&Value::Null, &Err(format!("a request is at most {MAX_LINE} bytes"))));
            return;
        }
        // A line that is not UTF-8 is answered like any other bad line; the client stays.
        let Ok(line) = String::from_utf8(bytes) else {
            if writeln!(writer, "{}", reply(&Value::Null, &Err("not UTF-8".to_owned()))).is_err() {
                return;
            }
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let (id, request) = parse(&line);
        let answer = match request {
            Ok(Request::Subscribe { topics }) => {
                let events = subscribe(topics.clone());
                let names: Vec<&str> = topics.iter().map(|topic| topic.name()).collect();
                if writeln!(writer, "{}", reply(&id, &Ok(json!({ "subscribed": names })))).is_err() {
                    return;
                }
                // Until the client goes: the write that finds it gone ends the thread and, with it, the subscription.
                for event in events {
                    if writeln!(writer, "{event}").is_err() {
                        return;
                    }
                }
                return;
            }
            Ok(request) => handler(request),
            Err(e) => Err(e),
        };
        if writeln!(writer, "{}", reply(&id, &answer)).is_err() {
            return;
        }
    }
}

// --- the UI thread -------------------------------------------------------------------------------------------------

type Job = (Request, mpsc::Sender<Result<Value, String>>);
static QUEUE: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());
/// The message-only window of the UI thread, the pipe's name, and the file that tells `nekominal cli` about it.
static WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static NAME: OnceLock<String> = OnceLock::new();

/// Hands a request to the UI thread and waits for its answer.
fn to_ui(request: Request) -> Result<Value, String> {
    let (sender, answer) = mpsc::channel();
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push_back((request, sender));
    let window = WINDOW.load(std::sync::atomic::Ordering::Acquire);
    // SAFETY: posting to a window from another thread is allowed.
    unsafe { PostMessageW(Some(HWND(window as *mut _)), WM_API, WPARAM(0), LPARAM(0)) }.map_err(|e| e.to_string())?;
    answer.recv_timeout(ANSWER_TIMEOUT).map_err(|_| "the window did not answer in time".to_owned())?
}

/// Answers what is queued; false when a window was busy and something waits for the next try.
fn drain() -> bool {
    let jobs: Vec<Job> = QUEUE.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect();
    let windows = crate::win::apps();
    let mut waiting = VecDeque::new();
    for (request, sender) in jobs {
        match crate::app::automation::answer(&windows, &request) {
            Some(answer) => {
                let _ = sender.send(answer);
            }
            None => waiting.push_back((request, sender)),
        }
    }
    let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    let done = waiting.is_empty();
    // What came in meanwhile goes after what waited.
    waiting.extend(queue.drain(..));
    *queue = waiting;
    done
}

unsafe extern "system" fn api_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_API || (msg == WM_TIMER && wparam.0 == RETRY_TIMER) {
        // SAFETY: a timer of this window, from its own thread.
        unsafe {
            let _ = KillTimer(Some(hwnd), RETRY_TIMER);
            if !drain() {
                SetTimer(Some(hwnd), RETRY_TIMER, 50, None);
            }
        }
        return LRESULT(0);
    }
    // SAFETY: default handling of every other message.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The file in which a running instance writes its pipe's name.
fn instances_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Nekominal").join("api"))
}

/// The API while it runs: dropped (the process ends), its file goes.
pub struct Running(Option<PathBuf>);

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(file) = &self.0 {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// Starts the API, from the UI thread before its first window, so that the first panes get its variables. What fails
/// goes to the log, and the terminal works without it.
pub fn start() -> Running {
    match try_start() {
        Ok(file) => Running(file),
        Err(e) => {
            crate::util::log(&format!("api: {e}"));
            Running(None)
        }
    }
}

fn try_start() -> Result<Option<PathBuf>, String> {
    let sid = user_sid()?;
    let name = pipe_name(&sid, std::process::id());
    // SAFETY: a class and a message-only window of this thread.
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW { lpfnWndProc: Some(api_proc), hInstance: module.into(), lpszClassName: w!("NekominalApi"), ..Default::default() };
        RegisterClassW(&class);
        let window = CreateWindowExW(WINDOW_EX_STYLE(0), w!("NekominalApi"), None, WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(module.into()), None)
            .map_err(|e| format!("window: {e}"))?;
        WINDOW.store(window.0 as isize, std::sync::atomic::Ordering::Release);
    }
    serve(&name, Descriptor::new(&sddl(&sid))?, Arc::new(to_ui))?;
    let _ = NAME.set(name.clone());
    let file = instances_dir().map(|dir| dir.join(std::process::id().to_string()));
    if let Some(file) = &file
        && let Err(e) = std::fs::create_dir_all(file.parent().unwrap_or(file)).and_then(|_| std::fs::write(file, &name))
    {
        crate::util::log(&format!("api: {}: {e}", file.display()));
    }
    Ok(file)
}

/// The variables of a new pane: the pipe and the pane's id, while the API runs.
pub fn pane_environment(pane: &str) -> Vec<(String, String)> {
    NAME.get().map(|name| vec![(PIPE_VAR.to_owned(), name.clone()), (PANE_VAR.to_owned(), pane.to_owned())]).unwrap_or_default()
}

// --- the client ----------------------------------------------------------------------------------------------------

/// Opens the pipe `name`, waiting a little while every instance is taken (the server makes the next one right away).
pub fn connect(name: &str) -> std::io::Result<File> {
    const ERROR_PIPE_BUSY: i32 = 231;
    // Only a pipe of this computer: any other name (a broken instance file, a slip in --pipe) would open that file and
    // write the request into it.
    if !name.get(..9).is_some_and(|start| start.eq_ignore_ascii_case(r"\\.\pipe\")) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, r"not the name of a pipe (\\.\pipe\…)"));
    }
    for _ in 0..100 {
        match std::fs::OpenOptions::new().read(true).write(true).open(name) {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => std::thread::sleep(Duration::from_millis(20)),
            other => return other,
        }
    }
    Err(std::io::Error::from_raw_os_error(ERROR_PIPE_BUSY))
}

/// The pipe of the instance a command goes to: `NEKOMINAL_PIPE` (the instance of the pane it runs in), else the newest
/// running instance. The file of an instance that is gone (it crashed) is removed on the way.
pub fn connect_default() -> Result<File, String> {
    if let Some(name) = std::env::var(PIPE_VAR).ok().filter(|name| !name.is_empty()) {
        return connect(&name).map_err(|e| format!("{name}: {e}"));
    }
    connect_newest(&instances_dir().ok_or("LOCALAPPDATA is not set")?)
}

/// The newest instance whose file in `dir` names a pipe that opens; the files of instances that are gone, or that name
/// no pipe, are removed on the way.
fn connect_newest(dir: &std::path::Path) -> Result<File, String> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path()))).collect())
        .unwrap_or_default();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, file) in found {
        let Ok(name) = std::fs::read_to_string(&file) else { continue };
        match connect(name.trim()) {
            Ok(pipe) => return Ok(pipe),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidInput) => {
                let _ = std::fs::remove_file(&file);
            }
            Err(_) => {}
        }
    }
    Err("no running Nekominal was found".to_owned())
}

/// Sends one request and prints every line that comes back: one answer, or for `subscribe` the answer and then the events
/// until the instance goes. The exit code: 0 when the answer is ok, 1 when it is not, 2 when it could not be asked.
pub fn run_cli(pipe: Option<&str>, request: &Value) -> i32 {
    let connected = match pipe {
        Some(name) => connect(name).map_err(|e| format!("{name}: {e}")),
        None => connect_default(),
    };
    let pipe = match connected {
        Ok(pipe) => pipe,
        Err(e) => {
            println!("{}", reply(&Value::Null, &Err(e)));
            return 2;
        }
    };
    if writeln!(&pipe, "{request}").is_err() {
        println!("{}", reply(&Value::Null, &Err("the instance closed the pipe".to_owned())));
        return 2;
    }
    let mut code = 2;
    for line in BufReader::new(&pipe).lines() {
        let Ok(line) = line else { break };
        println!("{line}");
        let _ = std::io::stdout().flush();
        if code == 2 {
            code = if serde_json::from_str::<Value>(&line).ok().and_then(|answer| answer.get("ok")?.as_bool()) == Some(true) { 0 } else { 1 };
            if request.get("cmd").and_then(Value::as_str) != Some("subscribe") {
                break;
            }
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Result<Request, String> {
        parse(line).1
    }

    #[test]
    fn every_command_parses_from_its_line_with_its_defaults() {
        assert_eq!(parsed(r#"{"cmd":"list"}"#), Ok(Request::List));
        assert_eq!(parsed(r#"{"cmd":"send-text","pane":"7-1","text":"ls\r"}"#), Ok(Request::SendText { pane: "7-1".into(), text: "ls\r".into() }));
        assert_eq!(parsed(r#"{"cmd":"get-text","pane":"7-1"}"#), Ok(Request::GetText { pane: "7-1".into(), lines: None }));
        assert_eq!(parsed(r#"{"cmd":"get-text","pane":"7-1","lines":200}"#), Ok(Request::GetText { pane: "7-1".into(), lines: Some(200) }));
        assert_eq!(parsed(r#"{"cmd":"new-tab"}"#), Ok(Request::NewTab { profile: None, cwd: None, command: vec![] }));
        assert_eq!(
            parsed(r#"{"cmd":"new-tab","profile":"pwsh","cwd":"D:\\src","command":["cmd","/k","echo hi"]}"#),
            Ok(Request::NewTab { profile: Some("pwsh".into()), cwd: Some(r"D:\src".into()), command: vec!["cmd".into(), "/k".into(), "echo hi".into()] })
        );
        assert_eq!(parsed(r#"{"cmd":"split","pane":"7-1"}"#), Ok(Request::Split { pane: "7-1".into(), direction: Direction::Right, cwd: None, command: vec![] }));
        assert!(matches!(parsed(r#"{"cmd":"split","pane":"7-1","direction":"down"}"#), Ok(Request::Split { direction: Direction::Down, .. })));
        assert_eq!(parsed(r#"{"cmd":"focus","pane":"7-1"}"#), Ok(Request::Focus { pane: "7-1".into() }));
        assert_eq!(parsed(r#"{"cmd":"close","pane":"7-1"}"#), Ok(Request::Close { pane: "7-1".into() }));
        assert_eq!(parsed(r#"{"cmd":"set-title","pane":"7-1","title":""}"#), Ok(Request::SetTitle { pane: "7-1".into(), title: String::new() }));
        assert_eq!(parsed(r#"{"cmd":"notify","title":"Build"}"#), Ok(Request::Notify { title: "Build".into(), body: String::new(), pane: None }));
        assert_eq!(parsed(r#"{"cmd":"subscribe"}"#), Ok(Request::Subscribe { topics: vec![Topic::AgentState, Topic::CommandFinished] }), "output is asked for, not given");
        assert_eq!(parsed(r#"{"cmd":"subscribe","events":["output"]}"#), Ok(Request::Subscribe { topics: vec![Topic::Output] }));
    }

    #[test]
    fn a_request_that_is_not_whole_or_not_right_says_what_is_wrong() {
        assert!(parsed("ls").unwrap_err().starts_with("not JSON"));
        assert_eq!(parsed("[1]").unwrap_err(), "a request is a JSON object");
        assert_eq!(parsed(r#"{"id":1}"#).unwrap_err(), r#""cmd" is missing"#);
        assert_eq!(parsed(r#"{"cmd":"reboot"}"#).unwrap_err(), "unknown command reboot");
        assert_eq!(parsed(r#"{"cmd":"send-text","text":"x"}"#).unwrap_err(), r#""pane" is missing"#);
        assert_eq!(parsed(r#"{"cmd":"send-text","pane":"","text":"x"}"#).unwrap_err(), r#""pane" is empty"#);
        assert_eq!(parsed(r#"{"cmd":"send-text","pane":"1-1","text":""}"#).unwrap_err(), r#""text" is empty"#);
        assert_eq!(parsed(r#"{"cmd":"send-text","pane":1,"text":"x"}"#).unwrap_err(), r#""pane" must be a string"#);
        assert_eq!(parsed(r#"{"cmd":"get-text","pane":"1-1","lines":0}"#).unwrap_err(), r#""lines" must be a whole number above 0"#);
        assert_eq!(parsed(r#"{"cmd":"get-text","pane":"1-1","lines":-3}"#).unwrap_err(), r#""lines" must be a whole number above 0"#);
        assert_eq!(parsed(r#"{"cmd":"new-tab","command":"cmd /k"}"#).unwrap_err(), r#""command" must be an array of strings"#);
        assert_eq!(parsed(r#"{"cmd":"new-tab","command":["cmd",1]}"#).unwrap_err(), r#""command" must be an array of strings"#);
        assert_eq!(parsed(r#"{"cmd":"split","pane":"1-1","direction":"up"}"#).unwrap_err(), "\"direction\" is right or down, not up");
        assert!(parsed(r#"{"cmd":"subscribe","events":["keys"]}"#).unwrap_err().starts_with("unknown event \"keys\""));
    }

    #[test]
    fn the_answer_carries_the_id_of_the_request_whatever_json_it_is() {
        let (id, _) = parse(r#"{"id":"a-7","cmd":"list"}"#);
        assert_eq!(reply(&id, &Ok(json!({ "windows": [] }))), r#"{"id":"a-7","ok":true,"result":{"windows":[]}}"#);
        assert_eq!(reply(&json!(3), &Err("no pane 9-9".into())), r#"{"error":"no pane 9-9","id":3,"ok":false}"#);
        assert_eq!(parse(r#"{"cmd":"list"}"#).0, Value::Null, "no id answers with null");
        assert_eq!(parse("{").0, Value::Null, "nor does a line that is not JSON");
        assert_eq!(event_line(Topic::CommandFinished, "7-2", json!({ "exit": 1 })), r#"{"event":"command-finished","exit":1,"pane":"7-2"}"#);
    }

    #[test]
    fn the_pipe_is_named_by_the_user_and_the_process_and_open_to_that_user_alone() {
        let sid = user_sid().expect("the SID of this process's user");
        assert!(sid.starts_with("S-1-"), "{sid}");
        assert_eq!(pipe_name(&sid, 42), format!(r"\\.\pipe\nekominal-{sid}-42"));
        assert_eq!(sddl(&sid), format!("O:{sid}D:P(A;;GA;;;{sid})"));
        // What Windows made of it: the owner, a protected DACL and one entry, which allows, for the user.
        let made = Descriptor::new(&sddl(&sid)).unwrap().to_sddl();
        let dacl = made.split_once("D:").map(|(_, dacl)| dacl).unwrap();
        assert!(made.starts_with(&format!("O:{sid}")), "{made}");
        assert!(dacl.starts_with("P("), "the DACL inherits nothing: {made}");
        assert_eq!(dacl.matches('(').count(), 1, "one entry: {made}");
        assert!(dacl.starts_with("P(A;;") && dacl.ends_with(&format!(";;;{sid})")), "{made}");
        assert!(Descriptor::new("not sddl").is_err());
    }

    #[test]
    fn what_a_request_does_not_need_is_ignored_and_null_is_missing() {
        assert_eq!(parsed(r#"{"cmd":"focus","pane":"1-1","extra":[1,2],"text":5}"#), Ok(Request::Focus { pane: "1-1".into() }), "members of other commands are ignored");
        assert_eq!(parsed(r#"{"cmd":"get-text","pane":"1-1","lines":null}"#), Ok(Request::GetText { pane: "1-1".into(), lines: None }));
        assert_eq!(parsed(r#"{"cmd":"notify","title":"T","body":null,"pane":"2-1"}"#), Ok(Request::Notify { title: "T".into(), body: String::new(), pane: Some("2-1".into()) }));
        assert_eq!(parsed(r#"{"cmd":"new-tab","command":[]}"#), Ok(Request::NewTab { profile: None, cwd: None, command: vec![] }));
        assert_eq!(parsed(r#"{"cmd":"send-text","pane":"1-1","text":" "}"#), Ok(Request::SendText { pane: "1-1".into(), text: " ".into() }), "a space is text");
        assert_eq!(parsed(r#"{"cmd":"list","cmd":"close","pane":"1-1"}"#), Ok(Request::Close { pane: "1-1".into() }), "of a member given twice, the last counts");
        assert_eq!(parsed(" {\"cmd\":\"list\"}\r\n"), Ok(Request::List), "spaces and CR LF around the object");
        let (id, _) = parse(r#"{"id":{"n":[1,"a"]},"cmd":"list"}"#);
        assert_eq!(id, json!({ "n": [1, "a"] }), "an id is any JSON, given back as it was");
    }

    #[test]
    fn a_member_of_the_wrong_type_or_value_is_named() {
        assert_eq!(parsed(r#"{"cmd":7}"#).unwrap_err(), r#""cmd" must be a string"#);
        assert_eq!(parsed(r#"{"cmd":"get-text","pane":"1-1","lines":2.5}"#).unwrap_err(), r#""lines" must be a whole number above 0"#);
        assert_eq!(parsed(r#"{"cmd":"get-text","pane":"1-1","lines":"20"}"#).unwrap_err(), r#""lines" must be a whole number above 0"#);
        assert_eq!(parsed(r#"{"cmd":"get-text","pane":"1-1","lines":18446744073709551615}"#), Ok(Request::GetText { pane: "1-1".into(), lines: Some(usize::MAX) }), "more lines than there are is all of them");
        assert_eq!(parsed(r#"{"cmd":"set-title","pane":"1-1"}"#).unwrap_err(), r#""title" is missing"#);
        assert_eq!(parsed(r#"{"cmd":"notify","body":"b"}"#).unwrap_err(), r#""title" is missing"#);
        assert_eq!(parsed(r#"{"cmd":"notify","title":"T","pane":3}"#).unwrap_err(), r#""pane" must be a string"#);
        assert_eq!(parsed(r#"{"cmd":"split","pane":"1-1","direction":1}"#).unwrap_err(), r#""direction" must be a string"#);
        assert_eq!(parsed(r#"{"cmd":"new-tab","cwd":["D:\\"]}"#).unwrap_err(), r#""cwd" must be a string"#);
        assert_eq!(parsed(r#"{"cmd":"subscribe","events":"output"}"#).unwrap_err(), r#""events" must be an array of names"#);
        assert_eq!(parsed(r#"{"cmd":"subscribe","events":[1]}"#).unwrap_err().split(';').next(), Some("unknown event 1"));
        assert!(parsed("").unwrap_err().starts_with("not JSON"));
        assert!(parsed("\u{feff}{\"cmd\":\"list\"}").unwrap_err().starts_with("not JSON"), "a byte order mark is not JSON");
        assert_eq!(parsed("null").unwrap_err(), "a request is a JSON object");
    }

    /// A server of a pipe of its own, answering `list` and echoing the rest as an error.
    fn served() -> String {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let name = format!(r"\\.\pipe\nekominal-test-{}-n{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let handler: Handler = Arc::new(|request| match request {
            Request::List => Ok(json!("listed")),
            other => Err(format!("echo: {other:?}")),
        });
        serve(&name, Descriptor::new(&sddl(&user_sid().unwrap())).unwrap(), handler).expect("served");
        name
    }

    #[test]
    fn lines_are_answered_however_the_writes_cut_them() {
        let pipe = connect(&served()).unwrap();
        let mut lines = BufReader::new(&pipe).lines();
        // Two requests in one write, with empty lines and CR LF between them.
        (&pipe).write_all(b"{\"id\":1,\"cmd\":\"list\"}\r\n\n   \r\n{\"id\":2,\"cmd\":\"list\"}\n").unwrap();
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"id":1,"ok":true,"result":"listed"}"#);
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"id":2,"ok":true,"result":"listed"}"#, "the empty lines between are not answered");
        // One request cut across writes, with a pause inside a character of two bytes.
        let line = "{\"id\":3,\"cmd\":\"set-title\",\"pane\":\"1-1\",\"title\":\"é\"}\n".as_bytes();
        let cut = line.iter().position(|b| *b == 0xC3).unwrap() + 1;
        (&pipe).write_all(&line[..cut]).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        (&pipe).write_all(&line[cut..]).unwrap();
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"error":"echo: SetTitle { pane: \"1-1\", title: \"é\" }","id":3,"ok":false}"#);
    }

    #[test]
    fn a_line_that_is_not_utf8_is_answered_and_the_client_stays() {
        let pipe = connect(&served()).unwrap();
        let mut lines = BufReader::new(&pipe).lines();
        (&pipe).write_all(b"{\"id\":1,\"cmd\":\"set-title\",\"pane\":\"1-1\",\"title\":\"\xff\xfe\"}\n").unwrap();
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"error":"not UTF-8","id":null,"ok":false}"#);
        writeln!(&pipe, r#"{{"id":2,"cmd":"list"}}"#).unwrap();
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"id":2,"ok":true,"result":"listed"}"#);
    }

    #[test]
    fn a_line_longer_than_the_limit_is_refused_and_the_client_let_go() {
        let pipe = connect(&served()).unwrap();
        // Past the limit with no line break: the rest after the limit fits the pipe's buffer, so the write ends. (Writing
        // from a second thread would not do: on a handle without overlapped I/O, the read waits behind the write.)
        (&pipe).write_all(&vec![b'x'; MAX_LINE as usize + 1024]).unwrap();
        let mut lines = BufReader::new(&pipe).lines();
        assert_eq!(lines.next().unwrap().unwrap(), format!(r#"{{"error":"a request is at most {MAX_LINE} bytes","id":null,"ok":false}}"#));
        assert!(lines.next().is_none_or(|line| line.is_err()), "and the pipe is closed");
    }

    #[test]
    fn the_cli_finds_the_newest_live_instance_and_clears_files_that_name_none() {
        let dir = std::env::temp_dir().join(format!("nekominal-api-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let at = |file: &str, text: &str, age: u64| {
            std::fs::write(dir.join(file), text).unwrap();
            let when = std::time::SystemTime::now() - Duration::from_secs(age);
            std::fs::File::options().write(true).open(dir.join(file)).unwrap().set_modified(when).unwrap();
        };
        // A file of the user's that a broken instance file names: it must not be opened for writing.
        let victim = dir.with_extension("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        at("1", &format!(r"\\.\pipe\nekominal-test-{}-crashed", std::process::id()), 10);
        at("2", &victim.display().to_string(), 5);
        at("3", "", 1);
        at("4", "\u{feff}garbage \0 \u{1}", 0);
        assert_eq!(connect_newest(&dir).unwrap_err(), "no running Nekominal was found");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        let left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(left.is_empty(), "the files that name no live pipe are gone: {left:?}");
        // A live instance older than a crashed one is still found.
        at("5", &format!("{}\r\n", served()), 20);
        at("6", &format!(r"\\.\pipe\nekominal-test-{}-crashed", std::process::id()), 0);
        let pipe = connect_newest(&dir).expect("the live one");
        writeln!(&pipe, r#"{{"cmd":"list"}}"#).unwrap();
        assert!(BufReader::new(&pipe).lines().next().unwrap().unwrap().contains("listed"));
        assert!(!dir.join("6").exists() && dir.join("5").exists());
        assert!(connect(victim.to_str().unwrap()).is_err_and(|e| e.kind() == std::io::ErrorKind::InvalidInput), "--pipe with a path is refused too");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(&victim).unwrap();
    }

    #[test]
    fn many_clients_at_once_each_get_their_own_answers() {
        let name = served();
        let clients: Vec<_> = (0..8)
            .map(|client| {
                let name = name.clone();
                std::thread::spawn(move || {
                    let pipe = connect(&name).unwrap();
                    let mut lines = BufReader::new(&pipe).lines();
                    for n in 0..25 {
                        writeln!(&pipe, r#"{{"id":"{client}-{n}","cmd":"list"}}"#).unwrap();
                        assert_eq!(lines.next().unwrap().unwrap(), format!(r#"{{"id":"{client}-{n}","ok":true,"result":"listed"}}"#));
                    }
                })
            })
            .collect();
        for client in clients {
            client.join().unwrap();
        }
    }

    #[test]
    fn the_cli_exits_0_when_answered_ok_1_when_refused_and_2_when_it_could_not_ask() {
        let name = served();
        assert_eq!(run_cli(Some(&name), &json!({ "cmd": "list" })), 0);
        assert_eq!(run_cli(Some(&name), &json!({ "cmd": "focus", "pane": "1-1" })), 1);
        assert_eq!(run_cli(Some(&name), &json!({ "cmd": "nonsense" })), 1, "what the instance refuses is its answer");
        assert_eq!(run_cli(Some(r"\\.\pipe\nekominal-test-nobody-here"), &json!({ "cmd": "list" })), 2);
        // An instance that goes without an answer.
        let gone = format!(r"\\.\pipe\nekominal-test-{}-gone", std::process::id());
        let pipe = instance(&gone, &Descriptor::new(&sddl(&user_sid().unwrap())).unwrap(), true).unwrap();
        let server = std::thread::spawn(move || {
            // SAFETY: the handle of the File, a pipe instance in blocking mode.
            let _ = unsafe { ConnectNamedPipe(HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&pipe)), None) };
            let mut line = String::new();
            let _ = BufReader::new(&pipe).read_line(&mut line);
        });
        assert_eq!(run_cli(Some(&gone), &json!({ "cmd": "list" })), 2);
        server.join().unwrap();
    }

    #[test]
    fn a_client_is_answered_over_a_real_pipe_and_a_subscriber_gets_events() {
        let sid = user_sid().unwrap();
        let name = format!(r"\\.\pipe\nekominal-test-{}-{}", std::process::id(), line!());
        let handler: Handler = Arc::new(|request| match request {
            Request::List => Ok(json!({ "windows": [{ "id": 1, "tabs": [] }] })),
            other => Err(format!("echo: {other:?}")),
        });
        serve(&name, Descriptor::new(&sddl(&sid)).unwrap(), handler).expect("served");
        assert!(serve(&name, Descriptor::new(&sddl(&sid)).unwrap(), Arc::new(|_| Ok(Value::Null))).is_err(), "a second server cannot take the name");

        let pipe = connect(&name).expect("connected");
        let mut lines = BufReader::new(&pipe).lines();
        writeln!(&pipe, r#"{{"id":1,"cmd":"list"}}"#).unwrap();
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"id":1,"ok":true,"result":{"windows":[{"id":1,"tabs":[]}]}}"#);
        writeln!(&pipe, r#"{{"id":2,"cmd":"focus","pane":"1-1"}}"#).unwrap();
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"error":"echo: Focus { pane: \"1-1\" }","id":2,"ok":false}"#);
        writeln!(&pipe, "nonsense").unwrap();
        assert!(lines.next().unwrap().unwrap().contains("not JSON"), "a bad line is answered, and the client stays");

        // Several clients at once: each has an instance of its own.
        let other = connect(&name).expect("a second client");
        writeln!(&other, r#"{{"id":"x","cmd":"list"}}"#).unwrap();
        assert!(BufReader::new(&other).lines().next().unwrap().unwrap().starts_with(r#"{"id":"x","ok":true"#));

        writeln!(&pipe, r#"{{"id":3,"cmd":"subscribe","events":["command-finished"]}}"#).unwrap();
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"id":3,"ok":true,"result":{"subscribed":["command-finished"]}}"#);
        while !wants(Topic::CommandFinished) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!wants(Topic::Output));
        publish(Topic::Output, "1-1", json!({}));
        publish(Topic::CommandFinished, "1-1", json!({ "exit": 0 }));
        assert_eq!(lines.next().unwrap().unwrap(), r#"{"event":"command-finished","exit":0,"pane":"1-1"}"#, "only what it follows");
        drop(lines);
        drop(pipe);
        // The client is gone: the next event finds it so and drops it.
        publish(Topic::CommandFinished, "1-1", json!({ "exit": 0 }));
        std::thread::sleep(Duration::from_millis(100));
        publish(Topic::CommandFinished, "1-1", json!({ "exit": 0 }));
        assert!(!wants(Topic::CommandFinished));
    }
}
