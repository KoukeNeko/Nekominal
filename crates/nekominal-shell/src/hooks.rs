//! The other coding agents and the state of their pane: what Codex CLI, Gemini CLI and Aider say about their turns.
//!
//! Claude Code reports through its status line (`agent.rs`). The others have hooks of their own, and each is handed this
//! program with `--agent-hook <agent>`, which writes the state to a file named by the pane (`NEKOMINAL_SESSION`) that the
//! window reads once a second and keeps as the agent record of the pane (OSC 7501), so the tab, the bar and the agents
//! panel show it as they show any agent's. Their users' own configuration is never written: the hook reaches each one for
//! one run, through the PowerShell functions of the integration, and not at all where the user set that hook already.
//!
//! - Codex CLI: `notify`, a program run with a JSON argument when a turn ends (`{"type":"agent-turn-complete", "cwd",
//!   "thread-id", "turn-id", "input-messages", "last-assistant-message"}`), given with `-c notify=[...]`. Done only:
//!   nothing says a turn started or that an approval is asked (its lifecycle hooks do, but they run only once the user
//!   trusts them in `/hooks`). No model, tokens or cost.
//! - Gemini CLI: `hooks` of its settings, a shell command with JSON on stdin (`session_id`, `cwd`, `hook_event_name`,
//!   `timestamp`): `BeforeAgent` (working), `AfterAgent` (done), `Notification` of type `ToolPermission` (waiting). Given
//!   through `GEMINI_CLI_SYSTEM_DEFAULTS_PATH`, the lowest layer of its settings, whose hooks it adds to the user's. No
//!   usage in these events.
//! - Aider: `--notifications-command`, run through the shell with nothing handed over when it waits for the user, after
//!   a reply or at a yes/no question alike: done. Its tokens and cost are only printed in the terminal.

use std::path::Path;

/// The argument that makes this program the hook of an agent: `--agent-hook <agent> [payload]`.
const FLAG: &str = "--agent-hook";
/// The extension of the file of a pane that holds what its agent's hook last said.
pub const EXTENSION: &str = "hook";

/// The agents known by the name of their program, with the name they are shown by.
const AGENTS: &[(&str, &str)] =
    &[("claude", "Claude Code"), ("codex", "Codex"), ("gemini", "Gemini CLI"), ("aider", "Aider"), ("opencode", "OpenCode")];

/// The agent a program is, by its file name (`codex-x86_64-pc-windows-msvc.exe` is Codex): its id.
pub fn agent_of_program(file: &str) -> Option<&'static str> {
    let file = file.to_ascii_lowercase();
    AGENTS.iter().map(|(id, _)| *id).find(|id| file.starts_with(id))
}

/// The name an agent is shown by, from its id or the app of its OSC 7501 reports; an app nobody knows as itself.
pub fn agent_name(app: &str) -> String {
    AGENTS.iter().find(|(id, _)| app.eq_ignore_ascii_case(id)).map_or_else(|| app.to_owned(), |(_, name)| (*name).to_owned())
}

/// The OSC 7501 report a hook of `agent` stands for, from what the hook was handed (Codex: JSON as its last argument;
/// Gemini: JSON on stdin; Aider: nothing). None for an event that says nothing about the state.
pub fn report_of(agent: &str, payload: &str) -> Option<String> {
    let value: Option<serde_json::Value> = serde_json::from_str(payload.trim_start_matches('\u{feff}')).ok();
    let field = |key: &str| value.as_ref().and_then(|v| v.get(key)).and_then(serde_json::Value::as_str);
    let state = match agent {
        // Later versions may hand over other types: only the end of a turn means done.
        "codex" => (field("type")? == "agent-turn-complete").then_some("done")?,
        "gemini" => match field("hook_event_name")? {
            "BeforeAgent" => "working",
            "AfterAgent" => "done",
            "Notification" if field("notification_type") == Some("ToolPermission") => "blocked:kind=permission",
            _ => return None,
        },
        "aider" => "done",
        _ => return None,
    };
    Some(format!("state={state}:id={agent}:app={agent}"))
}

/// Run as the hook of an agent when the program was started with `--agent-hook`: writes what it stands for to the file of
/// the pane and prints nothing (Gemini CLI reads stdout as the hook's answer, and nothing is no objection). Returns
/// whether the program was started for that.
pub fn hook_main() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some(FLAG) {
        return false;
    }
    let agent = args.get(1).map_or("", String::as_str);
    let payload = if agent == "gemini" {
        // One JSON value, read up to its end, as for the status line.
        let mut values = serde_json::Deserializer::from_reader(std::io::stdin().lock()).into_iter::<serde_json::Value>();
        values.next().and_then(Result::ok).map(|v| v.to_string()).unwrap_or_default()
    } else {
        args.get(2).cloned().unwrap_or_default()
    };
    if let Some(report) = report_of(agent, &payload)
        && let Some(file) = std::env::var(crate::agent::SESSION_VAR).ok().and_then(|s| crate::agent::session_file(&s, EXTENSION))
    {
        crate::agent::write_whole(&file, &report);
    }
    true
}

/// Reads again what the hook of a pane's agent said, when it said something new since `stamp`.
pub fn poll(session: &str, stamp: &mut Option<std::time::SystemTime>) -> Option<nekominal_term::agent_status::Report> {
    let file = crate::agent::session_file(session, EXTENSION)?;
    let now = std::fs::metadata(&file).ok().and_then(|m| m.modified().ok());
    if now == *stamp {
        return None;
    }
    *stamp = now;
    nekominal_term::agent_status::parse_report(std::fs::read_to_string(file).ok()?.trim())
}

/// The variables a pane gets for the PowerShell functions that hand the hooks over, with the settings of Gemini CLI
/// written next to the files of the panes. Empty when this program cannot say where it is.
pub fn environment() -> Vec<(String, String)> {
    let Ok(exe) = std::env::current_exe() else { return Vec::new() };
    let mut env = vec![("NEKOMINAL_AIDER_NOTIFY".to_owned(), aider_command(&exe))];
    if let Some(notify) = codex_notify(&exe) {
        env.push(("NEKOMINAL_CODEX_NOTIFY".to_owned(), notify));
    }
    if let Some(file) = std::env::var_os("LOCALAPPDATA").map(|dir| Path::new(&dir).join("Nekominal").join("gemini-defaults.json")) {
        let text = gemini_settings(&exe);
        if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
            crate::agent::write_whole(&file, &text);
        }
        env.push(("NEKOMINAL_GEMINI_DEFAULTS".to_owned(), file.to_string_lossy().into_owned()));
    }
    env
}

/// The `-c` override that makes this program Codex's `notify`: TOML literal strings, which take a path as it is. None for
/// a path with a `'`, which a literal string cannot hold.
fn codex_notify(exe: &Path) -> Option<String> {
    let exe = exe.to_string_lossy();
    (!exe.contains('\'')).then(|| format!("notify=['{exe}','{FLAG}','codex']"))
}

/// Aider's `--notifications-command`, which it runs through `cmd.exe`: quoted for a path with spaces.
fn aider_command(exe: &Path) -> String {
    let exe = exe.to_string_lossy();
    if exe.contains(' ') { format!("\"{exe}\" {FLAG} aider") } else { format!("{exe} {FLAG} aider") }
}

/// The settings Gemini CLI takes as its system defaults in a pane: this program as the hook of the events that tell its
/// state. Gemini runs a hook through PowerShell on Windows, where a quoted path needs the call operator.
fn gemini_settings(exe: &Path) -> String {
    let exe = exe.to_string_lossy();
    let command = if exe.contains([' ', '\'']) { format!("& '{}' {FLAG} gemini", exe.replace('\'', "''")) } else { format!("{exe} {FLAG} gemini") };
    let hook = serde_json::json!([{ "hooks": [{ "name": "nekominal", "type": "command", "command": command }] }]);
    serde_json::json!({ "hooks": { "BeforeAgent": hook, "AfterAgent": hook, "Notification": hook } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekominal_term::agent_status::{AgentState, BlockedKind, Report, parse_report};

    fn state(agent: &str, payload: &str) -> Option<(AgentState, Option<BlockedKind>, Option<String>)> {
        match parse_report(&report_of(agent, payload)?)? {
            Report::Set(record) => Some((record.state, record.kind, record.app)),
            _ => None,
        }
    }

    #[test]
    fn the_end_of_a_codex_turn_is_done_and_nothing_else_it_hands_over_is() {
        let turn = r#"{"type":"agent-turn-complete","thread-id":"b5f6","turn-id":"12345","cwd":"C:\\work","client":"codex-tui",
            "input-messages":["fix it"],"last-assistant-message":"Done: fixed"}"#;
        assert_eq!(state("codex", turn), Some((AgentState::Done, None, Some("codex".into()))));
        assert_eq!(state("codex", r#"{"type":"approval-requested"}"#), None, "a type of a later version is not a turn's end");
        assert_eq!(state("codex", ""), None);
        assert_eq!(state("codex", "not json"), None);
    }

    #[test]
    fn gemini_events_say_working_done_and_waiting_for_a_permission() {
        let event = |name: &str, extra: &str| format!(r#"{{"session_id":"s","cwd":"C:\\w","hook_event_name":"{name}","timestamp":"t"{extra}}}"#);
        assert_eq!(state("gemini", &event("BeforeAgent", r#","prompt":"hi""#)).map(|s| s.0), Some(AgentState::Working));
        assert_eq!(state("gemini", &event("AfterAgent", r#","prompt_response":"ok""#)).map(|s| s.0), Some(AgentState::Done));
        let asked = event("Notification", r#","notification_type":"ToolPermission","message":"run ls?""#);
        assert_eq!(state("gemini", &asked), Some((AgentState::Blocked, Some(BlockedKind::Permission), Some("gemini".into()))));
        assert_eq!(state("gemini", &event("Notification", r#","notification_type":"Other""#)), None);
        assert_eq!(state("gemini", &event("SessionEnd", "")), None);
        assert_eq!(state("gemini", "\u{feff}{\"hook_event_name\":\"AfterAgent\"}").map(|s| s.0), Some(AgentState::Done));
    }

    #[test]
    fn aider_hands_over_nothing_and_waiting_is_done_while_an_unknown_agent_says_nothing() {
        assert_eq!(state("aider", "").map(|s| s.0), Some(AgentState::Done));
        assert_eq!(report_of("other", r#"{"type":"agent-turn-complete"}"#), None);
        assert_eq!(report_of("", ""), None);
    }

    #[test]
    fn agents_are_known_by_the_name_of_their_program() {
        assert_eq!(agent_of_program("claude.exe"), Some("claude"));
        assert_eq!(agent_of_program("codex-x86_64-pc-windows-msvc.exe"), Some("codex"));
        assert_eq!(agent_of_program("Gemini.cmd"), Some("gemini"));
        assert_eq!(agent_of_program("aider.exe"), Some("aider"));
        assert_eq!(agent_of_program("node.exe"), None, "an agent run by node is known by its hook only");
        assert_eq!(agent_of_program("pwsh.exe"), None);
        assert_eq!(agent_name("codex"), "Codex");
        assert_eq!(agent_name("Claude"), "Claude Code");
        assert_eq!(agent_name("my-bot"), "my-bot", "an app of an OSC 7501 report nobody knows goes by its own name");
    }

    #[test]
    fn the_hooks_name_this_program_as_each_agent_runs_its_command() {
        let plain = Path::new(r"C:\n\nekominal.exe");
        let spaced = Path::new(r"C:\Program Files\N\n.exe");
        assert_eq!(codex_notify(plain).as_deref(), Some(r"notify=['C:\n\nekominal.exe','--agent-hook','codex']"));
        assert_eq!(codex_notify(Path::new(r"C:\O'Neil\n.exe")), None, "a TOML literal string cannot hold a quote");
        assert_eq!(aider_command(plain), r"C:\n\nekominal.exe --agent-hook aider");
        assert_eq!(aider_command(spaced), r#""C:\Program Files\N\n.exe" --agent-hook aider"#);
        let settings: serde_json::Value = serde_json::from_str(&gemini_settings(spaced)).unwrap();
        for event in ["BeforeAgent", "AfterAgent", "Notification"] {
            assert_eq!(settings["hooks"][event][0]["hooks"][0]["command"], r"& 'C:\Program Files\N\n.exe' --agent-hook gemini", "{event}");
            assert_eq!(settings["hooks"][event][0]["hooks"][0]["type"], "command");
        }
        let plain: serde_json::Value = serde_json::from_str(&gemini_settings(plain)).unwrap();
        assert_eq!(plain["hooks"]["AfterAgent"][0]["hooks"][0]["command"], r"C:\n\nekominal.exe --agent-hook gemini");
    }

    #[test]
    fn a_hook_file_is_read_again_only_when_it_changed() {
        let session = crate::agent::new_session();
        let Some(file) = crate::agent::session_file(&session, EXTENSION) else { return };
        let mut stamp = None;
        assert_eq!(poll(&session, &mut stamp), None, "no file, nothing to say");
        crate::agent::write_whole(&file, &report_of("codex", r#"{"type":"agent-turn-complete"}"#).unwrap());
        assert!(matches!(poll(&session, &mut stamp), Some(Report::Set(r)) if r.state == AgentState::Done));
        assert_eq!(poll(&session, &mut stamp), None, "the same file is not applied twice");
        crate::agent::forget(&session);
        assert!(!file.exists(), "the hook file goes with the pane's other file");
    }
}
