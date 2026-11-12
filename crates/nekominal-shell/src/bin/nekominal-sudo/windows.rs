//! `nekominal-sudo [-D folder] command [arguments]`: runs a command as administrator in the terminal it is typed in. The
//! shell stays what it was (its folder, its variables, what runs in it), and the command reads and writes the terminal as
//! if it ran there: Windows asks the user once, an elevated copy of this program attaches to the console of this one,
//! and starts the command on it. What the command says goes on the screen of the pane and what is typed reaches it.
//!
//! It is a program of its own, with a console, because a shell waits only for those: the window program
//! (`nekominal-next.exe`) would give the prompt back at once, and the shell and the command would read the keys together.
//! The command lines it reads and makes are `nekominal_pty::sudo`.
//!
//! shortcuts: the command gets the environment Windows gives an elevated program (not this shell's), and the input and
//! output of this program cannot be redirected; add the variables over a pipe, and the redirected handles as sudo does,
//! when either is needed.

use std::path::Path;

use nekominal_pty::{
    handoff::Process,
    sudo::{self, HelperRequest, Request},
};
use windows::{
    Win32::{
        Foundation::CloseHandle,
        System::{
            Console::{
                AttachConsole, CTRL_BREAK_EVENT, CTRL_C_EVENT, FreeConsole, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
                STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCtrlHandler,
            },
            Environment::GetCommandLineW,
            Threading::{CreateProcessW, GetExitCodeProcess, INFINITE, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject},
        },
        UI::{
            Shell::{IsUserAnAdmin, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_HIDE,
        },
    },
    core::{BOOL, HRESULT, HSTRING, PCWSTR, PWSTR, w},
};

/// `ERROR_CANCELLED`: the user said no to the prompt of Windows.
const ERROR_CANCELLED: u32 = 1223;
/// What the elevated copy answers when it could not do its part (an exit code of a command is not one of these).
const REFUSED: u32 = 0xE000_0001;
const NO_CONSOLE: u32 = 0xE000_0002;
const NOT_STARTED: u32 = 0xE000_0003;

pub fn main() {
    // SAFETY: the command line of this process, which lives as long as it.
    let raw = unsafe { GetCommandLineW().to_string().unwrap_or_default() };
    let code = match sudo::helper_request(&raw) {
        Some(request) => helper(&request),
        None => match sudo::parse(&raw) {
            Ok(request) => client(request),
            Err(e) => {
                eprintln!("{e}");
                2
            }
        },
    };
    std::process::exit(code as i32);
}

// --- the terminal's side -----------------------------------------------------------------------------------------------

/// Control-C and Control-Break go to every program on the console: this one (and the elevated copy) leave them to the
/// command, which ends or not by them as it does in any terminal. Not an ignored flag, which the command would inherit.
unsafe extern "system" fn leave_to_the_command(kind: u32) -> BOOL {
    BOOL::from(kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT)
}

fn on_a_console() -> bool {
    [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].into_iter().all(|which: STD_HANDLE| {
        let mut mode = Default::default();
        // SAFETY: plain queries of the standard handles of this process.
        unsafe { GetStdHandle(which).is_ok_and(|handle| GetConsoleMode(handle, &mut mode).is_ok()) }
    })
}

fn client(request: Request) -> i32 {
    if !on_a_console() {
        eprintln!("nekominal-sudo works in a terminal: its input and output cannot be redirected");
        return 2;
    }
    let folder = request.directory.or_else(|| std::env::current_dir().ok().map(|dir| dir.display().to_string()));
    let line = sudo::command_line(&request.command);
    // SAFETY: plain calls; the handler lives for ever.
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(leave_to_the_command), true);
        if IsUserAnAdmin().as_bool() {
            // Elevated already (UAC is off, or this is an administrator's terminal): there is nothing to ask.
            return run(&line, folder.as_deref());
        }
    }
    match elevated_copy(&sudo::helper_arguments(std::process::id(), folder.as_deref(), &line)) {
        Ok(REFUSED) => refused("the elevated program does not trust the program that asked"),
        Ok(NO_CONSOLE) => refused("the elevated program could not use this terminal"),
        Ok(NOT_STARTED) => refused("the command could not be started"),
        Ok(code) => code as i32,
        Err(Elevation::Declined) => refused("elevation was declined"),
        Err(Elevation::Failed(e)) => refused(&e),
    }
}

fn refused(why: &str) -> i32 {
    eprintln!("nekominal-sudo: {why}");
    1
}

enum Elevation {
    Declined,
    Failed(String),
}

/// Starts this program elevated with `parameters` (Windows asks the user), waits for it and gives its exit code.
fn elevated_copy(parameters: &str) -> Result<u32, Elevation> {
    let exe = std::env::current_exe().map_err(|e| Elevation::Failed(e.to_string()))?;
    let (file, parameters) = (HSTRING::from(exe.as_os_str()), HSTRING::from(parameters));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the strings live through the call; the process handle is closed once its exit code is read.
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            return Err(if e.code() == HRESULT::from_win32(ERROR_CANCELLED) { Elevation::Declined } else { Elevation::Failed(e.to_string()) });
        }
        if info.hProcess.is_invalid() {
            return Err(Elevation::Failed("Windows gave no process for the elevated program".into()));
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0u32;
        let read = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        read.map(|()| code).map_err(|e| Elevation::Failed(e.to_string()))
    }
}

// --- the elevated copy -------------------------------------------------------------------------------------------------

/// Whether process `pid` runs this program: only the program that asked for elevation is served.
fn is_this_program(pid: u32) -> bool {
    let (Ok(process), Ok(ours)) = (Process::open_to_query(pid), std::env::current_exe()) else { return false };
    let text = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().to_lowercase();
    process.image().is_some_and(|theirs| text(&theirs) == text(&ours))
}

fn helper(request: &HelperRequest) -> i32 {
    if !is_this_program(request.parent) {
        return REFUSED as i32;
    }
    // SAFETY: plain calls: the console of this copy (a hidden one) is left for the one of the program that asked.
    unsafe {
        let _ = FreeConsole();
        if AttachConsole(request.parent).is_err() {
            return NO_CONSOLE as i32;
        }
        let _ = SetConsoleCtrlHandler(Some(leave_to_the_command), true);
    }
    run(&request.line, request.folder.as_deref())
}

/// Starts `line` on the console of this process, in `folder`, and gives its exit code.
fn run(line: &str, folder: Option<&str>) -> i32 {
    let mut line: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
    let folder = folder.map(HSTRING::from);
    let startup = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: the buffers live through the calls; the handles the call makes are closed here.
    unsafe {
        let started = CreateProcessW(PCWSTR::null(), Some(PWSTR(line.as_mut_ptr())), None, None, false, Default::default(), None, folder.as_ref().map_or(PCWSTR::null(), |folder| PCWSTR(folder.as_ptr())), &startup, &mut process);
        if started.is_err() {
            return NOT_STARTED as i32;
        }
        let _ = CloseHandle(process.hThread);
        WaitForSingleObject(process.hProcess, INFINITE);
        let mut code = 0u32;
        let _ = GetExitCodeProcess(process.hProcess, &mut code);
        let _ = CloseHandle(process.hProcess);
        code as i32
    }
}
