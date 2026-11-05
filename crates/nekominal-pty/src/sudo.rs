//! The command lines of `nekominal-sudo` (`crates/nekominal-shell/src/bin/nekominal-sudo.rs`): what the user typed, what is
//! made of it for the elevated copy, and what the copy reads back. Plain text in, text out, so that it is tested here.

use std::path::Path;

use crate::{netdrive, profiles};

pub const USAGE: &str = "usage: nekominal-sudo [-D folder] command [arguments]\nRuns the command as administrator in this terminal; Windows asks first.";

/// The first word of `line` with its quotes taken off, and what comes after it.
pub fn next_token(line: &str) -> Option<(String, &str)> {
    let line = line.trim_start();
    if line.is_empty() {
        return None;
    }
    let (mut word, mut quoted, mut end) = (String::new(), false, line.len());
    for (at, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                end = at;
                break;
            }
            c => word.push(c),
        }
    }
    Some((word, &line[end..]))
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub directory: Option<String>,
    /// The command as it was typed: what follows the options, quotes and all.
    pub command: String,
}

/// What `raw` (the command line of the program) asks for.
pub fn parse(raw: &str) -> Result<Request, String> {
    let (_, mut rest) = next_token(raw).ok_or(USAGE)?;
    let mut directory = None;
    while let Some((word, after)) = next_token(rest) {
        match word.as_str() {
            "-D" | "--chdir" => {
                let (folder, after) = next_token(after).ok_or("-D needs a folder")?;
                directory = Some(folder);
                rest = after;
            }
            "--" => {
                rest = after;
                break;
            }
            _ => break,
        }
    }
    let command = rest.trim().to_owned();
    if command.is_empty() { Err(USAGE.to_owned()) } else { Ok(Request { directory, command }) }
}

/// The command line that starts `command` as the shell would have found it: its program through the PATH of this terminal
/// (the elevated program has another), or through `cmd` when it is no program (`dir`) or a batch file.
pub fn command_line(command: &str) -> String {
    let Some((program, rest)) = next_token(command) else { return command.to_owned() };
    match profiles::resolve_program(&program) {
        Some(path) if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd")) => {
            format!("\"{}\"{rest}", path.display())
        }
        _ => format!("cmd.exe /d /s /c \"{command}\""),
    }
}

/// A folder as an argument in quotes: a mapped network drive as its share (the elevated copy has the drives of its own
/// session), and `\.` at the end so that a drive root does not escape the closing quote.
pub fn folder_argument(folder: &str) -> String {
    let folder = netdrive::unc_of(Path::new(folder)).map_or_else(|| folder.to_owned(), |share| share.display().to_string());
    format!("\"{}\\.\"", folder.trim_end_matches('\\').replace('"', ""))
}

/// The arguments of the elevated copy for the program that asked (`parent`): `--elevated <parent> <folder> -- <line>`.
pub fn helper_arguments(parent: u32, folder: Option<&str>, line: &str) -> String {
    format!("--elevated {parent} {} -- {line}", folder.map_or_else(|| "\"\"".to_owned(), folder_argument))
}

#[derive(Debug, PartialEq, Eq)]
pub struct HelperRequest {
    pub parent: u32,
    pub folder: Option<String>,
    pub line: String,
}

/// What the elevated copy was started with; none for a program that is not it.
pub fn helper_request(raw: &str) -> Option<HelperRequest> {
    let (_, rest) = next_token(raw)?;
    let (flag, rest) = next_token(rest)?;
    if flag != "--elevated" {
        return None;
    }
    let (parent, rest) = next_token(rest)?;
    let (folder, rest) = next_token(rest)?;
    let (dash, line) = next_token(rest)?;
    let parent = parent.parse().ok().filter(|_| dash == "--")?;
    Some(HelperRequest { parent, folder: Some(folder).filter(|folder| !folder.is_empty()), line: line.trim().to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_loses_its_quotes_and_the_rest_is_left_as_it_was_typed() {
        assert_eq!(next_token(r#"  "C:\Program Files\x.exe" a  "b c""#), Some((r"C:\Program Files\x.exe".to_owned(), r#" a  "b c""#)));
        assert_eq!(next_token("one"), Some(("one".to_owned(), "")));
        assert_eq!(next_token("   "), None);
    }

    #[test]
    fn the_command_is_what_follows_the_program_and_its_options() {
        let parsed = |raw| parse(raw).map(|r| (r.directory, r.command));
        assert_eq!(parsed(r#""C:\a b\nekominal-sudo.exe" whoami /groups"#), Ok((None, "whoami /groups".into())));
        assert_eq!(parsed(r#"sudo -D "C:\My Folder" dir /b"#), Ok((Some(r"C:\My Folder".into()), "dir /b".into())));
        assert_eq!(parsed("sudo -- -D is the command"), Ok((None, "-D is the command".into())), "after -- nothing is an option");
        assert_eq!(parsed(r#"sudo cmd /c "echo  two  spaces""#), Ok((None, r#"cmd /c "echo  two  spaces""#.into())), "quotes and spaces as typed");
        assert_eq!(parsed("sudo").unwrap_err(), USAGE);
        assert_eq!(parsed("sudo -D").unwrap_err(), "-D needs a folder");
        assert_eq!(parsed("sudo -D x").unwrap_err(), USAGE, "a folder and no command");
    }

    #[test]
    fn a_program_is_found_by_the_path_of_this_terminal_and_anything_else_goes_through_cmd() {
        let found = command_line("cmd /c echo hi");
        assert!(found.starts_with('"') && found.to_lowercase().ends_with(r#"system32\cmd.exe" /c echo hi"#), "{found}");
        // `ver` is of cmd and no program anywhere (`dir` is one in the usr\bin of Git, which a runner has on its PATH).
        assert_eq!(command_line("ver"), r#"cmd.exe /d /s /c "ver""#, "a command of cmd, not a program");
        assert_eq!(command_line("nekominal-no-such-program x"), r#"cmd.exe /d /s /c "nekominal-no-such-program x""#);
    }

    #[test]
    fn the_elevated_copy_reads_back_what_the_terminal_side_made() {
        let line = r#""C:\Windows\System32\whoami.exe" /groups"#;
        let raw = format!(r#""C:\x\nekominal-sudo.exe" {}"#, helper_arguments(4242, Some(r"C:\Program Files"), line));
        let request = helper_request(&raw).expect("the elevated copy");
        assert_eq!(request, HelperRequest { parent: 4242, folder: Some(r"C:\Program Files\.".into()), line: line.into() });
        let nowhere = helper_request(&format!("x {}", helper_arguments(7, None, "dir"))).unwrap();
        assert_eq!((nowhere.folder, nowhere.line.as_str()), (None, "dir"), "no folder, and the command line whole");
        assert!(helper_request(r"nekominal-sudo.exe whoami").is_none(), "a typed command is not the copy");
        assert!(helper_request(r"nekominal-sudo.exe --elevated x C:\ -- y").is_none(), "a parent that is no number");
        assert_eq!(folder_argument(r"C:\"), r#""C:\.""#, "a drive root keeps its closing quote");
    }
}
