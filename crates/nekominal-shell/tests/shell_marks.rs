//! The shell integration in a real interactive PowerShell under ConPTY: what the marks it writes tell the terminal.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, Instant},
};

use nekominal_model::{profiles, shell_integration::powershell_args};
use nekominal_pane::{Pane, UiEvent};
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_term::{CoreConfig, osc::PromptMark};

/// Waits up to 30 seconds for `done`, checking the pane's terminal every 100 ms.
fn wait(pane: &Pane, mut done: impl FnMut(&Pane) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if done(pane) {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    false
}

#[test]
#[ignore = "starts Windows PowerShell under ConPTY"]
fn a_typed_command_s_output_is_marked_from_its_first_line_to_its_last_with_its_exit_code() {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let config = SpawnConfig {
        program: std::path::Path::new(&root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
        args: powershell_args(&["-NoLogo".to_owned(), "-NoProfile".to_owned()]),
        cwd: None,
        env: BTreeMap::new(),
        cols: 80,
        rows: 30,
        width_model: WidthModel::Wcswidth,
        inherit_environment: false,
    };
    let pane = Pane::spawn_with(config, CoreConfig::default()).expect("powershell starts under ConPTY");
    let prompted = |pane: &Pane| pane.term.lock().prompt_lines().len();
    assert!(wait(&pane, |pane| prompted(pane) >= 1), "the first prompt is marked");
    pane.send(b"Write-Output one two\r".to_vec());
    let screen = |pane: &Pane| pane.term.lock().text_between((-50, 0), (29, 79));
    assert!(wait(&pane, |pane| prompted(pane) >= 2), "the prompt after the command is marked; the screen:\n{}", screen(&pane));
    pane.send(b"cmd /c exit 3\r".to_vec());
    assert!(wait(&pane, |pane| prompted(pane) >= 3), "and after the failing one");
    let outputs = pane.term.lock().command_outputs();
    let text = pane.term.lock().last_command_output();
    let first = pane.term.lock().text_between((outputs[0].first, 0), (outputs[0].last, 79));
    pane.shutdown();
    assert_eq!(outputs.len(), 1, "the failing command wrote nothing: {outputs:?}");
    assert_eq!(first, "one\ntwo", "the output starts on the line after the command typed");
    assert_eq!(outputs[0].exit, Some(0));
    assert_eq!(text.as_deref(), Some("one\ntwo"));
}

/// The shell the discovery step finds of `kind`, as a discovered profile of the model.
fn found(kind: &str) -> Option<profiles::Profile> {
    let shell = nekominal_pty::profiles::discover().into_iter().find(|p| p.kind == kind && (kind != "cmd" || p.id == "cmd"))?;
    let discovered = profiles::DiscoveredProfile { id: shell.id, name: shell.name, kind: shell.kind, icon: shell.icon, program: shell.program, args: shell.args };
    profiles::merge(&[discovered], &[]).pop()
}

/// The profile started under ConPTY as the window starts it with shell integration on, plus `env`, in `cwd`.
fn spawn(profile: &profiles::Profile, env: &[(&str, &str)], cwd: Option<&Path>) -> Pane {
    let mut all = profiles::launch_env(profile, true);
    all.extend(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let config = SpawnConfig {
        program: profile.program.clone().into(),
        args: profiles::launch_args(profile, true),
        cwd: cwd.map(Path::to_path_buf),
        env: all,
        cols: 100,
        rows: 30,
        width_model: WidthModel::Wcswidth,
        inherit_environment: true,
    };
    Pane::spawn_with(config, CoreConfig::default()).expect("the shell starts under ConPTY")
}

fn prompts(pane: &Pane) -> usize {
    pane.term.lock().prompt_lines().len()
}

fn screen(pane: &Pane) -> String {
    pane.term.lock().text_between((-200, 0), (29, 99))
}

/// Types `line` and waits for the prompt after it.
fn run(pane: &Pane, line: &str) {
    let before = prompts(pane);
    pane.send(format!("{line}\r").into_bytes());
    assert!(wait(pane, |pane| prompts(pane) > before), "no prompt after {line:?}; the screen:\n{}", screen(pane));
}

fn reported(pane: &Pane) -> (Vec<String>, Vec<PromptMark>) {
    let (mut dirs, mut marks) = (Vec::new(), Vec::new());
    for event in pane.events.try_iter() {
        match event {
            UiEvent::Cwd(dir) => dirs.push(dir),
            UiEvent::PromptMark(mark) => marks.push(mark),
            _ => {}
        }
    }
    (dirs, marks)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nekominal {name} {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "starts cmd under ConPTY"]
fn cmd_marks_its_prompts_reports_its_folder_and_has_claude_take_the_settings() {
    let cmd = found("cmd").expect("cmd is found");
    let dir = temp_dir("cmd 你好");
    let pane = spawn(&cmd, &[("NEKOMINAL_CLAUDE_SETTINGS", r"C:\n x\claude-settings.json")], Some(&dir));
    assert!(wait(&pane, |pane| prompts(pane) >= 1), "the first prompt is marked; the screen:\n{}", screen(&pane));
    run(&pane, "echo one");
    run(&pane, "doskey /macros");
    let text = screen(&pane);
    let (dirs, marks) = reported(&pane);
    pane.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(text.contains(&format!("{}>echo one\none", dir.display())), "the prompt still looks like $P$G:\n{text}");
    assert!(text.contains(r#"claude=claude --settings "C:\n x\claude-settings.json" $*"#), "{text}");
    assert_eq!(dirs.last().map(PathBuf::from), Some(dir.clone()), "{dirs:?}");
    assert!(marks.contains(&PromptMark::CommandEnd { exit_code: None }), "cmd has no exit code to give: {marks:?}");
    assert!(marks.contains(&PromptMark::CommandStart), "{marks:?}");
}

/// A home of the test's own, so that the user's files do not change what the shell prints: its login file sets a prompt
/// and a PROMPT_COMMAND that keeps the status it sees.
fn bash_home(name: &str) -> PathBuf {
    let home = temp_dir(name);
    std::fs::write(home.join(".bash_profile"), "PROMPT_COMMAND='__mine=$?'\nPS1='mine\\$ '\n").unwrap();
    home
}

/// What a bash with the integration did with three commands: one that writes two lines, one that fails with 3, and one
/// that reports the status the user's PROMPT_COMMAND saw.
fn check_bash(pane: &Pane, home_shown: &str) {
    assert!(wait(pane, |pane| prompts(pane) >= 1), "the first prompt is marked; the screen:\n{}", screen(pane));
    run(pane, "printf 'one\\ntwo\\n'");
    run(pane, "echo bad; (exit 3)");
    run(pane, "echo \"mine=$__mine\"; type claude | grep -c -- --settings");
    let outputs = pane.term.lock().command_outputs();
    let texts: Vec<String> = outputs.iter().map(|o| pane.term.lock().text_between((o.first, 0), (o.last, 99))).collect();
    let text = screen(pane);
    assert_eq!(texts, ["one\ntwo", "bad", "mine=3\n1"], "the screen:\n{text}");
    assert_eq!(outputs.iter().map(|o| o.exit).collect::<Vec<_>>(), [Some(0), Some(3), Some(0)]);
    assert!(text.contains("mine$ printf"), "the user's prompt is kept:\n{text}");
    let (dirs, _) = reported(pane);
    assert_eq!(dirs.last().map(String::as_str), Some(home_shown), "{dirs:?}");
}

#[test]
#[ignore = "starts Git Bash under ConPTY"]
fn git_bash_marks_prompts_outputs_and_exit_codes_and_keeps_the_user_s_prompt_command() {
    let Some(bash) = found("bash") else {
        eprintln!("Git Bash is not installed: skipped");
        return;
    };
    let home = bash_home("bash");
    let pane = spawn(&bash, &[("HOME", &home.to_string_lossy()), ("NEKOMINAL_CLAUDE_SETTINGS", r"C:\n\claude-settings.json")], Some(&home));
    let shown = home.to_string_lossy().into_owned();
    check_bash(&pane, &shown);
    pane.shutdown();
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
#[ignore = "starts WSL under ConPTY"]
fn wsl_bash_marks_prompts_outputs_and_exit_codes() {
    let Some(wsl) = found("wsl") else {
        eprintln!("no WSL distribution: skipped");
        return;
    };
    // The distribution's own login files run (the default shell may not even be bash: then nothing is marked).
    let dir = temp_dir("wsl");
    let pane = spawn(&wsl, &[("NEKOMINAL_CLAUDE_SETTINGS", r"C:\n\claude-settings.json")], None);
    assert!(wait(&pane, |pane| prompts(pane) >= 1), "the first prompt is marked; the screen:\n{}", screen(&pane));
    run(&pane, &format!("cd \"$(wslpath -u '{}')\"", dir.display()));
    run(&pane, "printf 'one\\ntwo\\n'");
    run(&pane, "echo bad; (exit 3)");
    run(&pane, "type claude | grep -c -- claude-settings-wsl.json");
    let outputs = pane.term.lock().command_outputs();
    let texts: Vec<String> = outputs.iter().map(|o| pane.term.lock().text_between((o.first, 0), (o.last, 99))).collect();
    let (dirs, _) = reported(&pane);
    pane.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(texts, ["one\ntwo", "bad", "1"]);
    assert_eq!(outputs.iter().map(|o| o.exit).collect::<Vec<_>>(), [Some(0), Some(3), Some(0)]);
    assert!(dirs.contains(&dir.to_string_lossy().into_owned()), "a folder on a Windows drive is reported as Windows sees it: {dirs:?}");
}
