//! Shell integration: scripts that make a shell report its working directory and where each prompt, command
//! and exit status begins, as OSC 133 marks (`A` prompt start, `B` command start, `D;<code>` command end)
//! plus the directory (`OSC 9;9` for PowerShell, `OSC 7` for the Unix shells). The emulator reads them to
//! jump between prompts and to open new tabs in the current directory.
//!
//! The PowerShell script also gives Claude Code the status line of the window: when `NEKOMINAL_CLAUDE_SETTINGS` names a
//! settings file (the window sets it while its status bar is on), `claude` runs `claude.exe --settings <that file>`,
//! unless the user has a `claude` function of their own.
//!
//! Only PowerShell is wired into profiles ([`powershell_args`]). The Unix scripts are ready to be sourced
//! by whatever starts those shells (an rc file, `--rcfile`, `ZDOTDIR`); they must run after the user's own
//! prompt is set because they wrap it.

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
}
if ($env:NEKOMINAL_CLAUDE_SETTINGS -and -not (Test-Path function:global:claude)) {
  function global:claude {
    $exe = (Get-Command claude -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if (-not $exe -and (Test-Path "$HOME\.local\bin\claude.exe")) { $exe = "$HOME\.local\bin\claude.exe" }
    if ($exe) { & $exe --settings $env:NEKOMINAL_CLAUDE_SETTINGS @args } else { Write-Error 'claude.exe was not found' }
  }
}"#;

/// Bash: the directory goes out as an OSC 7 `file://` URI, percent-encoded byte by byte (the technique
/// of Apple's Terminal integration, so multi-byte characters in a path survive).
pub const BASH_SCRIPT: &str = r#"if [[ -z ${__nekominal_integration-} ]]; then
  __nekominal_integration=1
  __nekominal_precmd() {
    local exit_code=$? i char hex url_path=
    local LC_CTYPE=C LC_ALL=
    for ((i = 0; i < ${#PWD}; i++)); do
      char=${PWD:i:1}
      case $char in
        [a-zA-Z0-9/._~-]) url_path+=$char ;;
        *) printf -v hex '%02X' "'$char"; url_path+=%${hex: -2:2} ;;
      esac
    done
    printf '\e]133;D;%s\a\e]7;file://%s%s\a' "$exit_code" "${HOSTNAME-}" "$url_path"
  }
  PROMPT_COMMAND="__nekominal_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
fi"#;

/// Zsh: as Bash, through `precmd_functions`; the marks sit in `%{ %}` so they take no prompt width.
pub const ZSH_SCRIPT: &str = r#"if [[ -z ${__nekominal_integration-} ]]; then
  typeset -g __nekominal_integration=1
  __nekominal_precmd() {
    local exit_code=$? i char hex url_path=
    local LC_CTYPE=C LC_ALL=
    for ((i = 1; i <= ${#PWD}; i++)); do
      char=${PWD[i]}
      case $char in
        [a-zA-Z0-9/._~-]) url_path+=$char ;;
        *) printf -v hex '%02X' "'$char"; url_path+=%${hex: -2:2} ;;
      esac
    done
    printf '\e]133;D;%s\a\e]7;file://%s%s\a' "$exit_code" "${HOST-}" "$url_path"
  }
  precmd_functions=(__nekominal_precmd $precmd_functions)
  PS1=$'%{\e]133;A\a%}'"$PS1"$'%{\e]133;B\a%}'
fi"#;

/// Fish: wraps `fish_prompt`.
pub const FISH_SCRIPT: &str = r#"if not set -q __nekominal_integration
    set -g __nekominal_integration 1
    functions --copy fish_prompt __nekominal_prompt
    function fish_prompt
        set -l exit_code $status
        printf '\e]133;D;%s\a\e]7;file://%s%s\a\e]133;A\a' $exit_code $hostname (string escape --style=url -- $PWD)
        __nekominal_prompt
        printf '\e]133;B\a'
    end
end"#;

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

    /// Every OSC introducer of a Unix script, up to its terminator: the text between `\e]` and `\a`.
    fn unix_osc_payloads(script: &str) -> Vec<&str> {
        script
            .split("\\e]")
            .skip(1)
            .map(|rest| rest.split("\\a").next().expect("split yields one item"))
            .collect()
    }

    /// What is wrong with the sequences a Unix script emits; empty when it emits exactly the intended four.
    fn unexpected_sequences(script: &str) -> Vec<String> {
        let allowed = ["133;A", "133;B", "133;D;%s", "7;file://%s%s"];
        let payloads = unix_osc_payloads(script);
        let mut found = Vec::new();
        for payload in &payloads {
            if !allowed.contains(payload) {
                found.push(format!("unexpected OSC {payload:?}"));
            }
        }
        for wanted in allowed {
            if !payloads.contains(&wanted) {
                found.push(format!("missing OSC {wanted:?}"));
            }
        }
        for forbidden in ["\\e[", "\\033", "\\x1b", "\\E[", "\\E]"] {
            if script.contains(forbidden) {
                found.push(format!("{forbidden} is not an integration sequence"));
            }
        }
        found
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
    fn the_unix_scripts_emit_only_osc_133_marks_and_the_directory() {
        for (name, script) in [
            ("bash", BASH_SCRIPT),
            ("zsh", ZSH_SCRIPT),
            ("fish", FISH_SCRIPT),
        ] {
            assert_eq!(unexpected_sequences(script), Vec::<String>::new(), "{name}");
            assert_eq!(unix_osc_payloads(script).len(), 4, "{name}");
            assert!(
                script.contains("__nekominal_integration"),
                "{name}: sourcing it twice must change nothing"
            );
        }
    }

    #[test]
    fn the_scripts_never_contain_a_raw_escape_or_bell_byte() {
        for script in [POWERSHELL_SCRIPT, BASH_SCRIPT, ZSH_SCRIPT, FISH_SCRIPT] {
            assert!(
                !script.chars().any(|c| c == '\u{1b}' || c == '\u{7}'),
                "the sequences are spelled out, not embedded"
            );
        }
    }

    #[test]
    fn the_sequence_check_flags_an_extra_or_missing_sequence() {
        let extra = format!("{BASH_SCRIPT}\nprintf '\\e]0;title\\a'");
        assert_eq!(unexpected_sequences(&extra), ["unexpected OSC \"0;title\""]);
        let colour = format!("{ZSH_SCRIPT}\nprintf '\\e[31m'");
        assert_eq!(
            unexpected_sequences(&colour),
            ["\\e[ is not an integration sequence"]
        );
        let without_marks = BASH_SCRIPT.replace("\\e]133;B\\a", "");
        assert_eq!(
            unexpected_sequences(&without_marks),
            ["missing OSC \"133;B\""]
        );
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

    /// Needs Git Bash on PATH or in the usual places.
    #[test]
    #[ignore = "starts bash"]
    fn real_bash_marks_the_exit_status_and_percent_encodes_the_directory() {
        let bash = find_program(&["bash.exe"])
            .or_else(|| {
                ["C:\\Program Files\\Git\\bin\\bash.exe"]
                    .iter()
                    .map(PathBuf::from)
                    .find(|p| p.exists())
            })
            .expect("bash is installed");
        let directory = std::env::temp_dir().join("nekominal model 你好");
        std::fs::create_dir_all(&directory).unwrap();
        // A non-interactive bash drops PS1 from the environment, so the script sets the user's prompt itself.
        let script = format!(
            "PS1='$ '\n{BASH_SCRIPT}\nfalse\n__nekominal_precmd\nprintf '\\n---\\n%s' \"$PS1\"\n"
        );
        let output = Command::new(bash)
            .args(["--noprofile", "--norc", "-c", &script])
            .current_dir(&directory)
            .output()
            .expect("bash starts");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let (marks, prompt) = text.split_once("\n---\n").expect("both parts are printed");
        assert!(
            marks.starts_with("\u{1b}]133;D;1\u{7}\u{1b}]7;file://"),
            "{marks:?}"
        );
        assert!(
            marks.ends_with("nekominal%20model%20%E4%BD%A0%E5%A5%BD\u{7}"),
            "{marks:?}"
        );
        assert_eq!(prompt, "\\[\\e]133;A\\a\\]$ \\[\\e]133;B\\a\\]");
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
