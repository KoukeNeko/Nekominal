//! `updater.exe`: updates Nekominal with Scoop while the programs of its tabs keep running (`docs/updater-design.md`).
//!
//! The Nekominal where the user chose Update copies this program out of the app's folder, starts it with
//! `--pipe <name> --app <...\scoop\apps\nekominal> --version <its version>`, and hands its windows over on that pipe
//! (`nekominal_pty::handoff`). Then this program:
//!
//! 1. asks every other running Nekominal (through its automation pipe) to hand over too, and asks those that cannot (run
//!    as administrator, or without the API) to save their session and close;
//! 2. runs `scoop update` and `scoop update nekominal`, again for a while when Scoop finds something still running from
//!    the app's folder (the hooks of Claude Code run `Nekominal.exe` for a few milliseconds at a time);
//! 3. starts the Nekominal Scoop now runs with `--adopt <pipe>`, which takes the windows; when it does not (it fails to
//!    start, or to adopt), the version that handed over, which Scoop keeps;
//! 4. ends, once one has adopted, closing its copies of the handles; the sessions nobody adopted end with it.
//!
//! It is not named Nekominal: Scoop's manifest stops every process of that name before it installs.
#![windows_subsystem = "windows"]

mod holder;
mod scoop;
mod window;

use std::{
    io::Write,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use holder::Holder;
use window::{Ui, View};

/// How long the Nekominal that started the update has to hand over, another Nekominal to hand over or close, Scoop to
/// stop finding something running, and a started Nekominal to adopt the windows.
const GIVE_TIMEOUT: Duration = Duration::from_secs(30);
const OTHERS_TIMEOUT: Duration = Duration::from_secs(15);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(90);
const RUNNING_RETRY: Duration = Duration::from_secs(30);
const ADOPT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Default, PartialEq)]
struct Args {
    pipe: String,
    app: Option<PathBuf>,
    version: String,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--pipe" => parsed.pipe = value()?,
            "--app" => parsed.app = Some(PathBuf::from(value()?)).filter(|app| !app.as_os_str().is_empty()),
            "--version" => parsed.version = value()?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if parsed.pipe.is_empty() {
        return Err("--pipe is missing".into());
    }
    Ok(parsed)
}

/// What the window says, in the user's language: Chinese for a Chinese Windows, English otherwise, as the app.
struct Words {
    zh: bool,
}

impl Words {
    fn new() -> Words {
        // SAFETY: a plain query.
        let lang = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        Words { zh: lang & 0x3ff == 0x04 }
    }

    fn say(&self, en: &str, zh: &str) -> String {
        if self.zh { zh } else { en }.to_owned()
    }
}

fn log(text: &str) {
    if let Some(path) = std::env::var_os("NEKOMINAL_LOG")
        && let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(file, "updater: {text}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&args) {
        Ok(args) => args,
        Err(e) => {
            log(&e);
            return;
        }
    };
    // The pipe first: the Nekominal that started this connects as soon as it can.
    let holder = match Holder::serve(&args.pipe) {
        Ok(holder) => holder,
        Err(e) => {
            log(&format!("pipe {}: {e}", args.pipe));
            return;
        }
    };
    let view = Arc::new(Mutex::new(View { from: args.version.clone(), ..View::default() }));
    let shared = view.clone();
    window::run("Nekominal update", view, move |ui| update(&args, &holder, &shared, ui));
}

/// Changes the view and draws it again.
fn show(view: &Mutex<View>, ui: Ui, change: impl FnOnce(&mut View)) {
    change(&mut view.lock().unwrap());
    ui.refresh();
}

fn update(args: &Args, holder: &Arc<Holder>, view: &Mutex<View>, ui: Ui) {
    let words = Words::new();
    show(view, ui, |v| v.status = words.say("Taking over the tabs…", "正在接手分頁…"));
    if holder.wait_for_bundles(1, GIVE_TIMEOUT) == 0 {
        show(view, ui, |v| {
            v.status = words.say("Nekominal did not hand its tabs over; nothing was updated.", "Nekominal 沒有交出分頁，所以沒有更新。");
            v.failed = true;
        });
        return ui.finish(0);
    }
    let first = holder.givers()[0].clone();

    show(view, ui, |v| v.status = words.say("Asking the other Nekominal windows to hand over…", "正在請其他 Nekominal 視窗交接…"));
    let asked = ask_others(&args.pipe, first.0);
    holder.wait_for_bundles(1 + asked.len(), OTHERS_TIMEOUT);
    for process in &asked {
        process.exited_within(OTHERS_TIMEOUT);
    }
    if let Some(app) = &args.app {
        close_the_rest(app, view, ui, &words);
    }
    show(view, ui, |v| v.step = 1);

    let mut scoop_said = None;
    if let Some(app) = &args.app {
        scoop_said = run_scoop(app, view, ui, &words);
    }
    let installed = args.app.as_deref().and_then(scoop::installed);
    show(view, ui, |v| {
        v.step = 2;
        v.to = installed.clone().or_else(|| Some(args.version.clone()));
    });

    let (all, live) = holder.panes();
    log(&format!("holding {all} panes, {live} of them running"));
    let candidates = scoop::candidates(args.app.as_deref(), &first.2, Path::new(&first.1));
    let mut adopted = None;
    for exe in candidates.iter().filter(|exe| exe.is_file()) {
        let version = exe.parent().and_then(|dir| dir.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let version = if version.eq_ignore_ascii_case("current") { installed.clone().unwrap_or(version) } else { version };
        show(view, ui, |v| v.status = if words.zh { format!("正在啟動 Nekominal {version}…") } else { format!("Starting Nekominal {version}…") });
        if let Some(sessions) = start_adopting(exe, holder) {
            adopted = Some(sessions);
            break;
        }
        log(&format!("{} did not adopt the tabs", exe.display()));
    }

    match adopted {
        Some(sessions) => {
            let (kept, ended) = holder.finish(&sessions);
            log(&format!("adopted {kept}, ended {ended}"));
            let failed = scoop_said.is_some();
            show(view, ui, |v| {
                v.step = window::STEPS;
                v.status = match &scoop_said {
                    Some(_) => words.say(
                        "Scoop did not update Nekominal; it was started again with every tab.",
                        "Scoop 沒有更新 Nekominal；已帶著所有分頁重新啟動。",
                    ),
                    None => {
                        if words.zh { format!("完成：{kept} 個窗格的程式繼續執行。") } else { format!("Done: the programs of {kept} panes kept running.") }
                    }
                };
                v.failed = failed;
            });
            // What went wrong stays on screen until it is read; a clean update closes by itself.
            ui.finish(if failed { 0 } else { 2500 });
        }
        None => {
            holder.finish(&[]);
            show(view, ui, |v| {
                v.status = words.say(
                    "Nekominal could not be started again, so the programs of its tabs have ended.",
                    "無法重新啟動 Nekominal，分頁裡的程式已結束。",
                );
                v.failed = true;
            });
            ui.finish(0);
        }
    }
}

/// Asks every other Nekominal that runs the automation API to hand over to this updater: the processes asked.
fn ask_others(pipe: &str, first: u32) -> Vec<nekominal_pty::handoff::Process> {
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join(r"Nekominal\api")) else { return Vec::new() };
    let mut asked = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else { continue };
        if pid == first || pid == std::process::id() {
            continue;
        }
        let Ok(name) = std::fs::read_to_string(entry.path()) else { continue };
        let Ok(process) = nekominal_pty::handoff::Process::open(pid) else { continue };
        if process.exited_within(Duration::ZERO) {
            continue;
        }
        match nekominal_pty::handoff::connect(name.trim(), Duration::from_secs(2)) {
            Ok(mut api) => {
                // It hands over and exits; there is no answer to wait for.
                let request = serde_json::json!({ "cmd": "hand-off", "pipe": pipe });
                if writeln!(api, "{request}").is_ok() {
                    log(&format!("asked {pid} to hand over"));
                    asked.push(process);
                }
            }
            Err(e) => log(&format!("instance {pid}: {e}")),
        }
    }
    asked
}

/// A Nekominal that still runs from the app's folder could not hand over (it runs as administrator, or without the
/// API): asked to save its session and close, as an update did before, and waited for.
fn close_the_rest(app: &Path, view: &Mutex<View>, ui: Ui, words: &Words) {
    // The hooks of Claude Code run Nekominal.exe for a few milliseconds: only what is still there a moment later counts.
    let first = scoop::running_from(app);
    if first.is_empty() {
        return;
    }
    std::thread::sleep(Duration::from_secs(2));
    let staying: Vec<u32> = scoop::running_from(app).into_iter().filter(|pid| first.contains(pid)).collect();
    if staying.is_empty() {
        return;
    }
    // SAFETY: registering a message name is a plain call.
    let message = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(windows_sys::core::w!("NekominalUpdateClose")) };
    for pid in &staying {
        post_to_windows_of(*pid, message);
    }
    show(view, ui, |v| {
        v.status = if words.zh {
            format!("正在等 {} 個無法交接的 Nekominal 視窗關閉（以系統管理員身分執行的視窗會關閉，並用儲存的文字還原）…", staying.len())
        } else {
            format!("Waiting for {} Nekominal windows that cannot hand over to close (a window run as administrator closes, and its text is restored)…", staying.len())
        };
    });
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    while Instant::now() < deadline && scoop::running_from(app).iter().any(|pid| staying.contains(pid)) {
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Posts `message` to every top-level window of process `pid`.
fn post_to_windows_of(pid: u32, message: u32) {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW},
    };
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: lparam is the (pid, message) pair below, alive through EnumWindows.
        unsafe {
            let (pid, message) = *(lparam as *const (u32, u32));
            let mut owner = 0u32;
            GetWindowThreadProcessId(hwnd, &mut owner);
            if owner == pid {
                PostMessageW(hwnd, message, 0, 0);
            }
        }
        1
    }
    let pair = (pid, message);
    // SAFETY: the callback reads the pair, which outlives the call.
    unsafe { EnumWindows(Some(each), &pair as *const (u32, u32) as LPARAM) };
}

/// Runs the update with Scoop, showing its last line. None when the version changed, else what Scoop said last.
fn run_scoop(app: &Path, view: &Mutex<View>, ui: Ui, words: &Words) -> Option<String> {
    let before = scoop::installed(app);
    show(view, ui, |v| v.status = words.say("Updating with Scoop…", "正在用 Scoop 更新…"));
    let line = |text: &str| show(view, ui, |v| v.detail = text.to_owned());
    // The buckets first: without them the bucket on this computer may not know the new version yet.
    if let Err(e) = scoop::run(app, "update", line) {
        log(&format!("scoop update: {e}"));
    }
    let started = Instant::now();
    let last = loop {
        match scoop::run(app, "update nekominal", line) {
            Ok(output) => {
                if !scoop::refused_as_running(&output) || started.elapsed() > RUNNING_RETRY {
                    break output.last().cloned().unwrap_or_default();
                }
                show(view, ui, |v| v.status = words.say("Something still runs from the Nekominal folder; trying again…", "Nekominal 資料夾裡仍有程式在執行，稍後再試…"));
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(e) => break format!("scoop: {e}"),
        }
    };
    let after = scoop::installed(app);
    log(&format!("scoop: {before:?} -> {after:?}: {last}"));
    (after.is_none() || after == before).then_some(last)
}

/// Starts `exe` to adopt the windows. The sessions it adopted; None when it did not start, ended, or did not answer in
/// time (it is then ended, so that two never read one session).
fn start_adopting(exe: &Path, holder: &Holder) -> Option<Vec<String>> {
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let start = |flags: u32| std::process::Command::new(exe).args(["--adopt", &holder.pipe]).current_dir(&home).creation_flags(flags).spawn();
    // Out of any job this updater runs in, so that what ends that job does not end the new Nekominal.
    let mut child = match start(CREATE_BREAKAWAY_FROM_JOB | DETACHED_PROCESS).or_else(|_| start(DETACHED_PROCESS)) {
        Ok(child) => child,
        Err(e) => {
            log(&format!("{}: {e}", exe.display()));
            return None;
        }
    };
    holder.expect_taker(child.id());
    let adopted = holder.wait_adopted(ADOPT_TIMEOUT, || child.try_wait().ok().flatten().is_some());
    if adopted.is_none() {
        let _ = child.kill();
    }
    adopted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arguments_name_the_pipe_and_the_app_and_nothing_else_is_taken() {
        let args = |list: &[&str]| parse(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(
            args(&["--pipe", r"\\.\pipe\x", "--app", r"C:\s\apps\nekominal", "--version", "0.2.3"]).unwrap(),
            Args { pipe: r"\\.\pipe\x".into(), app: Some(PathBuf::from(r"C:\s\apps\nekominal")), version: "0.2.3".into() }
        );
        assert_eq!(args(&["--pipe", "p", "--app", ""]).unwrap().app, None, "an empty folder is none");
        assert_eq!(args(&["--app", "a"]).unwrap_err(), "--pipe is missing");
        assert_eq!(args(&["--pipe"]).unwrap_err(), "--pipe needs a value");
        assert_eq!(args(&["--pipe", "p", "--force"]).unwrap_err(), "unknown argument --force");
    }
}
