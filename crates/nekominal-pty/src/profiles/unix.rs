//! Shell discovery on Unix: the login shell first, then the other shells of `/etc/shells`, then the `~/.ssh/config` hosts.

use std::{
    env,
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::*;

/// Kind of a shell that has no kind of its own: the model reads it as `ProfileKind::Shell`.
pub const KIND_SHELL: &str = "shell";

/// Icon of a shell without a mark of its own (the model's `GENERIC_ICON`).
const ICON_TERMINAL: &str = "terminal";

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// `exe` in the first absolute folder of `path` (a PATH) that has it; a relative entry would name the current directory.
fn find_in(path: &OsStr, exe: &str) -> Option<PathBuf> {
    env::split_paths(path).filter(|dir| dir.is_absolute()).map(|dir| dir.join(exe)).find(|candidate| is_executable(candidate))
}

/// Resolves a program name through PATH (never the current directory); a path is taken as it is when it is a program.
pub fn resolve_program(program: &str) -> Option<PathBuf> {
    let direct = Path::new(program);
    if direct.components().count() > 1 || direct.is_absolute() {
        return is_executable(direct).then(|| direct.to_path_buf());
    }
    find_in(&env::var_os("PATH")?, program)
}

/// The shells `/etc/shells` lists, as paths that exist.
fn listed_shells() -> Vec<PathBuf> {
    fs::read_to_string("/etc/shells")
        .map(|text| text.lines().map(str::trim).filter(|line| line.starts_with('/')).map(PathBuf::from).filter(|path| is_executable(path)).collect())
        .unwrap_or_default()
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

fn shell_profile(path: &Path) -> Profile {
    let name = file_name(path);
    let (kind, args): (&'static str, &[&str]) = if name == "bash" { (KIND_BASH, &["--login"]) } else { (KIND_SHELL, &[]) };
    profile(&name, &name, kind, ICON_TERMINAL, path, args)
}

fn ssh_hosts() -> Vec<String> {
    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".ssh").join("config"))
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|config| parse_ssh_hosts(&config))
        .unwrap_or_default()
}

/// Finds the shells on this machine. The login shell is the first, so it is the default; every other shell of
/// `/etc/shells` follows once per name (`/bin/bash` and `/usr/bin/bash` are one), then the ssh hosts in file order.
/// Blocking, so call it off the UI thread.
pub fn discover() -> Vec<Profile> {
    let login = env::var_os("SHELL").map(PathBuf::from).filter(|path| is_executable(path));
    let mut profiles: Vec<Profile> = Vec::new();
    for path in login.into_iter().chain(listed_shells()) {
        if !profiles.iter().any(|found| found.id == file_name(&path)) {
            profiles.push(shell_profile(&path));
        }
    }
    if let Some(ssh) = resolve_program("ssh") {
        for host in ssh_hosts() {
            profiles.push(profile(&format!("ssh-{host}"), &host, KIND_SSH, "ssh", &ssh, &[&host]));
        }
    }
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shells_found_have_one_profile_per_name_and_the_login_shell_first() {
        let profiles = discover();
        let mut ids: Vec<&str> = profiles.iter().map(|profile| profile.id.as_str()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "{profiles:?}");
        if let Some(login) = env::var_os("SHELL").map(PathBuf::from).filter(|path| is_executable(path)) {
            assert_eq!(profiles[0].id, file_name(&login));
        }
    }

    #[test]
    fn a_bare_name_is_found_on_the_path_and_a_missing_one_is_not() {
        assert!(resolve_program("sh").is_some());
        assert!(resolve_program("no-such-program-nekominal").is_none());
    }
}
