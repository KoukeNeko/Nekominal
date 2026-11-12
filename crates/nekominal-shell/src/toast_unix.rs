//! Notifications of the desktop (`notify-send`). A click on one does nothing: the command gives no answer back.

use std::process::{Command, Stdio};

use crate::win::Hwnd;

/// The tab id that stands for the notice of an update, as on Windows.
pub const UPDATE: u64 = u64::MAX;

/// Shows a notification with `title` and `body`. The window and the tab are what a click would go back to.
pub fn show(_hwnd: Hwnd, _tab: u64, title: &str, body: &str) -> Result<(), String> {
    Command::new("notify-send")
        .args(["--app-name=Nekominal", "--icon=utilities-terminal", "--"])
        .arg(title)
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("notify-send: {e}"))
}
