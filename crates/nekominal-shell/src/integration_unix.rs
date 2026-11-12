//! What Windows has of the shell (the right-click entry of Explorer, the jump list, the registry) has no counterpart here:
//! the calls say there is nothing to do.

use std::path::{Path, PathBuf};

/// The program an entry outside the app starts.
pub fn launch_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

pub fn has_explorer_menu() -> bool {
    false
}

pub fn explorer_command(exe: &Path) -> String {
    format!("\"{}\" -d \"%V\"", exe.display())
}

/// The words of the menu of a file manager.
pub struct MenuLabels<'a> {
    pub menu: &'a str,
    pub open: &'a str,
    pub admin: &'a str,
}

pub fn set_explorer_menu(_on: bool, _labels: &MenuLabels) -> Result<(), String> {
    Ok(())
}

/// The arguments of a task that starts the profile with this id.
pub fn profile_arguments(id: &str) -> String {
    format!("-p \"{}\"", id.replace('"', ""))
}

pub fn run_elevated(_args: &str) -> Result<(), String> {
    Err("a window cannot be started as administrator here".into())
}

pub fn set_jump_list(_profiles: &[(String, String, Option<PathBuf>)]) -> Result<(), String> {
    Ok(())
}
