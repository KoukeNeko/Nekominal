//! Claude Code and the status bar: where its numbers come from.
//!
//! Claude Code runs a `statusLine` command whenever its session changes and hands it the session as JSON on stdin. In a pane
//! of Nekominal that command is this program with `--statusline`: it writes what it is handed to a file named by the pane
//! it runs in (`NEKOMINAL_SESSION`, set for every pane), and the window reads the file of the focused pane once a second.
//! Nothing leaves the machine, and Claude Code's own settings are never written: the command reaches it only through
//! `--settings`, which the `claude` function of the PowerShell integration adds when `NEKOMINAL_CLAUDE_SETTINGS` names a
//! file. The window sets that variable for a new pane while the status bar is on, unless Claude Code has a status line of
//! the user's own, which it would replace.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

/// The variable that names the pane a program runs in, for the command Claude Code runs.
pub const SESSION_VAR: &str = "NEKOMINAL_SESSION";
/// The argument that makes this program the `statusLine` command instead of a window.
const FLAG: &str = "--statusline";
/// The variable that names the settings file the `claude` function of the PowerShell integration hands to Claude Code.
pub const CLAUDE_SETTINGS_VAR: &str = "NEKOMINAL_CLAUDE_SETTINGS";

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A name for a new pane, unique on this machine while the process lives.
pub fn new_session() -> String {
    format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Where the file of a pane is; `None` for a name this program did not hand out.
fn usage_file(session: &str) -> Option<PathBuf> {
    if session.is_empty() || !session.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return None;
    }
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("Nekominal").join("agents").join(format!("{session}.json")))
}

/// Run as the `statusLine` command when the program was started with `--statusline`: keeps what Claude Code hands over
/// for the window and prints nothing (the window shows it). Returns whether the program was started for that.
pub fn statusline_main() -> bool {
    if std::env::args().nth(1).as_deref() != Some(FLAG) {
        return false;
    }
    // One JSON value, read up to its end and not to the end of the stream: nothing then hangs on a caller that keeps stdin
    // open after writing.
    let mut values = serde_json::Deserializer::from_reader(std::io::stdin().lock()).into_iter::<serde_json::Value>();
    let Some(Ok(value)) = values.next() else { return true };
    let input = value.to_string();
    if let Some(file) = std::env::var(SESSION_VAR).ok().as_deref().and_then(usage_file)
        && let Some(dir) = file.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        // Written beside and moved over, so that the window never reads half a file.
        let part = file.with_extension("part");
        if std::fs::write(&part, input).is_ok() {
            let _ = std::fs::rename(&part, &file);
        }
    }
    true
}

/// What the agent of a pane last said about its session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub model: Option<String>,
    /// The share of the context window in use, 0 to 100.
    pub context: Option<f64>,
    pub cost: Option<f64>,
    pub duration_ms: Option<u64>,
    pub cwd: Option<String>,
}

/// The fields of the JSON Claude Code hands its `statusLine` command that the bar shows.
pub fn parse_usage(json: &str) -> Option<Usage> {
    let value: serde_json::Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    let text = |pointer: &str| value.pointer(pointer).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
    let number = |pointer: &str| value.pointer(pointer).and_then(serde_json::Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0);
    Some(Usage {
        model: text("/model/display_name"),
        context: number("/context_window/used_percentage").map(|n| n.min(100.0)),
        cost: number("/cost/total_cost_usd"),
        duration_ms: number("/cost/total_duration_ms").map(|n| n as u64),
        cwd: text("/workspace/current_dir").or_else(|| text("/cwd")),
    })
}

/// When the file of a pane was last written, to tell whether it has to be read again.
pub fn usage_stamp(session: &str) -> Option<SystemTime> {
    std::fs::metadata(usage_file(session)?).ok()?.modified().ok()
}

pub fn read_usage(session: &str) -> Option<Usage> {
    parse_usage(&std::fs::read_to_string(usage_file(session)?).ok()?)
}

/// Takes the file of a pane that is gone.
pub fn forget(session: &str) {
    if let Some(file) = usage_file(session) {
        let _ = std::fs::remove_file(file);
    }
}

/// Whether `dir` is on a disk of this machine: a drive letter whose drive is fixed, removable, optical or in memory. This
/// runs every second on the thread of the window, so nothing else is looked at: a share (`\\host\x`) makes Windows connect
/// and hand over the user's credentials, and a mapped drive whose server sleeps blocks for seconds.
fn on_local_disk(dir: &Path) -> bool {
    let text = dir.to_string_lossy();
    let mut chars = text.chars();
    let (Some(letter), Some(':')) = (chars.next(), chars.next()) else { return false };
    if !letter.is_ascii_alphabetic() {
        return false;
    }
    // SAFETY: a NUL-terminated root path that lives through the call.
    let kind = unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(&windows::core::HSTRING::from(format!("{letter}:\\"))) };
    // DRIVE_REMOVABLE, DRIVE_FIXED, DRIVE_CDROM, DRIVE_RAMDISK; not DRIVE_REMOTE (4) or one that is not there.
    matches!(kind, 2 | 3 | 5 | 6)
}

/// The branch the repository that holds `dir` is on, or the start of the commit when no branch is checked out.
pub fn git_branch(dir: &Path) -> Option<String> {
    if !on_local_disk(dir) {
        return None;
    }
    for folder in dir.ancestors() {
        let git = folder.join(".git");
        let head = if git.is_dir() {
            git.join("HEAD")
        } else if git.is_file() {
            // A worktree or a submodule: `.git` is a file that says where the repository is.
            let pointer = std::fs::read_to_string(&git).ok()?;
            let target = folder.join(pointer.strip_prefix("gitdir:")?.trim());
            // The file can name any folder, a share among them: the same rule holds for it.
            if !on_local_disk(&target) {
                return None;
            }
            target.join("HEAD")
        } else {
            continue;
        };
        let head = std::fs::read_to_string(head).ok()?;
        let head = head.trim();
        return Some(match head.strip_prefix("ref:") {
            Some(reference) => reference.trim().strip_prefix("refs/heads/").unwrap_or(reference.trim()).to_owned(),
            None => head.chars().take(7).collect(),
        });
    }
    None
}

/// Claude Code as this machine has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Claude {
    pub exe: Option<PathBuf>,
    pub version: Option<String>,
    /// The user's settings of Claude Code give it a status line of their own, which Nekominal leaves alone.
    pub own_status_line: bool,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// Claude Code's settings of the user: `CLAUDE_CONFIG_DIR` or `~/.claude`.
fn claude_settings() -> Option<PathBuf> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| home().map(|h| h.join(".claude")))?;
    Some(dir.join("settings.json"))
}

/// Looks for Claude Code: `claude.exe` on the path, then where its own installer puts it.
pub fn claude() -> Claude {
    let on_path = std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|dir| dir.join("claude.exe")).find(|exe| exe.is_file()));
    let installed = home().map(|h| h.join(".local").join("bin").join("claude.exe")).filter(|exe| exe.is_file());
    // The native installer keeps every version it has in a folder of its own, named by the version.
    let version = home()
        .and_then(|h| std::fs::read_dir(h.join(".local").join("share").join("claude").join("versions")).ok())
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.split('.').all(|part| part.parse::<u32>().is_ok()))
        .max_by_key(|name| name.split('.').map(|part| part.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>());
    let text = claude_settings().and_then(|file| std::fs::read_to_string(file).ok()).unwrap_or_default();
    Claude { exe: on_path.or(installed), version, own_status_line: has_status_line(&text) }
}

/// Whether Claude Code's settings `text` set a status line; a file that cannot be read sets none.
fn has_status_line(text: &str) -> bool {
    let value: Option<serde_json::Value> = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok();
    value.as_ref().and_then(|v| v.get("statusLine")).is_some_and(|line| !line.is_null())
}

/// The settings Claude Code takes with `--settings` in a pane: a status line that is this program. Forward slashes,
/// because Claude Code runs the command through Git Bash when it can, which eats backslashes; quotes for a path with spaces.
fn status_line_settings(exe: &Path) -> String {
    let exe = exe.to_string_lossy().replace('\\', "/");
    let command = if exe.contains(' ') { format!("\"{exe}\" {FLAG}") } else { format!("{exe} {FLAG}") };
    serde_json::json!({ "statusLine": { "type": "command", "command": command } }).to_string()
}

/// Writes the file `NEKOMINAL_CLAUDE_SETTINGS` names and returns where it is: next to the files of the panes, rewritten
/// when this program moved (another build, an update).
pub fn claude_settings_file() -> Option<PathBuf> {
    let file = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Nekominal").join("claude-settings.json");
    let text = status_line_settings(&std::env::current_exe().ok()?);
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        std::fs::create_dir_all(file.parent()?).ok()?;
        std::fs::write(&file, &text).ok()?;
    }
    Some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_line_json_gives_the_numbers_of_the_bar() {
        let json = r#"{"session_id":"abc","model":{"id":"claude-opus","display_name":"Opus"},"workspace":{"current_dir":"C:\\work\\repo"},
            "cost":{"total_cost_usd":0.8412,"total_duration_ms":134000},"context_window":{"context_window_size":200000,"used_percentage":42}}"#;
        let usage = parse_usage(json).unwrap();
        assert_eq!(usage.model.as_deref(), Some("Opus"));
        assert_eq!(usage.context, Some(42.0));
        assert_eq!(usage.cost, Some(0.8412));
        assert_eq!(usage.duration_ms, Some(134_000));
        assert_eq!(usage.cwd.as_deref(), Some("C:\\work\\repo"));
        assert_eq!(parse_usage(r#"{"model":{}}"#), Some(Usage::default()), "an older version without the fields shows nothing");
        assert_eq!(parse_usage("not json"), None);
        assert_eq!(parse_usage(r#"{"context_window":{"used_percentage":250}}"#).unwrap().context, Some(100.0));
    }

    #[test]
    fn a_pane_name_is_only_ever_one_this_program_hands_out() {
        assert!(new_session().bytes().all(|b| b.is_ascii_digit() || b == b'-'));
        assert_ne!(new_session(), new_session());
        assert!(usage_file("..\\..\\evil").is_none());
        assert!(usage_file("").is_none());
    }

    #[test]
    fn a_status_line_of_the_user_s_own_is_found_and_left_alone() {
        assert!(!has_status_line(""));
        assert!(!has_status_line(r#"{"theme":"dark"}"#));
        assert!(!has_status_line(r#"{"statusLine": null}"#));
        assert!(has_status_line(r#"{"statusLine":{"type":"command","command":"~/.claude/line.sh"}}"#));
        assert!(has_status_line("\u{feff}{\"statusLine\":{\"command\":\"x\"}}"));
        assert!(!has_status_line("{\"a\": 1,}"), "a file Claude Code cannot read either sets nothing");
    }

    #[test]
    fn the_settings_of_a_pane_run_this_program_with_forward_slashes() {
        let plain: serde_json::Value = serde_json::from_str(&status_line_settings(Path::new(r"C:\n\nekominal.exe"))).unwrap();
        assert_eq!(plain["statusLine"]["command"], "C:/n/nekominal.exe --statusline");
        assert_eq!(plain["statusLine"]["type"], "command");
        let spaced: serde_json::Value = serde_json::from_str(&status_line_settings(Path::new(r"C:\Program Files\N\n.exe"))).unwrap();
        assert_eq!(spaced["statusLine"]["command"], "\"C:/Program Files/N/n.exe\" --statusline");
    }

    #[test]
    fn the_branch_is_read_from_the_head_of_the_repository_above_the_folder() {
        let root = std::env::temp_dir().join(format!("nekominal-branch-{}", new_session()));
        let deep = root.join("src").join("inner");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/feature/bar\n").unwrap();
        assert_eq!(git_branch(&deep).as_deref(), Some("feature/bar"));
        std::fs::write(root.join(".git").join("HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(git_branch(&root).as_deref(), Some("0123456"));
        // A worktree: `.git` names the folder of the repository that holds its HEAD.
        let tree = root.join("tree");
        std::fs::create_dir_all(root.join("wt")).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join(".git"), "gitdir: ../wt\n").unwrap();
        std::fs::write(root.join("wt").join("HEAD"), "ref: refs/heads/side\n").unwrap();
        assert_eq!(git_branch(&tree).as_deref(), Some("side"));
        assert_eq!(git_branch(Path::new(r"\\nas\share\repo")), None, "a share is not looked at");
        assert_eq!(git_branch(Path::new("//nas/share/repo")), None);
        assert_eq!(git_branch(Path::new("/home/me")), None, "a path of another system");
        // A `.git` file that points at a share, as an archive can hold one.
        std::fs::write(tree.join(".git"), "gitdir: \\\\host\\x\n").unwrap();
        assert_eq!(git_branch(&tree), None, "the folder a .git file names is held to the same rule");
        assert!(on_local_disk(&root) && !on_local_disk(Path::new("relative\\dir")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
