//! Shell discovery (PowerShell, cmd, Git Bash, WSL distributions, Visual Studio developer shells,
//! `~/.ssh/config` hosts) and the environment contract of a session.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    env,
    ffi::{OsStr, OsString},
    fs,
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use crate::conpty::{SpawnConfig, WidthModel};

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

/// Upper bound for everything external that discovery runs (`wsl.exe`, `vswhere.exe`).
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Without this a GUI process flashes a console window for every probe it starts.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Architecture the Visual Studio developer shells target: the one this build runs on.
const DEV_ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "x64"
};

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
        let args = if self.kind == KIND_WSL {
            wsl_args_for_cwd(&self.args, cwd)
        } else {
            self.args.clone()
        };
        SpawnConfig {
            program: PathBuf::from(&self.program),
            args,
            cwd: cwd.map(Path::to_path_buf),
            env: default_environment(),
            cols,
            rows,
            width_model: WidthModel::default(),
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

/// Whether this process inherited `NO_COLOR`. PowerShell 7 then switches
/// `$PSStyle.OutputRendering` to PlainText whatever the value is, which strips every ANSI sequence
/// from its output, and every child inherits the variable. It is never removed from the
/// environment of a session; the settings page shows this instead.
pub fn inherited_no_color() -> bool {
    has_variable(env::vars_os().map(|(name, _)| name), "NO_COLOR")
}

/// `PSModulePath` without the folders of PowerShell 7 (its own `Modules`, those under `Documents\PowerShell` and
/// `Program Files\PowerShell`, the Store package): `None` when nothing else is left. A process PowerShell 7 started carries
/// them, and Windows PowerShell started from that process loads PowerShell 7's `Microsoft.PowerShell.Utility`, which it
/// cannot use: `Get-FileHash` is then missing, and Scoop fails every hash check. PowerShell 7 adds its folders again
/// itself, so taking them out is what `pwsh` does when it starts `powershell.exe`.
pub fn windows_powershell_module_path(value: &str) -> Option<String> {
    let kept: Vec<&str> = value
        .split(';')
        .filter(|entry| {
            let lower = entry.trim().trim_end_matches('\\').to_ascii_lowercase();
            let seven = lower.ends_with("\\powershell\\modules") || lower.contains("\\powershell\\7") || lower.contains("\\microsoft.powershell_");
            !entry.trim().is_empty() && !seven
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join(";"))
}

/// A variable a Claude Code session gives the programs it starts (`CLAUDECODE=1`, its session id, its pipe and token,
/// `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`). Nekominal started from such a session must not pass them on: `claude` in a
/// pane would think it runs inside that session, keep the tab's title, and hold another session's token.
pub fn is_claude_session_variable(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name == "CLAUDECODE" || name == "CLAUDE_PID" || ["CLAUDE_CODE_", "CLAUDE_AGENT_SDK_", "CLAUDE_PREVIEW_"].iter().any(|prefix| name.starts_with(prefix))
}

/// Environment variable names are case-insensitive on Windows.
fn has_variable(names: impl IntoIterator<Item = OsString>, wanted: &str) -> bool {
    names
        .into_iter()
        .any(|name| name.to_string_lossy().eq_ignore_ascii_case(wanted))
}

/// `symlink_metadata` is used because App Execution Aliases (such as the Store build of pwsh)
/// are reparse points that `metadata` refuses to follow.
fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|candidate| exists(candidate))
}

/// Resolves a bare program name through PATH (never the current directory).
pub fn resolve_program(program: &str) -> Option<PathBuf> {
    let direct = Path::new(program);
    if direct.components().count() > 1 || direct.is_absolute() {
        return exists(direct).then(|| direct.to_path_buf());
    }
    if direct.extension().is_some() {
        return find_on_path(program);
    }
    find_on_path(&format!("{program}.exe")).or_else(|| find_on_path(&format!("{program}.com")))
}

fn system32(relative: &str) -> PathBuf {
    let root = env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join(relative)
}

fn under_env_dir(variable: &str, relative: &[&str]) -> Option<PathBuf> {
    let mut path = PathBuf::from(env::var_os(variable)?);
    path.extend(relative);
    Some(path)
}

fn powershell_7() -> Option<PathBuf> {
    // Off PATH: the MSI location, then the Store build's App Execution Alias.
    find_on_path("pwsh.exe").or_else(|| {
        [
            under_env_dir("ProgramFiles", &["PowerShell", "7", "pwsh.exe"]),
            under_env_dir("LOCALAPPDATA", &["Microsoft", "WindowsApps", "pwsh.exe"]),
        ]
        .into_iter()
        .flatten()
        .find(|path| exists(path))
    })
}

/// Git for Windows installs `cmd\git.exe` and puts `cmd` on PATH, which finds installations in
/// any directory.
fn git_roots_from_path(path: &OsStr) -> Vec<PathBuf> {
    env::split_paths(path)
        .filter(|dir| {
            dir.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("cmd"))
                && exists(&dir.join("git.exe"))
        })
        .filter_map(|dir| dir.parent().map(Path::to_path_buf))
        .collect()
}

fn git_roots() -> Vec<PathBuf> {
    let mut roots = env::var_os("PATH")
        .map(|path| git_roots_from_path(&path))
        .unwrap_or_default();
    roots.extend(
        ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(|var| under_env_dir(var, &["Git"])),
    );
    roots.extend(under_env_dir("LOCALAPPDATA", &["Programs", "Git"]));
    // Scoop keeps apps under SCOOP, SCOOP_GLOBAL or the default `~\scoop`.
    for scoop in [
        env::var_os("SCOOP").map(PathBuf::from),
        under_env_dir("USERPROFILE", &["scoop"]),
        env::var_os("SCOOP_GLOBAL").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        roots.push(scoop.join("apps").join("git").join("current"));
    }
    roots
}

fn git_bash_in(roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .map(|root| root.join("bin").join("bash.exe"))
        .find(|bash| exists(bash))
}

/// `wsl.exe -l -q` prints UTF-16LE with NUL padding and CRLF line endings.
pub fn parse_wsl_list(raw: &[u8]) -> Vec<String> {
    let units: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim_matches(|c: char| c == '\u{feff}' || c.is_whitespace()))
        .filter(|name| !name.is_empty() && !name.starts_with("docker-desktop"))
        .map(str::to_owned)
        .collect()
}

/// Linux path of a Windows directory as WSL mounts it: `/mnt/<drive>/...` for a drive path, or
/// the path inside `distro` for a `\\wsl$\<distro>\...` (or `\\wsl.localhost`) share. `None` when
/// WSL cannot reach the location.
pub fn windows_path_to_wsl(path: &Path, distro: Option<&str>) -> Option<String> {
    let text = path.to_str()?;
    // Verbatim prefixes name the same locations: `\\?\C:\x` and `\\?\UNC\host\share`.
    let text: Cow<str> = match text.strip_prefix(r"\\?\UNC\") {
        Some(unc) => Cow::Owned(format!(r"\\{unc}")),
        None => Cow::Borrowed(text.strip_prefix(r"\\?\").unwrap_or(text)),
    };
    let text = text.replace('/', "\\");

    if let Some(share) = text.strip_prefix(r"\\") {
        let mut parts = share.split('\\');
        let host = parts.next()?;
        if !(host.eq_ignore_ascii_case("wsl$") || host.eq_ignore_ascii_case("wsl.localhost")) {
            return None;
        }
        if !parts.next()?.eq_ignore_ascii_case(distro?) {
            return None;
        }
        let rest: Vec<&str> = parts.filter(|part| !part.is_empty()).collect();
        return Some(format!("/{}", rest.join("/")));
    }

    let mut chars = text.chars();
    let drive = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    let rest = &text[2..];
    // `C:dir` is relative to the current directory of drive C and has no fixed Linux path.
    if !rest.is_empty() && !rest.starts_with('\\') {
        return None;
    }
    let mut linux = format!("/mnt/{}", drive.to_ascii_lowercase());
    for part in rest.split('\\').filter(|part| !part.is_empty()) {
        linux.push('/');
        linux.push_str(part);
    }
    Some(linux)
}

/// The WSL profile's arguments with `--cd` pointing at `cwd` when WSL can reach it.
fn wsl_args_for_cwd(args: &[String], cwd: Option<&Path>) -> Vec<String> {
    let mut args = args.to_vec();
    let distro = args
        .iter()
        .position(|arg| arg == "--distribution")
        .and_then(|at| args.get(at + 1))
        .cloned();
    let target = cwd.and_then(|dir| windows_path_to_wsl(dir, distro.as_deref()));
    let slot = args
        .iter()
        .position(|arg| arg == "--cd")
        .and_then(|at| args.get_mut(at + 1));
    if let (Some(target), Some(slot)) = (target, slot) {
        *slot = target;
    }
    args
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

fn ssh_hosts() -> Vec<String> {
    env::var_os("USERPROFILE")
        .map(|home| PathBuf::from(home).join(".ssh").join("config"))
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|config| parse_ssh_hosts(&config))
        .unwrap_or_default()
}

/// An installation reported by `vswhere.exe`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct VsInstall {
    id: String,
    path: PathBuf,
    version: String,
    /// Marketing year ("2022"), or the major version when vswhere reports none.
    release: String,
    /// "Community", "BuildTools", ...
    edition: String,
    prerelease: bool,
}

impl VsInstall {
    fn from_fields(fields: &HashMap<&str, &str>) -> Option<VsInstall> {
        let field = |name: &str| fields.get(name).copied().filter(|value| !value.is_empty());
        let version = field("installationVersion").unwrap_or_default();
        Some(VsInstall {
            id: field("instanceId")?.to_owned(),
            path: PathBuf::from(field("installationPath")?),
            version: version.to_owned(),
            release: field("catalog_productLineVersion")
                .or_else(|| version.split('.').next().filter(|major| !major.is_empty()))
                .unwrap_or("?")
                .to_owned(),
            edition: field("productId")
                .and_then(|product| product.rsplit('.').next())
                .unwrap_or_default()
                .to_owned(),
            prerelease: field("isPrerelease") == Some("1"),
        })
    }

    fn tools(&self, file: &str) -> PathBuf {
        self.path.join("Common7").join("Tools").join(file)
    }
}

fn version_key(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Parses `vswhere -format text`: one `key: value` block per installation, separated by blank
/// lines. Newest version first.
fn parse_vswhere(text: &str) -> Vec<VsInstall> {
    let mut installs = Vec::new();
    let mut fields = HashMap::new();
    for line in text.lines().chain(Some("")) {
        let line = line.trim();
        if line.is_empty() {
            installs.extend(VsInstall::from_fields(&fields));
            fields.clear();
        } else if let Some((key, value)) = line.split_once(':') {
            fields.insert(key.trim(), value.trim());
        }
    }
    installs.sort_by(|a, b| {
        version_key(&b.version)
            .cmp(&version_key(&a.version))
            .then_with(|| a.id.cmp(&b.id))
    });
    installs
}

fn ps_quote(text: &str) -> String {
    text.replace('\'', "''")
}

/// Developer Command Prompt and Developer PowerShell for each installation that has the scripts.
fn visual_studio_profiles(installs: &[VsInstall], cmd: &Path, powershell: &Path) -> Vec<Profile> {
    let mut profiles = Vec::new();
    for install in installs {
        let mut label = format!("VS {}", install.release);
        if install.prerelease {
            label.push_str(" Preview");
        }
        let ambiguous = installs
            .iter()
            .filter(|other| {
                other.release == install.release && other.prerelease == install.prerelease
            })
            .count()
            > 1;
        if ambiguous && !install.edition.is_empty() {
            label.push_str(&format!(" ({})", install.edition));
        }
        let arch = format!("-arch={DEV_ARCH}");
        let host_arch = format!("-host_arch={DEV_ARCH}");

        let dev_cmd = install.tools("VsDevCmd.bat");
        if exists(&dev_cmd) {
            // A command line after /k that starts with a quote goes through cmd's quote-stripping
            // rule, which breaks the script path when it contains `&`; starting with `call` avoids it.
            profiles.push(profile(
                &format!("vs-{}-cmd", install.id),
                &format!("Developer Command Prompt for {label}"),
                KIND_CMD,
                "visual-studio",
                cmd,
                &[
                    "/k",
                    "call",
                    &dev_cmd.to_string_lossy(),
                    "-startdir=none",
                    &arch,
                    &host_arch,
                ],
            ));
        }
        let dev_shell = install.tools("Microsoft.VisualStudio.DevShell.dll");
        if exists(&dev_shell) {
            let script = format!(
                "&{{Import-Module '{}'; Enter-VsDevShell {} -SkipAutomaticLocation -DevCmdArguments '{arch} {host_arch}'}}",
                ps_quote(&dev_shell.to_string_lossy()),
                install.id,
            );
            profiles.push(profile(
                &format!("vs-{}-powershell", install.id),
                &format!("Developer PowerShell for {label}"),
                KIND_POWERSHELL,
                "visual-studio",
                powershell,
                &["-NoLogo", "-NoExit", "-Command", &script],
            ));
        }
    }
    profiles
}

fn vswhere() -> Option<PathBuf> {
    under_env_dir(
        "ProgramFiles(x86)",
        &["Microsoft Visual Studio", "Installer", "vswhere.exe"],
    )
    .filter(|path| exists(path))
}

/// A child process whose output is read on its own thread so that waiting for it can time out.
struct Probe {
    child: Child,
    output: Receiver<Vec<u8>>,
}

impl Probe {
    fn start(program: &Path, args: &[&str]) -> Option<Probe> {
        let mut child = Command::new(program)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let (tx, output) = mpsc::channel();
        let reader = thread::Builder::new()
            .name("probe-read".into())
            .spawn(move || {
                let mut bytes = Vec::new();
                // A read error drops the sender, which `finish` reports as a failed probe.
                if stdout.read_to_end(&mut bytes).is_ok() {
                    let _ = tx.send(bytes);
                }
            });
        if reader.is_err() {
            // The child may already have exited, which makes both calls fail harmlessly.
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        Some(Probe { child, output })
    }

    /// The output of a probe that exited successfully before `deadline`. A slow or failing probe
    /// is killed and yields `None`.
    fn finish(mut self, deadline: Instant) -> Option<Vec<u8>> {
        let output = self
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()
            .filter(|_| self.exited_successfully(deadline));
        if output.is_none() {
            // Fails only when the child has already exited.
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        output
    }

    fn exited_successfully(&mut self, deadline: Instant) -> bool {
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                _ => return false,
            }
        }
    }
}

/// Finds the shells installed on this machine. The order is fixed (PowerShell, Windows
/// PowerShell, cmd, Git Bash, WSL distributions in `wsl.exe`'s order, Visual Studio developer
/// shells newest first, SSH hosts in file order) and the call returns within [`PROBE_TIMEOUT`]
/// plus file system lookups: `wsl.exe` and `vswhere.exe` run concurrently and are killed at the
/// deadline, which drops their profiles. A `wsl.exe` that fails (WSL not installed prints a
/// message and exits with an error) contributes nothing. Blocking, so call it off the UI thread.
pub fn discover() -> Vec<Profile> {
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let wsl = system32("wsl.exe");
    let wsl_probe = exists(&wsl)
        .then(|| Probe::start(&wsl, &["-l", "-q"]))
        .flatten();
    let vswhere_probe = vswhere().and_then(|path| {
        Probe::start(
            &path,
            &[
                "-products",
                "*",
                "-prerelease",
                "-nologo",
                "-utf8",
                "-format",
                "text",
            ],
        )
    });

    let mut profiles = Vec::new();

    if let Some(pwsh) = powershell_7() {
        profiles.push(profile(
            "pwsh",
            "PowerShell",
            KIND_POWERSHELL,
            "pwsh",
            &pwsh,
            &["-NoLogo"],
        ));
    }

    let windows_powershell = system32("WindowsPowerShell\\v1.0\\powershell.exe");
    let has_windows_powershell = exists(&windows_powershell);
    if has_windows_powershell {
        profiles.push(profile(
            "windows-powershell",
            "Windows PowerShell",
            KIND_POWERSHELL,
            "windows-powershell",
            &windows_powershell,
            &["-NoLogo"],
        ));
    }

    let cmd = env::var_os("ComSpec")
        .map(PathBuf::from)
        .unwrap_or_else(|| system32("cmd.exe"));
    let has_cmd = exists(&cmd);
    if has_cmd {
        profiles.push(profile("cmd", "命令提示字元", KIND_CMD, "cmd", &cmd, &[]));
    }

    if let Some(bash) = git_bash_in(&git_roots()) {
        profiles.push(profile(
            "git-bash",
            "Git Bash",
            KIND_BASH,
            "git-bash",
            &bash,
            &["--login", "-i"],
        ));
    }

    for distro in wsl_probe
        .and_then(|probe| probe.finish(deadline))
        .map(|raw| parse_wsl_list(&raw))
        .unwrap_or_default()
    {
        profiles.push(profile(
            &format!("wsl-{distro}"),
            &distro,
            KIND_WSL,
            "wsl",
            &wsl,
            &["--distribution", &distro, "--cd", "~"],
        ));
    }

    if has_cmd && has_windows_powershell {
        let installs = vswhere_probe
            .and_then(|probe| probe.finish(deadline))
            .map(|raw| parse_vswhere(&String::from_utf8_lossy(&raw)))
            .unwrap_or_default();
        profiles.extend(visual_studio_profiles(&installs, &cmd, &windows_powershell));
    }

    let ssh = system32("OpenSSH\\ssh.exe");
    if exists(&ssh) {
        for host in ssh_hosts() {
            profiles.push(profile(
                &format!("ssh-{host}"),
                &host,
                KIND_SSH,
                "ssh",
                &ssh,
                &[&host],
            ));
        }
    }

    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tree(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("nekominal-pty-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn decodes_utf16_wsl_output_and_skips_docker_distributions() {
        let text = "Ubuntu-24.04\r\ndocker-desktop\r\nDebian\r\n";
        let mut raw = vec![0xFF, 0xFE];
        raw.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(parse_wsl_list(&raw), vec!["Ubuntu-24.04", "Debian"]);
    }

    #[test]
    fn wsl_listing_tolerates_empty_output_and_a_dangling_byte() {
        assert!(parse_wsl_list(&[]).is_empty());
        assert!(parse_wsl_list(&[0xFF, 0xFE]).is_empty());
        let mut raw: Vec<u8> = "Debian\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        raw.push(0x41);
        assert_eq!(parse_wsl_list(&raw), vec!["Debian"]);
    }

    #[test]
    fn collects_concrete_ssh_hosts_only() {
        let config = "\
Host *
  ServerAliveInterval 60
host build-box  staging
  HostName 10.0.0.2
Host=lab
Host *.internal !secret dev-?
Host build-box
";
        assert_eq!(parse_ssh_hosts(config), vec!["build-box", "staging", "lab"]);
    }

    #[test]
    fn ssh_host_names_cannot_become_options() {
        assert_eq!(parse_ssh_hosts("Host -oProxyCommand=calc ok"), vec!["ok"]);
    }

    #[test]
    fn resolves_common_system_programs() {
        assert!(resolve_program("cmd").is_some());
        assert!(resolve_program("cmd.exe").is_some());
        assert!(resolve_program("definitely-not-a-program-nekominal").is_none());
    }

    const VSWHERE_SAMPLE: &str = "\
instanceId: 9c86f937
installationName: VisualStudio/17.14.42+37806.7
installationPath: C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools
installationVersion: 17.14.37806.7
productId: Microsoft.VisualStudio.Product.BuildTools
isPrerelease: 0
displayName: Visual Studio Build Tools 2022
catalog_productLineVersion: 2022
properties_campaignId:

instanceId: a1b2c3d4
installationPath: D:\\VS\\Preview
installationVersion: 18.0.1.2
productId: Microsoft.VisualStudio.Product.Community
isPrerelease: 1
";

    #[test]
    fn parses_vswhere_blocks_newest_first() {
        let installs = parse_vswhere(VSWHERE_SAMPLE);
        assert_eq!(installs.len(), 2);
        assert_eq!(installs[0].id, "a1b2c3d4");
        assert_eq!(installs[0].release, "18", "falls back to the major version");
        assert_eq!(installs[0].edition, "Community");
        assert!(installs[0].prerelease);
        assert_eq!(installs[1].id, "9c86f937");
        assert_eq!(installs[1].release, "2022");
        assert_eq!(installs[1].edition, "BuildTools");
        assert_eq!(
            installs[1].path,
            PathBuf::from("C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools")
        );
        assert!(!installs[1].prerelease);
    }

    #[test]
    fn vswhere_blocks_without_an_id_or_path_are_ignored() {
        assert!(parse_vswhere("").is_empty());
        assert!(parse_vswhere("installationPath: C:\\x\n\ninstanceId: only-id\n").is_empty());
        assert_eq!(
            parse_vswhere("instanceId: a\ninstallationPath: C:\\x").len(),
            1,
            "no trailing blank line"
        );
    }

    fn fake_install(
        root: &Path,
        id: &str,
        release: &str,
        edition: &str,
        prerelease: bool,
    ) -> VsInstall {
        let path = root.join(format!("{release}-{edition}"));
        touch(&path.join("Common7").join("Tools").join("VsDevCmd.bat"));
        touch(
            &path
                .join("Common7")
                .join("Tools")
                .join("Microsoft.VisualStudio.DevShell.dll"),
        );
        VsInstall {
            id: id.into(),
            path,
            version: format!("{release}.0"),
            release: release.into(),
            edition: edition.into(),
            prerelease,
        }
    }

    #[test]
    fn developer_shell_profiles_launch_the_installations_scripts() {
        let root = temp_tree("vs-profiles");
        let install = fake_install(&root, "abc123", "2022", "BuildTools", false);
        let profiles = visual_studio_profiles(
            std::slice::from_ref(&install),
            Path::new("C:\\cmd.exe"),
            Path::new("C:\\ps.exe"),
        );
        assert_eq!(profiles.len(), 2);

        let (dev_cmd, dev_ps) = (&profiles[0], &profiles[1]);
        assert_eq!(dev_cmd.id, "vs-abc123-cmd");
        assert_eq!(dev_cmd.name, "Developer Command Prompt for VS 2022");
        assert_eq!((dev_cmd.kind, dev_cmd.icon), (KIND_CMD, "visual-studio"));
        assert_eq!(dev_cmd.program, "C:\\cmd.exe");
        let bat = install.tools("VsDevCmd.bat").to_string_lossy().into_owned();
        assert_eq!(
            dev_cmd.args,
            [
                "/k",
                "call",
                &bat,
                "-startdir=none",
                &format!("-arch={DEV_ARCH}"),
                &format!("-host_arch={DEV_ARCH}")
            ]
        );

        assert_eq!(dev_ps.id, "vs-abc123-powershell");
        assert_eq!(dev_ps.name, "Developer PowerShell for VS 2022");
        assert_eq!(
            (dev_ps.kind, dev_ps.icon),
            (KIND_POWERSHELL, "visual-studio")
        );
        assert_eq!(&dev_ps.args[..3], ["-NoLogo", "-NoExit", "-Command"]);
        let script = &dev_ps.args[3];
        assert!(
            script.contains("Enter-VsDevShell abc123 -SkipAutomaticLocation"),
            "{script}"
        );
        assert!(
            script.contains("Microsoft.VisualStudio.DevShell.dll"),
            "{script}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn developer_shell_names_tell_installations_of_one_release_apart() {
        let root = temp_tree("vs-names");
        let installs = [
            fake_install(&root, "a", "2022", "Community", false),
            fake_install(&root, "b", "2022", "BuildTools", false),
            fake_install(&root, "c", "2022", "Community", true),
        ];
        let names: Vec<String> = visual_studio_profiles(&installs, Path::new("c"), Path::new("p"))
            .into_iter()
            .filter(|profile| profile.kind == KIND_CMD)
            .map(|profile| profile.name)
            .collect();
        assert_eq!(
            names,
            [
                "Developer Command Prompt for VS 2022 (Community)",
                "Developer Command Prompt for VS 2022 (BuildTools)",
                "Developer Command Prompt for VS 2022 Preview",
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn developer_shells_are_skipped_when_their_scripts_are_missing() {
        let root = temp_tree("vs-missing");
        let mut install = fake_install(&root, "x", "2022", "Community", false);
        fs::remove_file(install.tools("VsDevCmd.bat")).unwrap();
        let profiles = visual_studio_profiles(&[install.clone()], Path::new("c"), Path::new("p"));
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "vs-x-powershell");
        install.path = root.join("nowhere");
        assert!(visual_studio_profiles(&[install], Path::new("c"), Path::new("p")).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn powershell_arguments_escape_single_quotes() {
        assert_eq!(ps_quote("C:\\John's VS\\x.dll"), "C:\\John''s VS\\x.dll");
    }

    #[test]
    fn translates_windows_paths_for_wsl() {
        let wsl = |path: &str, distro| windows_path_to_wsl(Path::new(path), distro);
        assert_eq!(
            wsl("C:\\Users\\me\\my project", None).as_deref(),
            Some("/mnt/c/Users/me/my project")
        );
        assert_eq!(
            wsl("D:/work/repo", None).as_deref(),
            Some("/mnt/d/work/repo")
        );
        assert_eq!(wsl("c:\\", None).as_deref(), Some("/mnt/c"));
        assert_eq!(wsl("C:", None).as_deref(), Some("/mnt/c"));
        assert_eq!(
            wsl("\\\\?\\C:\\Users\\me", None).as_deref(),
            Some("/mnt/c/Users/me")
        );
        assert_eq!(
            wsl("C:relative", None),
            None,
            "drive-relative paths have no Linux path"
        );
        assert_eq!(wsl("relative\\dir", None), None);
        assert_eq!(wsl("\\\\server\\share\\dir", Some("Ubuntu")), None);
        assert_eq!(wsl("\\\\?\\UNC\\server\\share", Some("Ubuntu")), None);
    }

    #[test]
    fn translates_wsl_shares_of_the_profiles_own_distribution_only() {
        let wsl = |path: &str, distro| windows_path_to_wsl(Path::new(path), distro);
        assert_eq!(
            wsl("\\\\wsl$\\Ubuntu\\home\\me", Some("Ubuntu")).as_deref(),
            Some("/home/me")
        );
        assert_eq!(
            wsl("\\\\wsl.localhost\\ubuntu\\home\\me\\", Some("Ubuntu")).as_deref(),
            Some("/home/me")
        );
        assert_eq!(
            wsl("\\\\wsl$\\Ubuntu", Some("Ubuntu")).as_deref(),
            Some("/")
        );
        assert_eq!(wsl("\\\\wsl$\\Debian\\home\\me", Some("Ubuntu")), None);
        assert_eq!(wsl("\\\\wsl$\\Ubuntu\\home\\me", None), None);
    }

    fn wsl_profile() -> Profile {
        profile(
            "wsl-Ubuntu",
            "Ubuntu",
            KIND_WSL,
            "wsl",
            Path::new("C:\\Windows\\System32\\wsl.exe"),
            &["--distribution", "Ubuntu", "--cd", "~"],
        )
    }

    #[test]
    fn wsl_starts_in_the_translated_working_directory() {
        let dir = env::temp_dir();
        let config = wsl_profile().spawn_config(Some(&dir), 80, 24);
        let expected = windows_path_to_wsl(&dir, Some("Ubuntu")).unwrap();
        assert_eq!(config.args, ["--distribution", "Ubuntu", "--cd", &expected]);
        assert_eq!(config.cwd.as_deref(), Some(dir.as_path()));
    }

    #[test]
    fn wsl_keeps_its_home_when_the_directory_is_missing_or_unreachable() {
        let profile = wsl_profile();
        let missing = env::temp_dir().join("nekominal-pty-no-such-directory");
        for cwd in [None, Some(missing.as_path())] {
            let config = profile.spawn_config(cwd, 80, 24);
            assert_eq!(config.args, profile.args);
            assert_eq!(config.cwd, None);
        }
        // An existing directory WSL cannot reach (a UNC share would be one) keeps `--cd ~` too.
        assert_eq!(
            wsl_args_for_cwd(&profile.args, Some(Path::new("\\\\server\\share"))),
            profile.args
        );
    }

    #[test]
    fn other_profiles_pass_their_arguments_and_directory_through() {
        let pwsh = profile(
            "pwsh",
            "PowerShell",
            KIND_POWERSHELL,
            "pwsh",
            Path::new("C:\\pwsh.exe"),
            &["-NoLogo"],
        );
        let dir = env::temp_dir();
        let config = pwsh.spawn_config(Some(&dir), 120, 30);
        assert_eq!(config.program, PathBuf::from("C:\\pwsh.exe"));
        assert_eq!(config.args, ["-NoLogo"]);
        assert_eq!(config.cwd.as_deref(), Some(dir.as_path()));
        assert_eq!((config.cols, config.rows), (120, 30));
        assert_eq!(config.width_model, WidthModel::Graphemes);
        assert_eq!(config.env, default_environment());
    }

    #[test]
    fn the_module_path_of_powershell_7_is_taken_out_for_windows_powershell() {
        let from_pwsh = r"C:\Users\me\Documents\PowerShell\Modules;C:\Program Files\PowerShell\Modules;c:\program files\windowsapps\microsoft.powershell_7.6.8.0_x64__8wekyb3d8bbwe\Modules;C:\Program Files\PowerShell\7\Modules;C:\Program Files\WindowsPowerShell\Modules;C:\WINDOWS\system32\WindowsPowerShell\v1.0\Modules;D:\mine;D:\PowerShell\Scripts";
        assert_eq!(
            windows_powershell_module_path(from_pwsh).as_deref(),
            Some(r"C:\Program Files\WindowsPowerShell\Modules;C:\WINDOWS\system32\WindowsPowerShell\v1.0\Modules;D:\mine;D:\PowerShell\Scripts")
        );
        assert_eq!(windows_powershell_module_path(r"C:\Program Files\PowerShell\7\Modules;"), None, "nothing left: no variable, the defaults");
    }

    #[test]
    fn the_variables_of_a_claude_code_session_are_told_apart_from_the_users_own() {
        for name in ["CLAUDECODE", "CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "claude_code_session_id", "CLAUDE_AGENT_SDK_VERSION", "CLAUDE_PID"] {
            assert!(is_claude_session_variable(name), "{name}");
        }
        for name in ["CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY", "PATH", "CLAUDECODEX"] {
            assert!(!is_claude_session_variable(name), "{name}");
        }
    }

    #[test]
    fn the_default_environment_names_the_terminal() {
        let defaults = default_environment();
        assert_eq!(defaults["TERM_PROGRAM"], "Nekominal");
        assert_eq!(defaults["TERM_PROGRAM_VERSION"], env!("CARGO_PKG_VERSION"));
        assert_eq!(defaults["COLORTERM"], "truecolor");
        assert_eq!(defaults["TERM"], "xterm-256color");
        assert!(
            !defaults.contains_key("NO_COLOR"),
            "NO_COLOR is never set here"
        );
    }

    #[test]
    fn the_agent_environment_is_separate_from_the_default_one() {
        let agent = agent_environment();
        assert_eq!(agent["FORCE_HYPERLINK"], "1");
        assert_eq!(agent["CLAUDE_CODE_NATIVE_CURSOR"], "1");
        assert!(
            default_environment()
                .keys()
                .all(|name| !agent.contains_key(name))
        );
    }

    #[test]
    fn no_color_is_found_whatever_its_case_or_value() {
        let names = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(has_variable(names(&["PATH", "NO_COLOR"]), "NO_COLOR"));
        assert!(has_variable(names(&["no_color"]), "NO_COLOR"));
        assert!(!has_variable(
            names(&["PATH", "NO_COLORS", "COLOR"]),
            "NO_COLOR"
        ));
        assert!(!has_variable(names(&[]), "NO_COLOR"));
        // The real environment answers consistently with a direct lookup.
        assert_eq!(inherited_no_color(), env::var_os("NO_COLOR").is_some());
    }

    #[test]
    fn finds_git_bash_through_the_git_cmd_directory_on_path() {
        let root = temp_tree("git");
        let git = root.join("Tools").join("Git");
        touch(&git.join("cmd").join("git.exe"));
        touch(&git.join("bin").join("bash.exe"));
        let other = root.join("bin");
        touch(&other.join("git.exe"));

        let path = env::join_paths([other, git.join("cmd")]).unwrap();
        let roots = git_roots_from_path(&path);
        assert_eq!(
            roots,
            vec![git.clone()],
            "only a directory named cmd that holds git.exe counts"
        );
        assert_eq!(git_bash_in(&roots), Some(git.join("bin").join("bash.exe")));
        assert_eq!(git_bash_in(&[root.join("missing")]), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_probe_returns_the_output_of_a_command_that_succeeds() {
        let cmd = system32("cmd.exe");
        let probe = Probe::start(&cmd, &["/d", "/c", "echo", "probe-ok"]).unwrap();
        let output = probe
            .finish(Instant::now() + Duration::from_secs(10))
            .unwrap();
        assert!(String::from_utf8_lossy(&output).contains("probe-ok"));
    }

    #[test]
    fn a_probe_that_exits_with_an_error_yields_nothing() {
        let probe = Probe::start(
            &system32("cmd.exe"),
            &["/d", "/c", "echo not-installed & exit 1"],
        )
        .unwrap();
        assert_eq!(probe.finish(Instant::now() + Duration::from_secs(10)), None);
    }

    #[test]
    fn a_probe_that_outlives_the_deadline_is_killed() {
        let probe = Probe::start(&system32("ping.exe"), &["-n", "30", "127.0.0.1"]).unwrap();
        let started = Instant::now();
        assert_eq!(probe.finish(started + Duration::from_millis(300)), None);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "waited {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn discovery_is_deterministic_bounded_and_well_formed() {
        let started = Instant::now();
        let first = discover();
        let elapsed = started.elapsed();
        let ids: Vec<&str> = first.iter().map(|profile| profile.id.as_str()).collect();
        println!("discovered {ids:?} in {elapsed:?}");
        assert!(
            elapsed < PROBE_TIMEOUT + Duration::from_millis(500),
            "discovery took {elapsed:?}"
        );

        let second = discover();
        assert_eq!(first, second);

        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "ids must be unique: {ids:?}");

        for profile in &first {
            assert!(!profile.name.is_empty(), "{profile:?}");
            assert!(ICON_KEYS.contains(&profile.icon), "{profile:?}");
            assert!(
                [KIND_POWERSHELL, KIND_CMD, KIND_BASH, KIND_WSL, KIND_SSH].contains(&profile.kind),
                "{profile:?}"
            );
            assert!(exists(Path::new(&profile.program)), "{profile:?}");
        }
        assert!(
            ids.contains(&"cmd") && ids.contains(&"windows-powershell"),
            "{ids:?}"
        );
    }
}
