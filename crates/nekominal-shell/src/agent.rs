//! Claude Code and the status bar: where its numbers come from.
//!
//! Claude Code runs a `statusLine` command whenever its session changes and hands it the session as JSON on stdin. In a pane
//! of Nekominal that command is this program with `--statusline`: it writes what it is handed to a file named by the pane
//! it runs in (`NEKOMINAL_SESSION`, set for every pane), and the window reads the file of the focused pane once a second.
//! Nothing leaves the machine, and Claude Code's own settings are never written: the command reaches it only through
//! `--settings`, which the `claude` function (a doskey macro in cmd) of the shell integration adds when
//! `NEKOMINAL_CLAUDE_SETTINGS` names a file. The window sets that variable for a new pane while the status bar is on, unless
//! Claude Code has a status line of the user's own, which it would replace: then the file names hooks only. In WSL,
//! `WSLENV` carries both variables in and the pane back out to this program.
//!
//! The status line can be taken from us: a plugin's, or the user's own, wins over it, and then Claude Code never runs this
//! program. So the same file also gives Claude Code hooks (`--claude-hook`), which it merges with every other source's and
//! so always runs: they note where the session's transcript is (`<pane>.claude`), and the window reads the model and the
//! tokens of the last answer from the end of that transcript whenever the status line has not said what the context is.

use std::{
    io::{Read as _, Seek as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

/// The variable that names the pane a program runs in, for the command Claude Code runs.
pub const SESSION_VAR: &str = "NEKOMINAL_SESSION";
/// The argument that makes this program the `statusLine` command instead of a window.
const FLAG: &str = "--statusline";
/// The argument that makes this program a hook of Claude Code instead of a window.
const HOOK_FLAG: &str = "--claude-hook";
/// The extension of the file of a pane that holds what the hooks of Claude Code last said about its session.
const HOOK_EXTENSION: &str = "claude";
/// The events whose hooks note the session: it starts (or is resumed, cleared, compacted), and its turns start and end.
/// Not `PostToolUse`: it also runs for every tool of every subagent, which started this program hundreds of times in one
/// workflow, and the session it notes is the one the other three already did.
const HOOK_EVENTS: &[&str] = &["SessionStart", "UserPromptSubmit", "Stop"];
/// How much of the end of a transcript is read for its last answer: a few answers' worth, read in well under a millisecond.
const TRANSCRIPT_TAIL: u64 = 256 * 1024;
/// The variable that names the settings file the `claude` function of the shell integration hands to Claude Code.
pub const CLAUDE_SETTINGS_VAR: &str = "NEKOMINAL_CLAUDE_SETTINGS";

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A name for a new pane, unique on this machine while the process lives.
pub fn new_session() -> String {
    format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// A name a pane that came from another process keeps (an update hands the panes over): the names this process hands out
/// from now on are not it. Only a name of this process's own id can meet them, when Windows gave it the id of the one
/// that handed over.
pub fn reserve_session(session: &str) {
    let mine = format!("{}-", std::process::id());
    if let Some(n) = session.strip_prefix(&mine).and_then(|n| n.parse::<u64>().ok()) {
        NEXT.fetch_max(n + 1, Ordering::Relaxed);
    }
}

/// Where the file of a pane is; `None` for a name this program did not hand out.
fn usage_file(session: &str) -> Option<PathBuf> {
    session_file(session, "json")
}

/// A file of a pane with this extension (`json` for Claude Code's status line, `hook` for the hooks of the other agents).
pub fn session_file(session: &str, extension: &str) -> Option<PathBuf> {
    if session.is_empty() || !session.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return None;
    }
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("Nekominal").join("agents").join(format!("{session}.{extension}")))
}

/// Writes beside and moves over, so that the window never reads half a file, and flushed to disk before the move, so that a
/// crash leaves the old file or the new one, not an empty one. The name of the part is this process's: two commands that
/// run at once (a status line and a hook) do not write into one another's. Whether the file was written.
pub fn write_whole(file: &Path, text: &str) -> bool {
    let Some(dir) = file.parent() else { return false };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let part = file.with_extension(format!("{}.part", std::process::id()));
    let written = std::fs::File::create(&part)
        .and_then(|mut out| std::io::Write::write_all(&mut out, text.as_bytes()).and_then(|()| out.sync_all()))
        .and_then(|()| std::fs::rename(&part, file));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written.is_ok()
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
    if let Some(file) = std::env::var(SESSION_VAR).ok().as_deref().and_then(usage_file) {
        write_whole(&file, &value.to_string());
    }
    if let (Some(model), Some(size)) = (value.pointer("/model/id").and_then(|v| v.as_str()), value.pointer("/context_window/context_window_size").and_then(serde_json::Value::as_u64)) {
        remember_window(model, size);
    }
    crate::spend::record(&value);
    true
}

/// Run as a hook of Claude Code when the program was started with `--claude-hook`: notes where the session's transcript is
/// for the window, and prints nothing (Claude Code would add what a `SessionStart` or `UserPromptSubmit` hook prints to the
/// conversation). It exits 0 whatever happens, and writes only when something changed: it runs after every tool.
pub fn claude_hook_main() -> bool {
    if std::env::args().nth(1).as_deref() != Some(HOOK_FLAG) {
        return false;
    }
    let mut values = serde_json::Deserializer::from_reader(std::io::stdin().lock()).into_iter::<serde_json::Value>();
    let Some(Ok(input)) = values.next() else { return true };
    if let Some(file) = std::env::var(SESSION_VAR).ok().and_then(|s| session_file(&s, HOOK_EXTENSION)) {
        let old = std::fs::read_to_string(&file).ok();
        let bridge = std::env::var(BRIDGE_VAR).ok();
        if let Some(text) = hook_record(&input, old.as_deref(), bridge.as_deref()) {
            write_if_changed(&file, &text);
        }
    }
    true
}

/// The variable Claude Code sets for its hooks while the session has a Remote Control connection: the session's id on
/// claude.ai (`session_…`), the one in its `claude.ai/code` link. Gone when the connection ends.
const BRIDGE_VAR: &str = "CLAUDE_CODE_BRIDGE_SESSION_ID";

/// The Remote Control id of `bridge` when it is one: `session_` and letters, digits or `_`, nothing that could make the
/// link it goes into lead elsewhere.
pub fn remote_id(bridge: &str) -> Option<&str> {
    let rest = bridge.strip_prefix("session_")?;
    (!rest.is_empty() && rest.len() <= 64 && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')).then_some(bridge)
}

/// The link of a Remote Control session on claude.ai.
pub fn remote_link(id: &str) -> String {
    format!("https://claude.ai/code/{id}")
}

/// What the file of a pane keeps of a hook's input: the session, its transcript and folder, the model when the event
/// names one (`SessionStart` does in recent versions), and the Remote Control session while one is connected (`bridge`,
/// from the hook's environment; none is no connection as of this event). A model said before is kept while the session
/// is the same one.
fn hook_record(input: &serde_json::Value, old: Option<&str>, bridge: Option<&str>) -> Option<String> {
    let text = |value: &serde_json::Value, key: &str| value.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
    let session = text(input, "session_id");
    let transcript = text(input, "transcript_path")?;
    // A model is a string in some versions and `{id, display_name}` in others.
    let model = input.get("model").and_then(|m| m.as_str().map(str::to_owned).or_else(|| text(m, "display_name")).or_else(|| text(m, "id")));
    let old: Option<serde_json::Value> = old.and_then(|o| serde_json::from_str(o).ok());
    let model = model.or_else(|| old.as_ref().filter(|o| text(o, "session_id") == session).and_then(|o| text(o, "model")));
    let remote = bridge.and_then(remote_id);
    Some(serde_json::json!({ "session_id": session, "transcript_path": transcript, "cwd": text(input, "cwd"), "model": model, "remote": remote }).to_string())
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
    /// The tokens of the last request, whose sum of the first three is the context in use, and of its answer.
    pub input_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// The size of the context window the share is of.
    pub context_window: Option<u64>,
    /// The whole session's tokens, lines and time waiting for the API, as the status line has them.
    pub total_input_tokens: Option<u64>,
    pub total_output_tokens: Option<u64>,
    pub lines_added: Option<u64>,
    pub lines_removed: Option<u64>,
    pub api_duration_ms: Option<u64>,
    /// The session's Remote Control id (`session_…`) while it was connected at the last hook.
    pub remote: Option<String>,
}

/// The fields of the JSON Claude Code hands its `statusLine` command that the bar shows. `used_percentage` and
/// `current_usage` are null until the first answer.
pub fn parse_usage(json: &str) -> Option<Usage> {
    let value: serde_json::Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    let text = |pointer: &str| value.pointer(pointer).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
    let number = |pointer: &str| value.pointer(pointer).and_then(serde_json::Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0);
    let count = |pointer: &str| number(pointer).map(|n| n as u64);
    Some(Usage {
        model: text("/model/display_name"),
        context: number("/context_window/used_percentage").map(|n| n.min(100.0)),
        cost: number("/cost/total_cost_usd"),
        duration_ms: count("/cost/total_duration_ms"),
        cwd: text("/workspace/current_dir").or_else(|| text("/cwd")),
        input_tokens: count("/context_window/current_usage/input_tokens"),
        cache_creation_tokens: count("/context_window/current_usage/cache_creation_input_tokens"),
        cache_read_tokens: count("/context_window/current_usage/cache_read_input_tokens"),
        output_tokens: count("/context_window/current_usage/output_tokens"),
        context_window: count("/context_window/context_window_size").filter(|&n| n > 0),
        total_input_tokens: count("/context_window/total_input_tokens"),
        total_output_tokens: count("/context_window/total_output_tokens"),
        lines_added: count("/cost/total_lines_added"),
        lines_removed: count("/cost/total_lines_removed"),
        api_duration_ms: count("/cost/total_api_duration_ms"),
        remote: None,
    })
}

/// When the files the usage of a pane comes from were last written: the status line's, the hooks', and the transcript the
/// hooks named. Read every second, so the transcript is looked at only on a disk of this machine (a path of WSL is not).
pub type Stamp = [Option<SystemTime>; 3];

fn modified(file: &Path) -> Option<SystemTime> {
    std::fs::metadata(file).ok()?.modified().ok()
}

/// The stamp of a pane's files, `None` while Claude Code has said nothing in it.
pub fn usage_stamp(session: &str) -> Option<Stamp> {
    let line = modified(&usage_file(session)?);
    let hook_file = session_file(session, HOOK_EXTENSION)?;
    let hook = modified(&hook_file);
    let transcript = hook.and_then(|_| read_hook(&hook_file)).and_then(|record| hook_transcript(&record)).and_then(|t| modified(&t));
    (line.is_some() || hook.is_some()).then_some([line, hook, transcript])
}

fn read_hook(file: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// The transcript the hooks of a pane named, when it is on a disk of this machine.
// shortcut: a WSL pane's transcript is a Linux path and is skipped; map it to \\wsl.localhost\<distro> if WSL users ask.
fn hook_transcript(record: &serde_json::Value) -> Option<PathBuf> {
    Some(PathBuf::from(record.get("transcript_path")?.as_str()?)).filter(|path| on_local_disk(path))
}

/// What the status line last said of a pane's session, with what the transcript says filling in a context it has not
/// said: no status line ran (another took its place), or it ran before the first answer.
pub fn read_usage(session: &str) -> Option<Usage> {
    let line_file = usage_file(session)?;
    let line_json = std::fs::read_to_string(&line_file).ok();
    let line = line_json.as_deref().and_then(parse_usage);
    let hook = read_hook(&session_file(session, HOOK_EXTENSION)?);
    if line.is_none() && hook.is_none() {
        return None;
    }
    let text = |value: Option<&serde_json::Value>, key: &str| value.and_then(|v| v.get(key)).and_then(|v| v.as_str()).map(str::to_owned);
    let line_value: Option<serde_json::Value> = line_json.as_deref().and_then(|json| serde_json::from_str(json.trim_start_matches('\u{feff}')).ok());
    let transcript = hook.as_ref().and_then(hook_transcript);
    let turn = transcript.as_ref().and_then(|t| read_tail(t)).and_then(|(tail, cut)| last_turn(&tail, cut));
    let line = line.map(|usage| (usage, text(line_value.as_ref(), "session_id"), modified(&line_file)));
    let mut usage = combine(line, text(hook.as_ref(), "session_id"), turn.map(|turn| (turn, transcript.as_deref().and_then(modified))));
    usage.cwd = usage.cwd.or_else(|| text(hook.as_ref(), "cwd"));
    usage.model = usage.model.or_else(|| text(hook.as_ref(), "model").map(|m| model_name(&m)));
    usage.remote = text(hook.as_ref(), "remote").filter(|id| remote_id(id).is_some());
    Some(usage)
}

/// What the bar shows from the two sources: the status line, which runs only now and then when another one (a plugin's)
/// takes its place, and the transcript the hooks named, which is written at every answer. The context comes from the
/// newer of them; what only the status line knows (cost, time, lines, totals, the window) is kept from its last run. A
/// status line of another session than the one the hooks follow is the history of the pane, and left out.
fn combine(line: Option<(Usage, Option<String>, Option<SystemTime>)>, session: Option<String>, turn: Option<(Turn, Option<SystemTime>)>) -> Usage {
    let line = line.filter(|(_, of, _)| session.is_none() || of.is_none() || *of == session);
    let line_at = line.as_ref().and_then(|(_, _, at)| *at);
    let mut usage = line.map(|(usage, _, _)| usage).unwrap_or_default();
    if let Some((turn, at)) = turn {
        let newer = usage.context.is_none() || match (at, line_at) {
            (Some(at), Some(line_at)) => at > line_at,
            _ => true,
        };
        if newer {
            let window = usage.context_window.unwrap_or_else(|| window_size(known_window(&turn.model), turn.context()));
            fill_from_turn(&mut usage, &turn, window);
        }
    }
    usage
}

/// The last answer of a session's main conversation, as its transcript has it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Turn {
    model: String,
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
}

impl Turn {
    /// The context the request of the answer took: what it sent, written to the cache, and read from it.
    fn context(&self) -> u64 {
        self.input + self.cache_creation + self.cache_read
    }
}

/// The end of a transcript, and whether it was cut (its first line then is most likely a part of one).
fn read_tail(file: &Path) -> Option<(String, bool)> {
    let mut input = std::fs::File::open(file).ok()?;
    let length = input.metadata().ok()?.len();
    let start = length.saturating_sub(TRANSCRIPT_TAIL);
    input.seek(std::io::SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).ok()?;
    Some((String::from_utf8_lossy(&bytes).into_owned(), start > 0))
}

/// The last answer in the lines of a transcript with tokens: not a subagent's (`isSidechain`), and not one Claude Code
/// made up itself (an error, an interruption: model `<synthetic>`, no usage worth the name).
fn last_turn(tail: &str, cut: bool) -> Option<Turn> {
    let whole = if cut { tail.split_once('\n').map_or("", |(_, rest)| rest) } else { tail };
    whole.lines().rev().find_map(|line| {
        let entry: serde_json::Value = serde_json::from_str(line).ok()?;
        if entry.get("type")?.as_str()? != "assistant" || entry.get("isSidechain").and_then(serde_json::Value::as_bool) == Some(true) {
            return None;
        }
        let message = entry.get("message")?;
        let model = message.get("model")?.as_str().filter(|m| !m.is_empty() && *m != "<synthetic>")?;
        let usage = message.get("usage")?;
        let count = |key: &str| usage.get(key).and_then(serde_json::Value::as_u64).unwrap_or(0);
        let turn = Turn {
            model: model.to_owned(),
            input: count("input_tokens"),
            cache_creation: count("cache_creation_input_tokens"),
            cache_read: count("cache_read_input_tokens"),
            output: count("output_tokens"),
        };
        (turn.context() > 0).then_some(turn)
    })
}

/// What the status bar shows of an answer, against a window of `window` tokens.
fn fill_from_turn(usage: &mut Usage, turn: &Turn, window: u64) {
    // The newer answer's model: the session may have changed it since the status line last ran.
    usage.model = Some(model_name(&turn.model));
    usage.context = Some((turn.context() as f64 * 100.0 / window as f64).min(100.0));
    usage.context_window = Some(window);
    usage.input_tokens = Some(turn.input);
    usage.cache_creation_tokens = Some(turn.cache_creation);
    usage.cache_read_tokens = Some(turn.cache_read);
    usage.output_tokens = Some(turn.output);
}

/// The window a model's context is measured against: the one the status line once gave for it, else the usual 200k,
/// unless more than that is in use already (a 1M window, whose model id is the same).
fn window_size(known: Option<u64>, used: u64) -> u64 {
    known.filter(|&size| size >= used).unwrap_or(if used > 200_000 { 1_000_000 } else { 200_000 })
}

/// The name a model goes by on the bar, from its id: `claude-opus-4-1-20250805` is "Opus 4.1"; another id stays as it is.
fn model_name(id: &str) -> String {
    let Some(rest) = id.strip_prefix("claude-") else { return id.to_owned() };
    let parts: Vec<&str> = rest.split('-').filter(|part| !part.is_empty()).collect();
    let names: Vec<String> = parts
        .iter()
        .filter(|part| !part.bytes().all(|b| b.is_ascii_digit()))
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
        })
        .collect();
    // A date (`20250805`) is not part of the version.
    let version: Vec<&str> = parts.iter().copied().filter(|part| part.bytes().all(|b| b.is_ascii_digit()) && part.len() < 8).collect();
    if names.is_empty() { id.to_owned() } else { format!("{} {}", names.join(" "), version.join(".")).trim_end().to_owned() }
}

/// The windows the status line gave per model id, kept for the transcript's answers: `context-windows.json`.
fn windows_file() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Nekominal").join("context-windows.json"))
}

fn known_window(model: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(windows_file()?).ok()?).ok()?;
    value.get(model)?.as_u64().filter(|&n| n > 0)
}

fn remember_window(model: &str, size: u64) {
    let Some(file) = windows_file().filter(|_| size > 0 && known_window(model) != Some(size)) else { return };
    let mut known: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    known.insert(model.to_owned(), size.into());
    write_whole(&file, &serde_json::Value::Object(known).to_string());
}

/// Takes the files of a pane that is gone.
pub fn forget(session: &str) {
    for extension in ["json", HOOK_EXTENSION, crate::hooks::EXTENSION] {
        if let Some(file) = session_file(session, extension) {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// How long the file of a pane is kept unwritten: a pane's files go with it, but a Nekominal that crashed or was ended
/// leaves them. Not by whether their process runs: after an update, tabs live on under the name of the process that
/// handed them over.
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

/// Deletes the files of panes (and the parts of their writes) no one has written for `STALE_AFTER`.
pub fn clean_stale() {
    let Some(dir) = session_file("0", "json").and_then(|file| file.parent().map(Path::to_path_buf)) else { return };
    clean_stale_in(&dir, SystemTime::now());
}

fn clean_stale_in(dir: &Path, now: SystemTime) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = entry.metadata().and_then(|meta| meta.modified()).is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age > STALE_AFTER));
        let a_pane = entry.file_name().to_string_lossy().split('.').next().is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit() || b == b'-'));
        if old && a_pane {
            let _ = std::fs::remove_file(entry.path());
        }
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

/// The repository that holds a folder, as `git_branch` found it: the folder with its `.git`, the branch it is on, and the
/// text of its `.git` when that is a file (a worktree, a submodule). Kept with the pane, for the badges to use without
/// looking at the disk again.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkout {
    pub root: PathBuf,
    pub branch: String,
    pub git_file: Option<String>,
}

/// The branch the repository that holds `dir` is on, or the start of the commit when no branch is checked out.
pub fn git_branch(dir: &Path) -> Option<Checkout> {
    if !on_local_disk(dir) {
        return None;
    }
    for folder in dir.ancestors() {
        let git = folder.join(".git");
        let (head, git_file) = if git.is_dir() {
            (git.join("HEAD"), None)
        } else if git.is_file() {
            // A worktree or a submodule: `.git` is a file that says where the repository is.
            let pointer = std::fs::read_to_string(&git).ok()?;
            let target = folder.join(pointer.strip_prefix("gitdir:")?.trim());
            // The file can name any folder, a share among them: the same rule holds for it.
            if !on_local_disk(&target) {
                return None;
            }
            (target.join("HEAD"), Some(pointer))
        } else {
            continue;
        };
        let head = std::fs::read_to_string(head).ok()?;
        let head = head.trim();
        let branch = match head.strip_prefix("ref:") {
            Some(reference) => reference.trim().strip_prefix("refs/heads/").unwrap_or(reference.trim()).to_owned(),
            None => head.chars().take(7).collect(),
        };
        return Some(Checkout { root: folder.to_path_buf(), branch, git_file });
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

/// This program run with `flag` by Claude Code in a pane. Forward slashes, because Claude Code runs the command through
/// Git Bash when it can, which eats backslashes; always in double quotes, so that a `'` (`C:/Users/O'Brien`) ends no
/// string. None for a path bash would still expand inside them (`$`, `` ` ``): an unusable hook would block every prompt.
fn command(exe: &Path, flag: &str) -> Option<String> {
    let exe = exe.to_string_lossy().replace('\\', "/");
    (!exe.contains(['$', '`', '"'])).then(|| format!("\"{exe}\" {flag}"))
}

/// The same command for Claude Code of a WSL distribution, which a Linux shell runs through interop at the path `wslpath`
/// gives (it knows where the distribution mounts the drives).
fn wsl_command(exe: &Path, flag: &str) -> String {
    let exe = exe.to_string_lossy().replace('\'', r"'\''");
    format!("\"$(wslpath -u '{exe}')\" {flag}")
}

/// The settings Claude Code takes with `--settings` in a pane: this program as the hooks that note the session, and as
/// the status line unless the user has one of their own. `wsl` for the twin the `claude` function of the bash integration
/// picks, beside the other, inside WSL.
fn settings_json(exe: &Path, wsl: bool, status_line: bool) -> Option<String> {
    let command = |flag: &str| if wsl { Some(wsl_command(exe, flag)) } else { command(exe, flag) };
    let hook = serde_json::json!([{ "hooks": [{ "type": "command", "command": command(HOOK_FLAG)? }] }]);
    let hooks: serde_json::Map<String, serde_json::Value> = HOOK_EVENTS.iter().map(|event| ((*event).to_owned(), hook.clone())).collect();
    let mut settings = serde_json::json!({ "hooks": hooks });
    if status_line {
        settings["statusLine"] = serde_json::json!({ "type": "command", "command": command(FLAG)? });
    }
    Some(settings.to_string())
}

/// Writes `text` to `file` unless it holds it already: whole, as every window that starts writes it too.
pub fn write_if_changed(file: &Path, text: &str) -> Option<()> {
    (std::fs::read_to_string(file).ok().as_deref() == Some(text) || write_whole(file, text)).then_some(())
}

/// Writes the files `NEKOMINAL_CLAUDE_SETTINGS` names, next to the files of the panes and rewritten when this program
/// moved (another build, an update): `claude-settings.json` (status line and hooks) and `claude-hooks.json` (hooks only,
/// for a status line of the user's own, which the shell integration also picks for a project that sets one), each with a
/// WSL twin `-wsl.json`. Returns the one for a pane.
pub fn claude_settings_file(own_status_line: bool) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Nekominal");
    let exe = std::env::current_exe().ok()?;
    for (name, status_line) in [("claude-settings", true), ("claude-hooks", false)] {
        write_if_changed(&dir.join(format!("{name}-wsl.json")), &settings_json(&exe, true, status_line)?)?;
        write_if_changed(&dir.join(format!("{name}.json")), &settings_json(&exe, false, status_line)?)?;
    }
    Some(dir.join(if own_status_line { "claude-hooks.json" } else { "claude-settings.json" }))
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
    fn the_files_of_panes_unwritten_for_a_week_go_and_nothing_else_does() {
        let dir = std::env::temp_dir().join(format!("nekominal-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["123-1.json", "123-1.claude", "123-1.json.456.part", "notes.txt"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let later = SystemTime::now() + STALE_AFTER + std::time::Duration::from_secs(60);
        clean_stale_in(&dir, SystemTime::now());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4, "nothing is a week old yet");
        clean_stale_in(&dir, later);
        let left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["notes.txt"], "only the files of panes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pane_name_is_only_ever_one_this_program_hands_out() {
        assert!(new_session().bytes().all(|b| b.is_ascii_digit() || b == b'-'));
        assert_ne!(new_session(), new_session());
        // A pane handed over with a name of this process's id: the next names come after it, and other names change nothing.
        let taken = format!("{}-{}", std::process::id(), 1_000_000);
        reserve_session(&taken);
        reserve_session("1-99999999");
        reserve_session("not a name");
        let next: u64 = new_session().rsplit('-').next().unwrap().parse().unwrap();
        assert!(next > 1_000_000, "after the name kept, not over it: {next}");
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

    fn settings(exe: &str, wsl: bool, status_line: bool) -> serde_json::Value {
        serde_json::from_str(&settings_json(Path::new(exe), wsl, status_line).unwrap()).unwrap()
    }

    #[test]
    fn the_settings_of_a_pane_run_this_program_with_forward_slashes() {
        let plain = settings(r"C:\n\nekominal.exe", false, true);
        assert_eq!(plain["statusLine"]["command"], "\"C:/n/nekominal.exe\" --statusline");
        let quote = settings(r"C:\Users\O'Brien\scoop\apps\nekominal\current\Nekominal.exe", false, true);
        assert_eq!(quote["hooks"]["Stop"][0]["hooks"][0]["command"], "\"C:/Users/O'Brien/scoop/apps/nekominal/current/Nekominal.exe\" --claude-hook", "a ' in double quotes ends nothing");
        assert_eq!(settings_json(Path::new(r"C:\Users\$x\n.exe"), false, true), None, "a path bash would expand gets no hooks at all");
        assert_eq!(plain["statusLine"]["type"], "command");
        let spaced = settings(r"C:\Program Files\N\n.exe", false, true);
        assert_eq!(spaced["statusLine"]["command"], "\"C:/Program Files/N/n.exe\" --statusline");
        for event in HOOK_EVENTS {
            assert_eq!(spaced["hooks"][event][0]["hooks"][0]["command"], "\"C:/Program Files/N/n.exe\" --claude-hook", "{event}");
            assert_eq!(spaced["hooks"][event][0]["hooks"][0]["type"], "command");
        }
    }

    #[test]
    fn the_settings_of_a_wsl_pane_run_this_program_at_the_path_wslpath_gives() {
        let spaced = settings(r"C:\Program Files\N\n.exe", true, true);
        assert_eq!(spaced["statusLine"]["command"], r#""$(wslpath -u 'C:\Program Files\N\n.exe')" --statusline"#);
        assert_eq!(spaced["hooks"]["Stop"][0]["hooks"][0]["command"], r#""$(wslpath -u 'C:\Program Files\N\n.exe')" --claude-hook"#);
        let quoted = settings(r"C:\Bob's\n.exe", true, true);
        assert_eq!(quoted["statusLine"]["command"], r#""$(wslpath -u 'C:\Bob'\''s\n.exe')" --statusline"#, "a quote in the path ends no string");
    }

    #[test]
    fn a_status_line_of_the_user_s_own_still_gets_the_hooks_but_never_ours() {
        for wsl in [false, true] {
            let hooks = settings(r"C:\n\nekominal.exe", wsl, false);
            assert!(hooks.get("statusLine").is_none(), "--settings would win over the user's status line");
            assert_eq!(hooks["hooks"].as_object().unwrap().len(), HOOK_EVENTS.len());
            assert!(hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str().unwrap().ends_with(" --claude-hook"));
        }
    }

    #[test]
    fn the_newer_source_gives_the_context_and_the_last_status_line_of_the_session_its_cost() {
        use std::time::Duration;
        let (old, new) = (SystemTime::UNIX_EPOCH + Duration::from_secs(100), SystemTime::UNIX_EPOCH + Duration::from_secs(200));
        let line = Usage { model: Some("Opus 5.5".into()), context: Some(10.0), context_window: Some(1_000_000), cost: Some(1.25), duration_ms: Some(60_000), lines_added: Some(3), ..Default::default() };
        let turn = Turn { model: "claude-opus-5-5".into(), input: 1, cache_creation: 9_999, cache_read: 290_000, output: 40 };

        // The status line ran long ago; the transcript has a later answer: its context, the status line's cost.
        let both = combine(Some((line.clone(), Some("s1".into()), Some(old))), Some("s1".into()), Some((turn.clone(), Some(new))));
        assert_eq!((both.context, both.cache_read_tokens, both.context_window), (Some(30.0), Some(290_000), Some(1_000_000)));
        assert_eq!((both.cost, both.duration_ms, both.lines_added), (Some(1.25), Some(60_000), Some(3)), "kept from the last run");

        // The status line is the newer: it stands as it is.
        let fresh = combine(Some((line.clone(), Some("s1".into()), Some(new))), Some("s1".into()), Some((turn.clone(), Some(old))));
        assert_eq!((fresh.context, fresh.cache_read_tokens), (Some(10.0), None));

        // A status line of an earlier session in the pane is left out: no cost of another session.
        let other = combine(Some((line, Some("s0".into()), Some(old))), Some("s1".into()), Some((turn, Some(new))));
        assert_eq!((other.cost, other.context_window, other.context), (None, Some(1_000_000), Some(30.0)), "the window remembered for the model, not the old line's");
    }

    #[test]
    fn a_hook_notes_the_remote_control_session_while_it_is_connected_and_only_a_well_formed_one() {
        let input = serde_json::json!({"session_id":"s1","transcript_path":"C:\\t\\s1.jsonl"});
        let on: serde_json::Value = serde_json::from_str(&hook_record(&input, None, Some("session_01U8zozZirBz3yrQQbgyNVfM")).unwrap()).unwrap();
        assert_eq!(on["remote"], "session_01U8zozZirBz3yrQQbgyNVfM");
        assert_eq!(remote_link("session_01U8zozZirBz3yrQQbgyNVfM"), "https://claude.ai/code/session_01U8zozZirBz3yrQQbgyNVfM");
        let off: serde_json::Value = serde_json::from_str(&hook_record(&input, Some(&on.to_string()), None).unwrap()).unwrap();
        assert!(off["remote"].is_null(), "a hook without the variable says the connection is gone");
        for bad in ["session_", "sess_01", "session_01/../evil", "session_01?x=1", "https://claude.ai/code/session_01"] {
            assert_eq!(remote_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_hook_notes_the_transcript_and_keeps_a_model_said_before_in_the_same_session() {
        let start: serde_json::Value = serde_json::from_str(
            r#"{"session_id":"s1","transcript_path":"C:\\Users\\me\\.claude\\projects\\p\\s1.jsonl","cwd":"C:\\work","hook_event_name":"SessionStart","source":"startup","model":"claude-opus-5-5"}"#,
        ).unwrap();
        let first = hook_record(&start, None, None).unwrap();
        let noted: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(noted["transcript_path"], "C:\\Users\\me\\.claude\\projects\\p\\s1.jsonl");
        assert_eq!((noted["session_id"].as_str(), noted["cwd"].as_str(), noted["model"].as_str()), (Some("s1"), Some("C:\\work"), Some("claude-opus-5-5")));
        let tool: serde_json::Value = serde_json::from_str(r#"{"session_id":"s1","transcript_path":"C:\\t\\s1.jsonl","cwd":"C:\\work","hook_event_name":"Stop"}"#).unwrap();
        let later: serde_json::Value = serde_json::from_str(&hook_record(&tool, Some(&first), None).unwrap()).unwrap();
        assert_eq!(later["model"], "claude-opus-5-5", "the model of the session's start stays");
        let cleared: serde_json::Value = serde_json::from_str(r#"{"session_id":"s2","transcript_path":"C:\\t\\s2.jsonl","hook_event_name":"SessionStart","model":{"id":"claude-haiku-4-5","display_name":"Haiku 4.5"}}"#).unwrap();
        let cleared: serde_json::Value = serde_json::from_str(&hook_record(&cleared, Some(&first), None).unwrap()).unwrap();
        assert_eq!((cleared["transcript_path"].as_str(), cleared["model"].as_str()), (Some("C:\\t\\s2.jsonl"), Some("Haiku 4.5")));
        let other: serde_json::Value = serde_json::from_str(r#"{"session_id":"s3","transcript_path":"C:\\t\\s3.jsonl"}"#).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&hook_record(&other, Some(&first), None).unwrap()).unwrap()["model"].is_null(), "another session's model is not kept");
        assert_eq!(hook_record(&serde_json::json!({"session_id":"s1"}), None, None), None, "nothing to note without a transcript");
    }

    #[test]
    fn the_last_answer_of_the_main_conversation_is_read_from_the_end_of_the_transcript() {
        let tail = [
            r#"ns":5}},"type":"assistant","uuid":"cut-in-half"}"#,
            r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5-5","usage":{"input_tokens":3,"cache_creation_input_tokens":1200,"cache_read_input_tokens":40000,"output_tokens":512}}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-4-5","usage":{"input_tokens":9000,"output_tokens":10}}}"#,
            r#"{"type":"assistant","isSidechain":false,"message":{"model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"API Error"}]}}"#,
            r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5-5","content":[]}}"#,
            r#"{"type":"system","subtype":"turn_duration"}"#,
            "",
        ]
        .join("\n");
        let turn = last_turn(&tail, true).unwrap();
        assert_eq!(turn, Turn { model: "claude-opus-5-5".into(), input: 3, cache_creation: 1200, cache_read: 40000, output: 512 });
        assert_eq!(turn.context(), 41_203);
        assert_eq!(last_turn(r#"{"type":"assistant","message":{"model":"m","usage":{"input_tokens":7}}}"#, true), None, "a cut first line is never taken");
        assert_eq!(last_turn("", false), None);
        let mut usage = Usage { model: Some("Opus 5.5".into()), ..Usage::default() };
        fill_from_turn(&mut usage, &turn, 200_000);
        assert_eq!(usage.model.as_deref(), Some("Opus 5.5"), "the status line's name stays");
        assert!((usage.context.unwrap() - 20.6015).abs() < 1e-3);
        assert_eq!((usage.input_tokens, usage.cache_creation_tokens, usage.cache_read_tokens, usage.output_tokens), (Some(3), Some(1200), Some(40000), Some(512)));
        let mut usage = Usage::default();
        fill_from_turn(&mut usage, &turn, 200_000);
        assert_eq!(usage.model.as_deref(), Some("Opus 5.5"), "the name from the id");
    }

    #[test]
    fn the_end_of_a_long_transcript_is_read_from_its_last_quarter_megabyte() {
        let file = std::env::temp_dir().join(format!("nekominal-transcript-{}.jsonl", new_session()));
        let answer = r#"{"type":"assistant","message":{"model":"claude-opus-5-5","usage":{"input_tokens":10,"cache_read_input_tokens":90}}}"#;
        let filler = format!("{{\"type\":\"user\",\"pad\":\"{}\"}}\n", "x".repeat(1000));
        std::fs::write(&file, filler.repeat(400) + answer + "\n").unwrap();
        let (tail, cut) = read_tail(&file).unwrap();
        assert!(cut && tail.len() as u64 == TRANSCRIPT_TAIL);
        assert_eq!(last_turn(&tail, cut).map(|t| t.context()), Some(100));
        std::fs::write(&file, answer).unwrap();
        assert_eq!(read_tail(&file).map(|(_, cut)| cut), Some(false), "a short file is read whole");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn the_window_is_the_one_the_status_line_gave_else_200k_or_1m_past_that() {
        assert_eq!(window_size(Some(1_000_000), 50_000), 1_000_000);
        assert_eq!(window_size(None, 50_000), 200_000);
        assert_eq!(window_size(None, 200_000), 200_000);
        assert_eq!(window_size(None, 250_000), 1_000_000);
        assert_eq!(window_size(Some(200_000), 250_000), 1_000_000, "a window smaller than what is in use was another one's");
    }

    #[test]
    fn a_model_goes_by_its_family_and_version() {
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(model_name("claude-3-5-sonnet-20241022"), "Sonnet 3.5");
        assert_eq!(model_name("claude-fable"), "Fable");
        assert_eq!(model_name("gpt-6-luna"), "gpt-6-luna", "another id stays as it is");
        assert_eq!(model_name("claude-"), "claude-");
    }

    #[test]
    fn the_status_line_json_gives_the_tokens_lines_and_window() {
        let json = r#"{"context_window":{"context_window_size":1000000,"current_usage":{"input_tokens":5,"cache_creation_input_tokens":300,"cache_read_input_tokens":20000,"output_tokens":42},
            "total_input_tokens":123456,"total_output_tokens":7890,"used_percentage":2.03},"cost":{"total_api_duration_ms":4500,"total_lines_added":12,"total_lines_removed":3}}"#;
        let usage = parse_usage(json).unwrap();
        assert_eq!((usage.input_tokens, usage.cache_creation_tokens, usage.cache_read_tokens, usage.output_tokens), (Some(5), Some(300), Some(20000), Some(42)));
        assert_eq!((usage.context_window, usage.total_input_tokens, usage.total_output_tokens), (Some(1_000_000), Some(123_456), Some(7890)));
        assert_eq!((usage.lines_added, usage.lines_removed, usage.api_duration_ms), (Some(12), Some(3), Some(4500)));
        let early = parse_usage(r#"{"context_window":{"context_window_size":1000000,"current_usage":null,"used_percentage":null}}"#).unwrap();
        assert_eq!((early.context, early.input_tokens, early.context_window), (None, None, Some(1_000_000)), "null before the first answer");
    }

    #[test]
    fn the_branch_is_read_from_the_head_of_the_repository_above_the_folder() {
        let root = std::env::temp_dir().join(format!("nekominal-branch-{}", new_session()));
        let deep = root.join("src").join("inner");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/feature/bar\n").unwrap();
        assert_eq!(git_branch(&deep).map(|found| found.branch).as_deref(), Some("feature/bar"));
        assert_eq!(git_branch(&deep).map(|found| (found.root, found.git_file)), Some((root.clone(), None)), "the folder that holds the .git, found on the way");
        std::fs::write(root.join(".git").join("HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(git_branch(&root).map(|found| found.branch).as_deref(), Some("0123456"));
        // A worktree: `.git` names the folder of the repository that holds its HEAD.
        let tree = root.join("tree");
        std::fs::create_dir_all(root.join("wt")).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join(".git"), "gitdir: ../wt\n").unwrap();
        std::fs::write(root.join("wt").join("HEAD"), "ref: refs/heads/side\n").unwrap();
        assert_eq!(git_branch(&tree).map(|found| found.branch).as_deref(), Some("side"));
        assert_eq!(git_branch(&tree).and_then(|found| found.git_file).as_deref(), Some("gitdir: ../wt\n"), "the text of the .git file of a worktree, for the badges");
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
