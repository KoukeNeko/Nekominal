//! What is specific to Claude Code: where it lives, which slash commands never reach the model, and how to read
//! its screens.

use std::{path::PathBuf, time::Duration};

use crate::{
    probe::{EnterPolicy, Guard, Probe, ProbeError, Result, SpawnOptions},
    screen::Screen,
};

/// The CLI to probe: `NEKOMINAL_CLAUDE_EXE`, or the native install under the user's profile.
pub fn exe() -> PathBuf {
    if let Some(path) = std::env::var_os("NEKOMINAL_CLAUDE_EXE") {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    home.join(".local").join("bin").join("claude.exe")
}

/// An empty directory for the CLI's working directory: `NEKOMINAL_PROBE_CWD`, or `cwd` under the tap
/// directory (`target/agent-probe`). It is not the repository root, so no project settings, hooks or MCP
/// servers are loaded. The default sits inside the workspace the user already trusts: a folder the CLI does
/// not trust yet shows a trust dialog, and accepting that would write the user's CLI state.
pub fn throwaway_dir() -> PathBuf {
    let dir = std::env::var_os("NEKOMINAL_PROBE_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::probe::default_tap_dir().join("cwd"));
    // Failing here surfaces as a spawn error with the directory named.
    let _ = std::fs::create_dir_all(&dir);
    // A plain absolute path: no `..` components and no `\\?\` verbatim prefix, which the CLI would show.
    match dir.canonicalize() {
        Ok(full) => match full.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
            Some(plain) => PathBuf::from(plain),
            None => full,
        },
        Err(_) => dir,
    }
}

/// Slash commands the harness may submit. Each one is handled locally (a help screen or an exit) and none of
/// them sends a prompt to the model or writes a setting. Anything else is refused.
pub const LOCAL_SLASH_COMMANDS: &[&str] = &["/help", "/exit"];

pub fn policy() -> EnterPolicy {
    EnterPolicy { allowed_commands: LOCAL_SLASH_COMMANDS, read_input_box: input_box_text }
}

/// Options for probing Claude Code: 120x30, Nekominal's terminal environment, `DISABLE_AUTOUPDATER=1` (added
/// by the environment builder), an empty working directory and the no-model guard. `CLAUDE_CODE_SAFE_MODE=1`
/// starts the CLI without the user's hooks, plugins, MCP servers, custom commands, themes and keybindings:
/// none of them runs during a probe, and the screens do not depend on what is installed. Authentication,
/// the renderer and the terminal protocol are unaffected.
pub fn options(scenario: &str) -> SpawnOptions {
    let mut options =
        SpawnOptions::new(exe(), scenario).guard(Guard::NoModelCalls(policy())).env("CLAUDE_CODE_SAFE_MODE", "1");
    options.cwd = Some(throwaway_dir());
    options
}

/// [`options`] with the flicker-free alternate-screen renderer, switched on for this process only.
pub fn fullscreen_options(scenario: &str) -> SpawnOptions {
    options(scenario).env("CLAUDE_CODE_NO_FLICKER", "1")
}

/// Screens that mean the CLI is not at its main UI. The probe stops there: it never signs in, accepts a trust
/// dialog or goes through onboarding.
const NOT_THE_MAIN_UI: &[(&str, &str)] = &[
    ("Select login method", "login screen"),
    ("Sign in", "login screen"),
    ("Choose the text style", "onboarding"),
    ("Do you trust the files", "folder trust dialog"),
    ("Quick safety check", "folder trust dialog"),
];

/// What `screen` shows instead of the main UI, if it is one of the known screens that need the user.
pub fn blocking_screen(screen: &Screen) -> Option<&'static str> {
    NOT_THE_MAIN_UI.iter().find(|(text, _)| screen.contains(text)).map(|(_, what)| *what)
}

/// The version in the start banner (`Claude Code v2.1.300`).
pub fn banner_version(screen: &Screen) -> Option<String> {
    let pattern = regex::Regex::new(r"Claude Code v(\d+\.\d+\.\d+)").ok()?;
    screen.lines.iter().find_map(|line| pattern.captures(line).map(|caps| caps[1].to_owned()))
}

/// A started CLI at its main UI.
pub struct Started {
    pub probe: Probe,
    /// Time from the spawn until the input box was drawn.
    pub prompt_after: Duration,
    /// Time from the spawn until the output had been quiet for [`QUIET`] after that.
    pub stable_after: Duration,
}

const READY_TIMEOUT: Duration = Duration::from_secs(45);
/// Silence that counts as a settled screen.
pub const QUIET: Duration = Duration::from_millis(1500);

/// Spawns the CLI and waits for its input box and a settled screen. A login, onboarding or trust screen is
/// an error, not something to get past.
pub fn start(options: SpawnOptions) -> Result<Started> {
    let probe = Probe::spawn(options)?;
    let screen = probe.wait_until("the input box or a screen that needs the user", READY_TIMEOUT, |s| {
        input_box(s).is_some() || blocking_screen(s).is_some()
    })?;
    if let Some(what) = blocking_screen(&screen) {
        return Err(ProbeError::Other(format!(
            "the CLI shows a {what} instead of the main UI; the probe does not go past it"
        )));
    }
    let prompt_after = probe.elapsed();
    probe.wait_quiet(QUIET, READY_TIMEOUT)?;
    let stable_after = probe.elapsed();
    Ok(Started { probe, prompt_after, stable_after })
}

/// Types `command` (an empty input box is assumed) and waits until the box shows exactly that.
pub fn type_command(probe: &Probe, command: &str) -> Result<()> {
    probe.send_text(command)?;
    probe
        .wait_until(&format!("the input box to read {command:?}"), Duration::from_secs(5), |s| {
            input_box_text(s).as_deref() == Some(command)
        })
        .map(|_| ())
}

/// Types an allow-listed local command and presses Enter. The guard refuses anything else.
pub fn run_local_command(probe: &Probe, command: &str) -> Result<()> {
    type_command(probe, command)?;
    probe.send_enter().map(|_| ())
}

/// The prompt marker of the input box and of the selected row of a suggestion menu. The CLI follows it with a
/// no-break space in the box and with a plain space in menus.
const PROMPT_MARKER: char = '❯';

fn is_rule(line: &str) -> bool {
    line.starts_with('─')
}

/// The prompt line after the marker: spaces and no-break spaces dropped, trailing blanks dropped.
fn after_marker(line: &str) -> Option<&str> {
    line.strip_prefix(PROMPT_MARKER).map(|rest| rest.trim_start_matches([' ', '\u{a0}']).trim_end())
}

/// The input box on `screen`: the lines between the marker row (directly below a horizontal rule) and the next
/// rule. The box is where the CLI reads typed text, so this is what a submit would send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputBox {
    /// Row of the line that carries the marker.
    pub row: usize,
    /// Row of the rule that closes the box.
    pub end: usize,
    /// Lines of the box joined by newlines; empty while the placeholder hint is shown.
    pub text: String,
    /// The box is empty and shows the dim hint text.
    pub placeholder: bool,
}

pub fn input_box(screen: &Screen) -> Option<InputBox> {
    let row = (1..screen.rows)
        .rev()
        .find(|&row| is_rule(screen.line(row - 1)) && after_marker(screen.line(row)).is_some())?;
    let first = after_marker(screen.line(row))?;
    let dim = nekominal_term::alacritty_terminal::term::cell::Flags::DIM;
    // Column 2 holds the first text cell (marker and no-break space take 0 and 1).
    let placeholder = !first.is_empty() && screen.cols > 2 && screen.cell(row, 2).flags.contains(dim);
    // The closing rule must be on screen, otherwise lines of the box may be missing from what was read.
    let end = (row + 1..screen.rows).find(|&next| is_rule(screen.line(next)))?;
    let mut lines = vec![if placeholder { "" } else { first }];
    lines.extend((row + 1..end).map(|next| screen.line(next).trim_start_matches([' ', '\u{a0}']).trim_end()));
    Some(InputBox { row, end, text: lines.join("\n"), placeholder })
}

/// The word of the selected row of the suggestion menu (`/help` of `❯ /help  Show help`). The fullscreen
/// renderer draws the menu above the box, the classic one below it.
fn selected_suggestion(screen: &Screen, found: &InputBox) -> Option<String> {
    let outside = (0..found.row.saturating_sub(1)).chain(found.end + 1..screen.rows);
    outside.into_iter().find_map(|row| {
        let line = screen.line(row);
        let rest = line.trim_start().strip_prefix(PROMPT_MARKER)?;
        // The menu is indented; a marker at column 0 would be another prompt.
        (line.starts_with(' ') && rest.starts_with(' '))
            .then(|| rest.split_whitespace().next().unwrap_or("").to_owned())
    })
}

/// The text a submit would send, or `None` when it cannot be told safely: no input box, or an open suggestion
/// menu whose selected entry is not the command that was typed (Enter would run the entry).
pub fn input_box_text(screen: &Screen) -> Option<String> {
    let found = input_box(screen)?;
    if let Some(selected) = selected_suggestion(screen, &found) {
        let typed = found.text.split_whitespace().next().unwrap_or("");
        if selected != typed {
            return None;
        }
    }
    Some(found.text)
}

#[cfg(test)]
mod tests {
    use nekominal_term::alacritty_terminal::term::cell::Flags;

    use super::*;

    const RULE: &str = "────────────────────";

    fn screen(lines: &[&str]) -> Screen {
        Screen::from_lines(lines, 60)
    }

    #[test]
    fn the_box_text_follows_the_marker_and_its_no_break_space() {
        let s = screen(&["conversation", RULE, "❯\u{a0}/help", RULE, "  status"]);
        let found = input_box(&s).unwrap();
        assert_eq!((found.row, found.end, found.text.as_str(), found.placeholder), (2, 3, "/help", false));
        assert_eq!(input_box_text(&s).as_deref(), Some("/help"));
    }

    #[test]
    fn lines_of_a_multi_line_box_are_all_read_including_blank_ones() {
        let s = screen(&[RULE, "❯\u{a0}/help", "", "  and a prompt", RULE]);
        assert_eq!(input_box_text(&s).as_deref(), Some("/help\n\nand a prompt"));
        assert!(!policy().allows(&input_box_text(&s).unwrap()), "a command hiding a prompt is not a command");
    }

    #[test]
    fn a_box_without_its_closing_rule_cannot_be_read() {
        assert_eq!(input_box_text(&screen(&[RULE, "❯\u{a0}/help", "  tail"])), None);
        assert_eq!(input_box_text(&screen(&["❯\u{a0}/help", RULE])), None, "no rule above the marker row");
        assert_eq!(input_box_text(&screen(&["plain", "text"])), None);
    }

    #[test]
    fn dim_hint_text_is_a_placeholder_and_reads_as_empty() {
        let mut s = screen(&[RULE, "❯\u{a0}Try \"fix it\"", RULE]);
        s.cells[s.cols + 2].flags.insert(Flags::DIM);
        let found = input_box(&s).unwrap();
        assert!(found.placeholder);
        assert_eq!(found.text, "");
    }

    #[test]
    fn an_open_menu_must_select_the_typed_command() {
        let menu = ["  ❯ /help      Show help", "    /hello      Greet", RULE, "❯\u{a0}/help", RULE];
        assert_eq!(input_box_text(&screen(&menu)).as_deref(), Some("/help"));
        let other = ["  ❯ /hello      Greet", "    /help      Show help", RULE, "❯\u{a0}/help", RULE];
        assert_eq!(input_box_text(&screen(&other)), None, "Enter would run /hello");
        let message = ["❯ an earlier prompt in the conversation", RULE, "❯\u{a0}/help", RULE];
        assert_eq!(input_box_text(&screen(&message)).as_deref(), Some("/help"), "a prompt at column 0 is not a menu");
        // The classic renderer draws the menu below the box.
        let below = [RULE, "❯\u{a0}/help", RULE, "  ❯ /help      Show help", "    /hello      Greet"];
        assert_eq!(input_box_text(&screen(&below)).as_deref(), Some("/help"));
        let below_other = [RULE, "❯\u{a0}/help", RULE, "    /help      Show help", "  ❯ /hello      Greet"];
        assert_eq!(input_box_text(&screen(&below_other)), None, "Enter would run /hello");
    }

    #[test]
    fn login_trust_and_onboarding_screens_are_recognised_and_the_main_ui_is_not() {
        let trust = screen(&["Quick safety check: Is this a project you created or one you trust?", "❯ No, exit"]);
        assert_eq!(blocking_screen(&trust), Some("folder trust dialog"));
        assert_eq!(blocking_screen(&screen(&["Select login method:", "1. Claude account"])), Some("login screen"));
        assert_eq!(blocking_screen(&screen(&[" ▐▛███▛█   Claude Code v2.1.300", RULE, "❯\u{a0}", RULE])), None);
    }

    #[test]
    fn the_banner_version_is_read_from_the_start_screen() {
        assert_eq!(
            banner_version(&screen(&["", " ▐▛███▛█   Claude Code v2.1.300", "~\\cwd"])).as_deref(),
            Some("2.1.300")
        );
        assert_eq!(banner_version(&screen(&["no banner"])), None);
    }

    #[test]
    fn only_local_commands_are_allow_listed() {
        for command in LOCAL_SLASH_COMMANDS {
            assert!(command.starts_with('/'));
        }
        for refused in ["/config", "/theme", "/model", "/login", "/tui fullscreen", "/help me", "hello", ""] {
            assert!(!policy().allows(refused), "{refused:?}");
        }
        assert!(policy().allows("/help"));
        assert!(policy().allows(" /exit "));
    }

    #[test]
    fn options_use_the_guard_an_empty_directory_and_a_per_process_switch() {
        let options = fullscreen_options("unit");
        assert!(matches!(options.guard, Guard::NoModelCalls(_)));
        assert_eq!(options.env.get("CLAUDE_CODE_NO_FLICKER").map(String::as_str), Some("1"));
        let cwd = options.cwd.unwrap();
        assert!(
            cwd.is_dir() && !cwd.join("Cargo.toml").exists(),
            "{cwd:?} must be a directory that is not the workspace root"
        );
        assert!(!cwd.to_string_lossy().starts_with(r"\\?\") && !cwd.components().any(|c| c.as_os_str() == ".."));
    }
}
