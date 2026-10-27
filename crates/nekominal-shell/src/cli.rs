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
    /// The tabs of the saved layout of this name (`--layout <name>`, Nekominal's own, not `wt`'s).
    Layout(String),
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
        let mut layout = None;
        while let Some(word) = words.next() {
            match word {
                "--layout" => layout = words.next().map(str::to_owned),
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
        let layout_only = layout.is_some() && target == Target::default();
        actions.extend(layout.map(Action::Layout));
        match split {
            _ if layout_only => {}
            Some(_) if n > 0 => actions.push(Action::Split { vertical, target }),
            // No argument at all is a plain start; anything else, the first part and a part with no subcommand, a tab.
            _ if part.is_empty() => {}
            _ => actions.push(Action::NewTab(target)),
        }
    }
    actions
}

/// What `nekominal cli <command> [options] [words]` asks the automation API (`api.rs`) for, and the pipe given with
/// `--pipe` (else the instance of the pane it runs in, or the newest). The pane is `--pane`, else `NEKOMINAL_PANE` (`pane`).
/// `send-text` sends its words joined by spaces, or `stdin` when it has none; `set-title` and `notify` take their words
/// as the title (and the body after the first word of `notify`); `subscribe` the names of its events. A program for
/// `new-tab` and `split` comes after `--`.
pub fn api_request(args: &[String], pane: Option<String>, stdin: impl FnOnce() -> String) -> Result<(Option<String>, serde_json::Value), String> {
    use serde_json::{Value, json};
    let mut words = args.iter().map(String::as_str);
    let (mut pipe, mut options, mut rest, mut command) = (None, serde_json::Map::new(), Vec::new(), None);
    let mut cmd = None;
    while let Some(word) = words.next() {
        let mut value = |key: &str| words.next().map(str::to_owned).ok_or(format!("{word} needs a value ({key})"));
        match word {
            "--pipe" => pipe = Some(value("a pipe name")?),
            "--pane" | "-p" => {
                options.insert("pane".into(), value("a pane id")?.into());
            }
            "--lines" | "-n" => {
                let lines = value("a number")?;
                options.insert("lines".into(), lines.parse::<u64>().map(Value::from).map_err(|_| format!("--lines takes a number, not {lines}"))?);
            }
            "--profile" => {
                options.insert("profile".into(), value("a profile")?.into());
            }
            "--cwd" | "-d" => {
                options.insert("cwd".into(), value("a folder")?.into());
            }
            "--direction" => {
                options.insert("direction".into(), value("right or down")?.into());
            }
            "--" => command = Some(words.by_ref().map(str::to_owned).collect::<Vec<_>>()),
            _ if cmd.is_none() => cmd = Some(word.to_owned()),
            _ => rest.push(word.to_owned()),
        }
    }
    let cmd = cmd.ok_or("a command is missing: list, send-text, get-text, new-tab, split, focus, close, set-title, notify, subscribe")?;
    let mut request = json!({ "cmd": cmd });
    let object = request.as_object_mut().expect("an object");
    let takes_pane = !matches!(cmd.as_str(), "list" | "new-tab" | "subscribe");
    if takes_pane && !options.contains_key("pane") && let Some(pane) = pane.filter(|pane| !pane.is_empty()) {
        options.insert("pane".into(), pane.into());
    }
    object.extend(options);
    if let Some(command) = command {
        object.insert("command".into(), command.into());
    }
    match cmd.as_str() {
        "send-text" => {
            let text = if rest.is_empty() { stdin() } else { rest.join(" ") };
            object.insert("text".into(), text.into());
        }
        "set-title" => {
            object.insert("title".into(), rest.join(" ").into());
        }
        "notify" => {
            let (title, body) = rest.split_first().ok_or("notify needs a title")?;
            object.insert("title".into(), title.clone().into());
            object.insert("body".into(), body.join(" ").into());
        }
        "subscribe" if !rest.is_empty() => {
            object.insert("events".into(), rest.into());
        }
        _ if !rest.is_empty() => return Err(format!("{cmd} does not take {}", rest.join(" "))),
        _ => {}
    }
    // What the instance would refuse is told without asking it.
    crate::api::parse(&request.to_string()).1?;
    Ok((pipe, request))
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
    fn a_saved_layout_is_opened_by_its_name_and_more_tabs_can_follow() {
        assert_eq!(parse(&args("--layout agents")), [Action::Layout("agents".into())]);
        let parsed = parse(&["--layout".to_owned(), "two words".to_owned(), ";".to_owned(), "nt".to_owned()]);
        assert_eq!(parsed, [Action::Layout("two words".into()), Action::NewTab(Target::default())]);
        // Without a name it is a plain tab.
        assert_eq!(parse(&args("--layout")), [Action::NewTab(Target::default())]);
    }

    fn request(line: &str, pane: Option<&str>) -> Result<(Option<String>, serde_json::Value), String> {
        api_request(&args(line), pane.map(str::to_owned), || "from stdin".to_owned())
    }

    #[test]
    fn a_cli_command_becomes_the_request_of_the_api_with_the_pane_it_runs_in() {
        use serde_json::json;
        assert_eq!(request("list", Some("7-1")), Ok((None, json!({ "cmd": "list" }))), "list names no pane");
        assert_eq!(request("send-text echo hi", Some("7-1")), Ok((None, json!({ "cmd": "send-text", "pane": "7-1", "text": "echo hi" }))));
        assert_eq!(request("send-text --pane 9-2", Some("7-1")).unwrap().1, json!({ "cmd": "send-text", "pane": "9-2", "text": "from stdin" }));
        assert_eq!(request("--pipe P get-text -n 50", Some("7-1")), Ok((Some("P".into()), json!({ "cmd": "get-text", "pane": "7-1", "lines": 50 }))));
        assert_eq!(
            request(r"split --direction down -d D:\x -- cmd /k ver", Some("7-1")).unwrap().1,
            json!({ "cmd": "split", "pane": "7-1", "direction": "down", "cwd": r"D:\x", "command": ["cmd", "/k", "ver"] })
        );
        assert_eq!(request("new-tab --profile pwsh", Some("7-1")).unwrap().1, json!({ "cmd": "new-tab", "profile": "pwsh" }));
        assert_eq!(request("set-title build box", Some("7-1")).unwrap().1, json!({ "cmd": "set-title", "pane": "7-1", "title": "build box" }));
        assert_eq!(request("notify Done all tests pass", None).unwrap().1, json!({ "cmd": "notify", "title": "Done", "body": "all tests pass" }));
        assert_eq!(request("subscribe output", None).unwrap().1, json!({ "cmd": "subscribe", "events": ["output"] }));
    }

    #[test]
    fn a_cli_command_the_api_would_refuse_fails_before_it_is_sent() {
        assert_eq!(request("", None).unwrap_err().split(':').next(), Some("a command is missing"));
        assert_eq!(request("focus", None).unwrap_err(), r#""pane" is missing"#, "outside a pane, the pane must be named");
        assert_eq!(request("get-text -n many", Some("7-1")).unwrap_err(), "--lines takes a number, not many");
        assert_eq!(request("get-text --pane", None).unwrap_err(), "--pane needs a value (a pane id)");
        assert_eq!(request("close extra", Some("7-1")).unwrap_err(), "close does not take extra");
        assert_eq!(request("split --direction up", Some("7-1")).unwrap_err(), "\"direction\" is right or down, not up");
        assert_eq!(request("notify", None).unwrap_err(), "notify needs a title");
    }

    #[test]
    fn a_cli_command_maps_its_options_wherever_they_stand() {
        use serde_json::json;
        assert_eq!(request("-p 3-1 -n 5 get-text", None).unwrap().1, json!({ "cmd": "get-text", "pane": "3-1", "lines": 5 }), "options before the command");
        assert_eq!(request("new-tab -d C:\\ -- wsl -d Ubuntu", None).unwrap().1, json!({ "cmd": "new-tab", "cwd": "C:\\", "command": ["wsl", "-d", "Ubuntu"] }), "after -- the words are the program's");
        assert_eq!(request("subscribe", Some("7-1")).unwrap().1, json!({ "cmd": "subscribe" }), "no events, the default ones");
        assert_eq!(request("focus", Some("")).unwrap_err(), r#""pane" is missing"#, "an empty NEKOMINAL_PANE is none");
        // Text given as words does not read stdin, which may be a terminal waiting for input.
        let words = api_request(&args("send-text ls"), Some("1-1".into()), || panic!("stdin read")).unwrap().1;
        assert_eq!(words["text"], "ls");
        let piped = api_request(&args("send-text"), Some("1-1".into()), || "line 1\r\nline 2\n".to_owned()).unwrap().1;
        assert_eq!(piped["text"], "line 1\r\nline 2\n", "text from stdin goes as it is, its line breaks too");
        assert_eq!(api_request(&args("send-text"), Some("1-1".into()), String::new).unwrap_err(), r#""text" is empty"#);
    }

    #[test]
    fn a_cli_command_with_a_bad_value_names_it() {
        assert_eq!(request("get-text -n 0", Some("7-1")).unwrap_err(), r#""lines" must be a whole number above 0"#);
        assert_eq!(request("get-text -n -1", Some("7-1")).unwrap_err(), "--lines takes a number, not -1");
        assert_eq!(request("list --pipe", None).unwrap_err(), "--pipe needs a value (a pipe name)");
        assert_eq!(request("list all", None).unwrap_err(), "list does not take all");
        assert_eq!(request("reboot", None).unwrap_err(), "unknown command reboot");
        assert!(request("subscribe keys", None).unwrap_err().starts_with("unknown event \"keys\""));
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
