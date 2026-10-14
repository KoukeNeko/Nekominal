//! The environment of a probed program.
//!
//! The session that runs this harness carries secrets and session plumbing (`CLAUDE_CODE_*` tokens,
//! `ANTHROPIC_*`, `MCP_*`, `OTEL_*`, `NO_COLOR`, ...). `nekominal_pty` builds a child's environment as
//! "this process's environment plus overrides", so the harness cannot hand it an allow-list directly. It
//! therefore swaps this process's environment for the allow-list around the spawn call (which builds the
//! environment block and calls `CreateProcessW` synchronously) and restores it afterwards, under a lock.
//!
//! Nothing here ever prints a value.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    sync::Mutex,
};

/// Variable names (case-insensitive) a probed program may inherit.
pub const ALLOWED_NAMES: &[&str] = &[
    "SystemRoot",
    "windir",
    "ComSpec",
    "PATHEXT",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramData",
    "ProgramW6432",
    "NUMBER_OF_PROCESSORS",
    "OS",
    "COMPUTERNAME",
    "USERNAME",
    "USERDOMAIN",
    "LANG",
];

/// Prefix families that are allowed as well (`CommonProgramFiles*`, `PROCESSOR_*`).
pub const ALLOWED_PREFIXES: &[&str] = &["CommonProgramFiles", "PROCESSOR_"];

/// Name prefixes that must never reach a probed program. The allow-list already excludes them; this list is
/// what the tests assert against, and what `is_forbidden` reports.
pub const FORBIDDEN_PREFIXES: &[&str] = &[
    "CLAUDE",
    "ANTHROPIC",
    "MCP_",
    "OTEL_",
    "NO_COLOR",
    "AWS_",
    "AZURE_",
    "GOOGLE_",
    "GITHUB_",
    "GH_",
    "OPENAI",
    "GEMINI",
    "NPM_TOKEN",
];

/// The variable that stops the CLI from replacing its own executable while it is probed.
pub const DISABLE_AUTOUPDATER: (&str, &str) = ("DISABLE_AUTOUPDATER", "1");

pub fn is_allowed(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ALLOWED_NAMES.iter().any(|allowed| allowed.eq_ignore_ascii_case(name))
        || ALLOWED_PREFIXES.iter().any(|prefix| upper.starts_with(&prefix.to_ascii_uppercase()))
}

pub fn is_forbidden(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    FORBIDDEN_PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
        || upper.ends_with("_TOKEN")
        || upper.ends_with("_SECRET")
        || upper.ends_with("_API_KEY")
        || upper.contains("PASSWORD")
}

/// The allow-listed subset of `vars`, plus `PATH` assembled from `system_path` and `extra_path_dir`.
/// Pure: the caller supplies the variables, so tests never touch the real environment.
pub fn clean_environment_from(
    vars: impl IntoIterator<Item = (String, String)>,
    system_path: &str,
    extra_path_dir: Option<&str>,
) -> BTreeMap<String, String> {
    let mut clean = BTreeMap::new();
    for (name, value) in vars {
        if name.starts_with('=') || name.eq_ignore_ascii_case("PATH") || !is_allowed(&name) {
            continue;
        }
        clean.insert(name, value);
    }
    let mut path = system_path.trim_end_matches(';').to_owned();
    if let Some(dir) = extra_path_dir {
        path.push(';');
        path.push_str(dir);
    }
    clean.insert("PATH".to_owned(), path);
    clean
}

/// The allow-listed environment of this machine: this process's allow-listed variables, the machine-level
/// `PATH` (the user's own additions to the session's `PATH` are not inherited) and the CLI's install directory.
pub fn clean_environment() -> BTreeMap<String, String> {
    let vars =
        std::env::vars_os().filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)));
    let user_bin = std::env::var("USERPROFILE").ok().map(|home| format!("{home}\\.local\\bin"));
    clean_environment_from(vars, &machine_path(), user_bin.as_deref())
}

/// Claude Code behaviour switches a scenario may set although their names start with a forbidden prefix.
/// Everything else under a forbidden prefix is refused as an override, so a session token cannot be passed on
/// by a scenario either.
pub const ALLOWED_OVERRIDES: &[&str] = &[
    "CLAUDE_CODE_NO_FLICKER",
    "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN",
    "CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT",
    "CLAUDE_CODE_DISABLE_MOUSE",
    "CLAUDE_CODE_DISABLE_MOUSE_CLICKS",
    "CLAUDE_CODE_SCROLL_SPEED",
    "CLAUDE_CODE_NATIVE_CURSOR",
    "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
    "CLAUDE_CODE_FORCE_SYNC_OUTPUT",
    "CLAUDE_CODE_SAFE_MODE",
];

/// The variables a probed program gets: the allow-list, then `overrides` (the terminal's own contract such as
/// `TERM`, plus per-scenario extras). An override with a forbidden name that is not in [`ALLOWED_OVERRIDES`]
/// is an error.
pub fn child_environment(overrides: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, String> {
    let mut env = clean_environment();
    let (name, value) = DISABLE_AUTOUPDATER;
    env.insert(name.to_owned(), value.to_owned());
    for (name, value) in overrides {
        if is_forbidden(name) && !ALLOWED_OVERRIDES.iter().any(|allowed| allowed.eq_ignore_ascii_case(name)) {
            return Err(format!("environment override {name} is on the forbidden list"));
        }
        env.insert(name.clone(), value.clone());
    }
    Ok(env)
}

#[cfg(windows)]
fn machine_path() -> String {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::{
            Environment::ExpandEnvironmentStringsW,
            Registry::{HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW},
        },
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    let fallback = || {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        format!("{root}\\System32;{root};{root}\\System32\\Wbem;{root}\\System32\\WindowsPowerShell\\v1.0")
    };
    let subkey = wide("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment");
    let value_name = wide("Path");
    // SAFETY: plain registry and environment calls with valid, NUL-terminated buffers.
    unsafe {
        let mut key: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, subkey.as_ptr(), 0, KEY_READ, &mut key) != ERROR_SUCCESS {
            return fallback();
        }
        let mut size = 0u32;
        let status = RegQueryValueExW(key, value_name.as_ptr(), null(), null_mut(), null_mut(), &mut size);
        if status != ERROR_SUCCESS || size == 0 {
            RegCloseKey(key);
            return fallback();
        }
        let mut raw = vec![0u16; (size as usize).div_ceil(2) + 1];
        let status = RegQueryValueExW(key, value_name.as_ptr(), null(), null_mut(), raw.as_mut_ptr().cast(), &mut size);
        RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return fallback();
        }
        let len = raw.iter().position(|&unit| unit == 0).unwrap_or(raw.len());
        raw.truncate(len);
        raw.push(0);
        let needed = ExpandEnvironmentStringsW(raw.as_ptr(), null_mut(), 0);
        if needed == 0 {
            return fallback();
        }
        let mut expanded = vec![0u16; needed as usize];
        let written = ExpandEnvironmentStringsW(raw.as_ptr(), expanded.as_mut_ptr(), needed);
        if written == 0 {
            return fallback();
        }
        expanded.truncate((written as usize).saturating_sub(1));
        String::from_utf16_lossy(&expanded)
    }
}

#[cfg(not(windows))]
fn machine_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

static ENV_SWAP: Mutex<()> = Mutex::new(());

/// Restores the process environment when dropped.
struct Restore(Vec<(OsString, OsString)>);

impl Drop for Restore {
    fn drop(&mut self) {
        // Variables the swap added (`TERM`, ...) must not outlive it.
        for (name, _) in std::env::vars_os().collect::<Vec<_>>() {
            if !name.to_string_lossy().starts_with('=') {
                remove_var(&name);
            }
        }
        for (name, value) in std::mem::take(&mut self.0) {
            if !name.to_string_lossy().starts_with('=') {
                set_var(&name, &value);
            }
        }
    }
}

fn set_var(name: &OsStr, value: &OsStr) {
    // SAFETY: documented as always safe on Windows; callers hold `ENV_SWAP`, and nothing else in the
    // harness mutates the environment.
    unsafe { std::env::set_var(name, value) }
}

fn remove_var(name: &OsStr) {
    // SAFETY: as for `set_var`.
    unsafe { std::env::remove_var(name) }
}

/// Runs `f` while this process's environment is exactly `clean` (plus the hidden `=C:`-style per-drive
/// variables, which carry directories only), then restores the original environment. `f` must be the call
/// that builds the child's environment block and creates the process.
pub fn with_process_environment<R>(clean: &BTreeMap<String, String>, f: impl FnOnce() -> R) -> R {
    let _lock = ENV_SWAP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let original: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let restore = Restore(original.clone());
    for (name, _) in &original {
        if !name.to_string_lossy().starts_with('=') {
            remove_var(name);
        }
    }
    for (name, value) in clean {
        set_var(OsStr::new(name), OsStr::new(value));
    }
    let result = f();
    drop(restore);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn only_allow_listed_names_survive() {
        let clean = clean_environment_from(
            vars(&[
                ("SystemRoot", "C:\\Windows"),
                ("USERPROFILE", "C:\\Users\\u"),
                ("CLAUDE_CODE_MESSAGING_TOKEN", "x"),
                ("CLAUDECODE", "1"),
                ("ANTHROPIC_API_KEY", "x"),
                ("MCP_SERVER", "x"),
                ("OTEL_EXPORTER", "x"),
                ("NO_COLOR", "1"),
                ("PATH", "C:\\polluted"),
                ("PROCESSOR_ARCHITECTURE", "AMD64"),
                ("CommonProgramFiles(x86)", "C:\\c"),
                ("ProgramFiles(x86)", "C:\\p"),
                ("=C:", "C:\\here"),
                ("RANDOM_THING", "x"),
            ]),
            "C:\\Windows\\System32;",
            Some("C:\\Users\\u\\.local\\bin"),
        );
        let names: Vec<&str> = clean.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "CommonProgramFiles(x86)",
                "PATH",
                "PROCESSOR_ARCHITECTURE",
                "ProgramFiles(x86)",
                "SystemRoot",
                "USERPROFILE"
            ]
        );
        assert_eq!(
            clean["PATH"], "C:\\Windows\\System32;C:\\Users\\u\\.local\\bin",
            "PATH never comes from the session"
        );
    }

    #[test]
    fn names_match_case_insensitively() {
        assert!(is_allowed("systemroot"));
        assert!(is_allowed("Processor_Level"));
        assert!(!is_allowed("CLAUDE_CODE_SESSION_ID"));
        assert!(is_forbidden("claude_code_session_id"));
        assert!(is_forbidden("MY_SERVICE_TOKEN"));
        assert!(!is_forbidden("SystemRoot"));
    }

    #[test]
    fn nothing_allowed_is_forbidden() {
        for name in ALLOWED_NAMES {
            assert!(!is_forbidden(name), "{name}");
        }
        assert!(!is_forbidden(DISABLE_AUTOUPDATER.0));
    }

    #[test]
    fn the_host_session_variables_are_forbidden_and_never_overridable() {
        for name in [
            "CLAUDE_CODE_MESSAGING_TOKEN",
            "CLAUDE_CODE_MESSAGING_SOCKET",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDECODE",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            "MCP_SERVER_URL",
            "NO_COLOR",
        ] {
            assert!(is_forbidden(name), "{name}");
            assert!(!is_allowed(name), "{name}");
            let overrides = BTreeMap::from([(name.to_owned(), "x".to_owned())]);
            assert!(child_environment(&overrides).is_err(), "{name}");
        }
    }

    #[test]
    fn behaviour_switches_are_overridable_and_the_updater_stays_off() {
        for name in ALLOWED_OVERRIDES {
            let overrides = BTreeMap::from([((*name).to_owned(), "1".to_owned())]);
            let env = child_environment(&overrides).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(env[*name], "1");
            assert_eq!(env[DISABLE_AUTOUPDATER.0], DISABLE_AUTOUPDATER.1);
        }
    }

    #[test]
    fn the_overridable_switches_carry_no_secret() {
        for name in ALLOWED_OVERRIDES {
            for suffix in ["_TOKEN", "_SECRET", "_API_KEY"] {
                assert!(!name.ends_with(suffix), "{name}");
            }
            assert!(!name.contains("MESSAGING") && !name.contains("SESSION"), "{name}");
        }
    }
}
