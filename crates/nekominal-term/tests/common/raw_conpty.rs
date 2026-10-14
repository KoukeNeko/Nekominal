//! A minimal ConPTY launcher that takes the raw `CreatePseudoConsole` flags, so the glyph-width
//! modes that `nekominal_pty::SpawnConfig` cannot express (0x00, 0x18, and the 0x20 ambiguous-wide
//! bit) can be compared. Derived from `crates/nekominal-pty/src/conpty.rs`; loads the bundled
//! `conpty.dll` staged next to the test executable.

#![allow(dead_code, clippy::missing_transmute_annotations)]

use std::{
    ffi::{OsStr, c_void},
    fs::File,
    io::{self, Read, Write},
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
    ptr::{null, null_mut},
    sync::mpsc::{Receiver, channel},
    thread,
    time::{Duration, Instant},
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    System::{
        Console::{COORD, HPCON},
        LibraryLoader::{GetModuleHandleW, GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW},
        Pipes::CreatePipe,
        Threading::{
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
            InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

/// `PSEUDOCONSOLE_GLYPH_WIDTH_*` from microsoft/terminal `winconpty.h`.
pub const GLYPH_WIDTH_DEFAULT: u32 = 0x00;
pub const GLYPH_WIDTH_GRAPHEMES: u32 = 0x08;
pub const GLYPH_WIDTH_WCSWIDTH: u32 = 0x10;
pub const GLYPH_WIDTH_CONSOLE: u32 = 0x18;
pub const AMBIGUOUS_IS_WIDE: u32 = 0x20;

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> i32;
type ReleaseFn = unsafe extern "system" fn(HPCON) -> i32;

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

struct Dll {
    create: CreateFn,
    release: ReleaseFn,
}

fn load_bundled() -> Dll {
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let dll = wide(dir.join("conpty.dll").as_os_str());
    unsafe {
        let module = LoadLibraryExW(dll.as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
        assert!(!module.is_null(), "bundled conpty.dll not found next to the test executable");
        let symbol = |name: &[u8]| GetProcAddress(module, name.as_ptr()).expect("conpty.dll export");
        Dll {
            create: std::mem::transmute::<_, CreateFn>(symbol(b"CreatePseudoConsole\0")),
            release: std::mem::transmute::<_, ReleaseFn>(symbol(b"ReleasePseudoConsole\0")),
        }
    }
}

/// The in-box (kernel32) implementation, to compare with the bundled pair.
fn load_system() -> Dll {
    let name = wide(OsStr::new("kernel32.dll"));
    unsafe {
        let module = GetModuleHandleW(name.as_ptr());
        let symbol = |name: &[u8]| GetProcAddress(module, name.as_ptr());
        let create = symbol(b"CreatePseudoConsole\0").expect("CreatePseudoConsole");
        let release =
            symbol(b"ReleasePseudoConsole\0").or_else(|| symbol(b"ClosePseudoConsole\0")).expect("ClosePseudoConsole");
        Dll {
            create: std::mem::transmute::<_, CreateFn>(create),
            release: std::mem::transmute::<_, ReleaseFn>(release),
        }
    }
}

pub struct RawPty {
    dll: Dll,
    hpc: HPCON,
    process: HANDLE,
    input: File,
    pub output: Receiver<Vec<u8>>,
}

unsafe impl Send for RawPty {}

impl RawPty {
    pub fn spawn(program: &Path, args: &[String], cols: i16, rows: i16, flags: u32) -> io::Result<RawPty> {
        Self::spawn_from(load_bundled(), program, args, cols, rows, flags)
    }

    /// Same, but through the in-box kernel32 ConPTY.
    pub fn spawn_system(program: &Path, args: &[String], cols: i16, rows: i16, flags: u32) -> io::Result<RawPty> {
        Self::spawn_from(load_system(), program, args, cols, rows, flags)
    }

    fn spawn_from(dll: Dll, program: &Path, args: &[String], cols: i16, rows: i16, flags: u32) -> io::Result<RawPty> {
        unsafe {
            let (mut in_read, mut in_write, mut out_read, mut out_write): (HANDLE, HANDLE, HANDLE, HANDLE) =
                (null_mut(), null_mut(), null_mut(), null_mut());
            assert!(CreatePipe(&mut in_read, &mut in_write, null(), 0) != 0);
            assert!(CreatePipe(&mut out_read, &mut out_write, null(), 0) != 0);
            let mut hpc: HPCON = 0;
            let hr = (dll.create)(COORD { X: cols, Y: rows }, in_read, out_write, flags, &mut hpc);
            if hr < 0 {
                return Err(io::Error::other(format!("CreatePseudoConsole({flags:#x}) failed: {hr:#010x}")));
            }
            CloseHandle(in_read);
            CloseHandle(out_write);

            let mut command_line = String::new();
            command_line.push('"');
            command_line.push_str(&program.to_string_lossy());
            command_line.push('"');
            for arg in args {
                command_line.push(' ');
                if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
                    command_line.push_str(arg);
                } else {
                    command_line.push('"');
                    command_line.push_str(&arg.replace('"', "\\\""));
                    command_line.push('"');
                }
            }
            let application = wide(program.as_os_str());
            let mut command = wide(OsStr::new(&command_line));

            let mut size = 0usize;
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size);
            let mut list_buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
            let list = list_buffer.as_mut_ptr() as *mut c_void;
            assert!(InitializeProcThreadAttributeList(list, 1, 0, &mut size) != 0);
            assert!(
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                    hpc as *const c_void,
                    size_of::<HPCON>(),
                    null_mut(),
                    null(),
                ) != 0
            );
            let mut startup: STARTUPINFOEXW = zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.lpAttributeList = list;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
            startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
            startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
            let mut info: PROCESS_INFORMATION = zeroed();
            let ok = CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                null(),
                null(),
                &startup.StartupInfo,
                &mut info,
            );
            DeleteProcThreadAttributeList(list);
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            CloseHandle(info.hThread);

            let mut output_file = File::from_raw_handle(out_read);
            let (tx, rx) = channel();
            thread::spawn(move || {
                let mut buffer = vec![0u8; 32 * 1024];
                loop {
                    match output_file.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if tx.send(buffer[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
            Ok(RawPty { dll, hpc, process: info.hProcess, input: File::from_raw_handle(in_write), output: rx })
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.input.write_all(bytes).unwrap();
    }

    /// Collects output until the process exits (plus a quiet period) or `max` passes, answering
    /// whatever `on_chunk` returns (terminal replies).
    pub fn run(&mut self, max: Duration, mut on_chunk: impl FnMut(&[u8]) -> Vec<u8>) -> Vec<u8> {
        let deadline = Instant::now() + max;
        let mut raw = Vec::new();
        let mut exited_at: Option<Instant> = None;
        loop {
            if exited_at.is_none() && unsafe { WaitForSingleObject(self.process, 0) } == WAIT_OBJECT_0 {
                exited_at = Some(Instant::now());
            }
            if Instant::now() > deadline || exited_at.is_some_and(|t| t.elapsed() > Duration::from_millis(400)) {
                break;
            }
            if let Ok(chunk) = self.output.recv_timeout(Duration::from_millis(20)) {
                let reply = on_chunk(&chunk);
                if !reply.is_empty() {
                    self.write(&reply);
                }
                raw.extend_from_slice(&chunk);
            }
        }
        raw
    }
}

impl Drop for RawPty {
    fn drop(&mut self) {
        unsafe {
            TerminateProcess(self.process, 1);
            // The reader thread keeps draining while the console is released.
            let hpc = self.hpc;
            let release = self.dll.release;
            let handle = thread::spawn(move || {
                (release)(hpc);
            });
            let _ = handle.join();
            CloseHandle(self.process);
        }
    }
}
