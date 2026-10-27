//! Minimal ConPTY host.
//!
//! Loads the bundled, version-matched `conpty.dll` + `OpenConsole.exe` pair from the executable's
//! directory when both exist and falls back to the in-box kernel32 API otherwise. Children are
//! created inside a kill-on-close Job Object so closing a pane takes the whole process tree down.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString, c_void},
    fs::File,
    io::{self, Write},
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{Mutex, OnceLock},
};

use windows_sys::{
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::{
        Foundation::{CloseHandle, FILETIME, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
        System::{
            Console::{COORD, HPCON},
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation,
                QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
            },
            LibraryLoader::{
                GetModuleHandleW, GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
            },
            Pipes::CreatePipe,
            SystemInformation::OSVERSIONINFOW,
            Threading::{
                CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
                EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, GetProcessTimes, INFINITE,
                InitializeProcThreadAttributeList, OpenProcess, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
                STARTF_USESTDHANDLES,
                STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
            },
        },
    },
};

const GLYPH_WIDTH_GRAPHEMES: u32 = 0x08;
const GLYPH_WIDTH_WCSWIDTH: u32 = 0x10;

/// Cell-width model ConPTY uses to place the cursor. It must match the width model of the
/// emulator that consumes the output, otherwise the cursor drifts after wide characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WidthModel {
    /// Per-code-point `wcswidth` widths (`PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH`): plain alacritty_terminal.
    /// A ZWJ family is 6 columns here and 2 in the programs (string-width, Claude Code).
    Wcswidth,
    /// Grapheme-cluster widths (`PSEUDOCONSOLE_GLYPH_WIDTH_GRAPHEMES`): what programs count, and what the
    /// emulator does with the cluster layer `nekominal_term` puts over alacritty_terminal.
    #[default]
    Graphemes,
}

impl WidthModel {
    /// Value of the flags argument of `CreatePseudoConsole`.
    pub const fn flag(self) -> u32 {
        match self {
            WidthModel::Wcswidth => GLYPH_WIDTH_WCSWIDTH,
            WidthModel::Graphemes => GLYPH_WIDTH_GRAPHEMES,
        }
    }
}

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> i32;
type ResizeFn = unsafe extern "system" fn(HPCON, COORD) -> i32;
type CloseFn = unsafe extern "system" fn(HPCON);
type ReleaseFn = unsafe extern "system" fn(HPCON) -> i32;

/// Which ConPTY implementation is hosting the sessions.
#[derive(Clone, Debug)]
pub struct Backend {
    /// [`Backend::BUNDLED`] (conpty.dll + OpenConsole.exe) or [`Backend::SYSTEM`] (kernel32).
    pub kind: &'static str,
    pub version: String,
    /// Windows build whose ConPTY behaviour applies: the OS build for the in-box implementation,
    /// a current one for the bundled pair. It decides how lines reflow on resize.
    pub build_number: u32,
}

/// What the in-box ConPTY cannot do, for the notice the UI shows when it hosts the sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Degradation {
    /// [`WidthModel`] flags are accepted and ignored, so cursor positions can drift after wide
    /// characters.
    WidthModelIgnored,
    /// Four-byte UTF-8 sequences (emoji and other supplementary-plane characters) written to the
    /// session are mangled.
    FourByteUtf8Input,
}

impl Backend {
    pub const BUNDLED: &'static str = "bundled";
    pub const SYSTEM: &'static str = "system";

    pub fn is_bundled(&self) -> bool {
        self.kind == Self::BUNDLED
    }

    /// Empty for the bundled pair; the UI shows a notice when this is not empty.
    pub fn degradations(&self) -> &'static [Degradation] {
        if self.is_bundled() {
            &[]
        } else {
            &[
                Degradation::WidthModelIgnored,
                Degradation::FourByteUtf8Input,
            ]
        }
    }
}

/// Build number reported for the bundled ConPTY, which behaves like a current Windows.
const BUNDLED_BUILD_NUMBER: u32 = 26100;

fn os_build_number() -> u32 {
    // SAFETY: the struct is zero-initialised with its size set, as RtlGetVersion requires.
    unsafe {
        let mut info: OSVERSIONINFOW = zeroed();
        info.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
        if RtlGetVersion(&mut info) == 0 {
            info.dwBuildNumber
        } else {
            0
        }
    }
}

struct Api {
    create: CreateFn,
    resize: ResizeFn,
    close: CloseFn,
    /// Available from Windows 11 24H2 and in the bundled DLL; lets EOF reach the reader without
    /// the blocking behaviour of `ClosePseudoConsole`.
    release: Option<ReleaseFn>,
    backend: Backend,
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// # Safety
/// `module` must be a loaded module, `name` NUL-terminated, and `T` the function pointer type of
/// that export.
unsafe fn symbol<T>(module: *mut c_void, name: &[u8]) -> Option<T> {
    debug_assert!(name.ends_with(&[0]));
    // SAFETY: the caller guarantees `module` and the NUL-terminated `name`.
    let addr = unsafe { GetProcAddress(module, name.as_ptr()) }?;
    // SAFETY: the caller guarantees `T` is the export's function pointer type.
    Some(unsafe { std::mem::transmute_copy::<_, T>(&addr) })
}

/// # Safety
/// `module` must export the ConPTY functions with their documented signatures.
unsafe fn api_from_module(module: *mut c_void, backend: Backend) -> Option<Api> {
    // SAFETY: the caller guarantees the signatures that `symbol` reinterprets the exports as.
    unsafe {
        Some(Api {
            create: symbol(module, b"CreatePseudoConsole\0")?,
            resize: symbol(module, b"ResizePseudoConsole\0")?,
            close: symbol(module, b"ClosePseudoConsole\0")?,
            release: symbol(module, b"ReleasePseudoConsole\0"),
            backend,
        })
    }
}

fn load_bundled(dir: &Path) -> Option<Api> {
    let dll = dir.join("conpty.dll");
    if !dll.is_file() || !dir.join("OpenConsole.exe").is_file() {
        return None;
    }
    let version = std::fs::read_to_string(dir.join("VERSION")).unwrap_or_default();
    let path = wide(dll.as_os_str());
    // SAFETY: `path` is NUL-terminated; the module is never unloaded.
    unsafe {
        let module = LoadLibraryExW(path.as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
        if module.is_null() {
            return None;
        }
        api_from_module(
            module,
            Backend {
                kind: Backend::BUNDLED,
                version: version.trim().to_owned(),
                build_number: BUNDLED_BUILD_NUMBER,
            },
        )
    }
}

fn load_system() -> Option<Api> {
    let name = wide(OsStr::new("kernel32.dll"));
    // SAFETY: kernel32 is always loaded.
    unsafe {
        let module = GetModuleHandleW(name.as_ptr());
        api_from_module(
            module,
            Backend {
                kind: Backend::SYSTEM,
                version: String::new(),
                build_number: os_build_number(),
            },
        )
    }
}

fn api() -> io::Result<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        exe_dir
            .as_deref()
            .and_then(load_bundled)
            .or_else(load_system)
    })
    .as_ref()
    .ok_or_else(|| io::Error::other("ConPTY is not available on this system"))
}

pub fn backend() -> io::Result<Backend> {
    api().map(|api| api.backend.clone())
}

struct Handle(HANDLE);

// SAFETY: kernel handles may be used from any thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle is owned and closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn last_error(context: &str) -> io::Error {
    let code = unsafe { GetLastError() };
    let os = io::Error::from_raw_os_error(code as i32);
    io::Error::new(os.kind(), format!("{context}: {os}"))
}

fn create_pipe() -> io::Result<(Handle, Handle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: out-pointers are valid; null attributes make the handles non-inheritable.
    if unsafe { CreatePipe(&mut read, &mut write, null(), 0) } == 0 {
        return Err(last_error("CreatePipe"));
    }
    Ok((Handle(read), Handle(write)))
}

#[derive(Clone, Debug)]
pub struct SpawnConfig {
    /// Absolute path of the executable.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Working directory. A missing or invalid directory falls back to the home directory
    /// (see [`resolve_cwd`]).
    pub cwd: Option<PathBuf>,
    /// Overrides applied on top of the base environment.
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub width_model: WidthModel,
    /// The base is this process's environment, as the program that started it left it. Otherwise it is the one Windows
    /// gives a program started anew (the user's and the machine's variables, PATH as it is now), as Windows Terminal does:
    /// what the starting program set for itself (a Claude Code session, PowerShell 7's module folders) stays out of the
    /// session, and a variable changed since this process started is seen.
    pub inherit_environment: bool,
}

pub struct Pty {
    api: &'static Api,
    hpc: Mutex<Option<HPCON>>,
    input: Mutex<File>,
    process: Handle,
    job: Handle,
    pub pid: u32,
}

impl Pty {
    /// Spawns the child attached to a new pseudoconsole. Returns the session and its output stream.
    pub fn spawn(config: &SpawnConfig) -> io::Result<(Pty, File)> {
        let api = api()?;
        let (input_read, input_write) = create_pipe()?;
        let (output_read, output_write) = create_pipe()?;
        // Before the pseudoconsole: a `?` after it would leak the console, and its conhost with it.
        let job = create_kill_on_close_job()?;

        let hpc = create_pseudoconsole(
            api.create,
            coord(config.cols, config.rows),
            input_read.0,
            output_write.0,
            config.width_model,
        )?;
        // ConPTY owns its copies now; ours must close so EOF can be observed.
        drop(input_read);
        drop(output_write);

        let process = match spawn_in_pseudoconsole(hpc, job.0, config) {
            Ok(process) => process,
            Err(e) => {
                release(api, hpc);
                return Err(e);
            }
        };
        let pid = process.1;

        // SAFETY: both handles are owned, synchronous pipe ends that are not used elsewhere.
        let (input, output) = unsafe {
            (
                File::from_raw_handle(into_raw(input_write)),
                File::from_raw_handle(into_raw(output_read)),
            )
        };
        let pty = Pty {
            api,
            hpc: Mutex::new(Some(hpc)),
            input: Mutex::new(input),
            process: process.0,
            job,
            pid,
        };
        Ok((pty, output))
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut input = self.input.lock().unwrap();
        input.write_all(bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let guard = self.hpc.lock().unwrap();
        let Some(hpc) = *guard else {
            return Ok(());
        };
        let hr = unsafe { (self.api.resize)(hpc, coord(cols, rows)) };
        if hr < 0 {
            return Err(io::Error::other(format!(
                "ResizePseudoConsole (HRESULT {hr:#010x})"
            )));
        }
        Ok(())
    }

    /// Blocks until the child exits and returns its exit code.
    pub fn wait(&self) -> u32 {
        // SAFETY: the process handle lives as long as `self`.
        unsafe {
            if WaitForSingleObject(self.process.0, INFINITE) != WAIT_OBJECT_0 {
                return u32::MAX;
            }
            let mut code = 0u32;
            if GetExitCodeProcess(self.process.0, &mut code) == 0 {
                return u32::MAX;
            }
            code
        }
    }

    /// Every process currently in the job: the child and all its descendants.
    pub fn job_process_ids(&self) -> Vec<u32> {
        // The list header is two u32 counts followed by pointer-sized ids; u64 words keep the
        // buffer aligned for it. A job of more processes than fit (`make -j80`) says how many it
        // has: the list is asked for again with room for them, and for a few started meanwhile.
        let mut room = 64usize;
        for _ in 0..3 {
            let mut buffer = vec![0u64; 2 + room];
            let list = buffer.as_mut_ptr() as *mut JOBOBJECT_BASIC_PROCESS_ID_LIST;
            // SAFETY: the buffer is larger than the length passed, and the job handle is valid for
            // the lifetime of `self`. The ids read back lie inside the buffer because the API
            // reports at most the number that fit.
            unsafe {
                let ok = QueryInformationJobObject(
                    self.job.0,
                    JobObjectBasicProcessIdList,
                    list as *mut c_void,
                    (buffer.len() * size_of::<u64>()) as u32,
                    null_mut(),
                );
                let assigned = (*list).NumberOfAssignedProcesses as usize;
                if ok == 0 && assigned > room {
                    room = assigned + 16;
                    continue;
                }
                if ok == 0 {
                    return Vec::new();
                }
                let count = (*list).NumberOfProcessIdsInList as usize;
                return std::slice::from_raw_parts((*list).ProcessIdList.as_ptr(), count)
                    .iter()
                    .map(|&pid| pid as u32)
                    .collect();
            }
        }
        Vec::new()
    }

    /// The program the user is running now: the process of the job that started last, not counting the helpers of the
    /// console, or the child itself when it started nothing. None when the processes cannot be read.
    pub fn job(&self) -> Option<Job> {
        self.job_with(&process_names()?)
    }

    /// `job` from a list of the processes of the system (`process_names`) taken once for every session looked at
    /// together: a snapshot of the system per session would be the dear part.
    pub fn job_with(&self, names: &[(u32, u32, String)]) -> Option<Job> {
        let name_of = |pid: u32| names.iter().find(|(id, _, _)| *id == pid).map(|(_, _, name)| name.clone());
        let pids = with_descendants(self.job_process_ids(), names);
        // The child is in its job while it lives: an empty list is a query that failed, not a shell alone at its prompt.
        if pids.is_empty() {
            return None;
        }
        let mut newest: Option<(u64, u32, String)> = None;
        for pid in pids {
            if pid == self.pid {
                continue;
            }
            let Some(name) = name_of(pid) else { continue };
            if is_console_helper(&name) {
                continue;
            }
            let started = start_time(pid);
            if newest.as_ref().is_none_or(|(at, _, _)| started >= *at) {
                newest = Some((started, pid, name));
            }
        }
        let root = newest.is_none();
        let (pid, name) = newest.map(|(_, pid, name)| (pid, name)).or_else(|| Some((self.pid, name_of(self.pid)?)))?;
        Some(Job { name, path: image_path(pid), root })
    }


    /// Terminates the child and every process it started.
    pub fn kill(&self) {
        // SAFETY: both handles live as long as `self`.
        unsafe {
            TerminateJobObject(self.job.0, 1);
            TerminateProcess(self.process.0, 1);
        }
    }

    /// Releases the pseudoconsole so the output stream reaches EOF. The reader must keep draining
    /// the stream while this runs, because closing can block on unread output.
    pub fn close_console(&self) {
        if let Some(hpc) = self.hpc.lock().unwrap().take() {
            release(self.api, hpc);
        }
    }

    /// [`Pty::close_console`] when that cannot block. Without `ReleasePseudoConsole`,
    /// `ClosePseudoConsole` waits for the output pipe to drain, which hangs once no thread reads it
    /// any more (process exit). In that case the forwarder thread or `Drop` closes the console.
    pub fn close_console_if_nonblocking(&self) {
        if self.api.release.is_some() {
            self.close_console();
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.close_console();
    }
}

/// Creates the pseudoconsole. An implementation that rejects the width flag (instead of
/// accepting and ignoring it like the in-box one) gets a second attempt without it.
fn create_pseudoconsole(
    create: CreateFn,
    size: COORD,
    input: HANDLE,
    output: HANDLE,
    width_model: WidthModel,
) -> io::Result<HPCON> {
    let mut hpc: HPCON = 0;
    // SAFETY: the pipe handles are valid; ConPTY duplicates the ends it needs.
    let mut hr = unsafe { create(size, input, output, width_model.flag(), &mut hpc) };
    if hr < 0 {
        // SAFETY: as above.
        hr = unsafe { create(size, input, output, 0, &mut hpc) };
    }
    if hr < 0 {
        return Err(io::Error::from_raw_os_error(hr & 0xFFFF)
            .into_with_context(format!("CreatePseudoConsole (HRESULT {hr:#010x})")));
    }
    Ok(hpc)
}

fn release(api: &Api, hpc: HPCON) {
    // SAFETY: `hpc` came from `create` and is released once.
    unsafe {
        match api.release {
            Some(release) => {
                release(hpc);
            }
            None => (api.close)(hpc),
        }
    }
}

/// Directory a child starts in: `requested` when it is an existing directory, otherwise the home
/// directory, otherwise `None` (the child inherits this process's directory).
pub fn resolve_cwd(requested: Option<&Path>) -> Option<PathBuf> {
    pick_directory(
        requested,
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    )
}

fn pick_directory(requested: Option<&Path>, home: Option<PathBuf>) -> Option<PathBuf> {
    requested
        .filter(|dir| dir.is_dir())
        .map(Path::to_path_buf)
        .or_else(|| home.filter(|dir| dir.is_dir()))
}

fn coord(cols: u16, rows: u16) -> COORD {
    COORD {
        X: cols.clamp(1, i16::MAX as u16) as i16,
        Y: rows.clamp(1, i16::MAX as u16) as i16,
    }
}

fn into_raw(handle: Handle) -> *mut c_void {
    let raw = handle.0;
    std::mem::forget(handle);
    raw
}

fn create_kill_on_close_job() -> io::Result<Handle> {
    // SAFETY: plain Win32 calls with valid arguments.
    unsafe {
        let job = Handle(CreateJobObjectW(null(), null()));
        if job.0.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            return Err(last_error("SetInformationJobObject"));
        }
        Ok(job)
    }
}

/// Returns the process handle and its pid.
fn spawn_in_pseudoconsole(
    hpc: HPCON,
    job: HANDLE,
    config: &SpawnConfig,
) -> io::Result<(Handle, u32)> {
    let application = wide(config.program.as_os_str());
    let mut command_line = wide(OsStr::new(&build_command_line(
        &config.program,
        &config.args,
    )));
    let environment = build_environment_block(&config.env, config.inherit_environment);
    let cwd = resolve_cwd(config.cwd.as_deref()).map(|dir| wide(dir.as_os_str()));

    // SAFETY: the attribute list buffer outlives every use of the pointer, and every pointer
    // handed to CreateProcessW stays valid for the duration of the call.
    unsafe {
        let mut list_size = 0usize;
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut list_size);
        // usize elements give the buffer the pointer alignment the API expects.
        let mut list_buffer = vec![0usize; list_size.div_ceil(size_of::<usize>())];
        let list = list_buffer.as_mut_ptr() as *mut c_void;
        if InitializeProcThreadAttributeList(list, 1, 0, &mut list_size) == 0 {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }

        let result = (|| {
            if UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                hpc as *const c_void,
                size_of::<HPCON>(),
                null_mut(),
                null(),
            ) == 0
            {
                return Err(last_error("UpdateProcThreadAttribute(PSEUDOCONSOLE)"));
            }
            let mut startup: STARTUPINFOEXW = zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.lpAttributeList = list;
            // Without explicit invalid handles a child can still pick up this process's redirected
            // stdio (for example when launched from a shell that redirects output) and bypass the pseudoconsole.
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
            startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
            startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
            let mut info: PROCESS_INFORMATION = zeroed();

            let ok = CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr() as *const c_void,
                cwd.as_ref().map_or(null(), |dir| dir.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            );
            if ok == 0 {
                return Err(last_error(&format!(
                    "CreateProcessW {}",
                    config.program.display()
                )));
            }
            drop(Handle(info.hThread));
            let process = Handle(info.hProcess);
            // Packaged apps (the Store build of PowerShell) already run inside a system job and
            // reject a job list at creation, so the job is assigned afterwards. If the assignment
            // is refused, `kill` still terminates the process itself.
            AssignProcessToJobObject(job, process.0);
            Ok((process, info.dwProcessId))
        })();

        DeleteProcThreadAttributeList(list);
        result
    }
}

/// Quotes `program` and `args` into one command line following the MSVCRT parsing rules.
pub fn build_command_line(program: &Path, args: &[String]) -> String {
    let mut line = quote_argument(&program.to_string_lossy());
    for arg in args {
        line.push(' ');
        line.push_str(&quote_argument(arg));
    }
    line
}

fn quote_argument(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        return arg.to_owned();
    }
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes before a quote are doubled, then the quote itself is escaped.
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(ch);
                backslashes = 0;
            }
        }
    }
    // Trailing backslashes precede the closing quote and must be doubled.
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// Builds a UTF-16 environment block: the base (this process's variables when `inherit`, else the saved ones of
/// [`saved_environment`], and this process's when those cannot be read) plus overrides, deduplicated case-insensitively
/// and sorted as CreateProcess expects.
pub fn build_environment_block(overrides: &BTreeMap<String, String>, inherit: bool) -> Vec<u16> {
    let mut merged: BTreeMap<String, (OsString, OsString)> = BTreeMap::new();
    let base = if inherit { None } else { saved_environment() };
    for (key, value) in base.unwrap_or_else(|| std::env::vars_os().collect()) {
        let folded = key.to_string_lossy().to_uppercase();
        // Hidden per-drive variables such as "=C:" start with '=' and must be kept.
        merged.insert(folded, (key, value));
    }
    for (key, value) in overrides {
        merged.insert(
            key.to_uppercase(),
            (OsString::from(key), OsString::from(value)),
        );
    }
    let mut block = Vec::new();
    for (key, value) in merged.values() {
        block.extend(key.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    block
}

/// The environment Windows gives a program started anew for this user (`CreateEnvironmentBlock` without inheriting):
/// the machine's and the user's variables as the registry has them now, the volatile ones of the logon, PATH merged.
pub fn saved_environment() -> Option<Vec<(OsString, OsString)>> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Security::{TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY};
    use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token: HANDLE = null_mut();
    // SAFETY: the token of this process, closed by `Handle`; the block is read up to its double NUL and freed.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE, &mut token) == 0 {
            return None;
        }
        let token = Handle(token);
        let mut block: *mut c_void = null_mut();
        if CreateEnvironmentBlock(&mut block, token.0, 0) == 0 || block.is_null() {
            return None;
        }
        let mut vars = Vec::new();
        let mut at = block as *const u16;
        loop {
            let len = (0..).take_while(|&i| *at.add(i) != 0).count();
            if len == 0 {
                break;
            }
            let entry = OsString::from_wide(std::slice::from_raw_parts(at, len));
            vars.extend(split_entry(&entry));
            at = at.add(len + 1);
        }
        DestroyEnvironmentBlock(block);
        Some(mend_path(expand_references(vars)))
    }
}

/// A PATH whose quotes do not pair up (an installer that wrote `C:\Program Files\dotnet"`) loses its quotes. Windows
/// ignores the stray one, but Go's `filepath.SplitList` (gh, and the tools agents run) reads it as an opening quote and
/// takes every folder after it as part of one name: git and the rest of the user's PATH are then not found.
fn mend_path(vars: Vec<(OsString, OsString)>) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .map(|(key, value)| {
            let text = value.to_string_lossy();
            if key.to_string_lossy().eq_ignore_ascii_case("PATH") && text.matches('"').count() % 2 == 1 {
                (key, OsString::from(text.replace('"', "")))
            } else {
                (key, value)
            }
        })
        .collect()
}

/// The block expands a value before it holds every variable (`PSModulePath` names `%ProgramFiles%`, added after it), so
/// the `%NAME%` left are expanded once more from the block itself; a name it does not hold stays as it is.
fn expand_references(vars: Vec<(OsString, OsString)>) -> Vec<(OsString, OsString)> {
    let lookup: BTreeMap<String, String> = vars.iter().map(|(k, v)| (k.to_string_lossy().to_uppercase(), v.to_string_lossy().into_owned())).collect();
    let expand = |value: &OsStr| -> OsString {
        let text = value.to_string_lossy();
        if !text.contains('%') {
            return value.to_owned();
        }
        let mut out = String::new();
        let mut rest = text.as_ref();
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%').and_then(|end| lookup.get(&after[..end].to_uppercase()).map(|v| (end, v))) {
                Some((end, found)) if end > 0 => {
                    out.push_str(found);
                    rest = &after[end + 1..];
                }
                _ => {
                    out.push('%');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out.into()
    };
    // Only PSModulePath: any other value is as the registry has it, and a literal `%TEMP%` in a plain string stays.
    vars.iter().map(|(k, v)| (k.clone(), if k.to_string_lossy().eq_ignore_ascii_case("PSModulePath") { expand(v) } else { v.clone() })).collect()
}

/// `NAME=value` as its name and value; the name may start with `=` (`=C:=C:\x`), so the split is at the first `=` after
/// the first character.
fn split_entry(entry: &OsStr) -> Option<(OsString, OsString)> {
    use std::os::windows::ffi::OsStringExt;
    let wide: Vec<u16> = entry.encode_wide().collect();
    let eq = wide.iter().skip(1).position(|&c| c == b'=' as u16)? + 1;
    Some((OsString::from_wide(&wide[..eq]), OsString::from_wide(&wide[eq + 1..])))
}

trait IntoWithContext {
    fn into_with_context(self, context: String) -> io::Error;
}

impl IntoWithContext for io::Error {
    fn into_with_context(self, context: String) -> io::Error {
        io::Error::new(self.kind(), format!("{context}: {self}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_shell_starts_outside_its_job_is_found_by_its_parents() {
        // Ids no process has: their start times read as 0 alike, so only the tree decides.
        let p = |pid: u32, ppid: u32| (pid, ppid, format!("p{pid}"));
        let system = [p(4_000_001, 1), p(4_000_002, 4_000_001), p(4_000_003, 4_000_002), p(4_000_004, 9), p(4_000_005, 4_000_005)];
        let mut found = with_descendants(vec![4_000_001], &system);
        found.sort_unstable();
        assert_eq!(found, [4_000_001, 4_000_002, 4_000_003], "children and grandchildren, not a stranger");
        assert!(with_descendants(Vec::new(), &system).is_empty(), "a job that could not be read stays unread");
        let cycle = [p(4_000_011, 4_000_012), p(4_000_012, 4_000_011)];
        assert_eq!(with_descendants(vec![4_000_011], &cycle).len(), 2, "a loop of parents ends");
    }

    #[test]
    fn plain_arguments_are_not_quoted() {
        assert_eq!(quote_argument("-NoLogo"), "-NoLogo");
    }

    #[test]
    fn arguments_with_spaces_and_quotes_follow_msvcrt_rules() {
        assert_eq!(quote_argument(""), "\"\"");
        assert_eq!(quote_argument("a b"), "\"a b\"");
        assert_eq!(quote_argument("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(
            quote_argument("C:\\dir with space\\"),
            "\"C:\\dir with space\\\\\""
        );
        assert_eq!(quote_argument("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn command_line_quotes_the_program_path() {
        let line = build_command_line(
            Path::new("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
            &["-NoLogo".into(), "-Command".into(), "echo hi".into()],
        );
        assert_eq!(
            line,
            "\"C:\\Program Files\\PowerShell\\7\\pwsh.exe\" -NoLogo -Command \"echo hi\""
        );
    }

    #[test]
    fn a_fresh_environment_is_the_saved_one_with_the_overrides_on_top() {
        let saved = saved_environment().expect("Windows gives this user an environment");
        let has = |name: &str| saved.iter().any(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case(name));
        assert!(has("PATH") && has("USERPROFILE") && has("SystemRoot"), "the variables every program gets");
        let mut overrides = BTreeMap::new();
        overrides.insert("TERM".to_owned(), "xterm-256color".to_owned());
        let text = String::from_utf16_lossy(&build_environment_block(&overrides, false));
        assert!(text.split('\0').any(|entry| entry == "TERM=xterm-256color"));
        assert!(text.split('\0').any(|entry| entry.to_ascii_uppercase().starts_with("PATH=")));
        let split = |entry: &str| split_entry(OsStr::new(entry)).map(|(k, v)| (k.into_string().unwrap(), v.into_string().unwrap()));
        assert_eq!(split("=C:=C:\\x"), Some(("=C:".into(), "C:\\x".into())), "a hidden per-drive variable keeps its leading =");
        assert_eq!(split("A=b=c"), Some(("A".into(), "b=c".into())));
        let vars = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))).collect::<Vec<_>>();
        let expanded = expand_references(vars(&[("PSModulePath", r"%ProgramFiles%\M;%NOPE%;50%;%%"), ("ProgramFiles", r"C:\Program Files")]));
        assert_eq!(expanded[0].1, OsString::from(r"C:\Program Files\M;%NOPE%;50%;%%"), "a name the block does not hold stays");
        let literal = expand_references(vars(&[("MINE", "%ProgramFiles%"), ("ProgramFiles", r"C:\Program Files")]));
        assert_eq!(literal[0].1, OsString::from("%ProgramFiles%"), "a plain string the user wrote is kept as written");
        assert!(saved.iter().all(|(key, value)| !key.to_string_lossy().eq_ignore_ascii_case("PSModulePath") || !value.to_string_lossy().contains("%ProgramFiles%")));
    }

    #[test]
    fn a_path_with_a_stray_quote_loses_its_quotes_and_a_quoted_folder_keeps_them() {
        let vars = |path: &str| vec![(OsString::from("Path"), OsString::from(path)), (OsString::from("Other"), OsString::from(r#"a"b"#))];
        let mended = mend_path(vars(r#"C:\Windows;C:\Program Files\dotnet";C:\Users\me\scoop\shims"#));
        assert_eq!(mended[0].1, OsString::from(r"C:\Windows;C:\Program Files\dotnet;C:\Users\me\scoop\shims"));
        assert_eq!(mended[1].1, OsString::from(r#"a"b"#), "only PATH is mended");
        let quoted = r#"C:\Windows;"C:\odd;name";C:\x"#;
        assert_eq!(mend_path(vars(quoted))[0].1, OsString::from(quoted), "quotes that pair up are meant");
    }

    #[test]
    fn environment_block_applies_overrides_case_insensitively() {
        let mut overrides = BTreeMap::new();
        overrides.insert("path".to_owned(), "C:\\only".to_owned());
        overrides.insert("NEKOMINAL_TEST".to_owned(), "1".to_owned());
        let block = build_environment_block(&overrides, true);
        let text = String::from_utf16_lossy(&block);
        let entries: Vec<&str> = text.split('\0').filter(|e| !e.is_empty()).collect();
        assert!(entries.contains(&"path=C:\\only"));
        assert!(entries.contains(&"NEKOMINAL_TEST=1"));
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.to_uppercase().starts_with("PATH="))
                .count(),
            1
        );
        assert!(block.ends_with(&[0, 0]));
    }

    /// Nothing the user exported is dropped, `NO_COLOR` included; the settings page reports it
    /// instead.
    #[test]
    fn environment_block_keeps_every_inherited_variable() {
        let text = String::from_utf16_lossy(&build_environment_block(&BTreeMap::new(), true));
        let entries: Vec<&str> = text.split('\0').filter(|e| !e.is_empty()).collect();
        for (key, value) in std::env::vars_os() {
            let entry = format!("{}={}", key.to_string_lossy(), value.to_string_lossy());
            assert!(entries.contains(&entry.as_str()), "missing {entry}");
        }
    }

    #[test]
    fn the_default_width_model_is_wcswidth() {
        assert_eq!(WidthModel::default(), WidthModel::Graphemes);
        assert_eq!(WidthModel::Wcswidth.flag(), 0x10);
        assert_eq!(WidthModel::Graphemes.flag(), 0x08);
    }

    #[test]
    fn only_the_bundled_backend_is_free_of_degradations() {
        let backend = |kind| Backend {
            kind,
            version: String::new(),
            build_number: 0,
        };
        assert!(backend(Backend::BUNDLED).degradations().is_empty());
        assert_eq!(
            backend(Backend::SYSTEM).degradations(),
            [
                Degradation::WidthModelIgnored,
                Degradation::FourByteUtf8Input
            ]
        );
    }

    thread_local! {
        /// Width flags received by the fake `CreatePseudoConsole` functions on this thread.
        static FLAGS_SEEN: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;

    unsafe extern "system" fn create_accepting_any_flag(
        _: COORD,
        _: HANDLE,
        _: HANDLE,
        flags: u32,
        out: *mut HPCON,
    ) -> i32 {
        FLAGS_SEEN.with(|seen| seen.borrow_mut().push(flags));
        // SAFETY: `create_pseudoconsole` passes a valid out-pointer.
        unsafe { *out = 7 };
        0
    }

    unsafe extern "system" fn create_rejecting_flags(
        _: COORD,
        _: HANDLE,
        _: HANDLE,
        flags: u32,
        out: *mut HPCON,
    ) -> i32 {
        FLAGS_SEEN.with(|seen| seen.borrow_mut().push(flags));
        if flags != 0 {
            return E_INVALIDARG;
        }
        // SAFETY: `create_pseudoconsole` passes a valid out-pointer.
        unsafe { *out = 7 };
        0
    }

    unsafe extern "system" fn create_failing(
        _: COORD,
        _: HANDLE,
        _: HANDLE,
        flags: u32,
        _: *mut HPCON,
    ) -> i32 {
        FLAGS_SEEN.with(|seen| seen.borrow_mut().push(flags));
        E_INVALIDARG
    }

    fn create_with(create: CreateFn, width_model: WidthModel) -> (io::Result<HPCON>, Vec<u32>) {
        FLAGS_SEEN.with(|seen| seen.borrow_mut().clear());
        let result =
            create_pseudoconsole(create, coord(80, 24), null_mut(), null_mut(), width_model);
        (result, FLAGS_SEEN.with(|seen| seen.take()))
    }

    #[test]
    fn creation_asks_for_the_requested_width_model_once() {
        let (result, flags) = create_with(create_accepting_any_flag, WidthModel::default());
        assert_eq!(result.unwrap(), 7);
        assert_eq!(flags, [0x08]);
        let (_, flags) = create_with(create_accepting_any_flag, WidthModel::Wcswidth);
        assert_eq!(flags, [0x10]);
    }

    #[test]
    fn creation_retries_without_the_flag_when_an_implementation_rejects_it() {
        let (result, flags) = create_with(create_rejecting_flags, WidthModel::default());
        assert_eq!(result.unwrap(), 7);
        assert_eq!(flags, [0x08, 0]);
    }

    #[test]
    fn creation_failure_names_the_call_and_the_hresult() {
        let (result, flags) = create_with(create_failing, WidthModel::default());
        let message = result.unwrap_err().to_string();
        assert!(message.contains("CreatePseudoConsole"), "{message}");
        assert!(message.contains("0x80070057"), "{message}");
        assert_eq!(flags, [0x08, 0]);
    }

    #[test]
    fn a_missing_or_invalid_cwd_falls_back_to_the_home_directory() {
        let temp = std::env::temp_dir();
        let home = Some(temp.clone());
        let missing = temp.join("nekominal-no-such-directory");
        let file = std::env::current_exe().unwrap();
        assert_eq!(pick_directory(Some(&temp), None), Some(temp.clone()));
        assert_eq!(pick_directory(None, home.clone()), home);
        assert_eq!(pick_directory(Some(&missing), home.clone()), home);
        assert_eq!(
            pick_directory(Some(&file), home.clone()),
            home,
            "a file is not a directory"
        );
        assert_eq!(pick_directory(Some(&missing), Some(missing.clone())), None);
        assert_eq!(pick_directory(None, None), None);
    }
}

/// Every process of the system: its id, the id of the process that started it, and the name of its program. None when
/// the system cannot be read.
pub fn process_names() -> Option<Vec<(u32, u32, String)>> {
    // SAFETY: the snapshot is owned and closed by `Handle`; the entry is initialised with its size as the calls ask.
    unsafe {
        let snapshot = Handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0));
        if snapshot.0 == INVALID_HANDLE_VALUE || snapshot.0.is_null() {
            return None;
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut names = Vec::new();
        let mut more = Process32FirstW(snapshot.0, &mut entry);
        while more != 0 {
            let len = entry.szExeFile.iter().position(|&unit| unit == 0).unwrap_or(entry.szExeFile.len());
            names.push((entry.th32ProcessID, entry.th32ParentProcessID, String::from_utf16_lossy(&entry.szExeFile[..len])));
            more = Process32NextW(snapshot.0, &mut entry);
        }
        Some(names)
    }
}

/// `pids` and every process started under them, from the processes of the system (`process_names`). A shell installed
/// from the Store is a packaged app, and what it starts runs outside the job of the session: only by its parents is it
/// found. A child counts only when it started after its parent, so that a parent's id taken again by a later process
/// brings in nothing.
pub fn with_descendants(mut pids: Vec<u32>, processes: &[(u32, u32, String)]) -> Vec<u32> {
    if pids.is_empty() {
        return pids;
    }
    let mut at = 0;
    while at < pids.len() {
        let parent = pids[at];
        let born = start_time(parent);
        for (pid, _, _) in processes.iter().filter(|(pid, ppid, _)| *ppid == parent && *pid != parent) {
            if !pids.contains(pid) && start_time(*pid) >= born {
                pids.push(*pid);
            }
        }
        at += 1;
    }
    pids
}

/// A program a session runs: its file name, and its whole path when the process lets it be read (the icon comes from it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub name: String,
    pub path: Option<std::path::PathBuf>,
    /// Nothing runs but the program the session started (the shell at its prompt).
    pub root: bool,
}

/// The path of the program file of a process.
fn image_path(pid: u32) -> Option<std::path::PathBuf> {
    // SAFETY: the handle is owned and closed by `Handle`; the buffer is as long as the size passed.
    unsafe {
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid));
        if process.0.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        if QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len) == 0 {
            return None;
        }
        Some(std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize])))
    }
}

/// When a process started, in 100 ns ticks; 0 when it cannot be asked.
fn start_time(pid: u32) -> u64 {
    // SAFETY: the handle is owned and closed by `Handle`; the times are written into locals.
    unsafe {
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid));
        if process.0.is_null() {
            return 0;
        }
        let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
            (zeroed(), zeroed(), zeroed(), zeroed());
        if GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return 0;
        }
        (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)
    }
}

/// A program that is part of the console and not of what the user runs.
fn is_console_helper(name: &str) -> bool {
    ["conhost.exe", "openconsole.exe"].iter().any(|helper| name.eq_ignore_ascii_case(helper))
}

/// Process inspection shared by the lifetime tests.
#[cfg(test)]
pub(crate) mod test_support {
    use std::time::Duration;

    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};

    use super::*;

    impl Pty {
        pub(crate) fn console_closed(&self) -> bool {
            self.hpc.lock().unwrap().is_none()
        }
    }

    /// Holds a process object open so its pid cannot be reused while a test waits for it to exit.
    pub(crate) struct ProcessWatch(Handle);

    impl ProcessWatch {
        pub(crate) fn open(pid: u32) -> Option<ProcessWatch> {
            // SAFETY: plain Win32 call; a null result means the process is already gone.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            (!handle.is_null()).then(|| ProcessWatch(Handle(handle)))
        }

        pub(crate) fn exited_within(&self, timeout: Duration) -> bool {
            // SAFETY: the handle is owned by `self`.
            unsafe { WaitForSingleObject(self.0.0, timeout.as_millis() as u32) == WAIT_OBJECT_0 }
        }
    }
}
