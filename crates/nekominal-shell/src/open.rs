//! Opening a URL or a file with the program Windows associates with it (`ShellExecuteW`, verb "open").
//!
//! The callers decide what may be opened: links go through `nekominal_term::links::open_target` first, which lets
//! through only web, mail and local file links that do not run code. This module only asks the shell.

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use crate::util::log;

/// With this variable set (to anything) nothing is opened: the target is logged and the call succeeds. The GUI
/// scenarios set it, so a click on a link can be checked without starting a browser.
const NO_LAUNCH: &str = "NEKOMINAL_NO_LAUNCH";

/// Opens `target` (a URL or a path) with its default program.
pub fn shell_open(target: &str) -> Result<(), String> {
    if std::env::var_os(NO_LAUNCH).is_some() {
        log(&format!("not opened ({NO_LAUNCH} is set): {target}"));
        return Ok(());
    }
    let target = HSTRING::from(target);
    // SAFETY: both strings outlive the call, and no window owns the program that starts.
    let result = unsafe { ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL) };
    // The documented way to read the result: a value above 32 is success, anything else an `SE_ERR_*` code.
    let code = result.0 as isize;
    if code > 32 { Ok(()) } else { Err(format!("ShellExecute returned {code}")) }
}
