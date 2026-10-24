//! Claude Code and the status bar: where its numbers come from.
//!
//! Claude Code runs a `statusLine` command whenever its session changes and hands it the session as JSON on stdin. Once the
//! user connects it on the Agent page, that command is this program with `--statusline`: it writes what it is handed to a
//! file named by the pane it runs in (`NEKOMINAL_SESSION`, set for every pane), and the window reads the file of the
//! focused pane once a second. Nothing leaves the machine. Connecting adds one member to Claude Code's settings file and
//! disconnecting takes it out again; the rest of the file is left as it was written, byte for byte.

use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

/// The variable that names the pane a program runs in, for the command Claude Code runs.
pub const SESSION_VAR: &str = "NEKOMINAL_SESSION";
/// The argument that makes this program the `statusLine` command instead of a window.
const FLAG: &str = "--statusline";
const MEMBER: &str = "statusLine";

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
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
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

/// The branch the repository that holds `dir` is on, or the start of the commit when no branch is checked out.
pub fn git_branch(dir: &Path) -> Option<String> {
    // A network share is never looked at: this runs every second on the thread of the window, and looking connects.
    let text = dir.to_string_lossy();
    if text.chars().take(2).all(|c| matches!(c, '\\' | '/')) {
        return None;
    }
    for folder in dir.ancestors() {
        let git = folder.join(".git");
        let head = if git.is_dir() {
            git.join("HEAD")
        } else if git.is_file() {
            // A worktree or a submodule: `.git` is a file that says where the repository is.
            let pointer = std::fs::read_to_string(&git).ok()?;
            let target = Path::new(pointer.strip_prefix("gitdir:")?.trim());
            folder.join(target).join("HEAD")
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

/// Whether Claude Code's status line goes to this program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Linked,
    Unlinked,
    /// Claude Code has a status line command of somebody else's, which connecting would replace: it is left alone.
    Foreign,
}

/// Claude Code as this machine has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Claude {
    pub exe: Option<PathBuf>,
    pub version: Option<String>,
    pub link: Link,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// Claude Code's settings of the user: `CLAUDE_CONFIG_DIR` or `~/.claude`.
fn settings_file() -> Option<PathBuf> {
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
    let text = settings_file().and_then(|file| std::fs::read_to_string(file).ok()).unwrap_or_default();
    Claude { exe: on_path.or(installed), version, link: link_of(&text) }
}

fn link_of(settings: &str) -> Link {
    let value: Option<serde_json::Value> = serde_json::from_str(settings).ok();
    match value.as_ref().and_then(|v| v.get(MEMBER)) {
        None | Some(serde_json::Value::Null) => Link::Unlinked,
        Some(line) if line.get("command").and_then(|c| c.as_str()).is_some_and(|c| c.contains(FLAG)) => Link::Linked,
        Some(_) => Link::Foreign,
    }
}

/// The command Claude Code runs: this program, with forward slashes (Git Bash, which Claude Code prefers on Windows, eats
/// backslashes) and in quotes for a path with spaces.
fn command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy().replace('\\', "/");
    Ok(if exe.contains(' ') { format!("\"{exe}\" {FLAG}") } else { format!("{exe} {FLAG}") })
}

/// Points Claude Code's status line at this program; refused when it has a command of somebody else's.
pub fn link() -> Result<(), String> {
    let file = settings_file().ok_or("no home folder")?;
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let line = serde_json::json!({ "type": "command", "command": command()? });
    match linked(&text, &line.to_string())? {
        Some(added) => write(&file, &added),
        None => Ok(()),
    }
}

/// Claude Code's settings `text` with the status line `line` in it; `None` when it is in already.
fn linked(text: &str, line: &str) -> Result<Option<String>, String> {
    // Only a file that is understood is edited: a comma too many, a byte order mark or a file being written would be
    // made worse by an edit made as text.
    if !text.trim().is_empty() && serde_json::from_str::<serde_json::Value>(text).is_err() {
        return Err("Claude Code's settings file is not JSON this program understands".to_owned());
    }
    match link_of(text) {
        Link::Linked => return Ok(None),
        Link::Foreign => return Err("another statusLine is set".to_owned()),
        Link::Unlinked => {}
    }
    // `"statusLine": null` counts as none: it goes before the new member, or the file would hold the key twice.
    let text = if member(text, MEMBER).is_some() { without_member(text, MEMBER).ok_or("the statusLine member could not be taken out")? } else { text.to_owned() };
    with_member(&text, MEMBER, line).map(Some).ok_or_else(|| "the settings file is not a JSON object".to_owned())
}

/// Takes this program's status line out of Claude Code's settings; a command of somebody else's stays.
pub fn unlink() -> Result<(), String> {
    let file = settings_file().ok_or("no home folder")?;
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    if link_of(&text) != Link::Linked {
        return Ok(());
    }
    let removed = without_member(&text, MEMBER).ok_or("the statusLine member was not found")?;
    write(&file, &removed)
}

fn write(file: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = file.with_extension("json.nekominal");
    std::fs::write(&part, text).map_err(|e| e.to_string())?;
    std::fs::rename(&part, file).map_err(|e| e.to_string())
}

// --- the members of a JSON object, edited as text ---------------------------------------------------------------

/// Where the string that starts at `at` (its opening quote) ends: the index of its closing quote.
fn string_end(b: &[u8], at: usize) -> Option<usize> {
    let mut i = at + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Where the value that starts at `at` ends (exclusive).
fn value_end(b: &[u8], at: usize) -> Option<usize> {
    match *b.get(at)? {
        b'"' => string_end(b, at).map(|end| end + 1),
        b'{' | b'[' => {
            let mut depth = 0;
            let mut i = at;
            while i < b.len() {
                match b[i] {
                    b'"' => i = string_end(b, i)?,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        _ => Some(at + b[at..].iter().position(|c| matches!(c, b',' | b'}' | b']') || c.is_ascii_whitespace()).unwrap_or(b.len() - at)),
    }
}

fn skip_space(b: &[u8], mut at: usize) -> usize {
    while at < b.len() && b[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// The member `key` of the outermost object of `text`: from the quote of its key to the end of its value.
fn member(text: &str, key: &str) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    let (mut depth, mut expect_key, mut i) = (0, false, 0);
    while i < b.len() {
        match b[i] {
            b'"' => {
                let end = string_end(b, i)?;
                if depth == 1 && expect_key && &text[i + 1..end] == key {
                    let colon = skip_space(b, end + 1);
                    (b.get(colon) == Some(&b':')).then_some(())?;
                    return Some((i, value_end(b, skip_space(b, colon + 1))?));
                }
                expect_key = false;
                i = end;
            }
            b'{' | b'[' => {
                depth += 1;
                expect_key = b[i] == b'{' && depth == 1;
            }
            b'}' | b']' => depth -= 1,
            b',' if depth == 1 => expect_key = true,
            _ => {}
        }
        i += 1;
    }
    None
}

/// `text` with `"key": value` added as the last member of its outermost object, indented by two spaces; `None` when it is
/// not an object. An empty text becomes an object of that member alone.
fn with_member(text: &str, key: &str, value: &str) -> Option<String> {
    if text.trim().is_empty() {
        return Some(format!("{{\n  \"{key}\": {value}\n}}\n"));
    }
    let close = text.rfind('}')?;
    text.trim_start().starts_with('{').then_some(())?;
    let before = text[..close].trim_end();
    let comma = if before.ends_with('{') { "" } else { "," };
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    Some(format!("{before}{comma}{nl}  \"{key}\": {value}{nl}{}", &text[close..]))
}

/// `text` without the member `key` of its outermost object and the comma that went with it.
fn without_member(text: &str, key: &str) -> Option<String> {
    let (start, end) = member(text, key)?;
    let b = text.as_bytes();
    let before = text[..start].trim_end();
    if before.ends_with(',') {
        // Not the first member: it goes with the comma before it.
        return Some(format!("{}{}", &text[..before.len() - 1], &text[end..]));
    }
    let after = skip_space(b, end);
    if b.get(after) == Some(&b',') {
        // The first member: it goes with the comma after it, up to the next member.
        return Some(format!("{}{}", &text[..start], &text[skip_space(b, after + 1)..]));
    }
    // The only member: the object is left empty.
    Some(format!("{before}{}", &text[after..]))
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
    fn the_link_is_read_from_the_member_of_claude_code_s_settings() {
        assert_eq!(link_of(""), Link::Unlinked);
        assert_eq!(link_of(r#"{"theme":"dark"}"#), Link::Unlinked);
        assert_eq!(link_of(r#"{"statusLine":{"type":"command","command":"C:/n/nekominal.exe --statusline"}}"#), Link::Linked);
        assert_eq!(link_of(r#"{"statusLine":{"type":"command","command":"~/.claude/line.sh"}}"#), Link::Foreign);
    }

    #[test]
    fn connecting_adds_one_member_and_disconnecting_leaves_the_file_as_it_was() {
        let line = r#"{"type":"command","command":"C:/n/n.exe --statusline"}"#;
        for original in [
            "{\n  \"theme\": \"dark\",\n  \"env\": { \"A\": \"{,}\" }\n}\n",
            "{\r\n  \"theme\": \"dark\"\r\n}\r\n",
            "{\n  \"nested\": { \"statusLine\": 1 }\n}",
        ] {
            let added = with_member(original, MEMBER, line).unwrap();
            assert_eq!(link_of(&added), Link::Linked, "{added}");
            let value: serde_json::Value = serde_json::from_str(&added).unwrap();
            assert_eq!(value["theme"].as_str().or(Some("dark")), Some("dark"));
            assert_eq!(without_member(&added, MEMBER).unwrap().replace("\r\n", "\n").trim_end(), original.replace("\r\n", "\n").trim_end(), "{added}");
        }
        let alone = with_member("", MEMBER, line).unwrap();
        assert_eq!(link_of(&alone), Link::Linked);
        assert_eq!(with_member("{}", MEMBER, line).unwrap(), format!("{{\n  \"statusLine\": {line}\n}}"));
        assert_eq!(without_member(&with_member("{}", MEMBER, line).unwrap(), MEMBER).unwrap(), "{}");
        assert_eq!(with_member("[1]", MEMBER, line), None);
    }

    #[test]
    fn only_a_file_that_is_understood_is_connected_and_a_null_line_is_replaced() {
        let line = r#"{"type":"command","command":"C:/n/n.exe --statusline"}"#;
        let replaced = linked("{\n  \"statusLine\": null,\n  \"theme\": \"dark\"\n}", line).unwrap().unwrap();
        assert_eq!(replaced.matches("statusLine").count(), 1, "{replaced}");
        assert_eq!(link_of(&replaced), Link::Linked);
        assert_eq!(link_of(&without_member(&replaced, MEMBER).unwrap()), Link::Unlinked, "one disconnect takes it out");
        assert!(linked("{\"a\": 1,}", line).is_err(), "a comma too many is left alone");
        assert!(linked("\u{feff}{\"a\": 1}", line).is_err());
        assert!(linked(r#"{"statusLine":{"command":"~/line.sh"}}"#, line).is_err(), "somebody else's line stays");
        assert_eq!(linked(&replaced, line).unwrap(), None, "connected already");
        assert!(linked("", line).unwrap().is_some());
    }

    #[test]
    fn a_member_is_found_only_at_the_top_and_taken_out_with_its_comma() {
        let text = r#"{"statusLine": {"command": "a,b}"}, "theme": "x", "inner": {"statusLine": 2}}"#;
        assert_eq!(without_member(text, MEMBER).unwrap(), r#"{"theme": "x", "inner": {"statusLine": 2}}"#);
        let text = r#"{"theme": "x", "statusLine": {"command": "q\"}"}, "z": 1}"#;
        assert_eq!(without_member(text, MEMBER).unwrap(), r#"{"theme": "x", "z": 1}"#);
        assert_eq!(member(r#"{"a": "statusLine", "b": 1}"#, MEMBER), None, "a value of that name is no member");
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
        let _ = std::fs::remove_dir_all(&root);
    }
}
