//! The pseudo-terminal host on Unix, with the surface `conpty` has on Windows (`lib.rs` puts it in that module's place).
//!
//! The child gets a pty of its own as its controlling terminal and leads a session of its own, which is what a "job"
//! is here: the session of the child is every process it started that has not made a session of its own. A session is
//! not handed to another process: the in-place update that needs it is the Windows one.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

/// How wide the emulator counts the cells of a cluster. A pty has no cursor-placing console in between that must agree,
/// so this only keeps the settings and the sessions of both platforms the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WidthModel {
    Wcswidth,
    #[default]
    Graphemes,
}

/// Which pty host runs the sessions: always the system's.
#[derive(Clone, Debug)]
pub struct Backend {
    pub kind: &'static str,
    pub version: String,
    pub build_number: u32,
}

impl Backend {
    pub const SYSTEM: &'static str = "system";

    pub fn is_bundled(&self) -> bool {
        false
    }
}

pub fn backend() -> io::Result<Backend> {
    Ok(Backend { kind: Backend::SYSTEM, version: String::new(), build_number: 0 })
}

/// Serializable like on Windows: the same description starts a session in the pane of a window.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpawnConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// A missing or invalid directory falls back to the home directory (see [`resolve_cwd`]).
    pub cwd: Option<PathBuf>,
    /// Overrides applied on top of the base environment.
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub width_model: WidthModel,
    /// The base is this process's environment. Otherwise it is that environment without the variables that name the
    /// session of an agent or a terminal this process was started in, which the new session is not part of.
    pub inherit_environment: bool,
}

/// Variables of the terminal or the agent this process was started from, which a shell started anew does not have.
const SESSION_VARIABLES: [&str; 6] = ["TERM_PROGRAM", "TERM_PROGRAM_VERSION", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "TMUX", "STY"];

pub struct Pty {
    /// The master side, for writing the input and for the size.
    input: Mutex<File>,
    pub pid: u32,
}

fn os_error(context: &str) -> io::Error {
    let os = io::Error::last_os_error();
    io::Error::new(os.kind(), format!("{context}: {os}"))
}

fn window_size(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 }
}

impl Pty {
    /// Spawns the child on a new pty. Returns the session and its output stream.
    pub fn spawn(config: &SpawnConfig) -> io::Result<(Pty, File)> {
        let size = window_size(config.cols, config.rows);
        let (mut master, mut slave) = (-1, -1);
        // SAFETY: out-pointers are valid; a null name and termios leave the defaults, `size` is read.
        if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &size) } != 0 {
            return Err(os_error("openpty"));
        }
        // SAFETY: both descriptors were just made and nothing else owns them.
        let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        set_close_on_exec(&master);

        let mut command = Command::new(&config.program);
        command.args(&config.args);
        if !config.inherit_environment {
            for name in SESSION_VARIABLES {
                command.env_remove(name);
            }
        }
        command.envs(&config.env);
        if let Some(dir) = resolve_cwd(config.cwd.as_deref()) {
            command.current_dir(dir);
        }
        command.stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe calls between fork and exec. The child leads a session whose controlling
        // terminal is its stdin, the slave.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", config.program.display())))?;
        let pid = child.id();
        // The exit code is read by `wait`, which reaps the child by its id.
        drop(child);

        let output = File::from(master.try_clone()?);
        Ok((Pty { input: Mutex::new(File::from(master)), pid }, output))
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.input.lock().unwrap().write_all(bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let size = window_size(cols, rows);
        let input = self.input.lock().unwrap();
        // SAFETY: the master is open while `self` is; `size` is read.
        if unsafe { libc::ioctl(input.as_raw_fd(), libc::TIOCSWINSZ as _, &size) } != 0 {
            return Err(os_error("TIOCSWINSZ"));
        }
        Ok(())
    }

    /// Blocks until the child exits and returns its exit code (128 and the signal for a child a signal ended).
    pub fn wait(&self) -> u32 {
        let mut status = 0;
        loop {
            // SAFETY: plain call on a child of this process.
            let reaped = unsafe { libc::waitpid(self.pid as i32, &mut status, 0) };
            if reaped >= 0 {
                break;
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return u32::MAX;
            }
        }
        if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status) as u32
        } else if libc::WIFSIGNALED(status) {
            128 + libc::WTERMSIG(status) as u32
        } else {
            u32::MAX
        }
    }

    /// Every process of the session of the child: the child and what it started.
    pub fn job_process_ids(&self) -> Vec<u32> {
        let session = self.pid;
        proc_ids().into_iter().filter(|&pid| proc_stat(pid).is_some_and(|stat| stat.session == session)).collect()
    }

    /// The program the user is running now: the process of the session that started last, or the child itself when it
    /// started nothing. None when the processes cannot be read.
    pub fn job(&self) -> Option<Job> {
        self.job_with(&process_names()?)
    }

    /// `job` from a list of the processes of the system (`process_names`) taken once for every session looked at.
    pub fn job_with(&self, names: &[(u32, u32, String)]) -> Option<Job> {
        let name_of = |pid: u32| names.iter().find(|(id, _, _)| *id == pid).map(|(_, _, name)| name.clone());
        let pids = with_descendants(self.job_process_ids(), names);
        if pids.is_empty() {
            return None;
        }
        let mut newest: Option<(u64, u32, String)> = None;
        for pid in pids {
            if pid == self.pid {
                continue;
            }
            let Some(name) = name_of(pid) else { continue };
            let started = start_time(pid);
            if newest.as_ref().is_none_or(|(at, _, _)| started >= *at) {
                newest = Some((started, pid, name));
            }
        }
        let root = newest.is_none();
        let (pid, name) = newest.map(|(_, pid, name)| (pid, name)).or_else(|| Some((self.pid, name_of(self.pid)?)))?;
        Some(Job { name, path: image_path(pid), root })
    }

    /// Ends the child and every process of its session.
    pub fn kill(&self) {
        // SAFETY: plain calls on the ids of the child, which leads its own process group.
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
    }

    /// Nothing to release: the output stream reaches its end when the last process on the slave side is gone.
    pub fn close_console(&self) {}

    pub fn close_console_if_nonblocking(&self) {}
}

fn set_close_on_exec(fd: &OwnedFd) {
    // SAFETY: plain call on an open descriptor.
    unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
}

/// Directory a child starts in: `requested` when it is an existing directory, otherwise the home directory, otherwise
/// `None` (the child inherits this process's directory).
pub fn resolve_cwd(requested: Option<&Path>) -> Option<PathBuf> {
    requested.filter(|dir| dir.is_dir()).map(Path::to_path_buf).or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|dir| dir.is_dir()))
}

/// A command for a tool of the system (git, gh): on Unix the PATH of this process is the PATH of the panes.
pub fn tool(program: &str) -> Command {
    Command::new(program)
}

fn proc_ids() -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else { return Vec::new() };
    entries.flatten().filter_map(|entry| entry.file_name().to_str()?.parse().ok()).collect()
}

struct Stat {
    parent: u32,
    session: u32,
    /// Clock ticks after the boot.
    started: u64,
}

/// The fields of `/proc/<pid>/stat` that matter. The name in it is in parentheses and may hold any character, so the
/// rest is read after the last one.
fn proc_stat(pid: u32) -> Option<Stat> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = text.get(text.rfind(')')? + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the name: state, ppid, pgrp, session, … and the start time as the twentieth.
    Some(Stat { parent: fields.get(1)?.parse().ok()?, session: fields.get(3)?.parse().ok()?, started: fields.get(19)?.parse().ok()? })
}

/// Every process of the system: its id, the id of the process that started it, and the name of its program. None when
/// the system cannot be read.
pub fn process_names() -> Option<Vec<(u32, u32, String)>> {
    let names = proc_ids()
        .into_iter()
        .filter_map(|pid| {
            let parent = proc_stat(pid)?.parent;
            let name = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?.trim_end().to_owned();
            Some((pid, parent, name))
        })
        .collect::<Vec<_>>();
    (!names.is_empty()).then_some(names)
}

/// `pids` and every process started under them, from the processes of the system (`process_names`). A child counts
/// only when it started after its parent, so that a parent's id taken again by a later process brings in nothing.
pub fn with_descendants(mut pids: Vec<u32>, processes: &[(u32, u32, String)]) -> Vec<u32> {
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
    pub path: Option<PathBuf>,
    /// Nothing runs but the program the session started (the shell at its prompt).
    pub root: bool,
}

/// The path of the program file of a process.
pub fn image_path(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
}

fn start_time(pid: u32) -> u64 {
    proc_stat(pid).map_or(0, |stat| stat.started)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::time::{Duration, Instant};

    use super::*;

    /// Watches for a process to be gone.
    pub(crate) struct ProcessWatch(u32);

    impl ProcessWatch {
        pub(crate) fn open(pid: u32) -> Option<ProcessWatch> {
            Path::new(&format!("/proc/{pid}")).exists().then_some(ProcessWatch(pid))
        }

        pub(crate) fn exited_within(&self, timeout: Duration) -> bool {
            let deadline = Instant::now() + timeout;
            loop {
                // A child that ended and is not reaped yet is a zombie: gone as far as a test is concerned.
                let gone = fs::read_to_string(format!("/proc/{}/stat", self.0)).ok().and_then(|text| text.get(text.rfind(')')? + 2..)?.chars().next()).is_none_or(|state| state == 'Z');
                if gone {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
