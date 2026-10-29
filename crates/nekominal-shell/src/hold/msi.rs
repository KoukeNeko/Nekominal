//! What the updater does for a copy the installer put in place (`--msi <folder>`): downloads the installer of the new
//! version from its release, checks it against the checksum the release lists beside it, and runs it without a window.
//! The installer is per user, so it asks for no administrator; nothing runs from the folder by then (the shells' console
//! runs from a copy, the tabs are held here), so it replaces every file, and a failure rolls back to the version before.

use std::{os::windows::process::CommandExt, path::PathBuf, process::Command};

use crate::update;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// The exit codes of msiexec for an install that was made: done, and done with a restart Windows would like later.
const DONE: [i32; 2] = [0, 3010];

/// The folder the installer is downloaded into, and its log written to.
fn work_dir() -> PathBuf {
    std::env::temp_dir().join("Nekominal-update")
}

/// Installs version `to`, telling each step to `step`. What went wrong, when the version installed did not change.
pub fn install(to: &str, mut step: impl FnMut(Step)) -> Result<(), String> {
    let name = update::msi_name(to);
    let dir = work_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let msi = dir.join(&name);
    let log = dir.join(format!("{name}.log"));

    step(Step::Download);
    let listing = update::fetch(&update::release_file(to, &format!("{name}.sha256"))).ok_or_else(|| format!("{name}.sha256 could not be read"))?;
    let expected = update::sha256_of_listing(&listing).ok_or_else(|| format!("{name}.sha256 names no checksum"))?;
    update::download(&update::release_file(to, &name), &msi)?;
    let bytes = std::fs::read(&msi).map_err(|e| format!("{}: {e}", msi.display()))?;
    if update::sha256_hex(&bytes) != expected {
        let _ = std::fs::remove_file(&msi);
        return Err(format!("{name} does not match its checksum"));
    }

    step(Step::Install);
    // Restart Manager stays out: with no window it would close the programs it found holding a file, unasked.
    let status = Command::new("msiexec.exe")
        .arg("/i")
        .arg(&msi)
        .args(["/qn", "/norestart", "MSIRESTARTMANAGERCONTROL=Disable", "/l*v"])
        .arg(&log)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("msiexec: {e}"))?;
    let _ = std::fs::remove_file(&msi);
    match status.code() {
        Some(code) if DONE.contains(&code) => {
            let _ = std::fs::remove_file(&log);
            Ok(())
        }
        // The log stays, for what went wrong.
        code => Err(format!("the installer ended with {}; its log is {}", code.map_or("no code".to_owned(), |c| c.to_string()), log.display())),
    }
}

/// What [`install`] is doing.
pub enum Step {
    Download,
    Install,
}
