//! Shell discovery and the environment contract of a session. What is found is the platform's: PowerShell, cmd, Git Bash,
//! WSL distributions, Visual Studio developer shells and `~/.ssh/config` hosts on Windows; the login shell, the shells
//! of `/etc/shells` and the same ssh hosts on Unix.

use std::{collections::BTreeMap, path::{Path, PathBuf}};

use crate::conpty::{SpawnConfig, WidthModel};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

pub const KIND_POWERSHELL: &str = "powershell";
pub const KIND_CMD: &str = "cmd";
pub const KIND_BASH: &str = "bash";
pub const KIND_WSL: &str = "wsl";
pub const KIND_SSH: &str = "ssh";

/// Every value `Profile::icon` can take, for the UI to map to a glyph (and to test that it does).
pub const ICON_KEYS: [&str; 7] = [
    "pwsh",
    "windows-powershell",
    "cmd",
    "git-bash",
    "wsl",
    "visual-studio",
    "ssh",
];

/// A shell the user can open. `id` is unique and stable across runs, `kind` is one of the
/// `KIND_*` values and `icon` one of [`ICON_KEYS`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub icon: &'static str,
    pub program: String,
    pub args: Vec<String>,
}

impl Profile {
    /// Launch parameters for a new session of this profile. A working directory that is not an
    /// existing directory is dropped (the spawn then falls back to the home directory; WSL to its
    /// own home). WSL starts in the translated directory.
    pub fn spawn_config(&self, cwd: Option<&Path>, cols: u16, rows: u16) -> SpawnConfig {
        let cwd = cwd.filter(|dir| dir.is_dir());
        #[cfg(windows)]
        let args = if self.kind == KIND_WSL { windows::wsl_args_for_cwd(&self.args, cwd) } else { self.args.clone() };
        #[cfg(not(windows))]
        let args = self.args.clone();
        SpawnConfig {
            program: PathBuf::from(&self.program),
            args,
            cwd: cwd.map(Path::to_path_buf),
            env: default_environment(),
            cols,
            rows,
            width_model: WidthModel::default(),
            inherit_environment: true,
        }
    }
}

fn profile(
    id: &str,
    name: &str,
    kind: &'static str,
    icon: &'static str,
    program: &Path,
    args: &[&str],
) -> Profile {
    Profile {
        id: id.to_owned(),
        name: name.to_owned(),
        kind,
        icon,
        program: program.to_string_lossy().into_owned(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
    }
}

/// Variables every session gets so programs recognise the terminal. Callers merge profile
/// overrides on top.
pub fn default_environment() -> BTreeMap<String, String> {
    [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "Nekominal"),
        ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
    ]
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .into()
}

/// Variables that make coding-agent CLIs use the terminal's features in full: OSC 8 hyperlinks
/// where their detection fails (tmux, ssh) and the real cursor at the input caret, which IME
/// composition needs. Applied on top of [`default_environment`] when enabled in the settings.
pub fn agent_environment() -> BTreeMap<String, String> {
    [("FORCE_HYPERLINK", "1"), ("CLAUDE_CODE_NATIVE_CURSOR", "1")]
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .into()
}

/// Concrete (non-wildcard) `Host` entries of an OpenSSH client config.
pub fn parse_ssh_hosts(config: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let Some(rest) = line
            .get(..4)
            .filter(|keyword| keyword.eq_ignore_ascii_case("host"))
            .and_then(|_| line.get(4..))
        else {
            continue;
        };
        // "Host" must be followed by whitespace or '=' so "HostName" is not mistaken for it.
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '=') {
            continue;
        }
        for name in rest
            .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
            .split_whitespace()
        {
            // A leading '-' would be read as an option when the name becomes ssh's argument.
            if !name.contains(['*', '?', '!'])
                && !name.starts_with('-')
                && !hosts.iter().any(|h| h == name)
            {
                hosts.push(name.to_owned());
            }
        }
    }
    hosts
}
