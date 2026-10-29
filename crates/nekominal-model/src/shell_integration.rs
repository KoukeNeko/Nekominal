//! Shell integration: a script that makes PowerShell report its working directory and where each prompt, command
//! and exit status begins, as OSC 133 marks (`A` prompt start, `B` command start, `C` output start, `D;<code>` command end)
//! plus the directory (`OSC 9;9`). The emulator reads them to jump between prompts and to open new tabs in the
//! current directory.
//!
//! The PowerShell script also gives Claude Code the status line of the window: when `NEKOMINAL_CLAUDE_SETTINGS` names a
//! settings file (the window sets it while its status bar is on), `claude` runs `claude.exe --settings <that file>`,
//! unless the user has a `claude` function of their own; where the project's `.claude\settings*.json` sets a status line,
//! the file beside it that holds only the hooks (`claude-hooks.json`).
//! In the same way `codex`, `gemini` and `aider` get the window's hook for one run (`NEKOMINAL_CODEX_NOTIFY`, `_GEMINI_DEFAULTS`,
//! `_AIDER_NOTIFY`, see the shell's `hooks.rs`), unless the user's own configuration already sets that hook.
//!
//! It is wired into the PowerShell profiles by [`powershell_args`].
//!
//! cmd, Git Bash and the bash of WSL get the same marks from a script file this program writes ([`script_file`]): the
//! user's own files are never edited, the way VS Code injects its integration. cmd runs [`CMD_SCRIPT`] with `/k`; bash
//! starts with [`BASH_SCRIPT`] as its `--init-file`, which reads the user's login files first.

use std::{collections::BTreeMap, path::{Path, PathBuf}};

use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Wraps `prompt`, so a custom prompt keeps working. The script runs after the user's profile
/// (`-EncodedCommand` is not subject to the execution policy), and loading it twice changes nothing.
pub const POWERSHELL_SCRIPT: &str = r#"if (-not $global:__NekominalPrompt) {
  $global:__NekominalPrompt = $function:prompt
  function global:prompt {
    $success = $?
    $native = $global:LASTEXITCODE
    $esc = [char]27
    $bel = [char]7
    $exit = if ($success) { 0 } elseif ($native) { $native } else { 1 }
    $cwd = if ($PWD.Provider.Name -eq 'FileSystem') { $PWD.ProviderPath } else { '' }
    $text = & $global:__NekominalPrompt
    $global:LASTEXITCODE = $native
    ('{0}]133;D;{1}{2}{0}]133;A{2}{0}]9;9;"{3}"{2}{4}{0}]133;B{2}' -f $esc, $exit, $bel, $cwd, $text)
  }
  # The line typed is accepted when ReadLine returns: what follows is the command's output (133;C).
  $readLine = Get-Command PSConsoleHostReadLine -CommandType Function -ErrorAction SilentlyContinue
  if ($readLine) {
    $global:__NekominalReadLine = $readLine.ScriptBlock
    function global:PSConsoleHostReadLine {
      $line = & $global:__NekominalReadLine
      [Console]::Write("$([char]27)]133;C$([char]7)")
      $line
    }
  }
}
if ($env:NEKOMINAL_CLAUDE_SETTINGS -and -not (Test-Path function:global:claude)) {
  function global:claude {
    $exe = (Get-Command claude -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if (-not $exe -and (Test-Path "$HOME\.local\bin\claude.exe")) { $exe = "$HOME\.local\bin\claude.exe" }
    # A project that sets a status line of its own keeps it (--settings would win over it): only the hooks go in.
    $settings = $env:NEKOMINAL_CLAUDE_SETTINGS
    $own = Get-ChildItem -LiteralPath (Join-Path $PWD.ProviderPath '.claude') -Filter 'settings*.json' -ErrorAction SilentlyContinue | Select-String -SimpleMatch '"statusLine"' -Quiet
    if ($own) { $settings = $settings -replace 'claude-settings\.json$', 'claude-hooks.json' }
    if (-not $exe) { Write-Error 'claude.exe was not found' } else { & $exe --settings $settings @args }
  }
}
if ($env:NEKOMINAL_CODEX_NOTIFY -and -not (Test-Path function:global:codex)) {
  function global:codex {
    $exe = (Get-Command codex -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    $config = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { "$HOME\.codex" }
    # A notify of the user's own stays: the override would replace it for the run.
    $own = Get-Item -LiteralPath (Join-Path $config 'config.toml'), (Join-Path $PWD.ProviderPath '.codex\config.toml') -ErrorAction SilentlyContinue | Select-String -Pattern '^\s*notify\s*=' -Quiet
    if (-not $exe) { Write-Error 'codex was not found' } elseif ($own) { & $exe @args } else { & $exe -c $env:NEKOMINAL_CODEX_NOTIFY @args }
  }
}
if ($env:NEKOMINAL_GEMINI_DEFAULTS -and -not (Test-Path function:global:gemini)) {
  function global:gemini {
    $exe = (Get-Command gemini -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    # Only where nothing else is the layer of system defaults: the variable would hide an administrator's file.
    $taken = $env:GEMINI_CLI_SYSTEM_DEFAULTS_PATH -or (Test-Path -LiteralPath "$env:ProgramData\gemini-cli\system-defaults.json")
    if (-not $exe) { Write-Error 'gemini was not found' } elseif ($taken) { & $exe @args } else {
      $env:GEMINI_CLI_SYSTEM_DEFAULTS_PATH = $env:NEKOMINAL_GEMINI_DEFAULTS
      try { & $exe @args } finally { Remove-Item Env:GEMINI_CLI_SYSTEM_DEFAULTS_PATH }
    }
  }
}
if ($env:NEKOMINAL_AIDER_NOTIFY -and -not (Test-Path function:global:aider)) {
  function global:aider {
    $exe = (Get-Command aider -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    # A notifications command of the user's own stays: the flag would win over it.
    $own = $env:AIDER_NOTIFICATIONS_COMMAND -or (Get-Item -LiteralPath "$HOME\.aider.conf.yml", (Join-Path $PWD.ProviderPath '.aider.conf.yml') -ErrorAction SilentlyContinue | Select-String -Pattern '^\s*notifications-command\s*:' -Quiet)
    if (-not $exe) { Write-Error 'aider was not found' } elseif ($own) { & $exe @args } else { & $exe --notifications --notifications-command $env:NEKOMINAL_AIDER_NOTIFY @args }
  }
}"#;

/// Switches after which PowerShell runs the command or script the profile names; `-EncodedCommand` cannot
/// be added next to them. Compared without the leading `-` or `/` and without case.
const OWN_COMMAND_SWITCHES: &[&str] = &[
    "command",
    "c",
    "commandwithargs",
    "cwa",
    "encodedcommand",
    "e",
    "ec",
    "file",
    "f",
];

fn is_switch(arg: &str, names: &[&str]) -> bool {
    let name = arg.trim_start_matches(['-', '/']).to_ascii_lowercase();
    names.contains(&name.as_str())
}

/// `args` with the integration appended: `-NoExit -EncodedCommand <script>`, so the session stays open after
/// the script has run. A profile that already runs its own command or file (the Visual Studio developer
/// shells) comes back unchanged, since PowerShell takes only one.
pub fn powershell_args(args: &[String]) -> Vec<String> {
    let mut all = args.to_vec();
    if args.iter().any(|arg| is_switch(arg, OWN_COMMAND_SWITCHES)) {
        return all;
    }
    if !args.iter().any(|arg| is_switch(arg, &["noexit"])) {
        all.push("-NoExit".to_owned());
    }
    all.push("-EncodedCommand".to_owned());
    all.push(encode_command(POWERSHELL_SCRIPT));
    all
}

/// The form `-EncodedCommand` takes: the script as UTF-16LE, base64.
fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    STANDARD.encode(bytes)
}

/// cmd's prompt with the marks: `D` (cmd has no exit code to give), `A`, the directory, `$P$G`, `B`. Only cmd's default
/// prompt is replaced: a `PROMPT` of the user's (set in the environment, the profile or AutoRun) stays. Quotes are taken
/// out of `%PROMPT%` before the comparison so that no prompt can break the line. `claude` is a doskey macro that adds the
/// settings of the window, while it names some.
pub const CMD_SCRIPT: &str = r#"@if "%PROMPT:"=%"=="$P$G" prompt $e]133;D$e\$e]133;A$e\$e]9;9;$P$e\$P$G$e]133;B$e\
@if defined NEKOMINAL_CLAUDE_SETTINGS doskey claude=claude --settings "%NEKOMINAL_CLAUDE_SETTINGS%" $*
"#;

/// Bash with the marks. Started with this file as `--init-file` instead of `--login`, it reads what a login shell reads
/// first, then wraps the prompt: `PROMPT_COMMAND` writes `D;<status>` and the directory (as Windows sees it: `cygpath -w`
/// in Git Bash, `wslpath -w` in WSL when the folder is on a Windows drive, else the Linux path), `PS1` gets `A` and `B`
/// around it and `PS0` writes `C`. The user's own `PROMPT_COMMAND` (a string or an array) still runs, first, with the
/// status of the command. In WSL the file is first read by `sh` (see [`wsl_args`]), which starts the user's shell: bash
/// with this file, any other as a login shell.
pub const BASH_SCRIPT: &str = r#"if [ -z "$BASH_VERSION" ]; then
  unset IFS
  __nk_shell=$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)
  __nk_shell=${__nk_shell:-${SHELL:-/bin/sh}}
  case ${__nk_shell##*/} in
    bash) exec "$__nk_shell" --init-file "$NEKOMINAL_BASH_INIT" -i ;;
  esac
  exec "$__nk_shell" -l
fi
declare -F __nk_precmd >/dev/null && return

[ -r /etc/profile ] && . /etc/profile
if [ -r ~/.bash_profile ]; then . ~/.bash_profile
elif [ -r ~/.bash_login ]; then . ~/.bash_login
elif [ -r ~/.profile ]; then . ~/.profile
fi

__nk_commands=("${PROMPT_COMMAND[@]}")
unset PROMPT_COMMAND
PROMPT_COMMAND=__nk_precmd
type -P cygpath >/dev/null && __nk_cygpath=1

__nk_set_status() { return "$1"; }
__nk_dir() {
  if [ -n "$__nk_cygpath" ]; then cygpath -w "$PWD"
  elif [ -n "$WSL_DISTRO_NAME" ] && __nk_win=$(wslpath -w "$PWD" 2>/dev/null) && [[ $__nk_win == [A-Za-z]:* ]]; then printf '%s' "$__nk_win"
  else printf '%s' "$PWD"
  fi
}
__nk_precmd() {
  local __nk_status=$? __nk_line
  for __nk_line in "${__nk_commands[@]}"; do __nk_set_status "$__nk_status"; eval "$__nk_line"; done
  if [ "$PWD" != "$__nk_pwd" ]; then __nk_pwd=$PWD; __nk_cwd=$(__nk_dir); fi
  # A prompt command that sets PS1 again each time drops the marks: they are put back.
  [[ $PS1 == *'133;A'* ]] || PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
  [[ $PS0 == *'133;C'* ]] || PS0='\e]133;C\a'"$PS0"
  printf '\e]133;D;%s\a\e]9;9;"%s"\a' "$__nk_status" "$__nk_cwd"
  return "$__nk_status"
}

if [ -n "$NEKOMINAL_CLAUDE_SETTINGS" ] && ! declare -F claude >/dev/null; then
  claude() {
    local settings=$NEKOMINAL_CLAUDE_SETTINGS
    # A project that sets a status line of its own keeps it (--settings would win over it): only the hooks go in.
    grep -qs '"statusLine"' .claude/settings*.json </dev/null && settings=${settings%claude-settings.json}claude-hooks.json
    # Claude Code in WSL is a Linux program: its settings run the status line and hooks through interop.
    [ -n "$WSL_DISTRO_NAME" ] && settings=${settings%.json}-wsl.json
    command claude --settings "$settings" "$@"
  }
fi
:
"#;

/// The variable that names [`BASH_SCRIPT`] in WSL, which translates it to the Linux path (`WSLENV` with `/p`).
pub const BASH_INIT_VAR: &str = "NEKOMINAL_BASH_INIT";

/// Writes `text` to `name` in the folder of the shell scripts (`%LOCALAPPDATA%\Nekominal\shell`), unless it is there
/// already, and returns its path. Line ends are LF whatever the checkout made of this source: bash fails on a CR. Written
/// beside and moved over, under this process's name: another Nekominal starting a shell at the same moment reads the old
/// script or the new one, never half of one.
pub fn script_file(name: &str, text: &str) -> Option<PathBuf> {
    let file = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Nekominal").join("shell").join(name);
    let text = text.replace('\r', "");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        std::fs::create_dir_all(file.parent()?).ok()?;
        let part = file.with_extension(format!("{}.part", std::process::id()));
        if std::fs::write(&part, &text).and_then(|()| std::fs::rename(&part, &file)).is_err() {
            let _ = std::fs::remove_file(&part);
            return None;
        }
    }
    Some(file)
}

/// cmd's `args` with `call <init>` run by `/k`: added after the profile's own `/k` command, or as a new `/k`. A profile
/// that runs a command and exits (`/c`, `/r`) comes back unchanged, as does one whose switch is glued to its command.
/// The banner of cmd is not shown with `/k`.
pub fn cmd_args(args: &[String], init: &Path) -> Vec<String> {
    let init = init.to_string_lossy().into_owned();
    let mut all = args.to_vec();
    for arg in args {
        let lower = arg.to_ascii_lowercase();
        if lower == "/k" {
            all.extend(["&".to_owned(), "call".to_owned(), init]);
            return all;
        }
        if lower.starts_with("/c") || lower.starts_with("/r") || lower.starts_with("/k") {
            return all;
        }
    }
    all.extend(["/k".to_owned(), "call".to_owned(), init]);
    all
}

/// Git Bash's `args` replaced by `--init-file <init> -i`, which then reads the login files itself. Only an interactive
/// login shell (`--login -i`, `-l`, `-li`) is changed: anything else (a command, a script, other options) runs as written.
/// The path has forward slashes, which bash of MSYS takes as they are.
pub fn bash_args(args: &[String], init: &Path) -> Vec<String> {
    let known = ["--login", "-l", "-i", "--interactive", "-li", "-il"];
    let login = args.iter().any(|arg| matches!(arg.as_str(), "--login" | "-l" | "-li" | "-il"));
    if !login || !args.iter().all(|arg| known.contains(&arg.as_str())) {
        return args.to_vec();
    }
    vec!["--init-file".to_owned(), init.to_string_lossy().replace('\\', "/"), "-i".to_owned()]
}

/// The options of `wsl.exe` that take a value and choose where and as whom the shell starts, not what it runs.
const WSL_PLACE_OPTIONS: &[&str] = &["--distribution", "-d", "--cd", "--user", "-u", "--distribution-id"];

/// Whether `wsl.exe` with `args` starts the default shell of the distribution, which the integration may replace.
fn wsl_starts_default_shell(args: &[String]) -> bool {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if !WSL_PLACE_OPTIONS.contains(&arg.as_str()) || rest.next().is_none() {
            return false;
        }
    }
    true
}

/// WSL's `args` with `-e sh -c '. $NEKOMINAL_BASH_INIT'`: `sh` reads [`BASH_SCRIPT`], which starts the user's shell.
/// `IFS` is empty so that a path with spaces stays one word, and the line holds no double quote, which `wsl.exe`
/// would have to unescape. A profile that runs a command of its own comes back unchanged.
pub fn wsl_args(args: &[String]) -> Vec<String> {
    let mut all = args.to_vec();
    if wsl_starts_default_shell(args) {
        all.extend(["-e", "sh", "-c", "IFS=; . $NEKOMINAL_BASH_INIT"].map(str::to_owned));
    }
    all
}

/// The environment WSL needs for [`wsl_args`]: the script, and `WSLENV` naming it (as a path, into WSL only), the pane
/// (both ways: Claude Code in WSL runs this program's status line through interop) and the settings of Claude Code (as a
/// path), ahead of the user's `WSLENV`.
pub fn wsl_env(init: &Path, wslenv: Option<&str>) -> BTreeMap<String, String> {
    let ours = format!("NEKOMINAL_SESSION:NEKOMINAL_CLAUDE_SETTINGS/p:{BASH_INIT_VAR}/pu");
    let wslenv = match wslenv.filter(|v| !v.is_empty()) {
        Some(theirs) => format!("{ours}:{theirs}"),
        None => ours,
    };
    BTreeMap::from([
        (BASH_INIT_VAR.to_owned(), init.to_string_lossy().into_owned()),
        ("WSLENV".to_owned(), wslenv),
    ])
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process::Command};

    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    fn decode_command(encoded: &str) -> String {
        let bytes = STANDARD.decode(encoded).expect("valid base64");
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16(&units).expect("valid UTF-16")
    }

    #[test]
    fn appends_an_encoded_command_that_decodes_back_to_the_script() {
        let args = powershell_args(&strings(&["-NoLogo"]));
        assert_eq!(&args[..3], ["-NoLogo", "-NoExit", "-EncodedCommand"]);
        assert_eq!(args.len(), 4);
        assert_eq!(decode_command(&args[3]), POWERSHELL_SCRIPT);
    }

    #[test]
    fn the_encoding_is_utf16_little_endian_base64() {
        // "A" is 41 00 in UTF-16LE, "é" is E9 00, and U+1F600 is the surrogate pair 3D D8 00 DE.
        assert_eq!(encode_command("A"), "QQA=");
        assert_eq!(encode_command("é"), "6QA=");
        assert_eq!(encode_command("\u{1F600}"), "PdgA3g==");
        assert_eq!(encode_command(""), "");
    }

    #[test]
    fn leaves_the_original_arguments_untouched() {
        let original = strings(&["-NoLogo"]);
        let _ = powershell_args(&original);
        assert_eq!(original, ["-NoLogo"]);
    }

    #[test]
    fn works_without_arguments_and_does_not_repeat_no_exit() {
        let bare = powershell_args(&[]);
        assert_eq!(&bare[..2], ["-NoExit", "-EncodedCommand"]);
        let kept = powershell_args(&strings(&["-NoExit", "-NoProfile"]));
        assert_eq!(kept.iter().filter(|a| a.as_str() == "-NoExit").count(), 1);
        assert_eq!(&kept[..2], ["-NoExit", "-NoProfile"]);
        assert_eq!(kept[2], "-EncodedCommand");
    }

    #[test]
    fn a_profile_with_its_own_command_or_file_is_returned_unchanged() {
        for own in [
            "-Command",
            "-command",
            "-c",
            "/Command",
            "-CommandWithArgs",
            "-EncodedCommand",
            "-ec",
            "-File",
            "-f",
            "--command",
        ] {
            let args = strings(&["-NoLogo", "-NoExit", own, "&{ Import-Module x }"]);
            assert_eq!(powershell_args(&args), args, "{own}");
        }
        // Switches that merely start with the same letter do not count.
        for other in ["-NoProfile", "-ExecutionPolicy", "-ep", "-Version"] {
            assert_eq!(powershell_args(&strings(&[other])).len(), 4, "{other}");
        }
    }

    #[test]
    fn the_powershell_script_reports_the_directory_and_prompt_marks_and_nothing_else() {
        let payloads: Vec<&str> = POWERSHELL_SCRIPT
            .split("{0}]")
            .skip(1)
            .map(|rest| rest.split("{2}").next().unwrap())
            .collect();
        assert_eq!(payloads, ["133;D;{1}", "133;A", "9;9;\"{3}\"", "133;B"]);
        assert!(
            POWERSHELL_SCRIPT.contains("$global:LASTEXITCODE = $native"),
            "the user's $LASTEXITCODE survives the prompt"
        );
    }

    #[test]
    fn the_script_never_contains_a_raw_escape_or_bell_byte() {
        assert!(
            !POWERSHELL_SCRIPT.chars().any(|c| c == '\u{1b}' || c == '\u{7}'),
            "the sequences are spelled out, not embedded"
        );
    }

    #[test]
    fn cmd_runs_the_script_with_a_new_k_or_after_the_profile_s_own_k_command() {
        let init = Path::new(r"C:\Users\John Doe\AppData\Local\Nekominal\shell\cmd.cmd");
        let path = init.to_string_lossy().into_owned();
        assert_eq!(cmd_args(&[], init), ["/k", "call", &path]);
        assert_eq!(cmd_args(&strings(&["/d"]), init), ["/d", "/k", "call", &path]);
        let developer = strings(&["/k", "call", r"C:\VS\VsDevCmd.bat", "-arch=x64"]);
        assert_eq!(cmd_args(&developer, init), [&developer[..], &strings(&["&", "call", &path])].concat());
        assert_eq!(&cmd_args(&strings(&["/K", "echo", "hi"]), init)[3..],["&", "call", &path]);
        // A command that runs and exits, or a switch glued to its command, is left alone.
        for own in [&["/c", "dir"][..], &["/C", "dir"], &["/r", "dir"], &["/kecho", "hi"], &["/d", "/c", "x"]] {
            assert_eq!(cmd_args(&strings(own), init), strings(own), "{own:?}");
        }
    }

    #[test]
    fn the_cmd_script_keeps_a_prompt_of_the_user_s_own_and_has_no_exit_code_to_give() {
        let prompt = CMD_SCRIPT.lines().next().unwrap();
        assert!(prompt.starts_with(r#"@if "%PROMPT:"=%"=="$P$G" prompt "#), "only cmd's default prompt is replaced");
        assert!(prompt.ends_with(r"$e]133;D$e\$e]133;A$e\$e]9;9;$P$e\$P$G$e]133;B$e\"), "{prompt}");
        assert!(CMD_SCRIPT.contains(r#"doskey claude=claude --settings "%NEKOMINAL_CLAUDE_SETTINGS%" $*"#));
        assert!(CMD_SCRIPT.lines().all(|line| line.starts_with('@')), "no line is echoed");
    }

    #[test]
    fn git_bash_starts_with_the_init_file_only_as_an_interactive_login_shell() {
        let init = Path::new(r"C:\Users\John Doe\AppData\Local\Nekominal\shell\bash.sh");
        let expected = ["--init-file", "C:/Users/John Doe/AppData/Local/Nekominal/shell/bash.sh", "-i"];
        for login in [&["--login", "-i"][..], &["-l"], &["-li"], &["-i", "--login"]] {
            assert_eq!(bash_args(&strings(login), init), expected, "{login:?}");
        }
        for own in [&[][..], &["-i"], &["--login", "-c", "make"], &["-l", "script.sh"], &["--login", "--norc", "-i"], &["--rcfile", "x", "-i"]] {
            assert_eq!(bash_args(&strings(own), init), strings(own), "{own:?}");
        }
    }

    #[test]
    fn wsl_reads_the_script_with_sh_only_when_it_starts_the_default_shell() {
        let found = strings(&["--distribution", "Ubuntu 24.04", "--cd", "~"]);
        let with = wsl_args(&found);
        assert_eq!(&with[..4], &found[..]);
        assert_eq!(&with[4..], ["-e", "sh", "-c", "IFS=; . $NEKOMINAL_BASH_INIT"]);
        assert!(!with.iter().any(|arg| arg.contains('"')), "wsl.exe never has to unescape a quote");
        assert_eq!(wsl_args(&strings(&["-d", "Debian", "-u", "root"])).len(), 8);
        assert_eq!(wsl_args(&[]).len(), 4);
        for own in [&["-d", "Ubuntu", "-e", "htop"][..], &["-d", "Ubuntu", "--", "ls"], &["--exec", "zsh"], &["-d"], &["--shell-type", "login"], &["ls"]] {
            assert_eq!(wsl_args(&strings(own)), strings(own), "{own:?}");
        }
    }

    #[test]
    fn wsl_gets_the_script_and_the_pane_through_wslenv_ahead_of_the_user_s_own() {
        let init = Path::new(r"C:\n\shell\bash.sh");
        let env = wsl_env(init, None);
        assert_eq!(env[BASH_INIT_VAR], r"C:\n\shell\bash.sh");
        assert_eq!(env["WSLENV"], "NEKOMINAL_SESSION:NEKOMINAL_CLAUDE_SETTINGS/p:NEKOMINAL_BASH_INIT/pu");
        assert_eq!(wsl_env(init, Some("")).get("WSLENV"), env.get("WSLENV"));
        assert_eq!(wsl_env(init, Some("GOPATH/l:USERPROFILE/pu"))["WSLENV"], "NEKOMINAL_SESSION:NEKOMINAL_CLAUDE_SETTINGS/p:NEKOMINAL_BASH_INIT/pu:GOPATH/l:USERPROFILE/pu");
    }

    #[test]
    fn the_bash_script_keeps_the_user_s_prompt_command_and_marks_every_part_of_a_command() {
        for part in [
            r#"__nk_commands=("${PROMPT_COMMAND[@]}")"#,
            "unset PROMPT_COMMAND",
            r#"__nk_set_status "$__nk_status"; eval "$__nk_line""#,
            r#"PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'"#,
            r#"PS0='\e]133;C\a'"$PS0""#,
            r#"printf '\e]133;D;%s\a\e]9;9;"%s"\a' "$__nk_status" "$__nk_cwd""#,
            r#"cygpath -w "$PWD""#,
            r#"wslpath -w "$PWD""#,
            r#"exec "$__nk_shell" --init-file "$NEKOMINAL_BASH_INIT" -i"#,
        ] {
            assert!(BASH_SCRIPT.contains(part), "{part}");
        }
        assert!(!BASH_SCRIPT.chars().any(|c| c == '\u{1b}' || c == '\u{7}'));
        // The login files are read before the prompt is wrapped, so a PROMPT_COMMAND they set is the one kept.
        assert!(BASH_SCRIPT.find(". ~/.bash_profile").unwrap() < BASH_SCRIPT.find("__nk_commands=").unwrap());
    }

    #[test]
    fn a_script_file_is_written_once_with_lf_line_ends() {
        let file = script_file("test-script.sh", "a\r\nb\n").expect("LOCALAPPDATA is set");
        assert_eq!(std::fs::read(&file).unwrap(), b"a\nb\n");
        let written = std::fs::metadata(&file).unwrap().modified().unwrap();
        assert_eq!(script_file("test-script.sh", "a\nb\n"), Some(file.clone()));
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), written, "the same text is not written again");
        std::fs::remove_file(file).unwrap();
    }

    fn find_program(names: &[&str]) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
            .find(|candidate| candidate.exists())
    }

    /// Runs the PowerShell script in a real `pwsh`, then asks the wrapped prompt what it would print,
    /// base64-encoded so the escape bytes cannot be altered on the way out. `run_before` is a command
    /// to run first, to set `$?` and `$LASTEXITCODE`.
    fn real_powershell_prompt(run_before: &str, directory: &std::path::Path) -> String {
        let pwsh = find_program(&["pwsh.exe"]).expect("pwsh is on PATH");
        // `$p = prompt` is a statement of its own, as at a real prompt, so `$?` still belongs to `run_before`.
        let probe = format!(
            "{POWERSHELL_SCRIPT}\n{run_before}\n$p = prompt\n[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($p))"
        );
        let output = Command::new(pwsh)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-EncodedCommand",
                &encode_command(&probe),
            ])
            .current_dir(directory)
            // NO_COLOR would not change the bytes of a plain string, but keep the run independent of the host.
            .env_remove("NO_COLOR")
            .output()
            .expect("pwsh starts");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let encoded = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        String::from_utf8(
            STANDARD
                .decode(&encoded)
                .unwrap_or_else(|e| panic!("{e}: {encoded:?}")),
        )
        .expect("UTF-8 prompt")
    }

    /// Needs PowerShell 7 on PATH; run with `cargo test -p nekominal-model -- --ignored`.
    #[test]
    #[ignore = "starts pwsh"]
    fn real_powershell_marks_success_and_failure_and_reports_the_directory() {
        let directory = std::env::temp_dir().join("nekominal model 你好");
        std::fs::create_dir_all(&directory).unwrap();

        let ok = real_powershell_prompt("$null = $true", &directory);
        assert!(
            ok.starts_with("\u{1b}]133;D;0\u{7}\u{1b}]133;A\u{7}\u{1b}]9;9;\""),
            "{ok:?}"
        );
        assert!(ok.ends_with("\u{1b}]133;B\u{7}"), "{ok:?}");
        let reported = ok
            .split("\u{1b}]9;9;\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("a directory report");
        assert!(
            reported.to_lowercase().ends_with("nekominal model 你好"),
            "{reported}"
        );
        assert!(
            ok.contains("PS "),
            "the user's own prompt text is kept: {ok:?}"
        );

        let failed = real_powershell_prompt("& cmd.exe /c exit 3", &directory);
        assert!(failed.starts_with("\u{1b}]133;D;3\u{7}"), "{failed:?}");

        let cmdlet_failed = real_powershell_prompt(
            "Get-Item 'no such file' -ErrorAction SilentlyContinue",
            &directory,
        );
        assert!(
            cmdlet_failed.starts_with("\u{1b}]133;D;1\u{7}"),
            "{cmdlet_failed:?}"
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// Needs PowerShell 7 on PATH; run with `cargo test -p nekominal-model -- --ignored`.
    #[test]
    #[ignore = "starts pwsh"]
    fn real_powershell_claude_takes_the_settings_and_only_the_hooks_where_the_project_has_its_own_status_line() {
        let root = std::env::temp_dir().join(format!("nekominal-claude-fn-{}", std::process::id()));
        let (bin, plain, own) = (root.join("bin"), root.join("plain"), root.join("own"));
        for dir in [&bin, &plain, &own.join(".claude")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        // A stand-in for claude.exe that says what it was given.
        std::fs::write(bin.join("claude.cmd"), "@echo ARGS %*\r\n").unwrap();
        std::fs::write(own.join(".claude").join("settings.local.json"), r#"{ "statusLine": { "type": "command", "command": "x" } }"#).unwrap();
        let run = |dir: &std::path::Path| {
            let probe = format!("{POWERSHELL_SCRIPT}\nSet-Location -LiteralPath '{}'\nclaude --resume", dir.display());
            let output = Command::new(find_program(&["pwsh.exe"]).expect("pwsh is on PATH"))
                .args(["-NoLogo", "-NoProfile", "-EncodedCommand", &encode_command(&probe)])
                .env("PATH", format!("{};{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
                .env("NEKOMINAL_CLAUDE_SETTINGS", "C:\\n\\claude-settings.json")
                .output()
                .expect("pwsh starts");
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        assert_eq!(run(&plain), "ARGS --settings C:\\n\\claude-settings.json --resume");
        assert_eq!(run(&own), "ARGS --settings C:\\n\\claude-hooks.json --resume", "the project's own status line stays, with our hooks");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Needs PowerShell 7 on PATH; run with `cargo test -p nekominal-model -- --ignored`.
    #[test]
    #[ignore = "starts pwsh"]
    fn real_powershell_codex_gemini_and_aider_take_the_hook_unless_the_user_set_one() {
        let root = std::env::temp_dir().join(format!("nekominal-hook-fn-{}", std::process::id()));
        let (bin, home, own) = (root.join("bin"), root.join("home"), root.join("own"));
        for dir in [&bin, &home, &own.join(".codex")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        // Stand-ins that say what they were given; gemini says the variable it was run with.
        for name in ["codex", "aider"] {
            std::fs::write(bin.join(format!("{name}.cmd")), "@echo ARGS %*\r\n").unwrap();
        }
        std::fs::write(bin.join("gemini.cmd"), "@echo DEFAULTS %GEMINI_CLI_SYSTEM_DEFAULTS_PATH%\r\n").unwrap();
        std::fs::write(own.join(".codex").join("config.toml"), "model = \"o3\"\nnotify = [\"mine\"]\n").unwrap();
        std::fs::write(own.join(".aider.conf.yml"), "notifications-command: mine\n").unwrap();
        let run = |dir: &std::path::Path, command: &str| {
            let probe = format!("{POWERSHELL_SCRIPT}\nSet-Location -LiteralPath '{}'\n{command}\n$env:GEMINI_CLI_SYSTEM_DEFAULTS_PATH", dir.display());
            let output = Command::new(find_program(&["pwsh.exe"]).expect("pwsh is on PATH"))
                .args(["-NoLogo", "-NoProfile", "-EncodedCommand", &encode_command(&probe)])
                .env("PATH", format!("{};{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
                .env("USERPROFILE", &home)
                .env("CODEX_HOME", home.join(".codex"))
                .env_remove("GEMINI_CLI_SYSTEM_DEFAULTS_PATH")
                .env_remove("AIDER_NOTIFICATIONS_COMMAND")
                .env("NEKOMINAL_CODEX_NOTIFY", "notify=['C:\\n\\n.exe','--agent-hook','codex']")
                .env("NEKOMINAL_GEMINI_DEFAULTS", "C:\\n\\gemini-defaults.json")
                .env("NEKOMINAL_AIDER_NOTIFY", "C:\\n\\n.exe --agent-hook aider")
                .output()
                .expect("pwsh starts");
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        assert_eq!(run(&home, "codex resume"), "ARGS -c notify=['C:\\n\\n.exe','--agent-hook','codex'] resume");
        assert_eq!(run(&own, "codex resume"), "ARGS resume", "the project's own notify stays");
        assert_eq!(run(&home, "aider --yes"), "ARGS --notifications --notifications-command \"C:\\n\\n.exe --agent-hook aider\" --yes");
        assert_eq!(run(&own, "aider --yes"), "ARGS --yes", "the user's own notifications command stays");
        assert_eq!(run(&home, "gemini"), "DEFAULTS C:\\n\\gemini-defaults.json", "for the run only: the variable is gone after it");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
