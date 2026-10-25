//! What the command line asks the first window for, in the words of Windows Terminal's `wt`: a starting folder (`-d`), a
//! profile (`-p`, by name or id), a command line to run instead of the shell of the profile, and more tabs and panes
//! after `;` (`new-tab`/`nt`, `split-pane`/`sp` with `-H` below or `-V` beside). Pure: `main` gives the arguments.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub directory: Option<String>,
    pub profile: Option<String>,
    /// The program and its arguments, run instead of the profile's.
    pub command: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    NewTab(Target),
    /// A pane beside the focused one (`vertical`, `-V`) or below it (`-H`).
    Split { vertical: bool, target: Target },
}

static LAUNCH: std::sync::OnceLock<Vec<Action>> = std::sync::OnceLock::new();

/// Keeps what the command line of the process asked for, for the first window.
pub fn set_launch(actions: Vec<Action>) {
    let _ = LAUNCH.set(actions);
}

/// What the command line asked for; empty for a plain start.
pub fn launch() -> &'static [Action] {
    LAUNCH.get().map_or(&[], Vec::as_slice)
}

/// The actions of the command line, the first a tab; none when it asks for nothing (a plain start, which restores the
/// session or opens the default profile). An argument that is not understood is left for the command line it ends.
pub fn parse(args: &[String]) -> Vec<Action> {
    let mut actions = Vec::new();
    for (n, part) in args.split(|arg| arg == ";").enumerate() {
        let mut words = part.iter().map(String::as_str).peekable();
        let split = match words.peek().copied() {
            Some("new-tab" | "nt") => {
                words.next();
                None
            }
            Some("split-pane" | "sp") => {
                words.next();
                Some(true)
            }
            _ => None,
        };
        let mut target = Target::default();
        let mut vertical = split.unwrap_or(true);
        while let Some(word) = words.next() {
            match word {
                "-d" | "--startingDirectory" => target.directory = words.next().map(folder),
                "-p" | "--profile" => target.profile = words.next().map(str::to_owned),
                "-H" | "--horizontal" if split.is_some() => vertical = false,
                "-V" | "--vertical" if split.is_some() => vertical = true,
                "--" => {
                    target.command.extend(words.by_ref().map(str::to_owned));
                }
                other => {
                    target.command.push(other.to_owned());
                    target.command.extend(words.by_ref().map(str::to_owned));
                }
            }
        }
        match split {
            Some(_) if n > 0 => actions.push(Action::Split { vertical, target }),
            // No argument at all is a plain start; anything else, the first part and a part with no subcommand, a tab.
            _ if part.is_empty() => {}
            _ => actions.push(Action::NewTab(target)),
        }
    }
    actions
}

/// The folder `-d` names, as a path: `D:\x\.` (what Explorer's entry passes) is `D:\x`, and `C:"` is `C:\`, which is what
/// `"C:\"` becomes on a command line, its backslash escaping the quote (an entry written by an older version).
fn folder(raw: &str) -> String {
    let fixed = raw.strip_suffix('"').map_or_else(|| raw.to_owned(), |rest| format!("{rest}\\"));
    let path: std::path::PathBuf = std::path::Path::new(&fixed).components().collect();
    // A relative folder (`-d .`) is of the folder this was started in, which an elevated copy of it would not share.
    std::path::absolute(&path).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn a_plain_start_asks_for_nothing() {
        assert_eq!(parse(&[]), Vec::<Action>::new());
    }

    #[test]
    fn a_folder_a_profile_and_a_command_line_go_to_the_first_tab() {
        let target = |directory: Option<&str>, profile: Option<&str>, command: &[&str]| Target {
            directory: directory.map(str::to_owned),
            profile: profile.map(str::to_owned),
            command: command.iter().map(|s| s.to_string()).collect(),
        };
        assert_eq!(parse(&args(r"-d D:\src")), [Action::NewTab(target(Some(r"D:\src"), None, &[]))]);
        assert_eq!(parse(&args("-p pwsh -d C:\\")), [Action::NewTab(target(Some("C:\\"), Some("pwsh"), &[]))]);
        assert_eq!(parse(&args("cmd /k echo hi")), [Action::NewTab(target(None, None, &["cmd", "/k", "echo", "hi"]))]);
        assert_eq!(parse(&args("-d C:\\ -- ping -t localhost")), [Action::NewTab(target(Some("C:\\"), None, &["ping", "-t", "localhost"]))]);
    }

    #[test]
    fn the_folder_is_the_path_whatever_way_explorer_wrote_it() {
        for (raw, wanted) in [(r"D:\src\.", r"D:\src"), (r"C:\\.", r"C:\"), (r#"C:""#, r"C:\"), (r"D:\a b", r"D:\a b"), (r"C:\", r"C:\")] {
            assert_eq!(folder(raw), wanted, "{raw}");
        }
        let here = std::env::current_dir().unwrap();
        assert_eq!(folder("."), here.display().to_string(), "a relative folder is made whole where it was given");
    }

    #[test]
    fn more_tabs_and_panes_follow_after_semicolons() {
        let parsed = parse(&args(r"-d D:\a ; nt -p Ubuntu ; sp -H -d D:\b ; split-pane cmd"));
        assert_eq!(parsed.len(), 4);
        assert!(matches!(&parsed[1], Action::NewTab(t) if t.profile.as_deref() == Some("Ubuntu")));
        assert!(matches!(&parsed[2], Action::Split { vertical: false, target } if target.directory.as_deref() == Some(r"D:\b")));
        assert!(matches!(&parsed[3], Action::Split { vertical: true, target } if target.command == ["cmd"]));
        // A first part that only says "new-tab" still opens one.
        assert_eq!(parse(&args("nt")), [Action::NewTab(Target::default())]);
    }
}
