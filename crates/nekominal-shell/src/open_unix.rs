//! Opening a URL or a file with the program the desktop associates with it (`xdg-open`).
//!
//! The callers decide what may be opened: links go through `nekominal_term::links::open_target` first, which lets
//! through only web, mail and local file links that do not run code. This module only asks the desktop.

use std::process::{Command, Stdio};

use crate::util::log;

/// With this variable set (to anything) nothing is opened: the target is logged and the call succeeds. The GUI
/// scenarios set it, so a click on a link can be checked without starting a browser.
pub const NO_LAUNCH: &str = "NEKOMINAL_NO_LAUNCH";

fn start(program: &str, args: &[&str]) -> Result<(), String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("{program}: {e}"))
}

/// Shows the file at `path` selected in the file manager, where it can be asked to (the FileManager1 service of the
/// desktop); otherwise the folder it is in is opened.
pub fn reveal(path: &str) -> Result<(), String> {
    if std::env::var_os(NO_LAUNCH).is_some() {
        log(&format!("not shown ({NO_LAUNCH} is set): {path}"));
        return Ok(());
    }
    let uri = format!("file://{path}");
    let shown = Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.FileManager1",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("array:string:{uri}"))
        .arg("string:")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if shown {
        return Ok(());
    }
    let folder = std::path::Path::new(path).parent().map_or_else(|| path.to_owned(), |parent| parent.display().to_string());
    start("xdg-open", &[&folder])
}

/// Opens `target` (a URL or a path) with its default program.
pub fn shell_open(target: &str) -> Result<(), String> {
    if std::env::var_os(NO_LAUNCH).is_some() {
        log(&format!("not opened ({NO_LAUNCH} is set): {target}"));
        return Ok(());
    }
    start("xdg-open", &[target])
}
